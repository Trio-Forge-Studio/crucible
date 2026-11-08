use crate::call::Call;
use crate::cli::{DEFAULT_DEADLINE, DEFAULT_POLL_INTERVAL};
use crate::exec::Executor;
use crate::output::Output;
use crucible_oc::auth;
use crucible_oc::outcome::{Acceptance, Failure};
use crucible_oc::transport::Timing;
use oc_registry::{Auth, Op};
use serde_json::{Value, json};
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
    let (source, key) = crate::load_key(root).map_err(|failure| auth::decorate(failure, &primary.auth, None))?;
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
        check(&executor, &source, required)
    })();
    acceptance.into_inner().finish(result).map_err(|failure| auth::decorate(failure, &primary.auth, Some(&source)))
}

pub fn check(executor: &Executor<'_>, source: &auth::KeySource, require: Option<&Op>) -> Result<Value, Failure> {
    let introspect = executor.registry.by_key("POST /api-keys/v1/introspect").expect("introspection is registered");
    let reply = executor.send(&Call::new(introspect, vec![], vec![], None)?)?.json()?;
    let reply = auth::Introspection::parse(reply)?;
    let scopes = reply.scopes();
    if !reply.enabled || reply.expired {
        return Err(Failure::new(3, "auth", "the key is disabled or expired"));
    }
    if let Some(op) = require
        && let Auth::ApiKey { scopes: Some(required) } = &op.auth
    {
        let missing: Vec<_> =
            required.iter().filter(|scope| !scopes.contains(&scope.name)).map(|scope| scope.name.as_str()).collect();
        if !missing.is_empty() {
            return Err(Failure::new(3, "auth", format!("the selected key lacks {}", missing.join(", "))));
        }
    }
    Ok(
        json!({"keySource":source.label(),"name":reply.name,"enabled":reply.enabled,"expired":reply.expired,"scopes":scopes}),
    )
}
