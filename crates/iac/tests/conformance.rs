//! Drives `provision` against an in-process fake of Roblox with faults, and checks the invariants of
//! `specs/provision/Provision.tla` after every step: at most one resource per key, a lock that names only resources
//! that exist and belong to its key, no create after an unknown outcome without a list, and an intent in the journal for
//! every create whose outcome is unknown. At the end of each case, a developer resolves what the runs stopped on, as
//! the guide says, and fault-free runs must then bind every key.
//!
//! `PROPTEST_CASES=20000 cargo test -p crucible-iac --test conformance --release` runs it deeper than the default 1,000
//! cases.

use crucible_iac::journal::{self, Journal};
use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Fields, Kind, Problem};
use crucible_iac::plan::{Action, Id, Inputs, Listed, plan, undeclared};
use crucible_iac::provision::{Patch, Remote, Run, Stop, Store, adopt, observe};
use crucible_oc::ops::Image;
use crucible_oc::outcome::Failure;
use crucible_project::Manifest;
use proptest::prelude::*;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::Path;

const ENV: &str = "staging";

/// 1,000 cases unless `PROPTEST_CASES` says otherwise.
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(1000);
    ProptestConfig { cases, failure_persistence: None, ..ProptestConfig::default() }
}
const KEYS: [(Kind, &str); 3] = [(Kind::Product, "a"), (Kind::Product, "b"), (Kind::Badge, "c")];

fn names(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Badge => &["B1", "B2"],
        _ => &["N1", "N2", "N3"],
    }
}

type Key = (Kind, String);

/// What happens to one remote call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    /// The call reached Roblox, and its answer was lost.
    LostAfter,
    /// The call was lost before it reached Roblox, and the client cannot tell.
    LostBefore,
    /// Roblox answered with a failure: 5xx on a read or an update, a rejection on a create.
    Refused,
}

#[derive(Debug, Clone)]
struct Resource {
    kind: Kind,
    id: u64,
    name: String,
    description: String,
    price: Option<u64>,
    on: bool,
    /// The ghost owner: the key a create was for, or `None` for a resource someone made by hand.
    owner: Option<String>,
    /// How many more lists leave it out. The public badge list may lag behind a create, which is why provision never
    /// creates a badge again on its own.
    lagging: u8,
}

#[derive(Default)]
struct Disk {
    lock: Option<String>,
    journal: Option<String>,
}

#[derive(Default)]
struct World {
    want: RefCell<BTreeMap<Key, String>>,
    remote: RefCell<Vec<Resource>>,
    disk: RefCell<Disk>,
    faults: RefCell<VecDeque<Fault>>,
    /// Ghost: keys whose create lost its answer, with no list since.
    unknown: RefCell<BTreeSet<Key>>,
    violations: RefCell<Vec<String>>,
    /// The resources the current run sent an update to.
    updated: RefCell<Vec<(Kind, Id)>>,
}

impl World {
    fn config(&self) -> String {
        let mut text =
            "[places.start]\nproject = \"default.project.json\"\n\n[env.staging]\nuniverse = 1\nplace = 2\n".to_owned();
        for ((kind, key), name) in self.want.borrow().iter() {
            text += &format!("\n[{}.{key}]\nname = {name:?}\n", kind.table());
            if *kind != Kind::Badge {
                text += "price = 10\n";
            }
        }
        text
    }

    fn lock(&self) -> Lock {
        self.disk.borrow().lock.as_deref().map_or_else(Lock::empty, |text| Lock::parse(text).expect("the lock parses"))
    }

    fn journal(&self) -> Option<Journal> {
        let disk = self.disk.borrow();
        disk.journal.as_deref().map(|text| Journal::parse(ENV, text).expect("the journal parses"))
    }

    fn bound(&self, kind: Kind, key: &str) -> Option<Id> {
        self.lock().env(ENV).and_then(|env| env.entries(kind).get(key)).map(|entry| entry.id)
    }

    fn fault(&self) -> Fault {
        self.faults.borrow_mut().pop_front().unwrap_or(Fault::None)
    }

    fn violate(&self, what: String) {
        self.violations.borrow_mut().push(what);
    }

    fn make(&self, kind: Kind, name: &str, owner: Option<String>, price: Option<u64>, on: bool) -> Id {
        let mut remote = self.remote.borrow_mut();
        let id = 1000 + remote.len() as u64;
        remote.push(Resource { kind, id, name: name.into(), description: String::new(), price, on, owner, lagging: 0 });
        Id::new(id).unwrap()
    }
}

struct Fake<'w>(&'w World);

fn listed(resource: &Resource) -> Listed {
    Listed {
        id: Id::new(resource.id).unwrap(),
        name: resource.name.clone(),
        description: resource.description.clone(),
        price: resource.price,
        on: resource.on,
    }
}

fn unavailable() -> Failure {
    Failure::unavailable("HTTP 503")
}

impl Remote for Fake<'_> {
    fn list(&mut self, kind: Kind) -> Result<Vec<Listed>, Failure> {
        if self.0.fault() == Fault::Refused {
            return Err(unavailable());
        }
        self.0.unknown.borrow_mut().retain(|(of, _)| *of != kind);
        let mut remote = self.0.remote.borrow_mut();
        let mut shown = Vec::new();
        for resource in remote.iter_mut().filter(|resource| resource.kind == kind) {
            match resource.lagging {
                0 => shown.push(listed(resource)),
                _ => resource.lagging -= 1,
            }
        }
        Ok(shown)
    }

    fn get(&mut self, kind: Kind, id: Id) -> Result<Option<Listed>, Failure> {
        if self.0.fault() == Fault::Refused {
            return Err(unavailable());
        }
        let remote = self.0.remote.borrow();
        Ok(remote.iter().find(|resource| resource.kind == kind && resource.id == id.get()).map(listed))
    }

    fn create(&mut self, kind: Kind, fields: &Fields, _: Option<&Image>) -> Result<Id, Failure> {
        let world = self.0;
        let key = world
            .want
            .borrow()
            .iter()
            .find(|((of, _), name)| *of == kind && *name == fields.name())
            .map(|((_, key), _)| key.clone());
        let Some(key) = key else {
            world.violate(format!("a create of {kind:?} {:?} names no declared key", fields.name()));
            return Err(Failure::usage("no key"));
        };
        // NoBlindCreate: no create after an unknown outcome without a list, and none while a resource the key's adopt
        // check would take exists.
        if world.unknown.borrow().contains(&(kind, key.clone())) {
            world.violate(format!("{kind:?} {key} was created again after an unknown outcome, with no list between"));
        }
        let lock = world.lock();
        let entries = lock.env(ENV).map(|env| env.entries(kind).clone()).unwrap_or_default();
        let locked: BTreeSet<u64> = entries.values().map(|entry| entry.id.get()).collect();
        let journal = world.journal();
        let intent = journal.as_ref().and_then(|journal| journal.get(kind, &key).cloned());
        for resource in
            world.remote.borrow().iter().filter(|resource| resource.kind == kind && !locked.contains(&resource.id))
        {
            let new = intent
                .as_ref()
                .and_then(|intent| intent.before.as_ref())
                .is_some_and(|before| !before.contains(&Id::new(resource.id).unwrap()));
            if resource.name == fields.name() || new {
                world.violate(format!("{kind:?} {key} was created while {} was its candidate", resource.id));
            }
        }
        if intent.is_none() {
            world.violate(format!("{kind:?} {key} was created with no intent in the journal"));
        }
        for other in journal.iter().flat_map(|journal| journal.intents.iter()) {
            if !(other.kind == kind && other.key == key) && !entries.contains_key(&other.key) && other.kind == kind {
                world.violate(format!("{kind:?} {key} was created while {}'s intent was unresolved", other.key));
            }
        }
        let (price, on) = match fields {
            Fields::Priced { price, .. } => (Some(*price), true),
            Fields::Badge { enabled, .. } => (None, *enabled),
        };
        match world.fault() {
            Fault::None => Ok(world.make(kind, fields.name(), Some(key), price, on)),
            Fault::LostAfter => {
                world.make(kind, fields.name(), Some(key.clone()), price, on);
                if kind == Kind::Badge {
                    world.remote.borrow_mut().last_mut().unwrap().lagging = 1;
                }
                world.unknown.borrow_mut().insert((kind, key));
                Err(Failure::unknown("the answer was lost"))
            }
            Fault::LostBefore => {
                world.unknown.borrow_mut().insert((kind, key));
                Err(Failure::unknown("the answer was lost"))
            }
            Fault::Refused => Err(Failure::new(4, "rejected", "HTTP 400")),
        }
    }

    fn update(&mut self, kind: Kind, id: Id, patch: &Patch) -> Result<(), Failure> {
        if *patch == Patch::default() {
            self.0.violate(format!("an update of {kind:?} {id} that sets nothing"));
        }
        self.0.updated.borrow_mut().push((kind, id));
        let fault = self.0.fault();
        if matches!(fault, Fault::None | Fault::LostAfter) {
            let mut remote = self.0.remote.borrow_mut();
            let Some(resource) = remote.iter_mut().find(|resource| resource.kind == kind && resource.id == id.get())
            else {
                return Err(Failure::new(4, "rejected", "HTTP 404").with("status", serde_json::json!(404)));
            };
            if let Some(name) = &patch.name {
                resource.name = name.clone();
            }
            if let Some(description) = &patch.description {
                resource.description = description.clone();
            }
            if patch.price.is_some() {
                resource.price = patch.price;
            }
            if let Some(on) = patch.on {
                resource.on = on;
            }
        }
        match fault {
            Fault::None => Ok(()),
            Fault::Refused => Err(unavailable()),
            _ => Err(Failure::unknown("the answer was lost")),
        }
    }
}

struct Files<'w>(&'w World);

impl Store for Files<'_> {
    fn save_lock(&mut self, lock: &Lock) -> io::Result<()> {
        self.0.disk.borrow_mut().lock = Some(lock.render());
        Ok(())
    }

    fn save_journal(&mut self, _: &str, journal: &Journal) -> io::Result<()> {
        self.0.disk.borrow_mut().journal = (!journal.intents.is_empty()).then(|| journal.render());
        Ok(())
    }
}

fn no_icons(_: &Path) -> io::Result<Vec<u8>> {
    Err(io::ErrorKind::NotFound.into())
}

#[derive(Debug)]
enum Outcome {
    Done,
    Problems(Vec<Problem>),
    Failed(Failure),
    Stopped(Stop),
    Crashed,
}

/// One `crucible provision staging`, as the CLI runs it, that crashes after `crash_after` steps.
fn provision(world: &World, crash_after: Option<usize>) -> Outcome {
    let manifest = Manifest::parse(&world.config()).expect("the config parses");
    let experience = Experience::new(&manifest).expect("the config keeps the rules");
    let lock = world.lock();
    let journal = world.journal();
    let journals: BTreeMap<String, Journal> = journal.iter().map(|journal| (ENV.to_owned(), journal.clone())).collect();
    let env = &manifest.envs[ENV];
    let inputs = |observed| Inputs {
        experience: &experience,
        env_name: ENV,
        env,
        lock: lock.env(ENV),
        journal: journal.as_ref(),
        observed,
    };
    let mut problems = undeclared(&manifest.envs, Some(&lock), &journals);
    if let Err(found) = plan(&inputs(None), &mut no_icons) {
        problems.extend(found);
    }
    if !problems.is_empty() {
        return Outcome::Problems(problems);
    }
    let observed = match observe(&experience, env, lock.env(ENV), journal.as_ref(), &mut Fake(world)) {
        Ok(observed) => observed,
        Err(failure) => return Outcome::Failed(failure),
    };
    let inputs = inputs(Some(&observed));
    let plan = match plan(&inputs, &mut no_icons) {
        Ok(plan) => plan,
        Err(problems) => return Outcome::Problems(problems),
    };
    let unbound: Vec<Key> =
        world.want.borrow().keys().filter(|(kind, key)| world.bound(*kind, key).is_none()).cloned().collect();
    world.updated.borrow_mut().clear();
    let mut run = Run::new(&inputs, &plan, &lock, &mut no_icons).expect("no icons are read");
    for _ in 0..crash_after.unwrap_or(usize::MAX) {
        match run.step(&mut Fake(world), &mut Files(world)) {
            Ok(true) => {}
            Ok(false) => {
                for (kind, key) in &unbound {
                    let Some(id) = world.bound(*kind, key) else { continue };
                    let reported = run.applied().iter().any(|applied| {
                        applied.kind == kind.noun()
                            && applied.key == *key
                            && matches!(applied.action, Action::Create | Action::Adopt)
                            && applied.id == id.to_string()
                    });
                    if !reported {
                        world.violate(format!("{kind:?} {key} was bound to {id} and the run did not report it"));
                    }
                }
                for (kind, id) in world.updated.borrow().iter() {
                    let reported = run.applied().iter().any(|applied| {
                        applied.kind == kind.noun() && applied.action == Action::Update && applied.id == id.to_string()
                    });
                    if !reported {
                        world.violate(format!("{kind:?} {id} was updated and the run did not report it"));
                    }
                }
                return Outcome::Done;
            }
            Err(Stop::Unresolved { kind, key, other }) => {
                world.violate(format!("{kind:?} {key}'s intent was due while {other}'s was unresolved"));
                return Outcome::Crashed;
            }
            Err(stop) => return Outcome::Stopped(stop),
        }
    }
    Outcome::Crashed
}

fn check(world: &World) {
    let violations = world.violations.borrow();
    assert!(violations.is_empty(), "{violations:#?}");
    let remote = world.remote.borrow();
    let lock = world.lock();
    let journal = world.journal();
    for (kind, key) in KEYS {
        let owned: Vec<u64> =
            remote.iter().filter(|r| r.kind == kind && r.owner.as_deref() == Some(key)).map(|r| r.id).collect();
        assert!(owned.len() <= 1, "AtMostOne: {kind:?} {key} owns {owned:?}");
        if let Some(id) = world.bound(kind, key) {
            let resource = remote.iter().find(|r| r.kind == kind && r.id == id.get());
            let resource = resource
                .unwrap_or_else(|| panic!("LockNamesExisting: {kind:?} {key} names {id}, which does not exist"));
            match &resource.owner {
                Some(owner) => {
                    assert_eq!(owner, key, "LockNamesExisting: {kind:?} {key} names {owner}'s resource {id}")
                }
                None => assert!(
                    owned.is_empty(),
                    "LockNamesExisting: {kind:?} {key} names hand-made {id} and owns {owned:?}"
                ),
            }
        }
        if world.unknown.borrow().contains(&(kind, key.to_owned())) && world.bound(kind, key).is_none() {
            assert!(
                journal.as_ref().is_some_and(|journal| journal.get(kind, key).is_some()),
                "JournalCoversUnknown: {kind:?} {key}"
            );
        }
    }
    for kind in [Kind::Product, Kind::Badge] {
        let ids: Vec<_> =
            lock.env(ENV).iter().flat_map(|env| env.entries(kind).values().map(|entry| entry.id)).collect();
        assert_eq!(ids.len(), ids.iter().collect::<BTreeSet<_>>().len(), "two keys share an ID: {ids:?}");
    }
}

/// What the guide tells a developer to do when provision stops, with the ghost knowledge assumption A4 grants: the
/// developer knows which resource provision made for a key.
fn resolve(world: &World) {
    let manifest = Manifest::parse(&world.config()).unwrap();
    let experience = Experience::new(&manifest).unwrap();
    let env = &manifest.envs[ENV];
    for (kind, key) in KEYS {
        if world.bound(kind, key).is_some() {
            continue;
        }
        let owned =
            world.remote.borrow().iter().find(|r| r.kind == kind && r.owner.as_deref() == Some(key)).map(|r| r.id);
        match owned {
            Some(id) => {
                // Give it the declared name, so it is the key's candidate, then adopt it.
                let name = world.want.borrow()[&(kind, key.to_owned())].clone();
                for resource in world.remote.borrow_mut().iter_mut().filter(|r| r.kind == kind) {
                    if resource.id == id {
                        resource.name = name.clone();
                    } else if resource.name == name && resource.owner.is_none() {
                        resource.name = format!("hand {}", resource.id);
                    }
                }
                let lock = world.lock();
                let journal = world.journal();
                let inputs = Inputs {
                    experience: &experience,
                    env_name: ENV,
                    env,
                    lock: lock.env(ENV),
                    journal: journal.as_ref(),
                    observed: None,
                };
                if let Err(refused) =
                    adopt(&inputs, &lock, kind, key, Id::new(id).unwrap(), &mut Fake(world), &mut Files(world))
                {
                    world.violate(format!("crucible adopt refused {kind:?} {key}'s own resource {id}: {refused:?}"));
                }
            }
            None => {
                // Nothing was made for the key: drop its intent, and rename hand-made resources that share its name.
                if let Some(mut journal) = world.journal() {
                    journal.remove(kind, key);
                    Files(world).save_journal(ENV, &journal).unwrap();
                }
                let name = world.want.borrow()[&(kind, key.to_owned())].clone();
                let mut remote = world.remote.borrow_mut();
                let hand: Vec<usize> = (0..remote.len())
                    .filter(|&at| remote[at].kind == kind && remote[at].name == name && remote[at].owner.is_none())
                    .collect();
                for at in hand.into_iter().skip(1) {
                    remote[at].name = format!("hand {}", remote[at].id);
                }
            }
        }
    }
}

/// Fault-free runs, with the developer resolving each stop, bind every key under its declared name.
fn converge(world: &World) {
    world.faults.borrow_mut().clear();
    for _ in 0..4 {
        let outcome = provision(world, None);
        check(world);
        match outcome {
            Outcome::Done => {
                let remote = world.remote.borrow();
                for (kind, key) in KEYS {
                    let id = world.bound(kind, key).unwrap_or_else(|| panic!("{kind:?} {key} is not bound"));
                    let resource = remote.iter().find(|r| r.kind == kind && r.id == id.get()).unwrap();
                    assert_eq!(resource.name, world.want.borrow()[&(kind, key.to_owned())], "{kind:?} {key}");
                    let declared = (String::new(), (kind != Kind::Badge).then_some(10), true);
                    assert_eq!((resource.description.clone(), resource.price, resource.on), declared, "{kind:?} {key}");
                }
                let manifest = Manifest::parse(&world.config()).unwrap();
                let lock = world.lock();
                for resource in Experience::new(&manifest).unwrap().resolve(&manifest.envs[ENV]) {
                    let entry = &lock.env(ENV).unwrap().entries(resource.kind)[&resource.key];
                    let fields = resource.fields.as_ref().unwrap();
                    assert_eq!(
                        entry.fingerprint,
                        fields.fingerprint(),
                        "the lock records what {} declares",
                        resource.key
                    );
                }
                assert!(world.journal().is_none(), "a fault-free run leaves no intent");
                return;
            }
            Outcome::Problems(_) => resolve(world),
            other => panic!("a fault-free run ended with {other:?}"),
        }
        check(world);
    }
    panic!("four fault-free runs did not bind every key: {:?}", world.disk.borrow().lock);
}

#[derive(Debug, Clone)]
struct Reference {
    want: BTreeMap<Key, String>,
    hand: Vec<(Kind, String)>,
}

#[derive(Debug, Clone)]
enum Transition {
    Provision { faults: Vec<Fault>, crash_after: Option<usize> },
    Rename { key: usize, name: usize },
    RenameOnRoblox { pick: usize, name: usize },
    HandMade { badge: bool, name: usize },
}

fn distinct(want: &BTreeMap<Key, String>) -> bool {
    want.iter().all(|((kind, key), name)| {
        want.iter().all(|((of, other), theirs)| of != kind || other == key || theirs != name)
    })
}

fn fault() -> impl Strategy<Value = Fault> {
    prop_oneof![6 => Just(Fault::None), 1 => Just(Fault::LostAfter), 1 => Just(Fault::LostBefore), 1 => Just(Fault::Refused)]
}

impl ReferenceStateMachine for Reference {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        let hand = proptest::collection::vec((any::<bool>(), 0..3usize), 0..3);
        (0..3usize, 0..3usize, 0..2usize, hand)
            .prop_filter("names are distinct", |(a, b, _, _)| a != b)
            .prop_map(|(a, b, c, hand)| Reference {
                want: BTreeMap::from([
                    ((Kind::Product, "a".into()), names(Kind::Product)[a].into()),
                    ((Kind::Product, "b".into()), names(Kind::Product)[b].into()),
                    ((Kind::Badge, "c".into()), names(Kind::Badge)[c].into()),
                ]),
                hand: hand
                    .into_iter()
                    .map(|(badge, name)| {
                        let kind = if badge { Kind::Badge } else { Kind::Product };
                        (kind, names(kind)[name % names(kind).len()].to_owned())
                    })
                    .collect(),
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            6 => (proptest::collection::vec(fault(), 0..6), proptest::option::of(0..8usize))
                .prop_map(|(faults, crash_after)| Transition::Provision { faults, crash_after }),
            1 => (0..3usize, 0..3usize).prop_map(|(key, name)| Transition::Rename { key, name }),
            1 => (0..8usize, 0..4usize).prop_map(|(pick, name)| Transition::RenameOnRoblox { pick, name }),
            1 => (any::<bool>(), 0..3usize).prop_map(|(badge, name)| Transition::HandMade { badge, name }),
        ]
        .boxed()
    }

    fn preconditions(state: &Self, transition: &Transition) -> bool {
        match transition {
            Transition::Rename { key, name } => {
                distinct(&Self::apply(state.clone(), transition).want) && {
                    let (kind, _) = KEYS[*key];
                    *name < names(kind).len()
                }
            }
            _ => true,
        }
    }

    fn apply(mut state: Self, transition: &Transition) -> Self {
        if let Transition::Rename { key, name } = transition {
            let (kind, key) = KEYS[*key];
            if let Some(name) = names(kind).get(*name) {
                state.want.insert((kind, key.to_owned()), (*name).to_owned());
            }
        }
        state
    }
}

struct Conformance;

impl StateMachineTest for Conformance {
    type SystemUnderTest = World;
    type Reference = Reference;

    fn init_test(state: &Reference) -> World {
        let world = World { want: RefCell::new(state.want.clone()), ..World::default() };
        for (kind, name) in &state.hand {
            let price = (*kind != Kind::Badge).then_some(5);
            world.make(*kind, name, None, price, true);
        }
        world
    }

    fn apply(world: World, state: &Reference, transition: Transition) -> World {
        match transition {
            Transition::Provision { faults, crash_after } => {
                *world.faults.borrow_mut() = faults.into();
                match provision(&world, crash_after) {
                    Outcome::Failed(failure) => assert_eq!(failure.code, 7, "only a read fails before the plan"),
                    Outcome::Stopped(Stop::Remote { failure, .. }) => assert!(matches!(failure.code, 4 | 5 | 7)),
                    Outcome::Stopped(stop) => panic!("{stop:?}"),
                    Outcome::Done | Outcome::Problems(_) | Outcome::Crashed => {}
                }
            }
            Transition::Rename { .. } => *world.want.borrow_mut() = state.want.clone(),
            Transition::RenameOnRoblox { pick, name } => {
                let mut remote = world.remote.borrow_mut();
                if !remote.is_empty() {
                    let at = pick % remote.len();
                    let pool = names(remote[at].kind);
                    let edited = &mut remote[at];
                    edited.name = pool.get(name).map_or_else(|| format!("other {name}"), |name| (*name).to_owned());
                    edited.description = "Edited on Roblox.".into();
                    edited.price = edited.price.map(|price| price + 1);
                    edited.on = false;
                }
            }
            Transition::HandMade { badge, name } => {
                let kind = if badge { Kind::Badge } else { Kind::Product };
                world.make(kind, names(kind)[name % names(kind).len()], None, (!badge).then_some(5), true);
            }
        }
        world
    }

    fn check_invariants(world: &World, _: &Reference) {
        check(world);
    }

    fn teardown(world: World, _: Reference) {
        converge(&world);
    }
}

prop_state_machine! {
    #![proptest_config(config())]

    #[test]
    fn provision_keeps_the_model_s_invariants(sequential 1..16 => Conformance);
}

proptest! {
    #![proptest_config(config())]

    /// FaultFreeBinds: with no fault, no rename and no two resources under one declared name, one run binds every key.
    #[test]
    fn a_fault_free_run_binds_every_key(state in Reference::init_state()) {
        let world = Conformance::init_test(&state);
        let remote = world.remote.borrow().clone();
        for ((kind, _), name) in state.want.iter() {
            prop_assume!(remote.iter().filter(|r| r.kind == *kind && r.name == *name).count() <= 1);
        }
        let outcome = provision(&world, None);
        prop_assert!(matches!(outcome, Outcome::Done), "{outcome:?}");
        check(&world);
        for (kind, key) in KEYS {
            prop_assert!(world.bound(kind, key).is_some(), "{kind:?} {key}");
        }
    }
}

#[test]
fn a_crash_between_any_two_steps_leaves_a_state_the_next_run_finishes() {
    for crash_after in 0..12 {
        let world = World::default();
        *world.want.borrow_mut() = BTreeMap::from([
            ((Kind::Product, "a".into()), "N1".into()),
            ((Kind::Product, "b".into()), "N2".into()),
            ((Kind::Badge, "c".into()), "B1".into()),
        ]);
        world.make(Kind::Product, "N2", None, Some(5), false);
        provision(&world, Some(crash_after));
        check(&world);
        converge(&world);
        assert_eq!(world.remote.borrow().len(), 3, "crash after {crash_after}: no resource was made twice");
    }
}

#[test]
fn a_resource_gone_from_roblox_stops_the_run_and_is_never_made_again() {
    let world = World::default();
    *world.want.borrow_mut() = BTreeMap::from([((Kind::Product, "a".into()), "N1".into())]);
    assert!(matches!(provision(&world, None), Outcome::Done));
    world.remote.borrow_mut().clear();
    match provision(&world, None) {
        Outcome::Problems(problems) => {
            assert_eq!(problems.len(), 1);
            assert!(problems[0].message.contains("Roblox has no product 1000"), "{}", problems[0]);
        }
        other => panic!("{other:?}"),
    }
    assert!(world.remote.borrow().is_empty(), "nothing was created again");
    let _ = journal::path(ENV);
}
