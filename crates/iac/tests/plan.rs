use crucible_iac::journal::Journal;
use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Fields, Kind, Lifecycle, Problem, content_hash};
use crucible_iac::plan::{Action, Change, Field, Id, Inputs, Listed, Observed, Plan, StepIntent, differs, plan};
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
        self.plan_seen(env, lock, journal, None)
    }

    fn plan_seen(
        &self,
        env: &str,
        lock: Option<&str>,
        journal: Option<&str>,
        observed: Option<&Observed>,
    ) -> Result<Plan, Vec<Problem>> {
        let experience = self.experience()?;
        let lock = lock.map(|text| Lock::parse(text).unwrap());
        let journal = journal.map(|text| Journal::parse(env, text).unwrap());
        let inputs = Inputs {
            experience: &experience,
            env_name: env,
            env: &self.manifest.envs[env],
            lock: lock.as_ref().and_then(|lock| lock.env(env)),
            journal: journal.as_ref(),
            observed,
        };
        plan(&inputs, &mut |path: &Path| {
            self.files.get(path).cloned().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        })
    }

    /// The lock a provision of `env` would leave, with IDs counting up from 1000.
    fn applied(&self, env: &str) -> String {
        let experience = self.experience().unwrap();
        let mut lock = format!("version = 1\n[env.{env}]\nuniverse = {}\n", self.manifest.envs[env].universe);
        if let Some(settings) = experience.settings() {
            lock += &format!("[env.{env}.experience]\n");
            lock += &settings
                .private_server_price
                .map_or_else(String::new, |price| format!("private_server_price = {price}\n"));
            lock += &settings.voice_chat.map_or_else(String::new, |on| format!("voice_chat = {on}\n"));
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
fn a_lock_for_another_universe_is_refused() {
    let project = Project::new(CONFIG);
    let lock = project.applied("staging");
    let moved = Project::new(&CONFIG.replace("universe = 10772177432", "universe = 99999999999"));
    assert_eq!(
        messages(moved.plan("staging", Some(&lock), None).unwrap_err()),
        ["crucible.lock:3: env.staging.universe: crucible.lock records staging's resources in universe 10772177432, \
             and crucible.toml names universe 99999999999 for staging. If the universe ID changed by mistake, set \
             env.staging.universe back to 10772177432 in crucible.toml. If staging now deploys to universe \
             99999999999, start staging afresh: delete [env.staging] and every [env.staging.*] table from \
             crucible.lock, and the resources in universe 10772177432 stay on Roblox, unmanaged"]
    );
    let afresh = Project::new(&CONFIG.replace("universe = 10772177432", "universe = 99999999999"));
    let plan = afresh.plan("staging", Some(&lock.replace("[env.staging", "[env.qa")), None).unwrap();
    assert_eq!(plan.summary, BTreeMap::from([(Action::Create, 3), (Action::Update, 1)]));
}

#[test]
fn an_overlay_is_a_change_only_in_its_environment() {
    let project = Project::new(CONFIG);
    let staging = project.applied("staging");
    let production = project.applied("production");
    let universe = |lock: &str| lock.lines().nth(2).unwrap().to_owned();
    let staging = staging.replace("env.staging", "env.production").replace(&universe(&staging), &universe(&production));
    let plan = project.plan("production", Some(&staging), None).unwrap();
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
    let silver = "[passes.silver]\nname = \"Silver\"\nprice = 5\n";
    let silver_lock = Project::new(&(CONFIG.to_owned() + silver)).applied("staging");
    let problems =
        messages(Project::new(&(renamed.clone() + silver)).plan("staging", Some(&silver_lock), None).unwrap_err());
    assert!(problems[0].ends_with("one of passes.gold"), "a key the lock holds is not a rename: {problems:?}");
    let moved = Project::new(&renamed.replace("price = 299\n", "price = 299\nmoved_from = \"vip\"\n"));
    let plan = moved.plan("staging", Some(&lock), None).unwrap();
    let gold = &plan.steps[2];
    assert_eq!((gold.key.as_str(), gold.action, gold.id.as_deref()), ("gold", Action::Update, Some("1001")));
    assert_eq!((gold.moved_from.as_deref(), gold.changes.as_slice()), (Some("vip"), &[Change::Key][..]));
    assert_eq!(gold.handle.as_deref(), Some("Passes.Gold"));
}

/// A staging journal with one intent per `(kind, key, name)`.
fn journal(intents: &[(&str, &str, &str)]) -> String {
    let intents: Vec<_> =
        intents.iter().map(|(kind, key, name)| serde_json::json!({"kind": kind, "key": key, "name": name})).collect();
    serde_json::json!({"version": 1, "universe": 10772177432_u64, "intents": intents}).to_string()
}

fn intent(plan: &Plan, key: &str) -> (Action, Option<StepIntent>) {
    let step = plan.steps.iter().find(|step| step.key == key).unwrap();
    (step.action, step.intent.clone())
}

fn sent(name: &str, resolved: bool) -> Option<StepIntent> {
    Some(StepIntent { name: name.into(), resolved })
}

#[test]
fn an_intent_without_an_id_is_an_adopt_check_for_the_name_it_sent() {
    let project = Project::new(CONFIG);
    let journal = journal(&[("badge", "first-delivery", "First delivery"), ("pass", "vip", "Gold")]);
    let plan = project.plan("staging", None, Some(&journal)).unwrap();
    assert_eq!(intent(&plan, "first-delivery"), (Action::AdoptCheck, sent("First delivery", false)));
    assert_eq!(intent(&plan, "vip"), (Action::AdoptCheck, sent("Gold", false)), "the name it sent, not the declared");
    let plan = project.plan("staging", Some(&project.applied("staging")), Some(&journal)).unwrap();
    assert_eq!(intent(&plan, "vip"), (Action::NoOp, sent("Gold", true)), "a lock ID resolves the intent");
}

#[test]
fn a_moved_key_or_a_retired_lock_entry_carries_the_intent_of_its_key() {
    let project = Project::new(CONFIG);
    let lock = project.applied("staging");
    let journal = journal(&[("pass", "vip", "VIP")]);
    let moved = Project::new(&CONFIG.replace("[passes.vip]", "[passes.gold]\nmoved_from = \"vip\""));
    assert_eq!(
        intent(&moved.plan("staging", None, Some(&journal)).unwrap(), "gold"),
        (Action::AdoptCheck, sent("VIP", false))
    );
    assert_eq!(
        intent(&moved.plan("staging", Some(&lock), Some(&journal)).unwrap(), "gold"),
        (Action::Update, sent("VIP", true))
    );
    let retired = Project::new(&CONFIG.replace("price = 299\n", "price = 299\nretired = true\n"));
    let retired_lock = retired.applied("staging");
    let gone = Project::new(&CONFIG.replace("[passes.vip]\nname = \"VIP\"\nprice = 299\n", ""));
    assert_eq!(
        intent(&gone.plan("staging", Some(&retired_lock), Some(&journal)).unwrap(), "vip"),
        (Action::NoOp, sent("VIP", true))
    );
}

#[test]
fn an_intent_no_step_can_carry_stops_the_plan() {
    let path = crucible_iac::journal::path("staging").display().to_string();
    let undeclared = Project::new(CONFIG).plan("staging", None, Some(&journal(&[("product", "gem-pack", "Gem pack")])));
    let problems = undeclared.unwrap_err();
    assert_eq!(
        (problems[0].file.as_str(), problems[0].line, problems[0].key.as_str()),
        (path.as_str(), None, "products.gem-pack")
    );
    assert_eq!(
        messages(problems),
        [format!(
            "{path}: products.gem-pack: the journal holds an intent to create product gem-pack as \"Gem pack\", and \
             crucible.lock holds no ID for it, so the create may have landed in universe 10772177432. crucible.toml \
             declares no products.gem-pack. If it was renamed, set moved_from = \"gem-pack\" on the new key, one of \
             products.coin-pack. Declare it again until provision resolves the intent, or, if universe 10772177432 \
             holds no product its create made, run `crucible adopt staging product gem-pack --none`"
        )]
    );
    let lock = Project::new(CONFIG).applied("staging");
    let retired_lock =
        Project::new(&CONFIG.replace("price = 299\n", "price = 299\nretired = true\n")).applied("staging");
    let vip = journal(&[("pass", "vip", "VIP")]);
    for (config, lock, journal, expected) in [
        (
            CONFIG.replace("price = 299\n", "price = 299\nretired = true\n"),
            None,
            &vip,
            "crucible.toml retires passes.vip, so provision would never look for it. Remove retired = true until",
        ),
        (
            CONFIG.replace("price = 299\n", "untrack = true\n"),
            None,
            &vip,
            "crucible.toml untracks passes.vip, so provision would never look for it. Remove untrack = true until",
        ),
        (
            CONFIG.replace("[passes.vip]", "[passes.gold]"),
            None,
            &vip,
            ". If it was renamed, set moved_from = \"vip\" on the new key, one of passes.gold",
        ),
        (
            CONFIG.to_owned(),
            None,
            &journal(&[("product", "first-delivery", "First delivery")]),
            ", and badges.first-delivery is declared, but the intent is for a product",
        ),
        (
            CONFIG.replace("[passes.vip]\nname = \"VIP\"\nprice = 299\n", ""),
            Some(&lock),
            &vip,
            "and crucible.lock holds its ID 1001, so the create landed. crucible.toml no longer declares passes.vip",
        ),
        (
            CONFIG.replace("[passes.vip]\nname = \"VIP\"\nprice = 299\n", ""),
            Some(&retired_lock),
            &journal(&[("pass", "gold", "Gold")]),
            "the journal holds an intent to create pass gold as \"Gold\", and crucible.lock holds no ID for it",
        ),
    ] {
        let problems =
            messages(Project::new(&config).plan("staging", lock.map(String::as_str), Some(journal)).unwrap_err());
        assert!(
            problems.iter().any(|problem| problem.starts_with(&format!("{path}: ")) && problem.contains(expected)),
            "{expected}: {problems:#?}"
        );
    }
}

#[test]
fn a_name_another_key_s_unresolved_intent_sent_stops_the_plan() {
    let config = CONFIG.to_owned() + "[passes.gold]\nname = \"Gold\"\nprice = 1\n";
    let line = config.lines().position(|line| line == "[passes.gold]").unwrap() + 1;
    let project = Project::new(&config);
    let journal = journal(&[("pass", "vip", "Gold")]);
    let problems = messages(project.plan("staging", None, Some(&journal)).unwrap_err());
    assert_eq!(
        problems,
        [format!(
            "crucible.toml:{line}: passes.gold.name: \"Gold\" is the name the journal's intent for passes.vip sent, and \
             crucible.lock holds no ID for it, so its create may have landed under that name and the adopt check could \
             bind passes.gold to it. Give passes.gold another name until provision resolves that intent: record \
             passes.vip's pass with `crucible adopt staging pass vip <id>`, or run `crucible adopt staging pass vip \
             --none` if its create never landed"
        )]
    );
    let lock = project.applied("staging");
    assert!(project.plan("staging", Some(&lock), Some(&journal)).is_ok(), "the lock resolves vip's intent");

    let moved = config + "[passes.silver]\nmoved_from = \"old\"\nname = \"Silver\"\nprice = 5\n";
    let old = Project::new(&CONFIG.replace("[passes.vip]", "[passes.old]")).applied("staging");
    let problems = messages(Project::new(&moved).plan("staging", Some(&old), Some(&journal)).unwrap_err());
    assert!(
        problems.iter().any(|problem| problem.contains("passes.gold.name: \"Gold\" is the name")),
        "another key's moved_from leaves vip's intent unresolved: {problems:#?}"
    );
}

#[test]
fn a_message_that_names_crucible_adopt_names_the_environment() {
    let gold = CONFIG.to_owned() + "[passes.gold]\nname = \"Gold\"\nprice = 1\n";
    let moved = CONFIG.replace("[passes.vip]", "[passes.gold]\nmoved_from = \"vip\"");
    let mut problems = Vec::new();
    for (config, journal) in [
        (CONFIG.to_owned(), journal(&[("product", "gem-pack", "Gem pack")])),
        (gold, journal(&[("pass", "vip", "Gold")])),
        (moved, journal(&[("pass", "vip", "VIP"), ("pass", "gold", "VIP")])),
    ] {
        problems.extend(messages(Project::new(&config).plan("staging", None, Some(&journal)).unwrap_err()));
    }
    assert_eq!(problems.len(), 3, "{problems:#?}");
    for problem in problems {
        assert!(
            problem.contains("`crucible adopt staging pass ") || problem.contains("`crucible adopt staging product "),
            "{problem}"
        );
    }
}

#[test]
fn a_journal_for_another_universe_is_refused() {
    let project = Project::new(&CONFIG.replace("universe = 10772177432", "universe = 99999999999"));
    let problems = messages(project.plan("staging", None, Some(&journal(&[("pass", "vip", "VIP")]))).unwrap_err());
    let path = crucible_iac::journal::path("staging").display().to_string();
    assert_eq!(
        problems,
        [format!(
            "{path}: universe: the journal holds intents sent to universe 10772177432, and crucible.toml names \
             universe 99999999999 for staging. If the universe ID changed by mistake, set env.staging.universe back to \
             10772177432 in crucible.toml. If staging now deploys to universe 99999999999, look in universe \
             10772177432 for the resources its intents name, whose creates may have landed there, then delete {path}"
        )]
    );
}

#[test]
fn a_lock_table_or_journal_for_an_undeclared_environment_is_refused() {
    use crucible_iac::plan::undeclared;
    let project = Project::new(CONFIG);
    let lock = Lock::parse(&(project.applied("staging") + &project.applied("production").replace("version = 1\n", "")))
        .unwrap();
    let vip = journal(&[("pass", "vip", "VIP")]).replace("10772177432", "10769815992");
    let journals =
        |text: &str| BTreeMap::from([("production".to_owned(), Journal::parse("production", text).unwrap())]);
    assert_eq!(undeclared(&project.manifest.envs, Some(&lock), &journals(&vip)), [], "both are declared");

    let gone = Project::new(&CONFIG.replace("[env.production]\nuniverse = 10769815992\nplace = 116566609396165\n", ""));
    let envs = &gone.manifest.envs;
    let line = project.applied("staging").lines().count() + 2;
    assert_eq!(
        messages(undeclared(envs, Some(&lock), &BTreeMap::new())),
        [format!(
            "crucible.lock:{line}: env.production: crucible.toml declares no environment production, and crucible.lock \
             holds 3 resources of it in universe 10769815992. No plan reads them, so a renamed environment would \
             create each resource again. Declare [env.production] with universe = 10769815992 in crucible.toml again. \
             If it was renamed to an environment with universe 10769815992, rename [env.production] and every \
             [env.production.*] table in crucible.lock to the new name. If production is gone for good, delete \
             [env.production] and every [env.production.*] table from crucible.lock, and its resources stay on \
             Roblox, unmanaged"
        )]
    );

    let path = crucible_iac::journal::path("production").display().to_string();
    let staging_only = Lock::parse(&project.applied("staging")).unwrap();
    let problems = undeclared(envs, Some(&staging_only), &journals(&vip));
    assert_eq!((problems[0].file.as_str(), problems[0].line), (path.as_str(), None));
    assert_eq!(
        messages(problems),
        [format!(
            "{path}: env.production: {path} holds an intent to create pass vip as \"VIP\" in universe 10769815992, and \
             crucible.lock holds no ID for it, so the create may have landed. crucible.toml declares no environment \
             production, and {path} holds 1 intent sent to universe 10769815992. No plan reads them, so a renamed \
             environment would create each resource again. Declare [env.production] with universe = 10769815992 in \
             crucible.toml again. If it was renamed to an environment with universe 10769815992, rename {path} to the \
             new name. If production is gone for good, look in universe 10769815992 for the resources its intents \
             name, then delete {path}, and its resources stay on Roblox, unmanaged"
        )]
    );

    let no_vip = Project::new(&CONFIG.replace("[passes.vip]\nname = \"VIP\"\nprice = 299\n", ""));
    let without_vip = no_vip.applied("staging") + &no_vip.applied("production").replace("version = 1\n", "");
    let problems = undeclared(envs, Some(&Lock::parse(&without_vip).unwrap()), &journals(&vip));
    assert_eq!(
        (problems[0].file.as_str(), problems[0].line),
        (path.as_str(), None),
        "an unresolved intent points at the journal even when the lock has the environment too"
    );

    let resolved = messages(undeclared(envs, Some(&lock), &journals(&vip)));
    assert!(
        resolved[0].starts_with(&format!("crucible.lock:{line}: env.production: crucible.toml declares")),
        "{resolved:?}"
    );
    assert!(resolved[0].contains("and delete"), "{resolved:?}");

    let renamed = Project::new(&CONFIG.replace("[env.production]", "[env.live]"));
    let problems = messages(undeclared(&renamed.manifest.envs, Some(&lock), &BTreeMap::new()));
    assert!(problems[0].contains("with universe 10769815992, such as env.live, rename"), "{problems:?}");
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
fn two_declared_keys_of_a_kind_may_not_share_a_display_name_whatever_their_status() {
    let shared = CONFIG.to_owned() + "[passes.gold]\nname = \"VIP\"\nprice = 1\n";
    let line = shared.lines().position(|line| line == "[passes.vip]").unwrap() + 1;
    assert_eq!(
        messages(Project::new(&shared).experience().unwrap_err()),
        [format!(
            "crucible.toml:{line}: passes.vip.name: passes.gold and passes.vip both have the display name \"VIP\". A \
             declared key's name is taken whatever its status, because provision adopts an existing resource by its \
             name and could not tell them apart. Give one of them another name"
        )]
    );
    for (status, fix) in [
        ("retired = true\n", "Give one of them another name, or remove the name of the retired one"),
        (
            "untrack = true\n",
            "Give passes.vip another name: passes.gold is untracked and must keep the name its resource has on Roblox",
        ),
    ] {
        let config = CONFIG.to_owned() + "[passes.gold]\nname = \"VIP\"\n" + status;
        let problems = messages(Project::new(&config).experience().unwrap_err());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].ends_with(fix), "{problems:?}");
    }
    let untracked = CONFIG.to_owned() + "[passes.gold]\nuntrack = true\n";
    let line = untracked.lines().position(|line| line == "[passes.gold]").unwrap() + 1;
    assert_eq!(
        messages(Project::new(&untracked).experience().unwrap_err()),
        [format!(
            "crucible.toml:{line}: passes.gold.name: an untracked pass keeps its name while it is declared, so no \
             other key adopts its resource. Give it the name it has on Roblox"
        )]
    );
    for allowed in [
        CONFIG.to_owned() + "[products.vip]\nname = \"VIP\"\nprice = 1\n",
        CONFIG.to_owned() + "[passes.gold]\nretired = true\n",
        CONFIG.to_owned() + "[passes.gold]\nname = \"\"\nretired = true\n",
        CONFIG.to_owned() + "[passes.gold]\nname = \"Gold\"\nuntrack = true\n",
    ] {
        assert!(Project::new(&allowed).experience().is_ok(), "{allowed}");
    }
}

#[test]
fn two_environments_may_not_share_a_universe() {
    let shared = Project::new(&CONFIG.replace("universe = 10769815992", "universe = 10772177432"));
    let problems = shared.experience().unwrap_err();
    assert_eq!((problems[0].file.as_str(), problems[0].line), ("crucible.toml", None));
    assert_eq!(
        messages(problems),
        ["crucible.toml: env.staging.universe: env.production and env.staging both name universe 10772177432. \
          Provision adopts a resource by its name inside a universe, so each environment would bind the other's \
          resources, and its overlays would overwrite the other's values. Give each environment its own universe"]
    );
}

#[test]
fn a_display_name_has_no_whitespace_at_either_end() {
    for name in [" VIP", "VIP ", "VIP\n", "   ", "\u{feff}VIP", "VIP\u{3000}"] {
        for status in ["price = 299\n", "retired = true\n", "untrack = true\n"] {
            let toml = format!("name = \"{}\"\n{status}", name.replace('\n', "\\n"));
            let config = CONFIG.replace("name = \"VIP\"\nprice = 299\n", &toml);
            let problems = messages(Project::new(&config).experience().unwrap_err());
            assert_eq!(
                problems,
                [format!(
                    "crucible.toml:25: passes.vip.name: {name:?} starts or ends with whitespace. Crucible compares \
                     display names byte for byte and Roblox may trim them, so the name could stop matching its own \
                     resource. Remove the whitespace"
                )],
                "{status}"
            );
        }
    }
    for name in ["V I P", "VIP", "Café"] {
        let config = CONFIG.replace("name = \"VIP\"", &format!("name = \"{name}\""));
        assert!(Project::new(&config).experience().is_ok(), "{name:?}");
    }
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
    let free = config.replace("[badges.extra-4]\nname = \"Extra 4\"\n", "");
    assert!(Project::new(&free).plan("staging", None, None).unwrap().warnings.is_empty(), "five badges are free");
}

fn listed(id: u64, name: &str, price: Option<u64>, on: bool) -> Listed {
    Listed { id: Id::new(id).unwrap(), name: name.into(), description: String::new(), price, on }
}

/// What Roblox holds for the staging environment of `CONFIG` once each resource matches it.
fn matching() -> Observed {
    Observed {
        listed: BTreeMap::from([
            (Kind::Product, vec![listed(1000, "Coin pack", Some(1), true)]),
            (Kind::Pass, vec![listed(1001, "VIP", Some(299), true)]),
            (Kind::Badge, vec![listed(1002, "First delivery", None, true)]),
        ]),
        missing: BTreeMap::new(),
        settings: None,
    }
}

fn describe(observed: &mut Observed) {
    for listed in observed.listed.get_mut(&Kind::Product).unwrap() {
        listed.description = "100 coins.".into();
    }
    observed.listed.get_mut(&Kind::Badge).unwrap()[0].description = "Deliver your first parcel.".into();
}

#[test]
fn what_roblox_lists_decides_each_adopt_check() {
    let project = Project::new(CONFIG);
    let mut observed = matching();
    observed.listed.get_mut(&Kind::Pass).unwrap().clear();
    let plan = project.plan_seen("staging", None, None, Some(&observed)).unwrap();
    let found: Vec<_> =
        plan.steps.iter().skip(1).map(|step| (step.key.as_str(), step.action, step.id.clone())).collect();
    assert_eq!(
        found,
        [
            ("coin-pack", Action::Adopt, Some("1000".into())),
            ("vip", Action::Create, None),
            ("first-delivery", Action::Adopt, Some("1002".into())),
        ]
    );

    observed.listed.get_mut(&Kind::Product).unwrap().push(listed(1003, "Coin pack", Some(5), false));
    let problems = messages(project.plan_seen("staging", None, None, Some(&observed)).unwrap_err());
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(
        problems[0].contains(
            "universe 10772177432 holds 2 products that products.coin-pack could be: 1000, 1003. Provision adopts \
             only one, so it stops. Record the one that is products.coin-pack's with `crucible adopt staging product \
             coin-pack <id>`"
        ),
        "{}",
        problems[0]
    );
}

#[test]
fn an_adopt_names_the_update_it_sends() {
    let project = Project::new(CONFIG);
    let changes = |plan: &Plan, key: &str| {
        let step = plan.steps.iter().find(|step| step.key == key).unwrap();
        (step.action, step.changes.clone(), step.fields.clone())
    };
    let mut observed = matching();
    describe(&mut observed);
    let plan = project.plan_seen("staging", None, None, Some(&observed)).unwrap();
    assert_eq!(changes(&plan, "coin-pack"), (Action::Adopt, vec![Change::Icon], vec![]), "a declared icon is sent");
    assert_eq!(changes(&plan, "vip"), (Action::Adopt, vec![], vec![]));
    assert_eq!(changes(&plan, "first-delivery"), (Action::Adopt, vec![], vec![]));

    let products = observed.listed.get_mut(&Kind::Product).unwrap();
    products[0].description = String::new();
    products[0].price = Some(49);
    products[0].on = false;
    observed.listed.get_mut(&Kind::Badge).unwrap()[0].on = false;
    let plan = project.plan_seen("staging", None, None, Some(&observed)).unwrap();
    assert_eq!(
        changes(&plan, "coin-pack"),
        (Action::Adopt, vec![Change::Details, Change::Icon], vec![Field::Description, Field::Price, Field::Sale])
    );
    assert_eq!(changes(&plan, "first-delivery"), (Action::Adopt, vec![Change::Details], vec![Field::Enabled]));
    assert_eq!(plan.summary[&Action::Adopt], 3, "an adopt that updates is still one adopt");

    let lock = project.applied("staging");
    let plan = project.plan_seen("staging", Some(&lock), None, Some(&observed)).unwrap();
    assert_eq!(changes(&plan, "first-delivery"), (Action::Update, vec![Change::Details], vec![Field::Enabled]));
    let offline = Project::new(&CONFIG.replace("price = 299", "price = 300")).plan("staging", Some(&lock), None);
    assert_eq!(
        changes(&offline.unwrap(), "vip"),
        (Action::Update, vec![Change::Details], vec![]),
        "no list, no fields"
    );
}

#[test]
fn after_an_unknown_outcome_a_run_adopts_only_the_declared_name_and_stops_on_a_new_id_with_another() {
    let project = Project::new(CONFIG);
    let intent = |before: &str| {
        format!(
            r#"{{"version":1,"universe":10772177432,"intents":[{{"kind":"pass","key":"vip","name":"VIP","before":[{before}]}}]}}"#
        )
    };
    let plan_with = |passes: Vec<Listed>, journal: &str| {
        let mut observed = matching();
        observed.listed.insert(Kind::Pass, passes);
        project.plan_seen("staging", None, Some(journal), Some(&observed))
    };
    let renamed = || listed(1001, "Renamed on Roblox", Some(299), true);
    let old = || listed(7, "Old", None, false);
    let problems = messages(plan_with(vec![renamed(), old()], &intent("7")).unwrap_err());
    assert_eq!(problems.len(), 1, "{problems:#?}");
    for part in [
        "Since that intent, the universe gained 1001 (\"Renamed on Roblox\"), which does not carry the declared name \"VIP\"",
        "someone made it by hand",
        "If it is passes.vip's, record it with `crucible adopt staging pass vip <id>`",
        "run `crucible adopt staging pass vip --none`",
    ] {
        assert!(problems[0].contains(part), "{}", problems[0]);
    }

    let someone = || listed(1003, "Someone's", Some(299), true);
    let problems = messages(plan_with(vec![renamed(), someone(), old()], &intent("7")).unwrap_err());
    for part in [
        "gained 1001 (\"Renamed on Roblox\"), 1003 (\"Someone's\"), which do not carry",
        "someone made them by hand",
        "If one is passes.vip's, record it with",
    ] {
        assert!(problems[0].contains(part), "{}", problems[0]);
    }

    let declared = || listed(1002, "VIP", Some(299), true);
    let problems = messages(plan_with(vec![renamed(), declared(), old()], &intent("7")).unwrap_err());
    assert!(problems[0].contains("gained 1001"), "a new ID of another name stops the run beside the declared name");
    let plan = plan_with(vec![declared(), old()], &intent("7")).unwrap();
    assert_eq!(intent_of(&plan, "vip"), (Action::Adopt, Some("1002".into())));

    let plan = plan_with(vec![renamed(), old()], &intent("7, 1001")).unwrap();
    let step = plan.steps.iter().find(|step| step.key == "vip").unwrap();
    assert_eq!(
        (step.action, step.note.as_deref()),
        (Action::Create, Some("Roblox lists nothing the earlier create could have made"))
    );

    let sent_another = journal(&[("pass", "vip", "Renamed on Roblox")]);
    let problems = messages(plan_with(vec![renamed(), old()], &sent_another).unwrap_err());
    assert!(problems[0].contains("gained 1001"), "an intent without IDs stops on its sent name: {}", problems[0]);
    let sent_declared = journal(&[("pass", "vip", "VIP")]);
    let plan = plan_with(vec![declared(), old()], &sent_declared).unwrap();
    assert_eq!(intent_of(&plan, "vip"), (Action::Adopt, Some("1002".into())));
}

fn intent_of(plan: &Plan, key: &str) -> (Action, Option<String>) {
    let step = plan.steps.iter().find(|step| step.key == key).unwrap();
    (step.action, step.id.clone())
}

#[test]
fn a_resource_two_adopt_checks_find_stops_the_plan() {
    let config = CONFIG.to_owned() + "[passes.gold]\nname = \"Gold\"\nprice = 1\n";
    let project = Project::new(&config);
    let journal =
        r#"{"version":1,"universe":10772177432,"intents":[{"kind":"pass","key":"vip","name":"VIP","before":[]}]}"#;
    let mut observed = matching();
    observed.listed.insert(Kind::Pass, vec![listed(1001, "Gold", Some(299), true)]);
    let problems = messages(project.plan_seen("staging", None, Some(journal), Some(&observed)).unwrap_err());
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(problems[0].contains("passes.vip: the journal holds an intent"), "{}", problems[0]);
    assert!(
        problems[1].contains("the adopt checks of passes.gold and passes.vip both find pass 1001"),
        "{}",
        problems[1]
    );
}

#[test]
fn a_badge_whose_create_may_have_landed_waits_for_a_developer() {
    let project = Project::new(CONFIG);
    let journal = r#"{"version":1,"universe":10772177432,"intents":[{"kind":"badge","key":"first-delivery","name":"First delivery","before":[]}]}"#;
    for badges in [vec![], vec![listed(1002, "First delivery", None, true)]] {
        let mut observed = matching();
        observed.listed.insert(Kind::Badge, badges);
        let problems = messages(project.plan_seen("staging", None, Some(journal), Some(&observed)).unwrap_err());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("so provision does not settle a badge's create on its own"), "{}", problems[0]);
        assert!(problems[0].contains("run `crucible adopt staging badge first-delivery <id>`"), "{}", problems[0]);
    }
}

#[test]
fn a_change_on_roblox_is_an_update_even_when_the_lock_matches() {
    let project = Project::new(CONFIG);
    let lock = project.applied("staging");
    let mut observed = matching();
    describe(&mut observed);
    let plan = project.plan_seen("staging", Some(&lock), None, Some(&observed)).unwrap();
    assert!(plan.steps.iter().skip(1).all(|step| step.action == Action::NoOp), "{plan:#?}");
    for change in [
        |listed: &mut Listed| listed.name = "Coins".into(),
        |listed: &mut Listed| listed.description = "Free coins.".into(),
        |listed: &mut Listed| listed.price = None,
        |listed: &mut Listed| listed.on = false,
    ] {
        let mut changed = observed.clone();
        change(&mut changed.listed.get_mut(&Kind::Product).unwrap()[0]);
        let plan = project.plan_seen("staging", Some(&lock), None, Some(&changed)).unwrap();
        let step = plan.steps.iter().find(|step| step.key == "coin-pack").unwrap();
        assert_eq!((step.action, step.changes.as_slice()), (Action::Update, &[Change::Details][..]));
    }
    let mut disabled = observed.clone();
    disabled.listed.get_mut(&Kind::Badge).unwrap()[0].on = false;
    let plan = project.plan_seen("staging", Some(&lock), None, Some(&disabled)).unwrap();
    assert_eq!(intent(&plan, "first-delivery").0, Action::Update);
}

#[test]
fn a_lock_id_roblox_lacks_stops_the_plan() {
    let project = Project::new(CONFIG);
    let lock = project.applied("staging");
    let mut observed = matching();
    describe(&mut observed);
    let product = observed.listed.get_mut(&Kind::Product).unwrap().remove(0);
    let mut gone = observed.clone();
    gone.missing.insert((Kind::Product, product.id), None);
    let problems = messages(project.plan_seen("staging", Some(&lock), None, Some(&gone)).unwrap_err());
    assert!(
        problems[0].contains("Roblox has no product 1000. Provision never replaces a resource on its own"),
        "{problems:#?}"
    );
    assert!(problems[0].contains("delete [env.staging.products.coin-pack] from crucible.lock"), "{}", problems[0]);

    let mut hidden = observed.clone();
    hidden.missing.insert((Kind::Product, product.id), Some(product.clone()));
    let problems = messages(project.plan_seen("staging", Some(&lock), None, Some(&hidden)).unwrap_err());
    assert!(problems[0].contains("list of products leaves out product 1000"), "{problems:#?}");
    let mut off_sale = observed.clone();
    off_sale.missing.insert((Kind::Product, product.id), Some(Listed { on: false, ..product }));
    let problems = messages(project.plan_seen("staging", Some(&lock), None, Some(&off_sale)).unwrap_err());
    assert!(problems[0].contains("leaves out product 1000"), "only a badge is taken when off its list: {problems:#?}");

    let mut badge = matching();
    describe(&mut badge);
    let disabled = Listed { on: false, ..badge.listed.get_mut(&Kind::Badge).unwrap().remove(0) };
    badge.missing.insert((Kind::Badge, disabled.id), Some(disabled));
    let config = CONFIG.replace(
        "[env.staging.products.coin-pack]",
        "[env.staging.badges.first-delivery]\nenabled = false\n\n[env.staging.products.coin-pack]",
    );
    let disabled_project = Project::new(&config);
    let plan =
        disabled_project.plan_seen("staging", Some(&disabled_project.applied("staging")), None, Some(&badge)).unwrap();
    assert_eq!(intent(&plan, "first-delivery").0, Action::NoOp, "a disabled badge found by its ID is taken");
}

#[test]
fn a_retired_resource_back_on_sale_is_retired_again() {
    let config = CONFIG.replace("price = 299\n", "price = 299\nretired = true\n");
    let project = Project::new(&config);
    let lock = project.applied("staging");
    let mut observed = matching();
    describe(&mut observed);
    let plan = project.plan_seen("staging", Some(&lock), None, Some(&observed)).unwrap();
    let step = plan.steps.iter().find(|step| step.key == "vip").unwrap();
    assert_eq!((step.action, step.note.as_deref()), (Action::Retire, Some("back on sale or enabled on Roblox")));
    observed.listed.get_mut(&Kind::Pass).unwrap()[0].on = false;
    let plan = project.plan_seen("staging", Some(&lock), None, Some(&observed)).unwrap();
    assert_eq!(intent(&plan, "vip").0, Action::NoOp);
    let vip = observed.listed.get_mut(&Kind::Pass).unwrap().remove(0);
    observed.missing.insert((Kind::Pass, vip.id), None);
    let plan = project.plan_seen("staging", Some(&lock), None, Some(&observed)).unwrap();
    assert_eq!(intent(&plan, "vip").0, Action::NoOp, "a retired resource gone from Roblox is left alone");
}

#[test]
fn a_moved_key_that_is_retired_is_retired_and_then_only_renamed_in_the_lock() {
    let lock = Project::new(CONFIG).applied("staging");
    let moved = CONFIG.replace("[passes.vip]", "[passes.gold]\nmoved_from = \"vip\"\nretired = true");
    let plan = Project::new(&moved).plan("staging", Some(&lock), None).unwrap();
    let gold = plan.steps.iter().find(|step| step.key == "gold").unwrap();
    assert_eq!((gold.action, gold.changes.as_slice()), (Action::Retire, &[Change::Key][..]));
    let retired = Project::new(&CONFIG.replace("price = 299\n", "price = 299\nretired = true\n")).applied("staging");
    let plan = Project::new(&moved).plan("staging", Some(&retired), None).unwrap();
    let gold = plan.steps.iter().find(|step| step.key == "gold").unwrap();
    assert_eq!((gold.action, gold.changes.as_slice()), (Action::Update, &[Change::Key][..]));
}

#[test]
fn a_badge_differs_in_any_one_field() {
    let fields = Fields::Badge { name: "First".into(), description: "Deliver.".into(), enabled: true };
    let same =
        Listed { id: Id::new(1).unwrap(), name: "First".into(), description: "Deliver.".into(), price: None, on: true };
    assert!(!differs(&fields, &same));
    assert!(differs(&fields, &Listed { name: "Other".into(), ..same.clone() }));
    assert!(differs(&fields, &Listed { description: "Other.".into(), ..same.clone() }));
    assert!(differs(&fields, &Listed { on: false, ..same.clone() }));
    let priced = Fields::Priced { name: "First".into(), description: "Deliver.".into(), price: 5 };
    let sold = Listed { price: Some(5), ..same.clone() };
    assert!(!differs(&priced, &sold));
    for other in [
        Listed { name: "Other".into(), ..sold.clone() },
        Listed { description: "Other.".into(), ..sold.clone() },
        Listed { price: Some(6), ..sold.clone() },
        Listed { price: None, ..sold.clone() },
        Listed { on: false, ..sold.clone() },
    ] {
        assert!(differs(&priced, &other), "{other:?}");
    }
}
