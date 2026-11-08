//! The extraction half of the extension seam. Each query answers from the checker that ran the full check, or from the
//! program's packages.

use std::path::Path;

use super::Extractor;
use super::crucible::{Code, unalias};
use crate::Diagnostic;
use crate::extension::{HelperModule, Manifest, Signature, SiteId, Symbol, Type};
use crate::tree::Node;

/// What an extension reads and reports about the whole program.
pub struct ProgramCx<'x> {
    pub(super) x: &'x mut Extractor,
}

impl ProgramCx<'_> {
    /// Declares a module that `LowerCx::helper` calls into. A second declaration of a local is ignored.
    pub fn declare_helper(&mut self, module: &HelperModule) {
        self.x.declare_helper(module);
    }

    /// The `package.json` of the program's first file from `package`.
    pub fn manifest(&mut self, package: &str) -> Option<Manifest> {
        self.x.crucible.manifest(package)
    }

    /// Reports an error without a location, which fails the compile. `code` is TypeScript's error number, or 0.
    pub fn report(&mut self, code: i32, message: impl Into<String>) {
        self.x.facts.diagnostics.push(Diagnostic { path: None, line: 0, column: 0, code, message: message.into() });
    }
}

/// What an extension reads and reports about one source file.
pub struct ExtractCx<'x> {
    pub(super) x: &'x mut Extractor,
    pub(super) file: u32,
}

impl ExtractCx<'_> {
    /// The id of a node of the file being extracted.
    pub fn site(&self, n: Node) -> SiteId {
        SiteId { file: self.file, span: n.span() }
    }

    /// Reports an error at `at`, once per file, position and code, which fails the compile. `code` is TypeScript's
    /// error number, or 0.
    pub fn report(&mut self, at: Node, code: i32, message: &str) {
        self.x.report(at.raw(), Code::Extension(code), message);
    }

    /// The `name` of the `package.json` nearest above the file that has `n`.
    pub fn package_of(&mut self, n: Node) -> Option<String> {
        let file = tsrs_ast::get_source_file_of_node(n.raw())?;
        self.x.crucible.package_name(Path::new(file.file_name()))
    }

    pub fn symbol_at(&mut self, n: Node) -> Option<Symbol> {
        self.x.checker.get_symbol_at_location_exported(n.raw()).map(Symbol)
    }

    /// The symbol an import alias resolves to, or `symbol` when it is not an alias.
    pub fn aliased(&mut self, symbol: Symbol) -> Symbol {
        Symbol(unalias(&mut self.x.checker, symbol.0))
    }

    pub fn type_at(&mut self, n: Node) -> Type {
        Type(self.x.checker.get_type_at_location(n.raw()))
    }

    /// The type as TypeScript prints it in a message.
    pub fn type_text(&mut self, t: Type) -> String {
        self.x.checker.type_to_string_exported(t.0)
    }

    /// The signature a call resolves to.
    pub fn signature_at(&mut self, call: Node) -> Signature {
        Signature(self.x.checker.get_resolved_signature_exported(call.raw()))
    }

    pub fn return_type(&mut self, signature: Signature) -> Type {
        Type(self.x.checker.get_return_type_of_signature_exported(signature.0))
    }
}
