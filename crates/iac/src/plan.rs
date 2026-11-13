//! What `crucible provision <env>` does, from the config, the lock, the journal and, when provision has listed them,
//! the resources Roblox holds. It makes no network call. Without what Roblox holds, as `crucible plan` runs it, a create
//! is always preceded by provision's adopt check and an update is a change of the inputs the lock fingerprinted. With
//! it, the adopt check is decided: one candidate is adopted, none is created, and two stop the plan, as does a resource
//! new since the key's intent that carries another name than the declared one. Every intent in the
//! journal shows on the step of its resource, or stops the plan with a problem that names it, because an intent nobody
//! sees is a create nobody resolves.

use crate::journal::{self, Intent, Journal};
use crate::lock::{self, Entry, EnvLock, Status};
use crate::model::{Experience, Fields, Kind, Lifecycle, Problem, ResolvedSettings, Resource, content_hash};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::Path;

/// Roblox makes this many badges a day free in each experience and charges 100 Robux for each one after that.
pub const FREE_BADGES_PER_DAY: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// No ID in the lock: provision adopts the one resource with this name, or creates it. With what Roblox holds, no
    /// resource matched, so provision creates it.
    Create,
    /// The adopt check found one resource, whose ID provision records in the lock instead of creating one.
    Adopt,
    /// The journal holds an intent and the lock no ID: an earlier create's outcome is unknown, and provision looks
    /// for the resource before it does anything else. It adopts one with the declared name, and stops on one new since
    /// the intent that carries another name.
    AdoptCheck,
    /// `crucible adopt <env> <kind> <key> --none` cleared the key's intent: Roblox holds nothing the create made.
    Forget,
    Update,
    Retire,
    Untrack,
    NoOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// The name, description, price or enabled state differs from what the lock fingerprinted.
    Details,
    Icon,
    /// The lock records the resource as retired and the config no longer does.
    Restore,
    /// `moved_from` renames the lock entry.
    Key,
}

/// The journal's intent for a step's resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepIntent {
    /// The display name the create sent.
    pub name: String,
    /// The lock holds the ID, so the create landed and provision only clears the intent.
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub kind: &'static str,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    pub action: Action,
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<StepIntent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The experience settings step's settings, each that `[experience]` sets.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<Setting>,
}

/// One setting `[experience]` sets, and the value a provision writes for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Setting {
    /// Its key in `[experience]`.
    pub key: &'static str,
    pub value: Value,
    /// What Roblox holds, when provision read the universe and its reply stated the setting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roblox: Option<Value>,
}

fn settings(declared: &ResolvedSettings, held: Option<&ResolvedSettings>) -> Vec<Setting> {
    let mut settings = Vec::new();
    if let Some(price) = declared.private_server_price {
        let roblox = held.and_then(|held| held.private_server_price).map(|price| json!(price));
        settings.push(Setting { key: "private_server_price", value: json!(price), roblox });
    }
    if let Some(on) = declared.voice_chat {
        let roblox = held.and_then(|held| held.voice_chat).map(|on| json!(on));
        settings.push(Setting { key: "voice_chat", value: json!(on), roblox });
    }
    settings
}

/// The settings `crucible.lock` records, and what the universe is known to hold: each setting Roblox's reply states,
/// and the lock's value for each the reply leaves out or when the read failed.
pub(crate) fn known_settings(inputs: &Inputs) -> (ResolvedSettings, ResolvedSettings) {
    let recorded = inputs.lock.and_then(|lock| lock.experience.clone()).unwrap_or_default();
    let known = match inputs.observed.and_then(Observed::held_settings) {
        Some(held) => held.or(&recorded),
        None => recorded.clone(),
    };
    (recorded, known)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub environment: String,
    pub universe: String,
    pub steps: Vec<Step>,
    pub summary: BTreeMap<Action, usize>,
    pub warnings: Vec<String>,
}

pub struct Inputs<'a> {
    pub experience: &'a Experience,
    pub env_name: &'a str,
    pub env: &'a crucible_project::Env,
    pub lock: Option<&'a EnvLock>,
    pub journal: Option<&'a Journal>,
    /// What provision listed on Roblox, or `None` for the offline plan.
    pub observed: Option<&'a Observed>,
}

pub type Id = NonZeroU64;

/// A resource as Roblox lists it. `on` is a product's or a pass's sale state, or a badge's enabled state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: Id,
    pub name: String,
    pub description: String,
    /// A badge has none. A product or a pass that Roblox holds no price for differs from every declared price.
    pub price: Option<u64>,
    pub on: bool,
}

/// What provision read from Roblox before it planned: the whole list of each kind it needs, and each lock ID a list
/// lacked, read by itself: `Some` when Roblox has it and `None` when it answered 404.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    pub listed: BTreeMap<Kind, Vec<Listed>>,
    pub missing: BTreeMap<(Kind, Id), Option<Listed>>,
    /// The experience settings the universe holds, or why they could not be read. `None` when the environment declares
    /// no settings. A plan without them compares the settings with the lock only.
    pub settings: Option<Result<ResolvedSettings, String>>,
}

impl Observed {
    /// The experience settings provision read from the universe.
    pub fn held_settings(&self) -> Option<&ResolvedSettings> {
        self.settings.as_ref()?.as_ref().ok()
    }

    /// The resource a lock entry names, or the problem that stops provision: Roblox has no resource with the ID, or
    /// its list lacked one that exists, so the list cannot be trusted to find a resource before a create. The public
    /// badge list may leave out a disabled badge, so one found by itself is taken.
    pub fn bound(&self, kind: Kind, id: Id) -> Result<&Listed, Option<&Listed>> {
        if let Some(listed) = self.listed.get(&kind).and_then(|listed| listed.iter().find(|listed| listed.id == id)) {
            return Ok(listed);
        }
        match self.missing.get(&(kind, id)) {
            Some(Some(found)) if kind == Kind::Badge && !found.on => Ok(found),
            Some(Some(found)) => Err(Some(found)),
            _ => Err(None),
        }
    }
}

/// `read` returns a file's bytes by its path relative to the project root, for icon hashes.
pub fn plan(
    inputs: &Inputs<'_>,
    read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Plan, Vec<Problem>> {
    let mut problems = Vec::new();
    if let Some(lock) = inputs.lock
        && lock.universe() != inputs.env.universe
    {
        problems.push(another_universe(inputs.env_name, lock, inputs.env.universe));
    }
    if let Some(journal) = inputs.journal
        && journal.universe != inputs.env.universe
    {
        problems.push(journal_elsewhere(inputs.env_name, journal.universe, inputs.env.universe));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let intents: &[Intent] = inputs.journal.map_or(&[], |journal| &journal.intents);
    let mut attached = vec![false; intents.len()];
    let mut steps = Vec::new();
    if let Some(declared) = inputs.experience.settings() {
        let (recorded, known) = known_settings(inputs);
        let held = inputs.observed.and_then(Observed::held_settings);
        let changed = !declared.unheld(&known).is_empty() || !declared.unheld(&recorded).is_empty();
        steps.push(Step {
            kind: "experience",
            key: "settings".into(),
            handle: None,
            action: if changed { Action::Update } else { Action::NoOp },
            id: Some(inputs.env.universe.to_string()),
            changes: if changed { vec![Change::Details] } else { vec![] },
            moved_from: None,
            intent: None,
            note: None,
            settings: settings(declared, held),
        });
    }
    let resources = inputs.experience.resolve(inputs.env);
    let empty = BTreeMap::new();
    for kind in Kind::ALL {
        let entries = inputs.lock.map_or(&empty, |lock| lock.entries(kind));
        let declared: Vec<&Resource> = resources.iter().filter(|resource| resource.kind == kind).collect();
        let mut claimed = BTreeSet::new();
        let mut sought: Vec<(&str, BTreeSet<Id>)> = Vec::new();
        for resource in &declared {
            let (entry, moved_from) = match entries.get(&resource.key) {
                Some(entry) => (Some(entry), None),
                None => match resource.moved_from.as_ref().and_then(|old| entries.get(old).map(|entry| (entry, old))) {
                    Some((entry, old)) => (Some(entry), Some(old.clone())),
                    None => (None, None),
                },
            };
            claimed.insert(moved_from.clone().unwrap_or_else(|| resource.key.clone()));
            let mut intent = None;
            if entry.is_some() || resource.lifecycle == Lifecycle::Managed {
                let mut found = intents.iter().enumerate().filter(|(_, intent)| {
                    intent.kind == kind
                        && (intent.key == resource.key || resource.moved_from.as_ref() == Some(&intent.key))
                });
                intent = found.next();
                if let Some((at, other)) = found.next() {
                    attached[at] = true;
                    problems.push(journal_problem(
                        inputs.env_name,
                        kind,
                        &resource.key,
                        format!(
                            "the journal holds intents for {table}.{} and {table}.{}, and moved_from makes them one \
                             resource. Record the one that landed with `crucible adopt {env} {noun} {key} <id>`, or \
                             run `crucible adopt {env} {noun} {key} --none` if neither did",
                            intent.map_or("", |(_, intent)| &intent.key),
                            other.key,
                            env = inputs.env_name,
                            noun = kind.noun(),
                            key = resource.key,
                            table = kind.table(),
                        ),
                    ));
                }
            }
            if let Some((at, _)) = intent {
                attached[at] = true;
            }
            let intent = intent.map(|(_, intent)| intent);
            let candidates = match (inputs.observed, entry, resource.lifecycle) {
                (Some(observed), None, Lifecycle::Managed) => {
                    let found = candidates(observed, kind, entries, resource, intent);
                    sought.push((&resource.key, found.adopt.iter().chain(found.strangers.keys()).copied().collect()));
                    Some(found)
                }
                _ => None,
            };
            match step(inputs, resource, entry, moved_from, intent, candidates.as_ref(), read) {
                Ok(step) => steps.push(step),
                Err(problem) => problems.push(problem),
            }
        }
        for (at, (key, found)) in sought.iter().enumerate() {
            for (other, theirs) in &sought[at + 1..] {
                for id in found.intersection(theirs) {
                    problems.push(Problem {
                        file: crucible_project::FILE.into(),
                        line: None,
                        key: format!("{}.{key}", kind.table()),
                        message: format!(
                            "the adopt checks of {table}.{key} and {table}.{other} both find {noun} {id}, so provision \
                             cannot tell whose it is. Look at it in universe {universe}, then record it with `crucible \
                             adopt {env} {noun} <key> {id}` for the key it belongs to",
                            table = kind.table(),
                            noun = kind.noun(),
                            universe = inputs.env.universe,
                            env = inputs.env_name,
                        ),
                    });
                }
            }
        }
        for (key, entry) in entries.iter().filter(|(key, _)| !claimed.contains(*key)) {
            if entry.status == Status::Retired {
                let intent = intents.iter().position(|intent| intent.kind == kind && intent.key == *key);
                if let Some(at) = intent {
                    attached[at] = true;
                }
                steps.push(Step {
                    kind: kind.noun(),
                    key: key.clone(),
                    handle: None,
                    action: Action::NoOp,
                    id: Some(entry.id.to_string()),
                    changes: vec![],
                    moved_from: None,
                    intent: intent.map(|at| StepIntent { name: intents[at].name.clone(), resolved: true }),
                    note: Some("retired and no longer declared; the lock keeps its ID".into()),
                    settings: vec![],
                });
                continue;
            }
            let mut message = format!(
                "crucible.lock holds its ID for {}, and crucible.toml no longer declares it. Declare it with retired = \
                 true to take it off Roblox, or untrack = true to keep it there unmanaged, before removing it",
                inputs.env_name
            );
            message += &renamed_hint(&declared, entries, key);
            problems.push(Problem {
                file: crucible_project::FILE.into(),
                line: None,
                key: format!("{}.{key}", kind.table()),
                message,
            });
        }
        for (at, intent) in intents.iter().enumerate().filter(|(_, intent)| intent.kind == kind) {
            let resolved = |intent: &Intent| {
                entries.contains_key(&intent.key)
                    || declared.iter().any(|resource| {
                        resource.key == intent.key
                            && resource.moved_from.as_ref().is_some_and(|old| entries.contains_key(old))
                    })
            };
            if !attached[at] {
                problems.push(unattached(inputs, &resources, &declared, entries, intent));
            }
            if !resolved(intent) {
                problems
                    .extend(declared.iter().filter_map(|resource| sent_name_taken(inputs.env_name, resource, intent)));
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut warnings = Vec::new();
    let badges = steps
        .iter()
        .filter(|step| step.kind == Kind::Badge.noun() && matches!(step.action, Action::Create | Action::AdoptCheck))
        .count();
    if badges > FREE_BADGES_PER_DAY {
        warnings.push(format!(
            "{badges} badges may need creating. Roblox makes {FREE_BADGES_PER_DAY} a day free in each experience and \
             charges 100 Robux for each one after that. Provision never pays: Roblox rejects a create past the free \
             ones, provision stops there, and a run the next day creates the rest."
        ));
    }
    let mut summary = BTreeMap::new();
    for step in &steps {
        *summary.entry(step.action).or_insert(0) += 1;
    }
    Ok(Plan {
        environment: inputs.env_name.into(),
        universe: inputs.env.universe.to_string(),
        steps,
        summary,
        warnings,
    })
}

/// The lock tables and journals of environments `crucible.toml` no longer declares. No plan reads them, so their IDs
/// and intents would vanish, and an environment renamed in `crucible.toml` alone would create every resource it already
/// holds again. `journals` holds every journal file, by the environment its name gives.
pub fn undeclared(
    envs: &BTreeMap<String, crucible_project::Env>,
    lock: Option<&lock::Lock>,
    journals: &BTreeMap<String, Journal>,
) -> Vec<Problem> {
    let locked = lock.map(|lock| lock.envs().keys()).into_iter().flatten();
    let names: BTreeSet<&String> = locked.chain(journals.keys()).filter(|name| !envs.contains_key(*name)).collect();
    names
        .into_iter()
        .map(|env| {
            let env_lock = lock.and_then(|lock| lock.env(env));
            let journal = journals.get(env);
            let path = journal::path(env).display().to_string();
            let unresolved: Vec<&Intent> = journal
                .map_or(&[][..], |journal| &journal.intents)
                .iter()
                .filter(|intent| env_lock.is_none_or(|locked| !locked.entries(intent.kind).contains_key(&intent.key)))
                .collect();
            let mut message = String::new();
            if let (Some(journal), false) = (journal, unresolved.is_empty()) {
                let creates: Vec<String> = unresolved
                    .iter()
                    .map(|intent| format!("{} {} as {:?}", intent.kind.noun(), intent.key, intent.name))
                    .collect();
                let (noun, them, create) =
                    if creates.len() == 1 { ("an intent", "it", "create") } else { ("intents", "them", "creates") };
                message += &format!(
                    "{path} holds {noun} to create {} in universe {}, and crucible.lock holds no ID for {them}, so the \
                     {create} may have landed. ",
                    creates.join(", "),
                    journal.universe
                );
            }
            message += &format!("crucible.toml declares no environment {env}");
            if let Some(locked) = env_lock {
                message += &format!(
                    ", and crucible.lock holds {} of it in universe {}",
                    count(locked.resources(), "resource"),
                    locked.universe()
                );
            }
            if let Some(journal) = journal {
                message += &format!(
                    ", and {path} holds {} sent to universe {}",
                    count(journal.intents.len(), "intent"),
                    journal.universe
                );
            }
            let universe =
                env_lock.map_or_else(|| journal.map(|journal| journal.universe), |locked| Some(locked.universe()));
            let universe = universe.expect("an undeclared environment comes from the lock or a journal");
            let same: Vec<String> = envs
                .iter()
                .filter(|(_, other)| other.universe == universe)
                .map(|(name, _)| format!("env.{name}"))
                .collect();
            let such = if same.is_empty() { String::new() } else { format!(", such as {}", same.join(", ")) };
            let (mut rename, mut gone) = (Vec::new(), Vec::new());
            if env_lock.is_some() {
                rename
                    .push(format!("rename [env.{env}] and every [env.{env}.*] table in crucible.lock to the new name"));
                gone.push(format!("delete [env.{env}] and every [env.{env}.*] table from crucible.lock"));
            }
            if let Some(journal) = journal {
                rename.push(format!("rename {path} to the new name"));
                gone.push(if unresolved.is_empty() {
                    format!("delete {path}")
                } else {
                    format!(
                        "look in universe {} for the resources its intents name, then delete {path}",
                        journal.universe
                    )
                });
            }
            message += &format!(
                ". No plan reads them, so a renamed environment would create each resource again. Declare [env.{env}] \
                 with universe = {universe} in crucible.toml again. If it was renamed to an environment with universe \
                 {universe}{such}, {}. If {env} is gone for good, {}, and its resources stay on Roblox, unmanaged",
                rename.join(", and "),
                gone.join(", and ")
            );
            let (file, line) = match env_lock {
                Some(locked) if unresolved.is_empty() => (lock::FILE.to_owned(), Some(locked.universe_line())),
                _ => (path, None),
            };
            Problem { file, line, key: format!("env.{env}"), message }
        })
        .collect()
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

/// IDs from another universe would read as resources this one already has, so nothing would be created in it.
fn another_universe(env: &str, lock: &EnvLock, universe: NonZeroU64) -> Problem {
    let locked = lock.universe();
    Problem {
        file: lock::FILE.into(),
        line: Some(lock.universe_line()),
        key: format!("env.{env}.universe"),
        message: format!(
            "crucible.lock records {env}'s resources in universe {locked}, and crucible.toml names universe \
             {universe} for {env}. If the universe ID changed by mistake, set env.{env}.universe back to {locked} in \
             crucible.toml. If {env} now deploys to universe {universe}, start {env} afresh: delete [env.{env}] and \
             every [env.{env}.*] table from crucible.lock, and the resources in universe {locked} stay on Roblox, \
             unmanaged"
        ),
    }
}

/// An intent's create may have landed in the universe it was sent to, which a plan for this one cannot check.
fn journal_elsewhere(env: &str, sent: NonZeroU64, universe: NonZeroU64) -> Problem {
    let path = journal::path(env).display().to_string();
    Problem {
        file: path.clone(),
        line: None,
        key: "universe".into(),
        message: format!(
            "the journal holds intents sent to universe {sent}, and crucible.toml names universe {universe} for \
             {env}. If the universe ID changed by mistake, set env.{env}.universe back to {sent} in crucible.toml. If \
             {env} now deploys to universe {universe}, look in universe {sent} for the resources its intents name, \
             whose creates may have landed there, then delete {path}"
        ),
    }
}

fn journal_problem(env: &str, kind: Kind, key: &str, message: String) -> Problem {
    Problem {
        file: journal::path(env).display().to_string(),
        line: None,
        key: format!("{}.{key}", kind.table()),
        message,
    }
}

/// The keys of `declared` that hold no lock entry and no `moved_from`, any of which may be `old` renamed.
fn renamed_hint(declared: &[&Resource], entries: &BTreeMap<String, Entry>, old: &str) -> String {
    let renamed: Vec<_> = declared
        .iter()
        .filter(|resource| resource.moved_from.is_none() && !entries.contains_key(&resource.key))
        .map(|resource| format!("{}.{}", resource.kind.table(), resource.key))
        .collect();
    if renamed.is_empty() {
        String::new()
    } else {
        format!(". If it was renamed, set moved_from = {old:?} on the new key, one of {}", renamed.join(", "))
    }
}

/// An intent no step carries: its key is gone, retired or untracked, so provision would never resolve it.
fn unattached(
    inputs: &Inputs<'_>,
    resources: &[Resource],
    declared: &[&Resource],
    entries: &BTreeMap<String, Entry>,
    intent: &Intent,
) -> Problem {
    let (kind, key, name) = (intent.kind, &intent.key, &intent.name);
    let (table, noun, universe) = (kind.table(), kind.noun(), inputs.env.universe);
    let mut message = format!("the journal holds an intent to create {noun} {key} as {name:?}, ");
    let yourself = format!(
        "or, if universe {universe} holds no {noun} its create made, run `crucible adopt {} {noun} {key} --none`",
        inputs.env_name
    );
    let lifecycle = declared.iter().find(|resource| resource.key == *key).map(|resource| resource.lifecycle);
    match (entries.get(key), lifecycle) {
        (Some(entry), _) => {
            message += &format!(
                "and crucible.lock holds its ID {}, so the create landed. crucible.toml no longer declares {table}.{key}. \
             Declare it again, with retired = true or untrack = true if it should leave, and provision clears the \
             intent",
                entry.id
            )
        }
        (None, Some(Lifecycle::Retired | Lifecycle::Untrack)) => {
            let (field, verb) =
                if lifecycle == Some(Lifecycle::Retired) { ("retired", "retires") } else { ("untrack", "untracks") };
            message += &format!(
                "and crucible.lock holds no ID for it, so the create may have landed in universe {universe}. \
                 crucible.toml {verb} {table}.{key}, so provision would never look for it. Remove {field} = true \
                 until provision resolves the intent, {yourself}"
            );
        }
        (None, _) => {
            message += &format!(
                "and crucible.lock holds no ID for it, so the create may have landed in universe {universe}. \
                 crucible.toml declares no {table}.{key}"
            );
            for other in Kind::ALL.into_iter().filter(|other| *other != kind) {
                if resources.iter().any(|resource| resource.kind == other && resource.key == *key) {
                    message += &format!(", and {}.{key} is declared, but the intent is for a {noun}", other.table());
                }
            }
            message += &renamed_hint(declared, entries, key);
            message += &format!(". Declare it again until provision resolves the intent, {yourself}");
        }
    }
    journal_problem(inputs.env_name, kind, key, message)
}

/// The adopt check of `resource` would find the resource an unresolved create of another key may have made under
/// that name, and could bind it to the wrong key.
fn sent_name_taken(env: &str, resource: &Resource, intent: &Intent) -> Option<Problem> {
    let name = resource.fields.as_ref()?.name();
    if resource.lifecycle != Lifecycle::Managed
        || name != intent.name
        || intent.key == resource.key
        || resource.moved_from.as_ref() == Some(&intent.key)
    {
        return None;
    }
    let (table, noun, key, other) = (resource.kind.table(), resource.kind.noun(), &resource.key, &intent.key);
    Some(Problem::at(
        resource.line,
        format!("{table}.{key}.name"),
        format!(
            "{name:?} is the name the journal's intent for {table}.{other} sent, and crucible.lock holds no ID for it, \
             so its create may have landed under that name and the adopt check could bind {table}.{key} to it. Give \
             {table}.{key} another name until provision resolves that intent: record {table}.{other}'s {noun} with \
             `crucible adopt {env} {noun} {other} <id>`, or run `crucible adopt {env} {noun} {other} --none` if its \
             create never landed"
        ),
    ))
}

fn step(
    inputs: &Inputs<'_>,
    resource: &Resource,
    entry: Option<&Entry>,
    moved_from: Option<String>,
    intent: Option<&Intent>,
    candidates: Option<&Found>,
    read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Step, Problem> {
    let kind = resource.kind;
    let remote = match (inputs.observed, entry, resource.lifecycle) {
        (Some(observed), Some(entry), Lifecycle::Managed | Lifecycle::Retired) => {
            match observed.bound(kind, entry.id) {
                Ok(remote) => Some(remote),
                Err(None) if resource.lifecycle == Lifecycle::Retired => None,
                Err(found) => return Err(unbound(inputs, resource, entry, found.is_some())),
            }
        }
        _ => None,
    };
    let icon = match (&resource.icon, resource.lifecycle) {
        (Some(path), Lifecycle::Managed) => Some(read(path).map(|bytes| content_hash(&bytes)).map_err(|error| {
            Problem::at(
                resource.line,
                format!("{}.{}.icon", kind.table(), resource.key),
                format!("cannot read {}: {error}", path.display()),
            )
        })?),
        _ => None,
    };
    let mut step = Step {
        kind: kind.noun(),
        key: resource.key.clone(),
        handle: Some(resource.handle()),
        action: Action::NoOp,
        id: entry.map(|entry| entry.id.to_string()),
        changes: vec![],
        moved_from,
        intent: intent.map(|intent| StepIntent { name: intent.name.clone(), resolved: entry.is_some() }),
        note: None,
        settings: vec![],
    };
    if step.moved_from.is_some() {
        step.changes.push(Change::Key);
    }
    match (resource.lifecycle, entry) {
        (Lifecycle::Untrack, Some(_)) => step.action = Action::Untrack,
        (Lifecycle::Untrack, None) => step.note = Some("not in crucible.lock".into()),
        (Lifecycle::Retired, Some(entry)) if entry.status == Status::Managed => step.action = Action::Retire,
        (Lifecycle::Retired, Some(_)) if remote.is_some_and(|remote| remote.on) => {
            step.action = Action::Retire;
            step.note = Some("back on sale or enabled on Roblox".into());
        }
        (Lifecycle::Retired, Some(_)) => {}
        (Lifecycle::Retired, None) => step.note = Some(format!("never provisioned in {}", inputs.env_name)),
        (Lifecycle::Managed, None) => {
            step.action = if intent.is_some() { Action::AdoptCheck } else { Action::Create };
            if let Some(found) = candidates {
                let mut ids = found.adopt.iter();
                match (ids.next(), ids.next(), intent) {
                    (_, _, Some(intent)) if kind == Kind::Badge => {
                        return Err(unresolved_badge(inputs, resource, intent));
                    }
                    (_, _, Some(intent)) if !found.strangers.is_empty() => {
                        return Err(strangers(inputs, resource, intent, &found.strangers));
                    }
                    (None, _, intent) => {
                        step.action = Action::Create;
                        if intent.is_some() {
                            step.note = Some("Roblox lists nothing the earlier create could have made".into());
                        }
                    }
                    (Some(id), None, _) => {
                        step.action = Action::Adopt;
                        step.id = Some(id.to_string());
                    }
                    (Some(_), Some(_), _) => return Err(ambiguous(inputs, resource, &found.adopt)),
                }
            }
        }
        (Lifecycle::Managed, Some(entry)) => {
            let fields = resource.fields.as_ref().expect("a managed resource has its fields");
            if entry.fingerprint != fields.fingerprint() || remote.is_some_and(|remote| differs(fields, remote)) {
                step.changes.push(Change::Details);
            }
            if icon.is_some() && entry.icon != icon {
                step.changes.push(Change::Icon);
            }
            if entry.status == Status::Retired {
                step.changes.push(Change::Restore);
            }
            if !step.changes.is_empty() {
                step.action = Action::Update;
            }
        }
    }
    if step.action == Action::NoOp && step.changes.contains(&Change::Key) {
        step.action = Action::Update;
    }
    Ok(step)
}

/// Whether Roblox's resource differs from what the config declares for a managed key.
pub fn differs(fields: &Fields, remote: &Listed) -> bool {
    match fields {
        Fields::Priced { name, description, price } => {
            *name != remote.name || *description != remote.description || remote.price != Some(*price) || !remote.on
        }
        Fields::Badge { name, description, enabled } => {
            *name != remote.name || *description != remote.description || remote.on != *enabled
        }
    }
}

/// Whether the adopt check of `resource` looks at `listed`, which no lock entry holds: it has the declared name or,
/// while the key has an intent, an ID the intent's run did not know, or the name an intent written without IDs sent.
/// `crucible adopt` takes any of them. A run adopts only one with the declared name, because a new ID with another name
/// may be the create renamed on Roblox, or a resource someone made by hand, and only a person can tell which.
pub fn is_candidate(listed: &Listed, resource: &Resource, intent: Option<&Intent>) -> bool {
    has_declared_name(listed, resource)
        || intent.is_some_and(|intent| match &intent.before {
            Some(before) => !before.contains(&listed.id),
            None => listed.name == intent.name,
        })
}

fn has_declared_name(listed: &Listed, resource: &Resource) -> bool {
    resource.fields.as_ref().is_some_and(|fields| fields.name() == listed.name)
}

/// What a key's adopt check found: the resources a run may adopt, and the others it looked at, by ID with their names.
struct Found {
    adopt: BTreeSet<Id>,
    strangers: BTreeMap<Id, String>,
}

fn candidates(
    observed: &Observed,
    kind: Kind,
    entries: &BTreeMap<String, Entry>,
    resource: &Resource,
    intent: Option<&Intent>,
) -> Found {
    let locked: BTreeSet<Id> = entries.values().map(|entry| entry.id).collect();
    let mut found = Found { adopt: BTreeSet::new(), strangers: BTreeMap::new() };
    let listed = observed.listed.get(&kind).into_iter().flatten();
    for listed in listed.filter(|listed| !locked.contains(&listed.id) && is_candidate(listed, resource, intent)) {
        if has_declared_name(listed, resource) {
            found.adopt.insert(listed.id);
        } else {
            found.strangers.insert(listed.id, listed.name.clone());
        }
    }
    found
}

fn ambiguous(inputs: &Inputs<'_>, resource: &Resource, found: &BTreeSet<Id>) -> Problem {
    let (table, noun, key) = (resource.kind.table(), resource.kind.noun(), &resource.key);
    let ids: Vec<String> = found.iter().map(ToString::to_string).collect();
    Problem::at(
        resource.line,
        format!("{table}.{key}"),
        format!(
            "universe {} holds {} {noun}s that {table}.{key} could be: {}. Provision adopts only one, so it stops. Record \
             the one that is {table}.{key}'s with `crucible adopt {} {noun} {key} <id>`, or rename the others on Roblox",
            inputs.env.universe,
            ids.len(),
            ids.join(", "),
            inputs.env_name,
        ),
    )
}

/// Resources new since the key's intent, or with the name an intent without IDs sent, that do not carry the declared
/// name. One may be the create, renamed on Roblox, or someone made it by hand, so provision takes none on its own.
fn strangers(inputs: &Inputs<'_>, resource: &Resource, intent: &Intent, found: &BTreeMap<Id, String>) -> Problem {
    let (table, noun, key, env) = (resource.kind.table(), resource.kind.noun(), &resource.key, inputs.env_name);
    let declared = resource.fields.as_ref().map_or("", Fields::name);
    let listed: Vec<String> = found.iter().map(|(id, name)| format!("{id} ({name:?})")).collect();
    let (them, one) = if found.len() == 1 { ("it", "it is") } else { ("them", "one is") };
    Problem::at(
        resource.line,
        format!("{table}.{key}"),
        format!(
            "the journal holds an intent to create {noun} {key} as {:?}, and crucible.lock holds no ID for it, so the \
             create may have landed in universe {}. Since that intent, the universe gained {}, which {} not carry the \
             declared name {declared:?}. The create may have been renamed on Roblox, or someone made {them} by hand, \
             so provision adopts none on its own. If {one} {table}.{key}'s, record it with `crucible adopt {env} {noun} \
             {key} <id>`. If none is, run `crucible adopt {env} {noun} {key} --none`, and provision creates it",
            intent.name,
            inputs.env.universe,
            listed.join(", "),
            if found.len() == 1 { "does" } else { "do" },
        ),
    )
}

/// The public badge list can lag behind a create, so it can leave out a badge whose answer was lost while it shows
/// another with the name, and provision leaves a badge's unknown outcome to someone who has looked.
fn unresolved_badge(inputs: &Inputs<'_>, resource: &Resource, intent: &Intent) -> Problem {
    let key = &resource.key;
    Problem::at(
        resource.line,
        format!("badges.{key}"),
        format!(
            "the journal holds an intent to create badge {key} as {:?}, and crucible.lock holds no ID for it, so the \
             create may have landed in universe {}. The public badge list can lag behind a create, so provision does \
             not settle a badge's create on its own. Look for it in the experience's badges on Roblox. If it is there, \
             run `crucible adopt {env} badge {key} <id>`. If it is not, run `crucible adopt {env} badge {key} --none` \
             and provision again",
            intent.name,
            inputs.env.universe,
            env = inputs.env_name,
        ),
    )
}

/// A lock entry whose ID Roblox's list lacks: the resource is gone, or the list left it out.
fn unbound(inputs: &Inputs<'_>, resource: &Resource, entry: &Entry, exists: bool) -> Problem {
    let (table, noun, key, id) = (resource.kind.table(), resource.kind.noun(), &resource.key, entry.id);
    let (env, universe) = (inputs.env_name, inputs.env.universe);
    let message = if exists {
        format!(
            "universe {universe}'s list of {table} leaves out {noun} {id}, which crucible.lock records for {key} and \
             Roblox returns when asked for it by ID. Provision relies on that list to find a resource before it creates \
             one, so it stops. Run it again later"
        )
    } else {
        format!(
            "crucible.lock records {key} in {env} as {noun} {id}, and Roblox has no {noun} {id}. Provision never \
             replaces a resource on its own, because a lock that names a missing resource may belong to another \
             universe. If {id} is gone, delete [env.{env}.{table}.{key}] from crucible.lock, and provision adopts or \
             creates it again"
        )
    };
    Problem::at(resource.line, format!("{table}.{key}"), message)
}
