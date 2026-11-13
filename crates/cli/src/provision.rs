use crate::output::Output;
use crate::plan::{Failure, Project, render};
use crucible_iac::model::Kind;
use crucible_iac::oc::Roblox;
use crucible_iac::plan::{Plan, plan, undeclared};
use crucible_iac::provision::{Applied, Files, Refused, Run, Stop, observe};
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
            if plan.steps.iter().any(|step| step.kind == "experience") {
                text += "  Provision does not write experience settings yet. They stay as they are on Roblox.\n";
            }
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
    Report { plan: None, applied: vec![], error: Some((failure.code, error)) }
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
    let mut roblox = Roblox { transport: &transport, universe: declared.universe };
    let lock = project.lock.as_ref().and_then(|lock| lock.env(env));
    let observed = match observe(&project.experience, declared, lock, project.journals.get(env), &mut roblox) {
        Ok(observed) => observed,
        Err(failure) => return remote(None, vec![], failure, Some(&source)),
    };
    let inputs = project.inputs(env, Some(&observed));
    let plan = match plan(&inputs, read) {
        Ok(plan) => plan,
        Err(problems) => {
            let message = "what Roblox holds stops the plan:".into();
            return failed(&Failure { code: 2, message, problems });
        }
    };
    if args.dry_run {
        return Report { plan: Some(plan), applied: vec![], error: None };
    }
    let empty = crucible_iac::lock::Lock::empty();
    let mut run = match Run::new(&inputs, &plan, project.lock.as_ref().unwrap_or(&empty), read) {
        Ok(run) => run,
        Err(error) => {
            let failure = Failure { code: 1, message: format!("cannot read an icon: {error}"), problems: vec![] };
            return Report { plan: Some(plan), ..failed(&failure) };
        }
    };
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
        None => Report { plan: Some(plan), applied, error: None },
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
            Report { plan: Some(plan), applied, error: Some((1, json!({ "kind": "io", "message": message }))) }
        }
    }
}

fn transport<'a>(key: &'a auth::Secret, output: &'a Output) -> Result<Transport<'a>, crucible_oc::outcome::Failure> {
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
    pub id: std::num::NonZeroU64,
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
        if let Some((_, error)) = report.error {
            document["error"] = error;
        }
        output.document(&document, code != 0)
    } else {
        match &report.error {
            Some((_, error)) => {
                output.note(error["message"].as_str().unwrap_or_default());
                Ok(())
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
    let (source, key) = match crate::load_key(root) {
        Ok(found) => found,
        Err(failure) => return remote(None, vec![], failure, None),
    };
    output.redact_also(&key);
    let transport = match transport(&key, output) {
        Ok(transport) => transport,
        Err(failure) => return remote(None, vec![], failure, Some(&source)),
    };
    let kind = match args.kind {
        KindArg::Product => Kind::Product,
        KindArg::Pass => Kind::Pass,
        KindArg::Badge => Kind::Badge,
    };
    let mut roblox = Roblox { transport: &transport, universe: declared.universe };
    let empty = crucible_iac::lock::Lock::empty();
    let lock = project.lock.as_ref().unwrap_or(&empty);
    let mut files = Files { root: root.to_owned() };
    match crucible_iac::provision::adopt(
        &project.inputs(env, None),
        lock,
        kind,
        &args.key,
        args.id,
        &mut roblox,
        &mut files,
    ) {
        Ok(applied) => Report { plan: None, applied: vec![applied], error: None },
        Err(Refused::Rule(message)) => failed(&Failure::usage(message)),
        Err(Refused::Remote(failure)) => remote(None, vec![], failure, Some(&source)),
        Err(Refused::Write { path, error }) => {
            failed(&Failure { code: 1, message: format!("cannot write {}: {error}", path.display()), problems: vec![] })
        }
    }
}

fn remote(
    plan: Option<Plan>,
    applied: Vec<Applied>,
    failure: crucible_oc::outcome::Failure,
    source: Option<&auth::KeySource>,
) -> Report {
    let auth = crucible_oc::Auth::ApiKey { scopes: None };
    let failure = auth::decorate(failure, &auth, source);
    Report { plan, applied, error: Some((failure.code, failure.error)) }
}
