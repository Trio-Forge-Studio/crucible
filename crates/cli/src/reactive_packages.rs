use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const SINGLETONS: [&str; 3] = ["@rbxts/charm", "@rbxts/vide", "@rbxts/charm-sync"];
const FIX: &str = "Align the declared ranges and use npm overrides or pnpm.overrides followed by reinstalling, then map only one package directory for each library.";

#[derive(Default)]
struct Scan {
    manifests: BTreeMap<PathBuf, Value>,
    copies: BTreeMap<String, BTreeMap<PathBuf, PathBuf>>,
    visited: BTreeSet<PathBuf>,
    packages: BTreeMap<PathBuf, Option<PathBuf>>,
}

fn read(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

impl Scan {
    fn manifest(&mut self, dir: &Path, mapped: bool) -> Result<bool, String> {
        let real = if let Some(cached) = self.packages.get(dir) {
            let Some(real) = cached else { return Ok(false) };
            real.clone()
        } else {
            let Some(value) = read(&dir.join("package.json"))? else {
                self.packages.insert(dir.to_path_buf(), None);
                return Ok(false);
            };
            let real = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
            self.manifests.insert(real.clone(), value);
            self.packages.insert(dir.to_path_buf(), Some(real.clone()));
            real
        };
        if mapped && let Some(name) = self.manifests[&real]["name"].as_str().filter(|name| SINGLETONS.contains(name)) {
            self.copies.entry(name.to_owned()).or_default().insert(real, dir.to_path_buf());
        }
        Ok(true)
    }

    fn mapped(&mut self, dir: &Path) -> Result<(), String> {
        let metadata = match fs::symlink_metadata(dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", dir.display())),
        };
        let real = if metadata.file_type().is_symlink() {
            dir.canonicalize().map_err(|e| e.to_string())?
        } else {
            if !metadata.is_dir() {
                return Ok(());
            }
            dir.to_path_buf()
        };
        if !self.visited.insert(real) {
            return Ok(());
        }
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let package = if entries.iter().any(|e| e.file_name() == "package.json") {
            self.manifest(dir, true)?
        } else {
            self.packages.insert(dir.to_path_buf(), None);
            false
        };
        if entries.iter().any(|e| e.file_name() == "default.project.json") {
            let mappings = crucible_project::rojo::project(&dir.join("default.project.json"))?;
            for (folder, _) in mappings.folders {
                self.mapped(&folder)?;
            }
            return Ok(());
        }
        for entry in entries {
            if package && entry.file_name() != "node_modules" {
                continue;
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()) {
                self.mapped(&entry.path())?;
            }
        }
        Ok(())
    }

    fn installed(&mut self, modules: &Path, seen: &mut BTreeSet<PathBuf>) -> Result<(), String> {
        if !modules.is_dir() {
            return Ok(());
        }
        let real = modules.canonicalize().map_err(|e| e.to_string())?;
        if !seen.insert(real) {
            return Ok(());
        }
        for entry in fs::read_dir(modules).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if !kind.is_dir() && !(kind.is_symlink() && entry.path().is_dir()) {
                continue;
            }
            if name.starts_with('@') {
                self.installed(&entry.path(), seen)?;
            } else if self.manifest(&entry.path(), false)? {
                self.installed(&entry.path().join("node_modules"), seen)?;
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Checker {
    scan: Scan,
    installed: BTreeSet<PathBuf>,
    done: BTreeSet<PathBuf>,
}

impl Checker {
    pub fn check(&mut self, root: &Path, project: &Path) -> Result<(), String> {
        let mappings = crucible_project::rojo::place(project)?;
        let scan = &mut self.scan;
        scan.copies.clear();
        scan.visited.clear();
        for (folder, _) in &mappings.folders {
            if !folder.is_dir() {
                continue;
            }
            scan.mapped(folder)?;
            // A mapping of package/out or package/src still uses that package's manifest.
            for ancestor in folder.ancestors() {
                if scan.manifest(ancestor, true)? {
                    break;
                }
            }
        }
        if scan.copies.is_empty() {
            return Ok(());
        }
        let installed = &mut self.installed;
        scan.installed(&root.join("node_modules"), installed)?;
        // pnpm places dependencies beside the real package in its virtual store.
        let done = &mut self.done;
        loop {
            let pending: Vec<_> = scan.manifests.keys().filter(|p| !done.contains(*p)).cloned().collect();
            if pending.is_empty() {
                break;
            }
            for package in pending {
                done.insert(package.clone());
                if let Some(modules) = package.ancestors().find(|p| p.file_name().is_some_and(|n| n == "node_modules"))
                {
                    scan.installed(modules, installed)?;
                }
            }
        }
        let mut errors = Vec::new();
        for (name, copies) in &scan.copies {
            let describe = |real: &Path, path: &Path| {
                format!("{} at {}", scan.manifests[real]["version"].as_str().unwrap_or("unknown"), path.display())
            };
            if copies.len() > 1 {
                errors.push(format!(
                    "multiple copies of {name}:\n  {}",
                    copies.iter().map(|(real, path)| describe(real, path)).collect::<Vec<_>>().join("\n  ")
                ));
            }
            for (real, path) in copies {
                let version = scan.manifests[real]["version"].as_str().unwrap_or("");
                let version = node_semver::Version::parse(version).map_err(|e| format!("{}: {e}", path.display()))?;
                for (owner, manifest) in &scan.manifests {
                    for field in ["dependencies", "peerDependencies"] {
                        if let Some(range) = manifest[field][name].as_str() {
                            let parsed = node_semver::Range::parse(range).map_err(|e| {
                                format!("{} declares unsupported {name} range {range}: {e}", owner.display())
                            })?;
                            if !parsed.satisfies(&version) {
                                errors.push(format!(
                                    "{name} {} does not satisfy {} {} at {}: {field} range {range}",
                                    describe(real, path),
                                    manifest["name"].as_str().unwrap_or("package"),
                                    manifest["version"].as_str().unwrap_or("unknown"),
                                    owner.display()
                                ));
                            }
                        }
                    }
                }
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(format!("{}\n{FIX}", errors.join("\n"))) }
    }
}
