use crucible_iac::journal::Journal;
use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Fields, Kind, ResolvedSettings};
use crucible_iac::plan::{Action, Id, Inputs, Listed, plan};
use crucible_iac::provision::{Patch, Remote, Run, Stop, Store, observe};
use crucible_oc::ops::Image;
use crucible_oc::outcome::Failure;
use crucible_project::Manifest;
use std::io;
use std::path::Path;

const PLACES: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 1
place = 2
"#;

#[derive(Default)]
struct Roblox {
    settings: Vec<ResolvedSettings>,
    fail: Option<u8>,
    products: Vec<Listed>,
    /// Each write, in the order the run sent it.
    calls: Vec<&'static str>,
    /// What a read of the universe answers, or `None` when the read fails.
    held: Option<ResolvedSettings>,
}

impl Remote for Roblox {
    fn list(&mut self, kind: Kind) -> Result<Vec<Listed>, Failure> {
        Ok(if kind == Kind::Product { self.products.clone() } else { vec![] })
    }
    fn get(&mut self, _: Kind, _: Id) -> Result<Option<Listed>, Failure> {
        Ok(None)
    }
    fn create(&mut self, _: Kind, _: &Fields, _: Option<&Image>) -> Result<Id, Failure> {
        self.calls.push("create");
        Ok(Id::new(900).unwrap())
    }
    fn update(&mut self, _: Kind, _: Id, _: &Patch) -> Result<(), Failure> {
        self.calls.push("update");
        Ok(())
    }
    fn update_settings(&mut self, settings: &ResolvedSettings) -> Result<(), Failure> {
        self.calls.push("settings");
        self.settings.push(settings.clone());
        match self.fail {
            Some(code) => Err(Failure::new(code, "outcome_unknown", "HTTP 503")),
            None => Ok(()),
        }
    }
    fn read_settings(&mut self) -> Result<ResolvedSettings, Failure> {
        self.held.clone().ok_or_else(|| Failure::new(3, "auth", "HTTP 403"))
    }
}

#[derive(Default)]
struct Disk {
    lock: Option<Lock>,
    journal: Option<Journal>,
}

impl Store for Disk {
    fn save_lock(&mut self, lock: &Lock) -> io::Result<()> {
        self.lock = Some(Lock::parse(&lock.render()).unwrap());
        Ok(())
    }
    fn save_journal(&mut self, _: &str, journal: &Journal) -> io::Result<()> {
        self.journal = Some(journal.clone());
        Ok(())
    }
}

/// One provision of `[experience]` holding `table`. Returns the settings it sent and how the run ended.
fn provision(disk: &mut Disk, roblox: &mut Roblox, table: &str) -> (Vec<ResolvedSettings>, Result<Vec<Action>, Stop>) {
    provision_of(disk, roblox, &format!("[experience]\n{table}"))
}

/// One provision of `PLACES` followed by `tables`.
fn provision_of(
    disk: &mut Disk,
    roblox: &mut Roblox,
    tables: &str,
) -> (Vec<ResolvedSettings>, Result<Vec<Action>, Stop>) {
    let manifest = Manifest::parse(&format!("{PLACES}\n{tables}")).unwrap();
    let experience = Experience::new(&manifest).unwrap();
    let mut read = |_: &Path| -> io::Result<Vec<u8>> { unreachable!("nothing has an icon") };
    let lock = disk.lock.clone().unwrap_or_else(Lock::empty);
    let env = &manifest.envs["staging"];
    let observed = observe(&experience, env, lock.env("staging"), None, roblox).unwrap();
    let inputs = Inputs {
        experience: &experience,
        env_name: "staging",
        env,
        lock: lock.env("staging"),
        journal: None,
        observed: Some(&observed),
    };
    let plan = plan(&inputs, &mut read).unwrap();
    let mut run = Run::new(&inputs, &plan, &lock, &mut read).unwrap();
    let before = roblox.settings.len();
    let ended = loop {
        match run.step(roblox, disk) {
            Ok(true) => {}
            Ok(false) => break Ok(run.applied().iter().map(|applied| applied.action).collect()),
            Err(stop) => break Err(stop),
        }
    };
    (roblox.settings[before..].to_vec(), ended)
}

fn recorded(disk: &Disk) -> Option<ResolvedSettings> {
    disk.lock.as_ref()?.env("staging")?.experience.clone()
}

#[test]
fn provision_writes_changed_settings_once_and_records_their_values() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox::default());
    let table = "private_server_price = 0\nvoice_chat = false\n";
    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!(sent, [ResolvedSettings { private_server_price: Some(0), voice_chat: Some(false) }]);
    assert_eq!(ended.unwrap(), [Action::Update]);
    assert_eq!(recorded(&disk), held(Some(0), Some(false)), "the lock records the settings it wrote");

    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]), "a second run has nothing to write");

    let (sent, _) = provision(&mut disk, &mut roblox, "voice_chat = true\n");
    assert_eq!(sent, [ResolvedSettings { private_server_price: None, voice_chat: Some(true) }], "unset stays unset");
    assert_eq!(recorded(&disk), held(Some(0), Some(true)), "the price no table sets keeps its value");
}

#[test]
fn an_empty_experience_table_has_nothing_to_send_or_record() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox::default());
    let (sent, ended) = provision(&mut disk, &mut roblox, "");
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]));
    assert_eq!(recorded(&disk), None);
}

#[test]
fn a_failed_settings_update_stops_the_run_and_leaves_the_lock_as_it_was() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox { fail: Some(5), ..Roblox::default() });
    let (sent, ended) = provision(&mut disk, &mut roblox, "voice_chat = true\n");
    assert_eq!(sent.len(), 1);
    assert!(matches!(ended, Err(Stop::Settings(failure)) if failure.code == 5));
    assert_eq!(recorded(&disk), None, "the next run sends the settings again");

    roblox.fail = None;
    let (sent, ended) = provision(&mut disk, &mut roblox, "voice_chat = true\n");
    assert_eq!((sent.len(), ended.unwrap()), (1, vec![Action::Update]));
}

#[test]
fn a_run_that_writes_settings_lists_the_write_and_one_that_sends_nothing_does_not() {
    use crucible_iac::provision::Write;
    for (table, writes) in [("voice_chat = true\n", vec![Write::Settings]), ("", vec![])] {
        let manifest = Manifest::parse(&format!("{PLACES}\n[experience]\n{table}")).unwrap();
        let experience = Experience::new(&manifest).unwrap();
        let mut read = |_: &Path| -> io::Result<Vec<u8>> { unreachable!("nothing has an icon") };
        let env = &manifest.envs["staging"];
        let observed = observe(&experience, env, None, None, &mut Roblox::default()).unwrap();
        let inputs = Inputs {
            experience: &experience,
            env_name: "staging",
            env,
            lock: None,
            journal: None,
            observed: Some(&observed),
        };
        let plan = plan(&inputs, &mut read).unwrap();
        let run = Run::new(&inputs, &plan, &Lock::empty(), &mut read).unwrap();
        assert_eq!(run.writes(), writes, "{table:?}");
    }
}

#[test]
fn settings_go_last_so_a_refused_settings_update_leaves_every_resource_step_applied() {
    let tables = "[experience]\nvoice_chat = true\n\n[products.coin-pack]\nname = \"Coin pack\"\nprice = 49\n\n\
                  [products.gem-pack]\nname = \"Gem pack\"\nprice = 99\n";
    let lock = "version = 1\n\n[env.staging]\nuniverse = 1\n\n[env.staging.products.gem-pack]\nid = 5\n\
                fingerprint = \"x\"\nstatus = \"managed\"\n";
    let mut disk = Disk { lock: Some(Lock::parse(lock).unwrap()), journal: None };
    let gem = Listed {
        id: Id::new(5).unwrap(),
        name: "Gem pack".into(),
        description: String::new(),
        price: Some(10),
        on: true,
    };
    let mut roblox = Roblox { fail: Some(4), products: vec![gem], ..Roblox::default() };
    let (_, ended) = provision_of(&mut disk, &mut roblox, tables);
    assert_eq!(roblox.calls, ["update", "create", "settings"], "the settings update is the last write");
    assert!(matches!(ended, Err(Stop::Settings(failure)) if failure.code == 4));
    let lock = disk.lock.as_ref().unwrap().env("staging").unwrap();
    let products = lock.entries(Kind::Product);
    assert_eq!(products["coin-pack"].id, Id::new(900).unwrap(), "the create is recorded");
    assert_ne!(products["gem-pack"].fingerprint, "x", "the update is recorded");
    assert!(disk.journal.as_ref().unwrap().intents.is_empty(), "no intent is left");
    assert_eq!(lock.experience, None, "the next run sends the settings again");
}

fn held(price: Option<u32>, voice: Option<bool>) -> Option<ResolvedSettings> {
    Some(ResolvedSettings { private_server_price: price, voice_chat: voice })
}

#[test]
fn provision_repairs_a_setting_roblox_holds_otherwise_and_writes_only_that_one() {
    let table = "private_server_price = 100\nvoice_chat = false\n";
    let (mut disk, mut roblox) = (Disk::default(), Roblox { held: held(Some(100), None), ..Roblox::default() });
    let (sent, _) = provision(&mut disk, &mut roblox, table);
    assert_eq!(sent, [ResolvedSettings { private_server_price: None, voice_chat: Some(false) }], "100 is held");
    assert_eq!(recorded(&disk), held(Some(100), Some(false)));

    roblox.held = held(Some(100), Some(true));
    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!(sent, [ResolvedSettings { private_server_price: None, voice_chat: Some(false) }], "the lock matched");
    assert_eq!(ended.unwrap(), [Action::Update]);
    assert_eq!(recorded(&disk), held(Some(100), Some(false)), "the lock still records what provision wrote");

    roblox.held = held(Some(100), Some(false));
    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]), "nothing differs");

    roblox.held = held(None, None);
    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!(
        (sent, ended.unwrap()),
        (vec![], vec![]),
        "a setting the reply does not state is compared with the lock"
    );
}

#[test]
fn a_failed_read_of_the_settings_is_kept_and_the_plan_falls_back_to_the_lock() {
    let table = "voice_chat = false\n";
    let (mut disk, mut roblox) = (Disk::default(), Roblox { held: held(None, Some(true)), ..Roblox::default() });
    let (sent, _) = provision(&mut disk, &mut roblox, table);
    assert_eq!(sent, [ResolvedSettings { private_server_price: None, voice_chat: Some(false) }]);
    roblox.held = None;
    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]), "the lock records the value, so nothing is written");

    let manifest = Manifest::parse(&format!("{PLACES}\n[experience]\n{table}")).unwrap();
    let experience = Experience::new(&manifest).unwrap();
    let observed = observe(&experience, &manifest.envs["staging"], None, None, &mut roblox).unwrap();
    assert_eq!(observed.settings, Some(Err("HTTP 403".into())), "observe keeps why, and goes on");
    let (mut fresh, mut roblox) = (Disk::default(), Roblox::default());
    let (sent, ended) = provision(&mut fresh, &mut roblox, table);
    assert_eq!(sent, [ResolvedSettings { private_server_price: None, voice_chat: Some(false) }]);
    assert_eq!(ended.unwrap(), [Action::Update], "without the lock or the read, it writes every setting");
}

#[test]
fn without_the_read_provision_sends_only_the_settings_whose_value_differs_from_the_lock() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox::default());
    let (sent, _) = provision(&mut disk, &mut roblox, "private_server_price = 100\nvoice_chat = true\n");
    assert_eq!(sent, [ResolvedSettings { private_server_price: Some(100), voice_chat: Some(true) }]);

    let (sent, ended) = provision(&mut disk, &mut roblox, "private_server_price = 100\nvoice_chat = false\n");
    assert_eq!(
        sent,
        [ResolvedSettings { private_server_price: None, voice_chat: Some(false) }],
        "a price write may cancel private server subscriptions, so an unchanged price is never sent"
    );
    assert_eq!(ended.unwrap(), [Action::Update]);
    assert_eq!(recorded(&disk), held(Some(100), Some(false)));

    let (sent, ended) = provision(&mut disk, &mut roblox, "voice_chat = false\n");
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]));
    let (sent, ended) = provision(&mut disk, &mut roblox, "private_server_price = 100\nvoice_chat = false\n");
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]), "the lock kept the price while no table set it");

    roblox.held = held(None, Some(true));
    let (sent, ended) = provision(&mut disk, &mut roblox, "private_server_price = 100\nvoice_chat = true\n");
    assert_eq!((sent, ended.unwrap()), (vec![], vec![Action::Update]), "Roblox holds the new value already");
    assert_eq!(recorded(&disk), held(Some(100), Some(true)));
}
