//! What `crucible provision <env>` would do, from the config, the lock and the journal alone. It makes no network
//! call, so it cannot see Roblox: a create is always preceded by provision's adopt check, and an update is a change
//! of the inputs the lock fingerprinted. Every intent in the journal shows on the step of its resource, or stops the
//! plan with a problem that names it, because an intent nobody sees is a create nobody resolves.

use crate::journal::{self, Intent, Journal};
use crate::lock::{self, Entry, EnvLock, Status};
use crate::model::{Experience, Kind, Lifecycle, Problem, Resource, content_hash};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::Path;

/// Roblox makes this many badges a day free in each experience and charges 100 Robux for each one after that.
pub const FREE_BADGES_PER_DAY: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// No ID in the lock: provision adopts the one resource with this name, or creates it.
    Create,
    /// The journal holds an intent and the lock no ID: an earlier create's outcome is unknown, and provision looks
    /// for the resource, under the declared name and the name the intent sent, before it does anything else.
    AdoptCheck,
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
    if let Some(settings) = inputs.experience.settings() {
        let applied = inputs.lock.and_then(|lock| lock.experience.as_ref());
        let changed = applied.is_none_or(|applied| applied.fingerprint != settings.fingerprint());
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
        });
    }
    let resources = inputs.experience.resolve(inputs.env);
    let empty = BTreeMap::new();
    for kind in Kind::ALL {
        let entries = inputs.lock.map_or(&empty, |lock| lock.entries(kind));
        let declared: Vec<&Resource> = resources.iter().filter(|resource| resource.kind == kind).collect();
        let mut claimed = BTreeSet::new();
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
                             resource. Resolve one with `crucible adopt {} <key> <id>`, or remove the one whose create \
                             never landed",
                            intent.map_or("", |(_, intent)| &intent.key),
                            other.key,
                            kind.noun(),
                            table = kind.table(),
                        ),
                    ));
                }
            }
            if let Some((at, _)) = intent {
                attached[at] = true;
            }
            match step(inputs, resource, entry, moved_from, intent.map(|(_, intent)| intent), read) {
                Ok(step) => steps.push(step),
                Err(problem) => problems.push(problem),
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
             charges 100 Robux for each one after that. Provision never pays: it defers the rest to a later day."
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
    let path = journal::path(inputs.env_name).display().to_string();
    let mut message = format!("the journal holds an intent to create {noun} {key} as {name:?}, ");
    let yourself = format!(
        "or resolve it yourself: record the {noun} named {name:?} in universe {universe} with `crucible adopt {noun} \
         {key} <id>`, or remove the intent from {path} if universe {universe} has no such {noun}"
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
             {table}.{key} another name until provision resolves that intent, or resolve it yourself with `crucible \
             adopt {noun} {other} <id>`, or remove the intent from {} if no {noun} has that name",
            journal::path(env).display()
        ),
    ))
}

fn step(
    inputs: &Inputs<'_>,
    resource: &Resource,
    entry: Option<&Entry>,
    moved_from: Option<String>,
    intent: Option<&Intent>,
    read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Step, Problem> {
    let kind = resource.kind;
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
    };
    if step.moved_from.is_some() {
        step.changes.push(Change::Key);
    }
    match (resource.lifecycle, entry) {
        (Lifecycle::Untrack, Some(_)) => step.action = Action::Untrack,
        (Lifecycle::Untrack, None) => step.note = Some("not in crucible.lock".into()),
        (Lifecycle::Retired, Some(entry)) if entry.status == Status::Managed => step.action = Action::Retire,
        (Lifecycle::Retired, Some(_)) => {}
        (Lifecycle::Retired, None) => step.note = Some(format!("never provisioned in {}", inputs.env_name)),
        (Lifecycle::Managed, None) if intent.is_some() => step.action = Action::AdoptCheck,
        (Lifecycle::Managed, None) => step.action = Action::Create,
        (Lifecycle::Managed, Some(entry)) => {
            let fields = resource.fields.as_ref().expect("a managed resource has its fields");
            if entry.fingerprint != fields.fingerprint() {
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
