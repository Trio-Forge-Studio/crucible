//! Crucible's class metadata and macros. Type ids must equal the ones `packages/crucible/transformer-src` writes,
//! because packages built by either compiler register classes with each other.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tsrs_ast::{Kind, ModifierFlags, Node, Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, SignatureKind, Type, TypeFlags};
use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::P;
use tsrs_tsoptions as tsoptions;

use super::{Extractor, types};
use crate::facts::{Attribute, Check, Class, ClassKind, Crucible, Dependency, Literal};
use crate::macros::Macro;
use crate::program::ConfigHost;

const PACKAGE: &str = "@trioforge/crucible";
const OWN_CLASSES: [&str; 3] = ["Subsystem", "CrucibleBehaviour", "CrucibleObject"];
/// `@trioforge/crucible/emit` exports a runtime `attribute` that is not the macro.
const MACRO_ENTRIES: [(&str, &[Macro]); 3] =
    [(".", &[Macro::Attribute, Macro::Inject]), ("./server", &[Macro::Boot]), ("./client", &[Macro::Boot])];

struct Package {
    dir: PathBuf,
    json: serde_json::Value,
}

pub(super) struct Api {
    exports: HashMap<P<Symbol>, &'static str>,
    macros: HashMap<P<Symbol>, Macro>,
    packages: HashMap<PathBuf, Option<Rc<Package>>>,
    configs: HashMap<PathBuf, Option<PathBuf>>,
    roots: HashMap<PathBuf, Option<PathBuf>>,
    root_dir: Option<PathBuf>,
    host: ConfigHost,
}

impl Api {
    pub fn new(program: &'static Program, c: &mut Checker, host: ConfigHost) -> Api {
        let options = program.options();
        let root_dir = (!options.root_dir.is_empty()).then(|| PathBuf::from(&options.root_dir));
        let mut api = Api {
            exports: HashMap::new(),
            macros: HashMap::new(),
            packages: HashMap::new(),
            configs: HashMap::new(),
            roots: HashMap::new(),
            root_dir,
            host,
        };
        let mut modules = Vec::new();
        for &file in program.source_files() {
            let path = PathBuf::from(file.file_name());
            if api.package_of(&path).is_none_or(|p| p.json["name"] != PACKAGE) {
                continue;
            }
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
                for &symbol in &exports {
                    if let Some(&m) = macros.iter().find(|m| m.name() == symbol.name()) {
                        let target = unalias(c, symbol);
                        api.macros.insert(target, m);
                    }
                }
            }
        }
        api
    }

    fn name(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<&'static str> {
        self.exports.get(&unalias(c, symbol?)).copied()
    }

    fn macro_of(&self, c: &mut Checker, symbol: Option<P<Symbol>>) -> Option<Macro> {
        self.macros.get(&unalias(c, symbol?)).copied()
    }

    pub fn macro_at(&self, c: &mut CheckerHandle, call: P<Node>) -> Option<Macro> {
        if self.macros.is_empty() {
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
                let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
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
        n.kind() == Kind::CallExpression && self.crucible.macro_at(&mut self.checker, n) == Some(m)
    }

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

    pub(super) fn crucible_inject(&mut self, call: P<Node>) -> Crucible {
        let target = call.type_arguments().first().and_then(|&argument| {
            let t = self.checker.get_type_from_type_node_exported(argument);
            match self.all_target(t) {
                Some(item) => self.id(item).map(Dependency::All),
                None => self.id(t).map(Dependency::One),
            }
        });
        Crucible::Inject(target.unwrap_or(Dependency::Invalid))
    }
}
