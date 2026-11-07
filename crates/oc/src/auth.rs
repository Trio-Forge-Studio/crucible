use crate::outcome::Failure;
use crucible_oc::Auth;
use serde::Deserialize;
use serde_json::json;
use std::ffi::OsString;

pub const KEY: &str = "CRUCIBLE_OC_API_KEY";

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
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn from_string(value: String) -> Self {
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

pub fn load(
    env: Option<OsString>,
    dot_env: impl FnOnce() -> Result<Option<String>, Failure>,
) -> Result<(KeySource, Secret), Failure> {
    let (source, value) = match env.map(OsString::into_string) {
        Some(Ok(value)) => (KeySource::Environment, value),
        Some(Err(_)) => return Err(Failure::new(3, "auth", format!("{KEY} is not valid Unicode"))),
        None => match dot_env()? {
            Some(value) => (KeySource::DotEnv, value),
            None => return Err(Failure::new(3, "auth", format!("set {KEY} in the environment or in .env"))),
        },
    };
    let value = value.trim();
    if value.is_empty() {
        let place = if source == KeySource::DotEnv { ".env" } else { "the environment" };
        return Err(Failure::new(3, "auth", format!("{KEY} in {place} is empty")));
    }
    Ok((source, Secret::from_string(value.into())))
}
