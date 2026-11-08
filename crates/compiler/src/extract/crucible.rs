use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tsrs_ast::{Kind, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, Type, TypeFlags};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;

use super::Extractor;
use crate::declarations::{DeclarationKind, Side};
pub(crate) use crate::ext::Code;
use crate::facts::ClassKind;
use crate::macros::Macro;
use crate::packages::{Packages, entry_types};
use crate::program::ConfigHost;
use crate::project::normalize;
use crate::tree::{File, Position};

const PACKAGE: &str = "@trioforge/crucible";
/// The entries whose exports the network rules place by side.
const SIDE_ENTRIES: [&str; 3] = [".", "./server", "./client"];
const DECLARATION_MACROS: [Macro; 11] = [
    Macro::ToServer,
    Macro::ToClient,
    Macro::AskServer,
    Macro::Receive,
    Macro::Replicated,
    Macro::ReplicatedPerPlayer,
    Macro::ProfileStore,
    Macro::DataStore,
    Macro::ProfileMessage,
    Macro::Config,
    Macro::PlayerConfig,
];

fn is_declaration_macro(name: &str) -> bool {
    DECLARATION_MACROS.iter().any(|m| m.name() == name)
}

pub(crate) struct Api {
    pub(crate) program: &'static Program,
    exports: HashMap<P<Symbol>, &'static str>,
    macros: HashMap<P<Symbol>, Macro>,
    pub(crate) packages: Packages,
    kinds: HashMap<P<Type>, Option<ClassKind>>,
    /// The declarations of `@trioforge/crucible/ui`.
    pub ui: Option<PathBuf>,
    pub widths: HashSet<P<Symbol>>,
    pub entries: HashMap<PathBuf, Side>,
    pub entry_exports: HashMap<P<Symbol>, Side>,
}

impl Api {
    pub fn new(program: &'static Program, c: &mut Checker, host: ConfigHost) -> Api {
        let options = program.options();
        let root_dir = (!options.root_dir.is_empty()).then(|| PathBuf::from(&options.root_dir));
        let mut api = Api {
            program,
            exports: HashMap::new(),
            macros: HashMap::new(),
            packages: Packages::new(root_dir, host),
            kinds: HashMap::new(),
            ui: None,
            widths: HashSet::new(),
            entries: HashMap::new(),
            entry_exports: HashMap::new(),
        };
        let mut modules = Vec::new();
        let mut entries_read = false;
        for &file in program.source_files() {
            let path = PathBuf::from(file.file_name());
            let Some(owner) = api.packages.package_of(&path) else { continue };
            if owner.json["name"] != PACKAGE {
                continue;
            }
            if !entries_read {
                entries_read = true;
                api.ui = entry_types(&owner, "./ui");
            }
            widths(c, file.as_node(), &mut api.widths);
            let Some(module) = file.as_node().symbol() else { continue };
            let exports = c.get_exports_of_module_exported(module);
            for &symbol in &exports {
                let target = unalias(c, symbol);
                api.exports.entry(target).or_insert(symbol.name());
            }
            modules.push((path, exports));
        }
        for (path, exports) in modules {
            let Some((owner, module)) = api.packages.module_path(&path) else { continue };
            for entry in SIDE_ENTRIES {
                if api.packages.entry_module(&owner, entry).as_deref() != Some(module.as_str()) {
                    continue;
                }
                let side = match entry {
                    "./server" => Some(Side::Server),
                    "./client" => Some(Side::Client),
                    _ => None,
                };
                if let Some(side) = side {
                    api.entries.insert(normalize(&path), side);
                }
                for &symbol in &exports {
                    let target = unalias(c, symbol);
                    if let Some(side) = side
                        && (side == Side::Client || !api.entry_exports.contains_key(&target))
                    {
                        api.entry_exports.insert(target, side);
                    }
                    if let Some(&m) = DECLARATION_MACROS.iter().find(|m| m.name() == symbol.name()) {
                        api.macros.insert(target, m);
                    }
                }
            }
        }
        api
    }

    pub fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    /// Crucible's transform applies to the programs that use the package, not to the package's own sources.
    pub fn owns(&mut self, file: &Path) -> bool {
        self.packages.package_of(file).is_some_and(|p| p.json["name"] == PACKAGE)
    }

    pub fn exports_name(&self, name: &str) -> bool {
        self.exports.values().any(|&n| n == name)
    }

    fn name(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.exports.get(&unalias(c, symbol?)).copied()
    }

    fn macro_of(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<Macro> {
        self.macros.get(&unalias(c, symbol?)).copied()
    }

    /// The Crucible macro a call expands, which is none while package metadata cannot be read, because then the
    /// transform does not run.
    pub fn macro_at(&mut self, c: &mut CheckerHandle, call: P<Node>) -> Option<Macro> {
        if self.macros.is_empty() || self.packages.invalid_metadata() {
            return None;
        }
        let symbol = c.get_symbol_at_location_exported(call.as_call_expression().expression);
        if let Some(m) = self.macro_of(c, symbol) {
            return Some(m);
        }
        let name = c.get_resolved_signature_exported(call).declaration()?.name()?;
        let symbol = c.get_symbol_at_location_exported(name);
        self.macro_of(c, symbol)
    }

    pub fn package_name(&mut self, file: &Path) -> Option<String> {
        self.packages.package_of(file).and_then(|p| p.json["name"].as_str().map(str::to_owned))
    }

    /// Declarations of the language and the Roblox API, which every program shares.
    pub(super) fn is_library(&mut self, file: P<SourceFile>) -> bool {
        let name = file.file_name().replace('\\', "/");
        if name.contains("/node_modules/@rbxts/") || self.program.is_source_file_default_library(file.path()) {
            return true;
        }
        self.packages.package_of(Path::new(&name)).is_some_and(|p| {
            p.json["name"].as_str().is_some_and(|n| n.starts_with("@rbxts/") || n == "@trioforge/compiler-types")
        })
    }

    /// The nearest folder named for a side in the path from the package of `source` to `file`'s folder.
    pub(super) fn side(&mut self, source: &Path, file: &Path) -> Option<Side> {
        let root = self.packages.package_of(source).map(|p| p.dir.clone()).unwrap_or_default();
        let folder = file.parent()?;
        let relative = folder.strip_prefix(&root).unwrap_or(folder);
        relative.components().rev().find_map(|c| match c.as_os_str().to_str()? {
            "server" => Some(Side::Server),
            "client" => Some(Side::Client),
            "shared" => Some(Side::Shared),
            _ => None,
        })
    }
}

fn widths(c: &mut Checker, n: P<Node>, out: &mut HashSet<P<Symbol>>) {
    if n.kind() == Kind::VariableDeclaration
        && let Some(name) = n.name().filter(|name| name.kind() == Kind::Identifier && name.text() == "width")
        && let Some(symbol) = c.get_symbol_at_location_exported(name)
    {
        out.insert(symbol);
    }
    n.for_each_child(&mut |child| {
        widths(c, child, out);
        false
    });
}

pub(super) fn unalias(c: &mut Checker, s: P<Symbol>) -> P<Symbol> {
    if s.flags().intersects(SymbolFlags::Alias) { c.get_aliased_symbol(s) } else { s }
}

fn target(t: P<Type>) -> P<Type> {
    if is_reference(t) { t.target().unwrap_or(t) } else { t }
}

pub(super) fn is_reference(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Object) && t.object_flags().intersects(ObjectFlags::Reference)
}

/// A global script, or a `declare global` block in a module.
pub(super) fn is_global(declaration: P<Node>) -> bool {
    let Some(file) = tsrs_ast::get_source_file_of_node(declaration) else { return false };
    if !tsrs_ast::is_external_module(file) {
        return true;
    }
    let mut parent = declaration.parent();
    while let Some(p) = parent {
        if p.kind() == Kind::ModuleDeclaration && tsrs_ast::is_global_scope_augmentation(p) {
            return true;
        }
        parent = p.parent();
    }
    false
}

/// Whether a call calls `n`, through any parentheses.
pub(super) fn is_callee(mut n: P<Node>) -> bool {
    while let Some(parent) = n.parent().filter(|p| p.kind() == Kind::ParenthesizedExpression) {
        n = parent;
    }
    n.parent().is_some_and(|p| p.kind() == Kind::CallExpression && p.as_call_expression().expression == n)
}

/// The local names that a file's imports bind.
fn import_names(statement: P<Node>) -> Vec<P<Node>> {
    match statement.kind() {
        Kind::ImportEqualsDeclaration => vec![statement.as_import_equals_declaration().name],
        Kind::ImportDeclaration => {
            let Some(clause) = statement.as_import_declaration().import_clause else { return Vec::new() };
            let clause = clause.as_import_clause();
            let mut names: Vec<P<Node>> = clause.name.into_iter().collect();
            if let Some(named) = clause.named_bindings.filter(|n| n.kind() == Kind::NamedImports) {
                names.extend(named.as_named_imports().elements.nodes().iter().filter_map(|e| e.name()));
            }
            names
        }
        _ => Vec::new(),
    }
}

impl Extractor {
    pub(crate) fn report(&mut self, at: P<Node>, code: Code, message: &str) {
        let Some(file) = tsrs_ast::get_source_file_of_node(at) else { return };
        let span = crate::tree::span(at, file.text());
        let path = file.file_name().replace('\\', "/");
        if self.reported.insert((path, span.start, code)) {
            self.push_diagnostic(at, span, code, message);
        }
    }

    pub(super) fn report_span(&mut self, at: P<Node>, span: crate::facts::Span, code: Code, message: &str) {
        let Some(file) = tsrs_ast::get_source_file_of_node(at) else { return };
        let path = file.file_name().replace('\\', "/");
        if self.reported_classes.insert((path, span, code, message.to_owned())) {
            self.push_diagnostic(at, span, code, message);
        }
    }

    fn push_diagnostic(&mut self, at: P<Node>, span: crate::facts::Span, code: Code, message: &str) {
        let file = tsrs_ast::get_source_file_of_node(at).unwrap();
        let path = file.file_name().replace('\\', "/");
        let start = span.start;
        let Position { line, column } = File::new(file).position(start);
        let (number, message) = code.render(message);
        self.facts.diagnostics.push(crate::Diagnostic { path: Some(path), line, column, code: number, message });
    }

    pub(super) fn api(&mut self, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.crucible.name(&mut self.checker, symbol)
    }

    fn bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        let t = target(t);
        if !t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return Vec::new();
        }
        self.checker.get_base_types_exported(t).to_vec()
    }

    fn class_kind(&mut self, t: P<Type>, seen: &mut HashSet<P<Type>>) -> Option<ClassKind> {
        if !seen.insert(t) {
            return None;
        }
        match self.api(t.symbol()) {
            Some("Subsystem") => return Some(ClassKind::Subsystem),
            Some("CrucibleBehaviour") => return Some(ClassKind::Behaviour),
            _ => {}
        }
        for base in self.bases(t) {
            if let Some(kind) = self.class_kind(base, seen) {
                return Some(kind);
            }
        }
        None
    }

    pub(super) fn kind_of(&mut self, t: P<Type>) -> Option<ClassKind> {
        if let Some(&kind) = self.crucible.kinds.get(&t) {
            return kind;
        }
        let kind = self.class_kind(t, &mut HashSet::new());
        self.crucible.kinds.insert(t, kind);
        kind
    }

    pub(super) fn instance_type(&mut self, class: P<Node>) -> Option<P<Type>> {
        let symbol = self.checker.get_symbol_of_declaration_exported(class)?;
        Some(self.checker.get_declared_type_of_symbol_exported(symbol))
    }

    pub(super) fn is_library_declaration(&mut self, declaration: P<Node>) -> bool {
        tsrs_ast::get_source_file_of_node(declaration).is_some_and(|f| self.crucible.is_library(f))
    }

    pub(super) fn is_crucible_call(&mut self, n: P<Node>, m: Macro) -> bool {
        n.kind() == Kind::CallExpression && self.crucible.macro_at(&mut self.checker, n) == Some(m)
    }

    /// Runs Crucible's rules over one file of a program that uses the package and stores the facts its macros
    /// lower from.
    pub(super) fn crucible_file(&mut self, file: P<SourceFile>) {
        if self.crucible.owns(Path::new(file.file_name())) {
            return;
        }
        if self.crucible.packages.invalid_metadata() {
            self.report(
                file.as_node(),
                Code::Setup,
                "Cannot read package metadata. Check the project's package.json files.",
            );
            return;
        }
        if !self.crucible.is_empty() {
            let names = self.macro_names(file);
            self.crucible_walk(file.as_node(), file, &names);
        }
    }

    /// The names that can refer to a network or data macro in `file`: the macros' own, and the ones its imports give.
    fn macro_names(&mut self, file: P<SourceFile>) -> HashSet<&'static str> {
        let mut names: HashSet<&'static str> = DECLARATION_MACROS.iter().map(|m| m.name()).collect();
        for &statement in file.as_node().statements() {
            for name in import_names(statement) {
                let symbol = self.checker.get_symbol_at_location_exported(name);
                if self.api(symbol).is_some_and(is_declaration_macro) {
                    names.insert(name.text());
                }
            }
        }
        names
    }

    fn crucible_walk(&mut self, n: P<Node>, file: P<SourceFile>, names: &HashSet<&'static str>) {
        match n.kind() {
            Kind::ImportDeclaration | Kind::ExportDeclaration => self.entry_import(n, n.module_specifier()),
            Kind::ImportType => {
                let argument = n.as_import_type_node().argument;
                let literal = (argument.kind() == Kind::LiteralType).then(|| argument.as_literal_type_node().literal);
                self.entry_import(n, literal);
            }
            Kind::ExternalModuleReference => self.entry_import(n, Some(n.as_external_module_reference().expression())),
            Kind::CallExpression => self.crucible_call(n, file),
            Kind::StringLiteral => self.side_entry_specifier(n, file),
            Kind::BinaryExpression => self.process_receipt(n),
            Kind::ModuleDeclaration => self.augmentation(n),
            Kind::Identifier => {
                if names.contains(n.text()) {
                    let parent = n.parent();
                    let specifier =
                        parent.is_some_and(|p| matches!(p.kind(), Kind::ImportSpecifier | Kind::ExportSpecifier));
                    let access = parent.filter(|p| {
                        p.kind() == Kind::PropertyAccessExpression && p.as_property_access_expression().name == n
                    });
                    if !specifier && !is_callee(access.unwrap_or(n)) {
                        let symbol = self.checker.get_symbol_at_location_exported(n);
                        self.macro_value(n, symbol);
                    }
                }
                self.side_entry_reference(n, file);
            }
            Kind::ElementAccessExpression if !is_callee(n) => {
                let t = self.checker.get_type_at_location(n);
                self.macro_value(n, t.symbol());
            }
            _ => {}
        }
        n.for_each_child(&mut |child| {
            self.crucible_walk(child, file, names);
            false
        });
    }

    fn entry_import(&mut self, n: P<Node>, specifier: Option<P<Node>>) {
        if let Some(specifier) = specifier
            && specifier.kind() == Kind::StringLiteral
            && let Some(entry) = specifier.text().strip_prefix("@trioforge/crucible/")
            && !matches!(entry, "server" | "client" | "vide" | "ui" | "util/random" | "maths/spline")
        {
            self.report(
                n,
                Code::Rule(4),
                "Import only the root, server, client, vide, ui, util/random or maths/spline Crucible entries.",
            );
        }
    }

    fn crucible_call(&mut self, n: P<Node>, file: P<SourceFile>) {
        let callee = n.as_call_expression().expression;
        if callee.kind() == Kind::ImportKeyword {
            self.entry_import(n, n.arguments().first().copied());
            return;
        }
        match self.crucible.macro_at(&mut self.checker, n) {
            Some(Macro::Receive) => self.receive(n, file),
            Some(m) => {
                if let Some(kind) = DeclarationKind::of(m) {
                    self.declaration(n, kind, file);
                }
            }
            None => {}
        }
        self.profile_store_call(n, file);
    }

    /// CRU023: a network or data macro anywhere but a call's callee, an import or an export.
    fn macro_value(&mut self, at: P<Node>, symbol: Option<P<Symbol>>) {
        if let Some(name) = self.api(symbol).filter(|name| is_declaration_macro(name)) {
            let message = format!("{name} is a declaration macro and cannot be used as a value.");
            self.report(at, Code::Rule(23), &message);
        }
    }
}
