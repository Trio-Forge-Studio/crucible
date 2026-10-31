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
        let mut command = self.binary(origin);
        command.arg("publish");
        command
    }
    fn binary(&self, origin: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crucible"));
        command
            .current_dir(self.0.path())
            .env("CRUCIBLE_OC_ORIGIN", origin)
            .env("CRUCIBLE_OC_API_KEY", KEY)
            .env("CRUCIBLE_CONFIG", self.0.path().join("config.toml"))
            .env_remove("CRUCIBLE_OC_PROFILE")
            .env_remove("FAIL_BUILD");
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
        assert_eq!(d["error"]["publish"]["published"].as_array().unwrap().len(), 1);
        assert_eq!(d["error"]["publish"]["published"][0]["versionNumber"], 7);
        assert_eq!(d["error"]["publish"]["next"], json!(["crucible", "publish", "production", "--place=start"]));
        if expected == 5 {
            assert!(d["error"]["hint"].as_str().unwrap().contains("versions before re-uploading"));
        }
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
    assert_eq!(
        d["error"]["publish"]["next"],
        json!(["crucible", "publish", "production", "--place=arena", "--place=start"])
    );
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
        (Some(""), vec!["production"], "No environments are configured"),
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

#[test]
fn credentials_follow_the_documented_precedence_without_fallthrough() {
    let game = Game::new(Some(&TWO.replace("universe=10", "universe=10\nprofile='manifest'")));
    fs::write(game.0.path().join("config.toml"), "[oc]\ndefault_profile='default'\n[oc.profiles.flag]\nkey_env='FLAG_KEY'\n[oc.profiles.variable]\nkey_env='VARIABLE_KEY'\n[oc.profiles.manifest]\nkey_env='MANIFEST_KEY'\n[oc.profiles.default]\nkey_env='DEFAULT_KEY'\n").unwrap();
    for (flag, variable, direct, expected) in [
        (Some("flag"), Some("variable"), true, "flag-key"),
        (None, Some("variable"), true, "variable-key"),
        (None, None, true, KEY),
        (None, None, false, "manifest-key"),
    ] {
        let server = Server::new(vec![Response::json(json!({"versionNumber":1}))]);
        let mut cmd = game.command(&server.origin);
        cmd.args(["production", "--place", "start"])
            .env("FLAG_KEY", "flag-key")
            .env("VARIABLE_KEY", "variable-key")
            .env("MANIFEST_KEY", "manifest-key")
            .env("DEFAULT_KEY", "default-key");
        if let Some(profile) = flag {
            cmd.args(["--profile", profile]);
        }
        if let Some(profile) = variable {
            cmd.env("CRUCIBLE_OC_PROFILE", profile);
        }
        if !direct {
            cmd.env_remove("CRUCIBLE_OC_API_KEY");
        }
        let out = cmd.output().unwrap();
        assert_eq!(code(&out), 0, "{}", document(&out));
        assert!(server.requests()[0].head.contains(&format!("x-api-key: {expected}")));
    }
    let server = Server::new(vec![]);
    let out = game.publish(&server.origin, &["production", "--profile", "missing"]);
    assert_eq!(code(&out), 3);
    assert!(server.requests().is_empty());
}

#[test]
fn upload_error_redacts_the_key_and_signed_urls() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![Response::Body(400, format!("{KEY} https://cdn.test/file?signature=secret"))]);
    let out = game.publish(&server.origin, &["production", "--verbose"]);
    assert_eq!(code(&out), 4);
    let combined = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!combined.contains(KEY));
    assert!(!combined.contains("signature=secret"));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn credential_failures_precede_build_and_leave_every_place_unattempted() {
    for manifest_profile in [false, true] {
        let manifest =
            if manifest_profile { TWO.replace("universe=10", "universe=10\nprofile='missing'") } else { TWO.into() };
        let game = Game::new(Some(&manifest));
        let server = Server::new(vec![]);
        let mut cmd = game.command(&server.origin);
        cmd.arg("production").env_remove("CRUCIBLE_OC_API_KEY");
        if !manifest_profile {
            cmd.args(["--profile", "missing"]);
        }
        let out = cmd.output().unwrap();
        assert_eq!(code(&out), 3);
        assert_eq!(
            document(&out),
            json!({"error":{"kind":"auth","message":"profile missing does not exist","status":null,
            "requiredScopes":[{"name":"universe-places:write","target":"v1"}],"scopeMetadata":"declared","keySource":null,"profiles":[],
            "publish":{"environment":"production","dryRun":false,"published":[],"failed":null,"notAttempted":["arena","start"]}}})
        );
        assert!(!game.0.path().join("out").exists());
        assert!(server.requests().is_empty());
    }
}

#[test]
fn user_default_profile_is_used_without_other_credentials() {
    let game = Game::new(Some(TWO));
    fs::write(
        game.0.path().join("config.toml"),
        "[oc]\ndefault_profile='default'\n[oc.profiles.default]\nkey_env='DEFAULT_KEY'",
    )
    .unwrap();
    let server = Server::new(vec![Response::json(json!({"versionNumber":1}))]);
    let out = game
        .command(&server.origin)
        .args(["production", "--place", "start"])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .env("DEFAULT_KEY", "default-key")
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert!(server.requests()[0].head.contains("x-api-key: default-key"));
}

#[test]
fn stdout_failure_preserves_upload_uncertainty() {
    for dry_run in [false, true] {
        let game = Game::new(Some(TWO));
        let reply = if dry_run {
            introspection(json!([{"name":"universe-places","operations":["write"]}]))
        } else {
            Response::json(json!({"versionNumber":1}))
        };
        let server = Server::new(vec![reply]);
        let mut command = game.command(&server.origin);
        command.args(["production", "--place", "start"]);
        if dry_run {
            command.arg("--dry-run");
        }
        let mut child =
            command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
        drop(child.stdout.take());
        assert_eq!(child.wait().unwrap().code(), Some(if dry_run { 7 } else { 5 }));
        assert_eq!(server.requests().len(), 1);
    }
}

#[test]
fn recovery_replays_explicit_profile_and_verbose_without_reuploading_successes() {
    let game = Game::new(Some(TWO));
    fs::write(game.0.path().join("config.toml"), "[oc.profiles.flag]\nkey_env='FLAG_KEY'\n").unwrap();
    let server = Server::new(vec![
        Response::json(json!({"versionNumber":7})),
        Response::Body(400, "bad request".into()),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game
        .command(&server.origin)
        .args(["production", "--profile", "flag", "--verbose"])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .env("FLAG_KEY", "flag-key")
        .output()
        .unwrap();
    assert_eq!(code(&out), 4, "{}", document(&out));
    let d = document(&out);
    let next: Vec<String> = serde_json::from_value(d["error"]["publish"]["next"].clone()).unwrap();
    assert_eq!(next[0], "crucible");
    let replay = game
        .binary(&server.origin)
        .args(&next[1..])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .env("FLAG_KEY", "flag-key")
        .output()
        .unwrap();
    println!(
        "next={next:?} replay exit={} stdout={} stderr={}",
        code(&replay),
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(code(&replay), 0, "{}", document(&replay));
    assert_eq!(document(&replay)["published"][0]["name"], "start");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("POST"));
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    for (request, id) in requests.iter().zip(["20", "30", "30"]) {
        assert!(request.head.contains(&format!("/places/{id}/versions?")), "{}", request.head);
        assert!(request.head.contains("x-api-key: flag-key"), "{}", request.head);
    }
}

#[test]
fn recovery_replays_profile_with_spaces_and_verbose_without_reuploading_successes() {
    let game = Game::new(Some(TWO));
    fs::write(game.0.path().join("config.toml"), "[oc.profiles.\"flag space\"]\nkey_env='FLAG_KEY'\n").unwrap();
    let server = Server::new(vec![
        Response::json(json!({"versionNumber":7})),
        Response::Body(400, "bad request".into()),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game
        .command(&server.origin)
        .args(["production", "--profile", "flag space", "--verbose"])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .env("FLAG_KEY", "flag-key")
        .output()
        .unwrap();
    assert_eq!(code(&out), 4, "{}", document(&out));
    let d = document(&out);
    let next: Vec<String> = serde_json::from_value(d["error"]["publish"]["next"].clone()).unwrap();
    assert_eq!(next[0], "crucible");
    let replay = game
        .binary(&server.origin)
        .args(&next[1..])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .env("FLAG_KEY", "flag-key")
        .output()
        .unwrap();
    println!(
        "next={next:?} replay exit={} stdout={} stderr={}",
        code(&replay),
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(code(&replay), 0, "{}", document(&replay));
    assert_eq!(document(&replay)["published"][0]["name"], "start");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("POST"));
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    for (request, id) in requests.iter().zip(["20", "30", "30"]) {
        assert!(request.head.contains(&format!("/places/{id}/versions?")), "{}", request.head);
        assert!(request.head.contains("x-api-key: flag-key"), "{}", request.head);
    }
}

#[test]
fn recovery_replays_hyphenated_place_and_every_unattempted_place() {
    let game = Game::new(Some(&TWO.replace("arena", "-arena")));
    let server = Server::new(vec![
        Response::Body(400, "bad request".into()),
        Response::json(json!({"versionNumber":7})),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game.publish(&server.origin, &["production"]);
    assert_eq!(code(&out), 4, "{}", document(&out));
    let d = document(&out);
    let next: Vec<String> = serde_json::from_value(d["error"]["publish"]["next"].clone()).unwrap();
    assert_eq!(next[0], "crucible");
    let replay = game.binary(&server.origin).args(&next[1..]).output().unwrap();
    println!("next={next:?} replay exit={} stdout={}", code(&replay), String::from_utf8_lossy(&replay.stdout));
    assert_eq!(code(&replay), 0, "{}", document(&replay));
    assert_eq!(document(&replay)["published"][0]["name"], "-arena");
    assert_eq!(document(&replay)["published"][1]["name"], "start");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    for (request, id) in requests.iter().zip(["20", "20", "30"]) {
        assert!(request.head.contains(&format!("/places/{id}/versions?")), "{}", request.head);
        assert!(request.head.contains(&format!("x-api-key: {KEY}")), "{}", request.head);
    }
}

#[test]
fn unreadable_manifest_is_a_json_usage_error() {
    let game = Game::new(None);
    fs::create_dir(game.0.path().join("crucible.toml")).unwrap();
    let out = game.publish("http://127.0.0.1:1", &["production"]);
    assert_eq!(code(&out), 2);
    let d = document(&out);
    assert_eq!(d["error"]["kind"], "usage");
    assert!(d["error"]["message"].as_str().unwrap().contains("cannot read crucible.toml"));
    assert!(!game.0.path().join("build").exists());
}
