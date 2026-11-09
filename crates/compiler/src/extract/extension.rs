//! The extraction half of the extension seam. Each query answers from the checker that ran the full check, or from the
//! program's packages.

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use super::Extractor;
use super::report::Code;
use crate::Diagnostic;
use crate::extension::{
    HelperModule, IndexInfo, Manifest, Signature, SignatureKind, SiteId, Symbol, SymbolFlags, Type,
};
use crate::tree::{File, Node};

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
        self.x.packages.manifest(package)
    }

    /// Reports an error without a location, which fails the compile. `code` is TypeScript's error number, or 0.
    pub fn report(&mut self, code: i32, message: impl Into<String>) {
        self.x.facts.diagnostics.push(Diagnostic { path: None, line: 0, column: 0, code, message: message.into() });
    }

    /// Every file of the program, declarations included, in the program's order.
    pub fn files(&self) -> Vec<File> {
        self.x.packages.program.source_files().iter().map(|&f| File::new(f)).collect()
    }

    /// Whether the program's file is one of the language's default libraries.
    pub fn is_default_library(&self, file: File) -> bool {
        self.x.packages.program.is_source_file_default_library(file.raw().path())
    }

    /// Whether a `package.json` above a file of the program does not parse.
    pub fn metadata_invalid(&self) -> bool {
        self.x.packages.invalid_metadata
    }

    /// The `name` of the `package.json` nearest above `file`.
    pub fn package_name(&mut self, file: &Path) -> Option<String> {
        self.x.packages.package_name(file)
    }

    /// The folder of the `package.json` nearest above `file`.
    pub fn package_dir(&mut self, file: &Path) -> Option<PathBuf> {
        self.x.packages.package_of(file).map(|p| p.dir.clone())
    }

    /// The `name` of the `package.json` nearest above the file that has `n`.
    pub fn package_of(&mut self, n: Node) -> Option<String> {
        self.package_name(Path::new(&n.file().path()))
    }

    /// The package that has `file`, and the file's module path within it, which a source file and the declaration
    /// built from it share.
    pub fn module_path(&mut self, file: &Path) -> Option<(String, String)> {
        let (owner, module) = self.x.packages.module_path(file)?;
        Some((owner.json["name"].as_str()?.to_owned(), module))
    }

    /// The module path of the `entry` export of the package that has `file`.
    pub fn entry_module(&mut self, file: &Path, entry: &str) -> Option<String> {
        let owner = self.x.packages.package_of(file)?;
        self.x.packages.entry_module(&owner, entry)
    }

    /// The symbols a module file exports, or none for a global script.
    pub fn exports_of(&mut self, file: File) -> Vec<Symbol> {
        let Some(module) = file.raw().as_node().symbol() else { return Vec::new() };
        self.x.checker.get_exports_of_module_exported(module).into_iter().map(Symbol).collect()
    }

    pub fn symbol_at(&mut self, n: Node) -> Option<Symbol> {
        self.x.checker.get_symbol_at_location_exported(n.raw()).map(Symbol)
    }

    /// The symbol an import alias resolves to, or `symbol` when it is not an alias.
    pub fn aliased(&mut self, symbol: Symbol) -> Symbol {
        Symbol(super::report::unalias(&mut self.x.checker, symbol.0))
    }

    pub fn symbol_of_declaration(&mut self, declaration: Node) -> Option<Symbol> {
        self.x.checker.get_symbol_of_declaration_exported(declaration.raw()).map(Symbol)
    }

    /// The symbol `name` means at `at`, or in the global scope without `at`.
    pub fn resolve_name(&mut self, name: &str, at: Option<Node>, meaning: SymbolFlags) -> Option<Symbol> {
        self.x.checker.resolve_name_exported(name, at.map(|n| n.raw()), meaning.0, false).map(Symbol)
    }

    pub fn type_at(&mut self, n: Node) -> Type {
        Type(self.x.checker.get_type_at_location(n.raw()))
    }

    pub fn type_of_symbol(&mut self, symbol: Symbol) -> Type {
        Type(self.x.checker.get_type_of_symbol_exported(symbol.0))
    }

    /// The type a class, interface or type alias declares.
    pub fn declared_type_of_symbol(&mut self, symbol: Symbol) -> Type {
        Type(self.x.checker.get_declared_type_of_symbol_exported(symbol.0))
    }

    pub fn type_from_type_node(&mut self, n: Node) -> Type {
        Type(self.x.checker.get_type_from_type_node_exported(n.raw()))
    }

    /// The type as TypeScript prints it in a message.
    pub fn type_text(&mut self, t: Type) -> String {
        self.x.checker.type_to_string_exported(t.0)
    }

    /// A property's type without the `undefined` its optionality adds.
    pub fn non_missing_type_of_symbol(&mut self, symbol: Symbol) -> Type {
        Type(self.x.checker.get_non_missing_type_of_symbol_exported(symbol.0))
    }

    pub fn non_nullable_type(&mut self, t: Type) -> Type {
        Type(self.x.checker.get_non_nullable_type(t.0))
    }

    pub fn index_infos_of_type(&mut self, t: Type) -> Vec<IndexInfo> {
        let infos = self.x.checker.get_index_infos_of_type_exported(t.0);
        infos.iter().map(|i| IndexInfo { key: Type(i.key_type()), value: Type(i.value_type()) }).collect()
    }

    pub fn is_array_type(&mut self, t: Type) -> bool {
        self.x.checker.is_array_type_exported(t.0)
    }

    /// The module a module specifier names.
    pub fn resolve_external_module(&mut self, specifier: Node) -> Option<Symbol> {
        self.x.checker.resolve_external_module_name_exported(specifier.raw(), None).map(Symbol)
    }

    /// The folder of the tsconfig the program was built from.
    pub fn config_dir(&self) -> PathBuf {
        self.x.config_dir.clone()
    }

    /// Whether the program compiles as a game, whose instance map places its output.
    pub fn is_game(&self) -> bool {
        self.x.game.is_some()
    }

    /// Where a game writes the output of a source file under the source root.
    pub fn out_path(&self, source: &Path) -> Option<PathBuf> {
        self.x.game.as_ref()?.out_path(source)
    }

    /// The output folder that a folder under a game's source root maps to.
    pub fn out_folder(&self, folder: &Path) -> Option<PathBuf> {
        self.x.game.as_ref()?.out_folder(folder)
    }

    /// The instance a game's output file or folder becomes, from the place's root. A `.server` or `.client` suffix
    /// and an `init` module name the script's instance, as Rojo does.
    pub fn instance_path(&self, out: &Path) -> Option<Vec<String>> {
        self.x.game.as_ref()?.instances.rbx_path(out)
    }

    pub fn base_types(&mut self, t: Type) -> Vec<Type> {
        self.x.checker.get_base_types_exported(t.0).iter().map(|&t| Type(t)).collect()
    }

    pub fn type_arguments(&mut self, t: Type) -> Vec<Type> {
        self.x.checker.get_type_arguments_exported(t.0).iter().map(|&t| Type(t)).collect()
    }

    pub fn property_of_type(&mut self, t: Type, name: &str) -> Option<Symbol> {
        self.x.checker.get_property_of_type_exported(t.0, name).map(Symbol)
    }

    pub fn properties_of_type(&mut self, t: Type) -> Vec<Symbol> {
        self.x.checker.get_properties_of_type_exported(t.0).iter().map(|&s| Symbol(s)).collect()
    }

    pub fn is_type_assignable_to(&mut self, source: Type, target: Type) -> bool {
        self.x.checker.is_type_assignable_to_exported(source.0, target.0)
    }

    pub fn signatures_of_type(&mut self, t: Type, kind: SignatureKind) -> Vec<Signature> {
        let kind = match kind {
            SignatureKind::Call => tsrs_checker::SignatureKind::Call,
            SignatureKind::Construct => tsrs_checker::SignatureKind::Construct,
        };
        self.x.checker.get_signatures_of_type_exported(t.0, kind).iter().map(|&s| Signature(s)).collect()
    }

    pub fn signature_from_declaration(&mut self, declaration: Node) -> Signature {
        Signature(self.x.checker.get_signature_from_declaration_exported(declaration.raw()))
    }

    /// The signature a call resolves to.
    pub fn signature_at(&mut self, call: Node) -> Signature {
        Signature(self.x.checker.get_resolved_signature_exported(call.raw()))
    }

    pub fn return_type(&mut self, signature: Signature) -> Type {
        Type(self.x.checker.get_return_type_of_signature_exported(signature.0))
    }
}

/// What an extension reads and reports about one source file. It answers every program query too.
pub struct ExtractCx<'x> {
    pub(super) program: ProgramCx<'x>,
    pub(super) file: u32,
    pub(super) extension: usize,
}

impl<'x> Deref for ExtractCx<'x> {
    type Target = ProgramCx<'x>;

    fn deref(&self) -> &ProgramCx<'x> {
        &self.program
    }
}

impl DerefMut for ExtractCx<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.program
    }
}

impl ExtractCx<'_> {
    /// The id of a node of the file being extracted.
    pub fn site(&self, n: Node) -> SiteId {
        SiteId { file: self.file, span: n.span() }
    }

    /// Reports an error at `at`, which fails the compile. A second report of the same code at the same position by
    /// the same extension is dropped. `code` is TypeScript's error number, or 0.
    pub fn report(&mut self, at: Node, code: i32, message: &str) {
        self.program.x.report(at.raw(), Code::Extension { extension: self.extension, code }, message);
    }
}
