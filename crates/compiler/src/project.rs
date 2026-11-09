//! The Roblox place a game compiles into. An instance map places the output folders and packages, and the emitted
//! code reaches roblox-ts's runtime library and other modules by those instance paths.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::facts::ModulePath;

/// An instance path from the place root: a service, then the names of its descendants.
pub(crate) type RbxPath = Vec<String>;

/// The instances a place's folders and files become, such as a Rojo project maps them. A file in a mapped folder
/// becomes the instance that Rojo would sync it to.
#[derive(Clone, Debug)]
pub struct InstanceMap {
    source: String,
    folders: Vec<(PathBuf, RbxPath)>,
    files: HashMap<PathBuf, RbxPath>,
}

impl InstanceMap {
    /// `source` names the map in messages, such as the project file it was read from. Each instance path starts at
    /// the place root, and each path is absolute. A later folder wins over an earlier one that holds it.
    pub fn new(
        source: impl Into<String>,
        folders: impl IntoIterator<Item = (PathBuf, Vec<String>)>,
        files: impl IntoIterator<Item = (PathBuf, Vec<String>)>,
    ) -> InstanceMap {
        InstanceMap {
            source: source.into(),
            folders: folders.into_iter().map(|(folder, rbx)| (normalize(&folder), rbx)).collect(),
            files: files.into_iter().map(|(file, rbx)| (to_luau(normalize(&file)), rbx)).collect(),
        }
    }

    /// The instance a file or folder becomes. A `.server` or `.client` suffix and an `init` module name the
    /// script's instance, as Rojo does.
    pub fn rbx_path(&self, file: &Path) -> Option<RbxPath> {
        let file = to_luau(normalize(file));
        if let Some(rbx) = self.files.get(&file) {
            return Some(rbx.clone());
        }
        let script = extension(&file) == Some("luau");
        let (folder, folder_rbx) = self.folders.iter().rev().find(|(folder, _)| file.starts_with(folder))?;
        let stripped = strip_rojo_extensions(&file);
        let mut parts: Vec<String> = stripped
            .strip_prefix(folder)
            .ok()?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if script && parts.last().is_some_and(|p| p == "init") {
            parts.pop();
        }
        let mut rbx = folder_rbx.clone();
        rbx.extend(parts);
        Some(rbx)
    }

    fn container(containers: &[&[&str]], rbx: &[String]) -> Option<usize> {
        containers.iter().position(|c| c.iter().zip(rbx).all(|(a, b)| a == b))
    }

    /// How the module at `module` is reached from the script at `source`. A module in the same isolated container
    /// is reached relatively, because the container's contents are copied, and any other from its service.
    pub fn import(&self, source: &[String], module: &[String]) -> Result<ModulePath, &'static str> {
        if Self::container(SERVER_CONTAINERS, module).is_some() && Self::container(SERVER_CONTAINERS, source).is_none()
        {
            return Err("a module in a server container cannot be imported from outside one");
        }
        match (Self::container(ISOLATED_CONTAINERS, source), Self::container(ISOLATED_CONTAINERS, module)) {
            (Some(a), Some(b)) if a == b => Ok(relative(source, module)),
            (_, Some(_)) => Err(
                "a module in an isolated container, such as StarterPlayerScripts, can only be imported from inside it",
            ),
            _ => absolute(module).ok_or("the module has no instance path"),
        }
    }
}

pub(crate) struct Game {
    pub instances: InstanceMap,
    root: PathBuf,
    out_dir: PathBuf,
    packages: Vec<Installed>,
    pub runtime_lib: RbxPath,
}

impl Game {
    pub fn load(
        instances: &InstanceMap,
        include: &Path,
        tsconfig_dir: &Path,
        root: &Path,
        out_dir: Option<&Path>,
    ) -> Result<Game, String> {
        let Some(out_dir) = out_dir else {
            return Err("a game needs an outDir in its tsconfig, which its Rojo project maps".to_owned());
        };
        let include_dir = std::path::absolute(include).map(|p| normalize(&p)).map_err(|e| e.to_string())?;
        let Some(runtime_lib) = instances.rbx_path(&include_dir.join(crate::include::RUNTIME_LIB)) else {
            return Err(format!(
                "{} maps no folder for {}, where roblox-ts's runtime library goes",
                instances.source,
                include.display()
            ));
        };
        let package = package_dir(tsconfig_dir).unwrap_or(tsconfig_dir);
        let packages = packages(&package.join("node_modules"));
        let instances = instances.clone();
        Ok(Game { instances, root: root.to_path_buf(), out_dir: normalize(out_dir), packages, runtime_lib })
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
        self.instances.rbx_path(&out)
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
