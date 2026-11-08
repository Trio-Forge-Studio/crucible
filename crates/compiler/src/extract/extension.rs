//! The extraction half of the extension facade. Each query answers from the checker that ran the full check, or from
//! the program's packages and place.

use std::path::{Path, PathBuf};

use tsrs_ast::SymbolFlags as TsSymbolFlags;
use tsrs_checker::SignatureKind;

use super::Extractor;
use super::crucible::{Code, unalias};
use crate::extension::{HelperModule, Node, Signature, SourceFile, Symbol, Type};
use crate::facts::Helper;
use crate::project::absolute;

pub struct ExtractCx<'x> {
    x: &'x mut Extractor,
}

impl Extractor {
    /// Resolves a helper module once per program. The first declaration of a local wins.
    pub(crate) fn declare_helper(&mut self, module: &'static HelperModule) {
        if self.facts.place.helpers.iter().any(|h| h.local == module.local) {
            return;
        }
        let path = match &self.game {
            Some(game) => self
                .crucible
                .entry_declaration(module.package, module.entry)
                .and_then(|declaration| game.module(&declaration))
                .and_then(|rbx| absolute(&rbx)),
            None => None,
        };
        self.facts.place.helpers.push(Helper { local: module.local, module: path, missing: module.missing });
    }
}

impl<'x> ExtractCx<'x> {
    pub(crate) fn new(x: &'x mut Extractor) -> Self {
        ExtractCx { x }
    }

    // Program, packages and place.

    /// Every file of the program, declarations included, in the program's order.
    pub fn source_files(&self) -> Vec<SourceFile> {
        self.x.crucible.program.source_files().iter().map(|&f| SourceFile(f)).collect()
    }

    /// The `name` of the `package.json` nearest above `path`.
    pub fn package_name(&mut self, path: impl AsRef<Path>) -> Option<String> {
        self.x.crucible.package_name(path.as_ref())
    }

    /// The folder of the `package.json` nearest above `path`.
    pub fn package_dir(&mut self, path: impl AsRef<Path>) -> Option<PathBuf> {
        self.x.crucible.package_dir(path.as_ref())
    }

    /// False when a `package.json` above a program file does not parse. The compiler reports it.
    pub fn package_metadata_valid(&self) -> bool {
        self.x.crucible.metadata_valid()
    }

    /// Whether `file` is the module that `entry` of its package's `exports` map names, such as `"./server"`.
    pub fn is_package_entry(&mut self, file: SourceFile, entry: &str) -> bool {
        self.x.crucible.is_package_entry(Path::new(&file.path()), entry)
    }

    /// `<package>/<module path>` of a file, the same for a source and the declaration built from it.
    pub fn module_id(&mut self, path: &str) -> Option<String> {
        self.x.crucible.module_id(Path::new(path))
    }

    /// A declaration of the language, the Roblox API or the compiler's types, which every program shares.
    pub fn is_library_declaration(&mut self, declaration: Node) -> bool {
        tsrs_ast::get_source_file_of_node(declaration.0).is_some_and(|f| self.x.crucible.is_library(f))
    }

    /// The folder the place's Rojo project maps the package from, or `None` outside a game.
    pub fn game_package_dir(&self) -> Option<PathBuf> {
        self.x.game.as_ref().map(|game| game.package.clone())
    }

    /// The output folder a source folder compiles into.
    pub fn out_folder(&self, folder: &Path) -> Option<PathBuf> {
        self.x.game.as_ref()?.out_folder(folder)
    }

    /// The output file a source file compiles into.
    pub fn out_path(&self, source: &Path) -> Option<PathBuf> {
        self.x.game.as_ref()?.out_path(source)
    }

    /// The instance path the Rojo project gives an output file or folder.
    pub fn instance_path(&self, out: &Path) -> Option<Vec<String>> {
        self.x.game.as_ref()?.rojo.rbx_path(out)
    }

    pub fn declare_helper(&mut self, module: &'static HelperModule) {
        self.x.declare_helper(module);
    }

    pub fn report(&mut self, at: Node, code: Code, message: &str) {
        self.x.report(at.0, code, message);
    }

    // Symbols.

    pub fn symbol_at(&mut self, node: Node) -> Option<Symbol> {
        self.x.checker.get_symbol_at_location_exported(node.0).map(Symbol)
    }

    /// The symbol an import or export alias stands for, or the symbol itself.
    pub fn unalias(&mut self, symbol: Symbol) -> Symbol {
        Symbol(unalias(&mut self.x.checker, symbol.0))
    }

    pub fn symbol_of_declaration(&mut self, declaration: Node) -> Option<Symbol> {
        self.x.checker.get_symbol_of_declaration_exported(declaration.0).map(Symbol)
    }

    /// The type a type name means at `at`, through the scopes and globals visible there.
    pub fn resolve_type_name(&mut self, name: &str, at: Node) -> Option<Symbol> {
        self.x.checker.resolve_name_exported(name, Some(at.0), TsSymbolFlags::Type, false).map(Symbol)
    }

    /// A module's exports, as the aliases the module declares.
    pub fn module_exports(&mut self, file: SourceFile) -> Vec<Symbol> {
        let Some(module) = file.0.as_node().symbol() else { return Vec::new() };
        self.x.checker.get_exports_of_module_exported(module).into_iter().map(Symbol).collect()
    }

    // Types.

    pub fn type_at(&mut self, node: Node) -> Type {
        Type(self.x.checker.get_type_at_location(node.0))
    }

    pub fn type_from_type_node(&mut self, node: Node) -> Type {
        Type(self.x.checker.get_type_from_type_node_exported(node.0))
    }

    /// The instance type of a class or interface symbol.
    pub fn declared_type(&mut self, symbol: Symbol) -> Type {
        Type(self.x.checker.get_declared_type_of_symbol_exported(symbol.0))
    }

    pub fn type_of_symbol(&mut self, symbol: Symbol) -> Type {
        Type(self.x.checker.get_type_of_symbol_exported(symbol.0))
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

    pub fn is_assignable(&mut self, source: Type, target: Type) -> bool {
        self.x.checker.is_type_assignable_to_exported(source.0, target.0)
    }

    // Signatures.

    pub fn construct_signatures(&mut self, t: Type) -> Vec<Signature> {
        self.x
            .checker
            .get_signatures_of_type_exported(t.0, SignatureKind::Construct)
            .iter()
            .map(|&s| Signature(s))
            .collect()
    }

    pub fn signature_of_declaration(&mut self, declaration: Node) -> Signature {
        Signature(self.x.checker.get_signature_from_declaration_exported(declaration.0))
    }

    pub fn return_type(&mut self, signature: Signature) -> Type {
        Type(self.x.checker.get_return_type_of_signature_exported(signature.0))
    }

    /// The declaration of the signature a call resolves to.
    pub fn resolved_signature_declaration(&mut self, call: Node) -> Option<Node> {
        self.x.checker.get_resolved_signature_exported(call.0).declaration().map(Node)
    }
}
