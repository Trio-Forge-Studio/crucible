use crate::cli::{DEFAULT_DEADLINE, DEFAULT_POLL_INTERVAL};
use crate::exec::Executor;
use crate::manifest::VersionType;
use crate::output::Output;
use crucible_oc::auth;
use crucible_oc::outcome::{Acceptance, Failure};
use crucible_oc::transport::Timing;
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
    /// Build and check the key's scopes without uploading.
    #[arg(long)]
    pub dry_run: bool,
    /// Write request and response details to stderr.
    #[arg(long)]
    pub verbose: bool,
}

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

fn run(root: &Path, args: Args, output: &mut Output) -> Result<Value, Failure> {
    let manifest = crate::build::manifest(root).map_err(|error| Failure::usage(error.to_string()))?;
    if !root.join(crate::manifest::FILE).exists() {
        return Err(Failure::usage(
            "publish requires crucible.toml. Example:\n[env.production]\nuniverse = 1234\nplace = 5678\n",
        ));
    }
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
    let targets: Vec<_> =
        env.places.keys().filter(|name| args.places.is_empty() || args.places.contains(name)).collect();
    let mut published = Vec::new();
    let progress = |published: &[Value], failed: Option<&String>, remaining: &[&String]| json!({"environment":args.env,"dryRun":args.dry_run,"published":published,"failed":failed,"notAttempted":remaining});
    let primary = crate::flows::op(&crate::REGISTRY, crate::flows::PUBLISH);
    let preflight = |failure: Failure| failure.with("publish", progress(&published, None, &targets));
    let (source, key) =
        crate::load_key(root).map_err(|failure| preflight(auth::decorate(failure, &primary.auth, None)))?;
    output.redact_also(&key);
    let execute = |file: Option<(&String, std::path::PathBuf)>, output: &mut Output| {
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
            match file {
                None => crate::whoami::check(&executor, &source, Some(primary)),
                Some((name, file)) => crate::flows::publish::run(
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
                ),
            }
        })();
        acceptance.into_inner().finish(result).map_err(|failure| auth::decorate(failure, &primary.auth, Some(&source)))
    };
    let built = crate::build::build(root, &manifest, &args.places, None).map_err(|error| {
        let failure = match error {
            crate::build::Error::Usage(message) => Failure::usage(message),
            crate::build::Error::Failed(message) => Failure::unavailable(message),
        };
        failure.with("publish", progress(&published, None, &targets))
    })?;
    if args.dry_run {
        let check =
            execute(None, output).map_err(|failure| failure.with("publish", progress(&published, None, &targets)))?;
        let mut document = progress(&published, None, &targets);
        document["auth"] = check;
        return Ok(document);
    }
    let mut remaining = built.into_iter();
    while let Some((name, file)) = remaining.next() {
        let result = execute(Some((&name, file)), output);
        match result {
            Ok(mut receipt) => {
                receipt["name"] = json!(name);
                published.push(receipt);
            }
            Err(mut failure) => {
                let remaining: Vec<_> = remaining.map(|(name, _)| name).collect();
                let names: Vec<_> = remaining.iter().collect();
                let mut state = progress(&published, Some(&name), &names);
                let mut next = vec!["crucible".to_owned(), "publish".to_owned(), args.env.clone()];
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
                return Err(failure.with("publish", state));
            }
        }
    }
    Ok(progress(&published, None, &[]))
}
