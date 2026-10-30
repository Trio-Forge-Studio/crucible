use crate::cli::{AuthCommand, Oc, OcCommand, PlaceCommand};
use crate::manifest::{Manifest, VersionType};
use crate::outcome::{Acceptance, Failure};
use crate::output::Output;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;

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
    /// Override the environment's credential profile.
    #[arg(long)]
    pub profile: Option<String>,
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
    let manifest = Manifest::read(root).map_err(|error| Failure::usage(error.to_string()))?.ok_or_else(|| {
        Failure::usage("publish requires crucible.toml. Example:\n[env.production]\nuniverse = 1234\nplace = 5678\n")
    })?;
    let env = manifest.envs.get(&args.env).ok_or_else(|| {
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
    let profile = effective_profile(
        args.profile.as_deref(),
        std::env::var("CRUCIBLE_OC_PROFILE").ok().as_deref(),
        std::env::var_os("CRUCIBLE_OC_API_KEY").is_some(),
        env.profile.as_deref(),
    );
    let execute = |command, output: &mut Output| {
        let acceptance = RefCell::new(Acceptance::default());
        let result = crate::run(
            Oc {
                profile: profile.clone(),
                deadline: 300.0,
                poll_interval: 2.0,
                retry_delay: None,
                verbose: args.verbose,
                command,
            },
            output,
            &acceptance,
        );
        acceptance.into_inner().finish(result)
    };
    let built = crate::build::build(root, &manifest, &args.places, None).map_err(|error| {
        let failure = match error {
            crate::build::Error::Usage(message) => Failure::usage(message),
            crate::build::Error::Failed(message) => Failure::unavailable(message),
        };
        failure.with("publish", progress(&published, None, &targets))
    })?;
    if args.dry_run {
        let check = execute(
            OcCommand::Auth(AuthCommand::Check {
                for_operation: Some(crate::flows::op(&crate::REGISTRY, crate::flows::PUBLISH).name.clone()),
            }),
            output,
        )
        .map_err(|failure| failure.with("publish", progress(&published, None, &targets)))?;
        let mut document = progress(&published, None, &targets);
        document["auth"] = check;
        return Ok(document);
    }
    for (index, (name, file)) in built.into_iter().enumerate() {
        let result = execute(
            OcCommand::Place(PlaceCommand::Publish(crate::flows::publish::Args {
                file,
                universe: env.universe.to_string(),
                place: env.places[&name].to_string(),
                version_type: match env.version_type {
                    VersionType::Saved => crate::flows::publish::VersionType::Saved,
                    VersionType::Published => crate::flows::publish::VersionType::Published,
                },
            })),
            output,
        );
        match result {
            Ok(mut receipt) => {
                receipt["name"] = json!(name);
                published.push(receipt);
            }
            Err(failure) => {
                return Err(failure.with("publish", progress(&published, Some(&name), &targets[index + 1..])));
            }
        }
    }
    Ok(progress(&published, None, &[]))
}

fn effective_profile(explicit: Option<&str>, variable: Option<&str>, key: bool, env: Option<&str>) -> Option<String> {
    explicit.or(variable).or_else(|| if key { None } else { env }).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_profile_is_below_existing_explicit_credentials() {
        assert_eq!(effective_profile(Some("flag"), Some("variable"), true, Some("env")), Some("flag".into()));
        assert_eq!(effective_profile(None, Some("variable"), true, Some("env")), Some("variable".into()));
        assert_eq!(effective_profile(None, None, true, Some("env")), None);
        assert_eq!(effective_profile(None, None, false, Some("env")), Some("env".into()));
        assert_eq!(effective_profile(None, None, false, None), None);
    }
}
