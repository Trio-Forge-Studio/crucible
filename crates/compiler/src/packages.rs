//! Which npm package owns a program file, and the file's module path inside that package. Extensions compute type ids
//! and entry points from it, and the compiler reads it for its own rules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tsrs_tsoptions as tsoptions;

use crate::program::ConfigHost;
use crate::project::normalize;

pub struct Package {
    pub dir: PathBuf,
    pub json: serde_json::Value,
}

impl Package {
    pub fn name(&self) -> Option<&str> {
        self.json["name"].as_str()
    }
}

pub struct Packages {
    packages: HashMap<PathBuf, Option<Rc<Package>>>,
    configs: HashMap<PathBuf, Option<PathBuf>>,
    roots: HashMap<PathBuf, Option<PathBuf>>,
    root_dir: Option<PathBuf>,
    host: ConfigHost,
    /// A `package.json` above a program file does not parse.
    invalid_metadata: bool,
}

impl Packages {
    pub(crate) fn new(root_dir: Option<PathBuf>, host: ConfigHost) -> Packages {
        Packages {
            packages: HashMap::new(),
            configs: HashMap::new(),
            roots: HashMap::new(),
            root_dir,
            host,
            invalid_metadata: false,
        }
    }

    pub fn invalid_metadata(&self) -> bool {
        self.invalid_metadata
    }

    pub fn package_of(&mut self, file: &Path) -> Option<Rc<Package>> {
        let mut dir = file.parent()?.to_path_buf();
        let mut visited = Vec::new();
        let found = loop {
            if let Some(cached) = self.packages.get(&dir) {
                break cached.clone();
            }
            visited.push(dir.clone());
            let manifest = dir.join("package.json");
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                let json = serde_json::from_str(&text).unwrap_or_else(|_| {
                    self.invalid_metadata = true;
                    serde_json::Value::Null
                });
                break Some(Rc::new(Package { dir: dir.clone(), json }));
            }
            if !dir.pop() {
                break None;
            }
        };
        for dir in visited {
            self.packages.insert(dir, found.clone());
        }
        found
    }

    fn config_of(&mut self, file: &Path) -> Option<PathBuf> {
        let mut dir = file.parent()?.to_path_buf();
        let mut visited = Vec::new();
        let found = loop {
            if let Some(cached) = self.configs.get(&dir) {
                break cached.clone();
            }
            visited.push(dir.clone());
            let config = dir.join("tsconfig.json");
            if config.is_file() {
                break Some(config);
            }
            if !dir.pop() {
                break None;
            }
        };
        for dir in visited {
            self.configs.insert(dir, found.clone());
        }
        found
    }

    fn config_root(&mut self, config: &Path) -> Option<PathBuf> {
        if let Some(root) = self.roots.get(config) {
            return root.clone();
        }
        let (parsed, _) =
            tsoptions::get_parsed_command_line_of_config_file(&config.to_string_lossy(), None, None, &self.host, None);
        let root = parsed.and_then(|parsed| {
            let options = parsed.compiler_options()?;
            let base = config.parent()?;
            let dirs = parsed
                .file_names()
                .iter()
                .map(Path::new)
                .filter(|file| !is_declaration(file))
                .filter_map(|file| file.parent().map(Path::to_path_buf))
                .collect();
            let root_dirs: Vec<PathBuf> =
                options.root_dirs.iter().flatten().map(|dir| normalize(&base.join(dir))).collect();
            let root_dir = (!options.root_dir.is_empty()).then(|| normalize(&base.join(&options.root_dir)));
            source_root(root_dir, root_dirs, dirs)
        });
        self.roots.insert(config.to_path_buf(), root.clone());
        root
    }

    fn source_root(&mut self, owner: &Package, file: &Path) -> PathBuf {
        let root = match self.config_of(file) {
            Some(config) if self.package_of(&config).is_some_and(|p| p.dir == owner.dir) => self.config_root(&config),
            _ => None,
        };
        root.or_else(|| self.root_dir.clone()).unwrap_or_else(|| owner.dir.clone())
    }

    /// The owning package and the module path inside it. Declarations and the sources they were built from have the
    /// same module path.
    pub fn module_path(&mut self, file: &Path) -> Option<(Rc<Package>, String)> {
        let owner = self.package_of(file)?;
        owner.json["name"].as_str()?;
        let root = if is_declaration(file) { declaration_root(&owner, file)? } else { self.source_root(&owner, file) };
        let relative = file.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
        let module = [".d.tsx", ".d.ts", ".tsx", ".ts"].iter().find_map(|ext| relative.strip_suffix(ext))?.to_owned();
        Some((owner, module))
    }

    /// The module path of the package's `entry` export, such as `.` or `./server`.
    pub fn entry_module(&mut self, owner: &Package, entry: &str) -> Option<String> {
        let exports = &owner.json["exports"];
        let mut types = Vec::new();
        types_condition(if entry == "." && exports.get(".").is_none() { exports } else { &exports[entry] }, &mut types);
        let types = types.into_iter().next().or_else(|| (entry == ".").then(|| package_types(owner)))?;
        self.module_path(&normalize(&owner.dir.join(types))).map(|(_, module)| module)
    }
}

fn is_declaration(file: &Path) -> bool {
    let name = file.to_string_lossy();
    name.ends_with(".d.ts") || name.ends_with(".d.tsx")
}

fn package_types(owner: &Package) -> String {
    owner.json["types"].as_str().or(owner.json["typings"].as_str()).unwrap_or("index.d.ts").to_owned()
}

fn declaration_root(owner: &Package, file: &Path) -> Option<PathBuf> {
    let exports = &owner.json["exports"];
    let mut entries = Vec::new();
    types_condition(if exports.get(".").is_some() { &exports["."] } else { exports }, &mut entries);
    let entry = entries
        .into_iter()
        .find(|entry| owner.dir.join(entry).parent().is_some_and(|dir| file.starts_with(normalize(dir))))
        .unwrap_or_else(|| package_types(owner));
    Some(normalize(owner.dir.join(entry).parent()?))
}

fn source_root(root_dir: Option<PathBuf>, root_dirs: Vec<PathBuf>, mut dirs: Vec<PathBuf>) -> Option<PathBuf> {
    if root_dir.is_some() {
        return root_dir;
    }
    dirs.extend(root_dirs);
    let mut root = dirs.first()?.clone();
    while !dirs.iter().all(|dir| dir.starts_with(&root)) {
        if !root.pop() {
            return None;
        }
    }
    Some(root)
}

/// The declaration file of the package's `entry` export.
pub fn entry_types(owner: &Package, entry: &str) -> Option<PathBuf> {
    let mut types = Vec::new();
    types_condition(&owner.json["exports"][entry], &mut types);
    types.first().map(|t| normalize(&owner.dir.join(t)))
}

fn types_condition(value: &serde_json::Value, out: &mut Vec<String>) {
    let Some(map) = value.as_object() else { return };
    match map.get("types").and_then(|t| t.as_str()) {
        Some(types) => out.push(types.to_owned()),
        None => map.values().for_each(|v| types_condition(v, out)),
    }
}
