mod entities;
pub mod extension;
mod extract;
mod facts;
mod include;
mod lower;
mod macros;
mod program;
mod project;
mod tree;
mod write;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use extension::Extension;

// The fact table is internal. Tests read it through `analyze`, which the `test-support` feature exports.
#[cfg(feature = "test-support")]
pub use facts::{
    Callee, Constant, Fact, Facts, Falsy, FileFacts, Import, Jsx, ModulePath, Place, Reason, Receiver, RuntimeLib,
    Site, Span, Unanswered,
};
#[cfg(not(feature = "test-support"))]
use facts::{FileFacts, Unanswered};
pub use include::write_include;
pub use lower::Error;
use lower::ImportBase;
#[cfg(feature = "test-support")]
pub use macros::{Macro, MacroKind};
pub use project::InstanceMap;
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
    /// A place. `instances` maps the output and `include`, the folder for the runtime library, which `write_include`
    /// fills. The place also holds `modules`, which the caller supplies.
    Game { instances: &'a InstanceMap, include: &'a Path, modules: &'a [VirtualModule<'a>] },
}

/// A module that game code imports by a specifier with no file behind it, such as one a build generates. The program
/// declares its types with `declare module`, as for any ambient module, and the compiler emits `source` at `path`.
#[derive(Clone, Copy, Debug)]
pub struct VirtualModule<'a> {
    /// The exact specifier imports name, such as `crucible:experience`.
    pub specifier: &'a str,
    /// The output path relative to the output directory, with forward slashes, such as `shared/experience.luau`.
    pub path: &'a str,
    pub source: &'a str,
}

/// Builds the program for `tsconfig`, checks every file with one checker, and returns the fact table of every source
/// file it compiles. Any diagnostic stops the build before facts are extracted.
#[cfg(feature = "test-support")]
pub fn analyze(tsconfig: &Path, target: Target<'_>) -> Result<Facts, Vec<Diagnostic>> {
    analyze_with(tsconfig, target, &mut [])
}

/// `analyze` with extensions, which keep their own facts.
#[cfg(feature = "test-support")]
pub fn analyze_with(
    tsconfig: &Path,
    target: Target<'_>,
    extensions: &mut [&mut dyn Extension],
) -> Result<Facts, Vec<Diagnostic>> {
    on_compiler_thread(|| {
        let checked = program::check(tsconfig)?;
        Ok(extract::extract(checked, target, extensions))
    })
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
    compile_with(tsconfig, target, &mut [])
}

/// `compile` with extensions, which the compiler calls in order at each of its hooks.
pub fn compile_with(
    tsconfig: &Path,
    target: Target<'_>,
    extensions: &mut [&mut dyn Extension],
) -> Result<Compiled, Failure> {
    on_compiler_thread(|| compile_on_thread(tsconfig, target, extensions))
}

/// tsrs's checker recurses with the source, so every caller needs the stack its CLI uses.
fn on_compiler_thread<T: Send>(run: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn_scoped(scope, run)
            .expect("cannot start the compiler thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn compile_on_thread(
    tsconfig: &Path,
    target: Target<'_>,
    extensions: &mut [&mut dyn Extension],
) -> Result<Compiled, Failure> {
    let checked = program::check(tsconfig).map_err(Failure::Check)?;
    let files = checked.files();
    let paths: Vec<String> = files.iter().map(|f| f.path()).collect();
    let root = checked.source_root();
    let out_dir = PathBuf::from(checked.out_dir().unwrap_or_else(|| root.clone()));
    let facts = extract::extract(checked, target, extensions);
    if !facts.diagnostics.is_empty() {
        return Err(Failure::Crucible(facts.diagnostics));
    }
    let extensions: Vec<&dyn Extension> = extensions.iter().map(|e| &**e).collect();
    let facts_by_path: HashMap<&str, (u32, &FileFacts)> =
        facts.files.iter().enumerate().map(|(id, f)| (f.path.as_str(), (id as u32, f))).collect();
    let mut modules = Vec::with_capacity(files.len());
    let mut errors = Vec::new();
    for (file, path) in files.into_iter().zip(&paths) {
        let Some(&(id, file_facts)) = facts_by_path.get(path.as_str()) else {
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
        match lower::lower(file, file_facts, &unanswered, import_base, &facts.place, id, &extensions) {
            Ok(chunk) => {
                let mut source = crucible_luau::print(&chunk);
                source.insert_str(0, HEADER);
                modules.push(Module { path: format!("{stem}.luau"), source });
            }
            Err(e) => errors.extend(e),
        }
    }
    if !errors.is_empty() {
        return Err(Failure::Lower(errors));
    }
    if let Target::Game { modules: virtual_modules, .. } = target {
        let compiled = modules.len();
        for module in virtual_modules {
            if let Some(other) = modules[..compiled].iter().find(|m| project::same_file(&m.path, module.path)) {
                let message = format!(
                    "the virtual module `{}` has the output path {}, which collides with the compiled module {}",
                    module.specifier, module.path, other.path
                );
                return Err(Failure::Crucible(vec![Diagnostic { path: None, line: 0, column: 0, code: 0, message }]));
            }
            modules.push(Module { path: module.path.to_owned(), source: format!("{HEADER}{}", module.source) });
        }
    }
    Ok(Compiled { out_dir, modules })
}
