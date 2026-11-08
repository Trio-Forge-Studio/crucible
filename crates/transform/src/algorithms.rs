use super::ATTRIBUTE;
use super::{Attribute, Check, Class, Dependency, DiCx};
use crucible_compiler::extension as types;
use crucible_compiler::extension::Side;
use crucible_compiler::extension::{ClassKind, Literal};
use crucible_compiler::extension::{Code, is_global, is_reference, unalias};
use crucible_compiler::extension::{Game as Place, RbxPath, normalize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{SignatureKind, Type, TypeFlags};
use tsrs_core::P;
const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];
const HOOK_LOOKALIKES: [&str; 5] = ["onInit", "onStart", "onTick", "onPhysics", "onRender"];
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generic {
    Provides,
    Target,
}
#[derive(Clone, Copy)]
pub(super) struct Made {
    turn: usize,
    this_type: bool,
}
fn target(t: P<Type>) -> P<Type> {
    if is_reference(t) { t.target().unwrap_or(t) } else { t }
}
fn is_class_or_interface(d: P<Node>) -> bool {
    matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration)
}
fn is_concrete_class(d: P<Node>) -> bool {
    d.kind() == Kind::ClassDeclaration && !tsrs_ast::get_combined_modifier_flags(d).intersects(ModifierFlags::Abstract)
}
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

impl DiCx<'_> {
    fn is_own_class(&mut self, t: P<Type>) -> bool {
        self.core.api(t.symbol()).is_some_and(|name| OWN_CLASSES.contains(&name))
    }
    pub(super) fn all_rules(&mut self, n: P<Node>) {
        let t = self.core.checker.get_type_from_type_node_exported(n);
        if let Some(item) = self.all_target(t)
            && self.generic_target(item, Generic::Target, &mut HashSet::new())
        {
            self.core.report(
                n,
                Code::Rule(14),
                "All targets must not instantiate generic parameters except string tags.",
            );
        }
    }

    fn global_type(&mut self, t: P<Type>, at: P<Node>, seen: &mut HashSet<P<Type>>) {
        if !seen.insert(t) {
            return;
        }
        for symbol in [t.alias().and_then(|a| a.symbol()), t.symbol()].into_iter().flatten() {
            for &declaration in symbol.declarations() {
                if !self.core.is_library_declaration(declaration) && is_global(declaration) {
                    self.core.report(
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
            let symbol =
                self.core.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.core.checker, s));
            for &declaration in symbol.map_or(&[][..], |s| s.declarations()) {
                if declaration.kind() == Kind::TypeAliasDeclaration {
                    if is_global(declaration) {
                        self.core.report(
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
        let id = self
            .core
            .crucible
            .module_identity(file.file_name())
            .map(|(package, module)| Box::<str>::from(format!("{package}/{module}:{}", symbol.name())));
        if id.is_none() {
            self.core.report(
                at,
                Code::Setup,
                "Cannot compute a type id. The declaring package needs a name and a mirrored source/declaration layout.",
            );
        }
        id
    }

    fn all_target(&mut self, t: P<Type>) -> Option<P<Type>> {
        if self.core.api(t.symbol()) != Some("All") || !is_reference(t) {
            return None;
        }
        self.core.checker.get_type_arguments_exported(t).first().copied()
    }

    fn is_nominal(&mut self, t: P<Type>) -> bool {
        t.symbol().is_some_and(|s| {
            s.declarations().iter().any(|&d| is_class_or_interface(d))
                && self.core.checker.get_declared_type_of_symbol_exported(s) == target(t)
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
        let instance = match self.core.checker.resolve_name_exported("Instance", Some(at), SymbolFlags::Type, false) {
            Some(s) => {
                let declared = self.core.checker.get_declared_type_of_symbol_exported(s);
                self.core.checker.is_type_assignable_to_exported(t, declared)
            }
            None => false,
        };
        let generic = is_reference(t)
            && !self.core.checker.get_type_arguments_exported(t).is_empty()
            && (!all || self.generic_target(t, Generic::Target, &mut HashSet::new()));
        let concrete_behaviour = self.core.kind_of(t) == Some(ClassKind::Behaviour)
            && t.symbol().is_some_and(|s| s.declarations().iter().any(|&d| is_concrete_class(d)));
        if !nominal || instance || t.flags().intersects(TypeFlags::Union) || generic || (!all && concrete_behaviour) {
            let message = if concrete_behaviour {
                "Inject All<X> or call X.get(instance) for a concrete behaviour."
            } else {
                "Constructor parameter must be a plain class, interface or All<T>."
            };
            self.core.report(at, Code::Rule(1), message);
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
            && self.core.checker.resolve_name_exported(name, Some(at), SymbolFlags::Type, false) == Some(symbol)
        {
            return if name == "Vector3" { Check::Vector3 } else { Check::Color3 };
        }
        self.core.report(
            at,
            Code::Rule(7),
            "Attribute fields support string, number, boolean, Vector3, Color3 and literal unions.",
        );
        Check::Invalid
    }

    /// The registration of a concrete Crucible class, with the rules its declaration breaks reported.
    pub(super) fn registration(&mut self, node: P<Node>) -> Option<Class> {
        let instance = self.core.instance_type(node)?;
        let kind = self.core.kind_of(instance)?;
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
            current = self.core.bases(c).first().copied();
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
            self.core.report(
                node,
                Code::Rule(3),
                "Registered classes must be named, non-generic, module-level classes with public constructors.",
            );
            return None;
        }
        let symbol = self.core.checker.get_symbol_of_declaration_exported(node)?;
        let static_type = self.core.checker.get_type_of_symbol_exported(symbol);
        let construct =
            self.core.checker.get_signatures_of_type_exported(static_type, SignatureKind::Construct).first().copied();
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
                    self.core.report(
                        declaration,
                        Code::Rule(1),
                        "Optional, defaulted and rest parameters are not injectable.",
                    );
                }
            }
            let t = self.core.checker.get_type_of_symbol_exported(parameter);
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
            let mut next = self.core.bases(current);
            for &declaration in current.symbol().map_or(&[][..], |s| s.declarations()) {
                for implemented in implemented(declaration) {
                    next.push(self.core.checker.get_type_at_location(implemented));
                }
            }
            stack.extend(next.into_iter().rev());
        }
        let id = self.type_id(instance, node);
        let mut class = Class { kind, id, deps, provides, tag: None, instance: Vec::new(), attributes: Vec::new() };
        if kind == ClassKind::Behaviour {
            let member = |x: &mut DiCx<'_>, name: &str| {
                let property = x.checker.get_property_of_type_exported(instance, name)?;
                Some(x.checker.get_type_of_symbol_exported(property))
            };
            class.tag = member(self, "tag").and_then(types::literal_string).map(Into::into);
            if class.tag.is_none() {
                self.core.report(node, Code::Rule(2), "A concrete behaviour tag must be exactly one string literal.");
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
                .map(|s| self.core.checker.get_declared_type_of_symbol_exported(s));
            for part in parts {
                let name = part.and_then(|p| p.symbol()).map(|s| s.name());
                let expected = match (name, allowed) {
                    (Some(name), Some(allowed)) => self.core.checker.get_property_of_type_exported(allowed, name),
                    _ => None,
                };
                let matches = expected.is_some_and(|e| {
                    self.core.checker.get_type_of_symbol_exported(e).symbol() == part.and_then(|p| p.symbol())
                });
                if !matches {
                    self.core.report(node, Code::Rule(8), "Instance type must be an Instances class or union of them.");
                }
                class.instance.push(name.unwrap_or("Instance").into());
            }
            for &property in self.core.checker.get_properties_of_type_exported(instance) {
                let Some(declaration) = property.value_declaration() else { continue };
                if declaration.kind() != Kind::PropertyDeclaration {
                    continue;
                }
                let Some(initializer) = declaration.initializer() else { continue };
                if !self.named_call(initializer, ATTRIBUTE) {
                    continue;
                }
                let t = self.core.checker.get_type_of_symbol_exported(property);
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
        if self.state.made.is_none() {
            let mut made = HashMap::new();
            let mut aliases = HashSet::new();
            for &file in self.core.crucible.source_files() {
                if !file.is_declaration_file() && !file.is_js() {
                    self.make_referenced(file.as_node(), &mut made, &mut aliases);
                }
            }
            self.state.made = Some(made);
        }
        self.state.made.get_or_insert_default()
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
        if let Some(symbol) = name.and_then(|name| self.core.checker.get_symbol_at_location_exported(name)) {
            let symbol = unalias(&mut self.core.checker, symbol);
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
            matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration) && !d.type_parameters().is_empty()
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
                let symbol =
                    self.core.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.core.checker, s));
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
            if is_reference(t) { self.core.checker.get_type_arguments_exported(t).to_vec() } else { Vec::new() };
        if !arguments.is_empty() {
            let generic = target(t);
            let tag = self
                .checker
                .get_property_of_type_exported(generic, "tag")
                .map(|p| self.core.checker.get_type_of_symbol_exported(p));
            let parameters = generic.try_as_interface_type().map_or(&[][..], |i| i.type_parameters());
            let behaviour = self.core.kind_of(generic) == Some(ClassKind::Behaviour);
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
        for base in self.core.resolved_bases(t) {
            if self.generic_target(base, base_mode, seen) {
                return true;
            }
        }
        for &declaration in declarations {
            for implemented in implemented(declaration) {
                let t = self.core.checker.get_type_at_location(implemented);
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
        if self.core.kind_of(t).is_some()
            && self.core.api(t.symbol()).is_none()
            && t.symbol().is_some_and(|s| s.declarations().iter().any(|&d| is_concrete_class(d)))
        {
            return true;
        }
        self.core.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, seen))
    }

    fn promise(&mut self, t: P<Type>, seen: &mut HashSet<P<Type>>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        t.symbol().is_some_and(|s| s.name() == "Promise")
            || (t.flags().intersects(TypeFlags::UnionOrIntersection)
                && t.types().iter().any(|&part| self.promise(part, seen)))
            || self.core.resolved_bases(t).into_iter().any(|base| self.promise(base, seen))
    }

    pub(super) fn crucible_class_rules(&mut self, n: P<Node>, file: P<SourceFile>) {
        let Some(t) = self.core.instance_type(n) else { return };
        if self.core.resolved_bases(t).into_iter().any(|base| self.concrete_base(base, &mut HashSet::new())) {
            self.core.report(n, Code::Rule(13), "Extend an abstract Crucible base, not a concrete registered class.");
        }
        if self.core.kind_of(t).is_none() {
            return;
        }
        let abstract_ = tsrs_ast::get_combined_modifier_flags(n).intersects(ModifierFlags::Abstract);
        if !abstract_ && self.generic_target(t, Generic::Provides, &mut HashSet::new()) {
            self.core.report(n, Code::Rule(14), "Provided types must not instantiate non-tag generic parameters.");
        }
        for implemented in implemented(n) {
            let target = self.core.checker.get_type_at_location(implemented);
            if self.generic_target(target, Generic::Provides, &mut HashSet::new()) {
                self.core.report(n, Code::Rule(14), "Provided interfaces must not instantiate generic parameters.");
            }
        }
        let class_file = tsrs_ast::get_source_file_of_node(n).map(|f| PathBuf::from(f.file_name())).unwrap_or_default();
        let side = self.core.crucible.side(Path::new(file.file_name()), &class_file);
        self.side_members(t, n, side, &mut HashSet::new());
        for &member in n.members() {
            if member.kind() != Kind::MethodDeclaration {
                continue;
            }
            let Some(name) = member.name().and_then(|nm| self.core.checker.get_symbol_at_location_exported(nm)) else {
                continue;
            };
            let name = name.name();
            let hook = self.core.crucible.lifecycle_hooks().contains(name);
            let resembles = HOOK_LOOKALIKES.contains(&name)
                || self
                    .core
                    .crucible
                    .lifecycle_hooks()
                    .iter()
                    .any(|h| h.to_lowercase() == name.to_lowercase() || one_edit(name, h));
            if !hook && resembles {
                self.core.report(
                    member,
                    Code::Rule(9),
                    "Method resembles a lifecycle hook. Use its exact Crucible declaration name.",
                );
            }
            if hook {
                let asynchronous = tsrs_ast::get_combined_modifier_flags(member).intersects(ModifierFlags::Async);
                let returns_promise = !asynchronous && {
                    let signature = self.core.checker.get_signature_from_declaration_exported(member);
                    let returned = self.core.checker.get_return_type_of_signature_exported(signature);
                    self.promise(returned, &mut HashSet::new())
                };
                if asynchronous || returns_promise {
                    self.core.report(
                        member,
                        Code::Rule(12),
                        "Crucible hooks must be synchronous and must not return a Promise.",
                    );
                }
            }
        }
    }

    fn side_members(&mut self, current: P<Type>, class: P<Node>, side: Option<Side>, seen: &mut HashSet<P<Type>>) {
        if !seen.insert(current) || self.core.api(current.symbol()).is_some() {
            return;
        }
        for &declaration in current.symbol().map_or(&[][..], |s| s.declarations()) {
            for &member in declaration.members() {
                let Some(name) = member.name().and_then(|nm| self.core.checker.get_symbol_at_location_exported(nm))
                else {
                    continue;
                };
                let at = if declaration == class { member } else { class };
                if side != Some(Side::Client) && matches!(name.name(), "preRender" | "renderPriority") {
                    self.core.report(at, Code::Rule(11), "preRender and renderPriority require a client class.");
                }
                if side != Some(Side::Server) && name.name() == "onShutdown" {
                    self.core.report(at, Code::Rule(16), "onShutdown requires a server class.");
                }
            }
        }
        for base in self.core.resolved_bases(current) {
            self.side_members(base, class, side, seen);
        }
    }

    pub(super) fn inject_rules(&mut self, n: P<Node>) {
        let t = n.type_arguments().first().map(|&own| self.core.checker.get_type_from_type_node_exported(own));
        let target = t.map(|t| self.all_target(t).unwrap_or(t));
        let mut inside = false;
        let mut parent = n.parent();
        while let Some(p) = parent {
            if matches!(p.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
                && !tsrs_ast::get_combined_modifier_flags(p).intersects(ModifierFlags::Abstract)
                && let Some(instance) = self.core.instance_type(p)
                && self.core.kind_of(instance).is_some()
            {
                inside = true;
            }
            parent = p.parent();
        }
        if inside || !target.is_some_and(|t| self.is_nominal(t)) {
            self.core.report(
                n,
                Code::Rule(5),
                "inject requires a class, interface or All target outside registered classes.",
            );
        }
        if let Some(target) = target
            && self.generic_target(target, Generic::Target, &mut HashSet::new())
        {
            self.core.report(
                n,
                Code::Rule(14),
                "inject targets must not instantiate generic parameters except string tags.",
            );
        }
    }

    pub(super) fn inject_target(&mut self, call: P<Node>) -> Dependency {
        let target = call.type_arguments().first().and_then(|&argument| {
            let t = self.core.checker.get_type_from_type_node_exported(argument);
            match self.all_target(t) {
                Some(item) => self.type_id(item, call).map(Dependency::All),
                None => self.type_id(t, call).map(Dependency::One),
            }
        });
        target.unwrap_or(Dependency::Invalid)
    }

    pub(super) fn attribute_rules(&mut self, n: P<Node>) {
        let field = n.parent().filter(|p| {
            p.kind() == Kind::PropertyDeclaration
                && p.initializer() == Some(n)
                && !tsrs_ast::get_combined_modifier_flags(*p).intersects(ModifierFlags::Static)
        });
        let behaviour = field
            .and_then(|f| f.parent())
            .and_then(|class| self.core.instance_type(class))
            .is_some_and(|t| self.core.kind_of(t) == Some(ClassKind::Behaviour));
        let Some(field) = field.filter(|_| behaviour) else {
            self.core.report(n, Code::Rule(15), "attribute() must initialize a behaviour instance field.");
            return;
        };
        let mut seen = HashSet::new();
        self.global_references(field.type_node(), field, &mut seen);
        for &argument in n.type_arguments() {
            self.global_references(Some(argument), field, &mut seen);
        }
        let t = self.core.checker.get_type_at_location(field);
        self.attribute_check(t, field);
    }

    pub(super) fn boot(&mut self, call: P<Node>) {
        let options = call.arguments().first().map(|&a| skip_wrappers(a));
        let Some(options) = options.filter(|o| o.kind() == Kind::ObjectLiteralExpression) else {
            self.core.report(call, Code::Rule(6), "boot requires literal load globs.");
            return;
        };
        let properties = options.properties();
        let load = properties.iter().rposition(|&p| property_name(p) == Some("load"));
        if load.is_none() {
            self.core.report(options, Code::Rule(6), "boot requires a literal load property.");
        }
        for (i, &property) in properties.iter().enumerate() {
            let computed = property.name().is_some_and(|n| n.kind() == Kind::ComputedPropertyName);
            if property.kind() == Kind::SpreadAssignment || computed {
                if load.is_none_or(|load| i > load) {
                    self.core.report(
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
                self.core.report(property, Code::Rule(6), "load must be an array of string literals.");
                continue;
            };
            for &entry in entries.elements() {
                if entry.kind() != Kind::StringLiteral {
                    self.core.report(entry, Code::Rule(6), "load entry must be a string literal.");
                    continue;
                }
                self.core.site();
                match self.load_glob(entry.text()) {
                    Ok(glob) => {
                        self.put(entry, "load", glob);
                    }
                    Err(message) => self.core.report(entry, Code::Rule(6), message),
                }
            }
        }
    }

    /// A load glob, relative to the package root, as the instance path pattern of the modules it matches.
    fn load_glob(&self, glob: &str) -> Result<String, &'static str> {
        let Some(game) = &self.core.game else {
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
        self.core.crucible.source_files().iter().any(|&file| {
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
        for &file in self.core.crucible.source_files() {
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
