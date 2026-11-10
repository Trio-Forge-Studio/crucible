use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
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

/// Writes each module under `out` through `replace`, skipping files whose bytes already match, and counts the files
/// it wrote. Then it deletes every `.luau` file under `out` that starts with the compiler's header and is not one of
/// `modules`, the temporary files of a write or a case-only rename that did not finish, and any folder that deletion
/// leaves empty, and counts the files it deleted. Files the compiler did not emit, such as `rbxtsc` output and
/// declarations, stay.
///
/// On a case-insensitive file system, a module whose path changed only in case can keep the old name of its file.
/// That file is renamed to the module's path, because Rojo names the instance after the file.
pub fn write(out: &Path, modules: &[Module]) -> io::Result<Written> {
    write_to(&mut Os, out, modules)
}

/// Writes `bytes` to a new file beside `path`, then renames it over `path`, so `path` holds its old bytes or all of
/// the new ones even when the write fails or the process dies. The new file's name ends in no extension Rojo reads,
/// and holds the process id, so a concurrent build never writes the same one.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_on(&mut Os, path, bytes)
}

fn replace_on(disk: &mut impl Disk, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut n = 1;
    let temporary = loop {
        let temporary = path.with_file_name(format!("{name}{WRITING}-{}-{n}", std::process::id()));
        match disk.create(&temporary, bytes) {
            Ok(()) => break temporary,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(error) => {
                let _ = disk.remove_file(&temporary);
                return Err(error);
            }
        }
    };
    disk.rename(&temporary, path).inspect_err(|_| {
        let _ = disk.remove_file(&temporary);
    })
}

trait Disk {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// The first `len` bytes, or the whole file when it is shorter.
    fn head(&self, path: &Path, len: usize) -> io::Result<Vec<u8>>;
    /// Writes a file that must not exist yet.
    fn create(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
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

    fn head(&self, path: &Path, len: usize) -> io::Result<Vec<u8>> {
        let mut head = Vec::with_capacity(len);
        std::fs::File::open(path)?.take(len as u64).read_to_end(&mut head)?;
        Ok(head)
    }

    fn create(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(bytes)
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
        replace_on(disk, &path, module.source.as_bytes())?;
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
            rename_case(disk, dir, &present, name, target)?;
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
            EntryKind::File if partial_write_of(name).is_some_and(|file| file.ends_with(".luau")) => {
                // A write that did not finish stopped somewhere in the module, which starts with the header.
                if HEADER.as_bytes().starts_with(&disk.head(&path, HEADER.len())?) {
                    disk.remove_file(&path)?;
                    changes.removed += 1;
                }
            }
            EntryKind::File
                if (is_temporary(name) || name.ends_with(".luau") && !wanted.is_some_and(|w| w.contains(name)))
                    && disk.head(&path, HEADER.len())? == HEADER.as_bytes() =>
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

/// Some file systems treat a rename that changes only case as a no-op, so it goes through a temporary name that no
/// entry of `dir` has, because a rename replaces the file it lands on. If the second rename fails, the entry goes back
/// to its old name.
fn rename_case(disk: &mut impl Disk, dir: &Path, present: &HashSet<&str>, from: &str, to: &str) -> io::Result<()> {
    let mut n = 1;
    let temporary = loop {
        let name = temporary_name(from, n);
        if !present.iter().any(|p| same_name(p, &name)) {
            break dir.join(name);
        }
        n += 1;
    };
    disk.rename(&dir.join(from), &temporary)?;
    if let Err(error) = disk.rename(&temporary, &dir.join(to)) {
        // When this fails too, the next sweep deletes the compiler's files under the temporary name.
        let _ = disk.rename(&temporary, &dir.join(from));
        return Err(error);
    }
    Ok(())
}

const TEMPORARY: &str = ".crucible-rename";

fn temporary_name(name: &str, n: u32) -> String {
    if n == 1 { format!("{name}{TEMPORARY}") } else { format!("{name}{TEMPORARY}-{n}") }
}

const WRITING: &str = ".crucible-write";

/// The name of the file `replace` was writing when it gave a temporary file this name.
pub(crate) fn partial_write_of(name: &str) -> Option<&str> {
    let (file, rest) = name.rsplit_once(WRITING)?;
    let (process, n) = rest.strip_prefix('-')?.split_once('-')?;
    let number = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (!file.is_empty() && number(process) && number(n)).then_some(file)
}

/// Whether `rename_case` could have given a module file this name.
fn is_temporary(name: &str) -> bool {
    let Some((module, suffix)) = name.rsplit_once(TEMPORARY) else { return false };
    let n = if suffix.is_empty() { Some(1) } else { suffix.strip_prefix('-').and_then(|n| n.parse().ok()) };
    module.ends_with(".luau") && n.is_some_and(|n| temporary_name(module, n) == name)
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
        /// A rename to exactly this path fails.
        refused: Option<String>,
        /// Each new file gets half its bytes, and then the write fails.
        full: bool,
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

        fn head(&self, path: &Path, len: usize) -> io::Result<Vec<u8>> {
            let mut bytes = self.read(path)?;
            bytes.truncate(len);
            Ok(bytes)
        }

        fn create(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            if self.entries.contains_key(&key(path)) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            let stored = path.to_string_lossy().replace('\\', "/");
            let written = if self.full { &bytes[..bytes.len() / 2] } else { bytes };
            self.entries.insert(key(path), (stored, Some(written.to_vec())));
            if self.full { Err(io::Error::from(io::ErrorKind::StorageFull)) } else { Ok(()) }
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
            if self.refused.as_ref() == Some(&to_stored) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
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
        assert_eq!(changes, Written { written: 1, renamed: 0, removed: 0 }, "the write's rename names the file");
        assert_eq!(disk.names("out"), ["FOO.luau", "Systems"]);
        assert_eq!(disk.read(Path::new("out/FOO.luau")).unwrap(), new_bytes[0].source.as_bytes());
    }

    #[test]
    fn a_case_only_rename_never_replaces_a_file_beside_it() {
        let mut disk = Folding::default();
        write_to(&mut disk, Path::new("out"), &[module("foo.luau", "return 1\n")]).unwrap();
        disk.create(Path::new("out/foo.luau.crucible-rename"), b"mine").unwrap();

        let changes = write_to(&mut disk, Path::new("out"), &[module("Foo.luau", "return 1\n")]).unwrap();
        assert_eq!(changes, Written { written: 0, renamed: 1, removed: 0 });
        assert_eq!(disk.names("out"), ["Foo.luau", "foo.luau.crucible-rename"]);
        assert_eq!(disk.read(Path::new("out/foo.luau.crucible-rename")).unwrap(), b"mine");
    }

    #[test]
    fn a_failed_case_only_rename_puts_the_module_back_and_the_next_write_finishes_it() {
        let mut disk = Folding { refused: Some("out/Foo.luau".to_owned()), ..Folding::default() };
        write_to(&mut disk, Path::new("out"), &[module("foo.luau", "return 1\n")]).unwrap();

        let recased = [module("Foo.luau", "return 1\n")];
        assert!(write_to(&mut disk, Path::new("out"), &recased).is_err());
        assert_eq!(disk.names("out"), ["foo.luau"]);

        disk.refused = None;
        let changes = write_to(&mut disk, Path::new("out"), &recased).unwrap();
        assert_eq!(changes, Written { written: 0, renamed: 1, removed: 0 });
        assert_eq!(disk.names("out"), ["Foo.luau"]);
    }

    #[test]
    fn the_sweep_removes_the_temporary_files_a_failed_rename_left_and_no_others() {
        let mut disk = Folding::default();
        let header = |body: &str| format!("{HEADER}{body}").into_bytes();
        disk.create_dir_all(Path::new("out")).unwrap();
        disk.create(Path::new("out/foo.luau.crucible-rename"), &header("return 1\n")).unwrap();
        disk.create(Path::new("out/foo.luau.crucible-rename-2"), &header("return 1\n")).unwrap();
        disk.create(Path::new("out/bar.luau.crucible-rename"), b"mine").unwrap();
        disk.create(Path::new("out/notes.crucible-rename"), &header("")).unwrap();
        disk.create(Path::new("out/foo.luau.crucible-rename-x"), &header("")).unwrap();

        let changes = write_to(&mut disk, Path::new("out"), &[module("Foo.luau", "return 1\n")]).unwrap();
        assert_eq!(changes, Written { written: 1, renamed: 0, removed: 2 });
        assert_eq!(
            disk.names("out"),
            ["bar.luau.crucible-rename", "Foo.luau", "foo.luau.crucible-rename-x", "notes.crucible-rename"]
        );
    }

    #[test]
    fn a_failed_write_leaves_each_module_whole_and_a_rerun_converges() {
        let mut disk = Folding::default();
        let old = [module("foo.luau", "return 1\n"), module("bar.luau", "return 2\n")];
        write_to(&mut disk, Path::new("out"), &old).unwrap();

        let new = [module("foo.luau", "return 3\n"), module("bar.luau", "return 4\n")];
        disk.full = true;
        assert!(write_to(&mut disk, Path::new("out"), &new).is_err());
        assert_eq!(disk.names("out"), ["bar.luau", "foo.luau"], "no temporary file stays");
        assert_eq!(disk.read(Path::new("out/foo.luau")).unwrap(), old[0].source.as_bytes());
        assert_eq!(disk.read(Path::new("out/bar.luau")).unwrap(), old[1].source.as_bytes());

        disk.refused = Some("out/bar.luau".to_owned());
        disk.full = false;
        assert!(write_to(&mut disk, Path::new("out"), &new).is_err());
        assert_eq!(disk.names("out"), ["bar.luau", "foo.luau"], "a refused rename removes its temporary file");
        assert_eq!(disk.read(Path::new("out/bar.luau")).unwrap(), old[1].source.as_bytes());

        disk.refused = None;
        assert_eq!(
            write_to(&mut disk, Path::new("out"), &new).unwrap(),
            Written { written: 1, renamed: 0, removed: 0 }
        );
        assert_eq!(write_to(&mut disk, Path::new("out"), &new).unwrap(), Written::default());
    }

    #[test]
    fn the_sweep_removes_what_a_killed_write_left_and_never_takes_it_for_a_module() {
        let mut disk = Folding::default();
        let header = HEADER.as_bytes();
        disk.create_dir_all(Path::new("out/systems")).unwrap();
        disk.create(Path::new("out/foo.luau.crucible-write-41-1"), b"").unwrap();
        disk.create(Path::new("out/foo.luau.crucible-write-41-2"), &header[..9]).unwrap();
        disk.create(Path::new("out/systems/bar.luau.crucible-write-7-1"), &[header, b"return"].concat()).unwrap();
        disk.create(Path::new("out/notes.luau.crucible-write-41-1"), b"mine").unwrap();
        disk.create(Path::new("out/foo.luau.crucible-write-41"), header).unwrap();
        disk.create(Path::new("out/data.json.crucible-write-41-1"), header).unwrap();

        let changes = write_to(&mut disk, Path::new("out"), &[module("foo.luau", "return 1\n")]).unwrap();
        assert_eq!(changes, Written { written: 1, renamed: 0, removed: 3 });
        assert_eq!(
            disk.names("out"),
            [
                "data.json.crucible-write-41-1",
                "foo.luau",
                "foo.luau.crucible-write-41",
                "notes.luau.crucible-write-41-1"
            ]
        );
    }
}
