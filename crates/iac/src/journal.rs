//! `.crucible/journal/<env>.json` holds the creates `provision` has started and not yet recorded in the lock, with the
//! universe they were sent to, the display name each one sent and the IDs its run knew before it. An intent the lock
//! does not resolve means a create's outcome is unknown, and the resource may exist under any name with an ID the
//! intent's run did not know.

use crate::model::Kind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU64;
use std::path::PathBuf;

pub const FORMAT: u32 = 1;

pub fn path(env: &str) -> PathBuf {
    PathBuf::from(".crucible").join("journal").join(format!("{env}.json"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    version: u32,
    /// The universe the creates were sent to.
    pub universe: NonZeroU64,
    pub intents: Vec<Intent>,
}

/// Fields this crucible does not know are ignored, so a later one can add some without a format change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub kind: Kind,
    pub key: String,
    /// The display name the create sent.
    pub name: String,
    /// The IDs of the kind the run knew before the create: its list and the creates it had answers for. The adopt check
    /// after an unknown outcome takes any other ID the lock does not hold, so a create renamed on Roblox after its
    /// answer was lost is still found. An intent written without it, by hand, is sought by its sent name instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<BTreeSet<NonZeroU64>>,
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
    pub fn new(universe: NonZeroU64) -> Self {
        Self { version: FORMAT, universe, intents: vec![] }
    }

    pub fn render(&self) -> String {
        serde_json::to_string_pretty(self).expect("a journal serializes") + "\n"
    }

    pub fn get(&self, kind: Kind, key: &str) -> Option<&Intent> {
        self.intents.iter().find(|intent| intent.kind == kind && intent.key == key)
    }

    pub fn remove(&mut self, kind: Kind, key: &str) -> Option<Intent> {
        let at = self.intents.iter().position(|intent| intent.kind == kind && intent.key == key)?;
        Some(self.intents.remove(at))
    }

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
        for (at, intent) in journal.intents.iter().enumerate() {
            if journal.intents[..at].iter().any(|other| other.kind == intent.kind && other.key == intent.key) {
                return Err(error(format!("holds two intents for {} {}", intent.kind.noun(), intent.key)));
            }
        }
        Ok(journal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_universe_and_each_intent_with_the_name_it_sent() {
        let journal = Journal::parse(
            "staging",
            r#"{"version":1,"universe":10772177432,"intents":[{"kind":"badge","key":"first-delivery","name":"First delivery","sent":true}]}"#,
        )
        .unwrap();
        assert_eq!(journal.universe.get(), 10772177432);
        assert_eq!(
            journal.intents,
            [Intent { kind: Kind::Badge, key: "first-delivery".into(), name: "First delivery".into(), before: None }]
        );
    }

    #[test]
    fn a_rendered_journal_reads_back_the_same() {
        let mut journal = Journal::new(NonZeroU64::new(5).unwrap());
        journal.intents.push(Intent {
            kind: Kind::Pass,
            key: "vip".into(),
            name: "VIP".into(),
            before: Some([3, 1].into_iter().map(|id| NonZeroU64::new(id).unwrap()).collect()),
        });
        let text = journal.render();
        assert!(text.contains("\"before\": [\n        1,\n        3\n      ]"), "{text}");
        assert_eq!(Journal::parse("staging", &text).unwrap(), journal);
        assert_eq!(journal.remove(Kind::Pass, "vip").map(|intent| intent.name), Some("VIP".into()));
        assert_eq!(journal.get(Kind::Pass, "vip"), None);
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
        for text in [
            r#"{"version":1,"universe":1,"intents":[{"kind":"place","key":"start","name":"Start"}]}"#,
            r#"{"version":1,"universe":1,"intents":[{"kind":"pass","key":"vip"}]}"#,
            r#"{"version":1,"intents":[]}"#,
            r#"{"version":1,"universe":0,"intents":[]}"#,
            "",
        ] {
            assert!(Journal::parse("staging", text).is_err(), "{text}");
        }
        let twice = r#"{"version":1,"universe":1,"intents":[{"kind":"pass","key":"vip","name":"VIP"},{"kind":"pass","key":"vip","name":"Gold"}]}"#;
        assert!(Journal::parse("staging", twice).unwrap_err().message.ends_with("holds two intents for pass vip"));
    }
}
