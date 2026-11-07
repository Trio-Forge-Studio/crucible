use serde::{Deserialize, Serialize};

/// What a response body holds. `S` describes a JSON body, such as the registry's schema outline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum Payload<S> {
    Empty,
    Json { shape: S },
    Text,
    Binary,
}

impl<S> Payload<S> {
    pub fn kind(&self) -> Payload<()> {
        match self {
            Self::Empty => Payload::Empty,
            Self::Json { .. } => Payload::Json { shape: () },
            Self::Text => Payload::Text,
            Self::Binary => Payload::Binary,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    ApiKey { scopes: Option<Vec<Scope>> },
    BodyKey { field: String },
    None,
}

impl Auth {
    pub fn required_scopes(&self) -> Option<&[Scope]> {
        match self {
            Self::ApiKey { scopes } => scopes.as_deref(),
            Self::BodyKey { .. } | Self::None => Some(&[]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Scope {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    Read,
    Idempotent,
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub statuses: Vec<StatusClass>,
    pub attempts: u8,
    pub delay_seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StatusClass {
    Exact(u16),
    Class(ClassName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassName {
    #[serde(rename = "5xx")]
    ServerError,
}

impl StatusClass {
    pub fn matches(self, status: u16) -> bool {
        match self {
            StatusClass::Exact(code) => code == status,
            StatusClass::Class(ClassName::ServerError) => (500..600).contains(&status),
        }
    }
}
