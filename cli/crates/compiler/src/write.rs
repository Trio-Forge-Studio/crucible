use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::path::Path;

use crate::Module;

/// The first line of every module `compile` emits. `write` treats a `.luau` file as its own only when the file
/// starts with it.
pub(crate) const HEADER: &str = "-- Compiled with crucible-compiler\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub written: usize,
    /// Files and folders whose names differed from a module path only in case, renamed to that path.
    pub renamed: usize,
    pub removed: usize,
}

/// Writes each module under `out`, skipping files whose bytes already match, and counts the files it wrote. Then it
/// deletes every `.luau` file under `out` that starts with the compiler's header and is not one of `modules`, and
/// any folder that deletion leaves empty, and counts the files it deleted. Files the compiler did not emit, such as
/// `rbxtsc` output and declarations, stay.
///
/// On a case-insensitive file system, a module whose source changed only in case is written into the existing file
/// under its old name. That file is renamed to the module's path, because Rojo names the instance after the file.
pub fn write(out: &Path, modules: &[Module]) -> io::Result<Written> {
    write_to(&mut Os, out, modules)
}

trait Disk {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn starts_with(&self, path: &Path, prefix: &[u8]) -> io::Result<bool>;
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn create_dir_all(&mut self, path: &Path) -> io::Result<()>;
    /// Empty when `dir` does not exist.
    fn entries(&self, dir: &Path) -> io::Result<Vec<Entry>>;
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&mut self, path: &Path) -> io::Result<()>;
    fn remove_dir(&mut self, path: &Path) -> io::Result<()>;
}

struct Entry {
    name: String,
    kind: EntryKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
    Other,
}

struct Os;

impl Disk for Os {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn starts_with(&self, path: &Path, prefix: &[u8]) -> io::Result<bool> {
        let mut head = Vec::with_capacity(prefix.len());
        std::fs::File::open(path)?.take(prefix.len() as u64).read_to_end(&mut head)?;
        Ok(head == prefix)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn entries(&self, dir: &Path) -> io::Result<Vec<Entry>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else { continue };
            let kind = entry.file_type()?;
            let kind = if kind.is_dir() {
                EntryKind::Dir
            } else if kind.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            out.push(Entry { name, kind });
        }
        Ok(out)
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_dir(&mut self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir(path)
    }
}

/// The names each folder of the output should hold, keyed by the folder's path relative to the output with forward
/// slashes, `""` for the output itself.
type Expected<'m> = HashMap<&'m str, HashSet<&'m str>>;

fn write_to(disk: &mut impl Disk, out: &Path, modules: &[Module]) -> io::Result<Written> {
    let mut changes = Written::default();
    for module in modules {
        let path = out.join(&module.path);
        if disk.read(&path).is_ok_and(|old| old == module.source.as_bytes()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            disk.create_dir_all(dir)?;
        }
        disk.write(&path, module.source.as_bytes())?;
        changes.written += 1;
    }
    let mut expected: Expected = HashMap::new();
    for module in modules {
        let path = module.path.as_str();
        let (mut folder, mut rest) = ("", path);
        while let Some((part, tail)) = rest.split_once('/') {
            expected.entry(folder).or_default().insert(part);
            folder = &path[..path.len() - tail.len() - 1];
            rest = tail;
        }
        expected.entry(folder).or_default().insert(rest);
    }
    sweep(disk, out, "", &expected, &mut changes)?;
    Ok(changes)
}

fn sweep(disk: &mut impl Disk, dir: &Path, folder: &str, expected: &Expected, changes: &mut Written) -> io::Result<()> {
    let wanted = expected.get(folder);
    let entries = disk.entries(dir)?;
    let present: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    for entry in &entries {
        let mut name = entry.name.as_str();
        let candidate = entry.kind == EntryKind::Dir || name.ends_with(".luau");
        if candidate
            && let Some(wanted) = wanted
            && !wanted.contains(name)
            && let Some(&target) = wanted.iter().find(|w| !present.contains(**w) && same_name(w, name))
        {
            // The exact name is absent from the listing although `write` just wrote it, so the file system
            // resolved that name to this entry and ignores case.
            rename_case(disk, &dir.join(name), &dir.join(target))?;
            changes.renamed += 1;
            name = target;
        }
        let path = dir.join(name);
        match entry.kind {
            EntryKind::Dir => {
                let inner = if folder.is_empty() { name.to_owned() } else { format!("{folder}/{name}") };
                let removed = changes.removed;
                sweep(disk, &path, &inner, expected, changes)?;
                if changes.removed > removed && disk.entries(&path)?.is_empty() {
                    disk.remove_dir(&path)?;
                }
            }
            EntryKind::File
                if name.ends_with(".luau")
                    && !wanted.is_some_and(|w| w.contains(name))
                    && disk.starts_with(&path, HEADER.as_bytes())? =>
            {
                disk.remove_file(&path)?;
                changes.removed += 1;
            }
            _ => {}
        }
    }
    Ok(())
}

fn same_name(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.to_lowercase() == b.to_lowercase()
}

/// Some file systems treat a rename that changes only case as a no-op, so it goes through a temporary name.
fn rename_case(disk: &mut impl Disk, from: &Path, to: &Path) -> io::Result<()> {
    let mut temporary = from.as_os_str().to_owned();
    temporary.push(".crucible-rename");
    disk.rename(from, Path::new(&temporary))?;
    disk.rename(Path::new(&temporary), to)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{Disk, Entry, EntryKind, HEADER, Written, write_to};
    use crate::Module;

    /// A case-insensitive, case-preserving file system, as Windows and macOS have by default.
    #[derive(Default)]
    struct Folding {
        /// Keyed by the lowercase path. The value holds the path with its stored case, and no contents for a folder.
        entries: BTreeMap<String, (String, Option<Vec<u8>>)>,
    }

    fn key(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/").to_lowercase()
    }

    fn not_found() -> io::Error {
        io::Error::from(io::ErrorKind::NotFound)
    }

    impl Folding {
        fn names(&self, dir: &str) -> Vec<String> {
            self.entries(Path::new(dir)).unwrap().into_iter().map(|e| e.name).collect()
        }
    }

    impl Disk for Folding {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            match self.entries.get(&key(path)) {
                Some((_, Some(bytes))) => Ok(bytes.clone()),
                _ => Err(not_found()),
            }
        }

        fn starts_with(&self, path: &Path, prefix: &[u8]) -> io::Result<bool> {
            Ok(self.read(path)?.starts_with(prefix))
        }

        fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let stored = path.to_string_lossy().replace('\\', "/");
            let entry = self.entries.entry(key(path)).or_insert((stored, None));
            entry.1 = Some(bytes.to_vec());
            Ok(())
        }

        fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
            let mut current = PathBuf::new();
            for part in path.components() {
                current.push(part);
                let stored = current.to_string_lossy().replace('\\', "/");
                self.entries.entry(key(&current)).or_insert((stored, None));
            }
            Ok(())
        }

        fn entries(&self, dir: &Path) -> io::Result<Vec<Entry>> {
            let prefix = format!("{}/", key(dir));
            Ok(self
                .entries
                .iter()
                .filter(|(k, _)| k.strip_prefix(&prefix).is_some_and(|rest| !rest.contains('/')))
                .map(|(_, (stored, contents))| Entry {
                    name: stored.rsplit('/').next().unwrap().to_owned(),
                    kind: if contents.is_some() { EntryKind::File } else { EntryKind::Dir },
                })
                .collect())
        }

        fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            let (from_key, to_stored) = (key(from), to.to_string_lossy().replace('\\', "/"));
            let moved: Vec<String> = self
                .entries
                .keys()
                .filter(|k| **k == from_key || k.starts_with(&format!("{from_key}/")))
                .cloned()
                .collect();
            if moved.is_empty() {
                return Err(not_found());
            }
            for k in moved {
                let (stored, contents) = self.entries.remove(&k).unwrap();
                let renamed = format!("{to_stored}{}", &stored[from_key.len()..]);
                self.entries.insert(key(Path::new(&renamed)), (renamed, contents));
            }
            Ok(())
        }

        fn remove_file(&mut self, path: &Path) -> io::Result<()> {
            self.entries.remove(&key(path)).map(drop).ok_or_else(not_found)
        }

        fn remove_dir(&mut self, path: &Path) -> io::Result<()> {
            self.entries.remove(&key(path)).map(drop).ok_or_else(not_found)
        }
    }

    fn module(path: &str, body: &str) -> Module {
        Module { path: path.to_owned(), source: format!("{HEADER}{body}") }
    }

    #[test]
    fn a_case_only_rename_keeps_the_module_under_its_new_name() {
        let mut disk = Folding::default();
        let first = [module("foo.luau", "return 1\n"), module("systems/bar.luau", "return 2\n")];
        write_to(&mut disk, Path::new("out"), &first).unwrap();

        let same_bytes = [module("Foo.luau", "return 1\n"), module("Systems/bar.luau", "return 2\n")];
        let changes = write_to(&mut disk, Path::new("out"), &same_bytes).unwrap();
        assert_eq!(changes, Written { written: 0, renamed: 2, removed: 0 });
        assert_eq!(disk.names("out"), ["Foo.luau", "Systems"]);
        assert_eq!(disk.names("out/Systems"), ["bar.luau"]);

        let new_bytes = [module("FOO.luau", "return 3\n"), module("Systems/bar.luau", "return 2\n")];
        let changes = write_to(&mut disk, Path::new("out"), &new_bytes).unwrap();
        assert_eq!(changes, Written { written: 1, renamed: 1, removed: 0 });
        assert_eq!(disk.names("out"), ["FOO.luau", "Systems"]);
        assert_eq!(disk.read(Path::new("out/FOO.luau")).unwrap(), new_bytes[0].source.as_bytes());
    }
}
