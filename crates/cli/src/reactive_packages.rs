use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crucible_project::rojo::{self, Folder, Ignore};
use serde_json::Value;

const SINGLETONS: [&str; 3] = ["@rbxts/charm", "@rbxts/vide", "@rbxts/charm-sync"];
const FIELDS: [&str; 2] = ["dependencies", "peerDependencies"];

/// The version range a dependency entry for `name` declares, or None when the entry names another source, such as a
/// path, a link, a Git or URL spec, a tag, a pnpm catalog or a bare workspace protocol. Package managers judge those by
/// the installed copy's own version, so the check does too. An `npm:` alias of the same package and a `workspace:` entry
/// with a range keep the range they hold.
fn declared_range(name: &str, spec: &str) -> Option<node_semver::Range> {
    let spec = spec.trim();
    let range = if let Some(alias) = spec.strip_prefix("npm:") {
        let at = alias.rfind('@').filter(|&at| at > 0)?;
        if alias[..at] != *name {
            return None;
        }
        &alias[at + 1..]
    } else if let Some(range) = spec.strip_prefix("workspace:") {
        range
    } else {
        spec
    };
    if range.is_empty() || matches!(range, "^" | "~") {
        return None;
    }
    node_semver::Range::parse(range).ok()
}

/// What the check reads from a package.json.
struct Manifest {
    name: Option<String>,
    version: Option<String>,
    /// Each entry for a library in `SINGLETONS`, as its field, the library and the entry.
    entries: Vec<(&'static str, &'static str, String)>,
}

impl Manifest {
    fn label(&self) -> String {
        format!("{} {}", self.name.as_deref().unwrap_or("package"), self.version.as_deref().unwrap_or("unknown"))
    }
}

/// A package folder at one instance of the place.
struct Package {
    path: PathBuf,
    manifest: Arc<Manifest>,
    /// The package whose node_modules holds this one in the place.
    parent: Option<Arc<Manifest>>,
}

type Listing = Arc<[(String, fs::FileType)]>;

/// Checks places, keeping what it reads from disk for the next place.
#[derive(Default)]
pub struct Checker {
    disk: Disk,
}

/// What the check has read from disk. Folders are walked on several threads, which share it.
#[derive(Default)]
struct Disk {
    manifests: Mutex<HashMap<PathBuf, Arc<Manifest>>>,
    listings: Mutex<HashMap<PathBuf, Option<Listing>>>,
    /// Whether a folder above a mapped folder holds a package.json.
    packages: Mutex<HashMap<PathBuf, bool>>,
}

impl Checker {
    /// Checks the place that `project` builds. It reads only what the place contains, the folders its Rojo tree reaches
    /// and the package.json of each package in them, and the game's own package.json.
    pub fn check(&mut self, root: &Path, project: &Path) -> Result<(), String> {
        let root = std::path::absolute(root).map(|root| lexical(&root)).map_err(|e| e.to_string())?;
        let packages = self.disk.walk(&root, &rojo::folders(project, &[], &Ignore::default())?)?;
        let mut copies: BTreeMap<&str, BTreeMap<&[String], &Package>> = BTreeMap::new();
        for (instance, package) in &packages {
            if let Some(name) = package.manifest.name.as_deref().and_then(|n| SINGLETONS.iter().find(|&&s| s == n)) {
                copies.entry(name).or_default().insert(instance, package);
            }
        }
        if copies.is_empty() {
            return Ok(());
        }
        let mut problems = Vec::new();
        for (name, at) in &copies {
            if at.len() > 1 {
                problems.push(Problem::Copies(name, at.values().copied().collect()));
            }
        }
        // Each declared entry with the copy that the declaring package, or the game when None, loads in the place.
        let mut declared: Vec<(&Package, Option<&Package>, &str, &str, &str)> = Vec::new();
        for (instance, declarer) in &packages {
            for (field, name, spec) in &declarer.manifest.entries {
                if let Some(copy) = copies.get(name).and_then(|at| loaded(at, instance, name)) {
                    declared.push((copy, Some(declarer), field, name, spec));
                }
            }
        }
        let game = self.disk.game(&root)?;
        if let Some(game) = &game {
            for (field, name, spec) in &game.manifest.entries {
                // The game's own code loads the copy its node_modules resolves to, wherever the place maps it.
                let resolve = |dir: &Path| fs::canonicalize(dir.join("node_modules").join(name)).ok();
                let Some(real) = root.ancestors().find_map(resolve) else { continue };
                let copies = copies.get(name).into_iter().flat_map(|at| at.values());
                for copy in copies.filter(|copy| fs::canonicalize(&copy.path).is_ok_and(|path| path == real)) {
                    declared.push((copy, None, field, name, spec));
                }
            }
        }
        let mut outside: BTreeMap<(&str, &Path), (&Package, Vec<String>)> = BTreeMap::new();
        for (copy, declarer, field, name, spec) in declared {
            let Some(range) = declared_range(name, spec) else { continue };
            let version = copy.manifest.version.as_deref().and_then(|v| node_semver::Version::parse(v).ok());
            if version.is_some_and(|version| !range.satisfies(&version)) {
                let by = match declarer {
                    Some(by) => {
                        format!("{field} range {spec} of {} at {}", by.manifest.label(), shown(&root, &by.path))
                    }
                    None => format!("{field} range {spec} of the game's package.json"),
                };
                let (_, ranges) = outside.entry((name, &copy.path)).or_insert_with(|| (copy, Vec::new()));
                if !ranges.contains(&by) {
                    ranges.push(by);
                }
            }
        }
        problems.extend(outside.into_iter().map(|((name, _), (copy, ranges))| Problem::Outside(name, copy, ranges)));
        if problems.is_empty() {
            return Ok(());
        }
        let agent = std::env::var("npm_config_user_agent").unwrap_or_default();
        let manager = crate::upgrade::detect_manager(&root, &agent).program();
        Err(problems.iter().map(|problem| problem.describe(&root, manager)).collect::<Vec<_>>().join("\n"))
    }
}

enum Problem<'a> {
    /// More than one copy of a library in the place.
    Copies(&'a str, Vec<&'a Package>),
    /// A copy outside ranges that packages loading it declare.
    Outside(&'a str, &'a Package, Vec<String>),
}

impl Problem<'_> {
    fn describe(&self, root: &Path, manager: &str) -> String {
        let version = |copy: &Package| copy.manifest.version.clone().unwrap_or_else(|| "unknown version".to_owned());
        match self {
            Self::Copies(name, copies) => {
                let mut text = format!("multiple copies of {name} in the place split its state:");
                for copy in copies {
                    text += &format!("\n  {} at {}", version(copy), shown(root, &copy.path));
                    if let Some(parent) = &copy.parent {
                        text += &format!(", installed for {}", parent.label());
                    }
                }
                let parents: BTreeSet<_> = copies.iter().filter_map(|c| c.parent.as_ref()?.name.as_deref()).collect();
                let parents: Vec<_> = parents.into_iter().collect();
                let agree = copies.windows(2).all(|pair| pair[0].manifest.version == pair[1].manifest.version);
                text += &match parents.as_slice() {
                    // A linked workspace package brings its own install, so installing again changes nothing.
                    [_, ..] if agree => format!(
                        "\nThe versions already agree, so keep the node_modules of {} out of the place: map only its \
                         built folder, such as `out`, or list that node_modules under `globIgnorePaths` in the Rojo project.",
                        parents.join(" and ")
                    ),
                    [] => format!(
                        "\nKeep one of these folders in the place: remove the other's mapping from the Rojo project, or \
                         the dependency that installs it and run `{manager} install`."
                    ),
                    [parent] => format!(
                        "\nMake the versions agree so {manager} installs one copy: give {name} a version that {parent} \
                         accepts, or upgrade {parent}, then run `{manager} install`."
                    ),
                    parents => format!(
                        "\nMake the versions agree so {manager} installs one copy: give {name} a version that {} accept, \
                         or upgrade them, then run `{manager} install`.",
                        parents.join(" and ")
                    ),
                };
                text
            }
            Self::Outside(name, copy, ranges) => {
                let (range, packages) = match ranges.len() {
                    1 => ("a range", "the package that declares it"),
                    _ => ("ranges", "the packages that declare them"),
                };
                let mut text = format!(
                    "{name} {} at {} does not satisfy {range} declared for it:",
                    version(copy),
                    shown(root, &copy.path)
                );
                for by in ranges {
                    text += &format!("\n  {by}");
                }
                text + &format!(
                    "\nChange the game's {name} to a version inside {}, or upgrade {packages}, then run `{manager} install`.",
                    if ranges.len() == 1 { "it" } else { "them" },
                )
            }
        }
    }
}

/// `path` relative to the game, with forward slashes, as messages show it.
fn shown(root: &Path, path: &Path) -> String {
    let path = lexical(path);
    let common = root.components().zip(path.components()).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return path.to_string_lossy().replace('\\', "/");
    }
    let up = root.components().count() - common;
    let parts: Vec<String> = std::iter::repeat_n("..".to_owned(), up)
        .chain(path.components().skip(common).map(|c| c.as_os_str().to_string_lossy().into_owned()))
        .collect();
    if parts.is_empty() { ".".to_owned() } else { parts.join("/") }
}

impl Disk {
    /// Every package the place's folders contain, by its instance. A folder reached twice at one instance is one
    /// package, the first in the tree's order.
    fn walk(&self, root: &Path, folders: &[Folder]) -> Result<BTreeMap<Vec<String>, Package>, String> {
        let next = AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8).min(folders.len());
        let mut walked = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut walked = Vec::new();
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(folder) = folders.get(index) else { break };
                            let mut walk = Walk { disk: self, root, packages: Vec::new(), links: Vec::new() };
                            walked.push((index, walk.folder(folder).map(|()| walk.packages)));
                        }
                        walked
                    })
                })
                .collect();
            workers.into_iter().flat_map(|worker| worker.join().expect("a walk does not panic")).collect::<Vec<_>>()
        });
        walked.sort_by_key(|(index, _)| *index);
        let mut packages = BTreeMap::new();
        for (_, found) in walked {
            for (instance, package) in found? {
                packages.entry(instance).or_insert(package);
            }
        }
        Ok(packages)
    }

    fn manifest(&self, root: &Path, file: &Path) -> Result<Arc<Manifest>, String> {
        if let Some(manifest) = self.manifests.lock().unwrap().get(file) {
            return Ok(manifest.clone());
        }
        let read = "The place contains this package, so the build reads its package.json to check its Charm, Vide and \
                    Charm Sync";
        let text = fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}. {read}", shown(root, file)))?;
        // npm accepts a byte order mark.
        let value: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|e| format!("{} is not valid JSON: {e}. {read}", shown(root, file)))?;
        if !value.is_object() {
            return Err(format!("{} is not a JSON object. {read}", shown(root, file)));
        }
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        let mut entries = Vec::new();
        for field in FIELDS {
            for name in SINGLETONS {
                if let Some(spec) = value[field][name].as_str() {
                    entries.push((field, name, spec.to_owned()));
                }
            }
        }
        let manifest = Arc::new(Manifest { name: text("name"), version: text("version"), entries });
        self.manifests.lock().unwrap().insert(file.to_path_buf(), manifest.clone());
        Ok(manifest)
    }

    fn game(&self, root: &Path) -> Result<Option<Package>, String> {
        let file = root.join("package.json");
        if !file.is_file() {
            return Ok(None);
        }
        Ok(Some(Package { path: root.to_path_buf(), manifest: self.manifest(root, &file)?, parent: None }))
    }

    /// The entries of a folder the place maps, or None when it is not a folder.
    fn list(&self, root: &Path, dir: &Path) -> Result<Option<Listing>, String> {
        if let Some(listing) = self.listings.lock().unwrap().get(dir) {
            return Ok(listing.clone());
        }
        let listing = match fs::read_dir(dir) {
            Ok(entries) => {
                let mut children = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|e| format!("cannot read {}: {e}", shown(root, dir)))?;
                    let kind =
                        entry.file_type().map_err(|e| format!("cannot read {}: {e}", shown(root, &entry.path())))?;
                    children.push((entry.file_name().to_string_lossy().into_owned(), kind));
                }
                Some(Arc::from(children))
            }
            Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => None,
            Err(error) => return Err(format!("cannot read {}: {error}", shown(root, dir))),
        };
        self.listings.lock().unwrap().insert(dir.to_path_buf(), listing.clone());
        Ok(listing)
    }

    /// The package around a mapped folder that is not one itself, below the game and outside any node_modules folder.
    fn owner(&self, root: &Path, folder: &Path) -> Option<PathBuf> {
        let above = |dir: &&Path| !root.starts_with(dir) && dir.file_name().is_some_and(|name| name != "node_modules");
        for dir in folder.ancestors().skip(1).take_while(above) {
            let known = self.packages.lock().unwrap().get(dir).copied();
            let manifest = known.unwrap_or_else(|| dir.join("package.json").is_file());
            self.packages.lock().unwrap().insert(dir.to_path_buf(), manifest);
            if manifest {
                return Some(dir.to_path_buf());
            }
        }
        None
    }
}

/// The copy of `name` that a package at instance `from` loads: the nearest `node_modules` copy up the instance tree, as
/// roblox-ts's `TS.getModule` finds it, or else a sibling, as Luau packages such as vide-charm and charm-sync require it.
fn loaded<'p>(copies: &BTreeMap<&[String], &'p Package>, from: &[String], name: &str) -> Option<&'p Package> {
    let parts: Vec<String> = name.split('/').map(str::to_owned).collect();
    for end in (0..=from.len()).rev() {
        let base = &from[..end];
        if base.last().is_some_and(|last| last == "node_modules") {
            continue;
        }
        let at: Vec<String> =
            base.iter().cloned().chain(["node_modules".to_owned()]).chain(parts.iter().cloned()).collect();
        if let Some(copy) = copies.get(at.as_slice()) {
            return Some(copy);
        }
    }
    let (_, parent) = from.split_last()?;
    let at: Vec<String> = parent.iter().chain(parts.last()).cloned().collect();
    copies.get(at.as_slice()).copied()
}

struct Walk<'a> {
    disk: &'a Disk,
    root: &'a Path,
    /// Each package the folder contains, with its instance, in the tree's order.
    packages: Vec<(Vec<String>, Package)>,
    /// The real paths of the links being walked through, so a link back into one of them ends the walk there.
    links: Vec<PathBuf>,
}

impl Walk<'_> {
    fn folder(&mut self, folder: &Folder) -> Result<(), String> {
        let links = std::mem::take(&mut self.links);
        let result = self.dir(&folder.path, &mut folder.instance.clone(), &folder.ignore, None, true);
        self.links = links;
        result
    }

    fn dir(
        &mut self,
        dir: &Path,
        instance: &mut Vec<String>,
        ignore: &Ignore,
        parent: Option<&Arc<Manifest>>,
        top: bool,
    ) -> Result<(), String> {
        let Some(children) = self.disk.list(self.root, dir)? else { return Ok(()) };
        if children.iter().any(|(name, _)| name == crucible_project::DEFAULT_PROJECT) {
            for folder in rojo::folders(&dir.join(crucible_project::DEFAULT_PROJECT), instance, ignore)? {
                self.folder(&folder)?;
            }
            return Ok(());
        }
        let mut package = None;
        if children.iter().any(|(name, kind)| name == "package.json" && !kind.is_dir()) {
            let manifest = self.disk.manifest(self.root, &dir.join("package.json"))?;
            package = Some(manifest.clone());
            let parent = parent.cloned();
            self.packages.push((instance.clone(), Package { path: dir.to_path_buf(), manifest, parent }));
        } else if top && let Some(owner) = self.disk.owner(self.root, dir) {
            // A mapping of a package's `out` or `src` folder puts that package's code in the place.
            let manifest = self.disk.manifest(self.root, &owner.join("package.json"))?;
            self.packages.push((instance.clone(), Package { path: owner, manifest, parent: None }));
        }
        let parent = package.as_ref().or(parent);
        let package = package.is_some();
        for (name, kind) in children.iter() {
            // Below a package, only its node_modules can hold another package.
            if package && name != "node_modules" {
                continue;
            }
            let path = dir.join(name);
            if ignore.ignores(&path) {
                continue;
            }
            if kind.is_symlink() {
                let Ok(target) = fs::canonicalize(&path) else { continue };
                if self.links.contains(&target) {
                    continue;
                }
                self.links.push(target);
                instance.push(name.clone());
                let result = self.dir(&path, instance, ignore, parent, false);
                instance.pop();
                self.links.pop();
                result?;
            } else if kind.is_dir() {
                instance.push(name.clone());
                let result = self.dir(&path, instance, ignore, parent, false);
                instance.pop();
                result?;
            } else if let Some(stem) = name.strip_suffix(".project.json").filter(|_| !package) {
                instance.push(stem.to_owned());
                let folders = rojo::folders(&path, instance, ignore);
                instance.pop();
                for folder in folders? {
                    self.folder(&folder)?;
                }
            }
        }
        Ok(())
    }
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            component => out.push(component),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_show_paths_from_the_game() {
        let root = std::path::absolute("games/a").unwrap();
        assert_eq!(shown(&root, &root.join("node_modules/@rbxts/charm")), "node_modules/@rbxts/charm");
        assert_eq!(shown(&root, &root.join("../../vendor/@rbxts/vide")), "../../vendor/@rbxts/vide");
        assert_eq!(shown(&root, &root), ".");
    }
}
