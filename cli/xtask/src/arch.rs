//! `cargo xtask arch` checks the architecture rules of `design/architecture.md` and compares what it finds with the
//! violation list checked in at `cli/xtask/arch.txt`. The list can only shrink: a violation it lacks fails the check,
//! and so does a listed one that no longer exists.
//!
//! A rule that names a crate applies once a workspace crate has that name, so rules about crates that do not exist
//! yet already run.

mod rust;
mod ts;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::run::{self, repo, slashed};

pub const LIST: &str = "cli/xtask/arch.txt";

/// The binary that composes the products. Rule 3 lets only it parse arguments, draw a terminal or read the
/// environment.
const CLI: &str = "crucible";
/// Rule 1's generic crates.
const GENERIC: &[&str] = &["crucible-compiler", "crucible-luau"];
/// Rule 2's infrastructure crates and what they may never reach. A `tsrs_` prefix covers every tsrs crate.
const INFRASTRUCTURE: &[&str] = &["crucible-iac", "crucible-oc"];
const NOT_FOR_INFRASTRUCTURE: &[&str] = &["crucible-compiler", "crucible-transform"];
const ABI: &str = "crucible-abi";
/// The repository's own tooling. It names what it checks, and it is not a library, so rules 3 and 4 skip its source.
const TOOLING: &str = "xtask";

/// Rule 1's literal needles: a string that contains one of these spells a framework module or global.
const FRAMEWORK_TEXT: &[&str] = &["@trioforge", "crucible/emit", "__crucible", "__ui"];
/// Rule 1's framework macros, the `Crucible` group of `crates/compiler/src/macros.rs` at the audited commit.
const FRAMEWORK_MACROS: &[&str] = &[
    "attribute",
    "inject",
    "boot",
    "toServer",
    "toClient",
    "askServer",
    "receive",
    "replicated",
    "replicatedPerPlayer",
    "profileStore",
    "dataStore",
    "profileMessage",
    "config",
    "playerConfig",
];
/// Rule 3's terminal and argument crates.
const CLI_ONLY_DEPENDENCIES: &[&str] = &["clap", "ratatui", "syntect"];
/// Rule 3's environment reads. `env!` and `option_env!` read the build's environment, so they are not listed.
const ENVIRONMENT_READS: &[&str] = &["env::var(", "env::var_os(", "env::vars(", "env::vars_os("];
/// The UI calls compiled JSX makes. The runtime's UI entry also exports functions games call, which are not ABI.
/// Rule 4 skips the one-letter ABI names, the `n` and `t` namespaces, because a one-letter string is too common to
/// scan for. The ABI crate's own check covers them once it exists.
const UI_ABI: &[&str] = &["bind", "append"];

const USAGE: &str = "usage: cargo xtask arch [check|record]";

pub fn main(args: &[String]) -> Result<(), String> {
    let root = repo();
    let found = violations(&root)?;
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["check"] => check(&root, &found),
        ["record"] => {
            fs::write(root.join(LIST), render(&found)).map_err(|e| format!("cannot write {LIST}: {e}"))?;
            println!("arch: recorded {} violations in {LIST} ({})", found.len(), summary(&found));
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn check(root: &Path, found: &[String]) -> Result<(), String> {
    let path = root.join(LIST);
    let listed = parse(&fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?);
    let (new, gone) = difference(&listed, found);
    if new.is_empty() && gone.is_empty() {
        println!("arch: {} violations, all listed in {LIST} ({})", found.len(), summary(found));
        return Ok(());
    }
    if !new.is_empty() {
        println!("arch: {} violations are not in {LIST}. Fix them:", new.len());
        new.iter().for_each(|line| println!("  + {line}"));
    }
    if !gone.is_empty() {
        println!("arch: {} listed violations no longer exist. Remove them with `cargo xtask arch record`:", gone.len());
        gone.iter().for_each(|line| println!("  - {line}"));
    }
    Err("arch: the violations differ from the list".into())
}

/// The lines in `found` but not `listed`, and the lines in `listed` but not `found`, counting repeats.
pub fn difference(listed: &[String], found: &[String]) -> (Vec<String>, Vec<String>) {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    found.iter().for_each(|line| *counts.entry(line).or_default() += 1);
    listed.iter().for_each(|line| *counts.entry(line).or_default() -= 1);
    let (mut new, mut gone) = (Vec::new(), Vec::new());
    for (line, count) in counts {
        let target = if count > 0 { &mut new } else { &mut gone };
        (0..count.abs()).for_each(|_| target.push(line.to_owned()));
    }
    (new, gone)
}

const HEADER: &str = "\
# Architecture rule violations that `cargo xtask arch` found when it was added. This list may only shrink.
# Each line is `<rule> <where> <what>`. Rust files are named by crate and crate-relative path, and lines carry no
# line numbers, so an edit elsewhere in a file never changes the list. A repeated line is a repeated violation.
";

pub fn render(found: &[String]) -> String {
    let mut text = HEADER.to_owned();
    found.iter().for_each(|line| {
        text.push_str(line);
        text.push('\n');
    });
    text
}

pub fn parse(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

/// Counts per rule, such as `1: 40, 4: 12`.
pub fn summary(found: &[String]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for line in found {
        *counts.entry(line.split(' ').next().unwrap_or_default()).or_default() += 1;
    }
    if counts.is_empty() {
        return "none".into();
    }
    counts.iter().map(|(rule, count)| format!("rule {rule}: {count}")).collect::<Vec<_>>().join(", ")
}

/// Every violation in the tree, sorted.
pub fn violations(root: &Path) -> Result<Vec<String>, String> {
    let workspace = Workspace::load(root)?;
    let abi = abi_names(root)?;
    let mut found = Vec::new();
    for package in &workspace.packages {
        let name = package.name.as_str();
        dependency_rules(&workspace, package, &mut found);
        let mut scans = Vec::new();
        for path in package.sources()? {
            let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            scans.push((path, rust::scan(&text)));
        }
        let tests: Vec<PathBuf> =
            scans.iter().flat_map(|(path, scan)| test_module_paths(path, &scan.test_modules)).collect();
        for (path, scan) in &scans {
            if tests.iter().any(|test| path == test || path.starts_with(test.with_extension(""))) {
                continue;
            }
            let at = format!("{name}:{}", slashed(path.strip_prefix(&package.dir).unwrap_or(path)));
            if GENERIC.contains(&name) {
                for literal in &scan.strings {
                    if let Some(shown) = framework_literal(literal) {
                        found.push(format!("1 {at} {shown}"));
                    }
                }
            }
            if name != CLI && name != TOOLING {
                for read in ENVIRONMENT_READS {
                    for _ in scan.code.matches(read) {
                        found.push(format!("3 {at} reads std::{}", read.trim_end_matches('(')));
                    }
                }
            }
            if name != ABI && name != TOOLING {
                for literal in &scan.strings {
                    let named = literal.len() > 1 && abi.contains(literal.as_str());
                    if named || literal.starts_with("@trioforge/crucible/") {
                        found.push(format!("4 {at} {}", shown(literal, literal)));
                    }
                }
            }
        }
    }
    for (from, to) in ts::root_reaches(root)? {
        found.push(format!("5 {from} imports {to}"));
    }
    found.sort();
    Ok(found)
}

/// The files of `#[cfg(test)] mod name;` declarations in `file`, each as `name.rs`. A test module's submodules live
/// in the folder of the same name, which the caller skips too.
fn test_module_paths(file: &Path, modules: &[String]) -> Vec<PathBuf> {
    let dir = file.parent().unwrap_or(file);
    let owns_folder = file.file_name().is_some_and(|name| name == "lib.rs" || name == "main.rs" || name == "mod.rs");
    let base = if owns_folder { dir.to_path_buf() } else { dir.join(file.file_stem().unwrap_or_default()) };
    modules.iter().flat_map(|module| [base.join(format!("{module}.rs")), base.join(module).join("mod.rs")]).collect()
}

fn framework_literal(literal: &str) -> Option<String> {
    if FRAMEWORK_MACROS.contains(&literal) {
        return Some(shown(literal, literal));
    }
    FRAMEWORK_TEXT.iter().find(|needle| literal.contains(**needle)).map(|needle| shown(literal, needle))
}

/// The literal in quotes, or only the matched text when the literal is long or spans lines.
fn shown(literal: &str, needle: &str) -> String {
    if literal.len() <= 80 && !literal.contains('\n') { format!("{literal:?}") } else { format!("…{needle:?}…") }
}

/// The names the transform emits calls to: the value exports of the runtime's emit entry and the UI calls.
fn abi_names(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut names = ts::value_exports(root, &root.join("packages/crucible/src/emit/index.ts"))?;
    names.extend(UI_ABI.iter().map(|name| (*name).to_owned()));
    Ok(names)
}

fn dependency_rules(workspace: &Workspace, package: &Package, found: &mut Vec<String>) {
    let name = package.name.as_str();
    if GENERIC.contains(&name) {
        for dependency in &package.workspace_dependencies {
            if dependency != "crucible-luau" {
                found.push(format!("1 {name} depends on {dependency}"));
            }
        }
    }
    if INFRASTRUCTURE.contains(&name) {
        for reached in workspace.reaches(&package.id) {
            if NOT_FOR_INFRASTRUCTURE.contains(&reached.as_str()) || reached.starts_with("tsrs_") {
                found.push(format!("2 {name} depends on {reached}"));
            }
        }
    }
    if name != CLI {
        for dependency in &package.dependencies {
            if CLI_ONLY_DEPENDENCIES.contains(&dependency.as_str()) {
                found.push(format!("3 {name} depends on {dependency}"));
            }
        }
    }
}

struct Package {
    id: String,
    name: String,
    dir: PathBuf,
    /// Normal and build dependencies by name. Rule 3 ignores dev-dependencies, because tests are not shipped.
    dependencies: BTreeSet<String>,
    /// Every dependency kind, limited to workspace members.
    workspace_dependencies: BTreeSet<String>,
}

impl Package {
    /// The crate's own source: `src` and `build.rs`. The rules apply to what the crate ships, so integration tests,
    /// examples and `#[cfg(test)]` code are left out. The framework's tests move with the framework in this refactor.
    fn sources(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        collect_rust(&self.dir.join("src"), &mut files)?;
        let build = self.dir.join("build.rs");
        if build.is_file() {
            files.push(build);
        }
        files.sort();
        Ok(files)
    }
}

fn collect_rust(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(dir) else { return Ok(()) };
    for entry in entries {
        let path = entry.map_err(|e| format!("cannot list {}: {e}", dir.display()))?.path();
        if path.is_dir() {
            collect_rust(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

struct Workspace {
    packages: Vec<Package>,
    /// Package ID to its name and the IDs of its normal and build dependencies.
    graph: BTreeMap<String, (String, Vec<String>)>,
}

impl Workspace {
    fn load(root: &Path) -> Result<Self, String> {
        let text = run::stdout(
            Command::new("cargo")
                .args(["metadata", "--format-version", "1", "--locked", "--manifest-path"])
                .arg(root.join("cli/Cargo.toml")),
        )?;
        let metadata: Value = serde_json::from_str(&text).map_err(|e| format!("cargo metadata: {e}"))?;
        let members: BTreeSet<&str> =
            metadata["workspace_members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let all = metadata["packages"].as_array().ok_or("cargo metadata lists no packages")?;
        let member_names: BTreeSet<&str> = all
            .iter()
            .filter(|package| package["id"].as_str().is_some_and(|id| members.contains(id)))
            .filter_map(|package| package["name"].as_str())
            .collect();
        let mut packages = Vec::new();
        for package in all.iter().filter(|package| package["id"].as_str().is_some_and(|id| members.contains(id))) {
            let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
            let dependencies = package["dependencies"].as_array().into_iter().flatten();
            let name = package["name"].as_str().unwrap_or_default().to_owned();
            packages.push(Package {
                id: package["id"].as_str().unwrap_or_default().to_owned(),
                dir: manifest.parent().map(Path::to_path_buf).unwrap_or_default(),
                dependencies: dependencies
                    .clone()
                    .filter(|dependency| dependency["kind"].as_str() != Some("dev"))
                    .filter_map(|dependency| dependency["name"].as_str().map(str::to_owned))
                    .collect(),
                workspace_dependencies: dependencies
                    .filter_map(|dependency| dependency["name"].as_str())
                    .filter(|dependency| member_names.contains(dependency) && *dependency != name)
                    .map(str::to_owned)
                    .collect(),
                name,
            });
        }
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        let names: BTreeMap<&str, &str> =
            all.iter().filter_map(|package| Some((package["id"].as_str()?, package["name"].as_str()?))).collect();
        let mut graph = BTreeMap::new();
        for node in metadata["resolve"]["nodes"].as_array().into_iter().flatten() {
            let Some(id) = node["id"].as_str() else { continue };
            let deps = node["deps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|dep| {
                    dep["dep_kinds"].as_array().into_iter().flatten().any(|kind| kind["kind"].as_str() != Some("dev"))
                })
                .filter_map(|dep| dep["pkg"].as_str().map(str::to_owned))
                .collect();
            graph.insert(id.to_owned(), (names.get(id).copied().unwrap_or_default().to_owned(), deps));
        }
        Ok(Self { packages, graph })
    }

    /// The names of every package `id` reaches through normal and build dependencies.
    fn reaches(&self, id: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut stack = vec![id.to_owned()];
        while let Some(at) = stack.pop() {
            let Some((_, deps)) = self.graph.get(&at) else { continue };
            for dep in deps {
                if seen.insert(dep.clone()) {
                    if let Some((name, _)) = self.graph.get(dep) {
                        names.insert(name.clone());
                    }
                    stack.push(dep.clone());
                }
            }
        }
        names
    }
}
