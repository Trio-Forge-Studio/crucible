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

#[derive(Default)]
struct Reader {
    mappings: Mappings,
    is_place: bool,
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
        self.tree(&base, None, tree, instance)
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
        if matches!(item.extension().and_then(|e| e.to_str()), Some("lua" | "luau" | "json" | "toml")) {
            self.mappings.files.push((item, instance.clone()));
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
    fn a_project_without_a_data_model_root_is_not_a_place() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "default.project.json", &json!({ "name": "lib", "tree": { "$path": "src" } }));
        let project = dir.path().join("default.project.json");
        assert_eq!(place(&project), Err(format!("{} has no DataModel root, so it is not a place", project.display())));
        write(dir.path(), "default.project.json", &json!({ "name": "lib" }));
        assert!(place(&project).unwrap_err().ends_with("needs a name and a tree"));
    }
}
