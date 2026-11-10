use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crucible_compiler::extension::{
    ElementFlags, File, Kind, ModifierFlags, Node, NodeFlags, ObjectFlags, SignatureKind, Symbol, SymbolFlags, Type,
    TypeFlags, is_numeric_name, js_number,
};

use crate::Code;
use crate::api::{Macro, normalize};
use crate::declarations::{
    DeclarationKind, MAX_ASK_TIMEOUT_SECONDS, MAX_SCHEMA_NODES, MAX_UNION_BRANCHES, Position, RESERVED_HANDLE_NAMES,
    Side, Spec, TypeArguments, VIEW_NAME_MAX_BYTES, VIEW_NAME_PREFIX,
};
use crate::facts::{ClassKind, Declaration, Fact, Field, KeyLiteral, Literal, MapKey, Schema, Tail};
use crate::registration::{Walk, is_global, is_reference};

#[derive(Default)]
pub(crate) struct Network {
    declarations: Option<Vec<Indexed>>,
    reached: Option<HashSet<Symbol>>,
    pub(crate) granted: Option<bool>,
}

struct Indexed {
    call: Node,
    kind: DeclarationKind,
    name: Option<&'static str>,
}

#[derive(Clone, Copy, Default)]
struct Context {
    /// The server sends it, and a client may not have streamed in an Instance it names.
    to_client: bool,
    nilable: bool,
    path: bool,
    marked: bool,
    root: bool,
}

struct Lowering {
    call: Node,
    spec: Spec,
    name: &'static str,
    report: bool,
    reached: HashSet<Symbol>,
    active: HashSet<Type>,
    inherited: HashSet<Symbol>,
    referenced: HashSet<Node>,
    /// The `extends` clauses of reached classes and interfaces, walked after the value so that a deep heritage chain
    /// does not nest calls.
    bases: Vec<(Node, String)>,
}

enum Engine {
    Vector3,
    Color3,
    CFrame,
    Instance(&'static str),
    Unsupported,
}

impl Engine {
    fn runtime(&self) -> Option<&'static str> {
        match self {
            Engine::Vector3 => Some("Vector3"),
            Engine::Color3 => Some("Color3"),
            Engine::CFrame => Some("CFrame"),
            Engine::Instance(_) => Some("Instance"),
            Engine::Unsupported => None,
        }
    }
}

enum Collection {
    Tuple,
    Array,
    Map,
}

enum Width {
    Brand(&'static str),
    Conflicting,
}

#[derive(Clone, Copy)]
pub(crate) enum Standard {
    Types,
    CompilerTypes,
}

fn node_text(n: Node) -> Option<&'static str> {
    matches!(
        n.kind(),
        Kind::Identifier
            | Kind::PrivateIdentifier
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::NoSubstitutionTemplateLiteral
    )
    .then(|| n.text())
}

fn segment(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_name(name: &str, max_bytes: usize) -> bool {
    name.len() <= max_bytes && name.split('.').all(segment) && name.split('.').next() != Some("Crucible")
}

/// A unique symbol's property name.
fn is_symbol_name(name: &str) -> bool {
    name.starts_with("\u{7f}@")
}

fn literal(t: Type) -> Option<Literal> {
    if let Some(s) = t.literal_string() {
        return Some(Literal::String(s.into()));
    }
    if let Some(n) = t.literal_number() {
        return Some(Literal::Number(n));
    }
    t.literal_bool().map(Literal::Boolean)
}

fn compare(a: &Literal, b: &Literal) -> Ordering {
    let rank = |l: &Literal| match l {
        Literal::Boolean(_) => 0,
        Literal::Number(_) => 1,
        Literal::String(_) => 2,
    };
    match (a, b) {
        (Literal::Boolean(a), Literal::Boolean(b)) => a.cmp(b),
        (Literal::Number(a), Literal::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Literal::String(a), Literal::String(b)) => a.as_bytes().cmp(b.as_bytes()),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// A literal's identity, with `0` and `-0` one value as in JavaScript.
fn literal_key(l: &Literal) -> (u8, String) {
    match l {
        Literal::Boolean(b) => (0, b.to_string()),
        Literal::Number(n) => (1, if *n == 0.0 { "0".to_owned() } else { n.to_string() }),
        Literal::String(s) => (2, s.to_string()),
    }
}

fn members(t: Type) -> Vec<Type> {
    if t.flags().intersects(TypeFlags::Union) { t.types() } else { vec![t] }
}

fn absent(t: Type) -> bool {
    t.flags().intersects(TypeFlags::Undefined)
        || (t.flags().intersects(TypeFlags::Union) && t.types().into_iter().any(absent))
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

pub(crate) fn replica(node: &Schema) -> Schema {
    let Schema::Object(fields) = node else { return node.clone() };
    let mut kept = Vec::new();
    for field in fields.iter().filter(|f| !f.server_only) {
        let value = replica(&field.value);
        if matches!(&value, Schema::Object(fields) if fields.is_empty()) {
            continue;
        }
        kept.push(Field { name: field.name.clone(), value, server_only: false });
    }
    Schema::Object(kept)
}

fn server_only_paths(node: &Schema) -> Vec<Vec<Box<str>>> {
    fn walk(node: &Schema, prefix: &mut Vec<Box<str>>, out: &mut Vec<Vec<Box<str>>>) {
        let Schema::Object(fields) = node else { return };
        for field in fields {
            prefix.push(field.name.clone());
            if field.server_only {
                out.push(prefix.clone());
            } else {
                walk(&field.value, prefix, out);
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(node, &mut Vec::new(), &mut out);
    out
}

fn view_name_problems(store: &str, node: &Schema) -> Vec<(String, String)> {
    let Schema::Object(fields) = node else { return Vec::new() };
    let prefix = format!("{VIEW_NAME_PREFIX}{store}.");
    let mut problems = Vec::new();
    for field in fields {
        let name = format!("{prefix}{}", field.name);
        if !segment(&field.name) {
            problems.push((
                format!("value[{}]", json_string(&field.name)),
                "a field the owner's client receives is named by one declaration segment, [A-Za-z][A-Za-z0-9_]*; rename it or mark it serverOnly".to_owned(),
            ));
        } else if name.len() > VIEW_NAME_MAX_BYTES {
            problems.push((
                format!("value.{}", field.name),
                format!(
                    "its view name {name} is {} bytes, over {VIEW_NAME_MAX_BYTES}; this store allows field names of at most {} bytes; rename the field, shorten the store name, or mark it serverOnly",
                    name.len(),
                    VIEW_NAME_MAX_BYTES as isize - prefix.len() as isize
                ),
            ));
        }
    }
    problems
}

pub(crate) fn calls(n: Node, out: &mut Vec<Node>) {
    if n.kind() == Kind::CallExpression {
        out.push(n);
    }
    n.for_each_child(&mut |child| calls(child, out));
}

fn added_declarations(body: Option<Node>, out: &mut Vec<Node>) {
    let Some(body) = body else { return };
    let statements = if body.kind() == Kind::ModuleBlock { body.statements().iter().collect() } else { vec![body] };
    for statement in statements {
        if statement.kind() == Kind::VariableStatement {
            out.extend(statement.declarations().iter());
        } else {
            out.push(statement);
        }
        if statement.kind() == Kind::ModuleDeclaration {
            added_declarations(statement.body(), out);
        }
    }
}

fn declaration_file(declaration: Node) -> PathBuf {
    normalize(Path::new(&declaration.file().path()))
}

fn ask_timeout(options: Node) -> Option<f64> {
    if options.kind() != Kind::ObjectLiteralExpression {
        return None;
    }
    let properties = options.properties();
    let (1, Some(property)) = (properties.len(), properties.get(0)) else { return None };
    if property.kind() != Kind::PropertyAssignment {
        return None;
    }
    let name = property.name()?;
    if !matches!(name.kind(), Kind::Identifier | Kind::StringLiteral) || name.text() != "timeout" {
        return None;
    }
    let value = property.initializer().filter(|i| i.kind() == Kind::NumericLiteral)?;
    let seconds = js_number(value.text());
    (seconds > 0.0 && seconds <= MAX_ASK_TIMEOUT_SECONDS).then_some(seconds)
}

impl Walk<'_, '_> {
    pub(crate) fn file_side(&mut self, file: File) -> Option<Side> {
        let path = PathBuf::from(file.path());
        self.side(&path, &path)
    }

    pub(crate) fn standard(&mut self, symbol: Symbol, which: Standard) -> bool {
        symbol.declarations().into_iter().any(|d| {
            let name = d.file().path();
            let package = self.cx.package_name(Path::new(&name));
            match which {
                Standard::Types => name.contains("/@rbxts/types/") || package.as_deref() == Some("@rbxts/types"),
                Standard::CompilerTypes => {
                    name.contains("/@rbxts/compiler-types/")
                        || matches!(package.as_deref(), Some("@rbxts/compiler-types" | "@trioforge/compiler-types"))
                }
            }
        })
    }

    fn signatures_of(&mut self, t: Type) -> bool {
        !self.cx.signatures_of_type(t, SignatureKind::Call).is_empty()
            || !self.cx.signatures_of_type(t, SignatureKind::Construct).is_empty()
    }

    fn shown(&mut self, t: Type) -> String {
        self.cx.type_text(t)
    }

    fn declarations(&mut self) -> &[Indexed] {
        if self.state.network.declarations.is_none() {
            let mut found = Vec::new();
            for file in self.cx.files() {
                if file.is_declaration() || file.is_js() {
                    continue;
                }
                let mut all = Vec::new();
                calls(file.root(), &mut all);
                for call in all {
                    if let Some(Macro::Declaration(kind)) = self.macro_at(call) {
                        let name =
                            call.arguments().get(0).filter(|a| a.kind() == Kind::StringLiteral).map(|a| a.text());
                        found.push(Indexed { call, kind, name });
                    }
                }
            }
            self.state.network.declarations = Some(found);
        }
        self.state.network.declarations.as_deref().unwrap_or_default()
    }

    fn location(&self, n: Node) -> String {
        let file = n.file();
        let line = file.position(n.span().start).line;
        let path = file.path();
        let dir = format!("{}/", self.cx.config_dir().to_string_lossy().replace('\\', "/").trim_end_matches('/'));
        format!("{}:{line}", path.strip_prefix(&dir).unwrap_or(&path))
    }

    fn placed(call: Node, file: File) -> bool {
        let declaration = call.parent().filter(|d| {
            d.kind() == Kind::VariableDeclaration
                && d.initializer() == Some(call)
                && d.name().is_some_and(|n| n.kind() == Kind::Identifier)
        });
        let list = declaration
            .and_then(|d| d.parent())
            .filter(|l| l.kind() == Kind::VariableDeclarationList && l.flags().intersects(NodeFlags::Const));
        let statement = list.and_then(|l| l.parent()).filter(|s| s.kind() == Kind::VariableStatement);
        statement.is_some_and(|s| {
            s.parent() == Some(file.root())
                && s.combined_modifier_flags().intersects(ModifierFlags::Export)
                && file.is_module()
        })
    }

    pub(crate) fn declaration(&mut self, call: Node, kind: DeclarationKind, file: File) {
        let spec = kind.spec();
        if !Self::placed(call, file) || self.file_side(file) != Some(spec.side) {
            let message =
                format!("Declare {} as an exported top-level const in a {} module.", spec.label, spec.side.folder());
            self.report(call, Code::Rule(17), &message);
        }
        let count = call.type_arguments().len();
        let fits = match spec.type_arguments {
            TypeArguments::Exactly(n) => count == n,
            TypeArguments::AtMostOne => count <= 1,
        };
        if !fits {
            let name = kind.name();
            let message = match (spec.position, spec.type_arguments) {
                (Position::Payload, _) => format!("{name} needs explicit payload types; use undefined for no payload."),
                (_, TypeArguments::AtMostOne) => format!("{name} needs at most one type argument."),
                _ => format!("{name} needs an explicit type argument."),
            };
            self.report(call, Code::Rule(17), &message);
        }
        let arguments = call.arguments();
        let name = arguments
            .get(0)
            .filter(|n| n.kind() == Kind::StringLiteral)
            .map(|n| n.text())
            .filter(|text| valid_name(text, spec.max_bytes));
        match name {
            None => {
                let message = format!(
                    "{} names are 1 to {} bytes of dot-separated segments [A-Za-z][A-Za-z0-9_]*; Crucible is reserved.",
                    spec.names, spec.max_bytes
                );
                self.report(call, Code::Rule(18), &message);
            }
            Some(text) => {
                let other = self
                    .declarations()
                    .iter()
                    .find(|d| d.call != call && d.name == Some(text) && d.kind.spec().names == spec.names)
                    .map(|d| d.call);
                if let Some(other) = other {
                    let message = format!("{text} is already declared at {}.", self.location(other));
                    self.report(call, Code::Rule(18), &message);
                }
            }
        }
        let mut tail = Tail::None;
        if kind == DeclarationKind::AskServer {
            let timeout = arguments.get(1).and_then(|options| {
                let timeout = ask_timeout(options);
                if timeout.is_none() {
                    self.report(
                        call,
                        Code::Rule(17),
                        "askServer options must be { timeout: <number literal in (0, 120]> }.",
                    );
                }
                timeout
            });
            tail = Tail::Timeout(timeout);
        }
        if arguments.len() > spec.arguments {
            self.report(call, Code::Rule(17), "Unexpected declaration arguments.");
        }
        let (schemas, _) = self.lower_declaration(call, kind, true);
        if spec.position != Position::Payload && schemas.is_empty() {
            let message = format!("Cannot read {}'s value type; give it an explicit type argument.", kind.name());
            self.report(call, Code::Rule(17), &message);
        }
        if spec.handles
            && let Some(value) = schemas.first()
        {
            tail = Tail::ServerOnly(server_only_paths(value));
        }
        self.store(call, Fact::Declaration(Declaration { kind, schemas, tail }));
    }

    fn lower_declaration(&mut self, call: Node, kind: DeclarationKind, report: bool) -> (Vec<Schema>, HashSet<Symbol>) {
        let spec = kind.spec();
        let first = call.arguments().get(0);
        let mut l = Lowering {
            call,
            spec,
            name: first.and_then(node_text).unwrap_or(kind.name()),
            report,
            reached: HashSet::new(),
            active: HashSet::new(),
            inherited: HashSet::new(),
            referenced: HashSet::new(),
            bases: Vec::new(),
        };
        let state = spec.position != Position::Payload;
        let root = if state { "value" } else { "payload" };
        for argument in call.type_arguments().iter() {
            self.references(&mut l, Some(argument), root);
        }
        let mut result = Vec::new();
        if !state {
            for (i, argument) in call.type_arguments().iter().enumerate() {
                let t = self.cx.type_from_type_node(argument);
                let location = match i {
                    1 => "response",
                    _ if kind == DeclarationKind::AskServer => "request",
                    _ => "payload",
                };
                let context = Context { to_client: kind == DeclarationKind::ToClient || i == 1, ..Context::default() };
                let schema = self.lower(&mut l, t, location, context, 1);
                result.push(schema);
            }
        } else if let Some(target) = self.target_of(call) {
            let value =
                self.lower(&mut l, target, "value", Context { path: true, root: true, ..Context::default() }, 1);
            if spec.store && !matches!(value, Schema::Object(_) | Schema::Map(..)) {
                let reason = format!("a store's value is a record; wrap a {} in an object", self.shown(target));
                self.fail(&l, "value", &reason);
            } else if spec.handles && matches!(value, Schema::Map(..)) {
                self.fail(
                    &l,
                    "value",
                    "a profile store's value is an object with declared fields; wrap the record in a field",
                );
            }
            result.push(value);
        }
        while let Some((base, location)) = l.bases.pop() {
            self.references(&mut l, Some(base), &location);
            let t = self.cx.type_at(base);
            self.mention(&mut l, t, &location);
        }
        if result.is_empty() {
            return (result, l.reached);
        }
        let nodes: usize = result.iter().map(|n| self.cost(&l, n, 1)).sum();
        if nodes > MAX_SCHEMA_NODES {
            self.fail(&l, root, "schema exceeds MAX_SCHEMA_NODES");
        }
        if !spec.handles {
            return (result, l.reached);
        }
        let view = replica(&result[0]);
        let store = first.and_then(node_text).unwrap_or("undefined");
        for (location, reason) in view_name_problems(store, &view) {
            self.fail_with(&l, &location, &reason, 18);
        }
        result.truncate(1);
        result.push(view);
        (result, l.reached)
    }

    fn fail(&mut self, l: &Lowering, location: &str, reason: &str) -> Schema {
        self.fail_with(l, location, reason, 19)
    }

    fn fail_with(&mut self, l: &Lowering, location: &str, reason: &str, rule: u16) -> Schema {
        if l.report {
            let message = format!("{} {location}: {reason}", l.name);
            self.report(l.call, Code::Rule(rule), &message);
        }
        Schema::Nil
    }

    fn target_of(&mut self, call: Node) -> Option<Type> {
        if let Some(argument) = call.type_arguments().get(0) {
            return Some(self.cx.type_from_type_node(argument));
        }
        let signature = self.cx.signature_at(call);
        let returned = self.cx.return_type(signature);
        if !is_reference(returned) {
            return None;
        }
        self.cx.type_arguments(returned).first().copied()
    }

    fn check_symbols(&mut self, l: &mut Lowering, symbols: [Option<Symbol>; 2], location: &str) {
        for symbol in symbols.into_iter().flatten() {
            l.reached.insert(symbol);
            let declarations: Vec<Node> =
                symbol.declarations().iter().copied().filter(|&d| !self.is_library_declaration(d)).collect();
            let files: HashSet<PathBuf> = declarations.iter().map(|&d| declaration_file(d)).collect();
            if declarations.iter().any(|&d| is_global(d)) || files.len() > 1 {
                let reason = format!(
                    "{} must be declared in one imported module; incremental builds cannot see global declarations or merges.",
                    symbol.name()
                );
                self.fail_with(l, location, &reason, 10);
            }
            if !l.inherited.insert(symbol) {
                continue;
            }
            for declaration in declarations {
                if !matches!(declaration.kind(), Kind::InterfaceDeclaration | Kind::ClassDeclaration) {
                    continue;
                }
                for clause in declaration.heritage_clauses().iter() {
                    if clause.token() != Kind::ExtendsKeyword {
                        continue;
                    }
                    for base in clause.types().iter().rev() {
                        l.bases.push((base, location.to_owned()));
                    }
                }
            }
        }
    }

    fn mention(&mut self, l: &mut Lowering, t: Type, location: &str) {
        self.check_symbols(l, [t.alias_symbol(), t.symbol()], location);
        if t.flags().intersects(TypeFlags::Intersection) {
            for part in t.types() {
                self.mention(l, part, location);
            }
        }
    }

    fn declared(&mut self, l: &mut Lowering, t: Type, location: &str) {
        self.check_symbols(l, [t.alias_symbol(), t.symbol()], location);
    }

    fn references(&mut self, l: &mut Lowering, syntax: Option<Node>, location: &str) {
        let Some(syntax) = syntax else { return };
        if !l.referenced.insert(syntax) {
            return;
        }
        match syntax.kind() {
            Kind::TypeReference | Kind::ExpressionWithTypeArguments => {
                let name = if syntax.kind() == Kind::TypeReference { syntax.type_name() } else { syntax.expression() };
                let symbol = self.cx.symbol_at(name).map(|s| self.cx.aliased(s));
                self.check_symbols(l, [symbol, None], location);
                for declaration in symbol.map_or_else(Vec::new, |s| s.declarations()) {
                    if declaration.kind() == Kind::TypeAliasDeclaration && !self.is_library_declaration(declaration) {
                        self.references(l, declaration.type_node(), location);
                    }
                }
            }
            Kind::TypeQuery => {
                let name = syntax.expr_name();
                let symbol = self.cx.symbol_at(name).map(|s| self.cx.aliased(s));
                self.check_symbols(l, [symbol, None], location);
                let queried = self.cx.type_at(name);
                let mut signatures = self.cx.signatures_of_type(queried, SignatureKind::Call);
                signatures.extend(self.cx.signatures_of_type(queried, SignatureKind::Construct));
                for signature in signatures {
                    let returned = self.cx.return_type(signature);
                    self.mention(l, returned, location);
                }
            }
            _ => {}
        }
        let mut children = Vec::new();
        syntax.for_each_child(&mut |child| children.push(child));
        for child in children {
            self.references(l, Some(child), location);
        }
    }

    fn lower(&mut self, l: &mut Lowering, t: Type, location: &str, context: Context, depth: u32) -> Schema {
        self.lower_marked(l, t, location, context, depth).0
    }

    fn lower_marked(
        &mut self,
        l: &mut Lowering,
        t: Type,
        location: &str,
        context: Context,
        depth: u32,
    ) -> (Schema, bool) {
        self.declared(l, t, location);
        let limit = l.spec.position.depth_limit();
        if depth > limit {
            return (self.fail(l, location, &format!("schema exceeds the depth limit of {limit}")), false);
        }
        let state = l.spec.position != Position::Payload;
        if state && t.flags().intersects(TypeFlags::Union) {
            let present: Vec<Type> =
                t.types().iter().copied().filter(|m| !m.flags().intersects(TypeFlags::Undefined)).collect();
            if present.len() == 1 && present.len() < t.types().len() && self.is_server_only(present[0]) {
                let (inner, marked) = self.lower_marked(l, present[0], location, context, depth);
                return if marked { (Schema::Union(vec![Schema::Nil, inner]), true) } else { (inner, false) };
            }
        }
        if state && self.is_server_only(t) {
            if !l.spec.handles {
                return (
                    self.fail(l, location, "serverOnly marks a profileStore template field; it means nothing here"),
                    false,
                );
            }
            if context.root {
                self.fail(
                    l,
                    location,
                    "serverOnly marks a template field, never the store's whole value; wrap the value in an object and mark the field",
                );
            }
            if context.marked {
                return (self.fail(l, location, "this field is already inside a serverOnly field"), false);
            }
            if !context.path {
                let reason = "serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member";
                return (self.fail(l, location, reason), false);
            }
            let inner = if is_reference(t) { self.cx.type_arguments(t).first().copied() } else { None };
            let Some(inner) = inner else { return (self.fail(l, location, "cannot encode a serverOnly mark"), false) };
            let (schema, _) = self.lower_marked(l, inner, location, Context { marked: true, ..context }, depth);
            return (schema, true);
        }
        if !l.active.insert(t) {
            return (self.fail(l, location, "recursive payload type"), false);
        }
        let schema = self.lower_type(l, t, location, context, depth);
        l.active.remove(&t);
        (schema, false)
    }

    fn lower_type(&mut self, l: &mut Lowering, t: Type, location: &str, context: Context, depth: u32) -> Schema {
        let state = l.spec.position != Position::Payload;
        let persist = l.spec.position == Position::Persist;
        let flags = t.flags();
        if flags.intersects(TypeFlags::Any) {
            return self.fail(l, location, "any cannot be checked; use unknown");
        }
        if flags.intersects(TypeFlags::Unknown) {
            return if state {
                self.fail(l, location, "unknown cannot be stored or replicated state; give the value a declared type")
            } else {
                Schema::Dynamic
            };
        }
        if flags.intersects(TypeFlags::Undefined) {
            return Schema::Nil;
        }
        if flags.intersects(TypeFlags::Void) {
            return self.fail(l, location, "use undefined for no payload");
        }
        if flags.intersects(
            TypeFlags::Never
                | TypeFlags::ESSymbolLike
                | TypeFlags::BigIntLike
                | TypeFlags::TemplateLiteral
                | TypeFlags::TypeParameter,
        ) {
            let reason = format!("cannot encode {}", self.shown(t));
            return self.fail(l, location, &reason);
        }
        if flags.intersects(TypeFlags::Boolean) {
            return Schema::Bool;
        }
        if let Some(value) = literal(t) {
            return match value {
                Literal::Number(n) if !n.is_finite() => self.fail(l, location, "nonfinite literal"),
                value => Schema::Literal(value),
            };
        }
        if flags.intersects(TypeFlags::String) {
            return Schema::String;
        }
        if flags.intersects(TypeFlags::Number) {
            return Schema::F64;
        }
        if let Some(width) = self.width(t) {
            return match width {
                Width::Brand("f64") => Schema::F64,
                Width::Brand("f32") => Schema::F32,
                Width::Brand("u8") => Schema::U8,
                Width::Brand("u16") => Schema::U16,
                Width::Brand("u32") => Schema::U32,
                _ => self.fail(l, location, "conflicting widths"),
            };
        }
        if flags.intersects(TypeFlags::Union) {
            return self.union(l, t, location, context, depth);
        }
        let flat = state && self.plain_intersection(t);
        if flags.intersects(TypeFlags::Intersection) && !flat {
            let reason = if t.types().iter().any(|p| p.flags().intersects(TypeFlags::NumberLike)) {
                "only a Crucible width brand may intersect number; this brand is foreign"
            } else {
                "only numeric width intersections are supported"
            };
            return self.fail(l, location, reason);
        }
        match self.engine(t) {
            Some(Engine::Instance(class)) => return self.instance(l, vec![class], location, context),
            Some(Engine::Unsupported) => return self.fail(l, location, "Roblox datatype not supported in v1"),
            Some(engine) if persist => {
                let runtime = engine.runtime().unwrap_or_default();
                let reason = format!("{runtime} cannot be saved: a save holds JSON; store its components");
                return self.fail(l, location, &reason);
            }
            Some(Engine::Vector3) => return Schema::Vector3,
            Some(Engine::Color3) => return Schema::Color3,
            Some(Engine::CFrame) => return Schema::CFrame,
            None => {}
        }
        let collection = self.collection(t);
        let arguments = if is_reference(t) { self.cx.type_arguments(t) } else { Vec::new() };
        match collection {
            Some(Collection::Tuple) => {
                let variable = t.target().is_some_and(|target| {
                    target
                        .element_flags()
                        .iter()
                        .any(|f| f.intersects(ElementFlags::Optional | ElementFlags::Rest | ElementFlags::Variadic))
                });
                if variable {
                    return self.fail(l, location, "tuples must be fixed length with no optional or rest elements");
                }
                let mut elements = Vec::new();
                for (i, &element) in arguments.iter().enumerate() {
                    elements.push(self.element(l, element, &format!("{location}[{i}]"), context, depth, false));
                }
                return Schema::Tuple(elements);
            }
            Some(Collection::Array) => {
                let element = arguments.first().copied().unwrap_or(t);
                if state && element.flags().intersects(TypeFlags::Never) {
                    let location = format!("{location}[]");
                    return self.fail(l, &location, "an empty array has no element type; annotate it: [] as string[]");
                }
                let element = self.element(l, element, &format!("{location}[]"), context, depth, false);
                return Schema::Array(Box::new(element));
            }
            Some(Collection::Map) => {
                let key = self.key(l, arguments.first().copied().unwrap_or(t), &format!("{location}.key"));
                let value = arguments.get(1).copied().unwrap_or(t);
                let value = self.element(l, value, &format!("{location}.value"), context, depth, true);
                return Schema::Map(key, Box::new(value));
            }
            None => {}
        }
        if flags.intersects(TypeFlags::Object) || flat {
            let indices = self.cx.index_infos_of_type(t);
            if let [index] = indices[..]
                && index.key.flags().intersects(TypeFlags::String)
                && self.cx.properties_of_type(t).is_empty()
            {
                let value = index.value;
                let value = self.element(l, value, &format!("{location}.value"), context, depth, true);
                return Schema::Map(MapKey::String, Box::new(value));
            }
            if !indices.is_empty() {
                return self.fail(l, location, "only a string index signature can encode a record");
            }
            return self.fields(l, t, location, context, depth, None);
        }
        let reason = format!("cannot encode {}", self.shown(t));
        self.fail(l, location, &reason)
    }

    /// State drops `undefined` from a map value, which a missing key means.
    fn element(
        &mut self,
        l: &mut Lowering,
        t: Type,
        location: &str,
        context: Context,
        depth: u32,
        value: bool,
    ) -> Schema {
        let state = l.spec.position != Position::Payload;
        let t = if state && value {
            self.cx.non_nullable_type(t)
        } else {
            if absent(t) {
                self.fail(l, location, "collection elements and map values cannot admit undefined");
            }
            t
        };
        let context = Context { to_client: context.to_client, ..Context::default() };
        self.lower(l, t, location, context, depth + 1)
    }

    fn instance(
        &mut self,
        l: &mut Lowering,
        mut classes: Vec<&'static str>,
        location: &str,
        context: Context,
    ) -> Schema {
        match l.spec.position {
            Position::Persist => {
                return self.fail(l, location, "Instances cannot be saved or replicated; store a stable id");
            }
            Position::State => {
                return self.fail(l, location, "replicated state cannot hold Instances; replicate a stable id");
            }
            Position::Payload => {}
        }
        if context.to_client && !context.nilable && classes != ["Player"] {
            self.fail_with(
                l,
                location,
                "a client may not have streamed this Instance in; admit undefined or wrap collection elements in objects with an optional field.",
                20,
            );
        }
        classes.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Schema::Instance(classes.into_iter().map(Into::into).collect())
    }

    fn fields(
        &mut self,
        l: &mut Lowering,
        t: Type,
        location: &str,
        context: Context,
        depth: u32,
        omit: Option<&str>,
    ) -> Schema {
        if self.signatures_of(t) {
            return self.fail(l, location, "functions cannot cross the network");
        }
        let mut properties: Vec<Symbol> =
            self.cx.properties_of_type(t).into_iter().filter(|p| Some(p.name()) != omit).collect();
        properties.sort_by(|a, b| a.name().as_bytes().cmp(b.name().as_bytes()));
        if properties.is_empty() && omit.is_none() {
            let reason = if l.spec.handles {
                "an empty object type accepts anything; annotate an empty template table: {} as Record<string, number>"
            } else {
                "an empty object type accepts anything; use undefined"
            };
            return self.fail(l, location, reason);
        }
        let mut fields = Vec::new();
        for property in properties {
            let name = property.name();
            let at = format!("{location}.{name}");
            let declaration = property.value_declaration().or_else(|| property.declarations().first().copied());
            for d in property.declarations() {
                if let Some(parent) = d.parent().filter(|p| p.kind() == Kind::InterfaceDeclaration)
                    && let Some(symbol) = parent.name().and_then(|n| self.cx.symbol_at(n))
                {
                    l.reached.insert(symbol);
                }
            }
            self.references(l, declaration.and_then(|d| d.type_node()), &at);
            if is_symbol_name(name) {
                self.fail(l, location, "symbol property names cannot cross the network");
            }
            if is_numeric_name(name) {
                self.fail(l, &at, "numeric property names are not supported; use a Map");
            }
            if property
                .declarations()
                .iter()
                .any(|d| matches!(d.kind(), Kind::MethodSignature | Kind::MethodDeclaration))
            {
                self.fail(l, &at, "functions cannot cross the network");
            }
            if l.spec.handles && RESERVED_HANDLE_NAMES.contains(&name) {
                let reason =
                    format!("`{name}` is a handle method name (set, update, insert, remove); rename the field");
                self.fail_with(l, &at, &reason, 26);
            }
            let value = self.cx.non_missing_type_of_symbol(property);
            if value.flags().intersects(TypeFlags::Undefined) {
                let reason = if l.spec.handles {
                    "a position that is always undefined; annotate it: undefined as string | undefined"
                } else {
                    "a field of type undefined carries no information; remove it"
                };
                let value = self.fail(l, &at, reason);
                fields.push(Field { name: name.into(), value, server_only: false });
                continue;
            }
            let child = Context {
                to_client: context.to_client,
                nilable: property.flags().intersects(SymbolFlags::Optional) || absent(value),
                path: context.path,
                marked: context.marked,
                root: false,
            };
            let (value, server_only) = self.lower_marked(l, value, &at, child, depth + 1);
            fields.push(Field { name: name.into(), value, server_only });
        }
        Schema::Object(fields)
    }

    fn key(&mut self, l: &mut Lowering, t: Type, location: &str) -> MapKey {
        self.declared(l, t, location);
        if t.flags().intersects(TypeFlags::String) {
            return MapKey::String;
        }
        let parts = members(t);
        if l.spec.position != Position::Payload && !parts.iter().all(|&p| p.literal_string().is_some()) {
            self.fail(
                l,
                location,
                "state maps are keyed by string or a union of string literals; frames carry string-keyed records",
            );
            return MapKey::String;
        }
        match self.width(t) {
            Some(Width::Brand("u8")) => return MapKey::U8,
            Some(Width::Brand("u16")) => return MapKey::U16,
            Some(Width::Brand("u32")) => return MapKey::U32,
            _ => {}
        }
        if parts.len() > MAX_UNION_BRANCHES {
            self.fail(l, location, "too many union branches");
            return MapKey::String;
        }
        let mut keys = Vec::new();
        for &part in &parts {
            match literal(part) {
                Some(Literal::String(s)) => keys.push(Literal::String(s)),
                Some(Literal::Number(n)) if n.is_finite() => keys.push(Literal::Number(n)),
                _ => {
                    self.fail(
                        l,
                        location,
                        "map keys must be string, u8, u16, u32 or a literal union; float keys can round to a duplicate",
                    );
                    return MapKey::String;
                }
            }
        }
        keys.sort_by(compare);
        MapKey::Literals(
            keys.into_iter()
                .map(|k| match k {
                    Literal::Number(n) => KeyLiteral::Number(n),
                    Literal::String(s) => KeyLiteral::String(s),
                    Literal::Boolean(_) => unreachable!("keys hold only strings and numbers"),
                })
                .collect(),
        )
    }

    fn union(&mut self, l: &mut Lowering, t: Type, location: &str, context: Context, depth: u32) -> Schema {
        let state = l.spec.position != Position::Payload;
        let parts = t.types();
        let missing = parts.iter().any(|p| p.flags().intersects(TypeFlags::Undefined));
        let context = Context { nilable: context.nilable || missing, path: false, ..context };
        let mut branches = if missing { vec![Schema::Nil] } else { Vec::new() };
        let mut literals = Vec::new();
        let mut typed: Vec<(Option<&'static str>, Schema)> = Vec::new();
        let mut objects = Vec::new();
        let mut classes: Vec<&'static str> = Vec::new();
        let boolean = parts.iter().filter(|p| p.flags().intersects(TypeFlags::BooleanLiteral)).count() == 2;
        if boolean {
            typed.push((Some("boolean"), Schema::Bool));
        }
        for &part in &parts {
            self.declared(l, part, location);
            if part.flags().intersects(TypeFlags::Undefined)
                || (boolean && part.flags().intersects(TypeFlags::BooleanLiteral))
            {
                continue;
            }
            if let Some(value) = literal(part) {
                literals.push(value);
                continue;
            }
            let engine = self.engine(part);
            if let Some(Engine::Instance(class)) = engine {
                classes.push(class);
                continue;
            }
            let object = (part.flags().intersects(TypeFlags::Object) || (state && self.plain_intersection(part)))
                && engine.is_none()
                && self.collection(part).is_none()
                && self.cx.index_infos_of_type(part).is_empty()
                && !self.signatures_of(part);
            if object {
                objects.push(part);
                continue;
            }
            let schema = self.lower(l, part, location, context, depth + 1);
            let runtime = engine.as_ref().and_then(Engine::runtime).or(match schema {
                Schema::Bool => Some("boolean"),
                Schema::F64 | Schema::F32 | Schema::U8 | Schema::U16 | Schema::U32 => Some("number"),
                Schema::String => Some("string"),
                Schema::Array(_) | Schema::Tuple(_) | Schema::Map(..) | Schema::Object(_) => Some("table"),
                Schema::Dynamic => Some("dynamic"),
                _ => None,
            });
            typed.push((runtime, schema));
        }
        if !classes.is_empty() {
            let mut unique = Vec::new();
            for class in classes {
                if !unique.contains(&class) {
                    unique.push(class);
                }
            }
            let schema = self.instance(l, unique, location, context);
            typed.push((Some("Instance"), schema));
        }
        let mut discriminant = "";
        let mut variants: Vec<(Literal, Schema)> = Vec::new();
        if objects.len() > 1 {
            let mut candidates: Vec<&'static str> =
                self.cx.properties_of_type(objects[0]).iter().map(|p| p.name()).collect();
            candidates.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            let mut found = None;
            for name in candidates {
                let mut seen = HashSet::new();
                let mut groups = Vec::new();
                for &object in &objects {
                    groups.push(self.discriminant_values(l, object, name, location, &mut seen));
                }
                if groups.iter().all(Option::is_some) {
                    found = Some((name, groups.into_iter().flatten().collect::<Vec<_>>()));
                    break;
                }
            }
            let Some((key, values)) = found else {
                let shown: Vec<String> = objects.iter().map(|&o| self.shown(o)).collect();
                let reason = format!(
                    "union members {} have no discriminant; give each a distinct literal property",
                    shown.join(", ")
                );
                return self.fail(l, location, &reason);
            };
            if is_numeric_name(key) || is_symbol_name(key) {
                return self.fail(l, location, "numeric or symbol property names are not supported; use a Map");
            }
            discriminant = key;
            for (object, values) in objects.iter().zip(values) {
                for value in values {
                    let body = self.fields(l, *object, location, context, depth + 2, Some(key));
                    variants.push((value, body));
                }
            }
        } else if let Some(&object) = objects.first() {
            let schema = self.lower(l, object, location, context, depth + 1);
            typed.push((Some("table"), schema));
        }
        let runtimes: HashSet<Option<&str>> = typed.iter().map(|(runtime, _)| *runtime).collect();
        if runtimes.len() != typed.len()
            || (!variants.is_empty() && typed.iter().any(|(runtime, _)| *runtime == Some("table")))
        {
            return self.fail(l, location, "union has ambiguous runtime kinds");
        }
        literals.sort_by(compare);
        branches.extend(literals.into_iter().map(Schema::Literal));
        typed.sort_by(|a, b| a.0.unwrap_or("").as_bytes().cmp(b.0.unwrap_or("").as_bytes()));
        branches.extend(typed.into_iter().map(|(_, schema)| schema));
        variants.sort_by(|a, b| compare(&a.0, &b.0));
        branches.extend(variants.into_iter().map(|(value, body)| Schema::Variant {
            key: discriminant.into(),
            value,
            body: Box::new(body),
        }));
        if branches.len() > MAX_UNION_BRANCHES {
            return self.fail(l, location, "too many union branches");
        }
        Schema::Union(branches)
    }

    fn discriminant_values(
        &mut self,
        l: &mut Lowering,
        object: Type,
        name: &str,
        location: &str,
        seen: &mut HashSet<(u8, String)>,
    ) -> Option<Vec<Literal>> {
        let property = self.cx.property_of_type(object, name)?;
        if property.flags().intersects(SymbolFlags::Optional) {
            return None;
        }
        let declaration = property.value_declaration().or_else(|| property.declarations().first().copied());
        self.references(l, declaration.and_then(|d| d.type_node()), &format!("{location}.{name}"));
        let t = self.cx.non_missing_type_of_symbol(property);
        let values: Vec<Option<Literal>> = members(t).into_iter().map(literal).collect();
        if values.iter().any(|v| v.as_ref().is_none_or(|v| seen.contains(&literal_key(v)))) {
            return None;
        }
        let values: Vec<Literal> = values.into_iter().flatten().collect();
        seen.extend(values.iter().map(literal_key));
        Some(values)
    }

    fn cost(&mut self, l: &Lowering, node: &Schema, depth: u32) -> usize {
        let limit = l.spec.position.depth_limit();
        if depth > limit {
            self.fail(l, "value", &format!("schema exceeds the depth limit of {limit}"));
        }
        let children: Vec<&Schema> = match node {
            Schema::Object(fields) => fields.iter().map(|f| &f.value).collect(),
            Schema::Union(nodes) | Schema::Tuple(nodes) => nodes.iter().collect(),
            Schema::Array(element) => vec![element],
            Schema::Map(key, value) => {
                let key = match key {
                    MapKey::Literals(keys) => {
                        if depth + 2 > limit && !keys.is_empty() {
                            self.fail(l, "value", &format!("schema exceeds the depth limit of {limit}"));
                        }
                        1 + keys.len()
                    }
                    _ => 1,
                };
                if depth + 1 > limit {
                    self.fail(l, "value", &format!("schema exceeds the depth limit of {limit}"));
                }
                return 1 + key + self.cost(l, value, depth + 1);
            }
            Schema::Variant { body, .. } => vec![body],
            _ => Vec::new(),
        };
        1 + children.into_iter().map(|child| self.cost(l, child, depth + 1)).sum::<usize>()
    }

    fn is_server_only(&mut self, t: Type) -> bool {
        self.api(t.symbol()) == Some("ServerOnly")
    }

    fn plain_intersection(&mut self, t: Type) -> bool {
        t.flags().intersects(TypeFlags::Intersection)
            && t.types()
                .to_vec()
                .into_iter()
                .all(|part| part.flags().intersects(TypeFlags::Object) && !self.signatures_of(part))
    }

    fn engine(&mut self, t: Type) -> Option<Engine> {
        let symbol = t.symbol()?;
        if !self.standard(symbol, Standard::Types) {
            return None;
        }
        let name = symbol.name();
        Some(match name {
            "Vector3" => Engine::Vector3,
            "Color3" => Engine::Color3,
            "CFrame" => Engine::CFrame,
            _ => {
                let at = symbol.declarations().first().copied();
                let instances = self.cx.resolve_name("Instances", at, SymbolFlags::Type);
                let known = instances.is_some_and(|i| {
                    let declared = self.cx.declared_type_of_symbol(i);
                    self.cx.property_of_type(declared, name).is_some()
                });
                if known { Engine::Instance(name) } else { Engine::Unsupported }
            }
        })
    }

    fn collection(&mut self, t: Type) -> Option<Collection> {
        if t.is_tuple_type() {
            return Some(Collection::Tuple);
        }
        if self.cx.is_array_type(t) {
            return Some(Collection::Array);
        }
        let symbol = t.symbol()?;
        (matches!(symbol.name(), "Map" | "ReadonlyMap") && self.standard(symbol, Standard::CompilerTypes))
            .then_some(Collection::Map)
    }

    fn width(&mut self, t: Type) -> Option<Width> {
        if !t.flags().intersects(TypeFlags::Intersection)
            || !t.types().iter().any(|p| p.flags().intersects(TypeFlags::Number))
        {
            return None;
        }
        let mut found: Vec<&'static str> = Vec::new();
        for part in t.types() {
            if part.flags().intersects(TypeFlags::Number) {
                continue;
            }
            let [property] = self.cx.properties_of_type(part)[..] else { return None };
            let name = property.declarations().first().and_then(|d| d.name())?;
            if name.kind() != Kind::ComputedPropertyName {
                return None;
            }
            let expression = name.expression();
            let symbol = self.cx.symbol_at(expression).map(|s| self.cx.aliased(s));
            if !symbol.is_some_and(|s| self.api.width_keys.contains(&s)) {
                return None;
            }
            let value = self.cx.non_missing_type_of_symbol(property);
            for member in members(value) {
                if let Some(width) = member.literal_string()
                    && !found.contains(&width)
                {
                    found.push(width);
                }
            }
        }
        Some(if let [width] = found[..] { Width::Brand(width) } else { Width::Conflicting })
    }

    pub(crate) fn receive(&mut self, call: Node, file: File) {
        let field = call.parent().filter(|p| {
            let flags = p.combined_modifier_flags();
            p.kind() == Kind::PropertyDeclaration
                && p.initializer() == Some(call)
                && flags.intersects(ModifierFlags::Readonly)
                && !flags.intersects(ModifierFlags::Static)
        });
        let instance = field.and_then(|f| f.parent()).and_then(|class| self.instance_type(class));
        let subsystem = instance.is_some_and(|t| self.kind_of(t) == Some(ClassKind::Subsystem));
        let (Some(field), Some(instance), true) = (field, instance, subsystem) else {
            self.report(call, Code::Rule(21), "receive() must initialize a readonly instance field of a Subsystem.");
            return;
        };
        let name = field.name().and_then(|n| self.cx.symbol_at(n)).map(|s| s.name());
        if let Some(name) = name {
            let mut seen = HashSet::new();
            let shadows = self.class_bases(instance).into_iter().any(|base| self.shadows(base, name, &mut seen));
            if shadows {
                self.report(call, Code::Rule(21), "A receiver field cannot shadow an inherited receiver.");
            }
        }
        let target = call.arguments().get(0).map(|a| self.cx.type_at(a));
        let token = target
            .filter(|t| !t.flags().intersects(TypeFlags::UnionOrIntersection))
            .and_then(|t| self.api(t.symbol()))
            .filter(|token| matches!(*token, "ToServer" | "ToClient" | "AskServer" | "ClientReady"));
        match token {
            None => {
                self.report(call, Code::Rule(21), "receive target must be exactly one message token or ClientReady.")
            }
            Some(token) => {
                let required = if token == "ToClient" { Side::Client } else { Side::Server };
                if self.file_side(file) != Some(required) {
                    let message = format!("{token} must be received by a {} Subsystem.", required.folder());
                    self.report(call, Code::Rule(22), &message);
                }
                if let Some(callback) = self.callback(call.arguments().get(1)) {
                    self.callback_returns(call, callback, token);
                }
            }
        }
        if let Some(name) = name {
            self.store(call, Fact::Receive { field: name.into() });
        }
    }

    fn callback_returns(&mut self, call: Node, callback: Node, token: &str) {
        let signature = self.cx.signature_from_declaration(callback);
        let returned = self.cx.return_type(signature);
        let parts = members(returned);
        let asynchronous = callback.combined_modifier_flags().intersects(ModifierFlags::Async);
        let promise = parts
            .iter()
            .any(|&t| t.symbol().is_some_and(|s| s.name() == "Promise" && self.standard(s, Standard::CompilerTypes)));
        if asynchronous || promise {
            self.report(
                call,
                Code::Rule(12),
                "Network callbacks cannot be async or return a Promise; spawn a thread inside the callback.",
            );
            return;
        }
        if token == "AskServer" {
            return;
        }
        let returns = parts.iter().any(|&t| {
            !t.flags().intersects(TypeFlags::Void | TypeFlags::Undefined | TypeFlags::Never)
                && !(token == "ClientReady" && !self.cx.signatures_of_type(t, SignatureKind::Call).is_empty())
        });
        if returns {
            let allowed = if token == "ClientReady" { " or a cleanup function" } else { "; use a block body" };
            let message =
                format!("{token} callbacks return nothing{allowed}; this one can return {}.", self.shown(returned));
            self.report(call, Code::Rule(12), &message);
        }
    }

    fn class_bases(&mut self, t: Type) -> Vec<Type> {
        if t.object_flags().intersects(ObjectFlags::ClassOrInterface) { self.cx.base_types(t) } else { Vec::new() }
    }

    fn shadows(&mut self, t: Type, name: &str, seen: &mut HashSet<Type>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        let property = self.cx.property_of_type(t, name);
        let receiver = property.is_some_and(|p| {
            p.declarations().iter().any(|&d| {
                d.kind() == Kind::PropertyDeclaration
                    && d.initializer().is_some_and(|i| self.is_macro_call(i, Macro::Receive))
            })
        });
        receiver || self.class_bases(t).into_iter().any(|base| self.shadows(base, name, seen))
    }

    fn callback(&mut self, argument: Option<Node>) -> Option<Node> {
        let argument = argument?;
        match argument.kind() {
            Kind::FunctionExpression | Kind::ArrowFunction => Some(argument),
            Kind::Identifier | Kind::PropertyAccessExpression => {
                let name = if argument.kind() == Kind::PropertyAccessExpression { argument.name()? } else { argument };
                let symbol = self.cx.symbol_at(name).map(|s| self.cx.aliased(s))?;
                let [declaration] = symbol.declarations()[..] else { return None };
                match declaration.kind() {
                    Kind::FunctionDeclaration | Kind::MethodDeclaration => Some(declaration),
                    Kind::VariableDeclaration if declaration.combined_node_flags().intersects(NodeFlags::Const) => {
                        declaration
                            .initializer()
                            .filter(|i| matches!(i.kind(), Kind::ArrowFunction | Kind::FunctionExpression))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(crate) fn side_entry_specifier(&mut self, literal: Node, file: File) {
        if self.api.side_entries.is_empty() {
            return;
        }
        let Some(parent) = literal.parent() else { return };
        let specifier = match parent.kind() {
            Kind::ImportDeclaration | Kind::ExportDeclaration => true,
            Kind::LiteralType => parent.parent().is_some_and(|p| p.kind() == Kind::ImportType),
            Kind::CallExpression => parent.expression().kind() == Kind::ImportKeyword,
            _ => false,
        };
        if !specifier {
            return;
        }
        let module = self.cx.resolve_external_module(literal);
        let Some(path) = module.and_then(|m| m.declarations().first().copied()).map(declaration_file) else {
            return;
        };
        self.wrong_side(literal, &path, file);
    }

    fn wrong_side(&mut self, at: Node, path: &Path, file: File) -> bool {
        let Some(&side) = self.api.side_entries.get(path) else { return false };
        if self.file_side(file) != Some(side) {
            let folder = side.folder();
            let message =
                format!("This file cannot reference @trioforge/crucible/{folder}; move it under a {folder} folder.");
            self.report(at, Code::Rule(24), &message);
        }
        true
    }

    pub(crate) fn side_entry_reference(&mut self, n: Node, file: File) {
        if self.api.side_entries.is_empty() {
            return;
        }
        let Some(symbol) = self.cx.symbol_at(n).map(|s| self.cx.aliased(s)) else {
            return;
        };
        let mut in_entry = false;
        for declaration in symbol.declarations() {
            let path = declaration_file(declaration);
            in_entry |= self.wrong_side(n, &path, file);
        }
        if in_entry {
            return;
        }
        let owner = match self.api.entry_exports.get(&symbol) {
            Some(&side) => Some(side),
            None if symbol.flags().intersects(SymbolFlags::Variable | SymbolFlags::Property) => {
                let t = self.cx.type_at(n);
                let alias = t.alias_symbol().map(|s| self.cx.aliased(s));
                let by_alias = alias.and_then(|a| self.api.entry_exports.get(&a).copied());
                by_alias.or_else(|| t.symbol().and_then(|s| self.api.entry_exports.get(&s).copied()))
            }
            None => None,
        };
        if let Some(owner) = owner
            && self.file_side(file) != Some(owner)
        {
            let folder = owner.folder();
            let message = format!(
                "This file cannot reference @trioforge/crucible/{folder} through {}; move it under a {folder} folder.",
                n.text()
            );
            self.report(n, Code::Rule(24), &message);
        }
    }

    fn reached(&mut self) -> &HashSet<Symbol> {
        if self.state.network.reached.is_none() {
            let declarations: Vec<(Node, DeclarationKind)> =
                self.declarations().iter().map(|d| (d.call, d.kind)).collect();
            let mut reached = HashSet::new();
            for (call, kind) in declarations {
                reached.extend(self.lower_declaration(call, kind, false).1);
            }
            self.state.network.reached = Some(reached);
        }
        self.state.network.reached.get_or_insert_default()
    }

    pub(crate) fn augmentation(&mut self, n: Node) {
        let named = n.name().is_some_and(|name| name.kind() == Kind::StringLiteral);
        if !named && !n.is_global_scope_augmentation() {
            return;
        }
        let mut added = Vec::new();
        added_declarations(n.body(), &mut added);
        let symbols: Vec<Symbol> =
            added.into_iter().filter_map(|d| d.name()).filter_map(|name| self.cx.symbol_at(name)).collect();
        if symbols.is_empty() {
            return;
        }
        if symbols.iter().any(|s| self.reached().contains(s)) {
            self.report(
                n,
                Code::Rule(10),
                "This augmentation changes a type a Crucible declaration lowers; incremental builds cannot see the merge. Declare the type in one module.",
            );
        }
    }
}
