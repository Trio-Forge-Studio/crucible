//! Registration and dependency injection: the rules a Crucible class and an `inject` or `attribute` call must follow, and
//! the facts `defineClass`, `resolve`, `resolveAll` and `attribute` lower from.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crucible_compiler::extension::{
    ExtractCx, File, Kind, ModifierFlags, Node, NodeFlags, ObjectFlags, SignatureKind, Symbol, SymbolFlags, Type,
    TypeFlags,
};

use crate::api::{Api, Macro, Made, OWN_CLASSES, is_declaration_macro};
use crate::declarations::Side;
use crate::facts::{Attribute, Check, Class, ClassKind, Dependency, Fact, Literal};
use crate::{Code, State};

/// Lifecycle names from other frameworks that a Crucible class may mean as a hook.
const HOOK_LOOKALIKES: [&str; 5] = ["onInit", "onStart", "onTick", "onPhysics", "onRender"];

/// Which generic instantiations a provided type or an `All` or `inject` target may use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generic {
    /// A type a class provides: only a behaviour's tag parameter may be instantiated.
    Provides,
    /// A type code resolves: only a behaviour's tag parameter, and only with `string`.
    Target,
}

pub(crate) struct Walk<'w, 'x> {
    pub(crate) cx: &'w mut ExtractCx<'x>,
    pub(crate) api: &'w mut Api,
    pub(crate) state: &'w mut State,
}

fn target(t: Type) -> Type {
    if is_reference(t) { t.target().unwrap_or(t) } else { t }
}

pub(crate) fn is_reference(t: Type) -> bool {
    t.flags().intersects(TypeFlags::Object) && t.object_flags().intersects(ObjectFlags::Reference)
}

fn is_class_or_interface(d: Node) -> bool {
    matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration)
}

fn is_concrete_class(d: Node) -> bool {
    d.kind() == Kind::ClassDeclaration && !d.combined_modifier_flags().intersects(ModifierFlags::Abstract)
}

/// A global script, or a `declare global` block in a module.
pub(crate) fn is_global(declaration: Node) -> bool {
    if !declaration.file().is_module() {
        return true;
    }
    let mut parent = declaration.parent();
    while let Some(p) = parent {
        if p.kind() == Kind::ModuleDeclaration && p.is_global_scope_augmentation() {
            return true;
        }
        parent = p.parent();
    }
    false
}

/// Whether a call calls `n`, through any parentheses.
fn is_callee(mut n: Node) -> bool {
    while let Some(parent) = n.parent().filter(|p| p.kind() == Kind::ParenthesizedExpression) {
        n = parent;
    }
    n.parent().is_some_and(|p| p.kind() == Kind::CallExpression && p.expression() == n)
}

/// The local names that a file's imports bind.
fn import_names(statement: Node) -> Vec<Node> {
    match statement.kind() {
        Kind::ImportEqualsDeclaration => statement.name().into_iter().collect(),
        Kind::ImportDeclaration => {
            let Some(clause) = statement.import_clause() else { return Vec::new() };
            let mut names: Vec<Node> = clause.name().into_iter().collect();
            if let Some(named) = clause.named_bindings().filter(|n| n.kind() == Kind::NamedImports) {
                names.extend(named.elements().iter().filter_map(|e| e.name()));
            }
            names
        }
        _ => Vec::new(),
    }
}

fn implemented(declaration: Node) -> Vec<Node> {
    declaration
        .heritage_clauses()
        .iter()
        .filter(|c| c.token() == Kind::ImplementsKeyword)
        .flat_map(|c| c.types().iter().collect::<Vec<_>>())
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

impl Walk<'_, '_> {
    pub(crate) fn file(&mut self, file: File) {
        let names = self.macro_names(file);
        self.walk(file.root(), file, &names);
    }

    /// The names that can refer to a network or data macro in `file`: the macros' own, and the ones its imports give.
    fn macro_names(&mut self, file: File) -> HashSet<&'static str> {
        let mut names: HashSet<&'static str> = Macro::declarations().map(Macro::name).collect();
        for statement in file.root().statements().iter() {
            for name in import_names(statement) {
                let symbol = self.cx.symbol_at(name);
                if self.api(symbol).is_some_and(is_declaration_macro) {
                    names.insert(name.text());
                }
            }
        }
        names
    }

    /// CRU023: a network or data macro anywhere but a call's callee, an import or an export.
    fn macro_value(&mut self, at: Node, symbol: Option<Symbol>) {
        if let Some(name) = self.api(symbol).filter(|name| is_declaration_macro(name)) {
            let message = format!("{name} is a declaration macro and cannot be used as a value.");
            self.report(at, Code::Rule(23), &message);
        }
    }

    pub(crate) fn report(&mut self, at: Node, code: Code, message: &str) {
        let (number, message) = code.render(message);
        self.cx.report(at, number, &message);
    }

    pub(crate) fn store(&mut self, at: Node, fact: Fact) {
        #[cfg(feature = "test-support")]
        self.state.recorded.push((at.file().path(), at.span(), fact.clone()));
        let site = self.cx.site(at);
        self.state.facts.insert(site, fact);
    }

    pub(crate) fn api(&mut self, symbol: Option<Symbol>) -> Option<&'static str> {
        self.api.name(self.cx, symbol)
    }

    pub(crate) fn macro_at(&mut self, call: Node) -> Option<Macro> {
        self.api.macro_at(self.cx, call)
    }

    fn walk(&mut self, n: Node, file: File, names: &HashSet<&'static str>) {
        match n.kind() {
            Kind::CallExpression => self.call(n, file),
            Kind::ImportDeclaration | Kind::ExportDeclaration => self.entry_import(n, n.module_specifier()),
            Kind::ImportType => {
                let argument = n.import_type_argument();
                let literal = (argument.kind() == Kind::LiteralType).then(|| argument.literal());
                self.entry_import(n, literal);
            }
            Kind::ExternalModuleReference => self.entry_import(n, Some(n.expression())),
            Kind::StringLiteral => self.side_entry_specifier(n, file),
            Kind::BinaryExpression => self.process_receipt(n),
            Kind::ModuleDeclaration => self.augmentation(n),
            Kind::Identifier => {
                if names.contains(n.text()) {
                    let parent = n.parent();
                    let specifier =
                        parent.is_some_and(|p| matches!(p.kind(), Kind::ImportSpecifier | Kind::ExportSpecifier));
                    let access = parent.filter(|p| p.kind() == Kind::PropertyAccessExpression && p.name() == Some(n));
                    if !specifier && !is_callee(access.unwrap_or(n)) {
                        let symbol = self.cx.symbol_at(n);
                        self.macro_value(n, symbol);
                    }
                }
                self.side_entry_reference(n, file);
            }
            Kind::ElementAccessExpression if !is_callee(n) => {
                let t = self.cx.type_at(n);
                self.macro_value(n, t.symbol());
            }
            Kind::ClassDeclaration | Kind::ClassExpression => self.class_rules(n),
            Kind::TypeReference => {
                let t = self.cx.type_from_type_node(n);
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
        n.for_each_child(&mut |child| self.walk(child, file, names));
        if matches!(n.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
            && let Some(class) = self.registration(n)
        {
            self.store(n, Fact::Class(class));
        }
    }

    fn call(&mut self, n: Node, file: File) {
        if n.expression().kind() == Kind::ImportKeyword {
            self.entry_import(n, n.arguments().get(0));
            return;
        }
        match self.macro_at(n) {
            Some(Macro::Inject) => {
                self.inject_rules(n);
                let target = self.inject_target(n);
                self.store(n, Fact::Inject(target));
            }
            Some(Macro::Attribute) => {
                self.attribute_rules(n);
                self.store(n, Fact::Attribute);
            }
            Some(Macro::Boot) => self.boot(n),
            Some(Macro::Receive) => self.receive(n, file),
            Some(Macro::Declaration(kind)) => self.declaration(n, kind, file),
            None => {}
        }
        self.profile_store_call(n, file);
    }

    fn is_own_class(&mut self, t: Type) -> bool {
        self.api(t.symbol()).is_some_and(|name| OWN_CLASSES.contains(&name))
    }

    fn bases(&mut self, t: Type) -> Vec<Type> {
        let t = target(t);
        if !t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return Vec::new();
        }
        self.cx.base_types(t)
    }

    /// The base types of a class or interface, or else of the class or interface that a reference instantiates.
    fn resolved_bases(&mut self, t: Type) -> Vec<Type> {
        if t.object_flags().intersects(ObjectFlags::ClassOrInterface) { self.cx.base_types(t) } else { self.bases(t) }
    }

    fn class_kind(&mut self, t: Type, seen: &mut HashSet<Type>) -> Option<ClassKind> {
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

    pub(crate) fn kind_of(&mut self, t: Type) -> Option<ClassKind> {
        if let Some(&kind) = self.api.kinds.get(&t) {
            return kind;
        }
        let kind = self.class_kind(t, &mut HashSet::new());
        self.api.kinds.insert(t, kind);
        kind
    }

    pub(crate) fn instance_type(&mut self, class: Node) -> Option<Type> {
        let symbol = self.cx.symbol_of_declaration(class)?;
        Some(self.cx.declared_type_of_symbol(symbol))
    }

    /// Declarations of the language and the Roblox API, which every program shares.
    pub(crate) fn is_library_declaration(&mut self, declaration: Node) -> bool {
        let file = declaration.file();
        let name = file.path();
        if name.contains("/node_modules/@rbxts/") || self.cx.is_default_library(file) {
            return true;
        }
        self.cx
            .package_name(Path::new(&name))
            .is_some_and(|n| n.starts_with("@rbxts/") || n == "@trioforge/compiler-types")
    }

    fn global_type(&mut self, t: Type, at: Node, seen: &mut HashSet<Type>) {
        if !seen.insert(t) {
            return;
        }
        for symbol in [t.alias_symbol(), t.symbol()].into_iter().flatten() {
            for declaration in symbol.declarations() {
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
            for part in t.types() {
                self.global_type(part, at, seen);
            }
        }
    }

    fn global_references(&mut self, node: Option<Node>, at: Node, seen: &mut HashSet<Node>) {
        let Some(node) = node else { return };
        if !seen.insert(node) {
            return;
        }
        if node.kind() == Kind::TypeReference {
            let symbol = self.cx.symbol_at(node.type_name()).map(|s| self.cx.aliased(s));
            for declaration in symbol.map_or_else(Vec::new, |s| s.declarations()) {
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
        node.for_each_child(&mut |child| self.global_references(Some(child), at, seen));
    }

    fn type_id(&mut self, t: Type, at: Node) -> Option<Box<str>> {
        self.global_type(t, at, &mut HashSet::new());
        let symbol = t.symbol()?;
        let declaration = symbol.declarations().into_iter().find(|&d| is_class_or_interface(d))?;
        let id = self.cx.module_path(Path::new(&declaration.file().path())).map(|(package, module)| {
            let name = symbol.name();
            format!("{package}/{module}:{name}").into()
        });
        if id.is_none() {
            self.report(
                at,
                Code::Setup,
                "Cannot compute a type id. The declaring package needs a name and a mirrored source/declaration layout.",
            );
        }
        id
    }

    fn all_target(&mut self, t: Type) -> Option<Type> {
        if self.api(t.symbol()) != Some("All") || !is_reference(t) {
            return None;
        }
        self.cx.type_arguments(t).first().copied()
    }

    fn is_nominal(&mut self, t: Type) -> bool {
        t.symbol().is_some_and(|s| {
            s.declarations().into_iter().any(is_class_or_interface) && self.cx.declared_type_of_symbol(s) == target(t)
        })
    }

    fn dependency(&mut self, t: Type, at: Node, all: bool) -> Dependency {
        if !all && let Some(item) = self.all_target(t) {
            return match self.dependency(item, at, true) {
                Dependency::One(id) => Dependency::All(id),
                other => other,
            };
        }
        self.global_type(t, at, &mut HashSet::new());
        let nominal = self.is_nominal(t);
        let instance = match self.cx.resolve_name("Instance", Some(at), SymbolFlags::Type) {
            Some(s) => {
                let declared = self.cx.declared_type_of_symbol(s);
                self.cx.is_type_assignable_to(t, declared)
            }
            None => false,
        };
        let generic = is_reference(t)
            && !self.cx.type_arguments(t).is_empty()
            && (!all || self.generic_target(t, Generic::Target, &mut HashSet::new()));
        let concrete_behaviour = self.kind_of(t) == Some(ClassKind::Behaviour)
            && t.symbol().is_some_and(|s| s.declarations().into_iter().any(is_concrete_class));
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

    fn attribute_check(&mut self, t: Type, at: Node) -> Check {
        self.global_type(t, at, &mut HashSet::new());
        let flags = t.flags();
        if flags.intersects(TypeFlags::Boolean) {
            return Check::Boolean;
        }
        if flags.intersects(TypeFlags::Union) {
            let parts: Vec<Type> =
                t.types().into_iter().filter(|p| !p.flags().intersects(TypeFlags::Undefined)).collect();
            let pair = parts.iter().any(|p| p.literal_bool() == Some(true))
                && parts.iter().any(|p| p.literal_bool() == Some(false));
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
        if let Some(s) = t.literal_string() {
            return Check::Literal(Literal::String(s.into()));
        }
        if let Some(n) = t.literal_number() {
            return Check::Literal(Literal::Number(n));
        }
        if let Some(b) = t.literal_bool() {
            return Check::Literal(Literal::Boolean(b));
        }
        if let Some(symbol) = t.symbol()
            && let name @ ("Vector3" | "Color3") = symbol.name()
            && self.cx.resolve_name(name, Some(at), SymbolFlags::Type) == Some(symbol)
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

    pub(crate) fn is_macro_call(&mut self, n: Node, m: Macro) -> bool {
        n.kind() == Kind::CallExpression && self.macro_at(n) == Some(m)
    }

    /// The registration of a concrete Crucible class, with the rules its declaration breaks reported.
    fn registration(&mut self, node: Node) -> Option<Class> {
        let instance = self.instance_type(node)?;
        let kind = self.kind_of(instance)?;
        if node.combined_modifier_flags().intersects(ModifierFlags::Abstract | ModifierFlags::Ambient)
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
                c.symbol().and_then(|s| s.declarations().into_iter().find(|d| d.kind() == Kind::ClassDeclaration));
            constructor = declaration.and_then(|d| d.members().iter().find(|m| m.kind() == Kind::Constructor));
            if constructor.is_some() {
                break;
            }
            current = self.bases(c).first().copied();
        }
        let hidden = constructor
            .is_some_and(|c| c.combined_modifier_flags().intersects(ModifierFlags::Private | ModifierFlags::Protected));
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
        let symbol = self.cx.symbol_of_declaration(node)?;
        let static_type = self.cx.type_of_symbol(symbol);
        let construct = self.cx.signatures_of_type(static_type, SignatureKind::Construct).first().copied();
        let mut deps = Vec::new();
        for parameter in construct.map_or_else(Vec::new, |c| c.parameters()) {
            let declaration = parameter.value_declaration().unwrap_or(node);
            self.global_references(declaration.type_node(), declaration, &mut HashSet::new());
            if declaration.kind() == Kind::Parameter
                && (declaration.has_question_token() || declaration.is_rest() || declaration.initializer().is_some())
            {
                self.report(declaration, Code::Rule(1), "Optional, defaulted and rest parameters are not injectable.");
            }
            let t = self.cx.type_of_symbol(parameter);
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
            for declaration in current.symbol().map_or_else(Vec::new, |s| s.declarations()) {
                for implemented in implemented(declaration) {
                    next.push(self.cx.type_at(implemented));
                }
            }
            stack.extend(next.into_iter().rev());
        }
        let id = self.type_id(instance, node);
        let mut class = Class { kind, id, deps, provides, tag: None, instance: Vec::new(), attributes: Vec::new() };
        if kind == ClassKind::Behaviour {
            let member = |w: &mut Self, name: &str| {
                let property = w.cx.property_of_type(instance, name)?;
                Some(w.cx.type_of_symbol(property))
            };
            class.tag = member(self, "tag").and_then(|t| t.literal_string()).map(Into::into);
            if class.tag.is_none() {
                self.report(node, Code::Rule(2), "A concrete behaviour tag must be exactly one string literal.");
            }
            let mut parts = match member(self, "instance") {
                Some(t) if t.flags().intersects(TypeFlags::Union) => t.types().into_iter().map(Some).collect(),
                t => vec![t],
            };
            if parts.len() > 1
                && let Some(made) = self.made()
            {
                parts.sort_by_key(|p| p.and_then(|t| made.get(&t.symbol()?)).map_or(usize::MAX, |m| m.turn));
            }
            let allowed = self
                .cx
                .resolve_name("Instances", Some(node), SymbolFlags::Type)
                .map(|s| self.cx.declared_type_of_symbol(s));
            for part in parts {
                let name = part.and_then(|p| p.symbol()).map(|s| s.name());
                let expected = match (name, allowed) {
                    (Some(name), Some(allowed)) => self.cx.property_of_type(allowed, name),
                    _ => None,
                };
                let matches =
                    expected.is_some_and(|e| self.cx.type_of_symbol(e).symbol() == part.and_then(|p| p.symbol()));
                if !matches {
                    self.report(node, Code::Rule(8), "Instance type must be an Instances class or union of them.");
                }
                class.instance.push(name.unwrap_or("Instance").into());
            }
            for property in self.cx.properties_of_type(instance) {
                let Some(declaration) = property.value_declaration() else { continue };
                if declaration.kind() != Kind::PropertyDeclaration {
                    continue;
                }
                let Some(initializer) = declaration.initializer() else { continue };
                if !self.is_macro_call(initializer, Macro::Attribute) {
                    continue;
                }
                let t = self.cx.type_of_symbol(property);
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
    fn made(&mut self) -> Option<&HashMap<Symbol, Made>> {
        if self.api.made.is_none() {
            let mut made = HashMap::new();
            let mut aliases = HashSet::new();
            for file in self.cx.files() {
                if !file.is_declaration() && !file.is_js() {
                    self.make_referenced(file.root(), &mut made, &mut aliases);
                }
            }
            self.api.made = Some(made);
        }
        self.api.made.as_ref()
    }

    fn make_referenced(&mut self, n: Node, made: &mut HashMap<Symbol, Made>, aliases: &mut HashSet<Symbol>) {
        let name = match n.parent() {
            // The transformer's walk makes a class's base at the class, and an interface's bases only when it makes
            // the interface.
            Some(clause) if clause.kind() == Kind::HeritageClause => {
                let class =
                    clause.parent().is_some_and(|c| matches!(c.kind(), Kind::ClassDeclaration | Kind::ClassExpression));
                (class && clause.token() == Kind::ExtendsKeyword).then(|| n.heritage_element_name())
            }
            _ => (n.kind() == Kind::TypeReference).then(|| n.type_name()),
        };
        if let Some(symbol) = name.and_then(|name| self.cx.symbol_at(name)) {
            let symbol = self.cx.aliased(symbol);
            if symbol.flags().intersects(SymbolFlags::Class | SymbolFlags::Interface) {
                self.make(symbol, made);
            } else if symbol.flags().intersects(SymbolFlags::TypeAlias) && aliases.insert(symbol) {
                for declaration in symbol.declarations() {
                    if let Some(aliased) = declaration.type_node() {
                        self.make_referenced(aliased, made, aliases);
                    }
                }
            }
        }
        n.for_each_child(&mut |child| self.make_referenced(child, made, aliases));
    }

    /// Makes a class or interface type as TypeScript 5's `getDeclaredTypeOfClassOrInterface` does, and returns whether
    /// the type has a `this` type.
    fn make(&mut self, symbol: Symbol, made: &mut HashMap<Symbol, Made>) -> bool {
        if let Some(m) = made.get(&symbol) {
            return m.this_type;
        }
        made.insert(symbol, Made { turn: made.len(), this_type: false });
        let generic = symbol.declarations().into_iter().any(|d| {
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
    fn thisless(&mut self, symbol: Symbol, made: &mut HashMap<Symbol, Made>) -> bool {
        for declaration in symbol.declarations() {
            if declaration.kind() != Kind::InterfaceDeclaration {
                continue;
            }
            if declaration.flags().intersects(NodeFlags::ContainsThis) {
                return false;
            }
            for base in declaration.extends_elements().iter() {
                let name = base.heritage_element_name();
                let symbol = self.cx.symbol_at(name).map(|s| self.cx.aliased(s));
                let Some(base) = symbol.filter(|s| s.flags().intersects(SymbolFlags::Interface)) else { return false };
                if self.make(base, made) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether `t` instantiates a generic parameter that its kind of use forbids.
    fn generic_target(&mut self, t: Type, mode: Generic, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().into_iter().any(|part| self.generic_target(part, mode, seen));
        }
        if self.is_own_class(t) {
            return false;
        }
        let arguments = if is_reference(t) { self.cx.type_arguments(t) } else { Vec::new() };
        if !arguments.is_empty() {
            let generic = target(t);
            let tag = self.cx.property_of_type(generic, "tag").map(|p| self.cx.type_of_symbol(p));
            let parameters = generic.type_parameters();
            let behaviour = self.kind_of(generic) == Some(ClassKind::Behaviour);
            if arguments.iter().enumerate().any(|(i, argument)| {
                !behaviour
                    || parameters.get(i).copied() != tag
                    || (mode != Generic::Provides && argument.flags() != TypeFlags::String)
            }) {
                return true;
            }
        }
        let declarations = t.symbol().map_or_else(Vec::new, |s| s.declarations());
        let base_mode =
            if declarations.iter().any(|d| d.kind() == Kind::ClassDeclaration) { Generic::Provides } else { mode };
        for base in self.resolved_bases(t) {
            if self.generic_target(base, base_mode, seen) {
                return true;
            }
        }
        for declaration in declarations {
            for implemented in implemented(declaration) {
                let t = self.cx.type_at(implemented);
                if self.generic_target(t, mode, seen) {
                    return true;
                }
            }
        }
        false
    }

    fn concrete_base(&mut self, t: Type, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        if t.flags().intersects(TypeFlags::Intersection) {
            return t.types().into_iter().any(|part| self.concrete_base(part, seen));
        }
        if self.kind_of(t).is_some()
            && self.api(t.symbol()).is_none()
            && t.symbol().is_some_and(|s| s.declarations().into_iter().any(is_concrete_class))
        {
            return true;
        }
        self.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, seen))
    }

    fn promise(&mut self, t: Type, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        t.symbol().is_some_and(|s| s.name() == "Promise")
            || (t.flags().intersects(TypeFlags::UnionOrIntersection)
                && t.types().into_iter().any(|part| self.promise(part, seen)))
            || self.resolved_bases(t).into_iter().any(|base| self.promise(base, seen))
    }

    /// The nearest folder named for a side in the path from the package of `source` to `file`'s folder.
    pub(crate) fn side(&mut self, source: &Path, file: &Path) -> Option<Side> {
        let root = self.cx.package_dir(source).unwrap_or_default();
        let folder = file.parent()?;
        let relative = folder.strip_prefix(&root).unwrap_or(folder);
        relative.components().rev().find_map(|c| match c.as_os_str().to_str()? {
            "server" => Some(Side::Server),
            "client" => Some(Side::Client),
            "shared" => Some(Side::Shared),
            _ => None,
        })
    }

    fn class_rules(&mut self, n: Node) {
        let Some(t) = self.instance_type(n) else { return };
        if self.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, &mut HashSet::new())) {
            self.report(n, Code::Rule(13), "Extend an abstract Crucible base, not a concrete registered class.");
        }
        if self.kind_of(t).is_none() {
            return;
        }
        let abstract_ = n.combined_modifier_flags().intersects(ModifierFlags::Abstract);
        if !abstract_ && self.generic_target(t, Generic::Provides, &mut HashSet::new()) {
            self.report(n, Code::Rule(14), "Provided types must not instantiate non-tag generic parameters.");
        }
        for implemented in implemented(n) {
            let target = self.cx.type_at(implemented);
            if self.generic_target(target, Generic::Provides, &mut HashSet::new()) {
                self.report(n, Code::Rule(14), "Provided interfaces must not instantiate generic parameters.");
            }
        }
        let class_file = PathBuf::from(n.file().path());
        let side = self.side(&class_file, &class_file);
        self.side_members(t, n, side, &mut HashSet::new());
        let hooks = self.api.hooks.clone();
        for member in n.members().iter() {
            if member.kind() != Kind::MethodDeclaration {
                continue;
            }
            let Some(name) = member.name().and_then(|nm| self.cx.symbol_at(nm)) else { continue };
            let name = name.name();
            let hook = hooks.contains(name);
            let resembles = HOOK_LOOKALIKES.contains(&name)
                || hooks.iter().any(|h| h.to_lowercase() == name.to_lowercase() || one_edit(name, h));
            if !hook && resembles {
                self.report(
                    member,
                    Code::Rule(9),
                    "Method resembles a lifecycle hook. Use its exact Crucible declaration name.",
                );
            }
            if hook {
                let asynchronous = member.combined_modifier_flags().intersects(ModifierFlags::Async);
                let returns_promise = !asynchronous && {
                    let signature = self.cx.signature_from_declaration(member);
                    let returned = self.cx.return_type(signature);
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

    fn side_members(&mut self, current: Type, class: Node, side: Option<Side>, seen: &mut HashSet<Type>) {
        if !seen.insert(current) || self.api(current.symbol()).is_some() {
            return;
        }
        for declaration in current.symbol().map_or_else(Vec::new, |s| s.declarations()) {
            for member in declaration.members().iter() {
                let Some(name) = member.name().and_then(|nm| self.cx.symbol_at(nm)) else { continue };
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

    fn inject_rules(&mut self, n: Node) {
        let t = n.type_arguments().get(0).map(|own| self.cx.type_from_type_node(own));
        let target = t.map(|t| self.all_target(t).unwrap_or(t));
        let mut inside = false;
        let mut parent = n.parent();
        while let Some(p) = parent {
            if matches!(p.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
                && !p.combined_modifier_flags().intersects(ModifierFlags::Abstract)
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

    fn inject_target(&mut self, call: Node) -> Dependency {
        let target = call.type_arguments().get(0).and_then(|argument| {
            let t = self.cx.type_from_type_node(argument);
            match self.all_target(t) {
                Some(item) => self.type_id(item, call).map(Dependency::All),
                None => self.type_id(t, call).map(Dependency::One),
            }
        });
        target.unwrap_or(Dependency::Invalid)
    }

    fn attribute_rules(&mut self, n: Node) {
        let field = n.parent().filter(|p| {
            p.kind() == Kind::PropertyDeclaration
                && p.initializer() == Some(n)
                && !p.combined_modifier_flags().intersects(ModifierFlags::Static)
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
        for argument in n.type_arguments().iter() {
            self.global_references(Some(argument), field, &mut seen);
        }
        let t = self.cx.type_at(field);
        self.attribute_check(t, field);
    }
}
