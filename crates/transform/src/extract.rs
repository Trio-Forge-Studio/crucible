use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crucible_compiler::ext::{Code, ExtractCx, SiteId};
use crucible_compiler::packages::entry_types;
use crucible_compiler::tsrs::ast::{self, Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use crucible_compiler::tsrs::checker::{Checker, LiteralValue, ObjectFlags, SignatureKind, Type, TypeFlags};
use crucible_compiler::tsrs::core::P;
use crucible_compiler::{Game, RbxPath, normalize};

use crate::{Attribute, Check, Class, ClassKind, Dependency, Fact, Literal};

const PACKAGE: &str = "@trioforge/crucible";
const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];
/// `@trioforge/crucible/emit` exports a runtime `attribute` that is not the macro.
const MACRO_ENTRIES: [(&str, &[Macro]); 3] =
    [(".", &[Macro::Attribute, Macro::Inject]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];
/// Lifecycle names from other frameworks that a Crucible class may mean as a hook.
const HOOK_LOOKALIKES: [&str; 5] = ["onInit", "onStart", "onTick", "onPhysics", "onRender"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Macro {
    Attribute,
    Inject,
    Boot,
}

impl Macro {
    fn name(self) -> &'static str {
        match self {
            Macro::Attribute => "attribute",
            Macro::Inject => "inject",
            Macro::Boot => "boot",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Server,
    Client,
    Shared,
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
#[derive(Clone, Copy, Debug)]
struct Made {
    turn: usize,
    this_type: bool,
}

/// What the transform reads from `@trioforge/crucible` once per program.
#[derive(Debug)]
pub(crate) struct Framework {
    exports: HashMap<P<Symbol>, &'static str>,
    macros: HashMap<P<Symbol>, Macro>,
    /// The protected instance methods Crucible's own classes declare, which are its lifecycle hooks.
    hooks: HashSet<&'static str>,
    kinds: HashMap<P<Type>, Option<ClassKind>>,
    /// When TypeScript 5 makes each class and interface type, computed when a union of instance classes needs it.
    made: Option<HashMap<P<Symbol>, Made>>,
    /// The declarations of `@trioforge/crucible/emit`.
    pub emit: Option<PathBuf>,
}

impl Framework {
    pub fn new(cx: &mut ExtractCx<'_>) -> Framework {
        let mut framework = Framework {
            exports: HashMap::new(),
            macros: HashMap::new(),
            hooks: HashSet::new(),
            kinds: HashMap::new(),
            made: None,
            emit: None,
        };
        let mut modules = Vec::new();
        for &file in cx.program().source_files() {
            let path = PathBuf::from(file.file_name());
            let Some(owner) = cx.packages().package_of(&path) else { continue };
            if owner.name() != Some(PACKAGE) {
                continue;
            }
            if framework.emit.is_none() {
                framework.emit = entry_types(&owner, "./emit");
            }
            let Some(module) = file.as_node().symbol() else { continue };
            let c = cx.checker();
            let exports = c.get_exports_of_module_exported(module);
            for &symbol in &exports {
                let target = unalias(c, symbol);
                framework.exports.entry(target).or_insert(symbol.name());
            }
            modules.push((path, exports));
        }
        for (path, exports) in modules {
            let Some((owner, module)) = cx.packages().module_path(&path) else { continue };
            for (entry, macros) in MACRO_ENTRIES {
                if cx.packages().entry_module(&owner, entry).as_deref() != Some(module.as_str()) {
                    continue;
                }
                for &symbol in &exports {
                    let target = unalias(cx.checker(), symbol);
                    if let Some(&m) = macros.iter().find(|m| m.name() == symbol.name()) {
                        framework.macros.insert(target, m);
                    }
                }
            }
        }
        for (&symbol, name) in &framework.exports {
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
                        framework.hooks.insert(name.text());
                    }
                }
            }
        }
        framework
    }

    fn name(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.exports.get(&unalias(c, symbol?)).copied()
    }

    fn macro_of(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<Macro> {
        self.macros.get(&unalias(c, symbol?)).copied()
    }
}

/// Runs the transform over one file: records the macro calls everywhere, and the rules and registrations in a
/// program that uses the package.
pub(crate) fn file(
    framework: &mut Framework,
    facts: &mut HashMap<SiteId, Fact>,
    cx: &mut ExtractCx<'_>,
    file: P<SourceFile>,
) {
    if framework.exports.is_empty() {
        return;
    }
    let path = Path::new(file.file_name());
    let own = cx.packages().package_of(path).is_some_and(|p| p.name() == Some(PACKAGE));
    let rules = !own && !cx.packages().invalid_metadata();
    let mut x = X { fw: framework, facts, cx, rules };
    x.walk(file.as_node(), file);
}

struct X<'s, 'c, 'x> {
    fw: &'s mut Framework,
    facts: &'s mut HashMap<SiteId, Fact>,
    cx: &'c mut ExtractCx<'x>,
    /// Whether the file is one the rules and registrations apply to.
    rules: bool,
}

fn unalias(c: &mut Checker, s: P<Symbol>) -> P<Symbol> {
    if s.flags().intersects(SymbolFlags::Alias) { c.get_aliased_symbol(s) } else { s }
}

fn target(t: P<Type>) -> P<Type> {
    if is_reference(t) { t.target().unwrap_or(t) } else { t }
}

fn is_reference(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Object) && t.object_flags().intersects(ObjectFlags::Reference)
}

fn is_class_or_interface(d: P<Node>) -> bool {
    matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration)
}

fn is_concrete_class(d: P<Node>) -> bool {
    d.kind() == Kind::ClassDeclaration && !ast::get_combined_modifier_flags(d).intersects(ModifierFlags::Abstract)
}

/// A global script, or a `declare global` block in a module.
fn is_global(declaration: P<Node>) -> bool {
    let Some(file) = ast::get_source_file_of_node(declaration) else { return false };
    if !ast::is_external_module(file) {
        return true;
    }
    let mut parent = declaration.parent();
    while let Some(p) = parent {
        if p.kind() == Kind::ModuleDeclaration && ast::is_global_scope_augmentation(p) {
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

/// The folder of a path as Node's `path.dirname` gives it, with `.` for a bare name.
fn dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(i) => trimmed[..i].trim_end_matches('/'),
        None if path.starts_with('/') => "/",
        None => ".",
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

fn literal_number(t: P<Type>) -> Option<f64> {
    match t.flags().intersects(TypeFlags::NumberLiteral).then(|| t.as_literal_type().value()) {
        Some(Some(LiteralValue::Number(n))) => Some(n.0),
        _ => None,
    }
}

fn literal_string(t: P<Type>) -> Option<&'static str> {
    match t.flags().intersects(TypeFlags::StringLiteral).then(|| t.as_literal_type().value()) {
        Some(Some(LiteralValue::String(s))) => Some(s),
        _ => None,
    }
}

fn literal_bool(t: P<Type>) -> Option<bool> {
    match t.flags().intersects(TypeFlags::BooleanLiteral).then(|| t.as_literal_type().value()) {
        Some(Some(LiteralValue::Boolean(b))) => Some(b),
        _ => None,
    }
}

impl X<'_, '_, '_> {
    fn walk(&mut self, n: P<Node>, file: P<SourceFile>) {
        match n.kind() {
            Kind::CallExpression => self.call(n),
            Kind::ClassDeclaration | Kind::ClassExpression if self.rules => self.class_rules(n, file),
            Kind::TypeReference if self.rules => {
                let t = self.cx.checker().get_type_from_type_node_exported(n);
                if let Some(item) = self.all_target(t)
                    && self.generic_target(item, Generic::Target, &mut HashSet::new())
                {
                    self.cx.report(
                        n,
                        Code::Rule(14),
                        "All targets must not instantiate generic parameters except string tags.",
                    );
                }
            }
            _ => {}
        }
        n.for_each_child(&mut |child| {
            self.walk(child, file);
            false
        });
        if self.rules
            && matches!(n.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
            && let Some(class) = self.registration(n)
        {
            self.cx.count_site();
            let site = self.cx.site(n);
            self.facts.insert(site, Fact::Class(class));
        }
    }

    /// The macro a call expands, which is none while package metadata cannot be read, because then the transform does
    /// not run.
    fn macro_at(&mut self, call: P<Node>) -> Option<Macro> {
        if self.fw.macros.is_empty() || self.cx.packages().invalid_metadata() {
            return None;
        }
        let c = self.cx.checker();
        let symbol = c.get_symbol_at_location_exported(call.as_call_expression().expression);
        if let Some(m) = self.fw.macro_of(c, symbol) {
            return Some(m);
        }
        let name = c.get_resolved_signature_exported(call).declaration()?.name()?;
        let symbol = c.get_symbol_at_location_exported(name);
        self.fw.macro_of(c, symbol)
    }

    fn call(&mut self, n: P<Node>) {
        let Some(m) = self.macro_at(n) else { return };
        let site = self.cx.site(n);
        let fact = match m {
            Macro::Inject => {
                if self.rules {
                    self.inject_rules(n);
                }
                self.cx.count_site();
                self.inject(n)
            }
            Macro::Attribute => {
                if self.rules {
                    self.attribute_rules(n);
                }
                Fact::Attribute
            }
            Macro::Boot => Fact::Boot(if self.rules { self.boot(n) } else { Vec::new() }),
        };
        self.facts.insert(site, fact);
    }

    fn api(&mut self, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.fw.name(self.cx.checker(), symbol)
    }

    fn is_own_class(&mut self, t: P<Type>) -> bool {
        self.api(t.symbol()).is_some_and(|name| OWN_CLASSES.contains(&name))
    }

    fn bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        let t = target(t);
        if !t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return Vec::new();
        }
        self.cx.checker().get_base_types_exported(t).to_vec()
    }

    /// The base types of a class or interface, or else of the class or interface that a reference instantiates.
    fn resolved_bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        if t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            self.cx.checker().get_base_types_exported(t).to_vec()
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

    fn kind_of(&mut self, t: P<Type>) -> Option<ClassKind> {
        if let Some(&kind) = self.fw.kinds.get(&t) {
            return kind;
        }
        let kind = self.class_kind(t, &mut HashSet::new());
        self.fw.kinds.insert(t, kind);
        kind
    }

    fn instance_type(&mut self, class: P<Node>) -> Option<P<Type>> {
        let c = self.cx.checker();
        let symbol = c.get_symbol_of_declaration_exported(class)?;
        Some(c.get_declared_type_of_symbol_exported(symbol))
    }

    /// Declarations of the language and the Roblox API, which every program shares.
    fn is_library_declaration(&mut self, declaration: P<Node>) -> bool {
        let Some(file) = ast::get_source_file_of_node(declaration) else { return false };
        let name = file.file_name().replace('\\', "/");
        if name.contains("/node_modules/@rbxts/") || self.cx.program().is_source_file_default_library(file.path()) {
            return true;
        }
        self.cx.packages().package_of(Path::new(&name)).is_some_and(|p| {
            p.name().is_some_and(|n| n.starts_with("@rbxts/") || n == "@trioforge/compiler-types")
        })
    }

    /// The nearest folder named for a side in the path from the package of `source` to `file`'s folder.
    fn side(&mut self, source: &Path, file: &Path) -> Option<Side> {
        let root = self.cx.packages().package_of(source).map(|p| p.dir.clone()).unwrap_or_default();
        let folder = file.parent()?;
        let relative = folder.strip_prefix(&root).unwrap_or(folder);
        relative.components().rev().find_map(|c| match c.as_os_str().to_str()? {
            "server" => Some(Side::Server),
            "client" => Some(Side::Client),
            "shared" => Some(Side::Shared),
            _ => None,
        })
    }

    fn global_type(&mut self, t: P<Type>, at: P<Node>, seen: &mut HashSet<P<Type>>) {
        if !seen.insert(t) {
            return;
        }
        for symbol in [t.alias().and_then(|a| a.symbol()), t.symbol()].into_iter().flatten() {
            for &declaration in symbol.declarations() {
                if !self.is_library_declaration(declaration) && is_global(declaration) {
                    self.cx.report(
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
            let c = self.cx.checker();
            let symbol = c.get_symbol_at_location_exported(name).map(|s| unalias(c, s));
            for &declaration in symbol.map_or(&[][..], |s| s.declarations()) {
                if declaration.kind() == Kind::TypeAliasDeclaration {
                    if is_global(declaration) {
                        self.cx.report(
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
        let file = ast::get_source_file_of_node(declaration)?;
        let id = self.cx.packages().module_path(Path::new(file.file_name())).and_then(|(owner, module)| {
            let package = owner.name()?;
            Some(format!("{package}/{module}:{}", symbol.name()).into())
        });
        if id.is_none() {
            self.cx.report(
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
        self.cx.checker().get_type_arguments_exported(t).first().copied()
    }

    fn is_nominal(&mut self, t: P<Type>) -> bool {
        t.symbol().is_some_and(|s| {
            s.declarations().iter().any(|&d| is_class_or_interface(d))
                && self.cx.checker().get_declared_type_of_symbol_exported(s) == target(t)
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
        let c = self.cx.checker();
        let instance = match c.resolve_name_exported("Instance", Some(at), SymbolFlags::Type, false) {
            Some(s) => {
                let declared = c.get_declared_type_of_symbol_exported(s);
                c.is_type_assignable_to_exported(t, declared)
            }
            None => false,
        };
        let generic = is_reference(t)
            && !self.cx.checker().get_type_arguments_exported(t).is_empty()
            && (!all || self.generic_target(t, Generic::Target, &mut HashSet::new()));
        let concrete_behaviour = self.kind_of(t) == Some(ClassKind::Behaviour)
            && t.symbol().is_some_and(|s| s.declarations().iter().any(|&d| is_concrete_class(d)));
        if !nominal || instance || t.flags().intersects(TypeFlags::Union) || generic || (!all && concrete_behaviour) {
            let message = if concrete_behaviour {
                "Inject All<X> or call X.get(instance) for a concrete behaviour."
            } else {
                "Constructor parameter must be a plain class, interface or All<T>."
            };
            self.cx.report(at, Code::Rule(1), message);
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
            let pair = parts.iter().any(|&p| literal_bool(p) == Some(true))
                && parts.iter().any(|&p| literal_bool(p) == Some(false));
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
        if let Some(s) = literal_string(t) {
            return Check::Literal(Literal::String(s.into()));
        }
        if let Some(n) = literal_number(t) {
            return Check::Literal(Literal::Number(n));
        }
        if let Some(b) = literal_bool(t) {
            return Check::Literal(Literal::Boolean(b));
        }
        if let Some(symbol) = t.symbol()
            && let name @ ("Vector3" | "Color3") = symbol.name()
            && self.cx.checker().resolve_name_exported(name, Some(at), SymbolFlags::Type, false) == Some(symbol)
        {
            return if name == "Vector3" { Check::Vector3 } else { Check::Color3 };
        }
        self.cx.report(
            at,
            Code::Rule(7),
            "Attribute fields support string, number, boolean, Vector3, Color3 and literal unions.",
        );
        Check::Invalid
    }

    fn is_macro_call(&mut self, n: P<Node>, m: Macro) -> bool {
        n.kind() == Kind::CallExpression && self.macro_at(n) == Some(m)
    }

    /// The registration of a concrete Crucible class, with the rules its declaration breaks reported.
    fn registration(&mut self, node: P<Node>) -> Option<Class> {
        let instance = self.instance_type(node)?;
        let kind = self.kind_of(instance)?;
        if ast::get_combined_modifier_flags(node).intersects(ModifierFlags::Abstract | ModifierFlags::Ambient)
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
            ast::get_combined_modifier_flags(c).intersects(ModifierFlags::Private | ModifierFlags::Protected)
        });
        if node.kind() != Kind::ClassDeclaration
            || node.name().is_none()
            || node.parent().is_none_or(|p| p.kind() != Kind::SourceFile)
            || !node.type_parameters().is_empty()
            || hidden
        {
            self.cx.report(
                node,
                Code::Rule(3),
                "Registered classes must be named, non-generic, module-level classes with public constructors.",
            );
            return None;
        }
        let c = self.cx.checker();
        let symbol = c.get_symbol_of_declaration_exported(node)?;
        let static_type = c.get_type_of_symbol_exported(symbol);
        let construct = c.get_signatures_of_type_exported(static_type, SignatureKind::Construct).first().copied();
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
                    self.cx.report(
                        declaration,
                        Code::Rule(1),
                        "Optional, defaulted and rest parameters are not injectable.",
                    );
                }
            }
            let t = self.cx.checker().get_type_of_symbol_exported(parameter);
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
                    next.push(self.cx.checker().get_type_at_location(implemented));
                }
            }
            stack.extend(next.into_iter().rev());
        }
        let id = self.type_id(instance, node);
        let mut class = Class { kind, id, deps, provides, tag: None, instance: Vec::new(), attributes: Vec::new() };
        if kind == ClassKind::Behaviour {
            let member = |x: &mut Self, name: &str| {
                let c = x.cx.checker();
                let property = c.get_property_of_type_exported(instance, name)?;
                Some(c.get_type_of_symbol_exported(property))
            };
            class.tag = member(self, "tag").and_then(literal_string).map(Into::into);
            if class.tag.is_none() {
                self.cx.report(node, Code::Rule(2), "A concrete behaviour tag must be exactly one string literal.");
            }
            let mut parts = match member(self, "instance") {
                Some(t) if t.flags().intersects(TypeFlags::Union) => t.types().iter().copied().map(Some).collect(),
                t => vec![t],
            };
            if parts.len() > 1 {
                let made = self.made();
                parts.sort_by_key(|p| p.and_then(|t| made.get(&t.symbol()?)).map_or(usize::MAX, |m| m.turn));
            }
            let c = self.cx.checker();
            let allowed = c
                .resolve_name_exported("Instances", Some(node), SymbolFlags::Type, false)
                .map(|s| c.get_declared_type_of_symbol_exported(s));
            for part in parts {
                let name = part.and_then(|p| p.symbol()).map(|s| s.name());
                let c = self.cx.checker();
                let expected = match (name, allowed) {
                    (Some(name), Some(allowed)) => c.get_property_of_type_exported(allowed, name),
                    _ => None,
                };
                let matches =
                    expected.is_some_and(|e| c.get_type_of_symbol_exported(e).symbol() == part.and_then(|p| p.symbol()));
                if !matches {
                    self.cx.report(node, Code::Rule(8), "Instance type must be an Instances class or union of them.");
                }
                class.instance.push(name.unwrap_or("Instance").into());
            }
            let properties = self.cx.checker().get_properties_of_type_exported(instance).to_vec();
            for property in properties {
                let Some(declaration) = property.value_declaration() else { continue };
                if declaration.kind() != Kind::PropertyDeclaration {
                    continue;
                }
                let Some(initializer) = declaration.initializer() else { continue };
                if !self.is_macro_call(initializer, Macro::Attribute) {
                    continue;
                }
                let t = self.cx.checker().get_type_of_symbol_exported(property);
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
        if self.fw.made.is_none() {
            let mut made = HashMap::new();
            let mut aliases = HashSet::new();
            for &file in self.cx.program().source_files() {
                if !file.is_declaration_file() && !file.is_js() {
                    self.make_referenced(file.as_node(), &mut made, &mut aliases);
                }
            }
            self.fw.made = Some(made);
        }
        self.fw.made.get_or_insert_default()
    }

    fn make_referenced(&mut self, n: P<Node>, made: &mut HashMap<P<Symbol>, Made>, aliases: &mut HashSet<P<Symbol>>) {
        let name = match n.parent() {
            // The transformer's walk makes a class's base at the class, and an interface's bases only when it makes
            // the interface.
            Some(clause) if clause.kind() == Kind::HeritageClause => {
                let class =
                    clause.parent().is_some_and(|c| matches!(c.kind(), Kind::ClassDeclaration | Kind::ClassExpression));
                (class && clause.as_heritage_clause().token == Kind::ExtendsKeyword)
                    .then(|| ast::get_heritage_clause_element_name(n))
            }
            _ => (n.kind() == Kind::TypeReference).then(|| n.as_type_reference_node().type_name),
        };
        let c = self.cx.checker();
        if let Some(symbol) = name.and_then(|name| c.get_symbol_at_location_exported(name)) {
            let symbol = unalias(c, symbol);
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
            for &base in ast::get_extends_heritage_clause_elements(declaration) {
                let name = ast::get_heritage_clause_element_name(base);
                let c = self.cx.checker();
                let symbol = c.get_symbol_at_location_exported(name).map(|s| unalias(c, s));
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
        let arguments =
            if is_reference(t) { self.cx.checker().get_type_arguments_exported(t).to_vec() } else { Vec::new() };
        if !arguments.is_empty() {
            let generic = target(t);
            let c = self.cx.checker();
            let tag = c.get_property_of_type_exported(generic, "tag").map(|p| c.get_type_of_symbol_exported(p));
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
                let t = self.cx.checker().get_type_at_location(implemented);
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

    fn class_rules(&mut self, n: P<Node>, file: P<SourceFile>) {
        let Some(t) = self.instance_type(n) else { return };
        if self.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, &mut HashSet::new())) {
            self.cx.report(n, Code::Rule(13), "Extend an abstract Crucible base, not a concrete registered class.");
        }
        if self.kind_of(t).is_none() {
            return;
        }
        let abstract_ = ast::get_combined_modifier_flags(n).intersects(ModifierFlags::Abstract);
        if !abstract_ && self.generic_target(t, Generic::Provides, &mut HashSet::new()) {
            self.cx.report(n, Code::Rule(14), "Provided types must not instantiate non-tag generic parameters.");
        }
        for implemented in implemented(n) {
            let target = self.cx.checker().get_type_at_location(implemented);
            if self.generic_target(target, Generic::Provides, &mut HashSet::new()) {
                self.cx.report(n, Code::Rule(14), "Provided interfaces must not instantiate generic parameters.");
            }
        }
        let class_file = ast::get_source_file_of_node(n).map(|f| PathBuf::from(f.file_name())).unwrap_or_default();
        let side = self.side(Path::new(file.file_name()), &class_file);
        self.side_members(t, n, side, &mut HashSet::new());
        for &member in n.members() {
            if member.kind() != Kind::MethodDeclaration {
                continue;
            }
            let Some(name) = member.name().and_then(|nm| self.cx.checker().get_symbol_at_location_exported(nm)) else {
                continue;
            };
            let name = name.name();
            let hook = self.fw.hooks.contains(name);
            let resembles = HOOK_LOOKALIKES.contains(&name)
                || self.fw.hooks.iter().any(|h| h.to_lowercase() == name.to_lowercase() || one_edit(name, h));
            if !hook && resembles {
                self.cx.report(
                    member,
                    Code::Rule(9),
                    "Method resembles a lifecycle hook. Use its exact Crucible declaration name.",
                );
            }
            if hook {
                let asynchronous = ast::get_combined_modifier_flags(member).intersects(ModifierFlags::Async);
                let returns_promise = !asynchronous && {
                    let c = self.cx.checker();
                    let signature = c.get_signature_from_declaration_exported(member);
                    let returned = c.get_return_type_of_signature_exported(signature);
                    self.promise(returned, &mut HashSet::new())
                };
                if asynchronous || returns_promise {
                    self.cx.report(
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
                let Some(name) = member.name().and_then(|nm| self.cx.checker().get_symbol_at_location_exported(nm))
                else {
                    continue;
                };
                let at = if declaration == class { member } else { class };
                if side != Some(Side::Client) && matches!(name.name(), "preRender" | "renderPriority") {
                    self.cx.report(at, Code::Rule(11), "preRender and renderPriority require a client class.");
                }
                if side != Some(Side::Server) && name.name() == "onShutdown" {
                    self.cx.report(at, Code::Rule(16), "onShutdown requires a server class.");
                }
            }
        }
        for base in self.resolved_bases(current) {
            self.side_members(base, class, side, seen);
        }
    }

    fn inject_rules(&mut self, n: P<Node>) {
        let t = n.type_arguments().first().map(|&own| self.cx.checker().get_type_from_type_node_exported(own));
        let target = t.map(|t| self.all_target(t).unwrap_or(t));
        let mut inside = false;
        let mut parent = n.parent();
        while let Some(p) = parent {
            if matches!(p.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
                && !ast::get_combined_modifier_flags(p).intersects(ModifierFlags::Abstract)
                && let Some(instance) = self.instance_type(p)
                && self.kind_of(instance).is_some()
            {
                inside = true;
            }
            parent = p.parent();
        }
        if inside || !target.is_some_and(|t| self.is_nominal(t)) {
            self.cx.report(
                n,
                Code::Rule(5),
                "inject requires a class, interface or All target outside registered classes.",
            );
        }
        if let Some(target) = target
            && self.generic_target(target, Generic::Target, &mut HashSet::new())
        {
            self.cx.report(
                n,
                Code::Rule(14),
                "inject targets must not instantiate generic parameters except string tags.",
            );
        }
    }

    fn inject(&mut self, call: P<Node>) -> Fact {
        let target = call.type_arguments().first().and_then(|&argument| {
            let t = self.cx.checker().get_type_from_type_node_exported(argument);
            match self.all_target(t) {
                Some(item) => self.type_id(item, call).map(Dependency::All),
                None => self.type_id(t, call).map(Dependency::One),
            }
        });
        Fact::Inject(target.unwrap_or(Dependency::Invalid))
    }

    fn attribute_rules(&mut self, n: P<Node>) {
        let field = n.parent().filter(|p| {
            p.kind() == Kind::PropertyDeclaration
                && p.initializer() == Some(n)
                && !ast::get_combined_modifier_flags(*p).intersects(ModifierFlags::Static)
        });
        let behaviour = field
            .and_then(|f| f.parent())
            .and_then(|class| self.instance_type(class))
            .is_some_and(|t| self.kind_of(t) == Some(ClassKind::Behaviour));
        let Some(field) = field.filter(|_| behaviour) else {
            self.cx.report(n, Code::Rule(15), "attribute() must initialize a behaviour instance field.");
            return;
        };
        let mut seen = HashSet::new();
        self.global_references(field.type_node(), field, &mut seen);
        for &argument in n.type_arguments() {
            self.global_references(Some(argument), field, &mut seen);
        }
        let t = self.cx.checker().get_type_at_location(field);
        self.attribute_check(t, field);
    }

    /// Each load glob's span and instance path pattern.
    fn boot(&mut self, call: P<Node>) -> Vec<(crucible_compiler::Span, Box<str>)> {
        let mut loads = Vec::new();
        let options = call.arguments().first().map(|&a| skip_wrappers(a));
        let Some(options) = options.filter(|o| o.kind() == Kind::ObjectLiteralExpression) else {
            self.cx.report(call, Code::Rule(6), "boot requires literal load globs.");
            return loads;
        };
        let properties = options.properties();
        let load = properties.iter().rposition(|&p| property_name(p) == Some("load"));
        if load.is_none() {
            self.cx.report(options, Code::Rule(6), "boot requires a literal load property.");
        }
        for (i, &property) in properties.iter().enumerate() {
            let computed = property.name().is_some_and(|n| n.kind() == Kind::ComputedPropertyName);
            if property.kind() == Kind::SpreadAssignment || computed {
                if load.is_none_or(|load| i > load) {
                    self.cx.report(
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
                self.cx.report(property, Code::Rule(6), "load must be an array of string literals.");
                continue;
            };
            for &entry in entries.elements() {
                if entry.kind() != Kind::StringLiteral {
                    self.cx.report(entry, Code::Rule(6), "load entry must be a string literal.");
                    continue;
                }
                self.cx.count_site();
                match self.load_glob(entry.text()) {
                    Ok(glob) => loads.push((self.cx.span(entry), glob.into())),
                    Err(message) => self.cx.report(entry, Code::Rule(6), message),
                }
            }
        }
        loads
    }

    /// A load glob, relative to the package root, as the instance path pattern of the modules it matches.
    fn load_glob(&self, glob: &str) -> Result<String, &'static str> {
        let Some(game) = self.cx.game() else {
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
    fn loads_module(&self, game: &Game, segments: &[&str]) -> bool {
        self.cx.program().source_files().iter().any(|&file| {
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
    fn folder_by_files(&self, game: &Game, folder: &Path) -> Result<Option<RbxPath>, ()> {
        let folder = normalize(folder);
        let mut found: Vec<RbxPath> = Vec::new();
        for &file in self.cx.program().source_files() {
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
