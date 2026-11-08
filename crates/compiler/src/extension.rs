//! The seam a framework transform plugs into. The compiler calls each extension at fixed points of its passes: once
//! before extraction, after its own extraction of each source file, once after the last file, and while lowering, at
//! each call and after each class declaration. An extension reads syntax through the compiler's tree view and the
//! checker through the handles and queries below, and never names a tsrs type. It keeps its own facts, keyed by
//! `SiteId`.

use std::path::PathBuf;

use crucible_luau::{Expr, Name, Stmt};
use tsrs_ast::Symbol as TsSymbol;
use tsrs_checker::{Signature as TsSignature, Type as TsType};
use tsrs_core::P;

pub use crate::extract::extension::{ExtractCx, ProgramCx};
pub use crate::facts::{HelperModule, Span};
pub use crate::lower::extension::LowerCx;
pub use crate::tree::{File, Kind, Node, Nodes, Position};

pub trait Extension: Send {
    fn name(&self) -> &'static str;

    /// Runs once with the whole program checked, before any file is extracted.
    fn begin_program(&mut self, _cx: &mut ProgramCx<'_>) {}

    /// Runs after the compiler extracted the facts of `file`.
    fn extract_file(&mut self, _cx: &mut ExtractCx<'_>, _file: File) {}

    /// Runs once after the last file was extracted.
    fn finish_program(&mut self, _cx: &mut ProgramCx<'_>) {}

    /// Lowers a call the extension owns in place of the compiler, or returns `None` to leave it to the compiler and
    /// the next extension.
    fn lower_call(&self, _cx: &mut LowerCx<'_, '_>, _call: Node) -> Option<Expr> {
        None
    }

    /// Statements that follow the declaration of a class bound to the local `local`.
    fn lower_class_tail(&self, _cx: &mut LowerCx<'_, '_>, _class: Node, _local: &Name) -> Vec<Stmt> {
        Vec::new()
    }
}

/// A node of a source file the compiler compiles. Extraction and lowering name the same node with the same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SiteId {
    pub(crate) file: u32,
    pub(crate) span: Span,
}

/// A symbol the checker bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol(pub(crate) P<TsSymbol>);

impl Symbol {
    pub fn name(&self) -> &'static str {
        self.0.name()
    }

    pub fn declarations(&self) -> Vec<Node> {
        self.0.declarations().iter().filter_map(|&d| Node::of(d)).collect()
    }
}

/// A type the checker computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Type(pub(crate) P<TsType>);

/// A call or declaration signature the checker resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Signature(pub(crate) P<TsSignature>);

/// A package's `package.json`.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub path: PathBuf,
    pub json: serde_json::Value,
}
