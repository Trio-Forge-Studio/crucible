use crucible_iac::journal::{Intent, Journal};
use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Fields, Kind};
use crucible_iac::plan::{Id, Listed};
use crucible_iac::provision::{Patch, Remote, observe};
use crucible_oc::ops::Image;
use crucible_oc::outcome::Failure;
use crucible_project::Manifest;

const CONFIG: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 1
place = 2

[products.coin-pack]
name = "Coin pack"
price = 49
"#;

struct Recorder {
    passes: Vec<Listed>,
    calls: Vec<String>,
}

impl Remote for Recorder {
    fn list(&mut self, kind: Kind) -> Result<Vec<Listed>, Failure> {
        self.calls.push(format!("list {}", kind.noun()));
        Ok(if kind == Kind::Pass { self.passes.clone() } else { vec![] })
    }
    fn get(&mut self, kind: Kind, id: Id) -> Result<Option<Listed>, Failure> {
        self.calls.push(format!("get {} {id}", kind.noun()));
        Ok(None)
    }
    fn create(&mut self, _: Kind, _: &Fields, _: Option<&Image>) -> Result<Id, Failure> {
        unreachable!("observe never writes")
    }
    fn update(&mut self, _: Kind, _: Id, _: &Patch) -> Result<(), Failure> {
        unreachable!("observe never writes")
    }
}

fn calls(lock: Option<&str>, journal: Option<&Journal>, passes: &[u64]) -> Vec<String> {
    let manifest = Manifest::parse(CONFIG).unwrap();
    let experience = Experience::new(&manifest).unwrap();
    let lock = lock.map(|text| Lock::parse(text).unwrap());
    let passes = passes
        .iter()
        .map(|id| Listed {
            id: Id::new(*id).unwrap(),
            name: "VIP".into(),
            description: "".into(),
            price: None,
            on: false,
        })
        .collect();
    let mut remote = Recorder { passes, calls: vec![] };
    let observed = observe(
        &experience,
        &manifest.envs["staging"],
        lock.as_ref().and_then(|lock| lock.env("staging")),
        journal,
        &mut remote,
    )
    .unwrap();
    assert_eq!(observed.missing.len(), remote.calls.iter().filter(|call| call.starts_with("get")).count());
    remote.calls
}

#[test]
fn each_kind_the_config_the_lock_or_the_journal_names_is_listed_and_no_other() {
    assert_eq!(calls(None, None, &[]), ["list product"]);

    let mut journal = Journal::new(Id::new(1).unwrap());
    journal.intents.push(Intent { kind: Kind::Badge, key: "first".into(), name: "First".into(), before: None });
    assert_eq!(calls(None, Some(&journal), &[]), ["list product", "list badge"]);

    let lock = "version = 1\n[env.staging]\nuniverse = 1\n[env.staging.passes.vip]\nid = 9\nfingerprint = \"\"\nstatus = \"retired\"\n";
    assert_eq!(calls(Some(lock), None, &[]), ["list product", "list pass", "get pass 9"]);
    assert_eq!(calls(Some(lock), None, &[9]), ["list product", "list pass"], "a listed ID is not read again");
}
