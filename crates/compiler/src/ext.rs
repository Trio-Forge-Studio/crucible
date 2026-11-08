//! The seam a framework transform plugs into. The compiler runs its passes and asks each extension at fixed points:
//! once per source file after its own extraction, once after the whole program, and during lowering at each call
//! and after each class. Extraction hands out tsrs's checker and syntax tree directly, so an extension spells tsrs
//! types. Lowering hands out the compiler's own tree view and Luau AST. An extension keeps its own facts, keyed by
//! `SiteId`.

use std::path::Path;

use crucible_luau::{Expr, Name, Stmt};
use tsrs_ast::{Node as TsNode, SourceFile};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;

use crate::extract::Extractor;
use crate::facts::{ModulePath, Span};
use crate::lower::Lower;
use crate::packages::Packages;
use crate::project::{Game, absolute};
use crate::tree::Node;

pub trait Extension: Send {
    fn name(&self) -> &'static str;
    /// Runs after the compiler extracted `file`'s facts.
    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: P<SourceFile>);
    /// Runs once after every file was extracted.
    fn finish_program(&mut self, cx: &mut ProgramCx<'_>);
    /// Lowers a call the extension owns, or `None` to leave it to the compiler.
    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr>;
    /// Statements that follow a class declaration bound to the local `name`.
    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, name: &Name) -> Vec<Stmt>;
}

/// What a diagnostic reports. A rule renders as `CRU` and its number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    Rule(u16),
    /// The project's packages do not let Crucible compute a type id.
    Setup,
    /// The place cannot hold a module where the program needs it.
    Place,
}

impl Code {
    /// The TypeScript error number and the printed message.
    pub fn render(self, message: &str) -> (i32, String) {
        match self {
            Code::Rule(rule) => (90000 + i32::from(rule), format!("CRU{rule:03}: {message}")),
            Code::Setup => (90000, format!("Crucible: {message}")),
            Code::Place => (0, message.to_owned()),
        }
    }
}

/// A source file, numbered in the order the compiler extracts and lowers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// A syntax node, by file and span. Extraction and lowering compute the same id for the same node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SiteId {
    pub file: FileId,
    pub span: Span,
}

pub struct ExtractCx<'a> {
    pub(crate) x: &'a mut Extractor,
    pub(crate) file: FileId,
}

impl ExtractCx<'_> {
    pub fn checker(&mut self) -> &mut CheckerHandle {
        &mut self.x.checker
    }

    pub fn program(&self) -> &'static Program {
        self.x.crucible.program
    }

    pub fn file(&self) -> FileId {
        self.file
    }

    pub fn site(&self, n: P<TsNode>) -> SiteId {
        SiteId { file: self.file, span: self.span(n) }
    }

    pub fn span(&self, n: P<TsNode>) -> Span {
        let text = tsrs_ast::get_source_file_of_node(n).map_or("", |f| f.text());
        crate::tree::span(n, text)
    }

    /// Counts a type-dependent site the extension answered.
    pub fn count_site(&mut self) {
        self.x.facts.sites += 1;
    }

    /// Reports once per file, position and code.
    pub fn report(&mut self, at: P<TsNode>, code: Code, message: &str) {
        self.x.report(at, code, message);
    }

    pub fn packages(&mut self) -> &mut Packages {
        &mut self.x.crucible.packages
    }

    /// The place, when the program compiles as a game.
    pub fn game(&self) -> Option<&Game> {
        self.x.game.as_ref()
    }
}

pub struct ProgramCx<'a> {
    pub(crate) x: &'a mut Extractor,
}

impl ProgramCx<'_> {
    /// The instance path of the module that `file` declares or compiles to, when the program is a game that maps it.
    pub fn game_module(&self, file: &Path) -> Option<ModulePath> {
        self.x.game.as_ref()?.module(file).and_then(|rbx| absolute(&rbx))
    }

    /// Declares a module that `LowerCx::helper` reaches through the local `local`. A file that uses one of its
    /// helpers imports it, or fails with `missing` when `path` is `None`.
    pub fn helper_module(&mut self, local: &'static str, path: Option<ModulePath>, missing: &str) {
        self.x.facts.place.helpers.push(crate::facts::HelperModule { local, path, missing: missing.into() });
    }
}

pub struct LowerCx<'a, 'b> {
    pub(crate) l: &'a mut Lower<'b>,
}

impl LowerCx<'_, '_> {
    pub fn file(&self) -> FileId {
        self.l.file_id
    }

    pub fn site(&self, n: Node) -> SiteId {
        SiteId { file: self.l.file_id, span: n.span() }
    }

    pub fn expr(&mut self, n: Node) -> Expr {
        self.l.expr(n)
    }

    /// A macro call's arguments in order, with a last argument that may return several values cut to one.
    pub fn macro_arguments(&mut self, call: Node) -> Vec<Expr> {
        self.l.macro_arguments(call)
    }

    /// `local.name`, importing the helper module declared as `local` into this file.
    pub fn helper(&mut self, local: &'static str, name: &'static str) -> Expr {
        self.l.helper(local, name)
    }

    /// Lowers the node at `span` to `value` the next time the compiler lowers it.
    pub fn substitute(&mut self, span: Span, value: Expr) {
        self.l.substitutions.insert(span, value);
    }

    pub fn unsupported(&mut self, n: Node, what: &str) -> Expr {
        self.l.unsupported(n, what)
    }
}

pub use crate::lower::skip_parens_and_assertions;
