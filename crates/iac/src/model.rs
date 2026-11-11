//! The experience a project declares, independent of the file it came from. `Experience::new` checks the rules that
//! span tables, and `Experience::resolve` applies one environment's overlays.

use crucible_project::experience::{Badge, Located, Priced, Settings};
use crucible_project::{Env, FILE, Manifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Product,
    Pass,
    Badge,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Product, Kind::Pass, Kind::Badge];

    pub fn table(self) -> &'static str {
        match self {
            Kind::Product => "products",
            Kind::Pass => "passes",
            Kind::Badge => "badges",
        }
    }

    /// The object of `crucible:experience` that holds this kind's handles.
    pub fn handles(self) -> &'static str {
        match self {
            Kind::Product => "Products",
            Kind::Pass => "Passes",
            Kind::Badge => "Badges",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Kind::Product => "product",
            Kind::Pass => "pass",
            Kind::Badge => "badge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Managed,
    Retired,
    Untrack,
}

/// What a provision writes for a resource, after the environment's overlay. The lock records its fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Fields {
    Priced { name: String, description: String, price: u64 },
    Badge { name: String, description: String, enabled: bool },
}

impl Fields {
    pub fn fingerprint(&self) -> String {
        fingerprint(self)
    }

    /// The display name, which provision's adopt check matches.
    pub fn name(&self) -> &str {
        match self {
            Fields::Priced { name, .. } | Fields::Badge { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub kind: Kind,
    pub key: String,
    pub line: usize,
    pub lifecycle: Lifecycle,
    pub moved_from: Option<String>,
    pub icon: Option<PathBuf>,
    /// `None` for a retired or untracked resource whose table no longer lists its fields.
    pub fields: Option<Fields>,
}

impl Resource {
    /// `coin-pack` becomes `Products.CoinPack`.
    pub fn handle(&self) -> String {
        format!("{}.{}", self.kind.handles(), handle(&self.key))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedSettings {
    pub private_server_price: Option<u32>,
    pub voice_chat: Option<bool>,
}

impl ResolvedSettings {
    pub fn fingerprint(&self) -> String {
        fingerprint(self)
    }
}

/// A rule the files break. `line` is a line of `file`, when one table holds the cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub key: String,
    pub message: String,
}

impl Problem {
    pub fn at(line: usize, key: impl Into<String>, message: impl Into<String>) -> Self {
        Self { file: FILE.into(), line: Some(line), key: key.into(), message: message.into() }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}: {}", self.file, self.key, self.message),
            None => write!(f, "{}: {}: {}", self.file, self.key, self.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Experience {
    settings: Option<ResolvedSettings>,
    resources: Vec<(Resource, Tables)>,
}

/// A resource's fields before an overlay, kept so each environment resolves its own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tables {
    Priced(Priced),
    Badge(Badge),
}

impl Experience {
    pub fn new(manifest: &Manifest) -> Result<Self, Vec<Problem>> {
        let mut problems = Vec::new();
        let declared = &manifest.experience;
        let mut resources = Vec::new();
        for (kind, tables) in [
            (Kind::Product, priced(&declared.products)),
            (Kind::Pass, priced(&declared.passes)),
            (
                Kind::Badge,
                declared
                    .badges
                    .iter()
                    .map(|(key, table)| (key, table.line, Tables::Badge(table.value.clone())))
                    .collect(),
            ),
        ] {
            let mut handles: BTreeMap<String, &str> = BTreeMap::new();
            let mut moves: BTreeMap<&str, &str> = BTreeMap::new();
            // No overlay sets a name, so two names that differ here differ in every environment.
            let mut names: BTreeMap<&str, &str> = BTreeMap::new();
            for (key, line, table) in &tables {
                let at = |field: &str| format!("{}.{key}{field}", kind.table());
                if !is_key(key) {
                    problems.push(Problem::at(
                        *line,
                        at(""),
                        "a key uses lowercase letters, digits and '-', and starts with a letter",
                    ));
                    continue;
                }
                if let Some(other) = handles.insert(handle(key), key) {
                    problems.push(Problem::at(
                        *line,
                        at(""),
                        format!(
                            "{}.{other} and {}.{key} both become the handle {}.{}",
                            kind.table(),
                            kind.table(),
                            kind.handles(),
                            handle(key)
                        ),
                    ));
                }
                let (retired, untrack, moved_from) = table.lifecycle();
                let lifecycle = match (retired, untrack) {
                    (true, true) => {
                        problems.push(Problem::at(*line, at(""), "set retired or untrack, not both"));
                        continue;
                    }
                    (true, false) => Lifecycle::Retired,
                    (false, true) => Lifecycle::Untrack,
                    (false, false) => Lifecycle::Managed,
                };
                if let Some(old) = moved_from {
                    if !is_key(old) || old == *key {
                        problems.push(Problem::at(*line, at(".moved_from"), format!("{old:?} is not another key")));
                    } else if tables.iter().any(|(other, ..)| *other == old) {
                        problems.push(Problem::at(
                            *line,
                            at(".moved_from"),
                            format!("{}.{old} is still declared, so nothing moved from it", kind.table()),
                        ));
                    } else if let Some(first) = moves.insert(old, key) {
                        problems.push(Problem::at(
                            *line,
                            at(".moved_from"),
                            format!("{}.{first} already moves from {old}", kind.table()),
                        ));
                    }
                }
                let price = table.price();
                if price == Some(0) {
                    problems.push(Problem::at(*line, at(".price"), zero_price(kind)));
                }
                if lifecycle == Lifecycle::Managed {
                    match table.name().filter(|name| !name.is_empty()) {
                        None => {
                            problems.push(Problem::at(*line, at(".name"), format!("a {} needs a name", kind.noun())))
                        }
                        Some(name) => {
                            if let Some(other) = names.insert(name, key) {
                                problems.push(Problem::at(
                                    *line,
                                    at(".name"),
                                    format!(
                                        "{table}.{other} and {table}.{key} both have the display name {name:?}. \
                                         Provision adopts an existing resource by its name, so it could not tell \
                                         them apart. Give one of them another name",
                                        table = kind.table()
                                    ),
                                ));
                            }
                        }
                    }
                    if kind != Kind::Badge && price.is_none() {
                        problems.push(Problem::at(
                            *line,
                            at(".price"),
                            format!("a {} needs a price in Robux", kind.noun()),
                        ));
                    }
                }
                let resource = Resource {
                    kind,
                    key: (*key).clone(),
                    line: *line,
                    lifecycle,
                    moved_from: moved_from.cloned(),
                    icon: table.icon().cloned(),
                    fields: None,
                };
                resources.push((resource, table.clone()));
            }
        }
        for (name, env) in &manifest.envs {
            let mut check = |kind: Kind, key: &String, line: usize, price: Option<u64>| {
                let at = format!("env.{name}.{}.{key}", kind.table());
                if !resources.iter().any(|(resource, _)| resource.kind == kind && resource.key == *key) {
                    problems.push(Problem::at(line, at, format!("no {}.{key} is declared", kind.table())));
                } else if price == Some(0) {
                    problems.push(Problem::at(line, format!("{at}.price"), zero_price(kind)));
                }
            };
            for (key, overlay) in &env.overlays.products {
                check(Kind::Product, key, overlay.line, overlay.value.price);
            }
            for (key, overlay) in &env.overlays.passes {
                check(Kind::Pass, key, overlay.line, overlay.value.price);
            }
            for (key, overlay) in &env.overlays.badges {
                check(Kind::Badge, key, overlay.line, None);
            }
        }
        let mut universes: BTreeMap<_, &String> = BTreeMap::new();
        for (name, env) in &manifest.envs {
            if let Some(first) = universes.insert(env.universe, name) {
                problems.push(Problem {
                    file: FILE.into(),
                    line: None,
                    key: format!("env.{name}.universe"),
                    message: format!(
                        "env.{first} and env.{name} both name universe {}. Provision adopts a resource by its name \
                         inside a universe, so each environment would bind the other's resources, and its overlays \
                         would overwrite the other's values. Give each environment its own universe",
                        env.universe
                    ),
                });
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        let settings = declared.settings.as_ref().map(
            |Located { value: Settings { private_server_price, voice_chat }, .. }| ResolvedSettings {
                private_server_price: *private_server_price,
                voice_chat: *voice_chat,
            },
        );
        Ok(Self { settings, resources })
    }

    pub fn settings(&self) -> Option<&ResolvedSettings> {
        self.settings.as_ref()
    }

    pub fn resolve(&self, env: &Env) -> Vec<Resource> {
        self.resources
            .iter()
            .map(|(resource, table)| {
                let mut resource = resource.clone();
                resource.fields = match table {
                    Tables::Priced(base) => {
                        let overlays =
                            if resource.kind == Kind::Product { &env.overlays.products } else { &env.overlays.passes };
                        let price = overlays.get(&resource.key).and_then(|overlay| overlay.value.price).or(base.price);
                        base.name.clone().zip(price).map(|(name, price)| Fields::Priced {
                            name,
                            description: base.description.clone().unwrap_or_default(),
                            price,
                        })
                    }
                    Tables::Badge(base) => {
                        let enabled = env.overlays.badges.get(&resource.key).and_then(|overlay| overlay.value.enabled);
                        base.name.clone().map(|name| Fields::Badge {
                            name,
                            description: base.description.clone().unwrap_or_default(),
                            enabled: enabled.or(base.enabled).unwrap_or(true),
                        })
                    }
                };
                resource
            })
            .collect()
    }
}

impl Tables {
    fn lifecycle(&self) -> (bool, bool, Option<&String>) {
        match self {
            Tables::Priced(t) => (t.retired, t.untrack, t.moved_from.as_ref()),
            Tables::Badge(t) => (t.retired, t.untrack, t.moved_from.as_ref()),
        }
    }

    fn name(&self) -> Option<&str> {
        match self {
            Tables::Priced(t) => t.name.as_deref(),
            Tables::Badge(t) => t.name.as_deref(),
        }
    }

    fn price(&self) -> Option<u64> {
        match self {
            Tables::Priced(t) => t.price,
            Tables::Badge(_) => None,
        }
    }

    fn icon(&self) -> Option<&PathBuf> {
        match self {
            Tables::Priced(t) => t.icon.as_ref(),
            Tables::Badge(t) => t.icon.as_ref(),
        }
    }
}

fn priced(tables: &crucible_project::experience::Tables<Priced>) -> Vec<(&String, usize, Tables)> {
    tables.iter().map(|(key, table)| (key, table.line, Tables::Priced(table.value.clone()))).collect()
}

fn zero_price(kind: Kind) -> String {
    format!("a price of 0 does not take a {} off sale. Set retired = true to take it off sale", kind.noun())
}

pub fn is_key(key: &str) -> bool {
    key.starts_with(|c: char| c.is_ascii_lowercase())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn handle(key: &str) -> String {
    key.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| first.to_ascii_uppercase().to_string() + chars.as_str())
        })
        .collect()
}

fn fingerprint(value: &impl Serialize) -> String {
    let json = serde_json::to_vec(value).expect("the fields serialize");
    format!("sha256:{:x}", Sha256::digest(json))
}

pub fn content_hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
