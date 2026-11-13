use crucible_iac::journal::Journal;
use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Fields, Kind, ResolvedSettings};
use crucible_iac::plan::{Id, Inputs, Listed, plan};
use crucible_iac::provision::{Patch, Remote, Run, Store, observe};
use crucible_oc::ops::Image;
use crucible_oc::outcome::Failure;
use crucible_project::Manifest;
use std::io;
use std::path::Path;

const CONFIG: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 1
place = 2

[products.coin-pack]
name = "Coin pack"
price = 49
icon = "coin.png"
"#;

struct Roblox {
    listed: Vec<Listed>,
    icons: Vec<Vec<u8>>,
}

impl Remote for Roblox {
    fn list(&mut self, _: Kind) -> Result<Vec<Listed>, Failure> {
        Ok(self.listed.clone())
    }
    fn get(&mut self, _: Kind, _: Id) -> Result<Option<Listed>, Failure> {
        Ok(None)
    }
    fn create(&mut self, _: Kind, _: &Fields, _: Option<&Image>) -> Result<Id, Failure> {
        unreachable!("the product exists")
    }
    fn update(&mut self, _: Kind, _: Id, patch: &Patch) -> Result<(), Failure> {
        assert_eq!(*patch, Patch { icon: patch.icon.clone(), ..Patch::default() }, "only the icon differs");
        let icon = patch.icon.as_ref().expect("an update here sends the icon");
        assert_eq!((icon.file_name.as_str(), icon.content_type.as_str()), ("coin.png", "image/png"));
        self.icons.push(icon.bytes.clone());
        Ok(())
    }
    fn update_settings(&mut self, _: &ResolvedSettings) -> Result<(), Failure> {
        unreachable!("the config declares no settings")
    }
    fn read_settings(&mut self) -> Result<ResolvedSettings, Failure> {
        unreachable!("the config declares no settings")
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

/// One provision with the icon file holding `icon`, returning the icons it sent.
fn provision(disk: &mut Disk, icon: &[u8]) -> Vec<Vec<u8>> {
    let manifest = Manifest::parse(CONFIG).unwrap();
    let experience = Experience::new(&manifest).unwrap();
    let mut read = |path: &Path| {
        assert_eq!(path, Path::new("coin.png"));
        Ok(icon.to_vec())
    };
    let product = Listed {
        id: Id::new(77).unwrap(),
        name: "Coin pack".into(),
        description: String::new(),
        price: Some(49),
        on: true,
    };
    let mut roblox = Roblox { listed: vec![product], icons: vec![] };
    let lock = disk.lock.clone().unwrap_or_else(Lock::empty);
    let env = &manifest.envs["staging"];
    let observed = observe(&experience, env, lock.env("staging"), None, &mut roblox).unwrap();
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
    while run.step(&mut roblox, disk).unwrap() {}
    roblox.icons
}

#[test]
fn an_icon_is_sent_when_the_lock_has_none_or_the_file_changed_and_never_again() {
    let mut disk = Disk::default();
    assert_eq!(provision(&mut disk, b"one"), [b"one"], "an adopted resource gets its declared icon");
    assert_eq!(provision(&mut disk, b"one"), Vec::<Vec<u8>>::new(), "the lock records the icon it sent");
    assert_eq!(provision(&mut disk, b"two"), [b"two"]);
    assert_eq!(provision(&mut disk, b"two"), Vec::<Vec<u8>>::new());
}
