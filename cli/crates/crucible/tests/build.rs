use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE_RBXTSC: &str = r#"
const fs = require("node:fs");
fs.appendFileSync("rbxtsc.log", JSON.stringify({ args: process.argv.slice(2), stale: fs.existsSync("out/stale") }) + "\n");
console.log("\x1b[90mrbxtsc progress\x1b[39m");
if (process.env.FAKE_RBXTSC_EXIT) {
	console.error("\x1b[96msrc/main.server.ts\x1b[0m:\x1b[93m1\x1b[0m:\x1b[93m1\x1b[0m - \x1b[91merror\x1b[0m\x1b[90m TS1005: \x1b[0mfake failure");
	process.exit(Number(process.env.FAKE_RBXTSC_EXIT));
}
fs.mkdirSync("out", { recursive: true });
fs.writeFileSync("out/main.server.luau", "print('hi')");
"#;

const PROJECT: &str = r#"{"name":"fixture","tree":{"$className":"DataModel","ReplicatedStorage":{"$className":"ReplicatedStorage","Out":{"$path":"out"}}}}"#;

struct Game {
    dir: tempfile::TempDir,
}

impl Game {
    fn new(manifest: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rbxts = root.join("node_modules/roblox-ts");
        fs::create_dir_all(rbxts.join("out/CLI")).unwrap();
        fs::write(rbxts.join("package.json"), r#"{"name":"roblox-ts","bin":{"rbxtsc":"out/CLI/cli.js"}}"#).unwrap();
        fs::write(rbxts.join("out/CLI/cli.js"), FAKE_RBXTSC).unwrap();
        let crucible = root.join("node_modules/@trioforge/crucible");
        fs::create_dir_all(crucible.join("transformer")).unwrap();
        fs::write(crucible.join("package.json"), r#"{"name":"@trioforge/crucible"}"#).unwrap();
        fs::write(crucible.join("transformer/index.js"), "module.exports = 1;").unwrap();
        for name in ["default.project.json", "arena.project.json"] {
            fs::write(root.join(name), PROJECT).unwrap();
        }
        if let Some(manifest) = manifest {
            fs::write(root.join("crucible.toml"), manifest).unwrap();
        }
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn build(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_crucible"))
            .arg("build")
            .args(args)
            .current_dir(self.dir.path())
            .env_remove("FAKE_RBXTSC_EXIT")
            .output()
            .unwrap()
    }

    fn compiles(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.dir.path().join("rbxtsc.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

const TWO_PLACES: &str =
    "[places.start]\nproject = \"default.project.json\"\n[places.arena]\nproject = \"arena.project.json\"\n";

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_place(path: &Path) {
    let bytes = fs::read(path).unwrap_or_else(|_| panic!("{} was built", path.display()));
    assert!(bytes.starts_with(b"<roblox!"), "{} is a binary place file", path.display());
}

fn printed(output: &Output) -> Vec<String> {
    text(&output.stdout).lines().map(|line| line.replace('\\', "/")).collect()
}

#[test]
fn builds_every_place_and_prints_only_their_paths() {
    let game = Game::new(Some(TWO_PLACES));
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let root = game.root();
    let expected = [root.join("build/arena.rbxl"), root.join("build/start.rbxl")];
    assert_eq!(printed(&out), ["build/arena.rbxl", "build/start.rbxl"], "stdout holds only place paths");
    expected.iter().for_each(|path| assert_place(path));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("rbxtsc progress") && stderr.contains("Built project"), "{stderr}");
    let compiles = game.compiles();
    assert_eq!(compiles.len(), 1, "roblox-ts runs once for every place");
    assert_eq!(compiles[0]["args"], serde_json::json!(["--type", "game"]));
}

#[test]
fn without_manifest_builds_start_from_default_project() {
    let game = Game::new(None);
    let out = game.build(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert_eq!(printed(&out), ["build/start.rbxl"]);
    assert_place(&game.root().join("build/start.rbxl"));
}

#[test]
fn place_filter_limits_rojo_and_output_renames_the_file() {
    let game = Game::new(Some(TWO_PLACES));
    let out = game.build(&["--place", "arena", "--output", "custom/arena-test.rbxl"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let custom = game.root().join("custom/arena-test.rbxl");
    assert_eq!(printed(&out), ["custom/arena-test.rbxl"]);
    assert_place(&custom);
    assert!(!game.dir.path().join("build").exists(), "no other place was built");
}

#[test]
fn usage_errors_exit_2_before_compiling() {
    let game = Game::new(Some(TWO_PLACES));
    for (args, message) in [
        (&["--place", "lobby"][..], "unknown place lobby. Places: arena, start"),
        (&["--output", "one.rbxl"][..], "--output needs exactly one place"),
        (&["--place", "start", "--place", "arena", "--output", "one.rbxl"][..], "--output needs exactly one place"),
    ] {
        let out = game.build(args);
        assert_eq!(code(&out), 2, "{args:?}");
        assert!(text(&out.stderr).contains(message), "{args:?}: {}", text(&out.stderr));
        assert!(out.stdout.is_empty());
    }
    let broken = Game::new(Some("[places.start]\nprojet = \"default.project.json\"\n"));
    let out = broken.build(&[]);
    assert_eq!(code(&out), 2);
    assert!(text(&out.stderr).contains("crucible.toml") && text(&out.stderr).contains("unknown field"));
    assert!(game.compiles().is_empty() && broken.compiles().is_empty(), "no usage error compiles");
}

#[test]
fn roblox_ts_failure_keeps_its_diagnostics_and_builds_nothing() {
    let game = Game::new(None);
    let out = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .arg("build")
        .current_dir(game.dir.path())
        .env("FAKE_RBXTSC_EXIT", "3")
        .output()
        .unwrap();
    assert_eq!(code(&out), 1);
    let stderr = text(&out.stderr);
    assert!(stderr.contains("src/main.server.ts:1:1 - error TS1005: fake failure"), "{stderr}");
    assert!(stderr.contains("crucible build: roblox-ts failed"), "{stderr}");
    assert!(!stderr.contains('\x1b'), "redirected stderr carries no colour: {stderr:?}");
    assert!(out.stdout.is_empty());
    assert!(!game.dir.path().join("build").exists());
    assert!(!game.dir.path().join("out/transformer.sha256").exists());
}

#[test]
fn rojo_failure_exits_1_with_rojo_diagnostics() {
    let game = Game::new(Some("[places.start]\nproject = \"missing.project.json\"\n"));
    let out = game.build(&[]);
    assert_eq!(code(&out), 1);
    let stderr = text(&out.stderr);
    assert!(stderr.contains("failed to canonicalize path") && stderr.contains("Caused by:"), "{stderr}");
    assert!(stderr.contains("crucible build: place start:"), "{stderr}");
    assert!(out.stdout.is_empty());
}

#[test]
fn transformer_change_compiles_from_clean() {
    let game = Game::new(None);
    let receipt = game.dir.path().join("out/transformer.sha256");
    let stale = game.dir.path().join("out/stale");
    assert_eq!(code(&game.build(&[])), 0);
    let first = fs::read_to_string(&receipt).unwrap();
    fs::write(&stale, "").unwrap();
    assert_eq!(code(&game.build(&[])), 0);
    assert_eq!(game.compiles()[1]["stale"], true, "an unchanged transformer keeps out");
    let transformer = game.dir.path().join("node_modules/@trioforge/crucible/transformer");
    fs::write(transformer.join("type-id.cjs"), "a").unwrap();
    let out = game.build(&[]);
    assert_eq!(code(&out), 0);
    assert!(text(&out.stderr).contains("transformer changed"));
    assert_eq!(game.compiles()[2]["stale"], false, "a changed transformer removes out before compiling");
    let second = fs::read_to_string(&receipt).unwrap();
    assert_ne!(first, second);
    fs::write(&stale, "").unwrap();
    fs::write(transformer.join("type-id.cjs"), "b").unwrap();
    assert_eq!(code(&game.build(&[])), 0);
    assert_eq!(game.compiles()[3]["stale"], false, "a same-length content edit invalidates");
    fs::create_dir(transformer.join("lib")).unwrap();
    fs::write(transformer.join("lib/macro.cjs"), "a").unwrap();
    fs::write(&stale, "").unwrap();
    assert_eq!(code(&game.build(&[])), 0);
    assert_eq!(game.compiles()[4]["stale"], false, "a new nested file invalidates");
    fs::write(&stale, "").unwrap();
    fs::write(transformer.join("lib/macro.cjs"), "b").unwrap();
    assert_eq!(code(&game.build(&[])), 0);
    assert_eq!(game.compiles()[5]["stale"], false, "a nested content edit invalidates");
}

#[test]
fn unreadable_manifest_exits_1() {
    let game = Game::new(None);
    fs::create_dir(game.dir.path().join("crucible.toml")).unwrap();
    let out = game.build(&[]);
    assert_eq!(code(&out), 1, "an I/O failure is not a usage error");
    assert!(text(&out.stderr).contains("cannot read crucible.toml"), "{}", text(&out.stderr));
    assert!(game.compiles().is_empty());
}

#[test]
fn missing_roblox_ts_exits_1() {
    let game = Game::new(None);
    fs::remove_dir_all(game.dir.path().join("node_modules/roblox-ts")).unwrap();
    let out = game.build(&[]);
    assert_eq!(code(&out), 1);
    assert!(text(&out.stderr).contains("roblox-ts is not installed"), "{}", text(&out.stderr));
}

#[cfg(debug_assertions)]
#[test]
fn killing_crucible_stops_its_rojo_child() {
    use std::io::BufRead;
    use std::time::Duration;
    let game = Game::new(None);
    let mut parent = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .arg("build")
        .current_dir(game.dir.path())
        .env("CRUCIBLE_TEST_HOLD_ROJO", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (lines, received) = std::sync::mpsc::channel();
    let stderr = std::io::BufReader::new(parent.stderr.take().unwrap());
    std::thread::spawn(move || {
        for line in stderr.lines() {
            let _ = lines.send(Some(line.unwrap()));
        }
        let _ = lines.send(None);
    });
    let pid = loop {
        match received.recv_timeout(Duration::from_secs(60)).expect("the Rojo child starts") {
            Some(line) => {
                if let Some(pid) = line.strip_prefix("rojo child ").and_then(|rest| rest.strip_suffix(" holding")) {
                    break pid.to_owned();
                }
            }
            None => panic!("crucible build exited before Rojo started"),
        }
    };
    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let gone = loop {
        match received.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(Some(_)) => continue,
            Ok(None) => break true,
            Err(_) => break false,
        }
    };
    if !gone {
        if cfg!(windows) {
            let _ = Command::new("taskkill").args(["/F", "/PID", &pid]).output();
        } else {
            let _ = Command::new("kill").args(["-9", &pid]).output();
        }
    }
    assert!(gone, "the Rojo child {pid} still holds stderr 10 s after its parent was killed");
}

#[test]
fn version_reports_embedded_rojo() {
    let out = Command::new(env!("CARGO_BIN_EXE_crucible")).arg("--version").output().unwrap();
    assert_eq!(text(&out.stdout).trim(), format!("crucible {} (rojo 7.7.1)", env!("CARGO_PKG_VERSION")));
}
