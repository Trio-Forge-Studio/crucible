use crate::call::Call;
use crate::exec::Executor;
use crate::outcome::Failure;
use crucible_oc::Auth;
use oc_registry::Op;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

const KEY: &str = "CRUCIBLE_OC_API_KEY";

pub struct Secret(String);
#[derive(Deserialize)]
pub struct Introspection {
    pub name: String,
    pub enabled: bool,
    pub expired: bool,
    scopes: Vec<IntrospectedScope>,
}
#[derive(Deserialize)]
struct IntrospectedScope {
    name: String,
    operations: Vec<String>,
}
impl Introspection {
    pub fn parse(value: serde_json::Value) -> Result<Self, Failure> {
        serde_json::from_value(value).map_err(|_| Failure::unavailable("introspection returned an invalid response"))
    }
    pub fn scopes(&self) -> Vec<String> {
        let mut scopes: Vec<_> = self
            .scopes
            .iter()
            .flat_map(|scope| scope.operations.iter().map(|op| format!("{}:{op}", scope.name)))
            .collect();
        scopes.sort();
        scopes.dedup();
        scopes
    }
}
impl Secret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
    pub(crate) fn from_string(value: String) -> Self {
        Self(value)
    }
}
#[derive(Debug, PartialEq)]
pub enum KeySource {
    Environment,
    DotEnv,
}
impl KeySource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Environment => KEY,
            Self::DotEnv => ".env",
        }
    }
}
pub fn scope_metadata(auth: &Auth) -> serde_json::Value {
    let scopes = auth.required_scopes();
    json!({"requiredScopes":scopes,"scopeMetadata":if scopes.is_some() { "declared" } else { "unknown" }})
}
pub fn decorate(mut failure: Failure, auth: &Auth, source: Option<&KeySource>) -> Failure {
    if failure.code == 3 {
        let error = failure.error.as_object_mut().expect("a failure is an object");
        error.entry("status").or_insert(serde_json::Value::Null);
        error.extend(scope_metadata(auth).as_object().expect("scope metadata is an object").clone());
        failure = failure.with("keySource", json!(source.map(KeySource::label)));
    }
    failure
}

pub fn load(root: &Path) -> Result<(KeySource, Secret), Failure> {
    let (source, value) = match std::env::var(KEY) {
        Ok(value) => (KeySource::Environment, value),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(Failure::new(3, "auth", format!("{KEY} is not valid Unicode")));
        }
        Err(std::env::VarError::NotPresent) => match dot_env(root)? {
            Some(value) => (KeySource::DotEnv, value),
            None => {
                return Err(Failure::new(3, "auth", format!("set {KEY} in the environment or in .env")));
            }
        },
    };
    let value = value.trim();
    if value.is_empty() {
        let place = if source == KeySource::DotEnv { ".env" } else { "the environment" };
        return Err(Failure::new(3, "auth", format!("{KEY} in {place} is empty")));
    }
    Ok((source, Secret::from_string(value.into())))
}
fn dot_env(root: &Path) -> Result<Option<String>, Failure> {
    let entries = match dotenvy::from_path_iter(root.join(".env")) {
        Ok(entries) => entries,
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Failure::usage("cannot read .env")),
    };
    let mut found = None;
    for entry in entries {
        let (name, value) = entry.map_err(|error| match error {
            dotenvy::Error::Io(_) => Failure::usage("cannot read .env"),
            _ => Failure::usage("cannot parse .env"),
        })?;
        if name == KEY && found.is_none() {
            found = Some(value);
        }
    }
    Ok(found)
}

pub fn check(executor: &Executor<'_>, source: &KeySource, require: Option<&Op>) -> Result<Value, Failure> {
    let introspect = executor.registry.by_key("POST /api-keys/v1/introspect").expect("introspection is registered");
    let reply = executor.send(&Call::new(introspect, vec![], vec![], None)?)?.json()?;
    let reply = Introspection::parse(reply)?;
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
