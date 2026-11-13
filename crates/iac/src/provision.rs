//! `crucible provision <env>`: make Roblox hold what the config declares, recording each ID in `crucible.lock`.
//! `specs/provision` models it, and `tests/conformance.rs` checks this code against the model's invariants.
//!
//! A run reads the lock and the journal, lists what Roblox holds through `observe`, plans with `plan::plan`, which
//! stays pure, and then applies the plan one effect at a time: each `Run::step` makes one remote call or writes one
//! file. A crash between two steps leaves files the next run starts from. The order of the effects is what makes that
//! safe:
//!
//! 1. One journal write drops each intent the lock resolves, and each intent whose adopt check found nothing: the list
//!    shows every landed create, so that create never landed. After it, no intent is unresolved.
//! 2. An adopt writes the ID to the lock, then clears the key's intent.
//! 3. An update or a retire writes to Roblox, then to the lock. Its writes are absolute, so a crash between repeats it.
//! 4. A create writes its intent, with the name it sends and the IDs the run knows, then sends the create, then writes
//!    the ID to the lock, then clears the intent. No intent is written while another is unresolved, so every resource
//!    the run did not know is the intent's own or one made by hand. An unknown outcome stops the run with the intent
//!    kept, and the next run's adopt check resolves it. A create Roblox certainly refused clears its intent, then stops
//!    the run. Nothing sends a create twice for one intent.
//! 5. The experience settings update like a resource, and the lock records their fingerprint. They go last, so a
//!    settings update Roblox refuses stops nothing a product, a pass or a badge needs.

use crate::journal::{Intent, Journal};
use crate::lock::{Entry, EnvLock, Lock, Status};
use crate::model::{Experience, Fields, Kind, Lifecycle, ResolvedSettings, Resource, content_hash};
use crate::plan::{Action, Change, Id, Inputs, Listed, Observed, Plan};
use crucible_oc::ops::Image;
use crucible_oc::outcome::Failure;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Roblox, as provision reaches it. `crucible_oc`'s typed operations implement it in `oc`, and tests fake it.
pub trait Remote {
    /// Every resource of the kind in the universe, read to the end.
    fn list(&mut self, kind: Kind) -> Result<Vec<Listed>, Failure>;
    /// One resource, or `None` when Roblox answers that it has no such resource.
    fn get(&mut self, kind: Kind, id: Id) -> Result<Option<Listed>, Failure>;
    /// A failure for which `may_have_landed` holds leaves the create's outcome unknown.
    fn create(&mut self, kind: Kind, fields: &Fields, icon: Option<&Image>) -> Result<Id, Failure>;
    fn update(&mut self, kind: Kind, id: Id, patch: &Patch) -> Result<(), Failure>;
    /// Writes the settings that are set and leaves the rest as they are. A run never calls it with none set.
    fn update_settings(&mut self, settings: &ResolvedSettings) -> Result<(), Failure>;
    /// The experience settings the universe holds, `None` for each the reply does not state.
    fn read_settings(&mut self) -> Result<ResolvedSettings, Failure>;
}

/// The fields an update sets. `on` is the sale state of a product or a pass, or whether a badge is enabled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Patch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<u64>,
    pub on: Option<bool>,
    pub icon: Option<Image>,
}

impl Patch {
    fn is_empty(&self) -> bool {
        *self == Patch::default()
    }
}

/// Whether a failed create may have made the resource. `Transport::run` fails an unsafe write with code 5 once an
/// attempt got a 2xx or a 5xx, or its request left the machine and the answer was lost. It fails with 3 on a 401 or a
/// 403, 4 on another 4xx, 7 on a 429 or a request that never left, and 2 on a request it could not build, and in each
/// of those Roblox made nothing. Any other code is taken as unknown.
pub fn may_have_landed(failure: &Failure) -> bool {
    !matches!(failure.code, 2 | 3 | 4 | 7)
}

/// Where the lock and the journal live.
pub trait Store {
    fn save_lock(&mut self, lock: &Lock) -> io::Result<()>;
    /// Removes the file when the journal holds no intent.
    fn save_journal(&mut self, env: &str, journal: &Journal) -> io::Result<()>;
}

/// The lock and the journals in a project folder. Each write goes to a temporary file in the target's folder, which
/// then replaces the target, so a crash leaves the old file or the new one, never part of one.
pub struct Files {
    pub root: PathBuf,
}

impl Store for Files {
    fn save_lock(&mut self, lock: &Lock) -> io::Result<()> {
        write_atomic(&self.root.join(crate::lock::FILE), lock.render().as_bytes())
    }

    fn save_journal(&mut self, env: &str, journal: &Journal) -> io::Result<()> {
        let path = self.root.join(crate::journal::path(env));
        if journal.intents.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(path.parent().expect("a journal lives in a folder"))?;
        write_atomic(&path, journal.render().as_bytes())
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let folder = path.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    sync_folder(folder);
    Ok(())
}

/// On Unix a rename survives a power loss only once its folder is flushed too. Windows cannot open a folder to flush.
/// The rename has happened by now, and some file systems refuse to flush a folder, so a failure here is not an error.
fn sync_folder(folder: &Path) {
    #[cfg(unix)]
    let _ = std::fs::File::open(folder).and_then(|folder| folder.sync_all());
    #[cfg(not(unix))]
    let _ = folder;
}

/// The kinds `observe` lists: each kind the environment declares or its lock or journal holds.
pub fn listed_kinds(
    experience: &Experience,
    env: &crucible_project::Env,
    lock: Option<&EnvLock>,
    journal: Option<&Journal>,
) -> Vec<Kind> {
    let declared = experience.resolve(env);
    Kind::ALL
        .into_iter()
        .filter(|&kind| {
            declared.iter().any(|resource| resource.kind == kind)
                || lock.is_some_and(|lock| !lock.entries(kind).is_empty())
                || journal.is_some_and(|journal| journal.intents.iter().any(|intent| intent.kind == kind))
        })
        .collect()
}

/// Lists each of `listed_kinds`, and reads by ID each lock ID a list lacks, so the plan can tell a resource that is
/// gone from one the list left out. When the environment declares experience settings, it reads what the universe
/// holds too. A failed read of the settings does not fail `observe`: it keeps why, and the plan compares the settings
/// with the lock only.
pub fn observe(
    experience: &Experience,
    env: &crucible_project::Env,
    lock: Option<&EnvLock>,
    journal: Option<&Journal>,
    remote: &mut dyn Remote,
) -> Result<Observed, Failure> {
    let mut observed = Observed::default();
    for kind in listed_kinds(experience, env, lock, journal) {
        let locked = lock.map(|lock| lock.entries(kind));
        let listed = remote.list(kind)?;
        for entry in locked.into_iter().flat_map(|entries| entries.values()) {
            if !listed.iter().any(|listed| listed.id == entry.id) {
                observed.missing.insert((kind, entry.id), remote.get(kind, entry.id)?);
            }
        }
        observed.listed.insert(kind, listed);
    }
    if experience.settings().is_some() {
        observed.settings = Some(remote.read_settings().map_err(|failure| {
            failure.error["message"].as_str().unwrap_or("Roblox's answer could not be read").to_owned()
        }));
    }
    Ok(observed)
}

/// What a run did, in order, for its report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub kind: &'static str,
    pub key: String,
    pub action: Action,
    pub id: String,
}

/// Why a run stopped before its plan was applied.
#[derive(Debug)]
pub enum Stop {
    /// Roblox refused or failed a call. `intent` is true when it was a create that may have landed, whose intent stays in
    /// the journal. A create Roblox refused had its intent cleared before the stop.
    Remote {
        kind: Kind,
        key: String,
        failure: Failure,
        intent: bool,
    },
    /// Roblox refused or failed the update of the experience settings. It is the last write, so every other effect was
    /// applied. It writes absolute values, so a rerun repeats it.
    Settings(Failure),
    Write {
        path: PathBuf,
        error: io::Error,
    },
    /// An intent would be written while another key's create may have landed. The plan's order rules it out, so this
    /// is a bug, and the run stops before the create.
    Unresolved {
        kind: Kind,
        key: String,
        other: String,
    },
}

/// A write a run will send to Roblox, for the scopes it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    Create(Kind),
    /// An update of a resource. A badge's details and its icon go through two operations.
    Update {
        kind: Kind,
        details: bool,
        icon: bool,
    },
    Settings,
}

enum Effect {
    /// One journal write that drops these intents.
    Prune(Vec<(Kind, String)>),
    /// One lock write that sets `key`'s entry, or removes it when `entry` is `None`, and removes `from`'s.
    Record {
        kind: Kind,
        key: String,
        from: Option<String>,
        entry: Option<Entry>,
        action: Action,
    },
    Update {
        kind: Kind,
        key: String,
        id: Id,
        patch: Patch,
    },
    Settings(ResolvedSettings),
    /// One lock write that records the fingerprint of the settings the run applied.
    RecordSettings(String),
    Intend {
        kind: Kind,
        key: String,
        name: String,
    },
    Create {
        kind: Kind,
        key: String,
        fields: Fields,
        icon: Option<(Image, String)>,
    },
    Clear {
        kind: Kind,
        key: String,
    },
}

pub struct Run {
    env: String,
    lock: Lock,
    universe: Id,
    journal: Journal,
    queue: VecDeque<Effect>,
    /// The IDs of each kind the run listed or got from a create's answer.
    known: BTreeMap<Kind, BTreeSet<Id>>,
    applied: Vec<Applied>,
}

impl Run {
    /// The effects that apply `plan`, which `plan::plan` made from `inputs` with what Roblox holds. `lock` is the whole
    /// file, other environments included, because each write replaces it. `read` reads icons.
    pub fn new(
        inputs: &Inputs<'_>,
        plan: &Plan,
        lock: &Lock,
        read: &mut dyn FnMut(&Path) -> io::Result<Vec<u8>>,
    ) -> io::Result<Self> {
        let observed = inputs.observed.expect("provision plans with what Roblox holds");
        let universe = inputs.env.universe;
        let journal = inputs.journal.cloned().unwrap_or_else(|| Journal::new(universe));
        let resources = inputs.experience.resolve(inputs.env);
        let entries = |kind: Kind| inputs.lock.map(|lock| lock.entries(kind));
        let remote = |kind: Kind, id: Id| {
            let listed = observed.listed.get(&kind).into_iter().flatten().find(|listed| listed.id == id);
            listed.or_else(|| observed.missing.get(&(kind, id)).and_then(Option::as_ref))
        };
        // An intent the lock resolves: its key, or the key it moved to, holds an ID.
        let mut prune: Vec<(Kind, String)> = journal
            .intents
            .iter()
            .filter(|intent| {
                let locked = |key: &String| entries(intent.kind).is_some_and(|entries| entries.contains_key(key));
                locked(&intent.key)
                    || resources.iter().any(|resource| {
                        resource.kind == intent.kind
                            && resource.key == intent.key
                            && resource.moved_from.as_ref().is_some_and(locked)
                    })
            })
            .map(|intent| (intent.kind, intent.key.clone()))
            .collect();
        let (mut adopts, mut changes, mut creates, mut settings) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        if let Some(declared) = inputs.experience.settings()
            && plan.steps.iter().any(|step| step.kind == "experience" && step.action == Action::Update)
        {
            let write = declared.unheld(observed.held_settings());
            if !write.is_empty() {
                settings.push(Effect::Settings(write));
            }
            settings.push(Effect::RecordSettings(declared.fingerprint()));
        }
        for step in &plan.steps {
            let Some(resource) =
                resources.iter().find(|resource| resource.kind.noun() == step.kind && resource.key == step.key)
            else {
                continue;
            };
            let kind = resource.kind;
            // The intent of this key, or of the key it moved from.
            let intents: Vec<String> = journal
                .intents
                .iter()
                .filter(|intent| {
                    intent.kind == kind
                        && (intent.key == resource.key || resource.moved_from.as_ref() == Some(&intent.key))
                })
                .map(|intent| intent.key.clone())
                .collect();
            let entry = entries(kind).and_then(|entries| {
                entries.get(&resource.key).or_else(|| step.moved_from.as_ref().and_then(|old| entries.get(old)))
            });
            match step.action {
                Action::Create => {
                    prune.extend(intents.into_iter().map(|key| (kind, key)));
                    let fields = resource.fields.clone().expect("a managed resource has its fields");
                    let icon = icon(resource, read)?;
                    creates.push(Effect::Intend { kind, key: resource.key.clone(), name: fields.name().to_owned() });
                    creates.push(Effect::Create { kind, key: resource.key.clone(), fields, icon });
                }
                Action::Adopt => {
                    let id: Id = step.id.as_ref().and_then(|id| id.parse().ok()).expect("an adopt names its ID");
                    let fields = resource.fields.as_ref().expect("a managed resource has its fields");
                    let entry = Entry { id, fingerprint: fields.fingerprint(), icon: None, status: Status::Managed };
                    adopts.push(record(resource, None, Some(entry.clone()), Action::Adopt));
                    adopts.extend(intents.into_iter().map(|key| Effect::Clear { kind, key }));
                    let remote = remote(kind, id).expect("an adopted resource was listed");
                    adopts.extend(update(resource, &entry, Some(remote), None, true, read)?);
                }
                _ => {
                    let Some(entry) = entry else { continue };
                    let remote = remote(kind, entry.id);
                    let from = step.moved_from.clone();
                    match step.action {
                        Action::Untrack => changes.push(record(resource, from, None, Action::Untrack)),
                        Action::Retire => {
                            let patch = Patch { on: Some(false), ..Patch::default() };
                            // A retire of a resource Roblox no longer has would fail on every run. The plan accepts
                            // it gone, so only the lock records it retired.
                            if remote.is_some_and(|remote| remote.on) {
                                changes.push(Effect::Update { kind, key: resource.key.clone(), id: entry.id, patch });
                            }
                            let retired = Entry { status: Status::Retired, ..entry.clone() };
                            changes.push(record(resource, from, Some(retired), Action::Retire));
                        }
                        Action::Update => {
                            let icon_changed = step.changes.contains(&Change::Icon);
                            changes.extend(update(resource, entry, remote, from, icon_changed, read)?);
                        }
                        _ => {}
                    }
                }
            }
        }
        let mut queue = VecDeque::new();
        if !prune.is_empty() {
            queue.push_back(Effect::Prune(prune));
        }
        queue.extend(adopts);
        queue.extend(changes);
        queue.extend(creates);
        queue.extend(settings);
        let mut known = BTreeMap::new();
        for (kind, listed) in &observed.listed {
            let ids: &mut BTreeSet<Id> = known.entry(*kind).or_default();
            ids.extend(listed.iter().map(|listed| listed.id));
        }
        for (kind, id) in observed.missing.iter().filter(|(_, found)| found.is_some()).map(|(at, _)| at) {
            known.entry(*kind).or_default().insert(*id);
        }
        Ok(Self {
            env: inputs.env_name.to_owned(),
            lock: lock.clone(),
            universe,
            journal,
            queue,
            known,
            applied: Vec::new(),
        })
    }

    /// The writes the run has left to send, in order.
    pub fn writes(&self) -> Vec<Write> {
        self.queue
            .iter()
            .filter_map(|effect| match effect {
                Effect::Create { kind, .. } => Some(Write::Create(*kind)),
                Effect::Update { kind, patch, .. } => Some(Write::Update {
                    kind: *kind,
                    details: patch.name.is_some()
                        || patch.description.is_some()
                        || patch.price.is_some()
                        || patch.on.is_some(),
                    icon: patch.icon.is_some(),
                }),
                Effect::Settings(_) => Some(Write::Settings),
                _ => None,
            })
            .collect()
    }

    /// What the run has done so far.
    pub fn applied(&self) -> &[Applied] {
        &self.applied
    }

    /// Makes one remote call or writes one file. Returns false once nothing is left.
    pub fn step(&mut self, remote: &mut dyn Remote, store: &mut dyn Store) -> Result<bool, Stop> {
        let Some(effect) = self.queue.pop_front() else { return Ok(false) };
        match effect {
            Effect::Prune(intents) => {
                for (kind, key) in intents {
                    self.journal.remove(kind, &key);
                }
                self.save_journal(store)?;
            }
            Effect::Record { kind, key, from, entry, action } => {
                let entries = self.lock.env_mut(&self.env, self.universe).entries_mut(kind);
                if let Some(old) = &from {
                    entries.remove(old);
                }
                let id = match entry {
                    Some(entry) => {
                        let id = entry.id;
                        entries.insert(key.clone(), entry);
                        Some(id)
                    }
                    None => entries.remove(&key).map(|entry| entry.id),
                };
                self.save_lock(store)?;
                let id = id.map(|id| id.to_string()).unwrap_or_default();
                self.applied.push(Applied { kind: kind.noun(), key, action, id });
            }
            Effect::Update { kind, key, id, patch } => {
                remote.update(kind, id, &patch).map_err(|failure| Stop::Remote {
                    kind,
                    key,
                    failure,
                    intent: false,
                })?;
            }
            Effect::Settings(settings) => remote.update_settings(&settings).map_err(Stop::Settings)?,
            Effect::RecordSettings(fingerprint) => {
                self.lock.env_mut(&self.env, self.universe).experience = Some(crate::lock::Applied { fingerprint });
                self.save_lock(store)?;
                let id = self.universe.to_string();
                self.applied.push(Applied { kind: "experience", key: "settings".into(), action: Action::Update, id });
            }
            Effect::Intend { kind, key, name } => {
                let env = self.lock.env(&self.env);
                let unresolved = self.journal.intents.iter().find(|intent| {
                    !(intent.kind == kind && intent.key == key)
                        && env.is_none_or(|env| !env.entries(intent.kind).contains_key(&intent.key))
                });
                if let Some(other) = unresolved {
                    return Err(Stop::Unresolved { kind, key, other: other.key.clone() });
                }
                let before = Some(self.known.get(&kind).cloned().unwrap_or_default());
                self.journal.remove(kind, &key);
                self.journal.intents.push(Intent { kind, key, name, before });
                self.save_journal(store)?;
            }
            Effect::Create { kind, key, fields, icon } => {
                let image = icon.as_ref().map(|(image, _)| image);
                let id = match remote.create(kind, &fields, image) {
                    Ok(id) => id,
                    Err(failure) => {
                        // A crash before this write leaves the intent of a create that made nothing, which the next
                        // run's adopt check handles as it handles a create lost before it landed.
                        let intent = may_have_landed(&failure);
                        if !intent {
                            self.journal.remove(kind, &key);
                            self.save_journal(store)?;
                        }
                        return Err(Stop::Remote { kind, key, failure, intent });
                    }
                };
                self.known.entry(kind).or_default().insert(id);
                let entry = Entry {
                    id,
                    fingerprint: fields.fingerprint(),
                    icon: icon.map(|(_, hash)| hash),
                    status: Status::Managed,
                };
                self.queue.push_front(Effect::Clear { kind, key: key.clone() });
                self.queue.push_front(Effect::Record {
                    kind,
                    key,
                    from: None,
                    entry: Some(entry),
                    action: Action::Create,
                });
            }
            Effect::Clear { kind, key } => {
                self.journal.remove(kind, &key);
                self.save_journal(store)?;
            }
        }
        Ok(!self.queue.is_empty())
    }

    fn save_lock(&self, store: &mut dyn Store) -> Result<(), Stop> {
        store.save_lock(&self.lock).map_err(|error| Stop::Write { path: crate::lock::FILE.into(), error })
    }

    fn save_journal(&self, store: &mut dyn Store) -> Result<(), Stop> {
        store
            .save_journal(&self.env, &self.journal)
            .map_err(|error| Stop::Write { path: crate::journal::path(&self.env), error })
    }
}

fn record(resource: &Resource, from: Option<String>, entry: Option<Entry>, action: Action) -> Effect {
    Effect::Record { kind: resource.kind, key: resource.key.clone(), from, entry, action }
}

/// The writes that bring a managed resource in line with its declaration, then the lock entry that records it, under
/// the key `from` moved to when it is set.
fn update(
    resource: &Resource,
    entry: &Entry,
    remote: Option<&Listed>,
    from: Option<String>,
    icon_changed: bool,
    read: &mut dyn FnMut(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<Vec<Effect>> {
    let fields = resource.fields.as_ref().expect("a managed resource has its fields");
    let mut patch = Patch::default();
    let (name, description, price, on) = match fields {
        Fields::Priced { name, description, price } => (name, description, Some(*price), true),
        Fields::Badge { name, description, enabled } => (name, description, None, *enabled),
    };
    if remote.is_none_or(|remote| remote.name != *name) {
        patch.name = Some(name.clone());
    }
    if remote.is_none_or(|remote| remote.description != *description) {
        patch.description = Some(description.clone());
    }
    if price.is_some_and(|price| remote.is_none_or(|remote| remote.price != Some(price))) {
        patch.price = price;
    }
    if remote.is_none_or(|remote| remote.on != on) {
        patch.on = Some(on);
    }
    let mut hash = entry.icon.clone();
    let icon = if icon_changed || entry.icon.is_none() { icon(resource, read)? } else { None };
    if let Some((image, digest)) = icon {
        patch.icon = Some(image);
        hash = Some(digest);
    }
    let mut effects = Vec::new();
    if !patch.is_empty() {
        effects.push(Effect::Update { kind: resource.kind, key: resource.key.clone(), id: entry.id, patch });
    }
    let applied = Entry { id: entry.id, fingerprint: fields.fingerprint(), icon: hash, status: Status::Managed };
    if *entry != applied || !effects.is_empty() || from.is_some() {
        effects.push(record(resource, from, Some(applied), Action::Update));
    }
    Ok(effects)
}

/// The declared icon and its content hash, for a managed resource that has one.
fn icon(
    resource: &Resource,
    read: &mut dyn FnMut(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<Option<(Image, String)>> {
    let Some(path) = &resource.icon else { return Ok(None) };
    let bytes = read(path)?;
    let hash = content_hash(&bytes);
    let file_name = path.file_name().map_or_else(|| "icon".into(), |name| name.to_string_lossy().into_owned());
    Ok(Some((Image { file_name, content_type: content_type(path).into(), bytes }, hash)))
}

fn content_type(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "bmp" => "image/bmp",
        "tga" => "image/tga",
        _ => "application/octet-stream",
    }
}

/// Why `adopt` wrote nothing.
#[derive(Debug)]
pub enum Refused {
    /// The files or Roblox rule the ID out. The message says why.
    Rule(String),
    Remote(Failure),
    Write {
        path: PathBuf,
        error: io::Error,
    },
}

/// `crucible adopt <env> <kind> <key> <id>`: records `id` for `key` after checking on Roblox that it is one of the key's
/// candidates and no other unbound key's, then clears the key's intent. It writes the lock before the journal, like a
/// run's adopt. `inputs` is the offline plan's, and `lock` the whole file.
pub fn adopt(
    inputs: &Inputs<'_>,
    lock: &Lock,
    kind: Kind,
    key: &str,
    id: Id,
    remote: &mut dyn Remote,
    store: &mut dyn Store,
) -> Result<Applied, Refused> {
    let rule = |message: String| Refused::Rule(message);
    let (table, noun) = (kind.table(), kind.noun());
    same_universe(inputs)?;
    let resources = inputs.experience.resolve(inputs.env);
    let declared: Vec<&Resource> = resources.iter().filter(|resource| resource.kind == kind).collect();
    let resource = declared
        .iter()
        .find(|resource| resource.key == key)
        .ok_or_else(|| rule(format!("crucible.toml declares no {table}.{key}")))?;
    if resource.lifecycle != Lifecycle::Managed {
        return Err(rule(format!("{table}.{key} is retired or untracked, so provision keeps no ID for it")));
    }
    let empty = BTreeMap::new();
    let entries = inputs.lock.map_or(&empty, |lock| lock.entries(kind));
    let bound = |resource: &Resource| {
        entries.contains_key(&resource.key) || resource.moved_from.as_ref().is_some_and(|old| entries.contains_key(old))
    };
    if bound(resource) {
        return Err(rule(format!("crucible.lock already holds an ID for {table}.{key} in {}", inputs.env_name)));
    }
    if let Some((other, _)) = entries.iter().find(|(_, entry)| entry.id == id) {
        return Err(rule(format!("crucible.lock holds {noun} {id} for {table}.{other}")));
    }
    let intent_of = |resource: &Resource| {
        inputs.journal.and_then(|journal| {
            journal.intents.iter().find(|intent| {
                intent.kind == kind && (intent.key == resource.key || resource.moved_from.as_ref() == Some(&intent.key))
            })
        })
    };
    let listed = remote.list(kind).map_err(Refused::Remote)?;
    let found = match listed.into_iter().find(|listed| listed.id == id) {
        Some(found) => found,
        None => match remote.get(kind, id).map_err(Refused::Remote)? {
            Some(found) => found,
            None if kind == Kind::Badge => {
                return Err(rule(format!(
                    "universe {} has no badge {id}. Roblox reads a badge by ID in any universe, and provision takes \
                     none that belongs to another universe, so check that {id} is a badge of this experience",
                    inputs.env.universe
                )));
            }
            None => return Err(rule(format!("universe {} has no {noun} {id}", inputs.env.universe))),
        },
    };
    if !crate::plan::is_candidate(&found, resource, intent_of(resource)) {
        return Err(rule(format!(
            "{noun} {id} is named {:?}, and {table}.{key} is declared as {:?}. Provision adopts a resource by its \
             declared name, or by an ID that is new since the key's unresolved intent. Give {table}.{key} that name, \
             or rename the {noun} on Roblox, then adopt it",
            found.name,
            resource.fields.as_ref().map_or("", Fields::name)
        )));
    }
    for other in
        declared.iter().filter(|other| other.key != key && other.lifecycle == Lifecycle::Managed && !bound(other))
    {
        if crate::plan::is_candidate(&found, other, intent_of(other)) {
            return Err(rule(format!(
                "{noun} {id} could also be {table}.{}, whose adopt check finds it too, so provision cannot tell \
                 whose it is. Rename it on Roblox, or give {table}.{} another name, then adopt it",
                other.key, other.key
            )));
        }
    }
    let fields = resource.fields.as_ref().expect("a managed resource has its fields");
    let mut lock = lock.clone();
    let entry = Entry { id, fingerprint: fields.fingerprint(), icon: None, status: Status::Managed };
    lock.env_mut(inputs.env_name, inputs.env.universe).entries_mut(kind).insert(key.to_owned(), entry);
    store.save_lock(&lock).map_err(|error| Refused::Write { path: crate::lock::FILE.into(), error })?;
    if let Some(mut journal) = inputs.journal.cloned() {
        journal.intents.retain(|intent| {
            !(intent.kind == kind && (intent.key == key || resource.moved_from.as_ref() == Some(&intent.key)))
        });
        store
            .save_journal(inputs.env_name, &journal)
            .map_err(|error| Refused::Write { path: crate::journal::path(inputs.env_name), error })?;
    }
    Ok(Applied { kind: noun, key: key.to_owned(), action: Action::Adopt, id: id.to_string() })
}

fn same_universe(inputs: &Inputs<'_>) -> Result<(), Refused> {
    let universe = inputs.env.universe;
    let elsewhere =
        inputs.lock.map(EnvLock::universe).into_iter().chain(inputs.journal.map(|journal| journal.universe));
    match elsewhere.into_iter().find(|other| *other != universe) {
        Some(other) => Err(Refused::Rule(format!(
            "crucible.lock or the journal records {} in universe {other}, and crucible.toml names universe {universe}. \
             Run `crucible plan {}` for what to do",
            inputs.env_name, inputs.env_name
        ))),
        None => Ok(()),
    }
}

/// `crucible adopt <env> <kind> <key> --none`: clears the key's unresolved intent, and the intent of the key it moved
/// from, after a person has looked and found none of the resources the adopt check would see to be its create. It
/// refuses while the lock holds the key's ID, because the create landed and provision clears the intent, and while a
/// resource carries the key's declared name, because the next run would adopt it. Returns what it did and the IDs the
/// adopt check would have seen, which the person passed over.
pub fn forget(
    inputs: &Inputs<'_>,
    kind: Kind,
    key: &str,
    remote: &mut dyn Remote,
    store: &mut dyn Store,
) -> Result<(Applied, Vec<Id>), Refused> {
    let rule = |message: String| Refused::Rule(message);
    let (table, noun, env) = (kind.table(), kind.noun(), inputs.env_name);
    same_universe(inputs)?;
    let resources = inputs.experience.resolve(inputs.env);
    let resource = resources.iter().find(|resource| resource.kind == kind && resource.key == key);
    let moved_from = resource.and_then(|resource| resource.moved_from.as_ref());
    let ours = |intent: &Intent| intent.kind == kind && (intent.key == key || moved_from == Some(&intent.key));
    let intents: Vec<&Intent> =
        inputs.journal.map_or(&[][..], |journal| &journal.intents).iter().filter(|i| ours(i)).collect();
    if intents.is_empty() {
        return Err(rule(format!("the journal holds no intent for {table}.{key} in {env}")));
    }
    let empty = BTreeMap::new();
    let entries = inputs.lock.map_or(&empty, |lock| lock.entries(kind));
    if let Some(entry) = entries.get(key).or_else(|| moved_from.and_then(|old| entries.get(old))) {
        return Err(rule(format!(
            "crucible.lock holds {noun} {} for {table}.{key}, so its create landed, and provision clears the intent",
            entry.id
        )));
    }
    let locked: BTreeSet<Id> = entries.values().map(|entry| entry.id).collect();
    let listed = remote.list(kind).map_err(Refused::Remote)?;
    let unbound = listed.iter().filter(|listed| !locked.contains(&listed.id));
    let mut passed = Vec::new();
    for listed in unbound {
        if let Some(resource) = resource.filter(|resource| resource.lifecycle == Lifecycle::Managed)
            && resource.fields.as_ref().is_some_and(|fields| fields.name() == listed.name)
        {
            return Err(rule(format!(
                "{noun} {} carries {table}.{key}'s declared name {:?}, so the next provision would adopt it. If it is \
                 {table}.{key}'s, run `crucible adopt {env} {noun} {key} {}`. If it is not, rename it on Roblox first",
                listed.id, listed.name, listed.id
            )));
        }
        let new = |intent: &&Intent| match &intent.before {
            Some(before) => !before.contains(&listed.id),
            None => listed.name == intent.name,
        };
        if intents.iter().any(new) {
            passed.push(listed.id);
        }
    }
    let mut journal = inputs.journal.cloned().expect("the journal holds the intent");
    journal.intents.retain(|intent| !ours(intent));
    store.save_journal(env, &journal).map_err(|error| Refused::Write { path: crate::journal::path(env), error })?;
    Ok((Applied { kind: noun, key: key.to_owned(), action: Action::Forget, id: String::new() }, passed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_s_content_type_follows_its_extension() {
        for (path, expected) in [
            ("icons/coin.png", "image/png"),
            ("coin.PNG", "image/png"),
            ("coin.jpg", "image/jpeg"),
            ("coin.jpeg", "image/jpeg"),
            ("coin.bmp", "image/bmp"),
            ("coin.tga", "image/tga"),
            ("coin.webp", "application/octet-stream"),
            ("coin", "application/octet-stream"),
        ] {
            assert_eq!(content_type(Path::new(path)), expected, "{path}");
        }
    }
}
