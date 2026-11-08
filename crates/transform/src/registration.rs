//! Class registration and dependency injection: the registration data of each concrete Crucible class, the
//! `inject` and `attribute` macros, and the rules they enforce.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crucible_compiler::extension::{
    Code, ExtractCx, LiteralValue, Modifiers, Node, Symbol, SymbolFlags, SyntaxKind as K, Type, TypeFlags,
};

use crate::api::OWN_CLASSES;
use crate::{Crucible, Macro};

/// Lifecycle names from other frameworks that a Crucible class may mean as a hook.
const HOOK_LOOKALIKES: [&str; 5] = ["onInit", "onStart", "onTick", "onPhysics", "onRender"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassKind {
    Subsystem,
    Behaviour,
}

/// The registration data of a concrete Crucible class. A type id is `<package>/<module path>:<name>`, and `None` where
/// the transform reports that it cannot compute one.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub kind: ClassKind,
    pub id: Option<Box<str>>,
    /// The constructor's parameters in order.
    pub deps: Vec<Dependency>,
    /// Every base class and implemented interface below Crucible's own classes.
    pub provides: Vec<Box<str>>,
    /// The behaviour's tag literal.
    pub tag: Option<Box<str>>,
    /// The behaviour's `Instances` class names.
    pub instance: Vec<Box<str>>,
    pub attributes: Vec<Attribute>,
}

/// A constructor parameter or `inject` target, by type id.
#[derive(Clone, Debug, PartialEq)]
pub enum Dependency {
    One(Box<str>),
    /// `All<T>`.
    All(Box<str>),
    /// A target CRU001 or CRU005 rejects, or one without a type id. The pass reports it and the compile fails.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub field: Box<str>,
    pub check: Check,
    /// False when `attribute()` has a default value.
    pub required: bool,
}

/// A runtime type check for an attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Boolean,
    String,
    Number,
    Vector3,
    Color3,
    Literal(Literal),
    Union(Vec<Check>),
    /// A type CRU007 rejects. The pass reports it and the compile fails.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Boolean(bool),
    Number(f64),
    String(Box<str>),
}

/// Which generic instantiations a provided type or an `All` or `inject` target may use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generic {
    /// A type a class provides: only a behaviour's tag parameter may be instantiated.
    Provides,
    /// A type code resolves: only a behaviour's tag parameter, and only with `string`.
    Target,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Server,
    Client,
    Shared,
}

/// When TypeScript 5's checker made a class or interface type, and whether the type has a `this` type.
#[derive(Clone, Copy)]
struct Made {
    turn: usize,
    this_type: bool,
}

#[derive(Default)]
pub(crate) struct Cache {
    kinds: HashMap<Type, Option<ClassKind>>,
    /// When TypeScript 5 makes each class and interface type, computed when a union of instance classes needs it.
    made: Option<HashMap<Symbol, Made>>,
}

fn literal_string(t: Type) -> Option<&'static str> {
    match t.literal() {
        Some(LiteralValue::String(s)) => Some(s),
        _ => None,
    }
}

fn literal_number(t: Type) -> Option<f64> {
    match t.literal() {
        Some(LiteralValue::Number(n)) => Some(n),
        _ => None,
    }
}

fn literal_bool(t: Type) -> Option<bool> {
    match t.literal() {
        Some(LiteralValue::Boolean(b)) => Some(b),
        _ => None,
    }
}

fn is_class_or_interface(d: Node) -> bool {
    matches!(d.kind(), K::ClassDeclaration | K::InterfaceDeclaration)
}

fn is_concrete_class(d: Node) -> bool {
    d.kind() == K::ClassDeclaration && !d.modifiers().intersects(Modifiers::ABSTRACT)
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

/// The nearest folder named for a side in the path from the package of `source` to `file`'s folder.
fn side(cx: &mut ExtractCx<'_>, source: &str, file: &Path) -> Option<Side> {
    let root = cx.package_dir(source).unwrap_or_default();
    let folder = file.parent()?;
    let relative = folder.strip_prefix(&root).unwrap_or(folder);
    relative.components().rev().find_map(|c| match c.as_os_str().to_str()? {
        "server" => Some(Side::Server),
        "client" => Some(Side::Client),
        "shared" => Some(Side::Shared),
        _ => None,
    })
}

impl Crucible {
    fn api_name(&self, cx: &mut ExtractCx<'_>, symbol: Option<Symbol>) -> Option<&'static str> {
        self.api.name(cx, symbol)
    }

    fn is_own_class(&self, cx: &mut ExtractCx<'_>, t: Type) -> bool {
        self.api_name(cx, t.symbol()).is_some_and(|name| OWN_CLASSES.contains(&name))
    }

    fn bases(cx: &mut ExtractCx<'_>, t: Type) -> Vec<Type> {
        let t = t.target();
        if !t.is_class_or_interface() {
            return Vec::new();
        }
        cx.base_types(t)
    }

    /// The base types of a class or interface, or else of the class or interface that a reference instantiates.
    fn resolved_bases(cx: &mut ExtractCx<'_>, t: Type) -> Vec<Type> {
        if t.is_class_or_interface() { cx.base_types(t) } else { Self::bases(cx, t) }
    }

    fn class_kind(&self, cx: &mut ExtractCx<'_>, t: Type, seen: &mut HashSet<Type>) -> Option<ClassKind> {
        if !seen.insert(t) {
            return None;
        }
        match self.api_name(cx, t.symbol()) {
            Some("Subsystem") => return Some(ClassKind::Subsystem),
            Some("CrucibleBehaviour") => return Some(ClassKind::Behaviour),
            _ => {}
        }
        for base in Self::bases(cx, t) {
            if let Some(kind) = self.class_kind(cx, base, seen) {
                return Some(kind);
            }
        }
        None
    }

    pub(crate) fn kind_of(&mut self, cx: &mut ExtractCx<'_>, t: Type) -> Option<ClassKind> {
        if let Some(&kind) = self.registration.kinds.get(&t) {
            return kind;
        }
        let kind = self.class_kind(cx, t, &mut HashSet::new());
        self.registration.kinds.insert(t, kind);
        kind
    }

    fn instance_type(cx: &mut ExtractCx<'_>, class: Node) -> Option<Type> {
        let symbol = cx.symbol_of_declaration(class)?;
        Some(cx.declared_type(symbol))
    }

    fn global_type(&self, cx: &mut ExtractCx<'_>, t: Type, at: Node, seen: &mut HashSet<Type>) {
        if !seen.insert(t) {
            return;
        }
        for symbol in [t.alias_symbol(), t.symbol()].into_iter().flatten() {
            for declaration in symbol.declarations() {
                if !cx.is_library_declaration(declaration) && declaration.is_global() {
                    cx.report(
                        at,
                        Code::Rule(10),
                        "Type comes from global declarations; import a module type for incremental correctness.",
                    );
                }
            }
        }
        if t.flags().intersects(TypeFlags::UNION) {
            for part in t.types() {
                self.global_type(cx, part, at, seen);
            }
        }
    }

    fn global_references(cx: &mut ExtractCx<'_>, node: Option<Node>, at: Node, seen: &mut HashSet<Node>) {
        let Some(node) = node else { return };
        if !seen.insert(node) {
            return;
        }
        if node.kind() == K::TypeReference
            && let Some(name) = node.type_name()
        {
            let symbol = cx.symbol_at(name).map(|s| cx.unalias(s));
            for declaration in symbol.map_or_else(Vec::new, |s| s.declarations()) {
                if declaration.kind() == K::TypeAliasDeclaration {
                    if declaration.is_global() {
                        cx.report(
                            at,
                            Code::Rule(10),
                            "Attribute and dependency aliases must come from module declarations.",
                        );
                    }
                    Self::global_references(cx, declaration.type_node(), at, seen);
                }
            }
        }
        let mut children = Vec::new();
        node.for_each_child(&mut |child| children.push(child));
        for child in children {
            Self::global_references(cx, Some(child), at, seen);
        }
    }

    fn type_id(&self, cx: &mut ExtractCx<'_>, t: Type, at: Node) -> Option<Box<str>> {
        self.global_type(cx, t, at, &mut HashSet::new());
        let symbol = t.symbol()?;
        let declaration = symbol.declarations().into_iter().find(|&d| is_class_or_interface(d))?;
        let file = declaration.source_file()?;
        let id = cx.module_id(&file.path()).map(|module| format!("{module}:{}", symbol.name()).into());
        if id.is_none() {
            cx.report(
                at,
                Code::Setup,
                "Cannot compute a type id. The declaring package needs a name and a mirrored source/declaration layout.",
            );
        }
        id
    }

    fn all_target(&self, cx: &mut ExtractCx<'_>, t: Type) -> Option<Type> {
        if self.api_name(cx, t.symbol()) != Some("All") || !t.is_reference() {
            return None;
        }
        cx.type_arguments(t).first().copied()
    }

    fn is_nominal(cx: &mut ExtractCx<'_>, t: Type) -> bool {
        t.symbol().is_some_and(|s| {
            s.declarations().into_iter().any(is_class_or_interface) && cx.declared_type(s) == t.target()
        })
    }

    fn dependency(&mut self, cx: &mut ExtractCx<'_>, t: Type, at: Node, all: bool) -> Dependency {
        if !all && let Some(item) = self.all_target(cx, t) {
            return match self.dependency(cx, item, at, true) {
                Dependency::One(id) => Dependency::All(id),
                other => other,
            };
        }
        self.global_type(cx, t, at, &mut HashSet::new());
        let nominal = Self::is_nominal(cx, t);
        let instance = match cx.resolve_type_name("Instance", at) {
            Some(s) => {
                let declared = cx.declared_type(s);
                cx.is_assignable(t, declared)
            }
            None => false,
        };
        let generic = t.is_reference()
            && !cx.type_arguments(t).is_empty()
            && (!all || self.generic_target(cx, t, Generic::Target, &mut HashSet::new()));
        let concrete_behaviour = self.kind_of(cx, t) == Some(ClassKind::Behaviour)
            && t.symbol().is_some_and(|s| s.declarations().into_iter().any(is_concrete_class));
        if !nominal || instance || t.flags().intersects(TypeFlags::UNION) || generic || (!all && concrete_behaviour) {
            let message = if concrete_behaviour {
                "Inject All<X> or call X.get(instance) for a concrete behaviour."
            } else {
                "Constructor parameter must be a plain class, interface or All<T>."
            };
            cx.report(at, Code::Rule(1), message);
            return Dependency::Invalid;
        }
        self.type_id(cx, t, at).map_or(Dependency::Invalid, Dependency::One)
    }

    fn attribute_check(&self, cx: &mut ExtractCx<'_>, t: Type, at: Node) -> Check {
        self.global_type(cx, t, at, &mut HashSet::new());
        let flags = t.flags();
        if flags.intersects(TypeFlags::BOOLEAN) {
            return Check::Boolean;
        }
        if flags.intersects(TypeFlags::UNION) {
            let parts: Vec<Type> =
                t.types().into_iter().filter(|p| !p.flags().intersects(TypeFlags::UNDEFINED)).collect();
            let pair = parts.iter().any(|&p| literal_bool(p) == Some(true))
                && parts.iter().any(|&p| literal_bool(p) == Some(false));
            let mut checks: Vec<Check> = Vec::new();
            for part in parts {
                if !(pair && part.flags().intersects(TypeFlags::BOOLEAN_LITERAL)) {
                    checks.push(self.attribute_check(cx, part, at));
                }
            }
            if pair {
                checks.push(Check::Boolean);
            }
            return if checks.len() == 1 { checks.remove(0) } else { Check::Union(checks) };
        }
        if flags.intersects(TypeFlags::STRING) {
            return Check::String;
        }
        if flags.intersects(TypeFlags::NUMBER) {
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
            && cx.resolve_type_name(name, at) == Some(symbol)
        {
            return if name == "Vector3" { Check::Vector3 } else { Check::Color3 };
        }
        cx.report(
            at,
            Code::Rule(7),
            "Attribute fields support string, number, boolean, Vector3, Color3 and literal unions.",
        );
        Check::Invalid
    }

    /// The registration of a concrete Crucible class, with the rules its declaration breaks reported.
    pub(crate) fn registration(&mut self, cx: &mut ExtractCx<'_>, node: Node) -> Option<Class> {
        let instance = Self::instance_type(cx, node)?;
        let kind = self.kind_of(cx, instance)?;
        if node.modifiers().intersects(Modifiers::ABSTRACT.union(Modifiers::AMBIENT)) || node.is_ambient_context() {
            return None;
        }
        let mut constructor = None;
        let mut current = Some(instance);
        let mut visited = HashSet::new();
        while let Some(c) = current
            && visited.insert(c)
        {
            let declaration =
                c.symbol().and_then(|s| s.declarations().into_iter().find(|d| d.kind() == K::ClassDeclaration));
            constructor = declaration.and_then(|d| d.members().into_iter().find(|m| m.kind() == K::Constructor));
            if constructor.is_some() {
                break;
            }
            current = Self::bases(cx, c).first().copied();
        }
        let hidden =
            constructor.is_some_and(|c| c.modifiers().intersects(Modifiers::PRIVATE.union(Modifiers::PROTECTED)));
        if node.kind() != K::ClassDeclaration
            || node.name().is_none()
            || node.parent().is_none_or(|p| p.kind() != K::SourceFile)
            || node.has_type_parameters()
            || hidden
        {
            cx.report(
                node,
                Code::Rule(3),
                "Registered classes must be named, non-generic, module-level classes with public constructors.",
            );
            return None;
        }
        let symbol = cx.symbol_of_declaration(node)?;
        let static_type = cx.type_of_symbol(symbol);
        let construct = cx.construct_signatures(static_type).first().copied();
        let mut deps = Vec::new();
        for parameter in construct.map_or_else(Vec::new, |c| c.parameters()) {
            let declaration = parameter.value_declaration().unwrap_or(node);
            Self::global_references(cx, declaration.type_node(), declaration, &mut HashSet::new());
            if declaration.kind() == K::Parameter
                && (declaration.has_question_token() || declaration.is_rest() || declaration.initializer().is_some())
            {
                cx.report(declaration, Code::Rule(1), "Optional, defaulted and rest parameters are not injectable.");
            }
            let t = cx.type_of_symbol(parameter);
            deps.push(self.dependency(cx, t, declaration, false));
        }
        let mut provides = Vec::new();
        let mut stack = vec![instance];
        let mut seen = HashSet::new();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) || self.is_own_class(cx, current) {
                continue;
            }
            if current != instance
                && let Some(id) = self.type_id(cx, current, node)
                && !provides.contains(&id)
            {
                provides.push(id);
            }
            let mut next = Self::bases(cx, current);
            for declaration in current.symbol().map_or_else(Vec::new, |s| s.declarations()) {
                for implemented in declaration.implemented_types() {
                    next.push(cx.type_at(implemented));
                }
            }
            stack.extend(next.into_iter().rev());
        }
        let id = self.type_id(cx, instance, node);
        let mut class = Class { kind, id, deps, provides, tag: None, instance: Vec::new(), attributes: Vec::new() };
        if kind == ClassKind::Behaviour {
            let member = |cx: &mut ExtractCx<'_>, name: &str| {
                let property = cx.property_of_type(instance, name)?;
                Some(cx.type_of_symbol(property))
            };
            class.tag = member(cx, "tag").and_then(literal_string).map(Into::into);
            if class.tag.is_none() {
                cx.report(node, Code::Rule(2), "A concrete behaviour tag must be exactly one string literal.");
            }
            let mut parts = match member(cx, "instance") {
                Some(t) if t.flags().intersects(TypeFlags::UNION) => t.types().into_iter().map(Some).collect(),
                t => vec![t],
            };
            if parts.len() > 1 {
                let made = self.made(cx);
                parts.sort_by_key(|p| p.and_then(|t| made.get(&t.symbol()?)).map_or(usize::MAX, |m| m.turn));
            }
            let allowed = cx.resolve_type_name("Instances", node).map(|s| cx.declared_type(s));
            for part in parts {
                let name = part.and_then(|p| p.symbol()).map(|s| s.name());
                let expected = match (name, allowed) {
                    (Some(name), Some(allowed)) => cx.property_of_type(allowed, name),
                    _ => None,
                };
                let matches = expected.is_some_and(|e| cx.type_of_symbol(e).symbol() == part.and_then(|p| p.symbol()));
                if !matches {
                    cx.report(node, Code::Rule(8), "Instance type must be an Instances class or union of them.");
                }
                class.instance.push(name.unwrap_or("Instance").into());
            }
            for property in cx.properties_of_type(instance) {
                let Some(declaration) = property.value_declaration() else { continue };
                if declaration.kind() != K::PropertyDeclaration {
                    continue;
                }
                let Some(initializer) = declaration.initializer() else { continue };
                if initializer.kind() != K::CallExpression
                    || self.api.macro_at(cx, initializer) != Some(Macro::Attribute)
                {
                    continue;
                }
                let t = cx.type_of_symbol(property);
                let check = self.attribute_check(cx, t, declaration);
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
    fn made(&mut self, cx: &mut ExtractCx<'_>) -> &HashMap<Symbol, Made> {
        if self.registration.made.is_none() {
            let mut made = HashMap::new();
            let mut aliases = HashSet::new();
            for file in cx.source_files() {
                if !file.is_declaration() && !file.is_js() {
                    make_referenced(cx, file.root(), &mut made, &mut aliases);
                }
            }
            self.registration.made = Some(made);
        }
        self.registration.made.get_or_insert_default()
    }

    /// Whether `t` instantiates a generic parameter that its kind of use forbids.
    fn generic_target(&mut self, cx: &mut ExtractCx<'_>, t: Type, mode: Generic, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        if t.flags().intersects(TypeFlags::UNION_OR_INTERSECTION) {
            return t.types().into_iter().any(|part| self.generic_target(cx, part, mode, seen));
        }
        if self.is_own_class(cx, t) {
            return false;
        }
        let arguments = if t.is_reference() { cx.type_arguments(t) } else { Vec::new() };
        if !arguments.is_empty() {
            let generic = t.target();
            let tag = cx.property_of_type(generic, "tag").map(|p| cx.type_of_symbol(p));
            let parameters = generic.type_parameters();
            let behaviour = self.kind_of(cx, generic) == Some(ClassKind::Behaviour);
            if arguments.iter().enumerate().any(|(i, argument)| {
                !behaviour
                    || parameters.get(i).copied() != tag
                    || (mode != Generic::Provides && argument.flags() != TypeFlags::STRING)
            }) {
                return true;
            }
        }
        let declarations = t.symbol().map_or_else(Vec::new, |s| s.declarations());
        let base_mode =
            if declarations.iter().any(|d| d.kind() == K::ClassDeclaration) { Generic::Provides } else { mode };
        for base in Self::resolved_bases(cx, t) {
            if self.generic_target(cx, base, base_mode, seen) {
                return true;
            }
        }
        for declaration in declarations {
            for implemented in declaration.implemented_types() {
                let t = cx.type_at(implemented);
                if self.generic_target(cx, t, mode, seen) {
                    return true;
                }
            }
        }
        false
    }

    fn concrete_base(&mut self, cx: &mut ExtractCx<'_>, t: Type, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        if t.flags().intersects(TypeFlags::INTERSECTION) {
            return t.types().into_iter().any(|part| self.concrete_base(cx, part, seen));
        }
        if self.kind_of(cx, t).is_some()
            && self.api_name(cx, t.symbol()).is_none()
            && t.symbol().is_some_and(|s| s.declarations().into_iter().any(is_concrete_class))
        {
            return true;
        }
        Self::resolved_bases(cx, t).into_iter().any(|base| self.concrete_base(cx, base, seen))
    }

    fn promise(cx: &mut ExtractCx<'_>, t: Type, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        t.symbol().is_some_and(|s| s.name() == "Promise")
            || (t.flags().intersects(TypeFlags::UNION_OR_INTERSECTION)
                && t.types().into_iter().any(|part| Self::promise(cx, part, seen)))
            || Self::resolved_bases(cx, t).into_iter().any(|base| Self::promise(cx, base, seen))
    }

    /// CRU014 on an `All<T>` type reference.
    pub(crate) fn all_rule(&mut self, cx: &mut ExtractCx<'_>, n: Node) {
        let t = cx.type_from_type_node(n);
        if let Some(item) = self.all_target(cx, t)
            && self.generic_target(cx, item, Generic::Target, &mut HashSet::new())
        {
            cx.report(n, Code::Rule(14), "All targets must not instantiate generic parameters except string tags.");
        }
    }

    pub(crate) fn class_rules(&mut self, cx: &mut ExtractCx<'_>, n: Node) {
        let Some(t) = Self::instance_type(cx, n) else { return };
        if Self::resolved_bases(cx, t).into_iter().any(|base| self.concrete_base(cx, base, &mut HashSet::new())) {
            cx.report(n, Code::Rule(13), "Extend an abstract Crucible base, not a concrete registered class.");
        }
        if self.kind_of(cx, t).is_none() {
            return;
        }
        let abstract_ = n.modifiers().intersects(Modifiers::ABSTRACT);
        if !abstract_ && self.generic_target(cx, t, Generic::Provides, &mut HashSet::new()) {
            cx.report(n, Code::Rule(14), "Provided types must not instantiate non-tag generic parameters.");
        }
        for implemented in n.implemented_types() {
            let target = cx.type_at(implemented);
            if self.generic_target(cx, target, Generic::Provides, &mut HashSet::new()) {
                cx.report(n, Code::Rule(14), "Provided interfaces must not instantiate generic parameters.");
            }
        }
        let source = n.source_file().map(|f| f.path()).unwrap_or_default();
        let side = side(cx, &source, &PathBuf::from(&source));
        self.side_members(cx, t, n, side, &mut HashSet::new());
        for member in n.members() {
            if member.kind() != K::MethodDeclaration {
                continue;
            }
            let Some(name) = member.name().and_then(|nm| cx.symbol_at(nm)) else {
                continue;
            };
            let name = name.name();
            let hook = self.api.hooks.contains(name);
            let resembles = HOOK_LOOKALIKES.contains(&name)
                || self.api.hooks.iter().any(|h| h.to_lowercase() == name.to_lowercase() || one_edit(name, h));
            if !hook && resembles {
                cx.report(
                    member,
                    Code::Rule(9),
                    "Method resembles a lifecycle hook. Use its exact Crucible declaration name.",
                );
            }
            if hook {
                let asynchronous = member.modifiers().intersects(Modifiers::ASYNC);
                let returns_promise = !asynchronous && {
                    let signature = cx.signature_of_declaration(member);
                    let returned = cx.return_type(signature);
                    Self::promise(cx, returned, &mut HashSet::new())
                };
                if asynchronous || returns_promise {
                    cx.report(
                        member,
                        Code::Rule(12),
                        "Crucible hooks must be synchronous and must not return a Promise.",
                    );
                }
            }
        }
    }

    fn side_members(
        &self,
        cx: &mut ExtractCx<'_>,
        current: Type,
        class: Node,
        side: Option<Side>,
        seen: &mut HashSet<Type>,
    ) {
        if !seen.insert(current) || self.api_name(cx, current.symbol()).is_some() {
            return;
        }
        for declaration in current.symbol().map_or_else(Vec::new, |s| s.declarations()) {
            for member in declaration.members() {
                let Some(name) = member.name().and_then(|nm| cx.symbol_at(nm)) else {
                    continue;
                };
                let at = if declaration == class { member } else { class };
                if side != Some(Side::Client) && matches!(name.name(), "preRender" | "renderPriority") {
                    cx.report(at, Code::Rule(11), "preRender and renderPriority require a client class.");
                }
                if side != Some(Side::Server) && name.name() == "onShutdown" {
                    cx.report(at, Code::Rule(16), "onShutdown requires a server class.");
                }
            }
        }
        for base in Self::resolved_bases(cx, current) {
            self.side_members(cx, base, class, side, seen);
        }
    }

    pub(crate) fn inject_rules(&mut self, cx: &mut ExtractCx<'_>, n: Node) {
        let t = n.type_arguments().first().map(|&own| cx.type_from_type_node(own));
        let target = t.map(|t| self.all_target(cx, t).unwrap_or(t));
        let mut inside = false;
        let mut parent = n.parent();
        while let Some(p) = parent {
            if matches!(p.kind(), K::ClassDeclaration | K::ClassExpression)
                && !p.modifiers().intersects(Modifiers::ABSTRACT)
                && let Some(instance) = Self::instance_type(cx, p)
                && self.kind_of(cx, instance).is_some()
            {
                inside = true;
            }
            parent = p.parent();
        }
        if inside || !target.is_some_and(|t| Self::is_nominal(cx, t)) {
            cx.report(n, Code::Rule(5), "inject requires a class, interface or All target outside registered classes.");
        }
        if let Some(target) = target
            && self.generic_target(cx, target, Generic::Target, &mut HashSet::new())
        {
            cx.report(n, Code::Rule(14), "inject targets must not instantiate generic parameters except string tags.");
        }
    }

    pub(crate) fn inject_target(&mut self, cx: &mut ExtractCx<'_>, call: Node) -> Dependency {
        let target = call.type_arguments().first().and_then(|&argument| {
            let t = cx.type_from_type_node(argument);
            match self.all_target(cx, t) {
                Some(item) => self.type_id(cx, item, call).map(Dependency::All),
                None => self.type_id(cx, t, call).map(Dependency::One),
            }
        });
        target.unwrap_or(Dependency::Invalid)
    }

    pub(crate) fn attribute_rules(&mut self, cx: &mut ExtractCx<'_>, n: Node) {
        let field = n.parent().filter(|p| {
            p.kind() == K::PropertyDeclaration
                && p.initializer() == Some(n)
                && !p.modifiers().intersects(Modifiers::STATIC)
        });
        let behaviour = match field.and_then(|f| f.parent()).and_then(|class| Self::instance_type(cx, class)) {
            Some(t) => self.kind_of(cx, t) == Some(ClassKind::Behaviour),
            None => false,
        };
        let Some(field) = field.filter(|_| behaviour) else {
            cx.report(n, Code::Rule(15), "attribute() must initialize a behaviour instance field.");
            return;
        };
        let mut seen = HashSet::new();
        Self::global_references(cx, field.type_node(), field, &mut seen);
        for argument in n.type_arguments() {
            Self::global_references(cx, Some(argument), field, &mut seen);
        }
        let t = cx.type_at(field);
        self.attribute_check(cx, t, field);
    }
}

fn make_referenced(cx: &mut ExtractCx<'_>, n: Node, made: &mut HashMap<Symbol, Made>, aliases: &mut HashSet<Symbol>) {
    let name = match n.parent() {
        // The transformer's walk makes a class's base at the class, and an interface's bases only when it makes the
        // interface.
        Some(clause) if clause.kind() == K::HeritageClause => {
            let class = clause.parent().is_some_and(|c| matches!(c.kind(), K::ClassDeclaration | K::ClassExpression));
            if class && clause.is_extends_clause() { n.heritage_name() } else { None }
        }
        _ if n.kind() == K::TypeReference => n.type_name(),
        _ => None,
    };
    if let Some(symbol) = name.and_then(|name| cx.symbol_at(name)) {
        let symbol = cx.unalias(symbol);
        if symbol.flags().intersects(SymbolFlags::CLASS.union(SymbolFlags::INTERFACE)) {
            make(cx, symbol, made);
        } else if symbol.flags().intersects(SymbolFlags::TYPE_ALIAS) && aliases.insert(symbol) {
            for declaration in symbol.declarations() {
                if let Some(aliased) = declaration.type_node() {
                    make_referenced(cx, aliased, made, aliases);
                }
            }
        }
    }
    let mut children = Vec::new();
    n.for_each_child(&mut |child| children.push(child));
    for child in children {
        make_referenced(cx, child, made, aliases);
    }
}

/// Makes a class or interface type as TypeScript 5's `getDeclaredTypeOfClassOrInterface` does, and returns whether the
/// type has a `this` type.
fn make(cx: &mut ExtractCx<'_>, symbol: Symbol, made: &mut HashMap<Symbol, Made>) -> bool {
    if let Some(m) = made.get(&symbol) {
        return m.this_type;
    }
    made.insert(symbol, Made { turn: made.len(), this_type: false });
    let generic = symbol.declarations().into_iter().any(|d| {
        matches!(d.kind(), K::ClassDeclaration | K::ClassExpression | K::InterfaceDeclaration)
            && d.has_type_parameters()
    });
    let this_type = symbol.flags().intersects(SymbolFlags::CLASS) || generic || !thisless(cx, symbol, made);
    if let Some(m) = made.get_mut(&symbol) {
        m.this_type = this_type;
    }
    this_type
}

/// TypeScript 5's `isThislessInterface`, which makes each base interface it reaches.
fn thisless(cx: &mut ExtractCx<'_>, symbol: Symbol, made: &mut HashMap<Symbol, Made>) -> bool {
    for declaration in symbol.declarations() {
        if declaration.kind() != K::InterfaceDeclaration {
            continue;
        }
        if declaration.contains_this() {
            return false;
        }
        for base in declaration.extends_types() {
            let symbol = base.heritage_name().and_then(|name| cx.symbol_at(name)).map(|s| cx.unalias(s));
            let Some(base) = symbol.filter(|s| s.flags().intersects(SymbolFlags::INTERFACE)) else { return false };
            if make(cx, base, made) {
                return false;
            }
        }
    }
    true
}
