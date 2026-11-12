use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn roblox_ts_game(root: &Path) -> String {
    let manifest = format!(
        "{{\n\t\"name\": \"game\",\n\t\"private\": true,\n\t\"dependencies\": {{\n\t\t\"@trioforge/crucible\": \"{VERSION}\"\n\t}},\n\t\"devDependencies\": {{\n\t\t\"@trioforge/crucible-cli\": \"{VERSION}\",\n\t\t\"roblox-ts\": \"3.0.0\",\n\t\t\"typescript\": \"5.5.3\",\n\t\t\"@rbxts/compiler-types\": \"3.0.0-types.0\"\n\t}}\n}}\n"
    );
    let mut value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let runtime: serde_json::Value =
        serde_json::from_str(include_str!("../../../packages/crucible/package.json")).unwrap();
    for peer in ["@rbxts/charm", "@rbxts/charm-sync"] {
        value["dependencies"][peer] = runtime["peerDependencies"][peer].clone();
    }
    let manifest = serde_json::to_string_pretty(&value).unwrap();
    fs::write(root.join("package.json"), &manifest).unwrap();
    for package in ["crucible-cli", "crucible"] {
        let dir = root.join("node_modules/@trioforge").join(package);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("package.json"), format!("{{\"version\":\"{VERSION}\"}}")).unwrap();
    }
    manifest
}

fn npm_exiting(bin: &Path, code: u8) {
    let (name, script) = if cfg!(windows) {
        ("npm.cmd", format!("@echo off\necho reinstall-output\nexit /b {code}\n"))
    } else {
        ("npm", format!("#!/bin/sh\necho reinstall-output\nexit {code}\n"))
    };
    fs::write(bin.join(name), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn crucible(root: &Path, bin: &Path, args: &[&str]) -> Output {
    let mut paths = vec![bin.to_path_buf()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    Command::new(env!("CARGO_BIN_EXE_crucible"))
        .args(args)
        .current_dir(root)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("npm_config_user_agent", "npm/11.0.0")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_failed_reinstall_off_roblox_ts_restores_package_json_and_exits_1_until_upgrade_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let (root, bin) = (dir.path().join("game"), dir.path().join("bin"));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let manifest = roblox_ts_game(&root);

    npm_exiting(&bin, 42);
    let failed = crucible(&root, &bin, &["__upgrade-apply", "0.2.0", "0.2.0", "0.2.0"]);
    let (stdout, stderr) = (text(&failed.stdout), text(&failed.stderr));
    assert_eq!(failed.status.code(), Some(1), "{stdout}\n{stderr}");
    assert_eq!(fs::read_to_string(root.join("package.json")).unwrap(), manifest, "package.json is restored");
    assert!(stderr.contains("reinstall-output"), "{stderr}");
    assert!(
        stderr.contains(
            "`npm install` failed. package.json is restored, so it still lists roblox-ts, typescript, @rbxts/compiler-types. Run `npm install` to return node_modules to it, then run `npx crucible upgrade` again to finish the move off roblox-ts"
        ),
        "{stderr}"
    );
    assert!(
        stdout.starts_with(&format!("Upgraded crucible and @trioforge/crucible from 0.2.0 to {VERSION}.\n")),
        "{stdout}"
    );
    assert!(!stdout.contains("Moved this game") && !stdout.contains("run check"), "{stdout}");

    npm_exiting(&bin, 0);
    let finished = crucible(&root, &bin, &["upgrade", VERSION]);
    let stdout = text(&finished.stdout);
    assert_eq!(finished.status.code(), Some(0), "{stdout}\n{}", text(&finished.stderr));
    assert!(
        stdout.starts_with(&format!(
            "crucible is already at {VERSION}.\n\nMoved this game from roblox-ts to Crucible's compiler:\n  package.json lists @trioforge/compiler-types {VERSION}\n  package.json no longer lists roblox-ts, typescript, @rbxts/compiler-types\n  ran `npm install`\n"
        )),
        "{stdout}"
    );
    let migrated = fs::read_to_string(root.join("package.json")).unwrap();
    assert!(!migrated.contains("roblox-ts") && migrated.contains("@trioforge/compiler-types"), "{migrated}");

    let again = crucible(&root, &bin, &["upgrade", VERSION]);
    assert_eq!(again.status.code(), Some(0));
    assert_eq!(text(&again.stdout), format!("crucible is already at {VERSION}.\n"));
}

fn apply_with_old_gitignore(gitignore: &str, old: &str) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("package.json"), "{\n\t\"name\": \"game\",\n\t\"private\": true\n}\n").unwrap();
    let installed = root.join("node_modules/@trioforge/crucible-cli");
    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("package.json"), format!("{{\"version\":\"{VERSION}\"}}")).unwrap();
    fs::write(root.join(".gitignore"), gitignore).unwrap();
    let snapshot = serde_json::json!({
        "format": 1,
        "version": VERSION,
        "owned": [{"path": ".gitignore", "section": null, "content": old}],
        "docs": {},
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .args(["__upgrade-apply", VERSION, VERSION, VERSION])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), snapshot.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    (fs::read_to_string(root.join(".gitignore")).unwrap(), text(&out.stdout))
}

#[test]
fn an_untouched_gitignore_gains_dot_env_and_an_edited_one_is_left_alone() {
    let old = "node_modules/\nout/\ninclude/\nbuild/\n";
    let (refreshed, report) = apply_with_old_gitignore(old, old);
    assert_eq!(refreshed, format!("{old}.env\n"));
    assert!(report.contains(".gitignore"), "{report}");

    let edited = format!("{old}notes/\n");
    let (kept, report) = apply_with_old_gitignore(&edited, old);
    assert_eq!(kept, edited);
    assert!(report.contains(".gitignore"), "{report}");
}
