use crate::cli::{DEFAULT_DEADLINE, DEFAULT_POLL_INTERVAL};
use crate::exec::Executor;
use crate::output::Output;
use crate::plan::Project;
use crucible_iac::plan::{Action, plan, undeclared};
use crucible_oc::auth;
use crucible_oc::outcome::{Acceptance, Failure};
use crucible_oc::transport::Timing;
use crucible_project::VersionType;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

#[derive(clap::Args)]
pub struct Args {
    /// The environment in crucible.toml.
    pub env: String,
    /// Publish only this place. Repeat to publish several.
    #[arg(long = "place", value_name = "NAME")]
    pub places: Vec<String>,
    /// Run the checks and build without uploading.
    #[arg(long)]
    pub dry_run: bool,
    /// Write request and response details to stderr.
    #[arg(long)]
    pub verbose: bool,
    /// Do not check the key's scopes first. An upload that lacks a scope then fails by itself.
    #[arg(long)]
    pub no_scope_check: bool,
}

/// What `crucible publish`, which 0.6.0 replaced with `crucible deploy`, answers.
pub const PUBLISH_REPLACED: &str =
    "crucible publish is now crucible deploy. Run `crucible deploy <env>`, which `crucible docs deploy` describes";

pub fn main(args: Args) -> ExitCode {
    let mut output = Output::new(args.verbose);
    let result = std::env::current_dir()
        .map_err(|_| Failure::unavailable("cannot read the current directory"))
        .and_then(|root| run(&root, args, &mut output));
    let (code, document) = match result {
        Ok(document) => (0, document),
        Err(failure) => (failure.code, failure.document()),
    };
    if output.document(&document, code != 0).is_err() && code == 0 {
        return ExitCode::from(if document["published"].as_array().is_some_and(|v| !v.is_empty()) { 5 } else { 7 });
    }
    ExitCode::from(code)
}

/// Refuses unless `crucible.toml`, `crucible.lock` and the journal are valid and the environment's offline plan has
/// nothing left to do for a product, a pass or a badge, so each is on Roblox and recorded in the committed lock. Pending
/// experience settings only warn: no game code reads them, and a refused settings update must not block a release.
fn preflight(root: &Path, project: &Project, env: &str, output: &Output) -> Result<(), Failure> {
    let mut problems = undeclared(&project.manifest.envs, project.lock.as_ref(), &project.journals);
    let plan = plan(&project.inputs(env, None), &mut |path: &Path| std::fs::read(root.join(path)));
    let plan = match plan {
        Ok(plan) if problems.is_empty() => plan,
        Ok(_) => return Err(rules(problems)),
        Err(found) => {
            problems.extend(found);
            return Err(rules(problems));
        }
    };
    let (settings, pending): (Vec<_>, Vec<_>) =
        plan.steps.iter().filter(|step| step.action != Action::NoOp).partition(|step| step.kind == "experience");
    if pending.is_empty() {
        if !settings.is_empty() {
            output.note(&format!(
                "Warning: crucible.lock does not record the experience settings crucible.toml declares for {env}, so \
                 Roblox may not hold them. Deploy goes on; run `crucible provision {env}` to write them."
            ));
        }
        return Ok(());
    }
    let list: Vec<String> = pending
        .iter()
        .map(|step| format!("{} {} {}", json!(step.action).as_str().unwrap_or_default(), step.kind, step.key))
        .collect();
    let mut message = format!(
        "crucible.lock does not record every product, pass and badge crucible.toml declares for {env}: {}. Run \
         `crucible provision {env}`, commit crucible.lock, then deploy",
        list.join(", ")
    );
    if pending.iter().any(|step| step.action == Action::AdoptCheck) {
        message += ". An adopt-check is a create whose outcome the journal has not settled";
    }
    Err(Failure::usage(message).with("steps", json!(pending)))
}

fn rules(problems: Vec<crucible_iac::model::Problem>) -> Failure {
    Failure::usage("the plan breaks the provisioning rules").with("problems", json!(problems))
}

fn run(root: &Path, args: Args, output: &mut Output) -> Result<Value, Failure> {
    let project = Project::load(root, "deploy").map_err(|found| {
        let failure = Failure::usage(found.message);
        if found.problems.is_empty() { failure } else { failure.with("problems", json!(found.problems)) }
    })?;
    let manifest = &project.manifest;
    let env = manifest.envs.get(&args.env).ok_or_else(|| {
        if manifest.envs.is_empty() {
            return Failure::usage(format!("unknown environment {}. No environments are configured", args.env));
        }
        Failure::usage(format!(
            "unknown environment {}. Environments: {}",
            args.env,
            manifest.envs.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;
    if let Some(unknown) = args.places.iter().find(|name| !env.places.contains_key(*name)) {
        return Err(Failure::usage(format!(
            "unknown place {unknown}. Places: {}",
            env.places.keys().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    preflight(root, &project, &args.env, output)?;
    let targets: Vec<_> =
        env.places.keys().filter(|name| args.places.is_empty() || args.places.contains(name)).collect();
    let mut published = Vec::new();
    let progress = |published: &[Value], failed: Option<&String>, remaining: &[&String]| json!({"environment":args.env,"dryRun":args.dry_run,"published":published,"failed":failed,"notAttempted":remaining});
    let primary = crate::flows::op(&crate::REGISTRY, crate::flows::PUBLISH);
    let early = |failure: Failure| failure.with("deploy", progress(&published, None, &targets));
    let (source, key) = crate::load_key(root).map_err(|failure| early(auth::decorate(failure, &primary.auth, None)))?;
    output.redact_also(&key);
    let introspected = if args.no_scope_check {
        None
    } else {
        let transport = crate::provision::transport(&key, output)
            .map_err(|failure| early(auth::decorate(failure, &primary.auth, Some(&source))))?;
        let introspected = crate::scopes::introspect(&transport, output)
            .and_then(|introspected| {
                crate::scopes::require(introspected.as_ref(), primary.auth.required_scopes().unwrap_or_default())?;
                Ok(introspected)
            })
            .map_err(|failure| early(auth::decorate(failure, &primary.auth, Some(&source))))?;
        introspected.map(|key| {
            json!({"keySource":source.label(),"name":key.name,"enabled":key.enabled,"expired":key.expired,"scopes":key.scopes()})
        })
    };
    let execute = |name: &String, file: std::path::PathBuf, output: &mut Output| {
        let acceptance = RefCell::new(Acceptance::default());
        let result = (|| {
            let executor = Executor::new(
                &crate::REGISTRY,
                std::env::var("CRUCIBLE_OC_ORIGIN").ok(),
                Some(&key),
                Timing {
                    deadline: Duration::from_secs_f64(DEFAULT_DEADLINE),
                    poll_interval: Duration::from_secs_f64(DEFAULT_POLL_INTERVAL),
                    retry_delay: None,
                },
                output,
                &acceptance,
            )?;
            crate::flows::publish::run(
                &executor,
                crate::flows::publish::Args {
                    file,
                    universe: env.universe.to_string(),
                    place: env.places[name].to_string(),
                    version_type: match env.version_type {
                        VersionType::Saved => crate::flows::publish::VersionType::Saved,
                        VersionType::Published => crate::flows::publish::VersionType::Published,
                    },
                },
            )
        })();
        acceptance.into_inner().finish(result).map_err(|failure| auth::decorate(failure, &primary.auth, Some(&source)))
    };
    let built = crate::build::build(root, manifest, &args.places, None).map_err(|error| {
        let failure = match error {
            crate::build::Error::Usage(message) => Failure::usage(message),
            crate::build::Error::Failed(message) => Failure::unavailable(message),
        };
        failure.with("deploy", progress(&published, None, &targets))
    })?;
    if args.dry_run {
        let mut document = progress(&published, None, &targets);
        document["auth"] = introspected.unwrap_or(Value::Null);
        return Ok(document);
    }
    let mut remaining = built.into_iter();
    while let Some((name, file)) = remaining.next() {
        match execute(&name, file, output) {
            Ok(mut receipt) => {
                receipt["name"] = json!(name);
                published.push(receipt);
            }
            Err(mut failure) => {
                let remaining: Vec<_> = remaining.map(|(name, _)| name).collect();
                let names: Vec<_> = remaining.iter().collect();
                let mut state = progress(&published, Some(&name), &names);
                let mut next = vec!["crucible".to_owned(), "deploy".to_owned(), args.env.clone()];
                if args.verbose {
                    next.push("--verbose".to_owned());
                }
                for name in std::iter::once(&name).chain(remaining.iter()) {
                    next.push(format!("--place={name}"));
                }
                state["next"] = json!(next);
                if failure.code == 5 {
                    failure.error["hint"] = json!(
                        "Check the failed place's versions before re-uploading it. Never rerun the environment without --place after a partial failure."
                    );
                }
                return Err(failure.with("deploy", state));
            }
        }
    }
    Ok(progress(&published, None, &[]))
}
