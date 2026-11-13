use crucible_iac::journal::{Intent, Journal};
use crucible_iac::lock::{Entry, Lock, Status};
use crucible_iac::model::{Experience, Fields, Kind, ResolvedSettings};
use crucible_iac::plan::{Action, Id, Inputs, Listed};
use crucible_iac::provision::{Applied, Patch, Refused, Remote, Store, adopt, forget};
use crucible_oc::ops::Image;
use crucible_oc::outcome::Failure;
use crucible_project::Manifest;
use std::collections::BTreeSet;
use std::io;

const CONFIG: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 1
place = 2

[products.alpha]
name = "Alpha"
price = 1

[products.beta]
name = "Beta"
price = 2

[products.moved]
name = "Moved"
price = 3
moved_from = "old"

[products.gone]
name = "Gone"
price = 4
retired = true

[badges.medal]
name = "Medal"
"#;

fn id(value: u64) -> Id {
    Id::new(value).unwrap()
}

fn product(id: u64, name: &str) -> Listed {
    Listed { id: self::id(id), name: name.into(), description: String::new(), price: Some(1), on: true }
}

struct Roblox {
    listed: Vec<Listed>,
    alone: Vec<Listed>,
}

impl Remote for Roblox {
    fn list(&mut self, _: Kind) -> Result<Vec<Listed>, Failure> {
        Ok(self.listed.clone())
    }
    fn get(&mut self, _: Kind, id: Id) -> Result<Option<Listed>, Failure> {
        Ok(self.alone.iter().find(|listed| listed.id == id).cloned())
    }
    fn create(&mut self, _: Kind, _: &Fields, _: Option<&Image>) -> Result<Id, Failure> {
        unreachable!("adopt never creates")
    }
    fn update(&mut self, _: Kind, _: Id, _: &Patch) -> Result<(), Failure> {
        unreachable!("adopt never updates")
    }
    fn update_settings(&mut self, _: &ResolvedSettings) -> Result<(), Failure> {
        unreachable!("adopt never writes settings")
    }
}

#[derive(Default)]
struct Disk {
    lock: Option<Lock>,
    journal: Option<Journal>,
}

impl Store for Disk {
    fn save_lock(&mut self, lock: &Lock) -> io::Result<()> {
        assert!(self.journal.is_none(), "the lock is written before the journal");
        self.lock = Some(lock.clone());
        Ok(())
    }
    fn save_journal(&mut self, _: &str, journal: &Journal) -> io::Result<()> {
        self.journal = Some(journal.clone());
        Ok(())
    }
}

fn intent(key: &str, name: &str, before: &[u64]) -> Intent {
    Intent {
        kind: Kind::Product,
        key: key.into(),
        name: name.into(),
        before: Some(before.iter().map(|value| id(*value)).collect::<BTreeSet<_>>()),
    }
}

fn journal(intents: Vec<Intent>) -> Option<Journal> {
    let mut journal = Journal::new(id(1));
    journal.intents = intents;
    Some(journal)
}

fn locked(keys: &[(&str, u64)], universe: u64) -> Lock {
    let mut lock = Lock::empty();
    let env = lock.env_mut("staging", id(universe));
    for (key, value) in keys {
        let entry = Entry { id: id(*value), fingerprint: String::new(), icon: None, status: Status::Managed };
        env.entries_mut(Kind::Product).insert((*key).into(), entry);
    }
    Lock::parse(&lock.render()).unwrap()
}

struct Case {
    lock: Lock,
    journal: Option<Journal>,
    roblox: Roblox,
}

impl Case {
    fn new(listed: Vec<Listed>) -> Self {
        Case { lock: Lock::empty(), journal: None, roblox: Roblox { listed, alone: vec![] } }
    }

    fn adopt(&mut self, key: &str, value: u64) -> (Result<Applied, Refused>, Disk) {
        self.adopt_as(Kind::Product, key, value)
    }

    fn adopt_as(&mut self, kind: Kind, key: &str, value: u64) -> (Result<Applied, Refused>, Disk) {
        let manifest = Manifest::parse(CONFIG).unwrap();
        let experience = Experience::new(&manifest).unwrap();
        let inputs = Inputs {
            experience: &experience,
            env_name: "staging",
            env: &manifest.envs["staging"],
            lock: self.lock.env("staging"),
            journal: self.journal.as_ref(),
            observed: None,
        };
        let mut disk = Disk::default();
        let result = adopt(&inputs, &self.lock, kind, key, id(value), &mut self.roblox, &mut disk);
        (result, disk)
    }

    fn forget(&mut self, key: &str) -> (Result<(Applied, Vec<Id>), Refused>, Disk) {
        let manifest = Manifest::parse(CONFIG).unwrap();
        let experience = Experience::new(&manifest).unwrap();
        let inputs = Inputs {
            experience: &experience,
            env_name: "staging",
            env: &manifest.envs["staging"],
            lock: self.lock.env("staging"),
            journal: self.journal.as_ref(),
            observed: None,
        };
        let mut disk = Disk::default();
        let result = forget(&inputs, Kind::Product, key, &mut self.roblox, &mut disk);
        (result, disk)
    }

    fn refused_forget(&mut self, key: &str) -> String {
        match self.forget(key) {
            (Err(Refused::Rule(message)), disk) => {
                assert!(disk.lock.is_none() && disk.journal.is_none(), "a refusal writes nothing");
                message
            }
            (other, _) => panic!("{other:?}"),
        }
    }

    fn refused(&mut self, key: &str, value: u64) -> String {
        match self.adopt(key, value) {
            (Err(Refused::Rule(message)), disk) => {
                assert!(disk.lock.is_none() && disk.journal.is_none(), "a refusal writes nothing");
                message
            }
            (other, _) => panic!("{other:?}"),
        }
    }
}

#[test]
fn adopt_refuses_what_the_files_rule_out() {
    let mut case = Case::new(vec![product(5, "Alpha")]);
    assert_eq!(case.refused("nope", 5), "crucible.toml declares no products.nope");
    assert_eq!(case.refused("gone", 5), "products.gone is retired or untracked, so provision keeps no ID for it");

    case.lock = locked(&[], 9);
    assert!(case.refused("alpha", 5).starts_with("crucible.lock or the journal records staging in universe 9"));
    case.lock = Lock::empty();
    case.journal = Some(Journal::new(id(9)));
    assert!(case.refused("alpha", 5).contains("in universe 9, and crucible.toml names universe 1"));
    case.journal = None;

    case.lock = locked(&[("alpha", 6)], 1);
    assert_eq!(case.refused("alpha", 5), "crucible.lock already holds an ID for products.alpha in staging");
    case.lock = locked(&[("old", 6)], 1);
    assert_eq!(case.refused("moved", 5), "crucible.lock already holds an ID for products.moved in staging");
    case.lock = locked(&[("beta", 5)], 1);
    assert_eq!(case.refused("alpha", 5), "crucible.lock holds product 5 for products.beta");
}

#[test]
fn adopt_refuses_what_roblox_rules_out() {
    let mut case = Case::new(vec![product(5, "Other")]);
    assert_eq!(case.refused("alpha", 7), "universe 1 has no product 7");
    assert!(
        case.refused("alpha", 5)
            .starts_with("product 5 is named \"Other\", and products.alpha is declared as \"Alpha\"")
    );

    case.journal = journal(vec![intent("beta", "Beta", &[])]);
    case.roblox.listed = vec![product(5, "Alpha")];
    let message = case.refused("alpha", 5);
    assert!(message.starts_with("product 5 could also be products.beta, whose adopt check finds it too"), "{message}");
    let message = case.refused("beta", 5);
    assert!(message.starts_with("product 5 could also be products.alpha, whose adopt check finds it too"), "{message}");
}

#[test]
fn adopt_says_a_badge_it_cannot_read_may_belong_to_another_universe() {
    let mut case = Case::new(vec![]);
    match case.adopt_as(Kind::Badge, "medal", 7) {
        (Err(Refused::Rule(message)), disk) => {
            assert!(disk.lock.is_none() && disk.journal.is_none(), "a refusal writes nothing");
            assert!(
                message.starts_with("universe 1 has no badge 7. Roblox reads a badge by ID in any universe"),
                "{message}"
            );
        }
        (other, _) => panic!("{other:?}"),
    }
}

#[test]
fn adopt_writes_the_lock_then_clears_the_key_s_intents() {
    let mut case = Case::new(vec![product(5, "Renamed"), product(6, "Beta")]);
    case.roblox.alone = vec![product(8, "Alpha")];
    case.journal = journal(vec![intent("old", "Moved", &[6]), intent("beta", "Beta", &[5, 6])]);
    let (applied, disk) = case.adopt("moved", 5);
    let applied = applied.unwrap();
    assert_eq!(
        (applied.kind, applied.key.as_str(), applied.action, applied.id.as_str()),
        ("product", "moved", Action::Adopt, "5")
    );
    let lock = disk.lock.unwrap();
    let entry = &lock.env("staging").unwrap().entries(Kind::Product)["moved"];
    assert_eq!((entry.id, entry.status, entry.icon.as_deref()), (id(5), Status::Managed, None));
    let fields = Fields::Priced { name: "Moved".into(), description: String::new(), price: 3 };
    assert_eq!(entry.fingerprint, fields.fingerprint());
    assert_eq!(disk.journal.unwrap().intents, [intent("beta", "Beta", &[5, 6])], "only the key's intent is cleared");

    case.journal = journal(vec![intent("alpha", "Alpha", &[])]);
    case.roblox.listed = vec![product(9, "Gone")];
    let (applied, _) = case.adopt("alpha", 9);
    assert_eq!(applied.unwrap().id, "9", "a retired key with the name never adopts, so it never shares a candidate");

    case.lock = locked(&[("beta", 6)], 1);
    case.roblox.listed = vec![product(5, "Beta"), product(6, "Beta")];
    let (applied, _) = case.adopt("alpha", 5);
    assert_eq!(applied.unwrap().id, "5", "a key the lock binds never shares a candidate");
    case.lock = Lock::empty();

    case.journal = None;
    let (applied, disk) = case.adopt("alpha", 8);
    assert_eq!(applied.unwrap().id, "8", "a resource the list leaves out is read by its ID");
    assert!(disk.journal.is_none(), "no journal, no journal write");
}

#[test]
fn adopt_none_clears_an_unresolved_intent_and_names_what_was_passed_over() {
    let mut case = Case::new(vec![product(5, "Someone's"), product(6, "Beta"), product(7, "Old")]);
    assert_eq!(case.refused_forget("alpha"), "the journal holds no intent for products.alpha in staging");

    case.journal = journal(vec![intent("old", "Moved", &[7]), intent("beta", "Beta", &[5, 6, 7])]);
    case.lock = locked(&[("old", 7)], 1);
    assert_eq!(
        case.refused_forget("moved"),
        "crucible.lock holds product 7 for products.moved, so its create landed, and provision clears the intent"
    );
    case.lock = Lock::empty();
    let message = case.refused_forget("beta");
    assert!(message.starts_with("product 6 carries products.beta's declared name \"Beta\""), "{message}");
    assert!(message.contains("run `crucible adopt staging product beta 6`"), "{message}");

    let (done, disk) = case.forget("moved");
    let (applied, passed) = done.unwrap();
    assert_eq!((applied.key.as_str(), applied.action, applied.id.as_str()), ("moved", Action::Forget, ""));
    assert_eq!(passed, [id(5), id(6)], "the IDs new since the intent of the key it moved from");
    assert_eq!(disk.journal.unwrap().intents, [intent("beta", "Beta", &[5, 6, 7])], "only the key's intent goes");
    assert!(disk.lock.is_none(), "it writes no lock");

    case.journal = journal(vec![intent("nope", "Nope", &[5, 6, 7])]);
    let (done, disk) = case.forget("nope");
    assert_eq!(done.unwrap().1, [], "an undeclared key's intent can go too");
    assert_eq!(disk.journal.unwrap().intents, []);

    let sent = Intent { before: None, ..intent("alpha", "Old", &[]) };
    case.journal = journal(vec![sent]);
    let (done, _) = case.forget("alpha");
    assert_eq!(done.unwrap().1, [id(7)], "an intent without IDs passes over only what carries the name it sent");
}
