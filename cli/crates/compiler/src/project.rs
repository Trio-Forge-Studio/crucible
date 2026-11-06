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

pub(crate) struct Game {
    pub rojo: Rojo,
    /// The package root `boot` globs are relative to: the nearest folder at or above `dir` with a `package.json`, or
    /// `dir`.
    pub package: PathBuf,
    root: PathBuf,
    out_dir: PathBuf,
    packages: Vec<Installed>,
    pub runtime_lib: RbxPath,
}

impl Game {
    pub fn load(
        project: &Path,
        include: &Path,
        tsconfig_dir: &Path,
        root: &Path,
        out_dir: Option<&Path>,
    ) -> Result<Game, String> {
        let absolute = |path: &Path| std::path::absolute(path).map(|p| normalize(&p)).map_err(|e| e.to_string());
        let rojo = Rojo::load(&absolute(project)?)?;
        if !rojo.is_game() {
            return Err(format!("{} has no DataModel root, so it is not a place", project.display()));
        }
        let Some(out_dir) = out_dir else {
            return Err("a game needs an outDir in its tsconfig, which its Rojo project maps".to_owned());
        };
        let Some(runtime_lib) = rojo.rbx_path(&absolute(include)?.join(crate::include::RUNTIME_LIB)) else {
            return Err(format!(
                "{} maps no folder for {}, where roblox-ts's runtime library goes",
                project.display(),
                include.display()
            ));
        };
        let package = package_dir(tsconfig_dir).unwrap_or(tsconfig_dir);
        let packages = packages(&package.join("node_modules"));
        Ok(Game {
            rojo,
            package: package.to_path_buf(),
            root: root.to_path_buf(),
            out_dir: normalize(out_dir),
            packages,
            runtime_lib,
        })
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
            Some(path) => luau_path(&runtime_path(&path), true)?,
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

fn runtime_path(file: &Path) -> PathBuf {
    let Some(package) = file.parent().and_then(package_dir) else { return file.to_path_buf() };
    let Some(manifest) = std::fs::read(package.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return file.to_path_buf();
    };
    let types = manifest.get("types").or_else(|| manifest.get("typings")).and_then(|v| v.as_str());
    let main = manifest.get("main").and_then(|v| v.as_str());
    match (types, main) {
        (Some(types), Some(main)) if normalize(&package.join(types)) == normalize(file) => package.join(main),
        _ => file.to_path_buf(),
    }
}

/// How a package reaches `file` when it is in a scoped package under a `node_modules` folder: the package that
/// `TS.getModule` finds by scope and name, then the module's folders and name inside it. The declaration
/// `src/index.d.ts` names the module `src`.
pub(crate) fn installed_module(file: &str) -> Option<ModulePath> {
    let (_, inside) = file.rsplit_once("/node_modules/")?;
    let mut parts = inside.splitn(3, '/');
    let scope = parts.next().filter(|s| s.starts_with('@') && s.len() > 1)?;
    let name = parts.next().filter(|n| !n.is_empty())?;
    let relative = parts.next()?;
    let runtime = runtime_path(Path::new(file)).to_string_lossy().replace('\\', "/");
    let relative =
        if runtime != file { runtime.rsplit_once("/node_modules/")?.1.splitn(3, '/').nth(2)? } else { relative };
    let module = luau_path(Path::new(relative), true)?;
    let mut path: Vec<Box<str>> =
        module.with_extension("").components().map(|c| c.as_os_str().to_string_lossy().into()).collect();
    if path.last().is_some_and(|last| &**last == "init") {
        path.pop();
    }
    Some(ModulePath::Package { scope: scope.into(), name: name.into(), path })
}

/// A TypeScript file's Luau module path: `.ts` and `.tsx` become `.luau`, and an `index` module is its folder's
/// `init`. A declaration also maps to the module it declares when `declaration` is set.
fn luau_path(file: &Path, declaration: bool) -> Option<PathBuf> {
    if declaration && matches!(extension(file), Some("lua" | "luau")) {
        return Some(file.with_extension("luau"));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn package(scope: &str, name: &str, path: &[&str]) -> Option<ModulePath> {
        Some(ModulePath::Package {
            scope: scope.into(),
            name: name.into(),
            path: path.iter().map(|&p| p.into()).collect(),
        })
    }

    #[test]
    fn package_types_entry_resolves_to_its_runtime_main() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/package-main");
        assert_eq!(runtime_path(&root.join("index.d.ts")), root.join("src/init.lua"));
        assert_eq!(runtime_path(&root.join("other.d.ts")), root.join("other.d.ts"));
        assert_eq!(luau_path(&runtime_path(&root.join("index.d.ts")), true), Some(root.join("src/init.luau")));
    }

    #[test]
    fn an_installed_module_is_its_package_then_its_path_inside() {
        let pnpm = "/repo/node_modules/.pnpm/@rbxts+vide@0.6.1/node_modules/@rbxts/vide/src/index.d.ts";
        assert_eq!(installed_module(pnpm), package("@rbxts", "vide", &["src"]), "the last node_modules counts");
        assert_eq!(
            installed_module("/game/node_modules/@scope/lib/out/util/helpers.d.ts"),
            package("@scope", "lib", &["out", "util", "helpers"]),
        );
        assert_eq!(installed_module("/game/node_modules/@scope/lib/index.d.ts"), package("@scope", "lib", &[]));
        assert_eq!(installed_module("/game/node_modules/lib/index.d.ts"), None, "only scoped packages");
        assert_eq!(installed_module("/game/src/util.ts"), None, "not installed");
    }
}
