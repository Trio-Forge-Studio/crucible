#[path = "support/game.rs"]
mod game;
#[path = "support/sequence.rs"]
mod sequence;
#[allow(dead_code)]
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
        fs::write(root.join("tsconfig.json"), game::ts7_config().to_string()).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/main.ts"), "export const value = 1;\n").unwrap();
        fs::write(
            root.join("default.project.json"),
            r#"{"name":"fixture","tree":{"$className":"DataModel","ReplicatedStorage":{"$className":"ReplicatedStorage","Out":{"$path":"out"},"rbxts_include":{"$path":"include"}}}}"#,
        )
        .unwrap();
        if let Some(text) = manifest {
            fs::write(root.join("crucible.toml"), text).unwrap();
        }
        Self(dir)
    }
    fn command(&self, origin: &str) -> Command {
        let mut command = self.binary(origin);
        command.arg("deploy");
        command
    }
    fn binary(&self, origin: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crucible"));
        command.current_dir(self.0.path()).env("CRUCIBLE_OC_ORIGIN", origin).env("CRUCIBLE_OC_API_KEY", KEY);
        command
    }
    fn deploy(&self, origin: &str, args: &[&str]) -> Output {
        self.command(origin).args(args).output().unwrap()
    }
}
const TWO: &str = "[places.arena]\nproject='default.project.json'\n[places.start]\nproject='default.project.json'\n[env.production]\nuniverse=10\nversion_type='Published'\nplaces={arena=20,start=30}\n";
fn introspection(scopes: serde_json::Value) -> Response {
    Response::json(json!({"name":"test","enabled":true,"expired":false,"scopes":scopes}))
}
/// The introspection reply of a key that may publish places, which deploy reads before it builds.
fn granted() -> Response {
    introspection(json!([{"name":"universe-places","operations":["write"]}]))
}

#[test]
fn publishes_built_bytes_with_receipts_and_stderr_progress() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![
        granted(),
        Response::json(json!({"versionNumber":7})),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game.deploy(&server.origin, &["production"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(
        document(&out),
        json!({"environment":"production","dryRun":false,"published":[
        {"name":"arena","universeId":"10","placeId":"20","versionNumber":7,"versionType":"published"},
        {"name":"start","universeId":"10","placeId":"30","versionNumber":8,"versionType":"published"}
    ],"failed":null,"notAttempted":[]})
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].head.starts_with("POST /api-keys/v1/introspect "), "{}", requests[0].head);
    for (request, name, id) in [(&requests[1], "arena", "20"), (&requests[2], "start", "30")] {
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
    assert!(stderr.contains("modules to out") && stderr.contains("Built project"));
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
        let server = Server::new(vec![granted(), Response::json(json!({"versionNumber":7})), reply]);
        let out = game.deploy(&server.origin, &["production"]);
        assert_eq!(code(&out), expected, "{}", document(&out));
        let d = document(&out);
        assert_eq!(d["error"]["kind"], kind);
        assert_eq!(d["error"]["deploy"]["published"].as_array().unwrap().len(), 1);
        assert_eq!(d["error"]["deploy"]["published"][0]["versionNumber"], 7);
        assert_eq!(d["error"]["deploy"]["next"], json!(["crucible", "deploy", "production", "--place=start"]));
        if expected == 5 {
            assert!(d["error"]["hint"].as_str().unwrap().contains("versions before re-uploading"));
        }
        assert_eq!(d["error"]["deploy"]["failed"], "start");
        assert_eq!(d["error"]["deploy"]["notAttempted"], json!([]));
        assert_eq!(server.requests().len(), 3);
    }
}

#[test]
fn first_failure_leaves_later_places_unattempted() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![granted(), Response::Body(400, "bad request".into())]);
    let out = game.deploy(&server.origin, &["production"]);
    assert_eq!(code(&out), 4);
    let d = document(&out);
    assert_eq!(d["error"]["deploy"]["published"], json!([]));
    assert_eq!(d["error"]["deploy"]["failed"], "arena");
    assert_eq!(d["error"]["deploy"]["notAttempted"], json!(["start"]));
    assert_eq!(
        d["error"]["deploy"]["next"],
        json!(["crucible", "deploy", "production", "--place=arena", "--place=start"])
    );
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn dry_run_builds_checks_scopes_and_never_uploads() {
    for (scopes, expected) in [(json!([{"name":"universe-places","operations":["write"]}]), 0), (json!([]), 3)] {
        let game = Game::new(Some(TWO));
        let server = Server::new(vec![introspection(scopes)]);
        let out = game.deploy(&server.origin, &["production", "--dry-run"]);
        assert_eq!(code(&out), expected, "{}", document(&out));
        assert_eq!(
            game.0.path().join("build/arena.rbxl").exists(),
            expected == 0,
            "the scope check precedes the build"
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].head.starts_with("POST /api-keys/v1/introspect "));
        if expected == 0 {
            assert_eq!(document(&out)["auth"]["keySource"], "CRUCIBLE_OC_API_KEY");
            assert_eq!(document(&out)["notAttempted"], json!(["arena", "start"]));
            assert_eq!(document(&out)["published"], json!([]));
        } else {
            assert_eq!(document(&out)["error"]["missingScopes"], json!(["universe-places:write"]));
        }
    }
}

#[test]
fn filter_deduplicates_and_single_place_defaults_to_saved() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![granted(), Response::json(json!({"versionNumber":1}))]);
    let out = game.deploy(&server.origin, &["production", "--place", "start", "--place", "start"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert!(!game.0.path().join("build/arena.rbxl").exists());
    assert_eq!(server.requests().len(), 2);
    let game = Game::new(Some("[env.production]\nuniverse=10\nplace=30\n"));
    let server = Server::new(vec![granted(), Response::json(json!({"versionNumber":2}))]);
    let out = game.deploy(&server.origin, &["production"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["published"][0]["versionType"], "saved");
}

#[test]
fn invalid_targets_and_arguments_are_json_usage_errors_without_requests() {
    let server = Server::new(vec![]);
    for (manifest, args, message) in [
        (None, vec!["production"], "deploy needs crucible.toml"),
        (Some(TWO), vec!["unknown"], "Environments: production"),
        (Some(""), vec!["production"], "No environments are configured"),
        (Some(TWO), vec!["production", "--place", "unknown"], "Places: arena, start"),
        (Some("unexpected=1"), vec!["production"], "unknown field"),
        (Some(&TWO.replace("universe=10", "universe=10\nprofile='publish'")), vec!["production"], "unknown field"),
        (Some(TWO), vec!["production", "--profile", "publish"], "unexpected"),
        (Some(TWO), vec![], "required"),
        (Some(TWO), vec!["production", "--invalid"], "unexpected"),
    ] {
        let game = Game::new(manifest);
        let out = game.deploy(&server.origin, &args);
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
    let server = Server::new(vec![granted()]);
    fs::write(game.0.path().join("src/main.ts"), "export const value: number = \"one\";\n").unwrap();
    let out = game.command(&server.origin).arg("production").output().unwrap();
    assert_eq!(code(&out), 7);
    assert_eq!(document(&out)["error"]["deploy"]["notAttempted"], json!(["arena", "start"]));
    assert_eq!(server.requests().len(), 1, "only the introspection");
}

#[test]
fn a_disabled_key_fails_as_whoami_does() {
    let reply = json!({"name":"test","enabled":false,"expired":false,"scopes":[]});
    let server = Server::new(vec![Response::json(reply.clone()), Response::json(reply)]);
    let oc = support::whoami(&server.origin).args(["--for", "universes.create-place-version"]).output().unwrap();
    let game = Game::new(Some(TWO));
    let deploy = game.deploy(&server.origin, &["production", "--dry-run"]);
    assert_eq!(code(&oc), 3);
    assert_eq!(code(&deploy), 3);
    let mut failure = document(&deploy);
    failure["error"].as_object_mut().unwrap().remove("deploy");
    assert_eq!(failure, document(&oc));
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn the_environment_key_wins_over_dot_env_and_dot_env_stands_alone() {
    let game = Game::new(Some(TWO));
    fs::write(game.0.path().join(".env"), "# the game's key\nexport CRUCIBLE_OC_API_KEY=\"dot-env-key\"\nOTHER=1\n")
        .unwrap();
    for (direct, expected) in [(true, KEY), (false, "dot-env-key")] {
        let server = Server::new(vec![granted(), Response::json(json!({"versionNumber":1}))]);
        let mut cmd = game.command(&server.origin);
        cmd.args(["production", "--place", "start"]);
        if !direct {
            cmd.env_remove("CRUCIBLE_OC_API_KEY");
        }
        let out = cmd.output().unwrap();
        assert_eq!(code(&out), 0, "{}", document(&out));
        assert_eq!(server.requests()[0].body, format!(r#"{{"apiKey":"{expected}"}}"#).as_bytes());
        assert!(server.requests()[1].head.contains(&format!("x-api-key: {expected}")));
    }
}

#[test]
fn a_dry_run_reports_the_dot_env_key_source() {
    let game = Game::new(Some(TWO));
    fs::write(game.0.path().join(".env"), "CRUCIBLE_OC_API_KEY=dot-env-key\n").unwrap();
    let server = Server::new(vec![granted()]);
    let out = game
        .command(&server.origin)
        .args(["production", "--dry-run"])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["auth"]["keySource"], ".env");
    assert_eq!(server.requests()[0].body, br#"{"apiKey":"dot-env-key"}"#);
}

#[test]
fn a_missing_key_precedes_the_build_and_leaves_every_place_unattempted() {
    let game = Game::new(Some(TWO));
    fs::write(game.0.path().join(".env"), "OTHER=1\n").unwrap();
    let server = Server::new(vec![]);
    let out = game.command(&server.origin).arg("production").env_remove("CRUCIBLE_OC_API_KEY").output().unwrap();
    assert_eq!(code(&out), 3);
    assert_eq!(
        document(&out),
        json!({"error":{"kind":"auth","message":"set CRUCIBLE_OC_API_KEY in the environment or in .env","status":null,
        "requiredScopes":[{"name":"universe-places:write","target":"v1"}],"scopeMetadata":"declared","keySource":null,
        "deploy":{"environment":"production","dryRun":false,"published":[],"failed":null,"notAttempted":["arena","start"]}}})
    );
    assert!(!game.0.path().join("out").exists());
    assert!(server.requests().is_empty());
}

#[test]
fn upload_error_redacts_the_key_and_signed_urls() {
    let game = Game::new(Some(TWO));
    let server =
        Server::new(vec![granted(), Response::Body(400, format!("{KEY} https://cdn.test/file?signature=secret"))]);
    let out = game.deploy(&server.origin, &["production", "--verbose"]);
    assert_eq!(code(&out), 4);
    let combined = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!combined.contains(KEY));
    assert!(!combined.contains("signature=secret"));
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn stdout_failure_preserves_upload_uncertainty() {
    for dry_run in [false, true] {
        let game = Game::new(Some(TWO));
        let mut replies = vec![granted()];
        if !dry_run {
            replies.push(Response::json(json!({"versionNumber":1})));
        }
        let server = Server::new(replies);
        let mut command = game.command(&server.origin);
        command.args(["production", "--place", "start"]);
        if dry_run {
            command.arg("--dry-run");
        }
        let mut child =
            command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
        drop(child.stdout.take());
        assert_eq!(child.wait().unwrap().code(), Some(if dry_run { 7 } else { 5 }));
        assert_eq!(server.requests().len(), if dry_run { 1 } else { 2 });
    }
}

#[test]
fn recovery_replays_verbose_and_reads_dot_env_again_without_reuploading_successes() {
    let game = Game::new(Some(TWO));
    fs::write(game.0.path().join(".env"), "CRUCIBLE_OC_API_KEY=dot-env-key\n").unwrap();
    let server = Server::new(vec![
        granted(),
        Response::json(json!({"versionNumber":7})),
        Response::Body(400, "bad request".into()),
        granted(),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game
        .command(&server.origin)
        .args(["production", "--verbose"])
        .env_remove("CRUCIBLE_OC_API_KEY")
        .output()
        .unwrap();
    assert_eq!(code(&out), 4, "{}", document(&out));
    let d = document(&out);
    let next: Vec<String> = serde_json::from_value(d["error"]["deploy"]["next"].clone()).unwrap();
    assert_eq!(next, ["crucible", "deploy", "production", "--verbose", "--place=start"]);
    let replay = game.binary(&server.origin).args(&next[1..]).env_remove("CRUCIBLE_OC_API_KEY").output().unwrap();
    assert_eq!(code(&replay), 0, "{}", document(&replay));
    assert_eq!(document(&replay)["published"][0]["name"], "start");
    assert!(String::from_utf8_lossy(&replay.stderr).contains("POST"));
    let requests: Vec<_> =
        server.requests().into_iter().filter(|request| request.head.contains("/versions?")).collect();
    assert_eq!((server.requests().len(), requests.len()), (5, 3));
    for (request, id) in requests.iter().zip(["20", "30", "30"]) {
        assert!(request.head.contains(&format!("/places/{id}/versions?")), "{}", request.head);
        assert!(request.head.contains("x-api-key: dot-env-key"), "{}", request.head);
    }
}

#[test]
fn recovery_replays_hyphenated_place_and_every_unattempted_place() {
    let game = Game::new(Some(&TWO.replace("arena", "-arena")));
    let server = Server::new(vec![
        granted(),
        Response::Body(400, "bad request".into()),
        granted(),
        Response::json(json!({"versionNumber":7})),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game.deploy(&server.origin, &["production"]);
    assert_eq!(code(&out), 4, "{}", document(&out));
    let d = document(&out);
    let next: Vec<String> = serde_json::from_value(d["error"]["deploy"]["next"].clone()).unwrap();
    assert_eq!(next[0], "crucible");
    let replay = game.binary(&server.origin).args(&next[1..]).output().unwrap();
    assert_eq!(code(&replay), 0, "{}", document(&replay));
    assert_eq!(document(&replay)["published"][0]["name"], "-arena");
    assert_eq!(document(&replay)["published"][1]["name"], "start");
    let requests: Vec<_> =
        server.requests().into_iter().filter(|request| request.head.contains("/versions?")).collect();
    assert_eq!((server.requests().len(), requests.len()), (5, 3));
    for (request, id) in requests.iter().zip(["20", "20", "30"]) {
        assert!(request.head.contains(&format!("/places/{id}/versions?")), "{}", request.head);
        assert!(request.head.contains(&format!("x-api-key: {KEY}")), "{}", request.head);
    }
}

#[test]
fn unreadable_manifest_is_a_json_usage_error() {
    let game = Game::new(None);
    fs::create_dir(game.0.path().join("crucible.toml")).unwrap();
    let out = game.deploy("http://127.0.0.1:1", &["production"]);
    assert_eq!(code(&out), 2);
    let d = document(&out);
    assert_eq!(d["error"]["kind"], "usage");
    assert!(d["error"]["message"].as_str().unwrap().contains("cannot read crucible.toml"));
    assert!(!game.0.path().join("build").exists());
}

const DECLARED: &str =
    "[products.coin-pack]\nname='Coin pack'\nprice=49\n[badges.first-delivery]\nname='First delivery'\n";

/// The lock `crucible provision production` writes for `TWO` with `DECLARED`, so the offline plan has nothing to do.
fn provisioned(game: &Game, settings: bool) {
    use crucible_iac::lock::{Entry, Lock, Status};
    let text = format!("{TWO}{DECLARED}{}", if settings { "[experience]\nvoice_chat=true\n" } else { "" });
    let manifest = crucible_project::Manifest::parse(&text).unwrap();
    let experience = crucible_iac::model::Experience::new(&manifest).unwrap();
    let env = &manifest.envs["production"];
    let mut lock = Lock::empty();
    let tables = lock.env_mut("production", env.universe);
    if let Some(settings) = experience.settings() {
        tables.experience = Some(crucible_iac::lock::Applied { fingerprint: settings.fingerprint() });
    }
    for (at, resource) in experience.resolve(env).into_iter().enumerate() {
        let fingerprint = resource.fields.as_ref().unwrap().fingerprint();
        let id = std::num::NonZeroU64::new(77 + at as u64).unwrap();
        tables
            .entries_mut(resource.kind)
            .insert(resource.key, Entry { id, fingerprint, icon: None, status: Status::Managed });
    }
    fs::write(game.0.path().join("crucible.toml"), text).unwrap();
    fs::write(game.0.path().join("crucible.lock"), lock.render()).unwrap();
}

#[test]
fn a_provisioned_environment_deploys() {
    let game = Game::new(Some(TWO));
    provisioned(&game, true);
    let server = Server::new(vec![
        granted(),
        Response::json(json!({"versionNumber":7})),
        Response::json(json!({"versionNumber":8})),
    ]);
    let out = game.deploy(&server.origin, &["production"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert_eq!(document(&out)["published"].as_array().unwrap().len(), 2);
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn deploy_refuses_what_provision_has_not_recorded_before_any_request() {
    let unprovisioned = format!("{TWO}{DECLARED}");
    let settings = format!("{TWO}{DECLARED}[experience]\nvoice_chat=false\n");
    let intent = r#"{"version":1,"universe":10,"intents":[{"kind":"badge","key":"first-delivery","name":"First delivery","before":[]}]}"#;
    let cases: [(&str, Option<&str>, &str, &[&str]); 4] = [
        (&unprovisioned, None, "create product coin-pack, create badge first-delivery", &["create", "create"]),
        (&settings, Some("lock"), "update experience settings", &["update"]),
        (&unprovisioned, Some("lock-without-badge"), "adopt-check badge first-delivery", &["adopt-check"]),
        (&unprovisioned, Some("lock-without-badge-no-intent"), "create badge first-delivery", &["create"]),
    ];
    for (manifest, lock, listed, actions) in cases {
        let game = Game::new(Some(TWO));
        provisioned(&game, false);
        if lock.is_none() {
            fs::remove_file(game.0.path().join("crucible.lock")).unwrap();
        }
        if lock.is_some_and(|lock| lock.starts_with("lock-without-badge")) {
            let text = fs::read_to_string(game.0.path().join("crucible.lock")).unwrap();
            let at = text.find("[env.production.badges.first-delivery]").unwrap();
            let end = text[at..].find("\n\n").map_or(text.len(), |end| at + end + 1);
            fs::write(game.0.path().join("crucible.lock"), format!("{}{}", &text[..at], &text[end..])).unwrap();
            if lock == Some("lock-without-badge") {
                fs::create_dir_all(game.0.path().join(".crucible/journal")).unwrap();
                fs::write(game.0.path().join(".crucible/journal/production.json"), intent).unwrap();
            }
        }
        fs::write(game.0.path().join("crucible.toml"), manifest).unwrap();
        let server = Server::new(vec![]);
        let out = game.deploy(&server.origin, &["production"]);
        let d = document(&out);
        assert_eq!(code(&out), 2, "{d}");
        assert_eq!(d["error"]["kind"], "usage");
        let message = d["error"]["message"].as_str().unwrap();
        assert!(message.contains(listed), "{message}");
        assert!(
            message.contains("Run `crucible provision production`, commit crucible.lock, then deploy"),
            "{message}"
        );
        let steps: Vec<&str> =
            d["error"]["steps"].as_array().unwrap().iter().map(|s| s["action"].as_str().unwrap()).collect();
        assert_eq!(steps, actions, "{d}");
        assert!(server.requests().is_empty(), "{listed}: deploy refuses before it reads the key");
        assert!(!game.0.path().join("build").exists());
    }
}

#[test]
fn deploy_refuses_files_that_break_the_provisioning_rules() {
    for (lock, said) in [
        ("version = 1\n\n[env.production]\nuniverse = 11\n", "universe 11"),
        ("version = 1\n\n[env.staging]\nuniverse = 12\n", "staging"),
        ("version = 9\n", "format 9"),
    ] {
        let game = Game::new(Some(TWO));
        fs::write(game.0.path().join("crucible.lock"), lock).unwrap();
        let server = Server::new(vec![]);
        let out = game.deploy(&server.origin, &["production"]);
        let d = document(&out);
        assert_eq!((code(&out), d["error"]["kind"].as_str()), (2, Some("usage")), "{d}");
        assert!(d.to_string().contains(said), "{said}: {d}");
        assert!(server.requests().is_empty());
    }
}

#[test]
fn an_unanswered_introspection_does_not_stop_deploy() {
    let game = Game::new(Some(TWO));
    let server = Server::new(vec![Response::Body(403, "denied".into()), Response::json(json!({"versionNumber":3}))]);
    let out = game.deploy(&server.origin, &["production", "--place", "start"]);
    assert_eq!(code(&out), 0, "{}", document(&out));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Could not check the key's scopes"));
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn crucible_publish_names_crucible_deploy() {
    let game = Game::new(Some(TWO));
    let out = game.binary("http://127.0.0.1:1").args(["publish", "production"]).output().unwrap();
    assert_eq!(code(&out), 2);
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 1);
    let d = document(&out);
    assert_eq!(d["error"]["kind"], "usage");
    assert!(d["error"]["message"].as_str().unwrap().contains("crucible deploy <env>"), "{d}");
}
