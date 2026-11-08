use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, SignatureKind, Type, TypeFlags};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;
use tsrs_tsoptions as tsoptions;

use super::{Extractor, types};
use crate::declarations::{DeclarationKind, Side};
use crate::facts::{Attribute, Check, Class, ClassKind, Crucible, Dependency, Literal, Site};
use crate::macros::Macro;
use crate::program::ConfigHost;
use crate::project::{Game as Place, RbxPath, normalize};
use crate::tree::{File, Position};

const PACKAGE: &str = "@trioforge/crucible";
const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];
/// `@trioforge/crucible/emit` exports a runtime `attribute` and runtime declaration functions that are not the macros.
const MACRO_ENTRIES: [(&str, &[Macro]); 3] =
    [(".", &[Macro::Attribute, Macro::Inject]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];
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
/// Lifecycle names from other frameworks that a Crucible class may mean as a hook.
const HOOK_LOOKALIKES: [&str; 5] = ["onInit", "onStart", "onTick", "onPhysics", "onRender"];

struct Package {
    dir: PathBuf,
    json: serde_json::Value,
}

/// What a Crucible diagnostic reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Code {
    Rule(u16),
    /// The project's packages do not let Crucible compute a type id.
    Setup,
    /// The place cannot hold a module where the program needs it.
    Place,
}

impl Code {
    /// The TypeScript error number and the printed message.
    pub(super) fn render(self, message: &str) -> (i32, String) {
        match self {
            Code::Rule(rule) => (90000 + i32::from(rule), format!("CRU{rule:03}: {message}")),
            Code::Setup => (90000, format!("Crucible: {message}")),
            Code::Place => (0, message.to_owned()),
        }
    }
}

/// Which generic instantiations a provided type or an `All` or `inject` target may use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generic {
    /// A type a class provides: only a behaviour's tag parameter may be instantiated.
    Provides,
    /// A type code resolves: only a behaviour's tag parameter, and only with `string`.
    Target,
}

/// When TypeScript 5's checker made a class or interface type, and whether the type has a `this` type.
#[derive(Clone, Copy)]
struct Made {
    turn: usize,
    this_type: bool,
}

pub(super) struct Api {
    pub(super) program: &'static Program,
    exports: HashMap<P<Symbol>, &'static str>,
    macros: HashMap<P<Symbol>, Macro>,
    /// The protected instance methods Crucible's own classes declare, which are its lifecycle hooks.
    hooks: HashSet<&'static str>,
    packages: HashMap<PathBuf, Option<Rc<Package>>>,
    kinds: HashMap<P<Type>, Option<ClassKind>>,
    configs: HashMap<PathBuf, Option<PathBuf>>,
    roots: HashMap<PathBuf, Option<PathBuf>>,
    root_dir: Option<PathBuf>,
    host: ConfigHost,
    /// A `package.json` above a program file does not parse.
    invalid_metadata: bool,
    /// When TypeScript 5 makes each class and interface type, computed when a union of instance classes needs it.
    made: Option<HashMap<P<Symbol>, Made>>,
    /// The declarations of `@trioforge/crucible/emit`.
    pub emit: Option<PathBuf>,
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
            hooks: HashSet::new(),
            packages: HashMap::new(),
            kinds: HashMap::new(),
            configs: HashMap::new(),
            roots: HashMap::new(),
            root_dir,
            host,
            invalid_metadata: false,
            made: None,
            emit: None,
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
            for (entry, macros) in MACRO_ENTRIES {
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
                    if let Some(&m) = macros.iter().chain(&DECLARATION_MACROS).find(|m| m.name() == symbol.name()) {
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

    pub fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    /// Crucible's transform applies to the programs that use the package, not to the package's own sources.
    pub fn owns(&mut self, file: &Path) -> bool {
        self.package_of(file).is_some_and(|p| p.json["name"] == PACKAGE)
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

    pub fn package_name(&mut self, file: &Path) -> Option<String> {
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

    fn type_id(&mut self, file: &str, name: &str) -> Option<Box<str>> {
        let (owner, module) = self.module_path(Path::new(file))?;
        let package = owner.json["name"].as_str()?;
        Some(format!("{package}/{module}:{name}").into())
    }

    /// Declarations of the language and the Roblox API, which every program shares.
    pub(super) fn is_library(&mut self, file: P<SourceFile>) -> bool {
        let name = file.file_name().replace('\\', "/");
        if name.contains("/node_modules/@rbxts/") || self.program.is_source_file_default_library(file.path()) {
            return true;
        }
        self.package_of(Path::new(&name)).is_some_and(|p| {
            p.json["name"].as_str().is_some_and(|n| n.starts_with("@rbxts/") || n == "@trioforge/compiler-types")
        })
    }

    /// The nearest folder named for a side in the path from the package of `source` to `file`'s folder.
    pub(super) fn side(&mut self, source: &Path, file: &Path) -> Option<Side> {
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

pub(super) fn unalias(c: &mut Checker, s: P<Symbol>) -> P<Symbol> {
    if s.flags().intersects(SymbolFlags::Alias) { c.get_aliased_symbol(s) } else { s }
}

fn target(t: P<Type>) -> P<Type> {
    if is_reference(t) { t.target().unwrap_or(t) } else { t }
}

pub(super) fn is_reference(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Object) && t.object_flags().intersects(ObjectFlags::Reference)
}

fn is_class_or_interface(d: P<Node>) -> bool {
    matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration)
}

fn is_concrete_class(d: P<Node>) -> bool {
    d.kind() == Kind::ClassDeclaration && !tsrs_ast::get_combined_modifier_flags(d).intersects(ModifierFlags::Abstract)
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

fn implemented(declaration: P<Node>) -> Vec<P<Node>> {
    let clauses = match declaration.kind() {
        Kind::ClassDeclaration => declaration.as_class_declaration().heritage_clauses(),
        Kind::ClassExpression => declaration.as_class_expression().heritage_clauses(),
        Kind::InterfaceDeclaration => declaration.as_interface_declaration().heritage_clauses(),
        _ => None,
    };
    clauses
        .map_or(&[][..], |l| l.nodes())
        .iter()
        .filter(|c| c.as_heritage_clause().token == Kind::ImplementsKeyword)
        .flat_map(|c| c.as_heritage_clause().types.get().nodes().iter().copied())
        .collect()
}

/// Whether `a` becomes `b` by inserting, deleting or replacing exactly one character.
fn one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if a.len() >= b.len() {
            i += 1;
        }
        if b.len() >= a.len() {
            j += 1;
        }
    }
    edits + a.len() - i + b.len() - j == 1
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
        if self.reported.insert((path, span.start, code)) {
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

    fn is_own_class(&mut self, t: P<Type>) -> bool {
        self.api(t.symbol()).is_some_and(|name| OWN_CLASSES.contains(&name))
    }

    fn bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        let t = target(t);
        if !t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return Vec::new();
        }
        self.checker.get_base_types_exported(t).to_vec()
    }

    /// The base types of a class or interface, or else of the class or interface that a reference instantiates.
    fn resolved_bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
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

    fn global_type(&mut self, t: P<Type>, at: P<Node>, seen: &mut HashSet<P<Type>>) {
        if !seen.insert(t) {
            return;
        }
        for symbol in [t.alias().and_then(|a| a.symbol()), t.symbol()].into_iter().flatten() {
            for &declaration in symbol.declarations() {
                if !self.is_library_declaration(declaration) && is_global(declaration) {
                    self.report(
                        at,
                        Code::Rule(10),
                        "Type comes from global declarations; import a module type for incremental correctness.",
                    );
                }
            }
        }
        if t.flags().intersects(TypeFlags::Union) {
            for &part in t.types() {
                self.global_type(part, at, seen);
            }
        }
    }

    fn global_references(&mut self, node: Option<P<Node>>, at: P<Node>, seen: &mut HashSet<P<Node>>) {
        let Some(node) = node else { return };
        if !seen.insert(node) {
            return;
        }
        if node.kind() == Kind::TypeReference {
            let name = node.as_type_reference_node().type_name;
            let symbol = self.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.checker, s));
            for &declaration in symbol.map_or(&[][..], |s| s.declarations()) {
                if declaration.kind() == Kind::TypeAliasDeclaration {
                    if is_global(declaration) {
                        self.report(
                            at,
                            Code::Rule(10),
                            "Attribute and dependency aliases must come from module declarations.",
                        );
                    }
                    self.global_references(declaration.type_node(), at, seen);
                }
            }
        }
        node.for_each_child(&mut |child| {
            self.global_references(Some(child), at, seen);
            false
        });
    }

    fn type_id(&mut self, t: P<Type>, at: P<Node>) -> Option<Box<str>> {
        self.global_type(t, at, &mut HashSet::new());
        let symbol = t.symbol()?;
        let declaration = symbol.declarations().iter().copied().find(|&d| is_class_or_interface(d))?;
        let file = tsrs_ast::get_source_file_of_node(declaration)?;
        let id = self.crucible.type_id(file.file_name(), symbol.name());
        if id.is_none() {
            self.report(
                at,
                Code::Setup,
                "Cannot compute a type id. The declaring package needs a name and a mirrored source/declaration layout.",
            );
        }
        id
    }

    fn all_target(&mut self, t: P<Type>) -> Option<P<Type>> {
        if self.api(t.symbol()) != Some("All") || !is_reference(t) {
            return None;
        }
        self.checker.get_type_arguments_exported(t).first().copied()
    }

    fn is_nominal(&mut self, t: P<Type>) -> bool {
        t.symbol().is_some_and(|s| {
            s.declarations().iter().any(|&d| is_class_or_interface(d))
                && self.checker.get_declared_type_of_symbol_exported(s) == target(t)
        })
    }

    fn dependency(&mut self, t: P<Type>, at: P<Node>, all: bool) -> Dependency {
        if !all && let Some(item) = self.all_target(t) {
            return match self.dependency(item, at, true) {
                Dependency::One(id) => Dependency::All(id),
                other => other,
            };
        }
        self.global_type(t, at, &mut HashSet::new());
        let nominal = self.is_nominal(t);
        let instance = match self.checker.resolve_name_exported("Instance", Some(at), SymbolFlags::Type, false) {
            Some(s) => {
                let declared = self.checker.get_declared_type_of_symbol_exported(s);
                self.checker.is_type_assignable_to_exported(t, declared)
            }
            None => false,
        };
        let generic = is_reference(t)
            && !self.checker.get_type_arguments_exported(t).is_empty()
            && (!all || self.generic_target(t, Generic::Target, &mut HashSet::new()));
        let concrete_behaviour = self.kind_of(t) == Some(ClassKind::Behaviour)
            && t.symbol().is_some_and(|s| s.declarations().iter().any(|&d| is_concrete_class(d)));
        if !nominal || instance || t.flags().intersects(TypeFlags::Union) || generic || (!all && concrete_behaviour) {
            let message = if concrete_behaviour {
                "Inject All<X> or call X.get(instance) for a concrete behaviour."
            } else {
                "Constructor parameter must be a plain class, interface or All<T>."
            };
            self.report(at, Code::Rule(1), message);
            return Dependency::Invalid;
        }
        self.type_id(t, at).map_or(Dependency::Invalid, Dependency::One)
    }

    fn attribute_check(&mut self, t: P<Type>, at: P<Node>) -> Check {
        self.global_type(t, at, &mut HashSet::new());
        let flags = t.flags();
        if flags.intersects(TypeFlags::Boolean) {
            return Check::Boolean;
        }
        if flags.intersects(TypeFlags::Union) {
            let parts: Vec<P<Type>> =
                t.types().iter().copied().filter(|p| !p.flags().intersects(TypeFlags::Undefined)).collect();
            let pair = parts.iter().any(|&p| types::literal_bool(p) == Some(true))
                && parts.iter().any(|&p| types::literal_bool(p) == Some(false));
            let mut checks: Vec<Check> = Vec::new();
            for part in parts {
                if !(pair && part.flags().intersects(TypeFlags::BooleanLiteral)) {
                    checks.push(self.attribute_check(part, at));
                }
            }
            if pair {
                checks.push(Check::Boolean);
            }
            return if checks.len() == 1 { checks.remove(0) } else { Check::Union(checks) };
        }
        if flags.intersects(TypeFlags::String) {
            return Check::String;
        }
        if flags.intersects(TypeFlags::Number) {
            return Check::Number;
        }
        if let Some(s) = types::literal_string(t) {
            return Check::Literal(Literal::String(s.into()));
        }
        if let Some(n) = types::literal_number(t) {
            return Check::Literal(Literal::Number(n));
        }
        if let Some(b) = types::literal_bool(t) {
            return Check::Literal(Literal::Boolean(b));
        }
        if let Some(symbol) = t.symbol()
            && let name @ ("Vector3" | "Color3") = symbol.name()
            && self.checker.resolve_name_exported(name, Some(at), SymbolFlags::Type, false) == Some(symbol)
        {
            return if name == "Vector3" { Check::Vector3 } else { Check::Color3 };
        }
        self.report(
            at,
            Code::Rule(7),
            "Attribute fields support string, number, boolean, Vector3, Color3 and literal unions.",
        );
        Check::Invalid
    }

    pub(super) fn is_crucible_call(&mut self, n: P<Node>, m: Macro) -> bool {
        n.kind() == Kind::CallExpression && self.crucible.macro_at(&mut self.checker, n) == Some(m)
    }

    /// The registration of a concrete Crucible class, with the rules its declaration breaks reported.
    fn registration(&mut self, node: P<Node>) -> Option<Class> {
        let instance = self.instance_type(node)?;
        let kind = self.kind_of(instance)?;
        if tsrs_ast::get_combined_modifier_flags(node).intersects(ModifierFlags::Abstract | ModifierFlags::Ambient)
            || node.flags().intersects(NodeFlags::Ambient)
        {
            return None;
        }
        let mut constructor = None;
        let mut current = Some(instance);
        let mut visited = HashSet::new();
        while let Some(c) = current
            && visited.insert(c)
        {
            let declaration =
                c.symbol().and_then(|s| s.declarations().iter().copied().find(|d| d.kind() == Kind::ClassDeclaration));
            constructor = declaration.and_then(|d| d.members().iter().copied().find(|m| m.kind() == Kind::Constructor));
            if constructor.is_some() {
                break;
            }
            current = self.bases(c).first().copied();
        }
        let hidden = constructor.is_some_and(|c| {
            tsrs_ast::get_combined_modifier_flags(c).intersects(ModifierFlags::Private | ModifierFlags::Protected)
        });
        if node.kind() != Kind::ClassDeclaration
            || node.name().is_none()
            || node.parent().is_none_or(|p| p.kind() != Kind::SourceFile)
            || !node.type_parameters().is_empty()
            || hidden
        {
            self.report(
                node,
                Code::Rule(3),
                "Registered classes must be named, non-generic, module-level classes with public constructors.",
            );
            return None;
        }
        let symbol = self.checker.get_symbol_of_declaration_exported(node)?;
        let static_type = self.checker.get_type_of_symbol_exported(symbol);
        let construct =
            self.checker.get_signatures_of_type_exported(static_type, SignatureKind::Construct).first().copied();
        let mut deps = Vec::new();
        for &parameter in construct.map_or(&[][..], |c| c.parameters()) {
            let declaration = parameter.value_declaration().unwrap_or(node);
            self.global_references(declaration.type_node(), declaration, &mut HashSet::new());
            if declaration.kind() == Kind::Parameter {
                let p = declaration.as_parameter_declaration();
                if p.question_token.get().is_some()
                    || p.dot_dot_dot_token().is_some()
                    || declaration.initializer().is_some()
                {
                    self.report(
                        declaration,
                        Code::Rule(1),
                        "Optional, defaulted and rest parameters are not injectable.",
                    );
                }
            }
            let t = self.checker.get_type_of_symbol_exported(parameter);
            deps.push(self.dependency(t, declaration, false));
        }
        let mut provides = Vec::new();
        let mut stack = vec![instance];
        let mut seen = HashSet::new();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) || self.is_own_class(current) {
                continue;
            }
            if current != instance
                && let Some(id) = self.type_id(current, node)
                && !provides.contains(&id)
            {
                provides.push(id);
            }
            let mut next = self.bases(current);
            for &declaration in current.symbol().map_or(&[][..], |s| s.declarations()) {
                for implemented in implemented(declaration) {
                    next.push(self.checker.get_type_at_location(implemented));
                }
            }
            stack.extend(next.into_iter().rev());
        }
        let id = self.type_id(instance, node);
        let mut class = Class { kind, id, deps, provides, tag: None, instance: Vec::new(), attributes: Vec::new() };
        if kind == ClassKind::Behaviour {
            let member = |x: &mut Extractor, name: &str| {
                let property = x.checker.get_property_of_type_exported(instance, name)?;
                Some(x.checker.get_type_of_symbol_exported(property))
            };
            class.tag = member(self, "tag").and_then(types::literal_string).map(Into::into);
            if class.tag.is_none() {
                self.report(node, Code::Rule(2), "A concrete behaviour tag must be exactly one string literal.");
            }
            let mut parts = match member(self, "instance") {
                Some(t) if t.flags().intersects(TypeFlags::Union) => t.types().iter().copied().map(Some).collect(),
                t => vec![t],
            };
            if parts.len() > 1 {
                let made = self.made();
                parts.sort_by_key(|p| p.and_then(|t| made.get(&t.symbol()?)).map_or(usize::MAX, |m| m.turn));
            }
            let allowed = self
                .checker
                .resolve_name_exported("Instances", Some(node), SymbolFlags::Type, false)
                .map(|s| self.checker.get_declared_type_of_symbol_exported(s));
            for part in parts {
                let name = part.and_then(|p| p.symbol()).map(|s| s.name());
                let expected = match (name, allowed) {
                    (Some(name), Some(allowed)) => self.checker.get_property_of_type_exported(allowed, name),
                    _ => None,
                };
                let matches = expected.is_some_and(|e| {
                    self.checker.get_type_of_symbol_exported(e).symbol() == part.and_then(|p| p.symbol())
                });
                if !matches {
                    self.report(node, Code::Rule(8), "Instance type must be an Instances class or union of them.");
                }
                class.instance.push(name.unwrap_or("Instance").into());
            }
            for &property in self.checker.get_properties_of_type_exported(instance) {
                let Some(declaration) = property.value_declaration() else { continue };
                if declaration.kind() != Kind::PropertyDeclaration {
                    continue;
                }
                let Some(initializer) = declaration.initializer() else { continue };
                if !self.is_crucible_call(initializer, Macro::Attribute) {
                    continue;
                }
                let t = self.checker.get_type_of_symbol_exported(property);
                let check = self.attribute_check(t, declaration);
                class.attributes.push(Attribute {
                    field: property.name().into(),
                    check,
                    required: initializer.arguments().is_empty(),
                });
            }
        }
        Some(class)
    }

    /// TypeScript 5 orders a union by when its checker made each member type, and the transformer lists a behaviour's
    /// instance classes in that order. The checker makes a type at the first reference in the transformer's walk of
    /// the program, and makes an interface's bases as it makes the interface. TypeScript 7 orders a union by name.
    fn made(&mut self) -> &HashMap<P<Symbol>, Made> {
        if self.crucible.made.is_none() {
            let mut made = HashMap::new();
            let mut aliases = HashSet::new();
            for &file in self.crucible.program.source_files() {
                if !file.is_declaration_file() && !file.is_js() {
                    self.make_referenced(file.as_node(), &mut made, &mut aliases);
                }
            }
            self.crucible.made = Some(made);
        }
        self.crucible.made.get_or_insert_default()
    }

    fn make_referenced(&mut self, n: P<Node>, made: &mut HashMap<P<Symbol>, Made>, aliases: &mut HashSet<P<Symbol>>) {
        let name = match n.parent() {
            // The transformer's walk makes a class's base at the class, and an interface's bases only when it makes
            // the interface.
            Some(clause) if clause.kind() == Kind::HeritageClause => {
                let class =
                    clause.parent().is_some_and(|c| matches!(c.kind(), Kind::ClassDeclaration | Kind::ClassExpression));
                (class && clause.as_heritage_clause().token == Kind::ExtendsKeyword)
                    .then(|| tsrs_ast::get_heritage_clause_element_name(n))
            }
            _ => (n.kind() == Kind::TypeReference).then(|| n.as_type_reference_node().type_name),
        };
        if let Some(symbol) = name.and_then(|name| self.checker.get_symbol_at_location_exported(name)) {
            let symbol = unalias(&mut self.checker, symbol);
            if symbol.flags().intersects(SymbolFlags::Class | SymbolFlags::Interface) {
                self.make(symbol, made);
            } else if symbol.flags().intersects(SymbolFlags::TypeAlias) && aliases.insert(symbol) {
                for &declaration in symbol.declarations() {
                    if let Some(aliased) = declaration.type_node() {
                        self.make_referenced(aliased, made, aliases);
                    }
                }
            }
        }
        n.for_each_child(&mut |child| {
            self.make_referenced(child, made, aliases);
            false
        });
    }

    /// Makes a class or interface type as TypeScript 5's `getDeclaredTypeOfClassOrInterface` does, and returns whether
    /// the type has a `this` type.
    fn make(&mut self, symbol: P<Symbol>, made: &mut HashMap<P<Symbol>, Made>) -> bool {
        if let Some(m) = made.get(&symbol) {
            return m.this_type;
        }
        made.insert(symbol, Made { turn: made.len(), this_type: false });
        let generic = symbol.declarations().iter().any(|&d| {
            matches!(d.kind(), Kind::ClassDeclaration | Kind::ClassExpression | Kind::InterfaceDeclaration)
                && !d.type_parameters().is_empty()
        });
        let this_type = symbol.flags().intersects(SymbolFlags::Class) || generic || !self.thisless(symbol, made);
        if let Some(m) = made.get_mut(&symbol) {
            m.this_type = this_type;
        }
        this_type
    }

    /// TypeScript 5's `isThislessInterface`, which makes each base interface it reaches.
    fn thisless(&mut self, symbol: P<Symbol>, made: &mut HashMap<P<Symbol>, Made>) -> bool {
        for &declaration in symbol.declarations() {
            if declaration.kind() != Kind::InterfaceDeclaration {
                continue;
            }
            if declaration.flags().intersects(NodeFlags::ContainsThis) {
                return false;
            }
            for &base in tsrs_ast::get_extends_heritage_clause_elements(declaration) {
                let name = tsrs_ast::get_heritage_clause_element_name(base);
                let symbol = self.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.checker, s));
                let Some(base) = symbol.filter(|s| s.flags().intersects(SymbolFlags::Interface)) else { return false };
                if self.make(base, made) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether `t` instantiates a generic parameter that its kind of use forbids.
    fn generic_target(&mut self, t: P<Type>, mode: Generic, seen: &mut HashSet<P<Type>>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().iter().any(|&part| self.generic_target(part, mode, seen));
        }
        if self.is_own_class(t) {
            return false;
        }
        let arguments = if is_reference(t) { self.checker.get_type_arguments_exported(t).to_vec() } else { Vec::new() };
        if !arguments.is_empty() {
            let generic = target(t);
            let tag = self
                .checker
                .get_property_of_type_exported(generic, "tag")
                .map(|p| self.checker.get_type_of_symbol_exported(p));
            let parameters = generic.try_as_interface_type().map_or(&[][..], |i| i.type_parameters());
            let behaviour = self.kind_of(generic) == Some(ClassKind::Behaviour);
            if arguments.iter().enumerate().any(|(i, argument)| {
                !behaviour
                    || parameters.get(i).copied() != tag
                    || (mode != Generic::Provides && argument.flags() != TypeFlags::String)
            }) {
                return true;
            }
        }
        let declarations = t.symbol().map_or(&[][..], |s| s.declarations());
        let base_mode =
            if declarations.iter().any(|d| d.kind() == Kind::ClassDeclaration) { Generic::Provides } else { mode };
        for base in self.resolved_bases(t) {
            if self.generic_target(base, base_mode, seen) {
                return true;
            }
        }
        for &declaration in declarations {
            for implemented in implemented(declaration) {
                let t = self.checker.get_type_at_location(implemented);
                if self.generic_target(t, mode, seen) {
                    return true;
                }
            }
        }
        false
    }

    fn concrete_base(&mut self, t: P<Type>, seen: &mut HashSet<P<Type>>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        if t.flags().intersects(TypeFlags::Intersection) {
            return t.types().iter().any(|&part| self.concrete_base(part, seen));
        }
        if self.kind_of(t).is_some()
            && self.api(t.symbol()).is_none()
            && t.symbol().is_some_and(|s| s.declarations().iter().any(|&d| is_concrete_class(d)))
        {
            return true;
        }
        self.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, seen))
    }

    fn promise(&mut self, t: P<Type>, seen: &mut HashSet<P<Type>>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        t.symbol().is_some_and(|s| s.name() == "Promise")
            || (t.flags().intersects(TypeFlags::UnionOrIntersection)
                && t.types().iter().any(|&part| self.promise(part, seen)))
            || self.resolved_bases(t).into_iter().any(|base| self.promise(base, seen))
    }

    /// Runs Crucible's rules over one file of a program that uses the package and stores the facts its macros and
    /// registered classes lower from.
    pub(super) fn crucible_file(&mut self, file: P<SourceFile>) {
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
            Kind::ClassDeclaration | Kind::ClassExpression => self.crucible_class_rules(n, file),
            Kind::TypeReference => {
                let t = self.checker.get_type_from_type_node_exported(n);
                if let Some(item) = self.all_target(t)
                    && self.generic_target(item, Generic::Target, &mut HashSet::new())
                {
                    self.report(
                        n,
                        Code::Rule(14),
                        "All targets must not instantiate generic parameters except string tags.",
                    );
                }
            }
            _ => {}
        }
        n.for_each_child(&mut |child| {
            self.crucible_walk(child, file, names);
            false
        });
        if matches!(n.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
            && let Some(class) = self.registration(n)
        {
            self.site();
            let span = self.span(n);
            self.table.store(span, Site::Crucible, |f| &mut f.crucible, Box::new(Crucible::Class(class)));
        }
    }

    fn entry_import(&mut self, n: P<Node>, specifier: Option<P<Node>>) {
        if let Some(specifier) = specifier
            && specifier.kind() == Kind::StringLiteral
            && let Some(entry) = specifier.text().strip_prefix("@trioforge/crucible/")
            && !matches!(entry, "server" | "client" | "vide" | "util/random" | "maths/spline")
        {
            self.report(
                n,
                Code::Rule(4),
                "Import only the root, server, client, vide, util/random or maths/spline Crucible entries.",
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
            Some(Macro::Inject) => self.inject_rules(n),
            Some(Macro::Attribute) => self.attribute_rules(n),
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

    fn crucible_class_rules(&mut self, n: P<Node>, file: P<SourceFile>) {
        let Some(t) = self.instance_type(n) else { return };
        if self.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, &mut HashSet::new())) {
            self.report(n, Code::Rule(13), "Extend an abstract Crucible base, not a concrete registered class.");
        }
        if self.kind_of(t).is_none() {
            return;
        }
        let abstract_ = tsrs_ast::get_combined_modifier_flags(n).intersects(ModifierFlags::Abstract);
        if !abstract_ && self.generic_target(t, Generic::Provides, &mut HashSet::new()) {
            self.report(n, Code::Rule(14), "Provided types must not instantiate non-tag generic parameters.");
        }
        for implemented in implemented(n) {
            let target = self.checker.get_type_at_location(implemented);
            if self.generic_target(target, Generic::Provides, &mut HashSet::new()) {
                self.report(n, Code::Rule(14), "Provided interfaces must not instantiate generic parameters.");
            }
        }
        let class_file = tsrs_ast::get_source_file_of_node(n).map(|f| PathBuf::from(f.file_name())).unwrap_or_default();
        let side = self.crucible.side(Path::new(file.file_name()), &class_file);
        self.side_members(t, n, side, &mut HashSet::new());
        for &member in n.members() {
            if member.kind() != Kind::MethodDeclaration {
                continue;
            }
            let Some(name) = member.name().and_then(|nm| self.checker.get_symbol_at_location_exported(nm)) else {
                continue;
            };
            let name = name.name();
            let hook = self.crucible.hooks.contains(name);
            let resembles = HOOK_LOOKALIKES.contains(&name)
                || self.crucible.hooks.iter().any(|h| h.to_lowercase() == name.to_lowercase() || one_edit(name, h));
            if !hook && resembles {
                self.report(
                    member,
                    Code::Rule(9),
                    "Method resembles a lifecycle hook. Use its exact Crucible declaration name.",
                );
            }
            if hook {
                let asynchronous = tsrs_ast::get_combined_modifier_flags(member).intersects(ModifierFlags::Async);
                let returns_promise = !asynchronous && {
                    let signature = self.checker.get_signature_from_declaration_exported(member);
                    let returned = self.checker.get_return_type_of_signature_exported(signature);
                    self.promise(returned, &mut HashSet::new())
                };
                if asynchronous || returns_promise {
                    self.report(
                        member,
                        Code::Rule(12),
                        "Crucible hooks must be synchronous and must not return a Promise.",
                    );
                }
            }
        }
    }

    fn side_members(&mut self, current: P<Type>, class: P<Node>, side: Option<Side>, seen: &mut HashSet<P<Type>>) {
        if !seen.insert(current) || self.api(current.symbol()).is_some() {
            return;
        }
        for &declaration in current.symbol().map_or(&[][..], |s| s.declarations()) {
            for &member in declaration.members() {
                let Some(name) = member.name().and_then(|nm| self.checker.get_symbol_at_location_exported(nm)) else {
                    continue;
                };
                let at = if declaration == class { member } else { class };
                if side != Some(Side::Client) && matches!(name.name(), "preRender" | "renderPriority") {
                    self.report(at, Code::Rule(11), "preRender and renderPriority require a client class.");
                }
                if side != Some(Side::Server) && name.name() == "onShutdown" {
                    self.report(at, Code::Rule(16), "onShutdown requires a server class.");
                }
            }
        }
        for base in self.resolved_bases(current) {
            self.side_members(base, class, side, seen);
        }
    }

    fn inject_rules(&mut self, n: P<Node>) {
        let t = n.type_arguments().first().map(|&own| self.checker.get_type_from_type_node_exported(own));
        let target = t.map(|t| self.all_target(t).unwrap_or(t));
        let mut inside = false;
        let mut parent = n.parent();
        while let Some(p) = parent {
            if matches!(p.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
                && !tsrs_ast::get_combined_modifier_flags(p).intersects(ModifierFlags::Abstract)
                && let Some(instance) = self.instance_type(p)
                && self.kind_of(instance).is_some()
            {
                inside = true;
            }
            parent = p.parent();
        }
        if inside || !target.is_some_and(|t| self.is_nominal(t)) {
            self.report(
                n,
                Code::Rule(5),
                "inject requires a class, interface or All target outside registered classes.",
            );
        }
        if let Some(target) = target
            && self.generic_target(target, Generic::Target, &mut HashSet::new())
        {
            self.report(
                n,
                Code::Rule(14),
                "inject targets must not instantiate generic parameters except string tags.",
            );
        }
    }

    pub(super) fn crucible_inject(&mut self, call: P<Node>) -> Crucible {
        let target = call.type_arguments().first().and_then(|&argument| {
            let t = self.checker.get_type_from_type_node_exported(argument);
            match self.all_target(t) {
                Some(item) => self.type_id(item, call).map(Dependency::All),
                None => self.type_id(t, call).map(Dependency::One),
            }
        });
        Crucible::Inject(target.unwrap_or(Dependency::Invalid))
    }

    fn attribute_rules(&mut self, n: P<Node>) {
        let field = n.parent().filter(|p| {
            p.kind() == Kind::PropertyDeclaration
                && p.initializer() == Some(n)
                && !tsrs_ast::get_combined_modifier_flags(*p).intersects(ModifierFlags::Static)
        });
        let behaviour = field
            .and_then(|f| f.parent())
            .and_then(|class| self.instance_type(class))
            .is_some_and(|t| self.kind_of(t) == Some(ClassKind::Behaviour));
        let Some(field) = field.filter(|_| behaviour) else {
            self.report(n, Code::Rule(15), "attribute() must initialize a behaviour instance field.");
            return;
        };
        let mut seen = HashSet::new();
        self.global_references(field.type_node(), field, &mut seen);
        for &argument in n.type_arguments() {
            self.global_references(Some(argument), field, &mut seen);
        }
        let t = self.checker.get_type_at_location(field);
        self.attribute_check(t, field);
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
