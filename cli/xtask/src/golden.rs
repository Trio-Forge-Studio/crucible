//! `cargo xtask golden` builds the runtime and the sample game from clean, hashes every output file, and compares
//! the hashes with the baseline checked in at `cli/xtask/golden.txt`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::run::{self, repo, slashed};

/// Output path, relative to the repository root with forward slashes, to its SHA-256.
pub type Manifest = BTreeMap<String, String>;

pub const BASELINE: &str = "cli/xtask/golden.txt";
/// Fragile Delivery's baseline is local, because its source is not in this repository.
const FD_BASELINE: &str = "build/golden/fd.txt";

/// The packages whose `build` script `pnpm build` runs, in order. The sample game type-checks against the
/// runtime's declarations, so the runtime builds first.
const PACKAGES: &[&str] = &["packages/crucible", "examples/sample-game"];

/// Everything those builds write. Each is removed first, so a file a change stops emitting cannot linger.
const OUTPUTS: &[&str] =
    &["packages/crucible/out", "examples/sample-game/out", "examples/sample-game/include", "build/sample-game.rbxl"];

const USAGE: &str = "usage: cargo xtask golden <record|check> [--fd <fragile-delivery checkout>]";

pub fn main(args: &[String]) -> Result<(), String> {
    let (mode, fd) = match args {
        [mode] => (mode.as_str(), None),
        [mode, flag, path] if flag == "--fd" => (mode.as_str(), Some(PathBuf::from(path))),
        _ => return Err(USAGE.into()),
    };
    let root = repo();
    let fd_manifest = |root: &Path| fd.as_deref().map(|fd| fd::manifest(root, fd)).transpose();
    match mode {
        "record" => {
            let manifest = build(&root)?;
            write(&root.join(BASELINE), &manifest)?;
            println!("golden: recorded {} files in {BASELINE}", manifest.len());
            if let Some(manifest) = fd_manifest(&root)? {
                write(&root.join(FD_BASELINE), &manifest)?;
                println!("golden: recorded {} Fragile Delivery files in {FD_BASELINE}", manifest.len());
            }
            Ok(())
        }
        "check" => {
            let manifest = build(&root)?;
            let mut differs = report(BASELINE, &read(&root.join(BASELINE))?, &manifest);
            if let Some(manifest) = fd_manifest(&root)? {
                let baseline = read(&root.join(FD_BASELINE)).map_err(|e| {
                    format!("{e}\nRecord it on the base commit first: cargo xtask golden record --fd <path>")
                })?;
                differs |= report(FD_BASELINE, &baseline, &manifest);
            }
            if differs { Err("golden: the output differs from the baseline".into()) } else { Ok(()) }
        }
        _ => Err(USAGE.into()),
    }
}

/// Builds the runtime and the sample game from clean and hashes what they write.
pub fn build(root: &Path) -> Result<Manifest, String> {
    for output in OUTPUTS {
        remove(&root.join(output))?;
    }
    for package in PACKAGES {
        run_build_script(&root.join(package))?;
    }
    let mut manifest = Manifest::new();
    for output in OUTPUTS {
        hash_into(&mut manifest, root, &root.join(output), "")?;
    }
    Ok(manifest)
}

/// Runs the package's own `build` script, so this check cannot drift from `pnpm build`. The scripts are one plain
/// command each, so they run without a shell, which behaves the same on every OS.
fn run_build_script(dir: &Path) -> Result<(), String> {
    let manifest = dir.join("package.json");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let package: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let script =
        package["scripts"]["build"].as_str().ok_or_else(|| format!("{} has no build script", manifest.display()))?;
    if script.contains(['&', '|', ';', '<', '>', '"', '\'', '$', '`']) {
        return Err(format!(
            "{}: golden runs build scripts without a shell, so `{script}` must be one plain command",
            manifest.display()
        ));
    }
    let mut words = script.split_whitespace();
    let program = words.next().ok_or_else(|| format!("{} has an empty build script", manifest.display()))?;
    run::status(Command::new(program).args(words).current_dir(dir))
}

fn remove(path: &Path) -> Result<(), String> {
    let result = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(_) => return Ok(()),
    };
    result.map_err(|e| format!("cannot remove {}: {e}", path.display()))
}

/// Adds every file at or under `path` to the manifest, keyed by its path relative to `base` after `prefix`.
fn hash_into(manifest: &mut Manifest, base: &Path, path: &Path, prefix: &str) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("the build did not write {}: {e}", path.display()))?;
    if meta.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)
            .and_then(|entries| entries.map(|entry| entry.map(|entry| entry.path())).collect::<Result<_, _>>())
            .map_err(|e| format!("cannot list {}: {e}", path.display()))?;
        entries.sort();
        for entry in entries {
            hash_into(manifest, base, &entry, prefix)?;
        }
    } else if meta.is_file() {
        let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let relative =
            path.strip_prefix(base).map_err(|_| format!("{} is outside {}", path.display(), base.display()))?;
        manifest.insert(format!("{prefix}{}", slashed(relative)), format!("{:x}", Sha256::digest(&bytes)));
    } else {
        return Err(format!("{} is neither a file nor a folder", path.display()));
    }
    Ok(())
}

const HEADER: &str = "# The SHA-256 of every file `pnpm build` writes, recorded by `cargo xtask golden record`.\n";

pub fn render(manifest: &Manifest) -> String {
    let mut text = HEADER.to_owned();
    for (path, hash) in manifest {
        text.push_str(&format!("{hash}  {path}\n"));
    }
    text
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let mut manifest = Manifest::new();
    for line in text.lines().map(|line| line.trim_end_matches('\r')) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hash, path) = line.split_once("  ").ok_or_else(|| format!("malformed manifest line: {line}"))?;
        manifest.insert(path.to_owned(), hash.to_owned());
    }
    Ok(manifest)
}

fn read(path: &Path) -> Result<Manifest, String> {
    parse(&fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
}

fn write(path: &Path, manifest: &Manifest) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, render(manifest)).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

pub fn compare(baseline: &Manifest, current: &Manifest) -> Diff {
    let mut diff = Diff::default();
    for (path, hash) in current {
        match baseline.get(path) {
            Some(old) if old == hash => {}
            Some(_) => diff.changed.push(path.clone()),
            None => diff.added.push(path.clone()),
        }
    }
    diff.removed = baseline.keys().filter(|path| !current.contains_key(*path)).cloned().collect();
    diff
}

/// Prints the comparison and returns whether anything differs.
fn report(name: &str, baseline: &Manifest, current: &Manifest) -> bool {
    let diff = compare(baseline, current);
    if diff.is_empty() {
        println!("golden: {} files identical to {name}", current.len());
        return false;
    }
    println!("golden: the output differs from {name}");
    for (label, paths) in [("changed", &diff.changed), ("added", &diff.added), ("removed", &diff.removed)] {
        for path in paths {
            let hash = current.get(path).or_else(|| baseline.get(path)).map_or("", String::as_str);
            println!("  {label:<8}{path}  {hash}");
        }
    }
    true
}

/// Fragile Delivery, built from a temporary copy so the checkout is never written to.
mod fd {
    use super::*;

    /// What FD's build writes, under the copy's root.
    fn outputs(copy: &Path) -> Result<Vec<PathBuf>, String> {
        let mut outputs = Vec::new();
        let places = copy.join("places");
        let mut names: Vec<_> = fs::read_dir(&places)
            .map_err(|e| format!("cannot list {}: {e}", places.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .collect();
        names.sort();
        for name in names {
            for folder in ["out", "include"] {
                let path = places.join(&name).join(folder);
                if path.is_dir() {
                    outputs.push(path);
                }
            }
        }
        let mut roots: Vec<_> = fs::read_dir(copy)
            .map_err(|e| format!("cannot list {}: {e}", copy.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.to_string_lossy().ends_with(".generated.project.json"))
            .collect();
        roots.sort();
        outputs.extend(roots);
        outputs.push(copy.join("build/compile"));
        Ok(outputs)
    }

    pub fn manifest(root: &Path, checkout: &Path) -> Result<Manifest, String> {
        let crucible = crucible_binary(root)?;
        let copy = Temporary::new("crucible-golden-fd")?;
        copy_sources(checkout, &copy.0)?;
        run::status(run::tool("pnpm").args(["install", "--frozen-lockfile"]).current_dir(&copy.0))?;
        use_local_crucible(&copy.0, &crucible)?;
        run::status(Command::new("node").args(["scripts/place.mjs", "compile"]).current_dir(&copy.0))?;
        let mut manifest = Manifest::new();
        for output in outputs(&copy.0)? {
            hash_into(&mut manifest, &copy.0, &output, "fd/")?;
        }
        Ok(manifest)
    }

    /// Builds this repository's `crucible` and returns its path.
    fn crucible_binary(root: &Path) -> Result<PathBuf, String> {
        let manifest = root.join("cli/Cargo.toml");
        let text = run::stdout(
            Command::new("cargo")
                .args(["build", "--quiet", "--locked", "--bin", "crucible", "--message-format=json", "--manifest-path"])
                .arg(&manifest),
        )?;
        text.lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find_map(|message| {
                let named = message["target"]["name"] == "crucible" && message["reason"] == "compiler-artifact";
                named.then(|| message["executable"].as_str().map(PathBuf::from)).flatten()
            })
            .ok_or_else(|| "cargo built no crucible executable".into())
    }

    /// Copies the files git tracks or would track, which leaves out `.git`, `node_modules` and every build output.
    fn copy_sources(checkout: &Path, copy: &Path) -> Result<(), String> {
        let listed = run::git(checkout, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
        for relative in listed.split('\0').filter(|path| !path.is_empty()) {
            let from = checkout.join(relative);
            if !from.is_file() {
                continue;
            }
            let to = copy.join(relative);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            fs::copy(&from, &to).map_err(|e| format!("cannot copy {}: {e}", from.display()))?;
        }
        Ok(())
    }

    /// FD runs `crucible` through the bin of `@trioforge/crucible-cli`. Replacing that package in the copy runs this
    /// repository's binary instead. pnpm may hard-link package files from its store, so the folder is removed and
    /// rewritten, never edited in place.
    fn use_local_crucible(copy: &Path, crucible: &Path) -> Result<(), String> {
        let package = copy.join("node_modules/@trioforge/crucible-cli");
        remove(&package)?;
        fs::create_dir_all(&package).map_err(|e| format!("cannot create {}: {e}", package.display()))?;
        let shim = format!(
            "const result = require(\"node:child_process\").spawnSync({}, process.argv.slice(2), {{ stdio: \"inherit\" }});\nprocess.exit(result.status ?? 1);\n",
            serde_json::to_string(&crucible.to_string_lossy()).map_err(|e| e.to_string())?
        );
        let manifest = r#"{ "name": "@trioforge/crucible-cli", "bin": { "crucible": "crucible.cjs" } }"#;
        fs::write(package.join("package.json"), manifest)
            .and_then(|()| fs::write(package.join("crucible.cjs"), shim))
            .map_err(|e| format!("cannot write the crucible shim: {e}"))
    }

    /// A folder under the system temporary folder, removed when dropped.
    struct Temporary(PathBuf);

    impl Temporary {
        fn new(name: &str) -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            remove(&path)?;
            fs::create_dir_all(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
            Ok(Self(path))
        }
    }

    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
