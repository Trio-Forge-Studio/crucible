use crate::output::Output;
use crate::plan::{Failure, Project, render};
use crucible_iac::model::Kind;
use crucible_iac::oc::Roblox;
use crucible_iac::plan::{Plan, plan, undeclared};
use crucible_iac::provision::{Applied, Files, Refused, Run, Stop, forget, listed_kinds, observe};
use crucible_oc::auth;
use crucible_oc::transport::{Event, Sink, SystemClock, Timing, Transport};
use serde_json::{Value, json};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

#[derive(clap::Args)]
pub struct Args {
    /// The environment in crucible.toml.
    pub env: String,
    /// List what Roblox holds and print the plan, without writing to Roblox or to a file.
    #[arg(long)]
    pub dry_run: bool,
    /// Print one JSON document instead of text.
    #[arg(long)]
    pub json: bool,
    /// Write request details to stderr.
    #[arg(long)]
    pub verbose: bool,
    /// Do not check the key's scopes first. A call that lacks a scope then fails by itself.
    #[arg(long)]
    pub no_scope_check: bool,
}

struct Notes<'a>(&'a Output);

impl Sink for Notes<'_> {
    fn event(&self, event: Event<'_>) {
        match event {
            Event::Note(line) => self.0.note(line),
            Event::Trace(line) => self.0.trace(line),
            Event::Accepted => {}
        }
    }
}

/// How a run ended, for the document and the exit code.
struct Report {
    plan: Option<Plan>,
    applied: Vec<Applied>,
    error: Option<(u8, Value)>,
    /// The IDs `crucible adopt --none` found and the developer passed over.
    passed: Vec<std::num::NonZeroU64>,
}

pub fn main(args: Args) -> ExitCode {
    let mut output = Output::new(args.verbose);
    let report = match std::env::current_dir() {
        Ok(root) => run(&root, &args, &mut output),
        Err(error) => failed(&Failure::read(Path::new("."), &error)),
    };
    let code = report.error.as_ref().map_or(0, |(code, _)| *code);
    let written = if args.json {
        let mut document = json!({ "plan": report.plan, "applied": report.applied, "dryRun": args.dry_run });
        if let Some((_, error)) = report.error {
            document["error"] = error;
        }
        output.document(&document, code != 0)
    } else {
        let mut text = String::new();
        if let Some(plan) = &report.plan {
            text += &render(std::slice::from_ref(plan));
            text.push('\n');
        }
        for applied in &report.applied {
            let action = json!(applied.action);
            text +=
                &format!("{} {} {} {}\n", action.as_str().unwrap_or_default(), applied.kind, applied.key, applied.id);
        }
        match &report.error {
            Some((_, error)) => {
                let mut message = error["message"].as_str().unwrap_or_default().to_owned();
                for problem in error["problems"].as_array().into_iter().flatten() {
                    let at = match problem["line"].as_u64() {
                        Some(line) => format!("{}:{line}", problem["file"].as_str().unwrap_or_default()),
                        None => problem["file"].as_str().unwrap_or_default().to_owned(),
                    };
                    let (key, said) = (problem["key"].as_str(), problem["message"].as_str());
                    message += &format!("\n  {at}: {}: {}", key.unwrap_or_default(), said.unwrap_or_default());
                }
                if let Some(hint) = error["hint"].as_str() {
                    message += &format!("\n{hint}");
                }
                output.note(&message);
            }
            None if args.dry_run => text += "Dry run: nothing was written.\n",
            None => text += &format!("Provisioned {}. Commit crucible.lock.\n", args.env),
        }
        output.text(&text)
    };
    match written {
        Err(_) if code == 0 => ExitCode::from(1),
        _ => ExitCode::from(code),
    }
}

fn failed(failure: &Failure) -> Report {
    let problems = &failure.problems;
    let kind = if failure.code == 2 { "usage" } else { "io" };
    let error = json!({ "kind": kind, "message": failure.message, "problems": problems });
    Report { plan: None, applied: vec![], error: Some((failure.code, error)), passed: vec![] }
}

fn run(root: &Path, args: &Args, output: &mut Output) -> Report {
    let project = match Project::load(root, "provision") {
        Ok(project) => project,
        Err(failure) => return failed(&failure),
    };
    let env = args.env.as_str();
    let Some(declared) = project.manifest.envs.get(env) else { return failed(&project.unknown(env)) };
    let read = &mut |path: &Path| std::fs::read(root.join(path));
    let mut problems = undeclared(&project.manifest.envs, project.lock.as_ref(), &project.journals);
    if let Err(found) = plan(&project.inputs(env, None), read) {
        problems.extend(found);
    }
    if !problems.is_empty() {
        let message = "the plan breaks the provisioning rules:".into();
        return failed(&Failure { code: 2, message, problems });
    }
    let (source, key) = match crate::load_key(root) {
        Ok(found) => found,
        Err(failure) => return remote(None, vec![], failure, None),
    };
    output.redact_also(&key);
    let transport = match transport(&key, output) {
        Ok(transport) => transport,
        Err(failure) => return remote(None, vec![], failure, Some(&source)),
    };
    let lock = project.lock.as_ref().and_then(|lock| lock.env(env));
    let key = if args.no_scope_check {
        None
    } else {
        match crate::scopes::introspect(&transport, output) {
            Ok(key) => key,
            Err(failure) => return remote(None, vec![], failure, Some(&source)),
        }
    };
    let reads = listed_kinds(&project.experience, declared, lock, project.journals.get(env));
    let needed = crucible_iac::oc::scopes(&reads, &[]);
    if let Err(failure) = crate::scopes::require(key.as_ref(), &needed) {
        return scoped(None, vec![], failure, Some(&source), Some(needed));
    }
    let mut roblox = Roblox { transport: &transport, universe: declared.universe };
    let observed = match observe(&project.experience, declared, lock, project.journals.get(env), &mut roblox) {
        Ok(observed) => observed,
        Err(failure) => return remote(None, vec![], failure, Some(&source)),
    };
    if let Some(Err(message)) = &observed.settings {
        output.note(&format!(
            "Could not read the experience settings Roblox holds: {message}. Going on; provision compares them with \
             crucible.lock instead."
        ));
    }
    let inputs = project.inputs(env, Some(&observed));
    let plan = match plan(&inputs, read) {
        Ok(plan) => plan,
        Err(problems) => {
            let message = "what Roblox holds stops the plan:".into();
            return failed(&Failure { code: 2, message, problems });
        }
    };
    if args.dry_run {
        return Report { plan: Some(plan), applied: vec![], error: None, passed: vec![] };
    }
    let empty = crucible_iac::lock::Lock::empty();
    let mut run = match Run::new(&inputs, &plan, project.lock.as_ref().unwrap_or(&empty), read) {
        Ok(run) => run,
        Err(error) => {
            let failure = Failure { code: 1, message: format!("cannot read an icon: {error}"), problems: vec![] };
            return Report { plan: Some(plan), ..failed(&failure) };
        }
    };
    let needed = crucible_iac::oc::scopes(&[], &run.writes());
    if let Err(failure) = crate::scopes::require(key.as_ref(), &needed) {
        return scoped(Some(plan), vec![], failure, Some(&source), Some(needed));
    }
    let mut files = Files { root: root.to_owned() };
    let stopped = loop {
        match run.step(&mut roblox, &mut files) {
            Ok(true) => {}
            Ok(false) => break None,
            Err(stop) => break Some(stop),
        }
    };
    let applied = run.applied().to_vec();
    match stopped {
        None => Report { plan: Some(plan), applied, error: None, passed: vec![] },
        Some(Stop::Remote { kind, key, failure, intent }) => {
            let mut report = remote(Some(plan), applied, failure, Some(&source));
            if let Some((_, error)) = &mut report.error {
                error["resource"] = json!(format!("{} {key}", kind.noun()));
                if intent {
                    error["hint"] = json!(format!(
                        "The create of {} {key} may have landed. Its intent stays in {}, and the next provision looks \
                         for the resource before it creates anything",
                        kind.noun(),
                        crucible_iac::journal::path(env).display()
                    ));
                }
            }
            report
        }
        Some(Stop::Settings(failure)) => {
            let mut report = remote(Some(plan), applied, failure, Some(&source));
            if let Some((_, error)) = &mut report.error {
                error["resource"] = json!("experience settings");
                error["hint"] = json!(
                    "Provision writes the experience settings last, so every product, pass and badge step was \
                     applied, and crucible.lock records it: commit it. The next provision sends the settings again, and \
                     until then `crucible deploy` warns about them and goes on"
                );
            }
            report
        }
        Some(Stop::Write { path, error }) => {
            let failure =
                Failure { code: 1, message: format!("cannot write {}: {error}", path.display()), problems: vec![] };
            Report { plan: Some(plan), applied, ..failed(&failure) }
        }
        Some(Stop::Unresolved { kind, key, other }) => {
            let message = format!(
                "provision would write the intent of {} {key} while the create of {other} may have landed. This is a \
                 bug in crucible; nothing was sent for {key}",
                kind.noun()
            );
            Report {
                plan: Some(plan),
                applied,
                error: Some((1, json!({ "kind": "internal", "message": message }))),
                passed: vec![],
            }
        }
    }
}

pub(crate) fn transport<'a>(
    key: &'a auth::Secret,
    output: &'a Output,
) -> Result<Transport<'a>, crucible_oc::outcome::Failure> {
    let timing =
        Timing { deadline: Duration::from_secs(300), poll_interval: Duration::from_secs(2), retry_delay: None };
    let origin = std::env::var("CRUCIBLE_OC_ORIGIN").ok();
    let proxy = ureq::Proxy::try_from_env();
    Transport::new(origin, Some(key), timing, proxy, Box::new(Notes(output)), Box::new(SystemClock))
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum KindArg {
    /// A developer product.
    Product,
    /// A game pass.
    Pass,
    /// A badge.
    Badge,
}

#[derive(clap::Args)]
pub struct AdoptArgs {
    /// The environment in crucible.toml.
    pub env: String,
    /// The kind of resource.
    pub kind: KindArg,
    /// The key in crucible.toml.
    pub key: String,
    /// The Roblox ID of the resource that is the key's.
    #[arg(required_unless_present = "none", conflicts_with = "none")]
    pub id: Option<std::num::NonZeroU64>,
    /// Clear the key's unresolved intent instead: you looked, and no resource on Roblox is the create it records.
    #[arg(long)]
    pub none: bool,
    /// Print one JSON document instead of text.
    #[arg(long)]
    pub json: bool,
    /// Write request details to stderr.
    #[arg(long)]
    pub verbose: bool,
}

pub fn adopt_main(args: AdoptArgs) -> ExitCode {
    let mut output = Output::new(args.verbose);
    let report = match std::env::current_dir() {
        Ok(root) => adopt(&root, &args, &mut output),
        Err(error) => failed(&Failure::read(Path::new("."), &error)),
    };
    let code = report.error.as_ref().map_or(0, |(code, _)| *code);
    let written = if args.json {
        let mut document = json!({ "applied": report.applied });
        if args.none && report.error.is_none() {
            document["passedOver"] = json!(report.passed);
        }
        if let Some((_, error)) = report.error {
            document["error"] = error;
        }
        output.document(&document, code != 0)
    } else {
        match &report.error {
            Some((_, error)) => {
                let mut message = error["message"].as_str().unwrap_or_default().to_owned();
                for problem in error["problems"].as_array().into_iter().flatten() {
                    let (file, key, said) =
                        (problem["file"].as_str(), problem["key"].as_str(), problem["message"].as_str());
                    message += &format!(
                        "\n  {}: {}: {}",
                        file.unwrap_or_default(),
                        key.unwrap_or_default(),
                        said.unwrap_or_default()
                    );
                }
                output.note(&message);
                Ok(())
            }
            None if args.none => {
                let applied = &report.applied[0];
                let passed = match report.passed.as_slice() {
                    [] => String::new(),
                    ids => format!(
                        " You passed over {}. If one of them was its create, the next run makes a second one.",
                        ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                    ),
                };
                output.text(&format!(
                    "forget {} {}\nCleared its intent from {}.{passed} Run `crucible provision {}`, which creates it.\n",
                    applied.kind,
                    applied.key,
                    crucible_iac::journal::path(&args.env).display(),
                    args.env
                ))
            }
            None => output.text(&format!(
                "adopt {} {} {}\nRecorded in crucible.lock. Commit it, then run `crucible provision {}`.\n",
                report.applied[0].kind, report.applied[0].key, report.applied[0].id, args.env
            )),
        }
    };
    match written {
        Err(_) if code == 0 => ExitCode::from(1),
        _ => ExitCode::from(code),
    }
}

fn adopt(root: &Path, args: &AdoptArgs, output: &mut Output) -> Report {
    let project = match Project::load(root, "adopt") {
        Ok(project) => project,
        Err(failure) => return failed(&failure),
    };
    let env = args.env.as_str();
    let Some(declared) = project.manifest.envs.get(env) else { return failed(&project.unknown(env)) };
    let kind = match args.kind {
        KindArg::Product => Kind::Product,
        KindArg::Pass => Kind::Pass,
        KindArg::Badge => Kind::Badge,
    };
    let problems = left_after(root, &project, env, kind, &args.key, args.id);
    if !problems.is_empty() {
        let message = "provision would still stop on these after it, so nothing was written:".into();
        return failed(&Failure { code: 2, message, problems });
    }
    let (source, key) = match crate::load_key(root) {
        Ok(found) => found,
        Err(failure) => return remote(None, vec![], failure, None),
    };
    output.redact_also(&key);
    let transport = match transport(&key, output) {
        Ok(transport) => transport,
        Err(failure) => return remote(None, vec![], failure, Some(&source)),
    };
    let mut roblox = Roblox { transport: &transport, universe: declared.universe };
    let empty = crucible_iac::lock::Lock::empty();
    let lock = project.lock.as_ref().unwrap_or(&empty);
    let mut files = Files { root: root.to_owned() };
    let inputs = project.inputs(env, None);
    let done = match args.id {
        Some(id) => crucible_iac::provision::adopt(&inputs, lock, kind, &args.key, id, &mut roblox, &mut files)
            .map(|applied| (applied, vec![])),
        None => forget(&inputs, kind, &args.key, &mut roblox, &mut files),
    };
    match done {
        Ok((applied, passed)) => Report { plan: None, applied: vec![applied], error: None, passed },
        Err(Refused::Rule(message)) => failed(&Failure::usage(message)),
        Err(Refused::Remote(failure)) => remote(None, vec![], failure, Some(&source)),
        Err(Refused::Write { path, error }) => {
            failed(&Failure { code: 1, message: format!("cannot write {}: {error}", path.display()), problems: vec![] })
        }
    }
}

/// What would stop provision once this adopt, or with `id` `None` this `--none`, is written: the problems of every
/// environment `crucible.toml` no longer declares, and the offline plan's with the key's ID in the lock and its intent
/// out of the journal. A problem the command resolves does not refuse it. An adopt of a key that is not declared and
/// managed is left to `adopt`'s own refusal.
fn left_after(
    root: &Path,
    project: &Project,
    env: &str,
    kind: Kind,
    key: &str,
    id: Option<std::num::NonZeroU64>,
) -> Vec<crucible_iac::model::Problem> {
    let declared = &project.manifest.envs[env];
    let mut problems = undeclared(&project.manifest.envs, project.lock.as_ref(), &project.journals);
    let resources = project.experience.resolve(declared);
    let resource = resources.iter().find(|resource| resource.kind == kind && resource.key == key);
    let mut lock = project.lock.clone().unwrap_or_else(crucible_iac::lock::Lock::empty);
    if let Some(id) = id {
        let Some(fields) = resource.and_then(|resource| resource.fields.as_ref()) else { return problems };
        let entry = crucible_iac::lock::Entry {
            id,
            fingerprint: fields.fingerprint(),
            icon: None,
            status: crucible_iac::lock::Status::Managed,
        };
        lock.env_mut(env, declared.universe).entries_mut(kind).insert(key.to_owned(), entry);
    }
    let moved_from = resource.and_then(|resource| resource.moved_from.as_ref());
    let journal = project.journals.get(env).cloned().map(|mut journal| {
        journal
            .intents
            .retain(|intent| !(intent.kind == kind && (intent.key == key || moved_from == Some(&intent.key))));
        journal
    });
    let inputs = crucible_iac::plan::Inputs {
        lock: lock.env(env),
        journal: journal.as_ref().filter(|journal| !journal.intents.is_empty()),
        ..project.inputs(env, None)
    };
    if let Err(found) = plan(&inputs, &mut |path: &Path| std::fs::read(root.join(path))) {
        problems.extend(found);
    }
    problems
}

fn remote(
    plan: Option<Plan>,
    applied: Vec<Applied>,
    failure: crucible_oc::outcome::Failure,
    source: Option<&auth::KeySource>,
) -> Report {
    scoped(plan, applied, failure, source, None)
}

/// `remote`, for a failure of the scope check, whose `requiredScopes` are the scopes the run needs.
fn scoped(
    plan: Option<Plan>,
    applied: Vec<Applied>,
    failure: crucible_oc::outcome::Failure,
    source: Option<&auth::KeySource>,
    needed: Option<Vec<crucible_oc::Scope>>,
) -> Report {
    let auth = crucible_oc::Auth::ApiKey { scopes: needed };
    let failure = auth::decorate(failure, &auth, source);
    Report { plan, applied, error: Some((failure.code, failure.error)), passed: vec![] }
}
