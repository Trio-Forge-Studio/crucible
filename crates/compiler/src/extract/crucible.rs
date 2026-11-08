use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tsrs_ast::{Kind, ModifierFlags, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, Type, TypeFlags};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;
use tsrs_tsoptions as tsoptions;

use super::Extractor;
use crate::declarations::{DeclarationKind, Side};
use crate::facts::ClassKind;
use crate::macros::Macro;
use crate::program::ConfigHost;
use crate::project::normalize;
use crate::tree::{File, Position};

const PACKAGE: &str = "@trioforge/crucible";
const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];
/// `@trioforge/crucible/emit` exports a runtime `attribute` and runtime declaration functions that are not the macros.
const MACRO_ENTRIES: [&str; 3] = [".", "./server", "./client"];
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

struct Package {
    dir: PathBuf,
    json: serde_json::Value,
}

/// What a Crucible diagnostic reports.
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

pub struct Api {
    pub(crate) program: &'static Program,
    exports: HashMap<P<Symbol>, &'static str>,
    macros: HashMap<P<Symbol>, Macro>,
    /// The protected instance methods Crucible's own classes declare, which are its lifecycle hooks.
    pub(crate) hooks: HashSet<&'static str>,
    packages: HashMap<PathBuf, Option<Rc<Package>>>,
    kinds: HashMap<P<Type>, Option<ClassKind>>,
    configs: HashMap<PathBuf, Option<PathBuf>>,
    roots: HashMap<PathBuf, Option<PathBuf>>,
    root_dir: Option<PathBuf>,
    host: ConfigHost,
    /// A `package.json` above a program file does not parse.
    pub(crate) invalid_metadata: bool,
    /// The declarations of `@trioforge/crucible/emit`.
    pub(crate) emit: Option<PathBuf>,
    /// The declarations of `@trioforge/crucible/ui`.
    pub(crate) ui: Option<PathBuf>,
    pub(crate) widths: HashSet<P<Symbol>>,
    pub(crate) entries: HashMap<PathBuf, Side>,
    pub(crate) entry_exports: HashMap<P<Symbol>, Side>,
}

impl Api {
    pub(crate) fn new(program: &'static Program, c: &mut Checker, host: ConfigHost) -> Api {
        let options = program.options();
        let root_dir = (!options.root_dir.is_empty()).then(|| PathBuf::from(&options.root_dir));
        let mut api = Api {
            program,
            exports: HashMap::new(),
            macros: HashMap::new(),
            hooks: HashSet::new(),
            packages: HashMap::new(),
            kinds: HashMap::new(),
            configs: HashMap::new(),
            roots: HashMap::new(),
            root_dir,
            host,
            invalid_metadata: false,
            emit: None,
            ui: None,
            widths: HashSet::new(),
            entries: HashMap::new(),
            entry_exports: HashMap::new(),
        };
        let mut modules = Vec::new();
        for &file in program.source_files() {
            let path = PathBuf::from(file.file_name());
            let Some(owner) = api.package_of(&path) else { continue };
            if owner.json["name"] != PACKAGE {
                continue;
            }
            if api.emit.is_none() {
                api.emit = entry_types(&owner, "./emit");
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
            let Some((owner, module)) = api.module_path(&path) else { continue };
            for entry in MACRO_ENTRIES {
                if api.entry_module(&owner, entry).as_deref() != Some(module.as_str()) {
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
        for (&symbol, name) in &api.exports {
            if !OWN_CLASSES.contains(name) {
                continue;
            }
            for &declaration in symbol.declarations() {
                for &member in declaration.members() {
                    let flags = member.modifier_flags();
                    if member.kind() == Kind::MethodDeclaration
                        && flags.intersects(ModifierFlags::Protected)
                        && !flags.intersects(ModifierFlags::Static)
                        && let Some(name) = member.name()
                    {
                        api.hooks.insert(name.text());
                    }
                }
            }
        }
        api
    }

    pub fn symbols(&mut self, checker: &mut Checker, entry: &str, name: &str) -> HashSet<P<Symbol>> {
        let mut found = HashSet::new();
        for &file in self.program.source_files() {
            let path = Path::new(file.file_name());
            let Some(owner) = self.package_of(path) else { continue };
            if owner.json["name"] != PACKAGE {
                continue;
            }
            let Some((_, module)) = self.module_path(path) else { continue };
            if self.entry_module(&owner, entry).as_deref() != Some(module.as_str()) {
                continue;
            }
            let Some(symbol) = file.as_node().symbol() else { continue };
            for symbol in checker.get_exports_of_module_exported(symbol) {
                if symbol.name() == name {
                    found.insert(unalias(checker, symbol));
                }
            }
        }
        found
    }

    pub(crate) fn has_macros(&self) -> bool {
        !self.macros.is_empty() && !self.invalid_metadata
    }

    pub fn source_files(&self) -> &'static [P<SourceFile>] {
        self.program.source_files()
    }
    pub fn lifecycle_hooks(&self) -> &HashSet<&'static str> {
        &self.hooks
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    /// Crucible's transform applies to the programs that use the package, not to the package's own sources.
    pub(crate) fn owns(&mut self, file: &Path) -> bool {
        self.package_of(file).is_some_and(|p| p.json["name"] == PACKAGE)
    }

    pub(crate) fn exports_name(&self, name: &str) -> bool {
        self.exports.values().any(|&n| n == name)
    }

    pub(crate) fn name(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.exports.get(&unalias(c, symbol?)).copied()
    }

    pub(crate) fn macro_of(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<Macro> {
        self.macros.get(&unalias(c, symbol?)).copied()
    }

    /// The Crucible macro a call expands, which is none while package metadata cannot be read, because then the
    /// transform does not run.
    pub(crate) fn macro_at(&self, c: &mut CheckerHandle, call: P<Node>) -> Option<Macro> {
        if self.macros.is_empty() || self.invalid_metadata {
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

    pub(crate) fn package_name(&mut self, file: &Path) -> Option<String> {
        self.package_of(file).and_then(|p| p.json["name"].as_str().map(str::to_owned))
    }

    fn package_of(&mut self, file: &Path) -> Option<Rc<Package>> {
        let mut dir = file.parent()?.to_path_buf();
        let mut visited = Vec::new();
        let found = loop {
            if let Some(cached) = self.packages.get(&dir) {
                break cached.clone();
            }
            visited.push(dir.clone());
            let manifest = dir.join("package.json");
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                let json = serde_json::from_str(&text).unwrap_or_else(|_| {
                    self.invalid_metadata = true;
                    serde_json::Value::Null
                });
                break Some(Rc::new(Package { dir: dir.clone(), json }));
            }
            if !dir.pop() {
                break None;
            }
        };
        for dir in visited {
            self.packages.insert(dir, found.clone());
        }
        found
    }

    fn config_of(&mut self, file: &Path) -> Option<PathBuf> {
        let mut dir = file.parent()?.to_path_buf();
        let mut visited = Vec::new();
        let found = loop {
            if let Some(cached) = self.configs.get(&dir) {
                break cached.clone();
            }
            visited.push(dir.clone());
            let config = dir.join("tsconfig.json");
            if config.is_file() {
                break Some(config);
            }
            if !dir.pop() {
                break None;
            }
        };
        for dir in visited {
            self.configs.insert(dir, found.clone());
        }
        found
    }

    fn config_root(&mut self, config: &Path) -> Option<PathBuf> {
        if let Some(root) = self.roots.get(config) {
            return root.clone();
        }
        let (parsed, _) =
            tsoptions::get_parsed_command_line_of_config_file(&config.to_string_lossy(), None, None, &self.host, None);
        let root = parsed.and_then(|parsed| {
            let options = parsed.compiler_options()?;
            let base = config.parent()?;
            let dirs = parsed
                .file_names()
                .iter()
                .map(Path::new)
                .filter(|file| !is_declaration(file))
                .filter_map(|file| file.parent().map(Path::to_path_buf))
                .collect();
            let root_dirs: Vec<PathBuf> =
                options.root_dirs.iter().flatten().map(|dir| normalize(&base.join(dir))).collect();
            let root_dir = (!options.root_dir.is_empty()).then(|| normalize(&base.join(&options.root_dir)));
            source_root(root_dir, root_dirs, dirs)
        });
        self.roots.insert(config.to_path_buf(), root.clone());
        root
    }

    fn source_root(&mut self, owner: &Package, file: &Path) -> PathBuf {
        let root = match self.config_of(file) {
            Some(config) if self.package_of(&config).is_some_and(|p| p.dir == owner.dir) => self.config_root(&config),
            _ => None,
        };
        root.or_else(|| self.root_dir.clone()).unwrap_or_else(|| owner.dir.clone())
    }

    /// Declarations and the sources they were built from have the same module path, so a class has one type id in
    /// every package that sees it.
    fn module_path(&mut self, file: &Path) -> Option<(Rc<Package>, String)> {
        let owner = self.package_of(file)?;
        owner.json["name"].as_str()?;
        let root = if is_declaration(file) { declaration_root(&owner, file)? } else { self.source_root(&owner, file) };
        let relative = file.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
        let module = [".d.tsx", ".d.ts", ".tsx", ".ts"].iter().find_map(|ext| relative.strip_suffix(ext))?.to_owned();
        Some((owner, module))
    }

    fn entry_module(&mut self, owner: &Package, entry: &str) -> Option<String> {
        let exports = &owner.json["exports"];
        let mut types = Vec::new();
        types_condition(if entry == "." && exports.get(".").is_none() { exports } else { &exports[entry] }, &mut types);
        let types = types.into_iter().next().or_else(|| (entry == ".").then(|| package_types(owner)))?;
        self.module_path(&normalize(&owner.dir.join(types))).map(|(_, module)| module)
    }

    pub fn module_identity(&mut self, file: &str) -> Option<(String, String)> {
        let (owner, module) = self.module_path(Path::new(file))?;
        Some((owner.json["name"].as_str()?.to_owned(), module))
    }

    /// Declarations of the language and the Roblox API, which every program shares.
    pub(crate) fn is_library(&mut self, file: P<SourceFile>) -> bool {
        let name = file.file_name().replace('\\', "/");
        if name.contains("/node_modules/@rbxts/") || self.program.is_source_file_default_library(file.path()) {
            return true;
        }
        self.package_of(Path::new(&name)).is_some_and(|p| {
            p.json["name"].as_str().is_some_and(|n| n.starts_with("@rbxts/") || n == "@trioforge/compiler-types")
        })
    }

    /// The nearest folder named for a side in the path from the package of `source` to `file`'s folder.
    pub fn side(&mut self, source: &Path, file: &Path) -> Option<Side> {
        let root = self.package_of(source).map(|p| p.dir.clone()).unwrap_or_default();
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

fn is_declaration(file: &Path) -> bool {
    let name = file.to_string_lossy();
    name.ends_with(".d.ts") || name.ends_with(".d.tsx")
}

fn package_types(owner: &Package) -> String {
    owner.json["types"].as_str().or(owner.json["typings"].as_str()).unwrap_or("index.d.ts").to_owned()
}

fn declaration_root(owner: &Package, file: &Path) -> Option<PathBuf> {
    let exports = &owner.json["exports"];
    let mut entries = Vec::new();
    types_condition(if exports.get(".").is_some() { &exports["."] } else { exports }, &mut entries);
    let entry = entries
        .into_iter()
        .find(|entry| owner.dir.join(entry).parent().is_some_and(|dir| file.starts_with(normalize(dir))))
        .unwrap_or_else(|| package_types(owner));
    Some(normalize(owner.dir.join(entry).parent()?))
}

fn source_root(root_dir: Option<PathBuf>, root_dirs: Vec<PathBuf>, mut dirs: Vec<PathBuf>) -> Option<PathBuf> {
    if root_dir.is_some() {
        return root_dir;
    }
    dirs.extend(root_dirs);
    let mut root = dirs.first()?.clone();
    while !dirs.iter().all(|dir| dir.starts_with(&root)) {
        if !root.pop() {
            return None;
        }
    }
    Some(root)
}

/// The declaration file of the package's `entry` export.
fn entry_types(owner: &Package, entry: &str) -> Option<PathBuf> {
    let mut types = Vec::new();
    types_condition(&owner.json["exports"][entry], &mut types);
    types.first().map(|t| normalize(&owner.dir.join(t)))
}

fn types_condition(value: &serde_json::Value, out: &mut Vec<String>) {
    let Some(map) = value.as_object() else { return };
    match map.get("types").and_then(|t| t.as_str()) {
        Some(types) => out.push(types.to_owned()),
        None => map.values().for_each(|v| types_condition(v, out)),
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

pub fn unalias(c: &mut Checker, s: P<Symbol>) -> P<Symbol> {
    if s.flags().intersects(SymbolFlags::Alias) { c.get_aliased_symbol(s) } else { s }
}

fn target(t: P<Type>) -> P<Type> {
    if is_reference(t) { t.target().unwrap_or(t) } else { t }
}

pub fn is_reference(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Object) && t.object_flags().intersects(ObjectFlags::Reference)
}

/// A global script, or a `declare global` block in a module.
pub fn is_global(declaration: P<Node>) -> bool {
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

/// The matcher of `load` in `src/internal/boot.ts`, over an instance path instead of the instance tree.
pub fn is_callee(mut n: P<Node>) -> bool {
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
    pub fn report(&mut self, at: P<Node>, code: Code, message: &str) {
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

    pub fn api(&mut self, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.crucible.name(&mut self.checker, symbol)
    }

    pub fn bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        let t = target(t);
        if !t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return Vec::new();
        }
        self.checker.get_base_types_exported(t).to_vec()
    }

    /// The base types of a class or interface, or else of the class or interface that a reference instantiates.
    pub fn resolved_bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        if t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            self.checker.get_base_types_exported(t).to_vec()
        } else {
            self.bases(t)
        }
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

    pub fn kind_of(&mut self, t: P<Type>) -> Option<ClassKind> {
        if let Some(&kind) = self.crucible.kinds.get(&t) {
            return kind;
        }
        let kind = self.class_kind(t, &mut HashSet::new());
        self.crucible.kinds.insert(t, kind);
        kind
    }

    pub fn instance_type(&mut self, class: P<Node>) -> Option<P<Type>> {
        let symbol = self.checker.get_symbol_of_declaration_exported(class)?;
        Some(self.checker.get_declared_type_of_symbol_exported(symbol))
    }

    pub fn is_library_declaration(&mut self, declaration: P<Node>) -> bool {
        tsrs_ast::get_source_file_of_node(declaration).is_some_and(|f| self.crucible.is_library(f))
    }

    pub(super) fn is_crucible_call(&mut self, n: P<Node>, m: Macro) -> bool {
        n.kind() == Kind::CallExpression && self.crucible.macro_at(&mut self.checker, n) == Some(m)
    }

    /// Runs Crucible's rules over one file of a program that uses the package and stores the facts its macros and
    /// registered classes lower from.
    pub(crate) fn crucible_file(&mut self, file: P<SourceFile>) {
        if self.crucible.owns(Path::new(file.file_name())) {
            return;
        }
        if self.crucible.invalid_metadata {
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
