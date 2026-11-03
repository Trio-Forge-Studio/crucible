//! Crucible's declaration macros and `receive`: the rules each call obeys, the schema a declared type lowers to, and the
//! program-wide CRU010 augmentation rule. The diagnostics equal the ones
//! `packages/crucible/transformer-src/network.cjs` reports.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{ElementFlags, SignatureKind, Type, TypeFlags};
use tsrs_core::{P, jsnum};

use super::Extractor;
use super::crucible::{Code, is_global, is_reference, unalias};
use super::types;
use crate::declarations::{
    DeclarationKind, MAX_ASK_TIMEOUT_SECONDS, MAX_SCHEMA_NODES, MAX_UNION_BRANCHES, Position, RESERVED_HANDLE_NAMES,
    Side, Spec, TypeArguments, VIEW_NAME_MAX_BYTES, VIEW_NAME_PREFIX,
};
use crate::facts::{ClassKind, Crucible, Declaration, Field, KeyLiteral, Literal, MapKey, Schema, Site, Tail};
use crate::macros::Macro;
use crate::project::normalize;
use crate::tree::File;

/// What the rules learn about the whole program, computed when a rule first asks.
#[derive(Default)]
pub(super) struct Program {
    declarations: Option<Vec<Indexed>>,
    /// Every symbol a declaration's lowering reaches.
    reached: Option<HashSet<P<Symbol>>>,
}

struct Indexed {
    call: P<Node>,
    kind: DeclarationKind,
    name: Option<&'static str>,
}

/// Where in a declared value a type sits.
#[derive(Clone, Copy, Default)]
struct Context {
    /// The server sends it, and a client may not have streamed in an Instance it names.
    to_client: bool,
    /// The value may be absent here.
    nilable: bool,
    /// On an object path from the value's root, where a `serverOnly` mark may sit.
    path: bool,
    /// Inside a `serverOnly` field.
    marked: bool,
    /// The store's whole value.
    root: bool,
}

/// One declaration's lowering.
struct Lowering {
    call: P<Node>,
    spec: Spec,
    /// The declaration's name, which starts each diagnostic about its value.
    name: &'static str,
    /// False while the augmentation rule lowers every declaration to learn what they reach.
    report: bool,
    reached: HashSet<P<Symbol>>,
    active: HashSet<P<Type>>,
    inherited: HashSet<P<Symbol>>,
    referenced: HashSet<P<Node>>,
    /// The `extends` clauses of reached classes and interfaces, walked after the value so that a deep heritage chain
    /// does not nest calls.
    bases: Vec<(P<Node>, String)>,
}

enum Engine {
    Vector3,
    Color3,
    CFrame,
    Instance(&'static str),
    /// A Roblox datatype the codec has no encoding for.
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
enum Standard {
    /// `@rbxts/types`, the Roblox API.
    Types,
    /// The compiler types, which declare `Map` and `Promise`.
    CompilerTypes,
}

fn node_text(n: P<Node>) -> Option<&'static str> {
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

/// A name that TypeScript treats as a number, because it is the text of the number it parses as.
fn is_numeric_name(name: &str) -> bool {
    jsnum::from_string(name).to_string() == name
}

/// A unique symbol's property name.
fn is_symbol_name(name: &str) -> bool {
    name.starts_with("\u{7f}@")
}

fn literal(t: P<Type>) -> Option<Literal> {
    if let Some(s) = types::literal_string(t) {
        return Some(Literal::String(s.into()));
    }
    if let Some(n) = types::literal_number(t) {
        return Some(Literal::Number(n));
    }
    types::literal_bool(t).map(Literal::Boolean)
}

/// Booleans, then numbers, then strings by their bytes.
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

fn members(t: P<Type>) -> Vec<P<Type>> {
    if t.flags().intersects(TypeFlags::Union) { t.types().to_vec() } else { vec![t] }
}

fn absent(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Undefined)
        || (t.flags().intersects(TypeFlags::Union) && t.types().iter().any(|&m| absent(m)))
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// The value the owner's client receives: the saved value without its `serverOnly` fields, and without any object those
/// removals empty.
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

/// The `serverOnly` fields' paths. Fields are sorted, so a parent's path comes before its children's.
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

/// Each replica field whose view declaration name breaks the naming rule, as its location and the reason.
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
                    VIEW_NAME_MAX_BYTES - prefix.len()
                ),
            ));
        }
    }
    problems
}

/// Every call in `n`.
fn calls(n: P<Node>, out: &mut Vec<P<Node>>) {
    if n.kind() == Kind::CallExpression {
        out.push(n);
    }
    n.for_each_child(&mut |child| {
        calls(child, out);
        false
    });
}

/// The declarations an augmentation adds, through variable lists and nested namespaces.
fn added_declarations(body: Option<P<Node>>, out: &mut Vec<P<Node>>) {
    let Some(body) = body else { return };
    let statements = if body.kind() == Kind::ModuleBlock { body.statements().to_vec() } else { vec![body] };
    for statement in statements {
        if statement.kind() == Kind::VariableStatement {
            let list = statement.as_variable_statement().declaration_list;
            out.extend(list.as_variable_declaration_list().declarations.nodes().iter().copied());
        } else {
            out.push(statement);
        }
        if statement.kind() == Kind::ModuleDeclaration {
            added_declarations(statement.body(), out);
        }
    }
}

fn declaration_file(declaration: P<Node>) -> Option<PathBuf> {
    tsrs_ast::get_source_file_of_node(declaration).map(|f| normalize(Path::new(f.file_name())))
}

/// An ask's timeout, when its options are exactly `{ timeout: <number literal in (0, 120]> }`.
fn ask_timeout(options: P<Node>) -> Option<f64> {
    if options.kind() != Kind::ObjectLiteralExpression {
        return None;
    }
    let [property] = options.properties() else { return None };
    if property.kind() != Kind::PropertyAssignment {
        return None;
    }
    let name = property.name()?;
    if !matches!(name.kind(), Kind::Identifier | Kind::StringLiteral) || name.text() != "timeout" {
        return None;
    }
    let value = property.initializer().filter(|i| i.kind() == Kind::NumericLiteral)?;
    let seconds = jsnum::from_string(value.text()).0;
    (seconds > 0.0 && seconds <= MAX_ASK_TIMEOUT_SECONDS).then_some(seconds)
}

impl Extractor {
    pub(super) fn file_side(&mut self, file: P<SourceFile>) -> Option<Side> {
        let path = Path::new(file.file_name());
        self.crucible.side(path, path)
    }

    /// Whether `symbol` is declared in a file of one of the standard Roblox packages.
    fn standard(&mut self, symbol: P<Symbol>, which: Standard) -> bool {
        symbol.declarations().iter().any(|&d| {
            let Some(file) = tsrs_ast::get_source_file_of_node(d) else { return false };
            let name = file.file_name().replace('\\', "/");
            let package = self.crucible.package_name(Path::new(&name));
            match which {
                Standard::Types => name.contains("/@rbxts/types/") || package.as_deref() == Some("@rbxts/types"),
                Standard::CompilerTypes => {
                    name.contains("/@rbxts/compiler-types/")
                        || matches!(package.as_deref(), Some("@rbxts/compiler-types" | "@trioforge/compiler-types"))
                }
            }
        })
    }

    fn signatures_of(&mut self, t: P<Type>) -> bool {
        !self.checker.get_signatures_of_type_exported(t, SignatureKind::Call).is_empty()
            || !self.checker.get_signatures_of_type_exported(t, SignatureKind::Construct).is_empty()
    }

    fn shown(&mut self, t: P<Type>) -> String {
        self.checker.type_to_string_exported(t)
    }

    /// Every declaration macro call in the program, in program order.
    fn declarations(&mut self) -> &[Indexed] {
        if self.network.declarations.is_none() {
            let mut found = Vec::new();
            for &file in self.crucible.program.source_files() {
                if file.is_declaration_file() || file.is_js() {
                    continue;
                }
                let mut all = Vec::new();
                calls(file.as_node(), &mut all);
                for call in all {
                    let kind = self.crucible.macro_at(&mut self.checker, call).and_then(DeclarationKind::of);
                    if let Some(kind) = kind {
                        let name =
                            call.arguments().first().filter(|a| a.kind() == Kind::StringLiteral).map(|a| a.text());
                        found.push(Indexed { call, kind, name });
                    }
                }
            }
            self.network.declarations = Some(found);
        }
        self.network.declarations.as_deref().unwrap_or_default()
    }

    /// `file:line` of a node, relative to the tsconfig's folder.
    fn location(&self, n: P<Node>) -> String {
        let Some(file) = tsrs_ast::get_source_file_of_node(n) else { return String::new() };
        let line = File::new(file).position(crate::tree::span(n, file.text()).start).line;
        let path = file.file_name().replace('\\', "/");
        let dir = format!("{}/", self.config_dir.to_string_lossy().replace('\\', "/").trim_end_matches('/'));
        format!("{}:{line}", path.strip_prefix(&dir).unwrap_or(&path))
    }

    /// Whether the declaration is an exported top-level const of a module.
    fn placed(call: P<Node>, file: P<SourceFile>) -> bool {
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
            s.parent() == Some(file.as_node())
                && tsrs_ast::get_combined_modifier_flags(s).intersects(ModifierFlags::Export)
                && tsrs_ast::is_external_module(file)
        })
    }

    /// CRU017 to CRU020 and CRU026 for one declaration macro call, and the fact its call lowers from.
    pub(super) fn declaration(&mut self, call: P<Node>, kind: DeclarationKind, file: P<SourceFile>) {
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
            .first()
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
            let timeout = arguments.get(1).and_then(|&options| {
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
        self.site();
        let span = self.span(call);
        let fact = Box::new(Crucible::Declaration(Declaration { kind, schemas, tail }));
        self.table.store(span, Site::Crucible, |f| &mut f.crucible, fact);
    }

    /// The schemas a declaration's call emits, and every symbol its value type reaches.
    fn lower_declaration(
        &mut self,
        call: P<Node>,
        kind: DeclarationKind,
        report: bool,
    ) -> (Vec<Schema>, HashSet<P<Symbol>>) {
        let spec = kind.spec();
        let first = call.arguments().first().copied();
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
        for &argument in call.type_arguments() {
            self.references(&mut l, Some(argument), root);
        }
        let mut result = Vec::new();
        if !state {
            for (i, &argument) in call.type_arguments().iter().enumerate() {
                let t = self.checker.get_type_from_type_node_exported(argument);
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
            let t = self.checker.get_type_at_location(base);
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

    /// A state's value type: its type argument, or else the type argument of what the call returns.
    fn target_of(&mut self, call: P<Node>) -> Option<P<Type>> {
        if let Some(&argument) = call.type_arguments().first() {
            return Some(self.checker.get_type_from_type_node_exported(argument));
        }
        let signature = self.checker.get_resolved_signature_exported(call);
        let returned = self.checker.get_return_type_of_signature_exported(signature);
        if !is_reference(returned) {
            return None;
        }
        self.checker.get_type_arguments_exported(returned).first().copied()
    }

    /// CRU010 for each symbol declared in a global script, a `declare global` block or more than one file, and the
    /// symbols' heritage, which a merge can change too.
    fn check_symbols(&mut self, l: &mut Lowering, symbols: [Option<P<Symbol>>; 2], location: &str) {
        for symbol in symbols.into_iter().flatten() {
            l.reached.insert(symbol);
            let declarations: Vec<P<Node>> =
                symbol.declarations().iter().copied().filter(|&d| !self.is_library_declaration(d)).collect();
            let files: HashSet<PathBuf> = declarations.iter().filter_map(|&d| declaration_file(d)).collect();
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
                let clauses = match declaration.kind() {
                    Kind::InterfaceDeclaration => declaration.as_interface_declaration().heritage_clauses(),
                    Kind::ClassDeclaration => declaration.as_class_declaration().heritage_clauses(),
                    _ => None,
                };
                for &clause in clauses.map_or(&[][..], |c| c.nodes()) {
                    let clause = clause.as_heritage_clause();
                    if clause.token != Kind::ExtendsKeyword {
                        continue;
                    }
                    for &base in clause.types.get().nodes().iter().rev() {
                        l.bases.push((base, location.to_owned()));
                    }
                }
            }
        }
    }

    fn mention(&mut self, l: &mut Lowering, t: P<Type>, location: &str) {
        self.check_symbols(l, [t.alias().and_then(|a| a.symbol()), t.symbol()], location);
        if t.flags().intersects(TypeFlags::Intersection) {
            for &part in t.types() {
                self.mention(l, part, location);
            }
        }
    }

    fn declared(&mut self, l: &mut Lowering, t: P<Type>, location: &str) {
        self.check_symbols(l, [t.alias().and_then(|a| a.symbol()), t.symbol()], location);
    }

    /// The symbols a type's syntax names, through aliases, `typeof` queries and what queried functions return.
    fn references(&mut self, l: &mut Lowering, syntax: Option<P<Node>>, location: &str) {
        let Some(syntax) = syntax else { return };
        if !l.referenced.insert(syntax) {
            return;
        }
        match syntax.kind() {
            Kind::TypeReference | Kind::ExpressionWithTypeArguments => {
                let name = if syntax.kind() == Kind::TypeReference {
                    syntax.as_type_reference_node().type_name
                } else {
                    syntax.as_expression_with_type_arguments().expression
                };
                let symbol = self.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.checker, s));
                self.check_symbols(l, [symbol, None], location);
                for &declaration in symbol.map_or(&[][..], |s| s.declarations()) {
                    if declaration.kind() == Kind::TypeAliasDeclaration && !self.is_library_declaration(declaration) {
                        self.references(l, declaration.type_node(), location);
                    }
                }
            }
            Kind::TypeQuery => {
                let name = syntax.as_type_query_node().expr_name;
                let symbol = self.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.checker, s));
                self.check_symbols(l, [symbol, None], location);
                let queried = self.checker.get_type_at_location(name);
                let mut signatures =
                    self.checker.get_signatures_of_type_exported(queried, SignatureKind::Call).to_vec();
                signatures.extend(self.checker.get_signatures_of_type_exported(queried, SignatureKind::Construct));
                for signature in signatures {
                    let returned = self.checker.get_return_type_of_signature_exported(signature);
                    self.mention(l, returned, location);
                }
            }
            _ => {}
        }
        let mut children = Vec::new();
        syntax.for_each_child(&mut |child| {
            children.push(child);
            false
        });
        for child in children {
            self.references(l, Some(child), location);
        }
    }

    fn lower(&mut self, l: &mut Lowering, t: P<Type>, location: &str, context: Context, depth: u32) -> Schema {
        self.lower_marked(l, t, location, context, depth).0
    }

    /// A type's schema, and whether a `serverOnly` mark wraps it.
    fn lower_marked(
        &mut self,
        l: &mut Lowering,
        t: P<Type>,
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
            let present: Vec<P<Type>> =
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
            let inner =
                if is_reference(t) { self.checker.get_type_arguments_exported(t).first().copied() } else { None };
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

    fn lower_type(&mut self, l: &mut Lowering, t: P<Type>, location: &str, context: Context, depth: u32) -> Schema {
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
        let arguments = if is_reference(t) { self.checker.get_type_arguments_exported(t).to_vec() } else { Vec::new() };
        match collection {
            Some(Collection::Tuple) => {
                let variable = t.target().is_some_and(|target| {
                    target
                        .as_tuple_type()
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
            let indices = self.checker.get_index_infos_of_type_exported(t);
            if let [index] = indices
                && index.key_type().flags().intersects(TypeFlags::String)
                && self.checker.get_properties_of_type_exported(t).is_empty()
            {
                let value = index.value_type();
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

    /// An array or tuple element or a map value. State drops `undefined` from a map value, which a missing key means.
    fn element(
        &mut self,
        l: &mut Lowering,
        t: P<Type>,
        location: &str,
        context: Context,
        depth: u32,
        value: bool,
    ) -> Schema {
        let state = l.spec.position != Position::Payload;
        let t = if state && value {
            self.checker.get_non_nullable_type(t)
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
        t: P<Type>,
        location: &str,
        context: Context,
        depth: u32,
        omit: Option<&str>,
    ) -> Schema {
        if self.signatures_of(t) {
            return self.fail(l, location, "functions cannot cross the network");
        }
        let mut properties: Vec<P<Symbol>> = self
            .checker
            .get_properties_of_type_exported(t)
            .iter()
            .copied()
            .filter(|p| Some(p.name()) != omit)
            .collect();
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
            for &d in property.declarations() {
                if let Some(parent) = d.parent().filter(|p| p.kind() == Kind::InterfaceDeclaration)
                    && let Some(symbol) = parent.name().and_then(|n| self.checker.get_symbol_at_location_exported(n))
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
            let value = self.checker.get_non_missing_type_of_symbol_exported(property);
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

    fn key(&mut self, l: &mut Lowering, t: P<Type>, location: &str) -> MapKey {
        self.declared(l, t, location);
        if t.flags().intersects(TypeFlags::String) {
            return MapKey::String;
        }
        let parts = members(t);
        if l.spec.position != Position::Payload && !parts.iter().all(|&p| types::literal_string(p).is_some()) {
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

    fn union(&mut self, l: &mut Lowering, t: P<Type>, location: &str, context: Context, depth: u32) -> Schema {
        let state = l.spec.position != Position::Payload;
        let parts = t.types().to_vec();
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
                && self.checker.get_index_infos_of_type_exported(part).is_empty()
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
                self.checker.get_properties_of_type_exported(objects[0]).iter().map(|p| p.name()).collect();
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

    /// The literal values `name` takes in `object`, when it is a required property of distinct literals.
    fn discriminant_values(
        &mut self,
        l: &mut Lowering,
        object: P<Type>,
        name: &str,
        location: &str,
        seen: &mut HashSet<(u8, String)>,
    ) -> Option<Vec<Literal>> {
        let property = self.checker.get_property_of_type_exported(object, name)?;
        if property.flags().intersects(SymbolFlags::Optional) {
            return None;
        }
        let declaration = property.value_declaration().or_else(|| property.declarations().first().copied());
        self.references(l, declaration.and_then(|d| d.type_node()), &format!("{location}.{name}"));
        let t = self.checker.get_non_missing_type_of_symbol_exported(property);
        let values: Vec<Option<Literal>> = members(t).into_iter().map(literal).collect();
        if values.iter().any(|v| v.as_ref().is_none_or(|v| seen.contains(&literal_key(v)))) {
            return None;
        }
        let values: Vec<Literal> = values.into_iter().flatten().collect();
        seen.extend(values.iter().map(literal_key));
        Some(values)
    }

    /// The number of schema nodes, with CRU019 where they nest past the depth limit.
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

    fn is_server_only(&mut self, t: P<Type>) -> bool {
        self.api(t.symbol()) == Some("ServerOnly")
    }

    /// An intersection of plain object types, which state flattens into one object.
    fn plain_intersection(&mut self, t: P<Type>) -> bool {
        t.flags().intersects(TypeFlags::Intersection)
            && t.types()
                .to_vec()
                .into_iter()
                .all(|part| part.flags().intersects(TypeFlags::Object) && !self.signatures_of(part))
    }

    fn engine(&mut self, t: P<Type>) -> Option<Engine> {
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
                let instances = self.checker.resolve_name_exported("Instances", at, SymbolFlags::Type, false);
                let known = instances.is_some_and(|i| {
                    let declared = self.checker.get_declared_type_of_symbol_exported(i);
                    self.checker.get_property_of_type_exported(declared, name).is_some()
                });
                if known { Engine::Instance(name) } else { Engine::Unsupported }
            }
        })
    }

    fn collection(&mut self, t: P<Type>) -> Option<Collection> {
        if t.is_tuple_type() {
            return Some(Collection::Tuple);
        }
        if self.checker.is_array_type_exported(t) {
            return Some(Collection::Array);
        }
        let symbol = t.symbol()?;
        (matches!(symbol.name(), "Map" | "ReadonlyMap") && self.standard(symbol, Standard::CompilerTypes))
            .then_some(Collection::Map)
    }

    /// The Crucible width a `number & { [width]?: "u8" }` brand names.
    fn width(&mut self, t: P<Type>) -> Option<Width> {
        if !t.flags().intersects(TypeFlags::Intersection)
            || !t.types().iter().any(|p| p.flags().intersects(TypeFlags::Number))
        {
            return None;
        }
        let mut found: Vec<&'static str> = Vec::new();
        for &part in t.types() {
            if part.flags().intersects(TypeFlags::Number) {
                continue;
            }
            let [property] = self.checker.get_properties_of_type_exported(part) else { return None };
            let property = *property;
            let name = property.declarations().first().and_then(|d| d.name())?;
            if name.kind() != Kind::ComputedPropertyName {
                return None;
            }
            let expression = name.expression()?;
            let symbol =
                self.checker.get_symbol_at_location_exported(expression).map(|s| unalias(&mut self.checker, s));
            if !symbol.is_some_and(|s| self.crucible.widths.contains(&s)) {
                return None;
            }
            let value = self.checker.get_non_missing_type_of_symbol_exported(property);
            for member in members(value) {
                if let Some(width) = types::literal_string(member)
                    && !found.contains(&width)
                {
                    found.push(width);
                }
            }
        }
        Some(if let [width] = found[..] { Width::Brand(width) } else { Width::Conflicting })
    }

    /// CRU021, CRU022 and CRU012 for one `receive` call, and the fact its call lowers from.
    pub(super) fn receive(&mut self, call: P<Node>, file: P<SourceFile>) {
        let field = call.parent().filter(|p| {
            let flags = tsrs_ast::get_combined_modifier_flags(*p);
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
        let name = field.name().and_then(|n| self.checker.get_symbol_at_location_exported(n)).map(|s| s.name());
        if let Some(name) = name {
            let mut seen = HashSet::new();
            let shadows = self.class_bases(instance).into_iter().any(|base| self.shadows(base, name, &mut seen));
            if shadows {
                self.report(call, Code::Rule(21), "A receiver field cannot shadow an inherited receiver.");
            }
        }
        let target = call.arguments().first().map(|&a| self.checker.get_type_at_location(a));
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
                if let Some(callback) = self.callback(call.arguments().get(1).copied()) {
                    self.callback_returns(call, callback, token);
                }
            }
        }
        if let Some(name) = name {
            self.site();
            let span = self.span(call);
            let fact = Box::new(Crucible::Receive { field: name.into() });
            self.table.store(span, Site::Crucible, |f| &mut f.crucible, fact);
        }
    }

    fn callback_returns(&mut self, call: P<Node>, callback: P<Node>, token: &str) {
        let signature = self.checker.get_signature_from_declaration_exported(callback);
        let returned = self.checker.get_return_type_of_signature_exported(signature);
        let parts = members(returned);
        let asynchronous = tsrs_ast::get_combined_modifier_flags(callback).intersects(ModifierFlags::Async);
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
                && !(token == "ClientReady"
                    && !self.checker.get_signatures_of_type_exported(t, SignatureKind::Call).is_empty())
        });
        if returns {
            let allowed = if token == "ClientReady" { " or a cleanup function" } else { "; use a block body" };
            let message =
                format!("{token} callbacks return nothing{allowed}; this one can return {}.", self.shown(returned));
            self.report(call, Code::Rule(12), &message);
        }
    }

    fn class_bases(&mut self, t: P<Type>) -> Vec<P<Type>> {
        if t.object_flags().intersects(tsrs_checker::ObjectFlags::ClassOrInterface) {
            self.checker.get_base_types_exported(t).to_vec()
        } else {
            Vec::new()
        }
    }

    /// Whether `t` or a base declares the field `name` with a `receive` initializer.
    fn shadows(&mut self, t: P<Type>, name: &str, seen: &mut HashSet<P<Type>>) -> bool {
        if !seen.insert(t) {
            return false;
        }
        let property = self.checker.get_property_of_type_exported(t, name);
        let receiver = property.is_some_and(|p| {
            p.declarations().iter().any(|&d| {
                d.kind() == Kind::PropertyDeclaration
                    && d.initializer().is_some_and(|i| self.is_crucible_call(i, Macro::Receive))
            })
        });
        receiver || self.class_bases(t).into_iter().any(|base| self.shadows(base, name, seen))
    }

    /// The function a callback argument names: a function written inline, a function or method declaration, or a
    /// const initialized with one.
    fn callback(&mut self, argument: Option<P<Node>>) -> Option<P<Node>> {
        let argument = argument?;
        match argument.kind() {
            Kind::FunctionExpression | Kind::ArrowFunction => Some(argument),
            Kind::Identifier | Kind::PropertyAccessExpression => {
                let name = if argument.kind() == Kind::PropertyAccessExpression {
                    argument.as_property_access_expression().name
                } else {
                    argument
                };
                let symbol =
                    self.checker.get_symbol_at_location_exported(name).map(|s| unalias(&mut self.checker, s))?;
                let [declaration] = symbol.declarations() else { return None };
                let declaration = *declaration;
                match declaration.kind() {
                    Kind::FunctionDeclaration | Kind::MethodDeclaration => Some(declaration),
                    Kind::VariableDeclaration
                        if tsrs_ast::get_combined_node_flags(declaration).intersects(NodeFlags::Const) =>
                    {
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

    /// Every symbol a declaration's lowering reaches.
    fn reached(&mut self) -> &HashSet<P<Symbol>> {
        if self.network.reached.is_none() {
            let declarations: Vec<(P<Node>, DeclarationKind)> =
                self.declarations().iter().map(|d| (d.call, d.kind)).collect();
            let mut reached = HashSet::new();
            for (call, kind) in declarations {
                reached.extend(self.lower_declaration(call, kind, false).1);
            }
            self.network.reached = Some(reached);
        }
        self.network.reached.get_or_insert_default()
    }

    /// CRU010 at a `declare global` or `declare module` block that adds a declaration to a symbol a declaration's
    /// value reaches.
    pub(super) fn augmentation(&mut self, n: P<Node>) {
        let named = n.name().is_some_and(|name| name.kind() == Kind::StringLiteral);
        if !named && !tsrs_ast::is_global_scope_augmentation(n) {
            return;
        }
        let mut added = Vec::new();
        added_declarations(n.body(), &mut added);
        let symbols: Vec<P<Symbol>> = added
            .into_iter()
            .filter_map(|d| d.name())
            .filter_map(|name| self.checker.get_symbol_at_location_exported(name))
            .collect();
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
