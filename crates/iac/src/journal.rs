//! `.crucible/journal/<env>.json` holds the creates `provision` has started and not yet recorded in the lock. This
//! module reads it for `plan`. An intent the lock does not resolve means a create's outcome is unknown.

use crate::model::Kind;
use serde::Deserialize;
use std::fmt;
use std::path::PathBuf;

pub const FORMAT: u32 = 1;

pub fn path(env: &str) -> PathBuf {
    PathBuf::from(".crucible").join("journal").join(format!("{env}.json"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Journal {
    version: u32,
    pub intents: Vec<Intent>,
}

/// Other fields belong to `provision`, which reads them, so the plan ignores them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Intent {
    pub kind: Kind,
    pub key: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct JournalError {
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl Journal {
    pub fn parse(env: &str, text: &str) -> Result<Self, JournalError> {
        let error = |message: String| JournalError { path: path(env), message };
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| error(e.to_string()))?;
        if let Some(format) = value["version"].as_u64().filter(|format| *format > u64::from(FORMAT)) {
            return Err(error(format!(
                "has format {format}, and this crucible reads format {FORMAT}. Upgrade crucible before provisioning"
            )));
        }
        let journal: Self = serde_json::from_value(value).map_err(|e| error(e.to_string()))?;
        if journal.version != FORMAT {
            return Err(error(format!("has format {}, which no crucible writes", journal.version)));
        }
        Ok(journal)
    }

    pub fn intends(&self, kind: Kind, key: &str) -> bool {
        self.intents.iter().any(|intent| intent.kind == kind && intent.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_intents_and_ignores_the_fields_provision_keeps() {
        let journal = Journal::parse(
            "staging",
            r#"{"version":1,"intents":[{"kind":"badge","key":"first-delivery","name":"First delivery","sent":true}]}"#,
        )
        .unwrap();
        assert!(journal.intends(Kind::Badge, "first-delivery"));
        assert!(!journal.intends(Kind::Product, "first-delivery"));
    }

    #[test]
    fn a_newer_or_malformed_journal_is_an_error() {
        let newer = Journal::parse("staging", r#"{"version":2,"intents":{"staging":[]}}"#).unwrap_err();
        assert_eq!(
            newer.to_string(),
            format!(
                "{}: has format 2, and this crucible reads format 1. Upgrade crucible before provisioning",
                path("staging").display()
            )
        );
        assert!(Journal::parse("staging", r#"{"version":1,"intents":[{"kind":"place","key":"start"}]}"#).is_err());
        assert!(Journal::parse("staging", "").is_err());
    }
}
