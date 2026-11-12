use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 10772177432
place = 132306245523809

[env.production]
universe = 10769815992
place = 116566609396165

[products.coin-pack]
name = "Coin pack"
price = 49

[badges.first-delivery]
name = "First delivery"
"#;

fn project(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("crucible.toml"), config).unwrap();
    dir
}

fn plan(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crucible")).current_dir(dir).arg("plan").args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("stdout is not JSON: {}", text(&output.stdout)))
}

#[test]
fn a_first_plan_prints_the_creates_of_one_environment() {
    let dir = project(CONFIG);
    let out = plan(dir.path(), &["staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "staging (universe 10772177432)\n\
         \x20 create  product coin-pack     Products.CoinPack\n\
         \x20 create  badge first-delivery  Badges.FirstDelivery\n\
         \x20 2 create.\n\
         \x20 Provision adopts the one existing resource with a create's name instead of creating it.\n"
    );
}

#[test]
fn json_plans_every_environment_and_reads_the_lock() {
    let dir = project(CONFIG);
    std::fs::write(
        dir.path().join("crucible.lock"),
        "version = 1\n[env.production]\nuniverse = 10769815992\n[env.production.products.coin-pack]\nid = 3301234567\n\
         fingerprint = \"sha256:old\"\nstatus = \"managed\"\n",
    )
    .unwrap();
    let out = plan(dir.path(), &["--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let document = document(&out);
    let environments = document["environments"].as_array().unwrap();
    assert_eq!(environments.iter().map(|plan| &plan["environment"]).collect::<Vec<_>>(), ["production", "staging"]);
    assert_eq!(
        environments[0]["steps"][0],
        json!({"kind":"product","key":"coin-pack","handle":"Products.CoinPack","action":"update","id":"3301234567","changes":["details"]})
    );
    assert_eq!(environments[0]["summary"], json!({"create": 1, "update": 1}));
    assert_eq!(environments[1]["steps"][0]["id"], Value::Null);
}

#[test]
fn a_broken_rule_exits_2_and_names_the_file_line_and_key() {
    let dir = project(&CONFIG.replace("price = 49", "price = 0"));
    let out = plan(dir.path(), &["--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        document(&out)["error"]["problems"],
        json!([{"file":"crucible.toml","line":12,"key":"products.coin-pack.price","message":"a price of 0 does not take a product off sale. Set retired = true to take it off sale"}])
    );
    let out = plan(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stdout).is_empty());
    assert!(
        text(&out.stderr).contains("\n  crucible.toml:12: products.coin-pack.price: a price of 0"),
        "{}",
        text(&out.stderr)
    );
}

const STAGING_LOCK: &str = "version = 1\n[env.staging]\nuniverse = 10772177432\n[env.staging.products.coin-pack]\n\
                            id = 3301234567\nfingerprint = \"sha256:old\"\nstatus = \"managed\"\n";

/// Runs `plan staging` as text and as JSON, checks both exit 2, and returns the JSON problems.
fn refused(dir: &Path) -> Value {
    let out = plan(dir, &["staging"]);
    assert_eq!(out.status.code(), Some(2), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stdout).is_empty());
    assert!(text(&out.stderr).starts_with("the plan breaks the provisioning rules:\n  "), "{}", text(&out.stderr));
    let out = plan(dir, &["staging", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let document = document(&out);
    assert_eq!(document["error"]["kind"], "usage");
    document["error"]["problems"].clone()
}

#[test]
fn a_lock_for_another_universe_exits_2() {
    let dir = project(&CONFIG.replace("universe = 10772177432", "universe = 99999999999"));
    std::fs::write(dir.path().join("crucible.lock"), STAGING_LOCK).unwrap();
    let problems = refused(dir.path());
    assert_eq!((&problems[0]["file"], &problems[0]["line"]), (&json!("crucible.lock"), &json!(3)));
    assert_eq!(problems[0]["key"], "env.staging.universe");
    let message = problems[0]["message"].as_str().unwrap();
    assert!(message.contains("universe 10772177432") && message.contains("universe 99999999999"), "{message}");
}

#[test]
fn a_locked_resource_removed_from_the_config_exits_2() {
    let dir = project(&CONFIG.replace("[products.coin-pack]\nname = \"Coin pack\"\nprice = 49\n", ""));
    std::fs::write(dir.path().join("crucible.lock"), STAGING_LOCK).unwrap();
    let problems = refused(dir.path());
    assert_eq!((&problems[0]["file"], &problems[0]["key"]), (&json!("crucible.toml"), &json!("products.coin-pack")));
}

fn write_journal(dir: &Path, journal: &Value) {
    std::fs::create_dir_all(dir.join(".crucible/journal")).unwrap();
    std::fs::write(dir.join(".crucible/journal/staging.json"), journal.to_string()).unwrap();
}

fn intents(universe: u64, intents: Value) -> Value {
    json!({"version": 1, "universe": universe, "intents": intents})
}

#[test]
fn an_intent_shows_on_its_step_with_the_name_it_sent() {
    let dir = project(CONFIG);
    write_journal(dir.path(), &intents(10772177432, json!([{"kind":"product","key":"coin-pack","name":"Coins"}])));
    let out = plan(dir.path(), &["staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "staging (universe 10772177432)\n\
         \x20 adopt-check  product coin-pack     Products.CoinPack       intent sent \"Coins\"\n\
         \x20 create       badge first-delivery  Badges.FirstDelivery\n\
         \x20 1 create, 1 adopt-check.\n\
         \x20 Provision adopts the one existing resource with a create's name instead of creating it.\n"
    );
    let document = document(&plan(dir.path(), &["staging", "--json"]));
    assert_eq!(document["environments"][0]["steps"][0]["intent"], json!({"name": "Coins", "resolved": false}));
}

#[test]
fn an_intent_no_step_carries_exits_2() {
    let dir = project(CONFIG);
    write_journal(dir.path(), &intents(10772177432, json!([{"kind":"pass","key":"vip","name":"VIP"}])));
    let problems = refused(dir.path());
    assert_eq!(problems[0]["key"], "passes.vip");
    assert!(problems[0]["file"].as_str().unwrap().ends_with("staging.json"), "{problems}");
}

#[test]
fn a_lock_table_or_journal_for_an_undeclared_environment_exits_2() {
    let renamed = CONFIG.replace("[env.staging]", "[env.stage]");
    let dir = project(&renamed);
    std::fs::write(dir.path().join("crucible.lock"), STAGING_LOCK).unwrap();
    for args in [&["stage"][..], &[]] {
        let out = plan(dir.path(), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", text(&out.stdout));
        assert!(
            text(&out.stderr).contains("crucible.lock:3: env.staging: crucible.toml declares no environment staging")
        );
    }
    let dir = project(&CONFIG.replace("[env.production]\nuniverse = 10769815992\nplace = 116566609396165\n", ""));
    std::fs::create_dir_all(dir.path().join(".crucible/journal")).unwrap();
    let journal = intents(10769815992, json!([{"kind":"pass","key":"vip","name":"VIP"}]));
    std::fs::write(dir.path().join(".crucible/journal/production.json"), journal.to_string()).unwrap();
    std::fs::write(dir.path().join(".crucible/journal/notes.txt"), "not a journal").unwrap();
    let problems = refused(dir.path());
    assert_eq!(problems.as_array().unwrap().len(), 1, "{problems}");
    assert_eq!((&problems[0]["key"], &problems[0]["line"]), (&json!("env.production"), &Value::Null));
    assert!(problems[0]["file"].as_str().unwrap().ends_with("production.json"), "{problems}");
    let message = problems[0]["message"].as_str().unwrap();
    assert!(message.contains("holds an intent to create pass vip as \"VIP\" in universe 10769815992"), "{message}");
}

#[test]
fn a_journal_for_another_universe_exits_2() {
    let dir = project(CONFIG);
    write_journal(dir.path(), &intents(99999999999, json!([])));
    let problems = refused(dir.path());
    assert_eq!(problems[0]["key"], "universe");
}

#[test]
fn a_name_another_key_s_intent_sent_exits_2() {
    let dir = project(CONFIG);
    write_journal(dir.path(), &intents(10772177432, json!([{"kind":"product","key":"gem-pack","name":"Coin pack"}])));
    let problems = refused(dir.path());
    let keys: Vec<_> = problems.as_array().unwrap().iter().map(|problem| problem["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["products.gem-pack", "products.coin-pack.name"]);
    assert_eq!((&problems[1]["file"], &problems[1]["line"]), (&json!("crucible.toml"), &json!(12)));
}

#[test]
fn two_keys_of_a_kind_with_one_display_name_exit_2() {
    let dir = project(&(CONFIG.to_owned() + "[products.gem-pack]\nname = \"Coin pack\"\nprice = 99\n"));
    let out = plan(dir.path(), &["staging", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let problems = &document(&out)["error"]["problems"];
    assert_eq!((&problems[0]["key"], &problems[0]["line"]), (&json!("products.gem-pack.name"), &json!(18)));
    let out = plan(dir.path(), &["staging"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("products.coin-pack and products.gem-pack both have the display name"));
}

#[test]
fn two_environments_with_one_universe_exit_2() {
    let dir = project(&CONFIG.replace("universe = 10769815992", "universe = 10772177432"));
    let out = plan(dir.path(), &["production", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let problems = &document(&out)["error"]["problems"];
    assert_eq!((&problems[0]["key"], &problems[0]["line"]), (&json!("env.staging.universe"), &Value::Null));
    let out = plan(dir.path(), &["production"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("env.production and env.staging both name universe 10772177432"));
}

#[test]
fn a_file_that_does_not_parse_names_its_line_as_the_guide_says() {
    let guide = include_str!("../../../docs/experience.md").replace("\r\n", "\n");
    let example = guide.split_once("A file that does not parse fails with the file and the line, such as `").unwrap().1;
    let example = example.split_once('`').unwrap().0;
    let dir = project(&CONFIG.replace("price = 49", "price = \"49\""));
    let out = plan(dir.path(), &["staging"]);
    assert_eq!((out.status.code(), text(&out.stderr)), (Some(2), format!("{example}\n")));
    let out = plan(dir.path(), &["staging", "--json"]);
    assert_eq!(document(&out)["error"], json!({"kind": "usage", "message": example, "problems": []}));
}

#[test]
fn an_unknown_environment_a_bad_lock_or_no_config_exits_2() {
    let dir = project(CONFIG);
    let out = plan(dir.path(), &["qa"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(text(&out.stderr), "unknown environment qa. Environments: production, staging\n");
    std::fs::write(dir.path().join("crucible.lock"), "version = 2\n").unwrap();
    let out = plan(dir.path(), &["staging", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(document(&out)["error"]["kind"], "usage");
    assert!(document(&out)["error"]["message"].as_str().unwrap().starts_with("crucible.lock has format 2"));
    let empty = tempfile::tempdir().unwrap();
    let out = plan(empty.path(), &["staging"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).starts_with("plan needs crucible.toml"));
}

#[test]
fn the_guide_example_plans() {
    let guide = include_str!("../../../docs/experience.md").replace("\r\n", "\n");
    let (_, rest) = guide.split_once("```toml\n").unwrap();
    let (config, _) = rest.split_once("```").unwrap();
    let dir = project(config);
    for icon in ["assets/icons/coin-pack.png", "assets/badges/first-delivery.png"] {
        std::fs::create_dir_all(dir.path().join(icon).parent().unwrap()).unwrap();
        std::fs::write(dir.path().join(icon), b"png").unwrap();
    }
    let out = plan(dir.path(), &["staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = guide.split_once("```text\n").unwrap().1.split_once("```").unwrap().0;
    assert_eq!(text(&out.stdout), shown, "docs/experience.md shows the plan of its own example");
}

#[test]
fn a_stray_file_in_the_journal_folder_names_itself_and_the_fix() {
    let dir = project(CONFIG);
    std::fs::create_dir_all(dir.path().join(".crucible/journal")).unwrap();
    std::fs::write(dir.path().join(".crucible/journal/notes.json"), "{\"todo\": []}").unwrap();
    let out = plan(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    let path = Path::new(".crucible").join("journal");
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&Path::new(".crucible/journal/notes.json").display().to_string()), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "Each .json file in {} is the journal of the environment its name gives, so a file that is not one stops \
             every plan. If it is not, move it out of the folder",
            path.display()
        )),
        "{stderr}"
    );
}
