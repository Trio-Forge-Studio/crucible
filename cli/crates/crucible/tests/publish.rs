#[path = "support/sequence.rs"]
mod sequence;
mod support;
use sequence::{Response, Server};
use serde_json::json;
use std::fs;
use std::process::{Command, Output};
use support::{KEY, code, document};

struct Game(tempfile::TempDir);
impl Game {
    fn new(manifest: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let compiler = root.join("node_modules/roblox-ts");
        fs::create_dir_all(&compiler).unwrap();
        fs::write(compiler.join("package.json"), r#"{"bin":{"rbxtsc":"cli.js"}}"#).unwrap();
        fs::write(
            compiler.join("cli.js"),
            r#"
const fs = require('node:fs');
console.log('compiler progress');
if (process.env.FAIL_BUILD) process.exit(1);
fs.mkdirSync('out', {recursive:true});
fs.writeFileSync('out/main.luau', 'return 1');
"#,
        )
        .unwrap();
        fs::write(
            root.join("default.project.json"),
            r#"{"name":"fixture","tree":{"$className":"DataModel","ReplicatedStorage":{"$path":"out"}}}"#,
        )
        .unwrap();
        if let Some(text) = manifest {
            fs::write(root.join("crucible.toml"), text).unwrap();
        }
        Self(dir)
    }
    fn command(&self, origin: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crucible"));
        command
            .current_dir(self.0.path())
            .env("CRUCIBLE_OC_ORIGIN", origin)
            .env("CRUCIBLE_OC_API_KEY", KEY)
            .env("CRUCIBLE_CONFIG", self.0.path().join("config.toml"))
            .env_remove("CRUCIBLE_OC_PROFILE")
            .env_remove("FAIL_BUILD")
            .arg("publish");
        command
    }
    fn publish(&self, origin: &str, args: &[&str]) -> Output {
        self.command(origin).args(args).output().unwrap()
    }
}
const TWO: &str = "[places.arena]\nproject='default.project.json'\n[places.start]\nproject='default.project.json'\n[env.production]\nuniverse=10\nversion_type='Published'\nplaces={arena=20,start=30}\n";
fn introspection(scopes: serde_json::Value) -> Response {
    Response::json(json!({"name":"test","enabled":true,"expired":false,"scopes":scopes}))
}

#[test]
fn publishes_built_bytes_with_receipts_and_stderr_progress() {
    let game = Game::new(Some(TWO));
    let server =
        Server::new(vec![Response::json(json!({"versionNumber":7})), Response::json(json!({"versionNumber":8}))]);
    let out = game.publish(&server.origin, &["production"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(
        document(&out),
        json!({"environment":"production","dryRun":false,"published":[
        {"name":"arena","universeId":"10","placeId":"20","versionNumber":7,"versionType":"published"},
        {"name":"start","universeId":"10","placeId":"30","versionNumber":8,"versionType":"published"}
    ],"failed":null,"notAttempted":[]})
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    for (request, name, id) in [(&requests[0], "arena", "20"), (&requests[1], "start", "30")] {
        assert!(
            request.head.contains(&format!("/universes/v1/10/places/{id}/versions?versionType=Published")),
            "{}",
            request.head
        );
        assert!(request.head.contains(KEY));
        assert_eq!(request.body, fs::read(game.0.path().join(format!("build/{name}.rbxl"))).unwrap());
        assert!(request.body.starts_with(b"<roblox!"));
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("compiler progress") && stderr.contains("Built project"));
}

#[test]
fn second_failure_preserves_first_receipt_and_its_own_classification() {
    for (reply, expected, kind) in [
        (Response::Body(403, "denied".into()), 3, "auth"),
        (Response::Body(400, "bad request".into()), 4, "rejected"),
        (Response::Reset, 5, "outcome_unknown"),
        (Response::Truncated(200), 5, "outcome_unknown"),
        (Response::json(json!({})), 5, "outcome_unknown"),
    ] {
        let game = Game::new(Some(TWO));
        let server = Server::new(vec![Response::json(json!({"versionNumber":7})), reply]);
        let out = game.publish(&server.origin, &["production"]);
        assert_eq!(code(&out), expected, "{}", document(&out));
        let d = document(&out);
        assert_eq!(d["error"]["kind"], kind);
        assert_eq!(d["error"]["publish"]["published"][0]["versionNumber"], 7);
        assert_eq!(d["error"]["publish"]["failed"], "start");
        assert_eq!(d["error"]["publish"]["notAttempted"], json!([]));
        assert_eq!(server.requests().len(), 2);
    }
}

#[test]
fn first_failure_leaves_later_places_unattempted() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![Response::Body(400, "bad request".into())]);
    let out = game.publish(&server.origin, &["production"]);
    assert_eq!(code(&out), 4);
    let d = document(&out);
    assert_eq!(d["error"]["publish"]["published"], json!([]));
    assert_eq!(d["error"]["publish"]["failed"], "arena");
    assert_eq!(d["error"]["publish"]["notAttempted"], json!(["start"]));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn dry_run_builds_checks_scopes_and_never_uploads() {
    for (scopes, expected) in [(json!([{"name":"universe-places","operations":["write"]}]), 0), (json!([]), 3)] {
        let game = Game::new(Some(TWO));
        let server = Server::new(vec![introspection(scopes)]);
        let out = game.publish(&server.origin, &["production", "--dry-run"]);
        assert_eq!(code(&out), expected, "{}", document(&out));
        assert!(game.0.path().join("build/arena.rbxl").exists());
        assert!(game.0.path().join("build/start.rbxl").exists());
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].head.starts_with("POST /api-keys/v1/introspect "));
        if expected == 0 {
            assert_eq!(document(&out)["notAttempted"], json!(["arena", "start"]));
            assert_eq!(document(&out)["published"], json!([]));
        }
    }
}

#[test]
fn filter_deduplicates_and_single_place_defaults_to_saved() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![Response::json(json!({"versionNumber":1}))]);
    let out = game.publish(&server.origin, &["production", "--place", "start", "--place", "start"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert!(!game.0.path().join("build/arena.rbxl").exists());
    assert_eq!(server.requests().len(), 1);
    let game = Game::new(Some("[env.production]\nuniverse=10\nplace=30\n"));
    let server = Server::new(vec![Response::json(json!({"versionNumber":2}))]);
    let out = game.publish(&server.origin, &["production"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["published"][0]["versionType"], "saved");
}

#[test]
fn invalid_targets_and_arguments_are_json_usage_errors_without_requests() {
    let server = Server::new(vec![]);
    for (manifest, args, message) in [
        (None, vec!["production"], "[env.production]"),
        (Some(TWO), vec!["unknown"], "Environments: production"),
        (Some(TWO), vec!["production", "--place", "unknown"], "Places: arena, start"),
        (Some("unexpected=1"), vec!["production"], "unknown field"),
        (Some(TWO), vec![], "required"),
        (Some(TWO), vec!["production", "--invalid"], "unexpected"),
    ] {
        let game = Game::new(manifest);
        let out = game.publish(&server.origin, &args);
        assert_eq!(code(&out), 2);
        assert_eq!(document(&out)["error"]["kind"], "usage");
        assert!(
            document(&out)["error"]["message"].as_str().unwrap().to_lowercase().contains(&message.to_lowercase()),
            "{}",
            document(&out)
        );
        assert!(!game.0.path().join("out").exists());
    }
    assert!(server.requests().is_empty());
}

#[test]
fn build_failure_is_unavailable_and_never_uploads() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![]);
    let out = game.command(&server.origin).arg("production").env("FAIL_BUILD", "1").output().unwrap();
    assert_eq!(code(&out), 7);
    assert_eq!(document(&out)["error"]["publish"]["notAttempted"], json!(["arena", "start"]));
    assert!(server.requests().is_empty());
}

#[test]
fn dry_run_reuses_oc_auth_check_validation() {
    let reply = json!({"name":"test","enabled":false,"expired":false,"scopes":[]});
    let server = Server::new(vec![Response::json(reply.clone()), Response::json(reply)]);
    let oc = support::command(&server.origin)
        .args(["auth", "check", "--for", "universes.create-place-version"])
        .output()
        .unwrap();
    let game = Game::new(Some(TWO));
    let publish = game.publish(&server.origin, &["production", "--dry-run"]);
    assert_eq!(code(&oc), 3);
    assert_eq!(code(&publish), 3);
    let mut failure = document(&publish);
    failure["error"].as_object_mut().unwrap().remove("publish");
    assert_eq!(failure, document(&oc));
    assert_eq!(server.requests().len(), 2);
}
