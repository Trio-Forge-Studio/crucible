mod classes;
mod declarations;
mod entities;
mod extract;
mod facts;
mod include;
mod lower;
pub mod luau;
mod macros;
mod program;
mod project;
mod tree;
mod write;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub use declarations::DeclarationKind;
pub use facts::{
    Attribute, Callee, Check, Class, ClassKind, ClassStyle, Constant, Crucible, Declaration, Dependency, Fact, Facts,
    Falsy, Field, FileFacts, Import, Jsx, KeyLiteral, Literal, MapKey, ModulePath, Place, Prop, Reason, Receiver,
    RuntimeLib, Schema, Site, Span, Tail, Unanswered,
};
pub use include::write_include;
pub use lower::Error;
use lower::ImportBase;
pub use macros::{Macro, MacroKind};
use write::HEADER;
pub use write::{Written, write};

/// A configuration, syntax, type or Crucible error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The absolute path with forward slashes, if the error has a location.
    pub path: Option<String>,
    /// One-based line and column, in bytes.
    pub line: u32,
    pub column: u32,
    /// TypeScript's code, 90000 plus the rule's number for a Crucible rule such as CRU001, and 0 for an error that
    /// has none.
    pub code: i32,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path}({},{}): ", self.line, self.column)?;
        }
        match self.code {
            0 => write!(f, "error: {}", self.message),
            code => write!(f, "error TS{code}: {}", self.message),
        }
    }
}

/// What a tsconfig compiles into. Relative paths resolve against the current folder, as the tsconfig's path does.
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    /// A library. Its modules reach each other relatively and take the runtime library from `_G[script]`.
    Package,
    /// A place. `project` is a Rojo project with a `DataModel` root that maps the output, and `include` is the folder
    /// it maps for the runtime library, which `write_include` fills.
    Game { project: &'a Path, include: &'a Path },
}

/// Builds the program for `tsconfig`, checks every file with one checker, and returns the fact table of every source
/// file it compiles. Any diagnostic stops the build before facts are extracted.
pub fn analyze(tsconfig: &Path, target: Target<'_>) -> Result<Facts, Vec<Diagnostic>> {
    let checked = program::check(tsconfig)?;
    Ok(extract::extract(checked, target))
}

/// The `outDir` that `tsconfig` sets, read without building its program, with the diagnostics of reading and parsing
/// it and every config it extends. The compile fails on any of them, but the `outDir` can still be read past them.
pub fn out_dir(tsconfig: &Path) -> (Option<PathBuf>, Vec<Diagnostic>) {
    let (dir, errors) = program::out_dir(tsconfig);
    (dir.map(PathBuf::from), errors)
}

#[derive(Clone, Debug)]
pub struct Compiled {
    /// The tsconfig's `outDir`, or the source root when it sets none. Each module's path is relative to it.
    pub out_dir: PathBuf,
    pub modules: Vec<Module>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    /// The output path relative to the output directory, with forward slashes. An `index` source is its
    /// folder's `init.luau`.
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub enum Failure {
    Check(Vec<Diagnostic>),
    /// Crucible's rules, and modules the game's place cannot reach.
    Crucible(Vec<Diagnostic>),
    Lower(Vec<Error>),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Check(diagnostics) | Failure::Crucible(diagnostics) => {
                diagnostics.iter().try_for_each(|d| writeln!(f, "{d}"))
            }
            Failure::Lower(errors) => errors.iter().try_for_each(|e| writeln!(f, "{e}")),
        }
    }
}

/// Any diagnostic, Crucible rule or lowering error fails the whole compile, and no module is returned.
pub fn compile(tsconfig: &Path, target: Target<'_>) -> Result<Compiled, Failure> {
    let checked = program::check(tsconfig).map_err(Failure::Check)?;
    let files = checked.files();
    let paths: Vec<String> = files.iter().map(|f| f.path()).collect();
    let root = checked.source_root();
    let out_dir = PathBuf::from(checked.out_dir().unwrap_or_else(|| root.clone()));
    let facts = extract::extract(checked, target);
    if !facts.diagnostics.is_empty() {
        return Err(Failure::Crucible(facts.diagnostics));
    }
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
        match lower::lower(file, file_facts, &unanswered, import_base, &facts.place) {
            Ok(chunk) => {
                let mut source = luau::print(&chunk);
                source.insert_str(0, HEADER);
                modules.push(Module { path: format!("{stem}.luau"), source });
            }
            Err(e) => errors.extend(e),
        }
    }
    if errors.is_empty() { Ok(Compiled { out_dir, modules }) } else { Err(Failure::Lower(errors)) }
}
