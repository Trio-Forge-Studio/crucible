//! `crucible.lock` records what Roblox assigned, by environment, then kind, then key, and the universe each
//! environment's entries belong to. This module reads it, edits it and writes it back whole. A lock that fails to parse
//! or has a newer format is an error and never reads as empty, because an empty lock means "create everything".

use crate::model::Kind;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;
use toml::Spanned;

pub const FILE: &str = "crucible.lock";
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    version: u32,
    #[serde(default)]
    env: BTreeMap<String, EnvLock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvLock {
    /// The universe the entries were provisioned in. Their IDs mean nothing in another one.
    universe: Spanned<NonZeroU64>,
    #[serde(skip)]
    universe_line: usize,
    pub experience: Option<Applied>,
    #[serde(default)]
    products: BTreeMap<String, Entry>,
    #[serde(default)]
    passes: BTreeMap<String, Entry>,
    #[serde(default)]
    badges: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: NonZeroU64,
    pub fingerprint: String,
    /// The content hash of the icon last uploaded.
    pub icon: Option<String>,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Managed,
    Retired,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LockError {
    Parse { line: Option<usize>, message: String },
    Newer { format: u64 },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line: Some(line), message } => write!(f, "{FILE}:{line}: {message}"),
            Self::Parse { line: None, message } => write!(f, "{FILE}: {message}"),
            Self::Newer { format } => write!(
                f,
                "{FILE} has format {format}, and this crucible reads format {FORMAT}. Upgrade crucible before using this lock"
            ),
        }
    }
}

impl Lock {
    /// The lock of a project nothing was provisioned for yet.
    pub fn empty() -> Self {
        Self { version: FORMAT, env: BTreeMap::new() }
    }

    pub fn parse(text: &str) -> Result<Self, LockError> {
        #[derive(Deserialize)]
        struct Format {
            version: Option<toml::Value>,
        }
        if let Ok(Format { version: Some(toml::Value::Integer(format)) }) = toml::from_str(text)
            && let Ok(format) = u64::try_from(format)
            && format > u64::from(FORMAT)
        {
            return Err(LockError::Newer { format });
        }
        let line = |offset: usize| text[..offset].matches('\n').count() + 1;
        let mut lock: Self = toml::from_str(text).map_err(|error| LockError::Parse {
            line: error.span().map(|span| line(span.start)),
            message: error.message().trim_end().to_owned(),
        })?;
        if lock.version != FORMAT {
            return Err(LockError::Parse { line: None, message: format!("version must be {FORMAT}") });
        }
        for env in lock.env.values_mut() {
            env.universe_line = line(env.universe.span().start);
        }
        Ok(lock)
    }

    pub fn env(&self, name: &str) -> Option<&EnvLock> {
        self.env.get(name)
    }

    /// Every environment the lock records, declared in `crucible.toml` or not.
    pub fn envs(&self) -> &BTreeMap<String, EnvLock> {
        &self.env
    }

    /// The environment's tables, made empty for `universe` when the lock has none.
    pub fn env_mut(&mut self, name: &str, universe: NonZeroU64) -> &mut EnvLock {
        self.env.entry(name.to_owned()).or_insert_with(|| EnvLock {
            universe: Spanned::new(0..0, universe),
            universe_line: 0,
            experience: None,
            products: BTreeMap::new(),
            passes: BTreeMap::new(),
            badges: BTreeMap::new(),
        })
    }

    /// The whole file, in one canonical layout, so a provision's change to it reads as a small diff.
    pub fn render(&self) -> String {
        let mut text = format!("version = {FORMAT}\n");
        for (name, env) in &self.env {
            let name = bare(name);
            text += &format!("\n[env.{name}]\nuniverse = {}\n", env.universe());
            if let Some(applied) = &env.experience {
                text += &format!("\n[env.{name}.experience]\nfingerprint = {}\n", quote(&applied.fingerprint));
            }
            for kind in Kind::ALL {
                for (key, entry) in env.entries(kind) {
                    text += &format!(
                        "\n[env.{name}.{}.{}]\nid = {}\nfingerprint = {}\n",
                        kind.table(),
                        bare(key),
                        entry.id,
                        quote(&entry.fingerprint)
                    );
                    if let Some(icon) = &entry.icon {
                        text += &format!("icon = {}\n", quote(icon));
                    }
                    let status = match entry.status {
                        Status::Managed => "managed",
                        Status::Retired => "retired",
                    };
                    text += &format!("status = \"{status}\"\n");
                }
            }
        }
        text
    }
}

fn quote(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

/// A key as a bare TOML key when it can be one. Keys `crucible.toml` accepts always can.
fn bare(key: &str) -> String {
    if !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        key.to_owned()
    } else {
        quote(key)
    }
}

impl EnvLock {
    pub fn universe(&self) -> NonZeroU64 {
        *self.universe.get_ref()
    }

    /// The line of `universe` in `crucible.lock`.
    pub fn universe_line(&self) -> usize {
        self.universe_line
    }

    pub fn entries(&self, kind: Kind) -> &BTreeMap<String, Entry> {
        match kind {
            Kind::Product => &self.products,
            Kind::Pass => &self.passes,
            Kind::Badge => &self.badges,
        }
    }

    pub fn entries_mut(&mut self, kind: Kind) -> &mut BTreeMap<String, Entry> {
        match kind {
            Kind::Product => &mut self.products,
            Kind::Pass => &mut self.passes,
            Kind::Badge => &mut self.badges,
        }
    }

    /// How many products, passes and badges the environment holds IDs for, retired ones included.
    pub fn resources(&self) -> usize {
        Kind::ALL.into_iter().map(|kind| self.entries(kind).len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
version = 1

[env.staging]
universe = 10772177432

[env.staging.experience]
fingerprint = "sha256:aa"

[env.staging.products.coin-pack]
id = 3301234567
fingerprint = "sha256:bb"
icon = "sha256:cc"
status = "managed"

[env.staging.badges.first-delivery]
id = 2124000000000000
fingerprint = "sha256:dd"
status = "retired"
"#;

    #[test]
    fn reads_entries_by_environment_kind_and_key() {
        let lock = Lock::parse(LOCK).unwrap();
        let staging = lock.env("staging").unwrap();
        assert_eq!((staging.universe().get(), staging.universe_line()), (10772177432, 5));
        assert_eq!(staging.experience, Some(Applied { fingerprint: "sha256:aa".into() }));
        let coin = &staging.entries(Kind::Product)["coin-pack"];
        assert_eq!(
            (coin.id.get(), coin.icon.as_deref(), coin.status),
            (3301234567, Some("sha256:cc"), Status::Managed)
        );
        assert_eq!(staging.entries(Kind::Badge)["first-delivery"].id.get(), 2124000000000000, "IDs stay exact");
        assert!(staging.entries(Kind::Pass).is_empty());
        assert_eq!(lock.env("production"), None);
    }

    #[test]
    fn a_rendered_lock_reads_back_the_same() {
        let lock = Lock::parse(LOCK).unwrap();
        let text = lock.render();
        assert_eq!(Lock::parse(&text).unwrap().render(), text);
        assert_eq!(
            Lock::parse(&text).unwrap().env("staging").unwrap().entries(Kind::Badge),
            lock.env("staging").unwrap().entries(Kind::Badge)
        );
        let mut edited = Lock::empty();
        let universe = NonZeroU64::new(7).unwrap();
        let entry = Entry {
            id: NonZeroU64::new(u64::from(u32::MAX) * 4).unwrap(),
            fingerprint: "sha256:\"x\"".into(),
            icon: None,
            status: Status::Retired,
        };
        edited.env_mut("Stage_2", universe).entries_mut(Kind::Pass).insert("vip".into(), entry.clone());
        let read = Lock::parse(&edited.render()).unwrap();
        assert_eq!(read.env("Stage_2").unwrap().universe(), universe);
        assert_eq!(read.env("Stage_2").unwrap().entries(Kind::Pass)["vip"], entry);
        assert_eq!(Lock::parse(&Lock::empty().render()).unwrap(), Lock::empty());
    }

    #[test]
    fn a_newer_format_stops_before_its_new_keys_are_read() {
        let error = Lock::parse("version = 2\n[env.staging.places.start]\nid = 1\n").unwrap_err();
        assert_eq!(error, LockError::Newer { format: 2 });
        assert_eq!(
            error.to_string(),
            "crucible.lock has format 2, and this crucible reads format 1. Upgrade crucible before using this lock"
        );
    }

    #[test]
    fn a_malformed_lock_is_an_error_never_an_empty_lock() {
        for (text, line) in [
            (
                "version = 1\n[env.staging]\nuniverse = 1\n[env.staging.products.coin-pack]\nid = 1.0\nfingerprint = \"x\"\nstatus = \"managed\"\n",
                Some(5),
            ),
            (
                "version = 1\n[env.staging]\nuniverse = 1\n[env.staging.products.coin-pack]\nid = 1\nfingerprint = \"x\"\nstatus = \"gone\"\n",
                Some(7),
            ),
            (
                "version = 1\n[env.staging]\nuniverse = 1\n[env.staging.products.coin-pack]\nid = 0\nfingerprint = \"x\"\nstatus = \"managed\"\n",
                Some(5),
            ),
            ("version = 1\n[env.staging]\nuniverse = 1\nname = \"x\"\n", Some(4)),
            ("version = 1\n[env.staging]\nuniverse = 0\n", Some(3)),
            (
                "version = 1\n[env.staging.products.coin-pack]\nid = 1\nfingerprint = \"x\"\nstatus = \"managed\"\n",
                Some(2),
            ),
            ("[env.staging]\n", Some(1)),
            ("version = 0\n", None),
            ("version = \n", Some(1)),
        ] {
            match Lock::parse(text) {
                Err(LockError::Parse { line: found, .. }) => assert_eq!(found, line, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }
}
