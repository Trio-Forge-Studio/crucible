mod extract;
mod facts;
mod lower;
pub mod luau;
mod macros;
mod program;
mod tree;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Read};
use std::path::Path;

pub use facts::{
    Attribute, Callee, Check, Class, ClassKind, Constant, Crucible, Dependency, Fact, Facts, Falsy, FileFacts, Import,
    Literal, Reason, Receiver, Site, Span, Unanswered,
};
pub use lower::Error;
use lower::ImportBase;
pub use macros::{Macro, MacroKind};

/// A configuration, syntax or type error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The absolute path with forward slashes, if the error has a location.
    pub path: Option<String>,
    /// One-based line and column, in bytes.
    pub line: u32,
    pub column: u32,
    pub code: i32,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path}({},{}): ", self.line, self.column)?;
        }
        write!(f, "error TS{}: {}", self.code, self.message)
    }
}

/// Builds the program for `tsconfig`, checks every file with one checker, and returns the fact table of every source
/// file it compiles. Any diagnostic stops the build before facts are extracted.
pub fn analyze(tsconfig: &Path) -> Result<Facts, Vec<Diagnostic>> {
    let checked = program::check(tsconfig)?;
    Ok(extract::extract(checked))
}

#[derive(Clone, Debug)]
pub struct Module {
    /// The output path relative to the output directory, with forward slashes. An `index` source is its
    /// folder's `init.luau`.
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub enum Failure {
    Check(Vec<Diagnostic>),
    Lower(Vec<Error>),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Check(diagnostics) => diagnostics.iter().try_for_each(|d| writeln!(f, "{d}")),
            Failure::Lower(errors) => errors.iter().try_for_each(|e| writeln!(f, "{e}")),
        }
    }
}

/// Any diagnostic or lowering error fails the whole compile, and no module is returned.
pub fn compile(tsconfig: &Path) -> Result<Vec<Module>, Failure> {
    let checked = program::check(tsconfig).map_err(Failure::Check)?;
    let files = checked.files();
    let paths: Vec<String> = files.iter().map(|f| f.path()).collect();
    let root = checked.root_dir().unwrap_or_else(|| common_dir(&paths));
    let facts = extract::extract(checked);
    let facts_by_path: HashMap<&str, &FileFacts> = facts.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut modules = Vec::with_capacity(files.len());
    let mut errors = Vec::new();
    for (file, path) in files.into_iter().zip(&paths) {
        let Some(file_facts) = facts_by_path.get(path.as_str()) else {
            let message = "the fact table has no entry for this file".to_owned();
            errors.push(Error { path: path.clone(), line: 1, column: 1, message });
            continue;
        };
        let relative = path.strip_prefix(root.as_str()).unwrap_or(path).trim_start_matches('/');
        let stem = relative.rsplit_once('.').map_or(relative, |(stem, _)| stem);
        let (stem, import_base) = match stem.strip_suffix("index") {
            Some(dir) if dir.is_empty() || dir.ends_with('/') => (format!("{dir}init"), ImportBase::Script),
            _ => (stem.to_owned(), ImportBase::ScriptParent),
        };
        let unanswered: Vec<&Unanswered> = facts.unanswered.iter().filter(|u| u.path == *path).collect();
        match lower::lower(file, file_facts, &unanswered, import_base) {
            Ok(chunk) => {
                let mut source = luau::print(&chunk);
                source.insert_str(0, HEADER);
                modules.push(Module { path: format!("{stem}.luau"), source });
            }
            Err(e) => errors.extend(e),
        }
    }
    if errors.is_empty() { Ok(modules) } else { Err(Failure::Lower(errors)) }
}

/// The first line of every module `compile` emits. `write` treats a `.luau` file as its own only when the file
/// starts with it.
const HEADER: &str = "-- Compiled with crucible-compiler\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub written: usize,
    pub removed: usize,
}

/// Writes each module under `out`, skipping files whose bytes already match, and counts the files it wrote. Then it
/// deletes every `.luau` file under `out` that starts with the compiler's header and is not one of `modules`, and
/// any folder that deletion leaves empty, and counts the files it deleted. Files the compiler did not emit, such as
/// `rbxtsc` output and declarations, stay.
pub fn write(out: &Path, modules: &[Module]) -> io::Result<Written> {
    let mut changes = Written::default();
    for module in modules {
        let path = out.join(&module.path);
        if std::fs::read(&path).is_ok_and(|old| old == module.source.as_bytes()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &module.source)?;
        changes.written += 1;
    }
    let current: HashSet<&str> = modules.iter().map(|m| m.path.as_str()).collect();
    changes.removed = remove_stale(out, "", &current)?;
    Ok(changes)
}

fn remove_stale(dir: &Path, prefix: &str, current: &HashSet<&str>) -> io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else { continue };
        let path = entry.path();
        let relative = format!("{prefix}{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let inside = remove_stale(&path, &format!("{relative}/"), current)?;
            if inside > 0 && std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
            removed += inside;
        } else if kind.is_file()
            && name.ends_with(".luau")
            && !current.contains(relative.as_str())
            && starts_with_header(&path)?
        {
            std::fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn starts_with_header(path: &Path) -> io::Result<bool> {
    let mut head = Vec::with_capacity(HEADER.len());
    std::fs::File::open(path)?.take(HEADER.len() as u64).read_to_end(&mut head)?;
    Ok(head == HEADER.as_bytes())
}

fn common_dir(paths: &[String]) -> String {
    let Some(first) = paths.first() else { return String::new() };
    let mut root = first.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
    while !paths.iter().all(|p| p.starts_with(&format!("{root}/"))) {
        match root.rsplit_once('/') {
            Some((parent, _)) => root = parent.to_owned(),
            None => return String::new(),
        }
    }
    root
}
