//! Crucible's compiler: a tsconfig in, the checker's diagnostics or Luau modules out.
//!
//! tsrs types stay inside three modules. `program` builds and checks the tsrs program, `extract` reads the checker
//! into the plain data of `facts`, and `tree` is the syntax view the lowering reads. The lowering reads the tree
//! and the fact table, never the checker, and builds the `luau` tree that the printer writes.

mod extract;
mod facts;
mod lower;
pub mod luau;
mod macros;
mod program;
mod tree;

use std::fmt;
use std::io;
use std::path::Path;

pub use facts::{
    Attribute, Callee, Check, Class, ClassKind, Constant, Crucible, Dependency, Fact, Facts, Falsy, FileFacts, Import,
    Literal, Reason, Receiver, Site, Span, Unanswered,
};
pub use lower::Error;
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

/// One emitted Luau module.
#[derive(Clone, Debug)]
pub struct Module {
    /// The output path relative to the output directory, with forward slashes. An `index` source is its
    /// folder's `init.luau`.
    pub path: String,
    pub source: String,
}

/// Why a compile wrote nothing.
#[derive(Clone, Debug)]
pub enum Failure {
    /// The program has configuration, syntax or type errors.
    Check(Vec<Diagnostic>),
    /// The lowering refused constructs or sites it cannot answer.
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

/// Checks the program for `tsconfig`, extracts its facts, and lowers every source file to Luau. Any diagnostic or
/// lowering error fails the whole compile.
pub fn compile(tsconfig: &Path) -> Result<Vec<Module>, Failure> {
    let checked = program::check(tsconfig).map_err(Failure::Check)?;
    let files = checked.files();
    let paths: Vec<String> = files.iter().map(|f| f.path()).collect();
    let root = checked.root_dir().unwrap_or_else(|| common_dir(&paths));
    let facts = extract::extract(checked);
    let mut modules = Vec::with_capacity(files.len());
    let mut errors = Vec::new();
    for ((file, path), file_facts) in files.into_iter().zip(&paths).zip(&facts.files) {
        debug_assert_eq!(*path, file_facts.path);
        let relative = path.strip_prefix(root.as_str()).unwrap_or(path).trim_start_matches('/');
        let stem = relative.rsplit_once('.').map_or(relative, |(stem, _)| stem);
        let (stem, init) = match stem.strip_suffix("index") {
            Some(dir) if dir.is_empty() || dir.ends_with('/') => (format!("{dir}init"), true),
            _ => (stem.to_owned(), false),
        };
        let unanswered: Vec<&Unanswered> = facts.unanswered.iter().filter(|u| u.path == *path).collect();
        match lower::lower(file, file_facts, &unanswered, init) {
            Ok(chunk) => modules.push(Module { path: format!("{stem}.luau"), source: luau::print(&chunk) }),
            Err(e) => errors.extend(e),
        }
    }
    if errors.is_empty() { Ok(modules) } else { Err(Failure::Lower(errors)) }
}

/// Writes each module under `out`, skipping files whose bytes already match. Returns how many files it wrote.
pub fn write(out: &Path, modules: &[Module]) -> io::Result<usize> {
    let mut written = 0;
    for module in modules {
        let path = out.join(&module.path);
        if std::fs::read(&path).is_ok_and(|old| old == module.source.as_bytes()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &module.source)?;
        written += 1;
    }
    Ok(written)
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
