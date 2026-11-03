//! The Roblox place a game compiles into. A Rojo project maps the output folders and packages to instances, and the
//! emitted code reaches roblox-ts's runtime library and other modules by those instance paths.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::facts::ModulePath;

/// An instance path from the place root: a service, then the names of its descendants.
pub(crate) type RbxPath = Vec<String>;

const PROJECT_FILE: &str = "default.project.json";

struct Partition {
    fs: PathBuf,
    rbx: RbxPath,
}

/// The folder and file mappings of a Rojo project, as `@roblox-ts/rojo-resolver` reads them.
pub(crate) struct Rojo {
    /// Later mappings first, so a nested folder wins over the folder around it.
    partitions: Vec<Partition>,
    files: HashMap<PathBuf, RbxPath>,
    is_game: bool,
}

/// Rojo's default project file in `dir`, else the first other project file there.
pub(crate) fn find_project(dir: &Path) -> Option<PathBuf> {
    let default = dir.join(PROJECT_FILE);
    if default.is_file() {
        return Some(default);
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str());
            name.and_then(|n| n.strip_suffix(".project.json")).is_some_and(|stem| !stem.is_empty())
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

impl Rojo {
    pub fn load(project: &Path) -> Result<Rojo, String> {
        let mut rojo = Rojo { partitions: Vec::new(), files: HashMap::new(), is_game: false };
        let mut rbx = Vec::new();
        rojo.config(project, false, &mut rbx)?;
        rojo.partitions.reverse();
        Ok(rojo)
    }

    pub fn is_game(&self) -> bool {
        self.is_game
    }

    fn config(&mut self, file: &Path, push: bool, rbx: &mut RbxPath) -> Result<(), String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", file.display()))?;
        let (Some(name), Some(tree)) = (json["name"].as_str(), json.get("tree").filter(|t| t.is_object())) else {
            return Err(format!("{} needs a name and a tree", file.display()));
        };
        let base = file.parent().unwrap_or(Path::new("")).to_path_buf();
        self.tree(&base, push.then_some(name), tree, rbx)
    }

    fn tree(
        &mut self,
        base: &Path,
        name: Option<&str>,
        tree: &serde_json::Value,
        rbx: &mut RbxPath,
    ) -> Result<(), String> {
        if let Some(name) = name {
            rbx.push(name.to_owned());
        }
        let path = match &tree["$path"] {
            serde_json::Value::String(path) => Some(path.as_str()),
            serde_json::Value::Object(optional) => optional.get("optional").and_then(|p| p.as_str()),
            _ => None,
        };
        if let Some(path) = path {
            self.path(normalize(&base.join(path)), rbx)?;
        }
        if tree["$className"] == "DataModel" {
            self.is_game = true;
        }
        if let Some(children) = tree.as_object() {
            for (child, subtree) in children {
                if !child.starts_with('$') && subtree.is_object() {
                    self.tree(base, Some(child), subtree, rbx)?;
                }
            }
        }
        if name.is_some() {
            rbx.pop();
        }
        Ok(())
    }

    fn path(&mut self, item: PathBuf, rbx: &mut RbxPath) -> Result<(), String> {
        let item = to_luau(item);
        if matches!(extension(&item), Some("luau" | "json" | "toml")) {
            self.files.insert(item, rbx.clone());
        } else if item.join(PROJECT_FILE).is_file() {
            self.config(&item.join(PROJECT_FILE), false, rbx)?;
        } else {
            self.partitions.push(Partition { fs: item, rbx: rbx.clone() });
        }
        Ok(())
    }

    /// The instance a file or folder becomes. A `.server` or `.client` suffix and an `init` module name the
    /// script's instance, as Rojo does.
    pub fn rbx_path(&self, file: &Path) -> Option<RbxPath> {
        let file = to_luau(normalize(file));
        if let Some(rbx) = self.files.get(&file) {
            return Some(rbx.clone());
        }
        let script = extension(&file) == Some("luau");
        let partition = self.partitions.iter().find(|p| file.starts_with(&p.fs))?;
        let stripped = strip_rojo_extensions(&file);
        let mut parts: Vec<String> = stripped
            .strip_prefix(&partition.fs)
            .ok()?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if script && parts.last().is_some_and(|p| p == "init") {
            parts.pop();
        }
        let mut rbx = partition.rbx.clone();
        rbx.extend(parts);
        Some(rbx)
    }

    fn container(&self, containers: &[&[&str]], rbx: &[String]) -> Option<usize> {
        if !self.is_game {
            return None;
        }
        containers.iter().position(|c| c.iter().zip(rbx).all(|(a, b)| a == b))
    }

    /// How the module at `module` is reached from the script at `source`. A module in the same isolated container
    /// is reached relatively, because the container's contents are copied, and any other from its service.
    pub fn import(&self, source: &[String], module: &[String]) -> Result<ModulePath, &'static str> {
        if self.container(SERVER_CONTAINERS, module).is_some() && self.container(SERVER_CONTAINERS, source).is_none() {
            return Err("a module in a server container cannot be imported from outside one");
        }
        match (self.container(ISOLATED_CONTAINERS, source), self.container(ISOLATED_CONTAINERS, module)) {
            (Some(a), Some(b)) if a == b => Ok(relative(source, module)),
            (_, Some(_)) => Err(
                "a module in an isolated container, such as StarterPlayerScripts, can only be imported from inside it",
            ),
            _ => absolute(module).ok_or("the module has no instance path"),
        }
    }
}

/// A program whose folder holds a Rojo project with a `DataModel` root. Without one the program is a package: its
/// modules reach each other relatively and get the runtime library from `_G[script]`.
pub(crate) struct Game {
    pub rojo: Rojo,
    /// The folder the tsconfig is in, which `boot` globs and `include` are relative to.
    pub dir: PathBuf,
    root: PathBuf,
    out_dir: PathBuf,
    packages: Vec<Installed>,
    pub runtime_lib: RbxPath,
}

impl Game {
    /// `Ok(None)` when the folder holds no project file or the project is not a place.
    pub fn load(dir: &Path, root: &Path, out_dir: Option<&Path>) -> Result<Option<Game>, String> {
        let Some(project_file) = find_project(dir) else { return Ok(None) };
        let rojo = Rojo::load(&project_file)?;
        if !rojo.is_game() {
            return Ok(None);
        }
        let Some(out_dir) = out_dir else {
            return Err("a game needs an outDir in its tsconfig, which its Rojo project maps".to_owned());
        };
        let Some(runtime_lib) = rojo.rbx_path(&dir.join("include/RuntimeLib.luau")) else {
            return Err("the Rojo project maps no folder for include, where roblox-ts's runtime library is".to_owned());
        };
        let packages = packages(&package_dir(dir).unwrap_or(dir).join("node_modules"));
        Ok(Some(Game {
            rojo,
            dir: dir.to_path_buf(),
            root: root.to_path_buf(),
            out_dir: normalize(out_dir),
            packages,
            runtime_lib,
        }))
    }

    /// Where the output of a source file under the source root goes.
    pub fn out_path(&self, source: &Path) -> Option<PathBuf> {
        let relative = source.strip_prefix(&self.root).ok()?;
        Some(self.out_dir.join(luau_path(relative, false)?))
    }

    /// The output folder that a folder under the source root maps to.
    pub fn out_folder(&self, folder: &Path) -> Option<PathBuf> {
        Some(self.out_dir.join(normalize(folder).strip_prefix(&self.root).ok()?))
    }

    /// The instance that the module compiled from, or declared by, `file` becomes.
    pub fn module(&self, file: &Path) -> Option<RbxPath> {
        let out = match self.through_node_modules(file) {
            Some(path) => luau_path(&path, true)?,
            None => self.out_path(file)?,
        };
        self.rojo.rbx_path(&out)
    }

    fn through_node_modules(&self, file: &Path) -> Option<PathBuf> {
        if self.packages.iter().any(|p| file.starts_with(&p.path)) {
            return Some(file.to_path_buf());
        }
        let package = self.packages.iter().find(|p| p.real.as_ref().is_some_and(|real| file.starts_with(real)))?;
        Some(package.path.join(file.strip_prefix(package.real.as_ref()?).ok()?))
    }
}

/// A package in the game's `node_modules`. The program names a linked package's files by their real path.
struct Installed {
    path: PathBuf,
    /// `None` when the link cannot be resolved.
    real: Option<PathBuf>,
}

/// The nearest folder at or above `dir` with a `package.json`.
fn package_dir(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|d| d.join("package.json").is_file())
}

/// Scoped packages only, as roblox-ts allows.
fn packages(node_modules: &Path) -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(scopes) = std::fs::read_dir(node_modules) else { return out };
    for scope in scopes.flatten() {
        if !scope.file_name().to_string_lossy().starts_with('@') {
            continue;
        }
        let Ok(names) = std::fs::read_dir(scope.path()) else { continue };
        for package in names.flatten() {
            let path = package.path();
            out.push(Installed { real: real_path(&path), path });
        }
    }
    out
}

fn real_path(path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    // Windows returns a verbatim `\\?\` path, which never shares a prefix with the program's file names.
    match real.to_str().and_then(|r| r.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with("UNC") => Some(PathBuf::from(plain)),
        _ => Some(real),
    }
}

/// A TypeScript file's Luau module path: `.ts` and `.tsx` become `.luau`, and an `index` module is its folder's
/// `init`. A declaration also maps to the module it declares when `declaration` is set.
fn luau_path(file: &Path, declaration: bool) -> Option<PathBuf> {
    let name = file.file_name()?.to_str()?;
    let stem = name.strip_suffix(".tsx").or_else(|| name.strip_suffix(".ts"))?;
    let stem = match stem.strip_suffix(".d") {
        Some(stem) if declaration => stem,
        Some(_) => return None,
        None => stem,
    };
    let stem = match stem.split_once('.') {
        Some(("index", rest)) => format!("init.{rest}"),
        _ if stem == "index" => "init".to_owned(),
        _ => stem.to_owned(),
    };
    Some(file.with_file_name(format!("{stem}.luau")))
}

const ISOLATED_CONTAINERS: &[&[&str]] = &[
    &["StarterPack"],
    &["StarterGui"],
    &["StarterPlayer", "StarterPlayerScripts"],
    &["StarterPlayer", "StarterCharacterScripts"],
    &["StarterPlayer", "StarterCharacter"],
    &["PluginDebugService"],
];

const SERVER_CONTAINERS: &[&[&str]] = &[&["ServerStorage"], &["ServerScriptService"]];

fn relative(from: &[String], to: &[String]) -> ModulePath {
    let common = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    ModulePath::Relative {
        parents: (from.len() - common) as u32,
        path: to[common..].iter().map(|s| s.as_str().into()).collect(),
    }
}

pub(crate) fn absolute(to: &[String]) -> Option<ModulePath> {
    let (service, path) = to.split_first()?;
    Some(ModulePath::Absolute {
        service: service.as_str().into(),
        path: path.iter().map(|s| s.as_str().into()).collect(),
    })
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|e| e.to_str())
}

fn to_luau(path: PathBuf) -> PathBuf {
    if extension(&path) == Some("lua") { path.with_extension("luau") } else { path }
}

fn strip_rojo_extensions(path: &Path) -> PathBuf {
    match extension(path) {
        Some("luau") => {
            let stem = path.with_extension("");
            match extension(&stem) {
                Some("server" | "client") => stem.with_extension(""),
                _ => stem,
            }
        }
        Some("json" | "toml") => path.with_extension(""),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}
