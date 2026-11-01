//! Crucible's class metadata and macros, ported from `packages/crucible/transformer-src`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tsrs_ast::{Kind, ModifierFlags, Node, Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, SignatureKind, Type, TypeFlags};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;

use super::{Extractor, types};
use crate::facts::{Attribute, Check, Class, ClassKind, Crucible, Dependency, Literal};
use crate::macros::{Macro, MacroKind};

const PACKAGE: &str = "@trioforge/crucible";
const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];

struct Package {
    dir: PathBuf,
    json: serde_json::Value,
}

/// The exports of `@trioforge/crucible` by their unaliased symbols, as the transform's `api(symbol)` reads them.
pub(super) struct Api {
    exports: HashMap<P<Symbol>, &'static str>,
    packages: HashMap<PathBuf, Option<std::rc::Rc<Package>>>,
    root_dir: Option<PathBuf>,
}

impl Api {
    pub fn new(program: &'static Program, c: &mut Checker) -> Api {
        let options = program.options();
        let root_dir = (!options.root_dir.is_empty()).then(|| PathBuf::from(&options.root_dir));
        let mut api = Api { exports: HashMap::new(), packages: HashMap::new(), root_dir };
        for &file in program.source_files() {
            if api.package_of(Path::new(file.file_name())).is_none_or(|p| p.json["name"] != PACKAGE) {
                continue;
            }
            let Some(module) = file.as_node().symbol() else { continue };
            for symbol in c.get_exports_of_module_exported(module) {
                let target = unalias(c, symbol);
                api.exports.entry(target).or_insert(symbol.name());
            }
        }
        api
    }

    fn name(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.exports.get(&unalias(c, symbol?)).copied()
    }

    /// The Crucible macro a callee resolves to.
    pub fn macro_at(&self, c: &mut CheckerHandle, callee: P<Node>) -> Option<Macro> {
        if self.exports.is_empty() {
            return None;
        }
        let symbol = c.get_symbol_at_location_exported(callee);
        let name = self.name(c, symbol)?;
        Macro::ALL.iter().copied().find(|m| m.kind() == MacroKind::Crucible && m.name() == name)
    }

    fn package_of(&mut self, file: &Path) -> Option<std::rc::Rc<Package>> {
        let mut dir = file.parent()?.to_path_buf();
        let mut visited = Vec::new();
        let found = loop {
            if let Some(cached) = self.packages.get(&dir) {
                break cached.clone();
            }
            visited.push(dir.clone());
            let manifest = dir.join("package.json");
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                break Some(std::rc::Rc::new(Package { dir: dir.clone(), json }));
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

    /// The transform's `typeId`: `<package>/<module path>:<name>`, with the module path relative to the source root
    /// for sources and to the matching `types` entry's directory for declarations.
    fn type_id(&mut self, file: &str, name: &str) -> Option<Box<str>> {
        let file = Path::new(file);
        let owner = self.package_of(file)?;
        let package = owner.json["name"].as_str()?;
        let file_name = file.to_string_lossy();
        let root = if file_name.ends_with(".d.ts") || file_name.ends_with(".d.tsx") {
            let exports = &owner.json["exports"];
            let mut entries = Vec::new();
            types_condition(if exports.get(".").is_some() { &exports["."] } else { exports }, &mut entries);
            let entry = entries
                .into_iter()
                .find(|entry| owner.dir.join(entry).parent().is_some_and(|dir| file.starts_with(normalize(dir))))
                .or_else(|| owner.json["types"].as_str().or(owner.json["typings"].as_str()).map(str::to_owned))
                .unwrap_or_else(|| "index.d.ts".to_owned());
            normalize(owner.dir.join(entry).parent()?)
        } else {
            match &self.root_dir {
                Some(root) if file.starts_with(root) => root.clone(),
                _ => owner.dir.clone(),
            }
        };
        let relative = file.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
        let module = [".d.tsx", ".d.ts", ".tsx", ".ts"].iter().find_map(|ext| relative.strip_suffix(ext))?;
        Some(format!("{package}/{module}:{name}").into())
    }
}

fn types_condition(value: &serde_json::Value, out: &mut Vec<String>) {
    let Some(map) = value.as_object() else { return };
    match map.get("types").and_then(|t| t.as_str()) {
        Some(types) => out.push(types.to_owned()),
        None => map.values().for_each(|v| types_condition(v, out)),
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

fn unalias(c: &mut Checker, s: P<Symbol>) -> P<Symbol> {
    if s.flags().intersects(SymbolFlags::Alias) { c.get_aliased_symbol(s) } else { s }
}

/// The transform's `type.target ?? type`.
fn target(t: P<Type>) -> P<Type> {
    if t.object_flags().intersects(ObjectFlags::Reference) { t.target().unwrap_or(t) } else { t }
}

impl Extractor {
    fn api(&mut self, symbol: Option<P<Symbol>>) -> Option<&'static str> {
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

    fn id(&mut self, t: P<Type>) -> Option<Box<str>> {
        let symbol = t.symbol()?;
        let declaration = symbol
            .declarations()
            .iter()
            .copied()
            .find(|d| matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration))?;
        let file = tsrs_ast::get_source_file_of_node(declaration)?;
        self.crucible.type_id(file.file_name(), symbol.name())
    }

    fn all_target(&mut self, t: P<Type>) -> Option<P<Type>> {
        if self.api(t.symbol()) != Some("All") {
            return None;
        }
        self.checker.get_type_arguments_exported(t).first().copied()
    }

    /// The transform's `injectable`, without the CRU014 generic-target check, which phase 4 ports with the rules.
    fn dependency(&mut self, t: P<Type>, all: bool) -> Dependency {
        if !all && let Some(item) = self.all_target(t) {
            return match self.dependency(item, true) {
                Dependency::One(id) => Dependency::All(id),
                other => other,
            };
        }
        let symbol = t.symbol();
        let nominal = symbol.is_some_and(|s| {
            s.declarations().iter().any(|d| matches!(d.kind(), Kind::ClassDeclaration | Kind::InterfaceDeclaration))
        }) && symbol.is_some_and(|s| self.checker.get_declared_type_of_symbol_exported(s) == target(t));
        let instance = match self.checker.resolve_name_exported("Instance", None, SymbolFlags::Type, false) {
            Some(s) => {
                let declared = self.checker.get_declared_type_of_symbol_exported(s);
                self.checker.is_type_assignable_to_exported(t, declared)
            }
            None => false,
        };
        let generic = !all
            && t.object_flags().intersects(ObjectFlags::Reference)
            && !self.checker.get_type_arguments_exported(t).is_empty();
        let concrete_behaviour = !all
            && self.class_kind(t, &mut HashSet::new()) == Some(ClassKind::Behaviour)
            && symbol.is_some_and(|s| {
                s.declarations().iter().any(|d| {
                    d.kind() == Kind::ClassDeclaration && !d.modifier_flags().intersects(ModifierFlags::Abstract)
                })
            });
        if !nominal || instance || t.flags().intersects(TypeFlags::Union) || generic || concrete_behaviour {
            return Dependency::Invalid;
        }
        self.id(t).map_or(Dependency::Invalid, Dependency::One)
    }

    fn check(&mut self, t: P<Type>) -> Check {
        let flags = t.flags();
        if flags.intersects(TypeFlags::Boolean) {
            return Check::Boolean;
        }
        if flags.intersects(TypeFlags::Union) {
            let parts: Vec<P<Type>> =
                t.types().iter().copied().filter(|p| !p.flags().intersects(TypeFlags::Undefined)).collect();
            let pair = parts.iter().any(|&p| types::literal_bool(p) == Some(true))
                && parts.iter().any(|&p| types::literal_bool(p) == Some(false));
            let mut checks: Vec<Check> = parts
                .into_iter()
                .filter(|p| !(pair && p.flags().intersects(TypeFlags::BooleanLiteral)))
                .map(|p| self.check(p))
                .collect();
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
        match t.symbol() {
            Some(s) if Some(s) == self.globals.vector3 => Check::Vector3,
            Some(s) if Some(s) == self.globals.color3 => Check::Color3,
            _ => Check::Invalid,
        }
    }

    fn is_crucible_call(&mut self, n: P<Node>, m: Macro) -> bool {
        n.kind() == Kind::CallExpression
            && self.crucible.macro_at(&mut self.checker, n.as_call_expression().expression) == Some(m)
    }

    /// The transform's `registration`, for a class whose instance type extends `Subsystem` or `CrucibleBehaviour`.
    pub(super) fn crucible_registration(&mut self, node: P<Node>) -> Option<Class> {
        let symbol = self.checker.get_symbol_of_declaration_exported(node)?;
        let instance = self.checker.get_declared_type_of_symbol_exported(symbol);
        let kind = self.class_kind(instance, &mut HashSet::new())?;
        let static_type = self.checker.get_type_of_symbol_exported(symbol);
        let construct =
            self.checker.get_signatures_of_type_exported(static_type, SignatureKind::Construct).first().copied();
        let mut deps = Vec::new();
        for &parameter in construct.map_or(&[][..], |c| c.parameters()) {
            let t = self.checker.get_type_of_symbol_exported(parameter);
            deps.push(self.dependency(t, false));
        }
        let mut provides = Vec::new();
        let mut stack = vec![instance];
        let mut seen = HashSet::new();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) || self.api(current.symbol()).is_some_and(|name| OWN_CLASSES.contains(&name)) {
                continue;
            }
            if current != instance
                && let Some(id) = self.id(current)
                && !provides.contains(&id)
            {
                provides.push(id);
            }
            let mut next = self.bases(current);
            for &declaration in current.symbol().map_or(&[][..], |s| s.declarations()) {
                let clauses = match declaration.kind() {
                    Kind::ClassDeclaration => declaration.as_class_declaration().heritage_clauses(),
                    Kind::InterfaceDeclaration => declaration.as_interface_declaration().heritage_clauses(),
                    _ => None,
                };
                for &clause in clauses.map_or(&[][..], |l| l.nodes()) {
                    if clause.as_heritage_clause().token == Kind::ImplementsKeyword {
                        for &implemented in clause.as_heritage_clause().types.get().nodes() {
                            next.push(self.checker.get_type_at_location(implemented));
                        }
                    }
                }
            }
            stack.extend(next.into_iter().rev());
        }
        let mut class = Class {
            kind,
            id: self.id(instance),
            deps,
            provides,
            tag: None,
            instance: Vec::new(),
            attributes: Vec::new(),
        };
        if kind == ClassKind::Behaviour {
            let member = |x: &mut Extractor, name: &str| {
                let property = x.checker.get_property_of_type_exported(instance, name)?;
                Some(x.checker.get_type_of_symbol_exported(property))
            };
            class.tag = member(self, "tag").and_then(types::literal_string).map(Into::into);
            if let Some(t) = member(self, "instance") {
                let parts = if t.flags().intersects(TypeFlags::Union) { t.types().to_vec() } else { vec![t] };
                class.instance = parts.iter().map(|p| p.symbol().map_or("Instance", |s| s.name()).into()).collect();
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
                let check = self.check(t);
                class.attributes.push(Attribute {
                    field: property.name().into(),
                    check,
                    required: initializer.arguments().is_empty(),
                });
            }
        }
        Some(class)
    }

    /// `inject<T>()` and `inject<All<T>>()`.
    pub(super) fn crucible_inject(&mut self, call: P<Node>) -> Option<Crucible> {
        let &argument = call.type_arguments().first()?;
        let t = self.checker.get_type_from_type_node_exported(argument);
        let target = match self.all_target(t) {
            Some(item) => self.id(item).map(Dependency::All),
            None => self.id(t).map(Dependency::One),
        };
        Some(Crucible::Inject(target.unwrap_or(Dependency::Invalid)))
    }
}
