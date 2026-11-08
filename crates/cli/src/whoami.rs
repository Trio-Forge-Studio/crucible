use crate::auth;
use crate::cli::{DEFAULT_DEADLINE, DEFAULT_POLL_INTERVAL};
use crate::exec::{Executor, Timing};
use crate::outcome::{Acceptance, Failure};
use crate::output::Output;
use serde_json::Value;
use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

#[derive(clap::Args)]
pub struct Args {
    /// Fail with exit 3 unless the key has the scopes this operation requires.
    #[arg(long = "for", value_name = "OPERATION")]
    pub for_operation: Option<String>,
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
        return ExitCode::from(7);
    }
    ExitCode::from(code)
}

fn run(root: &Path, args: Args, output: &mut Output) -> Result<Value, Failure> {
    let registry = &*crate::REGISTRY;
    let required = args
        .for_operation
        .as_ref()
        .map(|name| crate::op_ref(registry, std::slice::from_ref(name)).map(|(resolved, _)| resolved.op))
        .transpose()?;
    let primary = required.unwrap_or_else(|| registry.op("api-keys.introspect").expect("introspection is registered"));
    let (source, key) = auth::load(root).map_err(|failure| auth::decorate(failure, primary, None))?;
    output.redact_also(&key);
    let acceptance = RefCell::new(Acceptance::default());
    let result = (|| {
        let executor = Executor::new(
            registry,
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
        auth::check(&executor, &source, required)
    })();
    acceptance.into_inner().finish(result).map_err(|failure| auth::decorate(failure, primary, Some(&source)))
}
