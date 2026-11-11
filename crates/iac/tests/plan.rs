use crucible_iac::journal::Journal;
use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Kind, Lifecycle, Problem, content_hash};
use crucible_iac::plan::{Action, Change, Inputs, Plan, plan};
use crucible_project::Manifest;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CONFIG: &str = r#"
[places.start]
project = "default.project.json"

[experience]
voice_chat = false

[env.staging]
universe = 10772177432
place = 132306245523809

[env.staging.products.coin-pack]
price = 1

[env.production]
universe = 10769815992
place = 116566609396165

[products.coin-pack]
name = "Coin pack"
description = "100 coins."
price = 49
icon = "assets/coin-pack.png"

[passes.vip]
name = "VIP"
price = 299

[badges.first-delivery]
name = "First delivery"
description = "Deliver your first parcel."
"#;

struct Project {
    manifest: Manifest,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

impl Project {
    fn new(config: &str) -> Self {
        let files = BTreeMap::from([(PathBuf::from("assets/coin-pack.png"), b"png".to_vec())]);
        Self { manifest: Manifest::parse(config).unwrap(), files }
    }

    fn experience(&self) -> Result<Experience, Vec<Problem>> {
        Experience::new(&self.manifest)
    }

    fn plan(&self, env: &str, lock: Option<&str>, journal: Option<&str>) -> Result<Plan, Vec<Problem>> {
        let experience = self.experience()?;
        let lock = lock.map(|text| Lock::parse(text).unwrap());
        let journal = journal.map(|text| Journal::parse(env, text).unwrap());
        let inputs = Inputs {
            experience: &experience,
            env_name: env,
            env: &self.manifest.envs[env],
            lock: lock.as_ref().and_then(|lock| lock.env(env)),
            journal: journal.as_ref(),
        };
        plan(&inputs, &mut |path: &Path| {
            self.files.get(path).cloned().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        })
    }

    /// The lock a provision of `env` would leave, with IDs counting up from 1000.
    fn applied(&self, env: &str) -> String {
        let experience = self.experience().unwrap();
        let mut lock = String::from("version = 1\n");
        if let Some(settings) = experience.settings() {
            lock += &format!("[env.{env}.experience]\nfingerprint = {:?}\n", settings.fingerprint());
        }
        for (at, resource) in experience.resolve(&self.manifest.envs[env]).iter().enumerate() {
            let status = if resource.lifecycle == Lifecycle::Retired { "retired" } else { "managed" };
            lock += &format!(
                "[env.{env}.{}.{}]\nid = {}\nstatus = {status:?}\n",
                resource.kind.table(),
                resource.key,
                1000 + at
            );
            let fingerprint = resource.fields.as_ref().map_or_else(String::new, |fields| fields.fingerprint());
            lock += &format!("fingerprint = {fingerprint:?}\n");
            if let Some(icon) = &resource.icon {
                lock += &format!("icon = {:?}\n", content_hash(&self.files[icon]));
            }
        }
        lock
    }
}

fn actions(plan: &Plan) -> Vec<(&str, &str, Action)> {
    plan.steps.iter().map(|step| (step.kind, step.key.as_str(), step.action)).collect()
}

fn messages(problems: Vec<Problem>) -> Vec<String> {
    problems.iter().map(ToString::to_string).collect()
}

#[test]
fn a_first_provision_creates_everything_after_the_adopt_check() {
    let plan = Project::new(CONFIG).plan("staging", None, None).unwrap();
    assert_eq!(
        actions(&plan),
        [
            ("experience", "settings", Action::Update),
            ("product", "coin-pack", Action::Create),
            ("pass", "vip", Action::Create),
            ("badge", "first-delivery", Action::Create),
        ]
    );
    assert_eq!(plan.steps[1].handle.as_deref(), Some("Products.CoinPack"));
    assert_eq!(plan.summary, BTreeMap::from([(Action::Create, 3), (Action::Update, 1)]));
    assert_eq!((plan.environment.as_str(), plan.universe.as_str()), ("staging", "10772177432"));
}

#[test]
fn an_applied_environment_has_nothing_to_do() {
    let project = Project::new(CONFIG);
    for env in ["staging", "production"] {
        let plan = project.plan(env, Some(&project.applied(env)), None).unwrap();
        assert!(plan.steps.iter().all(|step| step.action == Action::NoOp), "{env}: {:?}", plan.steps);
        assert_eq!(plan.steps[1].id.as_deref(), Some("1000"));
    }
}

#[test]
fn an_overlay_is_a_change_only_in_its_environment() {
    let project = Project::new(CONFIG);
    let staging = project.applied("staging");
    let plan = project.plan("production", Some(&staging.replace("env.staging", "env.production")), None).unwrap();
    assert_eq!(plan.steps[1].action, Action::Update, "staging sells coin-pack for 1 Robux, production for 49");
    assert_eq!(plan.steps[1].changes, [Change::Details]);
    assert!(plan.steps[2..].iter().all(|step| step.action == Action::NoOp));
}

#[test]
fn a_changed_field_or_icon_is_an_update() {
    let mut project = Project::new(CONFIG);
    let lock = project.applied("staging");
    project.files.insert("assets/coin-pack.png".into(), b"new png".to_vec());
    let plan = project.plan("staging", Some(&lock), None).unwrap();
    assert_eq!((plan.steps[1].action, plan.steps[1].changes.as_slice()), (Action::Update, &[Change::Icon][..]));
    let renamed = Project::new(&CONFIG.replace("name = \"VIP\"", "name = \"Very important\""));
    let plan = renamed.plan("staging", Some(&lock), None).unwrap();
    assert_eq!((plan.steps[2].action, plan.steps[2].changes.as_slice()), (Action::Update, &[Change::Details][..]));
}

#[test]
fn a_missing_icon_fails_the_plan_before_a_create() {
    let mut project = Project::new(CONFIG);
    project.files.clear();
    let problems = messages(project.plan("staging", None, None).unwrap_err());
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("crucible.toml:19: products.coin-pack.icon: cannot read assets/coin-pack.png"),
        "{problems:?}"
    );
}

#[test]
fn removing_a_resource_needs_retired_or_untrack() {
    let project = Project::new(CONFIG);
    let lock = project.applied("staging");
    let removed = Project::new(&CONFIG.replace("[passes.vip]\nname = \"VIP\"\nprice = 299\n", ""));
    assert_eq!(
        messages(removed.plan("staging", Some(&lock), None).unwrap_err()),
        [
            "crucible.toml: passes.vip: crucible.lock holds its ID for staging, and crucible.toml no longer declares it. \
          Declare it with retired = true to take it off Roblox, or untrack = true to keep it there unmanaged, before \
          removing it"
        ]
    );
    let retired = Project::new(&CONFIG.replace("price = 299\n", "price = 299\nretired = true\n"));
    let plan = retired.plan("staging", Some(&lock), None).unwrap();
    assert_eq!(plan.steps[2].action, Action::Retire);
    let untracked = Project::new(&CONFIG.replace("price = 299\n", "untrack = true\n"));
    let plan = untracked.plan("staging", Some(&lock), None).unwrap();
    assert_eq!((plan.steps[2].action, plan.steps[2].id.as_deref()), (Action::Untrack, Some("1001")));
}

#[test]
fn a_retired_resource_stays_retired_and_may_leave_the_config() {
    let retired = Project::new(&CONFIG.replace("price = 299\n", "price = 299\nretired = true\n"));
    let lock = retired.applied("staging");
    assert_eq!(retired.plan("staging", Some(&lock), None).unwrap().steps[2].action, Action::NoOp);
    let gone = Project::new(&CONFIG.replace("[passes.vip]\nname = \"VIP\"\nprice = 299\n", ""));
    let plan = gone.plan("staging", Some(&lock), None).unwrap();
    let vip = plan.steps.iter().find(|step| step.key == "vip").unwrap();
    assert_eq!((vip.action, vip.id.as_deref()), (Action::NoOp, Some("1001")));
    let restored = Project::new(CONFIG).plan("staging", Some(&lock), None).unwrap();
    assert_eq!(
        (restored.steps[2].action, restored.steps[2].changes.as_slice()),
        (Action::Update, &[Change::Restore][..])
    );
}

#[test]
fn a_rename_keeps_the_id_only_with_moved_from() {
    let project = Project::new(CONFIG);
    let lock = project.applied("staging");
    let renamed = CONFIG.replace("[passes.vip]", "[passes.gold]");
    let problems = messages(Project::new(&renamed).plan("staging", Some(&lock), None).unwrap_err());
    assert!(
        problems[0].ends_with(". If it was renamed, set moved_from = \"vip\" on the new key, one of passes.gold"),
        "{problems:?}"
    );
    let moved = Project::new(&renamed.replace("price = 299\n", "price = 299\nmoved_from = \"vip\"\n"));
    let plan = moved.plan("staging", Some(&lock), None).unwrap();
    let gold = &plan.steps[2];
    assert_eq!((gold.key.as_str(), gold.action, gold.id.as_deref()), ("gold", Action::Update, Some("1001")));
    assert_eq!((gold.moved_from.as_deref(), gold.changes.as_slice()), (Some("vip"), &[Change::Key][..]));
    assert_eq!(gold.handle.as_deref(), Some("Passes.Gold"));
}

#[test]
fn an_intent_without_an_id_is_an_adopt_check() {
    let project = Project::new(CONFIG);
    let journal = r#"{"version":1,"intents":[{"kind":"badge","key":"first-delivery"},{"kind":"pass","key":"vip"}]}"#;
    let lock = project.applied("staging");
    let plan = project.plan("staging", None, Some(journal)).unwrap();
    assert_eq!(plan.steps[3].action, Action::AdoptCheck);
    assert_eq!(plan.steps[2].action, Action::AdoptCheck);
    let plan = project.plan("staging", Some(&lock), Some(journal)).unwrap();
    assert_eq!(plan.steps[2].action, Action::NoOp, "a lock ID resolves the intent");
}

#[test]
fn a_price_of_zero_is_not_off_sale() {
    for (config, problem) in [
        (CONFIG.replace("price = 49", "price = 0"), "crucible.toml:19: products.coin-pack.price"),
        (CONFIG.replace("price = 1\n", "price = 0\n"), "crucible.toml:12: env.staging.products.coin-pack.price"),
    ] {
        let problems = messages(Project::new(&config).experience().unwrap_err());
        assert_eq!(
            problems,
            [format!(
                "{problem}: a price of 0 does not take a product off sale. Set retired = true to take it off sale"
            )]
        );
    }
}

#[test]
fn keys_overlays_and_lifecycles_are_checked_across_tables() {
    for (config, problem) in [
        (
            CONFIG.replace("[passes.vip]", "[passes.Vip]"),
            "crucible.toml:25: passes.Vip: a key uses lowercase letters, digits and '-', and starts with a letter",
        ),
        (
            CONFIG.replace("[passes.vip]", "[passes.a-1b]") + "[passes.a1b]\nname = \"A\"\nprice = 1\n",
            "crucible.toml:32: passes.a1b: passes.a-1b and passes.a1b both become the handle Passes.A1b",
        ),
        (
            CONFIG.replace("[env.staging.products.coin-pack]", "[env.staging.products.coin-pak]"),
            "crucible.toml:12: env.staging.products.coin-pak: no products.coin-pak is declared",
        ),
        (
            CONFIG.replace("price = 299\n", "retired = true\nuntrack = true\n"),
            "crucible.toml:25: passes.vip: set retired or untrack, not both",
        ),
        (
            CONFIG.replace("price = 299\n", "price = 299\nmoved_from = \"vip\"\n"),
            "crucible.toml:25: passes.vip.moved_from: \"vip\" is not another key",
        ),
        (
            CONFIG.replace(
                "[passes.vip]\nname = \"VIP\"\nprice = 299\n",
                "[products.old]\nname = \"Old\"\nprice = 5\nmoved_from = \"coin-pack\"\n",
            ),
            "crucible.toml:25: products.old.moved_from: products.coin-pack is still declared, so nothing moved from it",
        ),
        (CONFIG.replace("price = 299\n", ""), "crucible.toml:25: passes.vip.price: a pass needs a price in Robux"),
        (
            CONFIG.replace("name = \"First delivery\"\n", ""),
            "crucible.toml:29: badges.first-delivery.name: a badge needs a name",
        ),
    ] {
        assert_eq!(messages(Project::new(&config).experience().unwrap_err()), [problem]);
    }
    let retired = Project::new(&CONFIG.replace("name = \"VIP\"\nprice = 299\n", "retired = true\n"));
    assert_eq!(retired.experience().unwrap().resolve(&retired.manifest.envs["staging"])[1].fields, None);
}

#[test]
fn a_badge_overlay_changes_only_its_environment() {
    let config = CONFIG.to_owned() + "[env.staging.badges.first-delivery]\nenabled = false\n";
    let project = Project::new(&config);
    let experience = project.experience().unwrap();
    let fingerprint =
        |env: &str| experience.resolve(&project.manifest.envs[env])[2].fields.clone().unwrap().fingerprint();
    assert_ne!(fingerprint("staging"), fingerprint("production"));
    assert_eq!(experience.resolve(&project.manifest.envs["production"])[2].kind, Kind::Badge);
}

#[test]
fn more_badge_creates_than_are_free_today_warn() {
    let mut config = CONFIG.to_owned();
    for at in 0..5 {
        config += &format!("[badges.extra-{at}]\nname = \"Extra {at}\"\n");
    }
    let plan = Project::new(&config).plan("staging", None, None).unwrap();
    assert_eq!(plan.warnings.len(), 1);
    assert!(
        plan.warnings[0].starts_with("6 badges may need creating. Roblox makes 5 a day free"),
        "{:?}",
        plan.warnings
    );
    assert!(Project::new(CONFIG).plan("staging", None, None).unwrap().warnings.is_empty());
}
