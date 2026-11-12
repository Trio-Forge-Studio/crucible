mod build;
mod call;
mod child;
mod cli;
mod create;
mod discover;
mod docs;
mod exec;
mod flows;
mod output;
mod plan;
mod provision;
mod publish;
mod reactive_packages;
mod upgrade;
mod whoami;

use crate::call::{Call, Source};
use crate::cli::{AssetCommand, Cli, Command, LuauCommand, Oc, OcCommand, PlaceCommand};
use crate::exec::Executor;
use crate::output::Output;
use clap::Parser;
use crucible_oc::auth;
use crucible_oc::outcome::{Acceptance, Failure};
use crucible_oc::transport::Timing;
use oc_registry::{Auth, Method, OpRef, Registry, Resolved};
use serde_json::{Value, json};
use std::process::ExitCode;
use std::sync::LazyLock;
use std::time::Duration;

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../oc-registry/spec/registry.json")).expect("embedded registry is valid")
});
fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(Cli { command: Command::Adopt(args) }) => return provision::adopt_main(args),
        Ok(Cli { command: Command::Build(args) }) => return build::main(args),
        Ok(Cli { command: Command::Create(args) }) => return create::main(args),
        Ok(Cli { command: Command::Docs { topic, plain } }) => return docs::run(topic.as_deref(), plain),
        Ok(Cli { command: Command::Oc(cli) }) => cli,
        Ok(Cli { command: Command::Plan(args) }) => return plan::main(args),
        Ok(Cli { command: Command::Provision(args) }) => return provision::main(args),
        Ok(Cli { command: Command::Publish(args) }) => return publish::main(args),
        Ok(Cli { command: Command::Upgrade(args) }) => return upgrade::main(args),
        Ok(Cli { command: Command::Whoami(args) }) => return whoami::main(args),
        Ok(Cli { command: Command::Compile { tsconfig, project, include } }) => {
            return build::compile_main(tsconfig, project, include);
        }
        Ok(Cli { command: Command::RojoBuild { project, output } }) => return build::rojo_main(project, output),
        Ok(Cli { command: Command::UpgradeApply { from, crucible, runtime } }) => {
            return upgrade::apply_main(from, crucible, runtime);
        }
        Err(error) => {
            if !matches!(
                std::env::args_os().nth(1).as_deref().and_then(std::ffi::OsStr::to_str),
                Some("oc" | "publish" | "whoami")
            ) {
                error.exit();
            }
            let output = Output::new(false);
            if !error.use_stderr() {
                return if output.text(&error.to_string()).is_ok() { ExitCode::SUCCESS } else { ExitCode::from(7) };
            }
            let failure = Failure::usage(error.kind().to_string());
            let _ = output.document(&failure.document(), true);
            return ExitCode::from(2);
        }
    };
    let mut output = Output::new(cli.verbose);
    let acceptance = std::cell::RefCell::new(Acceptance::default());
    let result = run(cli, &mut output, &acceptance);
    let (code, document) = match acceptance.borrow().finish(result) {
        Ok(v) => (0, v),
        Err(f) => (f.code, f.document()),
    };
    if output.document(&document, code != 0).is_err() {
        if code != 0 {
            return ExitCode::from(code);
        }
        let failure =
            acceptance.borrow().finish::<Value>(Err(Failure::unavailable("cannot write stdout"))).unwrap_err();
        return ExitCode::from(failure.code);
    }
    ExitCode::from(code)
}
fn run(cli: Oc, output: &mut Output, acceptance: &std::cell::RefCell<Acceptance>) -> Result<Value, Failure> {
    let registry = &*REGISTRY;
    match &cli.command {
        OcCommand::Find { query, limit } => return Ok(json!(registry.find(&query.join(" "), *limit))),
        OcCommand::Describe { operation } => {
            return Ok(discover::describe(registry, op_ref(registry, operation)?.0.op));
        }
        _ => {}
    }
    let prepared = if let OcCommand::Call(args) = &cli.command {
        let (resolved, used) = op_ref(registry, &args.words)?;
        let parts = call::parse_pairs(&args.parts)?.into_iter().map(|(k, v)| (k, Source::parse(&v))).collect();
        let body = call::read_body(resolved.op, args.body.as_deref().map(Source::parse), parts)?;
        if args.all && resolved.op.pagination.is_none() {
            return Err(Failure::usage("operation does not declare pagination"));
        }
        Some(Call::new(resolved.op, resolved.preset, call::parse_pairs(&args.words[used..])?, body)?)
    } else {
        None
    };
    let primary = if let Some(call) = &prepared {
        call.op()
    } else {
        match &cli.command {
            OcCommand::Asset(AssetCommand::Upload(args)) => flows::upload::primary(registry, args),
            OcCommand::Asset(AssetCommand::Download(args)) => flows::download::primary(registry, args),
            OcCommand::Place(PlaceCommand::Publish(_)) => flows::op(registry, flows::PUBLISH),
            OcCommand::Luau(LuauCommand::Run(args)) => flows::luau::primary(registry, args),
            _ => unreachable!("discovery returned before execution and calls are prepared"),
        }
    };
    let mut source = None;
    let result = (|| {
        let key = if matches!(primary.auth, Auth::None) {
            None
        } else {
            let root =
                std::env::current_dir().map_err(|_| Failure::unavailable("cannot read the current directory"))?;
            let (found, key) = load_key(&root)?;
            source = Some(found);
            output.redact_also(&key);
            Some(key)
        };
        let executor = Executor::new(
            registry,
            std::env::var("CRUCIBLE_OC_ORIGIN").ok(),
            key.as_ref(),
            Timing {
                deadline: Duration::from_secs_f64(cli.deadline),
                poll_interval: Duration::from_secs_f64(cli.poll_interval),
                retry_delay: cli.retry_delay.map(Duration::from_secs_f64),
            },
            output,
            acceptance,
        )?;
        match cli.command {
            OcCommand::Asset(AssetCommand::Upload(args)) => flows::upload::run(&executor, args),
            OcCommand::Asset(AssetCommand::Download(args)) => flows::download::run(&executor, args),
            OcCommand::Place(PlaceCommand::Publish(args)) => flows::publish::run(&executor, args),
            OcCommand::Luau(LuauCommand::Run(args)) => flows::luau::run(&executor, args),
            OcCommand::Call(args) => {
                let call = prepared.expect("call arguments were validated");
                if args.all {
                    executor.send_all(call, args.max_items, args.max_pages)
                } else {
                    let reply = executor.send(&call)?;
                    if !args.no_wait && call.op().completion.is_some() {
                        let mut first = reply.json()?;
                        if call.op().method == Method::Get && first.is_object() && !first["path"].is_string() {
                            let root = oc_registry::path::Template::parse(&call.op().path).root_len();
                            first["path"] = json!(
                                call.path().trim_start_matches('/').split('/').skip(root).collect::<Vec<_>>().join("/")
                            );
                        }
                        if let Some(path) = first["path"].as_str()
                            && let Ok((reader, captures)) = registry.reader_of(call.op(), path)
                        {
                            let poll = Call::new(reader, exec::decode_captures(captures.0), vec![], None)?;
                            executor.resume(format!("crucible oc call GET {}", poll.path()));
                        }
                        executor.finish(call.op(), first)
                    } else {
                        reply.json()
                    }
                }
            }
            _ => unreachable!("discovery returned before execution"),
        }
    })();
    result.map_err(|failure| auth::decorate(failure, &primary.auth, source.as_ref()))
}
fn load_key(root: &std::path::Path) -> Result<(auth::KeySource, auth::Secret), Failure> {
    auth::load(std::env::var_os(auth::KEY), || dot_env(root))
}
fn dot_env(root: &std::path::Path) -> Result<Option<String>, Failure> {
    let bytes = match std::fs::read(root.join(".env")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Failure::usage("cannot read .env")),
    };
    // dotenvy strips a UTF-8 BOM only when it loads the file into the environment, which the CLI never does.
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let mut found = None;
    for entry in dotenvy::from_read_iter(text) {
        let (name, value) = entry.map_err(|error| match error {
            dotenvy::Error::Io(_) => Failure::usage("cannot read .env"),
            _ => Failure::usage("cannot parse .env"),
        })?;
        if name == auth::KEY && found.is_none() {
            found = Some(value);
        }
    }
    Ok(found)
}
fn op_ref<'r>(registry: &'r Registry, words: &[String]) -> Result<(Resolved<'r>, usize), Failure> {
    let first = words.first().ok_or_else(|| Failure::usage("an operation name or METHOD and URL is required"))?;
    let method = match first.as_str() {
        "GET" => Some(Method::Get),
        "POST" => Some(Method::Post),
        "PUT" => Some(Method::Put),
        "PATCH" => Some(Method::Patch),
        "DELETE" => Some(Method::Delete),
        _ => None,
    };
    let (reference, used) = if let Some(method) = method {
        let text = words.get(1).ok_or_else(|| Failure::usage("METHOD requires a URL path"))?;
        let url = url::Url::parse(&if text.starts_with('/') {
            format!("https://apis.roblox.com{text}")
        } else {
            text.clone()
        })
        .map_err(|_| Failure::usage("invalid URL"))?;
        (
            OpRef::Url {
                method,
                path: url.path().into(),
                query: url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect(),
            },
            2,
        )
    } else {
        (OpRef::Name(first.clone()), 1)
    };
    let mut resolved = registry.resolve(&reference).map_err(|error| match error {
        oc_registry::ResolveError::Unknown { suggestions } => {
            Failure::usage(format!("unknown operation {}", words[..used].join(" ")))
                .with("suggestions", json!(suggestions))
        }
        oc_registry::ResolveError::Ambiguous { names } => {
            Failure::usage("ambiguous operation").with("suggestions", json!(names))
        }
    })?;
    if matches!(reference, OpRef::Url { .. }) {
        for (name, value) in &mut resolved.preset {
            if resolved.op.params.iter().any(|p| p.location == oc_registry::Location::Path && p.name == *name) {
                *value = percent_encoding::percent_decode_str(value).decode_utf8_lossy().into_owned();
            }
        }
    }
    Ok((resolved, used))
}
