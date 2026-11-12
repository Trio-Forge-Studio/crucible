//! The folders and files a Rojo project maps into a place, as `@roblox-ts/rojo-resolver` reads them.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::DEFAULT_PROJECT;

/// What a place's Rojo project maps, each path with the instance it becomes, named from the place root. Paths are
/// absolute.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Mappings {
    /// In the project's order, so a folder comes after the folders around it.
    pub folders: Vec<(PathBuf, Vec<String>)>,
    pub files: Vec<(PathBuf, Vec<String>)>,
}

/// Reads the project at `project` and the projects it nests. It fails unless the root is a `DataModel`.
pub fn place(project: &Path) -> Result<Mappings, String> {
    let file = std::path::absolute(project).map(|p| normalize(&p)).map_err(|e| e.to_string())?;
    let mut reader = Reader::default();
    reader.config(&file, &mut Vec::new())?;
    if !reader.is_place {
        return Err(format!("{} has no DataModel root, so it is not a place", project.display()));
    }
    Ok(reader.mappings)
}

/// A folder that a Rojo tree maps, with the instance it becomes and the `globIgnorePaths` Rojo applies inside it.
#[derive(Debug, Clone)]
pub struct Folder {
    pub path: PathBuf,
    pub instance: Vec<String>,
    pub ignore: Ignore,
}

/// The `globIgnorePaths` of the projects around a folder, in the order Rojo applies them.
#[derive(Debug, Clone, Default)]
pub struct Ignore(Vec<IgnoreRule>);

#[derive(Debug, Clone)]
struct IgnoreRule {
    base: PathBuf,
    glob: globset::GlobMatcher,
    negated: bool,
}

impl Ignore {
    /// Whether Rojo leaves the entry at `path` out of the place. As in Rojo, the last rule that matches decides, and a
    /// rule matches paths below the folder of the project that declares it.
    pub fn ignores(&self, path: &Path) -> bool {
        let mut ignored = false;
        for rule in &self.0 {
            if path.strip_prefix(&rule.base).is_ok_and(|suffix| rule.glob.is_match(suffix)) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

/// Reads the project at `project` as Rojo builds it into a place at `instance`, under the `ignore` rules of the projects
/// around it, and returns every folder its tree maps. Unlike [`place`], it follows a `$path` that names a
/// `*.project.json` file, and it keeps each project's `globIgnorePaths`. It does not look inside the folders, so one that
/// holds a `default.project.json`, which Rojo reads in its place, comes back as a folder.
pub fn folders(project: &Path, instance: &[String], ignore: &Ignore) -> Result<Vec<Folder>, String> {
    let file = std::path::absolute(project).map(|p| normalize(&p)).map_err(|e| e.to_string())?;
    let mut reader = Reader { walk: Some(Walk { ignore: ignore.clone(), folders: Vec::new() }), ..Reader::default() };
    reader.config(&file, &mut instance.to_vec())?;
    Ok(reader.walk.map(|walk| walk.folders).unwrap_or_default())
}

#[derive(Default)]
struct Reader {
    mappings: Mappings,
    is_place: bool,
    // Set by `folders`. `place` leaves it unset, so what the compiler receives does not change.
    walk: Option<Walk>,
}

struct Walk {
    ignore: Ignore,
    folders: Vec<Folder>,
}

impl Reader {
    fn config(&mut self, file: &Path, instance: &mut Vec<String>) -> Result<(), String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let json: Value =
            serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", file.display()))?;
        let (Some(_), Some(tree)) = (json["name"].as_str(), json.get("tree").filter(|t| t.is_object())) else {
            return Err(format!("{} needs a name and a tree", file.display()));
        };
        let base = file.parent().unwrap_or(Path::new("")).to_path_buf();
        let Some(reader) = &mut self.walk else { return self.tree(&base, None, tree, instance) };
        let outer = reader.ignore.0.len();
        for pattern in json["globIgnorePaths"].as_array().into_iter().flatten() {
            let pattern = pattern
                .as_str()
                .ok_or_else(|| format!("{} has a globIgnorePaths entry that is not a string", file.display()))?;
            let (negated, body) = match pattern.strip_prefix('!') {
                Some(body) => (true, body),
                None => (false, pattern.strip_prefix('\\').filter(|body| body.starts_with('!')).unwrap_or(pattern)),
            };
            let glob = globset::Glob::new(body)
                .map_err(|e| format!("{} has an invalid globIgnorePaths entry {pattern}: {e}", file.display()))?;
            reader.ignore.0.push(IgnoreRule { base: base.clone(), glob: glob.compile_matcher(), negated });
        }
        let result = self.tree(&base, None, tree, instance);
        if let Some(reader) = &mut self.walk {
            reader.ignore.0.truncate(outer);
        }
        result
    }

    fn tree(
        &mut self,
        base: &Path,
        name: Option<&str>,
        tree: &Value,
        instance: &mut Vec<String>,
    ) -> Result<(), String> {
        if let Some(name) = name {
            instance.push(name.to_owned());
        }
        let path = match &tree["$path"] {
            Value::String(path) => Some(path.as_str()),
            Value::Object(optional) => optional.get("optional").and_then(|p| p.as_str()),
            _ => None,
        };
        if let Some(path) = path {
            self.path(normalize(&base.join(path)), instance)?;
        }
        if tree["$className"] == "DataModel" {
            self.is_place = true;
        }
        if let Some(children) = tree.as_object() {
            for (child, subtree) in children {
                if !child.starts_with('$') && subtree.is_object() {
                    self.tree(base, Some(child), subtree, instance)?;
                }
            }
        }
        if name.is_some() {
            instance.pop();
        }
        Ok(())
    }

    fn path(&mut self, item: PathBuf, instance: &mut Vec<String>) -> Result<(), String> {
        let project_file = item.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".project.json"));
        if self.walk.is_some() && project_file {
            self.config(&item, instance)?;
        } else if matches!(item.extension().and_then(|e| e.to_str()), Some("lua" | "luau" | "json" | "toml")) {
            self.mappings.files.push((item, instance.clone()));
        } else if let Some(reader) = &mut self.walk {
            // The caller lists the folder anyway, so it finds a default.project.json there without a check per folder.
            reader.folders.push(Folder { path: item, instance: instance.clone(), ignore: reader.ignore.clone() });
        } else if item.join(DEFAULT_PROJECT).is_file() {
            self.config(&item.join(DEFAULT_PROJECT), instance)?;
        } else {
            self.mappings.folders.push((item, instance.clone()));
        }
        Ok(())
    }
}

fn normalize(path: &Path) -> PathBuf {
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
    use serde_json::json;

    fn write(dir: &Path, path: &str, value: &Value) {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, value.to_string()).unwrap();
    }

    fn instance(names: &[&str]) -> Vec<String> {
        names.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn a_place_maps_its_folders_files_and_nested_projects() {
        let dir = tempfile::tempdir().unwrap();
        let root = normalize(&std::path::absolute(dir.path()).unwrap());
        write(
            &root,
            "default.project.json",
            &json!({
                "name": "game",
                "tree": {
                    "$className": "DataModel",
                    "ReplicatedStorage": {
                        "Shared": { "$path": "out/shared" },
                        "Config": { "$path": "config.json" },
                        "Script": { "$path": "./src/../tool.lua" },
                        "Maybe": { "$path": { "optional": "optional" } },
                        "Package": { "$path": "package" },
                    },
                },
            }),
        );
        write(&root, "package/default.project.json", &json!({ "name": "package", "tree": { "$path": "lib" } }));
        let mappings = place(&root.join("default.project.json")).unwrap();
        let mut folders = mappings.folders;
        folders.sort();
        assert_eq!(
            folders,
            [
                (root.join("optional"), instance(&["ReplicatedStorage", "Maybe"])),
                (root.join("out/shared"), instance(&["ReplicatedStorage", "Shared"])),
                (root.join("package/lib"), instance(&["ReplicatedStorage", "Package"])),
            ],
        );
        let mut files = mappings.files;
        files.sort();
        assert_eq!(
            files,
            [
                (root.join("config.json"), instance(&["ReplicatedStorage", "Config"])),
                (root.join("tool.lua"), instance(&["ReplicatedStorage", "Script"])),
            ],
        );
    }

    #[test]
    fn folders_follow_project_files_and_keep_ignore_rules() {
        let dir = tempfile::tempdir().unwrap();
        let root = normalize(&std::path::absolute(dir.path()).unwrap());
        write(
            &root,
            "game/default.project.json",
            &json!({
                "name": "game",
                "globIgnorePaths": ["**/node_modules/@rbxts/*/node_modules/**", "!**/keep/**"],
                "tree": {
                    "$className": "DataModel",
                    "Packages": { "$path": "node_modules/@rbxts" },
                    "Extra": { "$path": "vendor.project.json" },
                    "Config": { "$path": "config.json" },
                },
            }),
        );
        write(
            &root,
            "game/vendor.project.json",
            &json!({ "name": "vendor", "globIgnorePaths": ["*.spec.luau"], "tree": { "$path": "../vendor" } }),
        );
        let project = root.join("game/default.project.json");
        let folders = folders(&project, &instance(&["Nested"]), &Ignore::default()).unwrap();
        let found: Vec<_> = folders.iter().map(|f| (f.path.clone(), f.instance.clone())).collect();
        assert_eq!(
            found,
            [
                (root.join("vendor"), instance(&["Nested", "Extra"])),
                (root.join("game/node_modules/@rbxts"), instance(&["Nested", "Packages"])),
            ],
        );
        let packages = &folders[1].ignore;
        assert!(packages.ignores(&root.join("game/node_modules/@rbxts/holder/node_modules/@rbxts/charm")));
        assert!(!packages.ignores(&root.join("game/node_modules/@rbxts/charm")));
        assert!(!packages.ignores(&root.join("game/node_modules/@rbxts/holder/node_modules/keep/charm")));
        assert!(!packages.ignores(&root.join("other/node_modules/@rbxts/holder/node_modules/@rbxts/charm")));
        assert!(!packages.ignores(&root.join("game/test.spec.luau")), "a nested project's rules stay inside it");
        let vendor = &folders[0].ignore;
        assert!(vendor.ignores(&root.join("game/a/test.spec.luau")));
        assert!(
            !vendor.ignores(&root.join("vendor/test.spec.luau")),
            "rules hold below the declaring project's folder"
        );
        let mappings = place(&project).unwrap();
        assert_eq!(mappings.folders, [(root.join("game/node_modules/@rbxts"), instance(&["Packages"]))]);
        let files: Vec<_> = mappings.files.into_iter().map(|(path, _)| path).collect();
        assert_eq!(files, [root.join("game/config.json"), root.join("game/vendor.project.json")], "place is unchanged");
    }

    #[test]
    fn folders_reject_invalid_ignore_globs() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "default.project.json", &json!({ "name": "lib", "globIgnorePaths": ["a[" ], "tree": {} }));
        let error = folders(&dir.path().join("default.project.json"), &[], &Ignore::default()).unwrap_err();
        assert!(error.contains("invalid globIgnorePaths entry a["), "{error}");
    }

    #[test]
    fn a_project_without_a_data_model_root_is_not_a_place() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "default.project.json", &json!({ "name": "lib", "tree": { "$path": "src" } }));
        let project = dir.path().join("default.project.json");
        assert_eq!(place(&project), Err(format!("{} has no DataModel root, so it is not a place", project.display())));
        write(dir.path(), "default.project.json", &json!({ "name": "lib" }));
        assert!(place(&project).unwrap_err().ends_with("needs a name and a tree"));
    }
}
