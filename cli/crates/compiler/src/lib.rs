//! The front end of Crucible's compiler: a tsconfig in, the checker's diagnostics or a fact table out.
//!
//! tsrs types stay inside two modules. `program` builds and checks the tsrs program, and `extract` reads the checker
//! into the plain data of `facts`. The lowering will read only the fact table.

mod extract;
mod facts;
mod macros;
mod program;

use std::fmt;
use std::path::Path;

pub use facts::{
    Attribute, Callee, Check, Class, ClassKind, Constant, Crucible, Dependency, Fact, Facts, Falsy, FileFacts, Import,
    Literal, Reason, Receiver, Site, Span, Unanswered,
};
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
