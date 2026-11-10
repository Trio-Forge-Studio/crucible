mod support;
use httpmock::{Method::POST, MockServer};
use serde_json::json;
use std::path::Path;
use support::*;

fn introspection<'a>(server: &'a MockServer, key: &str) -> httpmock::Mock<'a> {
    let key = key.to_owned();
    server.mock(move |w, t| {
        w.method(POST).path("/api-keys/v1/introspect").json_body(json!({"apiKey":key}));
        t.json_body(
            json!({"name":"game-key","enabled":true,"expired":false,"scopes":[{"name":"asset","operations":["read"]}]}),
        );
    })
}
fn project(env: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), env).unwrap();
    dir
}
fn without_key(origin: &str, dir: &Path) -> std::process::Command {
    let mut cmd = whoami(origin);
    cmd.current_dir(dir).env_remove("CRUCIBLE_OC_API_KEY");
    cmd
}

#[test]
fn whoami_prints_the_key_name_state_and_scopes() {
    let server = MockServer::start();
    let m = introspection(&server, KEY);
    let out = whoami(&server.base_url()).output().unwrap();
    assert_eq!(code(&out), 0);
    assert_eq!(
        document(&out),
        json!({"keySource":"CRUCIBLE_OC_API_KEY","name":"game-key","enabled":true,"expired":false,"scopes":["asset:read"]})
    );
    m.assert_calls(1);
}

#[test]
fn the_key_can_come_from_dot_env_alone() {
    let server = MockServer::start();
    let m = introspection(&server, "dot env key");
    let dir = project("# keys\nOTHER=1\nexport CRUCIBLE_OC_API_KEY=\"dot env key\" # the game's key\n");
    let out = without_key(&server.base_url(), dir.path()).output().unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["keySource"], ".env");
    m.assert_calls(1);
}

#[test]
fn dot_env_feeds_oc_calls_too() {
    let server = MockServer::start();
    let m = server.mock(|w, t| {
        w.path("/assets/v1/assets/42").header("x-api-key", "dot-env-key");
        t.json_body(json!({"path":"assets/42"}));
    });
    let dir = project("CRUCIBLE_OC_API_KEY=dot-env-key\n");
    let out = command(&server.base_url())
        .current_dir(dir.path())
        .env_remove("CRUCIBLE_OC_API_KEY")
        .args(["call", "assets.get", "assetId=42"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    m.assert_calls(1);
}

#[test]
fn a_dot_env_saved_with_a_utf8_bom_still_gives_its_key() {
    let server = MockServer::start();
    let m = introspection(&server, "bom key");
    for text in
        ["\u{feff}CRUCIBLE_OC_API_KEY=\"bom key\"\r\n", "\u{feff}# keys\r\nexport CRUCIBLE_OC_API_KEY='bom key'\r\n"]
    {
        let dir = project(text);
        let out = without_key(&server.base_url(), dir.path()).output().unwrap();
        assert_eq!(code(&out), 0, "{}", document(&out));
        assert_eq!(document(&out)["keySource"], ".env");
    }
    m.assert_calls(2);
    let twice = project("\u{feff}\u{feff}CRUCIBLE_OC_API_KEY=\"bom key\"\n");
    let out = without_key(&server.base_url(), twice.path()).output().unwrap();
    assert_eq!(code(&out), 2, "only one leading BOM is an encoding mark");
    assert_eq!(document(&out)["error"]["message"], "cannot parse .env");
    let environment = introspection(&server, KEY);
    let dir = project("\u{feff}CRUCIBLE_OC_API_KEY=dot-env-key\n");
    let out = whoami(&server.base_url()).current_dir(dir.path()).output().unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["keySource"], "CRUCIBLE_OC_API_KEY", "the environment still wins");
    m.assert_calls(2);
    environment.assert_calls(1);
}

#[test]
fn the_process_variable_wins_over_a_different_dot_env_key() {
    let server = MockServer::start();
    let m = introspection(&server, KEY);
    let dir = project("CRUCIBLE_OC_API_KEY=dot-env-key\n");
    let out = whoami(&server.base_url()).current_dir(dir.path()).output().unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["keySource"], "CRUCIBLE_OC_API_KEY");
    m.assert_calls(1);
}

#[test]
fn no_key_anywhere_is_an_auth_failure_that_names_both_places() {
    let server = MockServer::start();
    let m = server.mock(|_, t| {
        t.json_body(json!({}));
    });
    let with_other_keys = project("OTHER=1\n");
    let empty_dir = tempfile::tempdir().unwrap();
    for dir in [empty_dir.path(), with_other_keys.path()] {
        let out = without_key(&server.base_url(), dir).output().unwrap();
        assert_eq!(code(&out), 3);
        let error = &document(&out)["error"];
        assert_eq!(error["kind"], "auth");
        assert_eq!(error["message"], "set CRUCIBLE_OC_API_KEY in the environment or in .env");
        assert!(error["keySource"].is_null());
    }
    m.assert_calls(0);
}

#[test]
fn an_empty_key_is_an_auth_failure_that_names_its_source() {
    let server = MockServer::start();
    let m = server.mock(|_, t| {
        t.json_body(json!({}));
    });
    for (text, place) in [("CRUCIBLE_OC_API_KEY=\n", ".env"), ("CRUCIBLE_OC_API_KEY=\"  \"\n", ".env")] {
        let dir = project(text);
        let out = without_key(&server.base_url(), dir.path()).output().unwrap();
        assert_eq!(code(&out), 3);
        let error = &document(&out)["error"];
        assert_eq!(error["kind"], "auth");
        assert_eq!(error["message"], format!("CRUCIBLE_OC_API_KEY in {place} is empty"));
    }
    let out = whoami(&server.base_url()).env("CRUCIBLE_OC_API_KEY", " ").output().unwrap();
    assert_eq!(code(&out), 3);
    assert_eq!(document(&out)["error"]["message"], "CRUCIBLE_OC_API_KEY in the environment is empty");
    m.assert_calls(0);
}

#[test]
fn an_unparsable_or_unreadable_dot_env_is_a_usage_error_that_prints_no_line() {
    let server = MockServer::start();
    let m = server.mock(|_, t| {
        t.json_body(json!({}));
    });
    let broken = project("CRUCIBLE_OC_API_KEY=secret-value\n\"unterminated\n");
    let unreadable = tempfile::tempdir().unwrap();
    std::fs::create_dir(unreadable.path().join(".env")).unwrap();
    for (dir, message) in [(broken.path(), "cannot parse .env"), (unreadable.path(), "cannot read .env")] {
        let out = without_key(&server.base_url(), dir).output().unwrap();
        assert_eq!(code(&out), 2);
        let error = &document(&out)["error"];
        assert_eq!(error["kind"], "usage");
        assert_eq!(error["message"], message);
        assert!(!String::from_utf8_lossy(&out.stdout).contains("secret-value"));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("secret-value"));
    }
    m.assert_calls(0);
}

#[test]
fn only_the_current_directory_is_searched_and_public_calls_never_read_it() {
    let server = MockServer::start();
    let parent = project("CRUCIBLE_OC_API_KEY=parent-key\n");
    let child = parent.path().join("game");
    std::fs::create_dir(&child).unwrap();
    let out = without_key(&server.base_url(), &child).output().unwrap();
    assert_eq!(code(&out), 3, "a parent directory's .env is not read");

    let public = server.mock(|w, t| {
        w.path("/universes/v1/places/1818/universe").header_missing("x-api-key");
        t.json_body(json!({"universeId":13058}));
    });
    let broken = project("\"unterminated\n");
    let out = command(&server.base_url())
        .current_dir(broken.path())
        .env_remove("CRUCIBLE_OC_API_KEY")
        .args(["call", "universes.get-place-universe", "placeId=1818"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    public.assert_calls(1);
}

#[test]
fn a_missing_scope_fails_with_exit_3_and_an_unknown_operation_with_exit_2() {
    let server = MockServer::start();
    let m = introspection(&server, KEY);
    let out = whoami(&server.base_url()).args(["--for", "assets.create"]).output().unwrap();
    assert_eq!(code(&out), 3);
    let error = &document(&out)["error"];
    assert_eq!(error["kind"], "auth");
    assert!(error["message"].as_str().unwrap().starts_with("the selected key lacks asset:write"));
    assert_eq!(error["keySource"], "CRUCIBLE_OC_API_KEY");
    assert_eq!(error["scopeMetadata"], "declared");
    m.assert_calls(1);

    let out = whoami(&server.base_url()).args(["--for", "assets.gett"]).output().unwrap();
    assert_eq!(code(&out), 2);
    assert!(
        document(&out)["error"]["suggestions"].as_array().is_some_and(|names| names.iter().any(|n| n == "assets.get"))
    );
    m.assert_calls(1);
}

#[test]
fn whoami_argument_errors_are_json() {
    let out = whoami("http://127.0.0.1:1").arg("--bogus").output().unwrap();
    assert_eq!(code(&out), 2);
    assert_eq!(document(&out)["error"]["kind"], "usage");
}
