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
        "version = 1\n[env.production.products.coin-pack]\nid = 3301234567\nfingerprint = \"sha256:old\"\nstatus = \"managed\"\n",
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
