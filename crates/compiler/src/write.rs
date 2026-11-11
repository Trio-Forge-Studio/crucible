use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

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
/// `modules`, the temporary files of a case-only rename that failed, and any folder that deletion leaves empty, and
/// counts the files it deleted. Files the compiler did not emit, such as `rbxtsc` output and declarations, stay.
///
/// On a case-insensitive file system, a module whose path changed only in case can keep the old name of its file.
/// That file is renamed to the module's path, because Rojo names the instance after the file.
pub fn write(out: &Path, modules: &[Module]) -> Result<Written, WriteError> {
    write_to(&mut Os, out, modules)
}

/// A failed `write` or `write_include`, with the file or folder it failed on.
#[derive(Debug)]
pub struct WriteError {
    pub path: PathBuf,
    pub error: io::Error,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.error)
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

pub(crate) fn at(path: &Path) -> impl FnOnce(io::Error) -> WriteError + '_ {
    move |error| WriteError { path: path.to_owned(), error }
}

/// The one file that a build writes `destination`'s new bytes into before it renames that file over `destination`.
/// It sits beside `destination`, so the rename stays on one volume. Its name depends only on `destination`'s name, so
/// what a killed build left is the file the next build of `destination` writes again, and a build never needs to
/// remove a file it cannot tell is its own. The name is 26 characters and `extension`, whatever `destination`'s
/// length, and starts with `.crucible-`.
pub fn temporary_path(destination: &Path, extension: &str) -> PathBuf {
    let name = destination.file_name().map_or(&[][..], |name| name.as_encoded_bytes());
    // FNV-1a is fixed, so every build and every version of Crucible picks the same name.
    let hash = name
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
    let dot = if extension.is_empty() { "" } else { "." };
    destination.with_file_name(format!(".crucible-{hash:016x}{dot}{extension}"))
}

/// Writes `bytes` to `path`'s temporary file, then renames that file over `path`, so `path` holds its old bytes or
/// all of the new ones even when the write fails or the process dies. The temporary file ends in `.tmp`, which no
/// module or runtime library file does and Rojo reads in no folder. A second build that writes `path` at the same
/// time fails instead of writing the same temporary file.
///
/// A link or a read-only file at `path`, and a `path` the rename cannot replace, are written in place, as builds made
/// before temporary files wrote them, so a failure during that write can leave `path` partly written.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_on(&mut Os, path, bytes)
}

fn replace_on(disk: &mut impl Disk, path: &Path, bytes: &[u8]) -> io::Result<()> {
    if disk.in_place(path) {
        return disk.write(path, bytes);
    }
    let temporary = temporary_path(path, "tmp");
    let mut file = disk.open_temporary(&temporary)?;
    let replaced = match disk.write_temporary(&mut file, bytes) {
        Ok(()) => match disk.rename(&temporary, path) {
            Ok(()) => return Ok(()),
            // Windows refuses to rename over a file that another program holds open without delete sharing, which a
            // write in place may still get past. Emptying the temporary file gives that write the space it had before
            // temporary files, and the file keeps its name, so other builds stay out, until the write ends.
            Err(_) => {
                let _ = disk.write_temporary(&mut file, &[]);
                disk.write(path, bytes)
            }
        },
        Err(error) => Err(error),
    };
    let _ = disk.remove_file(&temporary);
    drop(file);
    replaced
}

/// Puts the file at `temporary` in `destination`'s place with a rename. A link or a read-only file at `destination`,
/// and a `destination` the rename cannot replace, get `temporary`'s bytes written in place, as builds made before
/// temporary files wrote them, after `temporary` is removed so that the write has the space it had then.
pub fn publish(temporary: &Path, destination: &Path) -> io::Result<()> {
    publish_on(&mut Os, temporary, destination)
}

fn publish_on(disk: &mut impl Disk, temporary: &Path, destination: &Path) -> io::Result<()> {
    if !disk.in_place(destination) && disk.rename(temporary, destination).is_ok() {
        return Ok(());
    }
    let bytes = disk.read(temporary)?;
    let _ = disk.remove_file(temporary);
    disk.write(destination, &bytes)
}

trait Disk {
    /// An open temporary file, which keeps other builds out of it until it is dropped.
    type Temporary;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn starts_with(&self, path: &Path, prefix: &[u8]) -> io::Result<bool>;
    /// Whether `path` is a link, whose target a write in place updates and a rename would not, or a read-only file,
    /// which a write in place fails on and a rename may still replace.
    fn in_place(&self, path: &Path) -> bool;
    /// Writes `path` in place.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Opens `path`, creating it, and fails with `ResourceBusy` while another build has it open.
    fn open_temporary(&mut self, path: &Path) -> io::Result<Self::Temporary>;
    /// Replaces the temporary file's bytes with `bytes`.
    fn write_temporary(&mut self, file: &mut Self::Temporary, bytes: &[u8]) -> io::Result<()>;
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
    type Temporary = File;

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn starts_with(&self, path: &Path, prefix: &[u8]) -> io::Result<bool> {
        let mut head = Vec::with_capacity(prefix.len());
        File::open(path)?.take(prefix.len() as u64).read_to_end(&mut head)?;
        Ok(head == prefix)
    }

    fn in_place(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink() || metadata.permissions().readonly())
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn open_temporary(&mut self, path: &Path) -> io::Result<File> {
        // It is emptied only once the lock is held, so the open cannot cut short another build's write.
        let file = OpenOptions::new().write(true).create(true).truncate(false).open(path)?;
        match file.try_lock() {
            // A file system without locks lets a concurrent build through, as builds made before temporary files did.
            Ok(()) | Err(TryLockError::Error(_)) => Ok(file),
            Err(TryLockError::WouldBlock) => Err(busy()),
        }
    }

    fn write_temporary(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        file.set_len(0)?;
        file.write_all(bytes)
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

fn write_to(disk: &mut impl Disk, out: &Path, modules: &[Module]) -> Result<Written, WriteError> {
    let mut changes = Written::default();
    for module in modules {
        let path = out.join(&module.path);
        if disk.read(&path).is_ok_and(|old| old == module.source.as_bytes()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            disk.create_dir_all(dir).map_err(at(dir))?;
        }
        replace_on(disk, &path, module.source.as_bytes()).map_err(at(&path))?;
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

fn sweep(
    disk: &mut impl Disk,
    dir: &Path,
    folder: &str,
    expected: &Expected,
    changes: &mut Written,
) -> Result<(), WriteError> {
    let wanted = expected.get(folder);
    let entries = disk.entries(dir).map_err(at(dir))?;
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
            rename_case(disk, dir, &present, name, target).map_err(at(&dir.join(target)))?;
            changes.renamed += 1;
            name = target;
        }
        let path = dir.join(name);
        match entry.kind {
            EntryKind::Dir => {
                let inner = if folder.is_empty() { name.to_owned() } else { format!("{folder}/{name}") };
                let removed = changes.removed;
                sweep(disk, &path, &inner, expected, changes)?;
                if changes.removed > removed && disk.entries(&path).map_err(at(&path))?.is_empty() {
                    disk.remove_dir(&path).map_err(at(&path))?;
                }
            }
            EntryKind::File
                if (is_temporary(name) || name.ends_with(".luau") && !wanted.is_some_and(|w| w.contains(name)))
                    && disk.starts_with(&path, HEADER.as_bytes()).map_err(at(&path))? =>
            {
                disk.remove_file(&path).map_err(at(&path))?;
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

fn busy() -> io::Error {
    io::Error::new(io::ErrorKind::ResourceBusy, "another build is writing it")
}

/// Whether `rename_case` could have given a module file this name.
fn is_temporary(name: &str) -> bool {
    let Some((module, suffix)) = name.rsplit_once(TEMPORARY) else { return false };
    let n = if suffix.is_empty() { Some(1) } else { suffix.strip_prefix('-').and_then(|n| n.parse().ok()) };
    module.ends_with(".luau") && n.is_some_and(|n| temporary_name(module, n) == name)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{Disk, Entry, EntryKind, HEADER, Os, Written, publish_on, temporary_path, write_to};
    use crate::Module;

    /// A case-insensitive, case-preserving file system, as Windows and macOS have by default.
    #[derive(Default)]
    struct Folding {
        /// Keyed by the lowercase path. The value holds the path with its stored case, and no contents for a folder.
        entries: BTreeMap<String, (String, Option<Vec<u8>>)>,
        /// A rename to exactly this path fails.
        refused: Option<String>,
        /// A write in place to exactly this path fails, as when another program holds the file.
        held: Option<String>,
        /// Each temporary file gets half its bytes, and then the write fails.
        full: bool,
        /// Another build has every temporary file open.
        busy: bool,
        /// The most bytes all files hold together. A write past it keeps the bytes that fit and fails.
        capacity: Option<usize>,
        /// Links, keyed by the lowercase path, to the path they point to. Reads and writes in place go to the target,
        /// and a rename replaces the link.
        links: BTreeMap<String, String>,
        /// Lowercase paths of read-only files. A write in place fails, and a rename replaces them, as on Unix.
        read_only: BTreeSet<String>,
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

        fn target(&self, path: &Path) -> PathBuf {
            self.links.get(&key(path)).map_or_else(|| path.to_owned(), PathBuf::from)
        }

        fn put(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let stored = path.to_string_lossy().replace('\\', "/");
            self.entries.entry(key(path)).or_insert((stored, None)).1 = Some(Vec::new());
            let used: usize = self.entries.values().filter_map(|(_, contents)| contents.as_ref().map(Vec::len)).sum();
            let fits = self.capacity.map_or(bytes.len(), |capacity| bytes.len().min(capacity.saturating_sub(used)));
            self.entries.get_mut(&key(path)).unwrap().1 = Some(bytes[..fits].to_vec());
            if fits == bytes.len() { Ok(()) } else { Err(io::Error::from(io::ErrorKind::StorageFull)) }
        }
    }

    impl Disk for Folding {
        type Temporary = PathBuf;

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            match self.entries.get(&key(&self.target(path))) {
                Some((_, Some(bytes))) => Ok(bytes.clone()),
                _ => Err(not_found()),
            }
        }

        fn starts_with(&self, path: &Path, prefix: &[u8]) -> io::Result<bool> {
            Ok(self.read(path)?.starts_with(prefix))
        }

        fn in_place(&self, path: &Path) -> bool {
            self.links.contains_key(&key(path)) || self.read_only.contains(&key(path))
        }

        fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let path = self.target(path);
            let stored = path.to_string_lossy().replace('\\', "/");
            if self.held.as_ref() == Some(&stored) || self.read_only.contains(&key(&path)) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            self.put(&path, bytes)
        }

        fn open_temporary(&mut self, path: &Path) -> io::Result<PathBuf> {
            if self.busy {
                return Err(super::busy());
            }
            let stored = path.to_string_lossy().replace('\\', "/");
            self.entries.entry(key(path)).or_insert((stored, Some(Vec::new())));
            Ok(path.to_owned())
        }

        fn write_temporary(&mut self, file: &mut PathBuf, bytes: &[u8]) -> io::Result<()> {
            let written = if self.full { &bytes[..bytes.len() / 2] } else { bytes };
            self.put(file, written)?;
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
            self.links.remove(&key(to));
            self.read_only.remove(&key(to));
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
        disk.write(Path::new("out/foo.luau.crucible-rename"), b"mine").unwrap();

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
        disk.write(Path::new("out/foo.luau.crucible-rename"), &header("return 1\n")).unwrap();
        disk.write(Path::new("out/foo.luau.crucible-rename-2"), &header("return 1\n")).unwrap();
        disk.write(Path::new("out/bar.luau.crucible-rename"), b"mine").unwrap();
        disk.write(Path::new("out/notes.crucible-rename"), &header("")).unwrap();
        disk.write(Path::new("out/foo.luau.crucible-rename-x"), &header("")).unwrap();

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

        disk.full = false;
        disk.refused = Some("out/bar.luau".to_owned());
        disk.held = Some("out/bar.luau".to_owned());
        assert!(write_to(&mut disk, Path::new("out"), &new).is_err());
        assert_eq!(disk.names("out"), ["bar.luau", "foo.luau"], "a refused rename removes its temporary file");
        assert_eq!(disk.read(Path::new("out/bar.luau")).unwrap(), old[1].source.as_bytes());

        disk.held = None;
        assert_eq!(
            write_to(&mut disk, Path::new("out"), &new).unwrap(),
            Written { written: 1, renamed: 0, removed: 0 },
            "a refused rename writes the file in place"
        );
        assert_eq!(disk.names("out"), ["bar.luau", "foo.luau"]);
        assert_eq!(disk.read(Path::new("out/bar.luau")).unwrap(), new[1].source.as_bytes());
        assert_eq!(write_to(&mut disk, Path::new("out"), &new).unwrap(), Written::default());
    }

    #[test]
    fn a_killed_writes_temporary_file_is_written_again_and_no_other_file_is_touched() {
        let mut disk = Folding::default();
        let header = HEADER.as_bytes();
        let left = temporary_path(Path::new("out/foo.luau"), "tmp");
        disk.create_dir_all(Path::new("out/systems")).unwrap();
        disk.write(&left, &header[..9]).unwrap();
        let started = [header, b"return"].concat();
        let mine: [(&str, &[u8]); 6] = [
            ("out/foo.luau.crucible-write-41-1", header),
            ("out/systems/bar.luau.crucible-write-7-1", &started),
            ("out/notes.luau.crucible-write-41-1", b"mine"),
            ("out/data.json.crucible-write-41-1", header),
            ("out/.crucible-0000000000000000.tmp", header),
            ("out/.crucible-build-41-1", b"mine"),
        ];
        for (path, bytes) in mine {
            disk.write(Path::new(path), bytes).unwrap();
        }

        let modules = [module("foo.luau", "return 1\n")];
        let changes = write_to(&mut disk, Path::new("out"), &modules).unwrap();
        assert_eq!(changes, Written { written: 1, renamed: 0, removed: 0 });
        assert!(disk.read(&left).is_err(), "the write renamed its temporary file over the module");
        assert_eq!(disk.read(Path::new("out/foo.luau")).unwrap(), modules[0].source.as_bytes());
        for (path, bytes) in mine {
            assert_eq!(disk.read(Path::new(path)).unwrap(), bytes, "{path} stays");
        }
    }

    #[test]
    fn a_second_build_writing_a_module_fails_and_touches_nothing() {
        let mut disk = Folding::default();
        let old = [module("foo.luau", "return 1\n")];
        write_to(&mut disk, Path::new("out"), &old).unwrap();
        disk.busy = true;
        let error = write_to(&mut disk, Path::new("out"), &[module("foo.luau", "return 2\n")]).unwrap_err();
        assert_eq!(error.error.kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(error.path, Path::new("out/foo.luau"));
        assert_eq!(disk.names("out"), ["foo.luau"]);
        assert_eq!(disk.read(Path::new("out/foo.luau")).unwrap(), old[0].source.as_bytes());
    }

    #[test]
    fn a_refused_rename_empties_the_temporary_file_before_the_module_is_written_in_place() {
        let mut disk = Folding::default();
        let old = module("foo.luau", "return 1\n");
        write_to(&mut disk, Path::new("out"), std::slice::from_ref(&old)).unwrap();
        let new = [module("foo.luau", &format!("return {}1\n", "1 + ".repeat(40)))];
        disk.capacity = Some(old.source.len() + new[0].source.len());
        disk.refused = Some("out/foo.luau".to_owned());
        assert_eq!(
            write_to(&mut disk, Path::new("out"), &new).unwrap(),
            Written { written: 1, renamed: 0, removed: 0 }
        );
        assert_eq!(disk.read(Path::new("out/foo.luau")).unwrap(), new[0].source.as_bytes());
        assert_eq!(disk.names("out"), ["foo.luau"]);
    }

    #[test]
    fn a_place_the_rename_cannot_replace_is_written_in_place_once_its_temporary_file_is_gone() {
        let (place, temporary) = (Path::new("build/start.rbxl"), temporary_path(Path::new("build/start.rbxl"), "rbxl"));
        let (old, new) = ([1; 10], [2; 30]);
        let mut disk = Folding {
            capacity: Some(old.len() + new.len()),
            refused: Some("build/start.rbxl".to_owned()),
            ..Folding::default()
        };
        disk.create_dir_all(Path::new("build")).unwrap();
        disk.put(place, &old).unwrap();
        disk.put(&temporary, &new).unwrap();
        publish_on(&mut disk, &temporary, place).unwrap();
        assert_eq!(disk.read(place).unwrap(), new);
        assert_eq!(disk.names("build"), ["start.rbxl"]);
    }

    #[test]
    fn a_link_or_a_read_only_file_is_written_in_place() {
        let mut disk = Folding::default();
        let old = [module("foo.luau", "return 1\n"), module("bar.luau", "return 2\n")];
        write_to(&mut disk, Path::new("out"), &old).unwrap();
        disk.remove_file(Path::new("out/foo.luau")).unwrap();
        disk.put(Path::new("elsewhere/foo.luau"), old[0].source.as_bytes()).unwrap();
        disk.links.insert(key(Path::new("out/foo.luau")), "elsewhere/foo.luau".to_owned());
        disk.read_only.insert(key(Path::new("out/bar.luau")));
        let new = [module("foo.luau", "return 3\n"), module("bar.luau", "return 4\n")];
        let error = write_to(&mut disk, Path::new("out"), &new).unwrap_err();
        assert_eq!(
            (error.path.as_path(), error.error.kind()),
            (Path::new("out/bar.luau"), io::ErrorKind::PermissionDenied)
        );
        assert_eq!(disk.read(Path::new("elsewhere/foo.luau")).unwrap(), new[0].source.as_bytes(), "the target changes");
        assert!(disk.links.contains_key("out/foo.luau"), "the link stays");
        assert_eq!(disk.read(Path::new("out/bar.luau")).unwrap(), old[1].source.as_bytes());
        assert_eq!(disk.names("out"), ["bar.luau"], "no temporary file is made");

        let temporary = |place: &str| temporary_path(Path::new(place), "rbxl");
        disk.put(Path::new("elsewhere/start.rbxl"), b"old").unwrap();
        disk.links.insert(key(Path::new("build/start.rbxl")), "elsewhere/start.rbxl".to_owned());
        disk.put(&temporary("build/start.rbxl"), b"new").unwrap();
        publish_on(&mut disk, &temporary("build/start.rbxl"), Path::new("build/start.rbxl")).unwrap();
        assert_eq!(disk.read(Path::new("elsewhere/start.rbxl")).unwrap(), b"new", "the target changes");
        assert!(disk.links.contains_key("build/start.rbxl"), "the link stays");
        disk.put(Path::new("build/arena.rbxl"), b"old").unwrap();
        disk.read_only.insert(key(Path::new("build/arena.rbxl")));
        disk.put(&temporary("build/arena.rbxl"), b"new").unwrap();
        let error = publish_on(&mut disk, &temporary("build/arena.rbxl"), Path::new("build/arena.rbxl")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(disk.read(Path::new("build/arena.rbxl")).unwrap(), b"old");
        assert_eq!(disk.names("build"), ["arena.rbxl"], "no temporary file stays");
    }

    #[test]
    fn the_temporary_file_is_named_by_its_destination_alone_and_is_short() {
        let long = "m".repeat(250) + ".luau";
        let cases = [
            ("out/foo.luau", "tmp"),
            ("other/foo.luau", "tmp"),
            ("out/Foo.luau", "tmp"),
            ("out/foo.luau", "rbxl"),
            ("out/start.rbxl", "rbxl"),
            (long.as_str(), "tmp"),
            ("out/x", ""),
        ];
        let mut names = Vec::new();
        for (path, extension) in cases {
            let temporary = temporary_path(Path::new(path), extension);
            assert_eq!(temporary.parent(), Path::new(path).parent(), "{path} keeps its folder");
            let name = temporary.file_name().unwrap().to_str().unwrap().to_owned();
            let dotted = if extension.is_empty() { 0 } else { extension.len() + 1 };
            assert!(name.starts_with(".crucible-") && name.len() == 26 + dotted, "{name}");
            assert_eq!(Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or(""), extension, "{name}");
            names.push(name);
        }
        assert_eq!(names[0], names[1], "the folder does not change the name");
        assert_ne!(names[0], names[2]);
        assert_ne!(names[0], names[3]);
    }

    /// An empty folder in the system's temporary folder.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crucible-write-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn listed(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_module_whose_name_is_as_long_as_a_file_name_can_be_is_written_and_replaced() {
        let out = scratch("long");
        let name = "m".repeat(250) + ".luau";
        for body in ["return 1\n", "return 2\n"] {
            let modules = [module(&name, body)];
            assert_eq!(super::write(&out, &modules).unwrap().written, 1);
            assert_eq!(std::fs::read(out.join(&name)).unwrap(), modules[0].source.as_bytes());
        }
        assert_eq!(listed(&out), [name]);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_module_another_build_is_writing_fails_until_that_build_lets_go() {
        let out = scratch("busy");
        std::fs::write(out.join("foo.luau"), "old").unwrap();
        let temporary = temporary_path(&out.join("foo.luau"), "tmp");
        let mut other = Os.open_temporary(&temporary).unwrap();
        Os.write_temporary(&mut other, b"half a mod").unwrap();
        let modules = [module("foo.luau", "return 1\n")];
        let error = super::write(&out, &modules).unwrap_err();
        assert_eq!(error.error.kind(), io::ErrorKind::ResourceBusy, "{error}");
        assert_eq!(std::fs::read(out.join("foo.luau")).unwrap(), b"old");
        drop(other);
        assert_eq!(std::fs::read(&temporary).unwrap(), b"half a mod", "the other build's file stays as it was");
        assert_eq!(super::write(&out, &modules).unwrap().written, 1);
        assert_eq!(std::fs::read(out.join("foo.luau")).unwrap(), modules[0].source.as_bytes());
        assert_eq!(listed(&out), ["foo.luau"]);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_folder_where_a_module_goes_fails_the_write_and_stays() {
        let out = scratch("folder");
        std::fs::create_dir_all(out.join("foo.luau")).unwrap();
        std::fs::write(out.join("foo.luau/notes.txt"), "mine").unwrap();
        let error = super::write(&out, &[module("foo.luau", "return 1\n")]).unwrap_err();
        assert_eq!(error.path, out.join("foo.luau"), "the error names the module");
        assert_eq!(std::fs::read(out.join("foo.luau/notes.txt")).unwrap(), b"mine");
        assert_eq!(listed(&out), ["foo.luau"], "the temporary file goes");
        let _ = std::fs::remove_dir_all(&out);
    }

    #[cfg(windows)]
    #[test]
    fn a_module_another_program_holds_without_delete_sharing_is_written_in_place() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        let out = scratch("held");
        std::fs::write(out.join("foo.luau"), "old").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(out.join("foo.luau"))
            .unwrap();
        let modules = [module("foo.luau", "return 1\n")];
        assert_eq!(super::write(&out, &modules).unwrap().written, 1);
        assert_eq!(std::fs::read(out.join("foo.luau")).unwrap(), modules[0].source.as_bytes());
        assert_eq!(listed(&out), ["foo.luau"], "the temporary file goes");
        drop(held);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Makes `path` read-only, and is false when this process can still write it, as root can on Unix.
    fn read_only(path: &Path) -> bool {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(path, permissions).unwrap();
        let writable = std::fs::OpenOptions::new().write(true).open(path).is_ok();
        if writable {
            eprintln!("skipped: this process can write read-only files");
        }
        !writable
    }

    fn writable(path: &Path) {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    /// Links `link` to `target`, and is false on Windows when this process may not make links.
    fn link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(target, link);
        match made {
            Ok(()) => true,
            // ERROR_PRIVILEGE_NOT_HELD: Windows lets only administrators and Developer Mode make links.
            Err(error) if cfg!(windows) && error.raw_os_error() == Some(1314) => {
                eprintln!("skipped: this process may not make links");
                false
            }
            Err(error) => panic!("cannot link {}: {error}", link.display()),
        }
    }

    #[test]
    fn a_read_only_module_keeps_its_bytes() {
        let out = scratch("read-only");
        let path = out.join("foo.luau");
        std::fs::write(&path, "old").unwrap();
        if read_only(&path) {
            let error = super::write(&out, &[module("foo.luau", "return 1\n")]).unwrap_err();
            assert_eq!(error.error.kind(), io::ErrorKind::PermissionDenied, "{error}");
            assert_eq!(error.path, path, "the error names the module");
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            assert_eq!(listed(&out), ["foo.luau"], "no temporary file stays");
        }
        writable(&path);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_module_that_is_a_link_writes_its_target_and_stays_a_link() {
        let dir = scratch("link");
        let (out, target) = (dir.join("out"), dir.join("elsewhere.luau"));
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(&target, "old").unwrap();
        if link(&target, &out.join("foo.luau")) {
            let modules = [module("foo.luau", "return 1\n")];
            assert_eq!(super::write(&out, &modules).unwrap().written, 1);
            assert_eq!(std::fs::read(&target).unwrap(), modules[0].source.as_bytes(), "the target holds the module");
            assert!(
                std::fs::symlink_metadata(out.join("foo.luau")).unwrap().file_type().is_symlink(),
                "the link stays"
            );
            assert_eq!(listed(&out), ["foo.luau"]);
            assert_eq!(super::write(&out, &modules).unwrap().written, 0, "the module is read through the link");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
