use crucible_iac::lock::Lock;
use crucible_iac::model::{Experience, Kind};
use crucible_iac::oc::scopes;
use crucible_iac::plan::{Id, Inputs, Listed, Observed, plan};
use crucible_iac::provision::{Run, Write};
use crucible_project::Manifest;
use std::collections::BTreeMap;
use std::path::Path;

fn names(reads: &[Kind], writes: &[Write]) -> Vec<String> {
    scopes(reads, writes).into_iter().map(|scope| scope.name).collect()
}

#[test]
fn reads_need_the_read_scopes_of_their_kinds_and_badges_need_none() {
    assert_eq!(names(&[Kind::Product, Kind::Pass], &[]), ["developer-product:read", "game-pass:read"]);
    assert_eq!(names(&[Kind::Badge], &[]), Vec::<String>::new(), "the public badge list needs no key");
}

#[test]
fn each_write_needs_the_scopes_its_operation_declares() {
    assert_eq!(names(&[], &[Write::Create(Kind::Product)]), ["developer-product:write"]);
    assert_eq!(names(&[], &[Write::Create(Kind::Badge)]), ["legacy-universe.badge:manage-and-spend-robux"]);
    let retire = Write::Update { kind: Kind::Pass, details: true, icon: false };
    assert_eq!(names(&[], &[retire]), ["game-pass:write"]);
    let badge = |details, icon| Write::Update { kind: Kind::Badge, details, icon };
    assert_eq!(names(&[], &[badge(false, true)]), ["legacy-badge:manage"], "only an icon change needs it");
    assert_eq!(
        names(&[], &[badge(true, false)]),
        ["legacy-universe.badge:manage-and-spend-robux", "legacy-universe.badge:write"]
    );
    assert_eq!(names(&[], &[Write::Settings]), ["universe:write"]);
}

#[test]
fn a_scope_two_calls_need_is_named_once() {
    let writes = [Write::Create(Kind::Product), Write::Update { kind: Kind::Product, details: true, icon: true }];
    assert_eq!(names(&[Kind::Product], &writes), ["developer-product:read", "developer-product:write"]);
}

const DECLARED: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 1
place = 2

[products.coin-pack]
name = "Coin pack"
price = 49

[passes.vip]
name = "VIP"
price = 299

[badges.first-delivery]
name = "First delivery"
"#;

/// The writes a run sends when the lock holds each declared key and Roblox lists `listed` for them.
fn writes(listed: [Listed; 3]) -> Vec<Write> {
    let manifest = Manifest::parse(DECLARED).unwrap();
    let experience = Experience::new(&manifest).unwrap();
    let mut lock = "version = 1\n\n[env.staging]\nuniverse = 1\n".to_owned();
    for (table, key, id) in [("products", "coin-pack", 10), ("passes", "vip", 20), ("badges", "first-delivery", 30)] {
        lock += &format!("\n[env.staging.{table}.{key}]\nid = {id}\nfingerprint = \"x\"\nstatus = \"managed\"\n");
    }
    let lock = Lock::parse(&lock).unwrap();
    let [product, pass, badge] = listed;
    let observed = Observed {
        listed: BTreeMap::from([(Kind::Product, vec![product]), (Kind::Pass, vec![pass]), (Kind::Badge, vec![badge])]),
        ..Observed::default()
    };
    let env = &manifest.envs["staging"];
    let inputs = Inputs {
        experience: &experience,
        env_name: "staging",
        env,
        lock: lock.env("staging"),
        journal: None,
        observed: Some(&observed),
    };
    let mut read = |_: &Path| -> std::io::Result<Vec<u8>> { unreachable!("nothing has an icon") };
    let plan = plan(&inputs, &mut read).unwrap();
    Run::new(&inputs, &plan, &lock, &mut read).unwrap().writes()
}

/// Changes one field of what Roblox lists for the product, the pass and the badge.
type Change = fn(&mut [Listed; 3]);

fn listed(id: u64, name: &str, price: Option<u64>) -> Listed {
    Listed { id: Id::new(id).unwrap(), name: name.into(), description: String::new(), price, on: true }
}

#[test]
fn an_update_of_any_one_field_needs_the_write_scope_of_its_kind() {
    let matching =
        || [listed(10, "Coin pack", Some(49)), listed(20, "VIP", Some(299)), listed(30, "First delivery", None)];
    assert_eq!(writes(matching()), [], "a lock change alone sends nothing");
    let details = |kind| Write::Update { kind, details: true, icon: false };
    let changed: [(&str, Change); 4] = [
        ("name", |listed| listed[0].name = "Old".into()),
        ("description", |listed| listed[1].description = "Old".into()),
        ("price", |listed| listed[0].price = Some(10)),
        ("enabled", |listed| listed[2].on = false),
    ];
    let expected = [details(Kind::Product), details(Kind::Pass), details(Kind::Product), details(Kind::Badge)];
    for ((field, change), expected) in changed.into_iter().zip(expected) {
        let mut listed = matching();
        change(&mut listed);
        let writes = writes(listed);
        assert_eq!(writes, [expected], "{field}");
        let needed = names(&[], &writes);
        assert!(!needed.is_empty(), "{field}: {needed:?}");
    }
}
