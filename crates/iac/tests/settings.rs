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
}

impl Remote for Roblox {
    fn list(&mut self, _: Kind) -> Result<Vec<Listed>, Failure> {
        Ok(vec![])
    }
    fn get(&mut self, _: Kind, _: Id) -> Result<Option<Listed>, Failure> {
        Ok(None)
    }
    fn create(&mut self, _: Kind, _: &Fields, _: Option<&Image>) -> Result<Id, Failure> {
        unreachable!("the config declares no resource")
    }
    fn update(&mut self, _: Kind, _: Id, _: &Patch) -> Result<(), Failure> {
        unreachable!("the config declares no resource")
    }
    fn update_settings(&mut self, settings: &ResolvedSettings) -> Result<(), Failure> {
        self.settings.push(settings.clone());
        match self.fail {
            Some(code) => Err(Failure::new(code, "outcome_unknown", "HTTP 503")),
            None => Ok(()),
        }
    }
}

#[derive(Default)]
struct Disk {
    lock: Option<Lock>,
}

impl Store for Disk {
    fn save_lock(&mut self, lock: &Lock) -> io::Result<()> {
        self.lock = Some(Lock::parse(&lock.render()).unwrap());
        Ok(())
    }
    fn save_journal(&mut self, _: &str, journal: &Journal) -> io::Result<()> {
        assert!(journal.intents.is_empty(), "nothing is created");
        Ok(())
    }
}

/// One provision of `[experience]` holding `table`. Returns the settings it sent and how the run ended.
fn provision(disk: &mut Disk, roblox: &mut Roblox, table: &str) -> (Vec<ResolvedSettings>, Result<Vec<Action>, Stop>) {
    let manifest = Manifest::parse(&format!("{PLACES}\n[experience]\n{table}")).unwrap();
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

fn fingerprint(disk: &Disk) -> Option<String> {
    disk.lock.as_ref()?.env("staging")?.experience.as_ref().map(|applied| applied.fingerprint.clone())
}

#[test]
fn provision_writes_changed_settings_once_and_records_their_fingerprint() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox::default());
    let table = "private_server_price = 0\nvoice_chat = false\n";
    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!(sent, [ResolvedSettings { private_server_price: Some(0), voice_chat: Some(false) }]);
    assert_eq!(ended.unwrap(), [Action::Update]);
    let recorded = fingerprint(&disk).expect("the lock records the settings it applied");

    let (sent, ended) = provision(&mut disk, &mut roblox, table);
    assert_eq!((sent, ended.unwrap()), (vec![], vec![]), "a second run has nothing to write");

    let (sent, _) = provision(&mut disk, &mut roblox, "voice_chat = true\n");
    assert_eq!(sent, [ResolvedSettings { private_server_price: None, voice_chat: Some(true) }], "unset stays unset");
    assert_ne!(fingerprint(&disk).unwrap(), recorded);
}

#[test]
fn an_empty_experience_table_sends_nothing_and_is_recorded() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox::default());
    let (sent, ended) = provision(&mut disk, &mut roblox, "");
    assert_eq!((sent, ended.unwrap()), (vec![], vec![Action::Update]));
    assert!(fingerprint(&disk).is_some());
}

#[test]
fn a_failed_settings_update_stops_the_run_and_leaves_the_lock_as_it_was() {
    let (mut disk, mut roblox) = (Disk::default(), Roblox { fail: Some(5), ..Roblox::default() });
    let (sent, ended) = provision(&mut disk, &mut roblox, "voice_chat = true\n");
    assert_eq!(sent.len(), 1);
    assert!(matches!(ended, Err(Stop::Settings(failure)) if failure.code == 5));
    assert_eq!(fingerprint(&disk), None, "the next run sends the settings again");

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
