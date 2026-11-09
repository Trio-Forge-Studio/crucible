use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tsrs_ast::{Kind, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, Type, TypeFlags};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;

use super::Extractor;
use super::packages::Packages;
use crate::declarations::{DeclarationKind, Side};
use crate::facts::{ClassKind, Crucible, Site};
use crate::macros::Macro;
use crate::project::{Game as Place, RbxPath, normalize};
use crate::tree::{File, Position};

pub(crate) const PACKAGE: &str = "@trioforge/crucible";
/// `@trioforge/crucible/emit` exports runtime declaration functions that are not the macros.
const MACRO_ENTRIES: [(&str, &[Macro]); 3] = [(".", &[]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];
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

/// What a Crucible diagnostic reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Code {
    Rule(u16),
    /// The project's packages do not let Crucible compute a type id.
    Setup,
    /// The place cannot hold a module where the program needs it.
    Place,
    /// An extension's, with its TypeScript error number. It writes the whole message.
    Extension(i32),
}

impl Code {
    /// The TypeScript error number and the printed message.
    pub(super) fn render(self, message: &str) -> (i32, String) {
        match self {
            Code::Rule(rule) => (90000 + i32::from(rule), format!("CRU{rule:03}: {message}")),
            Code::Setup => (90000, format!("Crucible: {message}")),
            Code::Place => (0, message.to_owned()),
            Code::Extension(code) => (code, message.to_owned()),
        }
    }
}

pub(super) struct Api {
    pub(super) program: &'static Program,
    exports: HashMap<P<Symbol>, &'static str>,
    macros: HashMap<P<Symbol>, Macro>,
    kinds: HashMap<P<Type>, Option<ClassKind>>,
    /// A `package.json` above a program file does not parse.
    pub(super) invalid_metadata: bool,
    pub widths: HashSet<P<Symbol>>,
    pub entries: HashMap<PathBuf, Side>,
    pub entry_exports: HashMap<P<Symbol>, Side>,
}

impl Api {
    pub fn new(c: &mut Checker, packages: &mut Packages) -> Api {
        let program = packages.program;
        let mut api = Api {
            program,
            exports: HashMap::new(),
            macros: HashMap::new(),
            kinds: HashMap::new(),
            invalid_metadata: packages.invalid_metadata,
            widths: HashSet::new(),
            entries: HashMap::new(),
            entry_exports: HashMap::new(),
        };
        let mut modules = Vec::new();
        for &file in program.source_files() {
            let path = PathBuf::from(file.file_name());
            let Some(owner) = packages.package_of(&path) else { continue };
            if owner.json["name"] != PACKAGE {
                continue;
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
            let Some((owner, module)) = packages.module_path(&path) else { continue };
            for (entry, macros) in MACRO_ENTRIES {
                if packages.entry_module(&owner, entry).as_deref() != Some(module.as_str()) {
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
                    if let Some(&m) = macros.iter().chain(&DECLARATION_MACROS).find(|m| m.name() == symbol.name()) {
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
    pub fn macro_at(&self, c: &mut CheckerHandle, call: P<Node>) -> Option<Macro> {
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

/// The folder of a path as Node's `path.dirname` gives it, with `.` for a bare name.
/// The matcher of `load` in `src/internal/boot.ts`, over an instance path instead of the instance tree.
fn path_matches(rbx: &[String], segments: &[&str]) -> bool {
    match segments.split_first() {
        None => rbx.is_empty(),
        Some((&"**", rest)) => path_matches(rbx, rest) || (!rbx.is_empty() && path_matches(&rbx[1..], segments)),
        Some((segment, rest)) => {
            rbx.split_first().is_some_and(|(name, tail)| name_matches(name, segment) && path_matches(tail, rest))
        }
    }
}

fn name_matches(name: &str, pattern: &str) -> bool {
    if let Some(open) = pattern.find('{') {
        let Some(close) = pattern[open..].find('}').map(|at| open + at) else { return false };
        return pattern[open + 1..close]
            .split(',')
            .any(|part| name_matches(name, &format!("{}{part}{}", &pattern[..open], &pattern[close + 1..])));
    }
    let Some(star) = pattern.find('*') else { return name == pattern };
    let Some(rest) = name.strip_prefix(&pattern[..star]) else { return false };
    let tail = &pattern[star + 1..];
    rest.char_indices().map(|(at, _)| at).chain([rest.len()]).any(|at| name_matches(&rest[at..], tail))
}

fn dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(i) => trimmed[..i].trim_end_matches('/'),
        None if path.starts_with('/') => "/",
        None => ".",
    }
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

fn skip_wrappers(mut n: P<Node>) -> P<Node> {
    while matches!(
        n.kind(),
        Kind::ParenthesizedExpression | Kind::AsExpression | Kind::TypeAssertionExpression | Kind::SatisfiesExpression
    ) {
        let Some(inner) = n.expression() else { break };
        n = inner;
    }
    n
}

fn property_name(p: P<Node>) -> Option<&'static str> {
    let name = p.name()?;
    match name.kind() {
        Kind::Identifier | Kind::StringLiteral | Kind::NumericLiteral | Kind::NoSubstitutionTemplateLiteral => {
            Some(name.text())
        }
        _ => None,
    }
}

impl Extractor {
    pub(super) fn report(&mut self, at: P<Node>, code: Code, message: &str) {
        let Some(file) = tsrs_ast::get_source_file_of_node(at) else { return };
        let span = crate::tree::span(at, file.text());
        let path = file.file_name().replace('\\', "/");
        if self.reported.insert((path, span.start, code.render("").0)) {
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
        tsrs_ast::get_source_file_of_node(declaration).is_some_and(|f| self.is_library(f))
    }

    /// Declarations of the language and the Roblox API, which every program shares.
    fn is_library(&mut self, file: P<SourceFile>) -> bool {
        let name = file.file_name().replace('\\', "/");
        if name.contains("/node_modules/@rbxts/") || self.crucible.program.is_source_file_default_library(file.path()) {
            return true;
        }
        self.packages.package_of(Path::new(&name)).is_some_and(|p| {
            p.json["name"].as_str().is_some_and(|n| n.starts_with("@rbxts/") || n == "@trioforge/compiler-types")
        })
    }

    /// Crucible's transform applies to the programs that use the package, not to the package's own sources.
    fn owns(&mut self, file: &Path) -> bool {
        self.packages.package_of(file).is_some_and(|p| p.json["name"] == PACKAGE)
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

    pub(super) fn is_crucible_call(&mut self, n: P<Node>, m: Macro) -> bool {
        n.kind() == Kind::CallExpression && self.crucible.macro_at(&mut self.checker, n) == Some(m)
    }

    /// Runs Crucible's rules over one file of a program that uses the package and stores the facts its macros and
    /// registered classes lower from.
    pub(super) fn crucible_file(&mut self, file: P<SourceFile>) {
        if self.owns(Path::new(file.file_name())) {
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
            && !matches!(entry, "server" | "client" | "ui" | "util/random" | "maths/spline")
        {
            self.report(
                n,
                Code::Rule(4),
                "Import only the root, server, client, ui, util/random or maths/spline Crucible entries.",
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
            Some(Macro::Boot) => self.boot(n),
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

    fn boot(&mut self, call: P<Node>) {
        let options = call.arguments().first().map(|&a| skip_wrappers(a));
        let Some(options) = options.filter(|o| o.kind() == Kind::ObjectLiteralExpression) else {
            self.report(call, Code::Rule(6), "boot requires literal load globs.");
            return;
        };
        let properties = options.properties();
        let load = properties.iter().rposition(|&p| property_name(p) == Some("load"));
        if load.is_none() {
            self.report(options, Code::Rule(6), "boot requires a literal load property.");
        }
        for (i, &property) in properties.iter().enumerate() {
            let computed = property.name().is_some_and(|n| n.kind() == Kind::ComputedPropertyName);
            if property.kind() == Kind::SpreadAssignment || computed {
                if load.is_none_or(|load| i > load) {
                    self.report(
                        property,
                        Code::Rule(6),
                        "A spread or computed key after load can replace its literal globs.",
                    );
                }
                continue;
            }
            if property_name(property) != Some("load") {
                continue;
            }
            let entries = (property.kind() == Kind::PropertyAssignment)
                .then(|| property.initializer())
                .flatten()
                .filter(|i| i.kind() == Kind::ArrayLiteralExpression);
            let Some(entries) = entries else {
                self.report(property, Code::Rule(6), "load must be an array of string literals.");
                continue;
            };
            for &entry in entries.elements() {
                if entry.kind() != Kind::StringLiteral {
                    self.report(entry, Code::Rule(6), "load entry must be a string literal.");
                    continue;
                }
                self.site();
                match self.load_glob(entry.text()) {
                    Ok(glob) => {
                        let span = self.span(entry);
                        self.table.store(
                            span,
                            Site::Crucible,
                            |f| &mut f.crucible,
                            Box::new(Crucible::Load(glob.into())),
                        );
                    }
                    Err(message) => self.report(entry, Code::Rule(6), message),
                }
            }
        }
    }

    /// A load glob, relative to the package root, as the instance path pattern of the modules it matches.
    fn load_glob(&self, glob: &str) -> Result<String, &'static str> {
        let Some(game) = &self.game else {
            return Err("Cannot load the running compiler's Rojo resolver or project mapping.");
        };
        let prefix = match glob.find(['?', '*', '{', '[']) {
            None => format!("{}/", dirname(glob)),
            Some(at) => glob[..glob[..at].rfind('/').map_or(0, |slash| slash + 1)].to_owned(),
        };
        let rest = glob.get(prefix.len()..).unwrap_or("");
        let suffix = rest.strip_suffix(".tsx").or_else(|| rest.strip_suffix(".ts")).unwrap_or(rest);
        let folder = game.package.join(&prefix);
        let by_folder = game.out_folder(&folder).and_then(|out| game.rojo.rbx_path(&out));
        let mapped = match by_folder {
            Some(mapped) => mapped,
            None => match self.folder_by_files(game, &folder) {
                Ok(Some(mapped)) => mapped,
                Ok(None) => return Err("load prefix is outside the Rojo mapping."),
                Err(()) => return Err("load prefix maps to more than one Rojo folder."),
            },
        };
        let pattern = format!("{}/{suffix}", mapped.join("/"));
        let segments: Vec<&str> = pattern.split('/').collect();
        if !self.loads_module(game, &segments) {
            return Err("load glob matches no module.");
        }
        Ok(format!("crucible-path:{pattern}"))
    }

    /// Only TypeScript compiled through the transform can define a class for boot to construct, so Luau files and
    /// other Rojo mappings are not candidates.
    fn loads_module(&self, game: &Place, segments: &[&str]) -> bool {
        self.crucible.program.source_files().iter().any(|&file| {
            if file.is_declaration_file() {
                return false;
            }
            let Some(out) = game.out_path(&normalize(Path::new(file.file_name()))) else { return false };
            let script = out
                .file_stem()
                .and_then(|stem| Path::new(stem).extension())
                .is_some_and(|kind| kind == "server" || kind == "client");
            !script && game.rojo.rbx_path(&out).is_some_and(|rbx| path_matches(&rbx, segments))
        })
    }

    /// A Rojo project may map a folder's files one by one, as Rogen does before their outputs exist. Each such file
    /// names the instance of the folder that holds it. `Err` when the files disagree.
    fn folder_by_files(&self, game: &Place, folder: &Path) -> Result<Option<RbxPath>, ()> {
        let folder = normalize(folder);
        let mut found: Vec<RbxPath> = Vec::new();
        for &file in self.crucible.program.source_files() {
            if file.is_declaration_file() {
                continue;
            }
            let source = normalize(Path::new(file.file_name()));
            let Ok(relative) = source.strip_prefix(&folder) else { continue };
            let mut parts: Vec<String> =
                relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let Some(last) = parts.pop() else { continue };
            let stem = last.strip_suffix(".tsx").or_else(|| last.strip_suffix(".ts")).unwrap_or(&last);
            let name = stem.strip_suffix(".server").or_else(|| stem.strip_suffix(".client")).unwrap_or(stem);
            if name == "index" {
                continue;
            }
            parts.push(name.to_owned());
            let Some(rbx) = game.out_path(&source).and_then(|out| game.rojo.rbx_path(&out)) else { continue };
            if rbx.len() > parts.len() && rbx.ends_with(&parts) {
                let candidate = rbx[..rbx.len() - parts.len()].to_vec();
                if !found.contains(&candidate) {
                    found.push(candidate);
                }
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(found.pop()),
            _ => Err(()),
        }
    }
}
