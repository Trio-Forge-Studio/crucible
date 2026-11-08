//! `cargo xtask arch` checks the architecture rules of `design/architecture.md` and compares what it finds with the
//! violation list checked in at `xtask/arch.txt`. A violation the list lacks fails the check, and so does a listed
//! one that no longer exists. `arch record` rewrites the list, so the check alone cannot stop it from growing.
//! `--base <rev>` adds the ratchet: it fails when a violation is new against the list at `<rev>`, or occurs more often
//! than there. CI passes the pull request's base, or the previous commit on `main`.
//!
//! A rule that names a crate applies once a workspace crate has that name, so rules about crates that do not exist
//! yet already run.
//!
//! What the count measures: the Rust rules read what `rust` reads, the files a shipped target compiles, and rule 5
//! reads what `ts` reads. Rules 1 and 4 count string literals, so a name assembled at run time, or coupling by meaning
//! such as the compiler's `MacroKind::Crucible` and `uses_crucible`, is not counted. Rule 3 sees `std::env` however it
//! is imported, but not an environment read through another crate. Rule 4 skips `n` and `t`, below.

mod rust;
mod ts;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::run::{self, repo, slashed};

pub const LIST: &str = "xtask/arch.txt";

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

/// The Cargo target kinds a crate ships. Tests, examples and benches are left out.
const SHIPPED: &[&str] = &["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin", "custom-build"];

const USAGE: &str = "usage: cargo xtask arch [check [--base <rev>] | record]";

pub fn main(args: &[String]) -> Result<(), String> {
    let root = repo();
    let (record, base) = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["check"] => (false, None),
        ["--base", base] | ["check", "--base", base] => (false, Some(base.to_string())),
        ["record"] => (true, None),
        _ => return Err(USAGE.into()),
    };
    let found = violations(&root)?;
    if record {
        fs::write(root.join(LIST), render(&found)).map_err(|e| format!("cannot write {LIST}: {e}"))?;
        println!("arch: recorded {} violations in {LIST} ({})", found.len(), summary(&found));
        return Ok(());
    }
    check(&root, &found)?;
    match base {
        Some(base) => ratchet(&root, &base, &found),
        None => Ok(()),
    }
}

fn ratchet(root: &Path, base: &str, found: &[String]) -> Result<(), String> {
    let Some(listed) = base_list(root, base)? else {
        println!("arch: {base} has no violation list, so the ratchet starts here with {} violations", found.len());
        return Ok(());
    };
    let (new, _) = difference(&listed, found);
    if new.is_empty() {
        println!("arch: no violation is new against {base} ({} there, {} now)", listed.len(), found.len());
        return Ok(());
    }
    println!("arch: {} violations are new against {base}, or occur more often than there:", new.len());
    new.iter().for_each(|line| println!("  + {line}"));
    Err(format!("arch: the violation list grew against {base}"))
}

/// The violation list at `rev`. A list moved since then is still found, because the ratchet must not reset when
/// `cli/` moves. `None` means `rev` predates the list, which only the commit that adds the list can meet: every
/// later base has one, and the check fails at any head that lacks one.
pub fn base_list(root: &Path, rev: &str) -> Result<Option<Vec<String>>, String> {
    let commit = run::git(root, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
        .map_err(|_| format!("arch: {rev} is not a commit this clone has"))?;
    let commit = commit.trim();
    if let Some(text) = run::git_maybe(root, &["show", &format!("{commit}:{LIST}")]) {
        return Ok(Some(parse(&text)));
    }
    let tree = run::git(root, &["ls-tree", "-r", "--name-only", commit])?;
    let moved: Vec<&str> =
        tree.lines().filter(|path| *path == "xtask/arch.txt" || path.ends_with("/xtask/arch.txt")).collect();
    match moved.as_slice() {
        [] => Ok(None),
        [path] => Ok(Some(parse(&run::git(root, &["show", &format!("{commit}:{path}")])?))),
        _ => Err(format!("arch: {rev} has more than one violation list: {}", moved.join(", "))),
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

/// Counts repeats, so a second occurrence of a listed line is new.
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

/// Such as `rule 1: 40, rule 4: 12`.
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

pub fn violations(root: &Path) -> Result<Vec<String>, String> {
    let workspace = Workspace::load(root)?;
    let abi = abi_names(root)?;
    let mut found = Vec::new();
    for package in &workspace.packages {
        let name = package.name.as_str();
        dependency_rules(&workspace, package, &mut found);
        for source in rust::sources(&package.roots)? {
            let path = &source.path;
            let relative = path.strip_prefix(&package.dir).or_else(|_| path.strip_prefix(root)).unwrap_or(path);
            let at = format!("{name}:{}", slashed(relative));
            if GENERIC.contains(&name) {
                for literal in &source.strings {
                    if let Some(shown) = framework_literal(literal) {
                        found.push(format!("1 {at} {shown}"));
                    }
                }
            }
            if name != CLI && name != TOOLING {
                for read in &source.environment {
                    found.push(format!("3 {at} {read}"));
                }
            }
            if name != ABI && name != TOOLING {
                for literal in &source.strings {
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

/// The names the transform emits calls to: the value exports of the runtime's emit entry. Rule 4 skips the one-letter
/// ones, the `n` and `t` namespaces, because a one-letter string is too common to scan for. `cargo xtask abi check`
/// covers them.
fn abi_names(root: &Path) -> Result<BTreeSet<String>, String> {
    ts::value_exports(root, &root.join("packages/crucible/src/emit/index.ts"))
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
    /// The source files of the targets the crate ships: its library, binaries and build script.
    roots: Vec<PathBuf>,
    /// Normal and build dependencies by name. Rule 3 ignores dev-dependencies, because tests are not shipped.
    dependencies: BTreeSet<String>,
    /// Every dependency kind, limited to workspace members.
    workspace_dependencies: BTreeSet<String>,
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
                .arg(root.join("Cargo.toml")),
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
            let roots = package["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|target| {
                    target["kind"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .any(|kind| SHIPPED.contains(&kind))
                })
                .filter_map(|target| target["src_path"].as_str().map(PathBuf::from))
                .collect();
            packages.push(Package {
                id: package["id"].as_str().unwrap_or_default().to_owned(),
                dir: manifest.parent().map(Path::to_path_buf).unwrap_or_default(),
                roots,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn lines(text: &[&str]) -> Vec<String> {
        let mut lines: Vec<String> = text.iter().map(|line| (*line).to_owned()).collect();
        lines.sort();
        lines
    }

    #[test]
    fn the_ratchet_refuses_a_new_line_and_a_higher_count() {
        let dir = std::env::temp_dir().join(format!("xtask-arch-ratchet-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        fs::write(dir.join("README"), "before the list\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "before"]);
        fs::create_dir_all(dir.join("xtask")).unwrap();
        fs::write(dir.join(LIST), render(&lines(&["1 a \"x\"", "1 a \"x\"", "4 b \"y\""]))).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "list"]);
        fs::create_dir_all(dir.join("moved/xtask")).unwrap();
        git(&dir, &["mv", LIST, "moved/xtask/arch.txt"]);
        git(&dir, &["commit", "-q", "-m", "move"]);

        let before = base_list(&dir, "HEAD~2");
        let listed = base_list(&dir, "HEAD~1");
        let moved = base_list(&dir, "HEAD");
        let unknown = base_list(&dir, "no-such-rev");
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(before, Ok(None));
        let listed = listed.unwrap().unwrap();
        assert_eq!(moved, Ok(Some(listed.clone())));
        assert!(unknown.is_err());
        let new = |found: &[&str]| difference(&listed, &lines(found)).0;
        assert!(new(&["1 a \"x\""]).is_empty());
        assert!(new(&["1 a \"x\"", "1 a \"x\"", "4 b \"y\""]).is_empty());
        assert_eq!(new(&["1 a \"x\"", "1 a \"x\"", "1 a \"x\""]), ["1 a \"x\""]);
        assert_eq!(new(&["4 b \"y\"", "4 c \"z\""]), ["4 c \"z\""]);
    }
}
