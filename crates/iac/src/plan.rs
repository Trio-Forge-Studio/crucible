//! What `crucible provision <env>` would do, from the config, the lock and the journal alone. It makes no network
//! call, so it cannot see Roblox: a create is always preceded by provision's adopt check, and an update is a change
//! of the inputs the lock fingerprinted.

use crate::journal::Journal;
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
    /// for the resource before it does anything else.
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
    if let Some(lock) = inputs.lock
        && lock.universe() != inputs.env.universe
    {
        return Err(vec![another_universe(inputs.env_name, lock, inputs.env.universe)]);
    }
    let mut problems = Vec::new();
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
            match step(inputs, resource, entry, moved_from, read) {
                Ok(step) => steps.push(step),
                Err(problem) => problems.push(problem),
            }
        }
        for (key, entry) in entries.iter().filter(|(key, _)| !claimed.contains(*key)) {
            if entry.status == Status::Retired {
                steps.push(Step {
                    kind: kind.noun(),
                    key: key.clone(),
                    handle: None,
                    action: Action::NoOp,
                    id: Some(entry.id.to_string()),
                    changes: vec![],
                    moved_from: None,
                    note: Some("retired and no longer declared; the lock keeps its ID".into()),
                });
                continue;
            }
            let mut message = format!(
                "crucible.lock holds its ID for {}, and crucible.toml no longer declares it. Declare it with retired = \
                 true to take it off Roblox, or untrack = true to keep it there unmanaged, before removing it",
                inputs.env_name
            );
            let renamed: Vec<_> = declared
                .iter()
                .filter(|resource| resource.moved_from.is_none() && !entries.contains_key(&resource.key))
                .map(|resource| format!("{}.{}", kind.table(), resource.key))
                .collect();
            if !renamed.is_empty() {
                message.push_str(&format!(
                    ". If it was renamed, set moved_from = {key:?} on the new key, one of {}",
                    renamed.join(", ")
                ));
            }
            problems.push(Problem {
                file: crucible_project::FILE,
                line: None,
                key: format!("{}.{key}", kind.table()),
                message,
            });
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
        file: lock::FILE,
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

fn step(
    inputs: &Inputs<'_>,
    resource: &Resource,
    entry: Option<&Entry>,
    moved_from: Option<String>,
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
        (Lifecycle::Managed, None) if inputs.journal.is_some_and(|journal| journal.intends(kind, &resource.key)) => {
            step.action = Action::AdoptCheck;
        }
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
