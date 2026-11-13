use httpmock::{Method, MockServer};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = r#"[places.start]
project = "default.project.json"

[env.staging]
universe = 1
place = 2

[products.coin-pack]
name = "Coin pack"
price = 49

[badges.first-delivery]
name = "First delivery"
"#;

const PRODUCTS: &str = "/developer-products/v2/universes/1/developer-products/creator";
const BADGES: &str = "/v1/universes/1/badges";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("crucible.toml"), CONFIG).unwrap();
    dir
}

fn crucible(dir: &Path, server: &MockServer, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crucible"))
        .current_dir(dir)
        .args(args)
        .env("CRUCIBLE_OC_API_KEY", "test-key")
        .env("CRUCIBLE_OC_ORIGIN", server.base_url())
        .env_remove("HTTPS_PROXY")
        .env_remove("HTTP_PROXY")
        .env_remove("ALL_PROXY")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("stdout is not JSON: {}", text(&output.stdout)))
}

fn product(id: u64) -> Value {
    json!({"productId": id, "name": "Coin pack", "description": "", "isForSale": true,
           "priceInformation": {"defaultPriceInRobux": 49}})
}

fn badge(id: u64) -> Value {
    json!({"id": id, "name": "First delivery", "description": "", "enabled": true})
}

fn lists(server: &MockServer, products: Vec<Value>, badges: Vec<Value>) -> (httpmock::Mock<'_>, httpmock::Mock<'_>) {
    let products = server.mock(|when, then| {
        when.method(Method::GET).path(PRODUCTS);
        then.status(200).json_body(json!({"developerProducts": products, "nextPageToken": null}));
    });
    let badges = server.mock(|when, then| {
        when.method(Method::GET).path(BADGES);
        then.status(200).json_body(json!({"data": badges, "nextPageCursor": null}));
    });
    (products, badges)
}

fn journal(dir: &Path, text: &str) {
    std::fs::create_dir_all(dir.join(".crucible").join("journal")).unwrap();
    std::fs::write(dir.join(".crucible").join("journal").join("staging.json"), text).unwrap();
}

#[test]
fn provision_adopts_by_name_creates_the_rest_and_records_each_id() {
    let dir = project();
    let server = MockServer::start();
    let (mut products, mut badges) = lists(&server, vec![product(77)], vec![]);
    let create = server.mock(|when, then| {
        when.method(Method::POST)
            .path("/legacy-badges/v1/universes/1/badges")
            .header("x-api-key", "test-key")
            .body_includes("name=\"name\"\r\n\r\nFirst delivery\r\n")
            .body_includes("name=\"expectedCost\"\r\n\r\n0\r\n");
        then.status(200).json_body(badge(88));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    create.assert_calls(1);
    assert_eq!(
        text(&out.stdout),
        "staging (universe 1)\n\
         \x20 adopt   product coin-pack     Products.CoinPack     77\n\
         \x20 create  badge first-delivery  Badges.FirstDelivery\n\
         \x20 1 create, 1 adopt.\n\
         \x20 Provision adopts the one existing resource with a create's name instead of creating it.\n\
         adopt product coin-pack 77\n\
         create badge first-delivery 88\n\
         Provisioned staging. Commit crucible.lock.\n"
    );
    let lock = std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap();
    assert!(lock.contains("[env.staging.products.coin-pack]\nid = 77\n"), "{lock}");
    assert!(lock.contains("[env.staging.badges.first-delivery]\nid = 88\n"), "{lock}");
    assert!(!dir.path().join(".crucible/journal/staging.json").exists(), "a finished run leaves no intent");

    products.delete();
    badges.delete();
    lists(&server, vec![product(77)], vec![badge(88)]);
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let document = document(&out);
    assert_eq!(document["applied"], json!([]), "a second run has nothing to do");
    assert_eq!(document["plan"]["summary"], json!({"no-op": 2}));
    create.assert_calls(1);
}

#[test]
fn a_lost_create_keeps_its_intent_and_the_next_run_asks_whose_a_renamed_one_is() {
    let dir = project();
    let server = MockServer::start();
    let (mut products, mut badges) = lists(&server, vec![], vec![badge(88)]);
    let mut create = server.mock(|when, then| {
        when.method(Method::POST).path("/developer-products/v2/universes/1/developer-products");
        then.status(503);
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(5), "{}", text(&out.stderr));
    create.assert_calls(1);
    let first = document(&out);
    assert_eq!(
        (first["error"]["kind"].as_str(), first["error"]["resource"].as_str()),
        (Some("outcome_unknown"), Some("product coin-pack"))
    );
    assert!(first["error"]["hint"].as_str().unwrap().contains("Its intent stays in"), "{first}");
    assert_eq!(first["applied"], json!([{"kind": "badge", "key": "first-delivery", "action": "adopt", "id": "88"}]));
    let journal: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join(".crucible/journal/staging.json")).unwrap())
            .unwrap();
    assert_eq!(journal["intents"], json!([{"kind": "product", "key": "coin-pack", "name": "Coin pack", "before": []}]));

    products.delete();
    badges.delete();
    create.delete();
    let renamed = json!({"productId": 99, "name": "Renamed on Roblox", "description": "", "isForSale": true,
                         "priceInformation": {"defaultPriceInRobux": 49}});
    lists(&server, vec![renamed], vec![badge(88)]);
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    let stderr = text(&out.stderr);
    for part in ["gained 99 (\"Renamed on Roblox\")", "`crucible adopt staging product coin-pack <id>`"] {
        assert!(stderr.contains(part), "{stderr}");
    }
    let lock = std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap();
    assert!(!lock.contains("id = 99"), "a stopped plan binds nothing: {lock}");
    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "coin-pack", "99"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let update = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/developer-products/v2/universes/1/developer-products/99")
            .body_includes("name=\"name\"\r\n\r\nCoin pack\r\n");
        then.status(204);
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
    update.assert_calls(1);
    assert_eq!(
        document(&out)["applied"],
        json!([{"kind": "product", "key": "coin-pack", "action": "update", "id": "99"}])
    );
    assert!(!dir.path().join(".crucible/journal/staging.json").exists());
}

#[test]
fn a_badge_create_refused_for_its_cost_clears_its_intent_and_the_next_day_creates_it() {
    let dir = project();
    let server = MockServer::start();
    let (mut products, mut badges) = lists(&server, vec![product(77)], vec![]);
    let mut refused = server.mock(|when, then| {
        when.method(Method::POST).path("/legacy-badges/v1/universes/1/badges");
        then.status(400).json_body(json!({"errors": [{"code": 0, "message": "Insufficient Robux"}]}));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(4), "{}", text(&out.stdout));
    let first = document(&out);
    assert_eq!(first["error"]["resource"], "badge first-delivery");
    assert!(first["error"].get("hint").is_none(), "nothing may have landed: {first}");
    assert!(!dir.path().join(".crucible/journal/staging.json").exists(), "the refused create leaves no intent");

    products.delete();
    badges.delete();
    refused.delete();
    lists(&server, vec![product(77)], vec![]);
    let create = server.mock(|when, then| {
        when.method(Method::POST).path("/legacy-badges/v1/universes/1/badges");
        then.status(200).json_body(badge(88));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    create.assert_calls(1);
    let lock = std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap();
    assert!(lock.contains("[env.staging.badges.first-delivery]\nid = 88\n"), "{lock}");
}

#[test]
fn adopt_none_clears_an_intent_whose_create_never_landed_and_provision_creates_it() {
    let dir = project();
    journal(
        dir.path(),
        r#"{"version":1,"universe":1,"intents":[{"kind":"product","key":"coin-pack","name":"Coin pack","before":[]}]}"#,
    );
    let server = MockServer::start();
    let stranger = json!({"productId": 5, "name": "Teammate's pack", "description": "", "isForSale": true,
                          "priceInformation": {"defaultPriceInRobux": 99}});
    lists(&server, vec![stranger], vec![badge(88)]);
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("`crucible adopt staging product coin-pack --none`"), "{}", text(&out.stderr));

    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "coin-pack", "--none", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
    assert_eq!(
        document(&out),
        json!({"applied": [{"kind": "product", "key": "coin-pack", "action": "forget", "id": ""}], "passedOver": [5]})
    );
    assert!(!dir.path().join(".crucible/journal/staging.json").exists());
    let create = server.mock(|when, then| {
        when.method(Method::POST).path("/developer-products/v2/universes/1/developer-products");
        then.status(200).json_body(product(6));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    create.assert_calls(1);
    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "coin-pack", "--none"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("the journal holds no intent for products.coin-pack"), "{}", text(&out.stderr));
}

#[test]
fn adopt_refuses_a_badge_another_universe_awards() {
    let dir = project();
    journal(
        dir.path(),
        r#"{"version":1,"universe":1,"intents":[{"kind":"badge","key":"first-delivery","name":"First delivery","before":[]}]}"#,
    );
    let server = MockServer::start();
    lists(&server, vec![product(77)], vec![]);
    server.mock(|when, then| {
        when.method(Method::GET).path("/v1/badges/555");
        then.status(200).json_body(json!({"id": 555, "name": "Someone else's badge", "description": "",
            "enabled": false, "awardingUniverse": {"id": 999, "name": "Other game", "rootPlaceId": 3}}));
    });
    let out = crucible(dir.path(), &server, &["adopt", "staging", "badge", "first-delivery", "555", "--json"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    let message = document(&out)["error"]["message"].as_str().unwrap().to_owned();
    assert!(message.contains("universe 1 has no badge 555"), "{message}");
    assert!(message.contains("provision takes none that belongs to another universe"), "{message}");
    assert!(!dir.path().join("crucible.lock").exists(), "nothing was written");
}

#[test]
fn adopt_refuses_while_provision_would_still_stop() {
    let dir = project();
    journal(
        dir.path(),
        r#"{"version":1,"universe":1,"intents":[{"kind":"product","key":"gem-pack","name":"Gem pack","before":[]}]}"#,
    );
    let server = MockServer::start();
    lists(&server, vec![product(77)], vec![]);
    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "coin-pack", "77"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("provision would still stop on these after it, so nothing was written:"), "{stderr}");
    assert!(stderr.contains("crucible.toml declares no products.gem-pack"), "{stderr}");
    assert!(!dir.path().join("crucible.lock").exists());

    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "gem-pack", "--none"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "coin-pack", "77"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn a_dry_run_lists_and_plans_and_writes_nothing() {
    let dir = project();
    let server = MockServer::start();
    lists(&server, vec![], vec![]);
    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).ends_with("  2 create.\n  Provision adopts the one existing resource with a create's name instead of creating it.\nDry run: nothing was written.\n"), "{}", text(&out.stdout));
    assert!(!dir.path().join("crucible.lock").exists());
}

#[test]
fn two_resources_with_a_declared_name_stop_the_run_until_one_is_adopted() {
    let dir = project();
    let server = MockServer::start();
    lists(&server, vec![product(5), product(6)], vec![badge(88)]);
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("`crucible adopt staging product coin-pack <id>`"), "{}", text(&out.stderr));
    assert!(!dir.path().join("crucible.lock").exists(), "a stopped plan writes nothing");

    let get = server.mock(|when, then| {
        when.method(Method::GET).path("/developer-products/v2/universes/1/developer-products/6/creator");
        then.status(200).json_body(product(6));
    });
    let out = crucible(dir.path(), &server, &["adopt", "staging", "product", "coin-pack", "6"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    get.assert_calls(0);
    let lock = std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap();
    assert!(lock.contains("[env.staging.products.coin-pack]\nid = 6\n"), "{lock}");

    let out = crucible(dir.path(), &server, &["adopt", "staging", "badge", "first-delivery", "6", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let message = document(&out)["error"]["message"].as_str().unwrap().to_owned();
    assert!(message.contains("universe 1 has no badge 6"), "{message}");
}

#[test]
fn provision_without_a_key_stops_before_any_call() {
    let dir = project();
    let server = MockServer::start();
    let out = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .current_dir(dir.path())
        .args(["provision", "staging", "--json"])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .env("CRUCIBLE_OC_ORIGIN", server.base_url())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stdout));
    assert_eq!(document(&out)["error"]["kind"], "auth");
}

#[test]
fn provision_writes_the_experience_settings_and_records_them_in_the_lock() {
    let dir = project();
    std::fs::write(dir.path().join("crucible.toml"), format!("{CONFIG}\n[experience]\nvoice_chat = true\n")).unwrap();
    let lock = "version = 1\n\n[env.staging]\nuniverse = 1\n\n[env.staging.products.coin-pack]\nid = 77\n\
                fingerprint = \"x\"\nstatus = \"managed\"\n\n[env.staging.badges.first-delivery]\nid = 88\n\
                fingerprint = \"x\"\nstatus = \"managed\"\n";
    std::fs::write(dir.path().join("crucible.lock"), lock).unwrap();
    let server = MockServer::start();
    let (mut products, mut badges) = lists(&server, vec![product(77)], vec![badge(88)]);
    let settings = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/cloud/v2/universes/1")
            .query_param("updateMask", "voiceChatEnabled")
            .header("x-api-key", "test-key")
            .json_body(json!({"voiceChatEnabled": true}));
        then.status(200).json_body(json!({"path": "universes/1", "voiceChatEnabled": true}));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    settings.assert_calls(1);
    let stdout = text(&out.stdout);
    assert!(stdout.contains("  update  experience settings"), "{stdout}");
    assert!(stdout.contains("  details, voice_chat = true\n"), "the plan shows the value it writes: {stdout}");
    assert!(stdout.contains("update experience settings 1\n"), "{stdout}");
    assert!(!stdout.contains("does not write"), "{stdout}");
    let lock = std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap();
    assert!(lock.contains("[env.staging.experience]\nfingerprint = \"sha256:"), "{lock}");

    products.delete();
    badges.delete();
    lists(&server, vec![product(77)], vec![badge(88)]);
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    settings.assert_calls(1);
    let document = document(&out);
    assert_eq!(document["applied"], json!([]), "the lock records the settings it applied");
    assert_eq!(document["plan"]["steps"][0]["settings"], json!([{"key": "voice_chat", "value": true}]));
}

#[test]
fn a_refused_settings_update_comes_after_every_resource_step_and_writes_no_fingerprint() {
    let dir = project();
    std::fs::write(dir.path().join("crucible.toml"), format!("{CONFIG}\n[experience]\nprivate_server_price = 0\n"))
        .unwrap();
    let server = MockServer::start();
    lists(&server, vec![product(77)], vec![]);
    let create = server.mock(|when, then| {
        when.method(Method::POST).path("/legacy-badges/v1/universes/1/badges");
        then.status(200).json_body(badge(88));
    });
    let settings = server.mock(|when, then| {
        when.method(Method::PATCH).path("/cloud/v2/universes/1");
        then.status(400).json_body(json!({"message": "bad"}));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(4), "{}", text(&out.stderr));
    create.assert_calls(1);
    settings.assert_calls(1);
    let document = document(&out);
    assert_eq!(document["error"]["resource"], "experience settings", "{document}");
    assert!(
        document["error"]["hint"].as_str().unwrap().contains("every product, pass and badge step was applied"),
        "{document}"
    );
    assert_eq!(
        document["applied"],
        json!([{"kind": "product", "key": "coin-pack", "action": "adopt", "id": "77"},
               {"kind": "badge", "key": "first-delivery", "action": "create", "id": "88"}])
    );
    let lock = std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap();
    assert!(lock.contains("[env.staging.badges.first-delivery]\nid = 88\n"), "{lock}");
    assert!(!lock.contains("[env.staging.experience]"), "{lock}");
    assert!(!dir.path().join(".crucible/journal/staging.json").exists(), "no intent is left");
}

fn introspection(server: &MockServer, scopes: Value, enabled: bool) -> httpmock::Mock<'_> {
    server.mock(|when, then| {
        when.method(Method::POST).path("/api-keys/v1/introspect").json_body(json!({"apiKey": "test-key"}));
        then.status(200).json_body(json!({"name": "ci", "enabled": enabled, "expired": false, "scopes": scopes}));
    })
}

#[test]
fn a_key_without_the_read_scopes_stops_provision_before_its_first_list() {
    let dir = project();
    let server = MockServer::start();
    let (products, badges) = lists(&server, vec![], vec![]);
    introspection(&server, json!([{"name": "universe-places", "operations": ["write"]}]), true);
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    let document = document(&out);
    assert_eq!(document["error"]["missingScopes"], json!(["developer-product:read"]), "{document}");
    let message = document["error"]["message"].as_str().unwrap();
    assert!(message.contains("the key lacks developer-product:read"), "{message}");
    assert!(message.contains("rerun with --no-scope-check"), "{message}");
    assert_eq!(
        (document["error"]["requiredScopes"][0]["name"].as_str(), document["error"]["scopeMetadata"].as_str()),
        (Some("developer-product:read"), Some("declared")),
        "the error names the scopes it checked: {document}"
    );
    products.assert_calls(0);
    badges.assert_calls(0);
}

#[test]
fn no_scope_check_skips_the_introspection() {
    let dir = project();
    let server = MockServer::start();
    let (products, _) = lists(&server, vec![], vec![]);
    let introspection = introspection(&server, json!([]), true);
    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run", "--no-scope-check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    introspection.assert_calls(0);
    products.assert_calls(1);
}

#[test]
fn an_introspection_that_does_not_answer_within_ten_seconds_is_a_note() {
    let dir = project();
    let server = MockServer::start();
    let (products, _) = lists(&server, vec![], vec![]);
    server.mock(|when, then| {
        when.method(Method::POST).path("/api-keys/v1/introspect");
        then.status(200).delay(std::time::Duration::from_secs(60)).json_body(json!({}));
    });
    let started = std::time::Instant::now();
    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run", "--json"]);
    let took = started.elapsed();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(took < std::time::Duration::from_secs(30), "took {took:?}");
    let stderr = text(&out.stderr);
    assert!(stderr.contains("Could not check the key's scopes within 10 s: request deadline passed"), "{stderr}");
    products.assert_calls(1);
}

#[test]
fn a_key_without_the_write_scopes_stops_provision_after_listing_and_before_any_write() {
    let dir = project();
    let server = MockServer::start();
    let (products, _) = lists(&server, vec![], vec![]);
    introspection(&server, json!([{"name": "developer-product", "operations": ["read"]}]), true);
    let create = server.mock(|when, then| {
        when.method(Method::POST);
        then.status(200).json_body(product(77));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    let document = document(&out);
    assert_eq!(
        document["error"]["missingScopes"],
        json!(["developer-product:write", "legacy-universe.badge:manage-and-spend-robux"]),
        "{document}"
    );
    assert_eq!(document["applied"], json!([]));
    products.assert_calls(1);
    create.assert_calls(0);
    assert!(!dir.path().join(".crucible/journal/staging.json").exists(), "no intent was written");

    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0), "a dry run writes nothing, so it needs only the read scopes");
}

#[test]
fn a_disabled_key_stops_provision_and_an_unanswered_introspection_does_not() {
    let dir = project();
    let server = MockServer::start();
    let (products, _) = lists(&server, vec![], vec![]);
    let mut disabled = introspection(&server, json!([]), false);
    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    assert_eq!(document(&out)["error"]["message"], "the key is disabled or expired");
    products.assert_calls(0);

    disabled.delete();
    server.mock(|when, then| {
        when.method(Method::POST).path("/api-keys/v1/introspect");
        then.status(403);
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("Could not check the key's scopes"), "{}", text(&out.stderr));
    products.assert_calls(1);
}

fn settings_lock(dir: &Path, table: &str) {
    let manifest = crucible_project::Manifest::parse(&format!("{CONFIG}\n[experience]\n{table}")).unwrap();
    let fingerprint = crucible_iac::model::Experience::new(&manifest).unwrap().settings().unwrap().fingerprint();
    std::fs::write(dir.join("crucible.toml"), format!("{CONFIG}\n[experience]\n{table}")).unwrap();
    let lock = format!(
        "version = 1\n\n[env.staging]\nuniverse = 1\n\n[env.staging.experience]\nfingerprint = {fingerprint:?}\n\n\
         [env.staging.products.coin-pack]\nid = 77\nfingerprint = \"x\"\nstatus = \"managed\"\n"
    );
    std::fs::write(dir.join("crucible.lock"), lock).unwrap();
}

#[test]
fn provision_repairs_a_setting_changed_on_roblox_that_the_lock_still_records() {
    let dir = project();
    settings_lock(dir.path(), "private_server_price = 100\nvoice_chat = false\n");
    let server = MockServer::start();
    lists(&server, vec![product(77)], vec![badge(88)]);
    server.mock(|when, then| {
        when.method(Method::GET).path("/cloud/v2/universes/1").header("x-api-key", "test-key");
        then.status(200)
            .json_body(json!({"path": "universes/1", "privateServerPriceRobux": 100, "voiceChatEnabled": true}));
    });
    let patch = server.mock(|when, then| {
        when.method(Method::PATCH)
            .path("/cloud/v2/universes/1")
            .query_param("updateMask", "voiceChatEnabled")
            .json_body(json!({"voiceChatEnabled": false}));
        then.status(200).json_body(json!({"path": "universes/1"}));
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains(
            "details, private_server_price = 100 (Roblox holds 100), voice_chat = false (Roblox holds true)\n"
        ),
        "{stdout}"
    );
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    patch.assert_calls(1);
    let document = document(&out);
    assert_eq!(
        document["plan"]["steps"][0]["settings"],
        json!([{"key": "private_server_price", "value": 100, "roblox": 100},
               {"key": "voice_chat", "value": false, "roblox": true}])
    );
}

#[test]
fn a_failed_read_of_the_settings_is_a_note_and_provision_compares_them_with_the_lock() {
    let dir = project();
    settings_lock(dir.path(), "voice_chat = false\n");
    let server = MockServer::start();
    lists(&server, vec![product(77)], vec![badge(88)]);
    let read = server.mock(|when, then| {
        when.method(Method::GET).path("/cloud/v2/universes/1");
        then.status(403).json_body(json!({"message": "denied"}));
    });
    let patch = server.mock(|when, then| {
        when.method(Method::PATCH).path("/cloud/v2/universes/1");
        then.status(200);
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    read.assert_calls(1);
    patch.assert_calls(0);
    assert!(text(&out.stderr).contains("Could not read the experience settings Roblox holds"), "{}", text(&out.stderr));
    let document = document(&out);
    assert_eq!(document["plan"]["steps"][0]["action"], "no-op", "{document}");
    assert_eq!(document["plan"]["steps"][0]["settings"], json!([{"key": "voice_chat", "value": false}]));
}

#[test]
fn a_price_change_needs_the_product_write_scope_before_any_update() {
    let dir = project();
    let lock = "version = 1\n\n[env.staging]\nuniverse = 1\n\n[env.staging.products.coin-pack]\nid = 77\n\
                fingerprint = \"x\"\nstatus = \"managed\"\n\n[env.staging.badges.first-delivery]\nid = 88\n\
                fingerprint = \"x\"\nstatus = \"managed\"\n";
    std::fs::write(dir.path().join("crucible.lock"), lock).unwrap();
    let server = MockServer::start();
    let mut cheap = product(77);
    cheap["priceInformation"]["defaultPriceInRobux"] = json!(10);
    lists(&server, vec![cheap], vec![badge(88)]);
    introspection(&server, json!([{"name": "developer-product", "operations": ["read"]}]), true);
    let update = server.mock(|when, then| {
        when.method(Method::PATCH);
        then.status(200);
    });
    let out = crucible(dir.path(), &server, &["provision", "staging", "--json"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    let document = document(&out);
    assert_eq!(document["error"]["missingScopes"], json!(["developer-product:write"]), "{document}");
    update.assert_calls(0);
    assert_eq!(std::fs::read_to_string(dir.path().join("crucible.lock")).unwrap(), lock, "nothing was recorded");
}
