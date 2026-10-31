use std::fs;
use std::process::{Command, Output};

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crucible")).arg("create").args(args).current_dir(root).output().unwrap()
}

#[test]
fn creates_default_and_explicit_targets_without_installing() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["my-game", "--no-install"], vec!["another-game", "nested/with spaces", "--no-install"]] {
        let output = run(dir.path(), &args);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let target = if args.len() == 2 { args[0] } else { args[1] };
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), target);
        let root = dir.path().join(target);
        let package: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("package.json")).unwrap()).unwrap();
        assert_eq!(package["name"], args[0]);
        assert_eq!(package["dependencies"]["@trioforge/crucible"], env!("CARGO_PKG_VERSION"));
        assert_eq!(package["devDependencies"]["crucible"], env!("CARGO_PKG_VERSION"));
        for file in [
            "tsconfig.json",
            "default.project.json",
            "crucible.toml",
            "AGENTS.md",
            "LICENSE",
            ".gitignore",
            ".gitattributes",
            "vite.config.ts",
            "src/server/main.server.ts",
            "src/client/main.client.ts",
        ] {
            let content = fs::read_to_string(root.join(file)).unwrap();
            assert!(!content.contains("__NAME__") && !content.contains("__VERSION__"), "{file}");
        }
        assert!(!root.join("node_modules").exists());
    }
}

#[test]
fn refuses_bad_names_files_and_nonempty_directories_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "../escape",
        "UPPER",
        "@scope/game",
        "bad name",
        "node_modules",
        "favicon.ico",
        "a.",
        "a ",
        "con",
        "prn.txt",
        "aux",
        "nul",
        "com1",
        "lpt9.json",
    ] {
        let output = run(dir.path(), &[name, "target", "--no-install"]);
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(!dir.path().join("target").exists());
    }
    fs::create_dir(dir.path().join("occupied")).unwrap();
    fs::write(dir.path().join("occupied/.keep"), "keep").unwrap();
    fs::write(dir.path().join("file"), "keep").unwrap();
    for target in [
        "occupied",
        "file",
        "file/sub",
        "missing/con",
        "missing/AUX.txt",
        "missing/NUL",
        "missing/COM9.log",
        "missing/LPT1",
        "missing/a.",
        "missing/a ",
    ] {
        let output = run(dir.path(), &["my-game", target, "--no-install"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!dir.path().join("missing").exists());
    }
    assert_eq!(fs::read_to_string(dir.path().join("occupied/.keep")).unwrap(), "keep");
    assert_eq!(fs::read_dir(dir.path().join("occupied")).unwrap().count(), 1);
}

#[test]
fn installs_with_the_detected_manager_and_reports_failure() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let script = if cfg!(windows) {
        "@echo off\necho %* > install.txt\necho install-output\nexit /b 0\n"
    } else {
        "#!/bin/sh\nprintf '%s' \"$*\" > install.txt\necho install-output\nexit 0\n"
    };
    let executable = bin.join(if cfg!(windows) { "pnpm.cmd" } else { "pnpm" });
    fs::write(&executable, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let path = std::env::join_paths(paths).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .args(["create", "installed"])
        .current_dir(dir.path())
        .env("PATH", &path)
        .env("npm_config_user_agent", "pnpm/12 npm/?")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read_to_string(dir.path().join("installed/install.txt")).unwrap().trim(), "install");
    assert!(String::from_utf8_lossy(&output.stderr).contains("install-output"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("install-output"));
    fs::write(&executable, script.replace("exit /b 0", "exit /b 1").replace("exit 0", "exit 1")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crucible"))
        .args(["create", "failed"])
        .current_dir(dir.path())
        .env("PATH", path)
        .env("npm_config_user_agent", "pnpm/12")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(dir.path().join("failed/package.json").is_file());
}

#[test]
fn yarn_uses_node_modules_even_without_installing() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let executable = bin.join(if cfg!(windows) { "yarn.cmd" } else { "yarn" });
    fs::write(
        &executable,
        if cfg!(windows) {
            "@echo off\ntype .yarnrc.yml > install.txt\nexit /b 0\n"
        } else {
            "#!/bin/sh\ncat .yarnrc.yml > install.txt\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    for (name, extra) in [("installed", vec![]), ("skipped", vec!["--no-install"])] {
        let output = Command::new(env!("CARGO_BIN_EXE_crucible"))
            .args(["create", name])
            .args(extra)
            .current_dir(dir.path())
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .env("npm_config_user_agent", "yarn/4.0")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            fs::read_to_string(dir.path().join(name).join(".yarnrc.yml")).unwrap(),
            "nodeLinker: node-modules\n"
        );
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("installed/install.txt")).unwrap().trim(),
        "nodeLinker: node-modules"
    );
    assert!(!dir.path().join("skipped/install.txt").exists());
    assert!(run(dir.path(), &["npm-game", "--no-install"]).status.success());
    assert!(!dir.path().join("npm-game/.yarnrc.yml").exists());
}
