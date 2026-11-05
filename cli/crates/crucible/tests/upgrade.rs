use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn roblox_ts_game(root: &Path) -> String {
    let manifest = format!(
        "{{\n\t\"name\": \"game\",\n\t\"private\": true,\n\t\"dependencies\": {{\n\t\t\"@trioforge/crucible\": \"{VERSION}\"\n\t}},\n\t\"devDependencies\": {{\n\t\t\"@trioforge/crucible-cli\": \"{VERSION}\",\n\t\t\"roblox-ts\": \"3.0.0\",\n\t\t\"typescript\": \"5.5.3\",\n\t\t\"@rbxts/compiler-types\": \"3.0.0-types.0\"\n\t}}\n}}\n"
    );
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
    let failed = crucible(&root, &bin, &["__upgrade-apply", "0.3.0", "0.3.0", "0.3.0"]);
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
    assert!(stdout.starts_with(&format!("Upgraded crucible and @trioforge/crucible from 0.3.0 to {VERSION}.\n")));
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
