mod call;
mod class;
mod expr;
mod pattern;
mod stmt;

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use crate::facts::{Fact, FileFacts, Unanswered};
use crate::luau::{Block, Expr, Name, Stmt};
use crate::tree::{File, Kind, Node, Nodes, Op, Position, UnaryOp};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The absolute path with forward slashes.
    pub path: String,
    /// One-based line and column, in bytes.
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({},{}): error: {}", self.path, self.line, self.column, self.message)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportBase {
    Script,
    ScriptParent,
}

pub(crate) fn lower(
    file: File,
    facts: &FileFacts,
    unanswered: &[&Unanswered],
    import_base: ImportBase,
) -> Result<Block, Vec<Error>> {
    let mut l = Lower {
        file,
        facts,
        path: file.path(),
        import_base,
        errors: Vec::new(),
        current_block: Vec::new(),
        temp_prefix: temp_prefix(file.text()),
        temps: 0,
        uses_ts: false,
        refs: references(file.root()),
        closure_assigned: closure_assigned(file.root()),
        hoisted_name_starts: HashSet::new(),
        exports: Vec::new(),
        namespace: None,
        breakables: Vec::new(),
    };
    for u in unanswered {
        let Position { line, column } = file.position(u.span.start);
        let message = format!("the type facts cannot answer this {:?} site ({:?})", u.site, u.reason);
        l.errors.push(Error { path: l.path.clone(), line, column, message });
    }
    let body = l.module(file.root().statements());
    if l.errors.is_empty() { Ok(body) } else { Err(l.errors) }
}

#[derive(Clone, PartialEq)]
enum Breakable {
    Loop { copy_back: Block },
    Switch,
}

impl Breakable {
    const LOOP: Breakable = Breakable::Loop { copy_back: Vec::new() };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Usage {
    Value,
    Statement,
}

pub(crate) struct Lower<'a> {
    file: File,
    facts: &'a FileFacts,
    path: String,
    import_base: ImportBase,
    errors: Vec<Error>,
    current_block: Block,
    temp_prefix: String,
    temps: u32,
    uses_ts: bool,
    refs: Vec<Reference>,
    closure_assigned: HashSet<&'static str>,
    hoisted_name_starts: HashSet<u32>,
    exports: Vec<(Name, Expr)>,
    namespace: Option<Name>,
    breakables: Vec<Breakable>,
}

fn temp_prefix(text: &str) -> String {
    let mut prefix = String::from("_");
    while text.match_indices(prefix.as_str()).any(|(at, _)| reads_as_temp(text, at, &prefix)) {
        prefix.push('_');
    }
    prefix
}

fn reads_as_temp(text: &str, at: usize, prefix: &str) -> bool {
    let before = text[..at].chars().next_back();
    let after = text[at + prefix.len()..].chars().next();
    !before.is_some_and(|c| c == '_' || c == '$' || c.is_alphanumeric()) && after.is_some_and(|c| c.is_ascii_digit())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Reference {
    text: &'static str,
    start: u32,
}

/// Whether an identifier reads or writes a variable, rather than naming a declaration or a property.
fn is_reference(identifier: Node) -> bool {
    !identifier.parent().is_some_and(|p| {
        let named = p.name() == Some(identifier);
        match p.kind() {
            Kind::PropertyAccessExpression => named,
            Kind::VariableDeclaration
            | Kind::Parameter
            | Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::ClassDeclaration
            | Kind::EnumDeclaration
            | Kind::ModuleDeclaration
            | Kind::PropertyAssignment
            | Kind::PropertyDeclaration
            | Kind::MethodDeclaration
            | Kind::EnumMember
            | Kind::ImportSpecifier
            | Kind::ImportClause
            | Kind::NamespaceImport => named,
            Kind::BindingElement => named || p.property_name() == Some(identifier),
            _ => false,
        }
    })
}

fn references(root: Node) -> Vec<Reference> {
    fn walk(n: Node, out: &mut Vec<Reference>) {
        if n.is_erased() {
            return;
        }
        if n.kind() == Kind::Identifier {
            if is_reference(n) {
                out.push(Reference { text: n.text(), start: n.span().start });
            }
            return;
        }
        n.for_each_child(&mut |c| walk(c, out));
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_unstable();
    out
}

/// A function, or code that runs later as part of one, such as an instance property's initializer.
fn runs_later(n: Node) -> bool {
    match n.kind() {
        Kind::ArrowFunction
        | Kind::FunctionExpression
        | Kind::FunctionDeclaration
        | Kind::MethodDeclaration
        | Kind::Constructor
        | Kind::GetAccessor
        | Kind::SetAccessor => true,
        Kind::PropertyDeclaration => !n.is_static(),
        _ => false,
    }
}

/// The names of the locals that a function assigns although they are declared outside it. Luau reads a local operand
/// when the operator runs, after the operands to its right, and a call among them can change such a local.
fn closure_assigned(root: Node) -> HashSet<&'static str> {
    fn walk(n: Node, function: Option<Node>, out: &mut HashSet<&'static str>) {
        if n.is_erased() {
            return;
        }
        let function = if runs_later(n) { Some(n) } else { function };
        if let Some(function) = function {
            match n.kind() {
                Kind::BinaryExpression if n.operator().is_assignment() => targets(n.left(), function, out),
                Kind::PrefixUnaryExpression | Kind::PostfixUnaryExpression
                    if matches!(n.unary_operator(), Some(UnaryOp::Increment | UnaryOp::Decrement)) =>
                {
                    targets(n.operand(), function, out)
                }
                _ => {}
            }
        }
        n.for_each_child(&mut |c| walk(c, function, out));
    }
    fn targets(n: Node, function: Node, out: &mut HashSet<&'static str>) {
        let n = skip_parens_and_assertions(n);
        match n.kind() {
            Kind::Identifier => {
                if n.declaration().is_some_and(|d| !function.contains(d)) {
                    out.insert(n.text());
                }
            }
            Kind::ArrayLiteralExpression => n.elements().iter().for_each(|e| targets(e, function, out)),
            Kind::ObjectLiteralExpression => {
                for p in n.properties().iter() {
                    match p.kind() {
                        Kind::PropertyAssignment => {
                            if let Some(value) = p.initializer() {
                                targets(value, function, out);
                            }
                        }
                        Kind::ShorthandPropertyAssignment => {
                            if let Some(name) = p.name() {
                                targets(name, function, out);
                            }
                        }
                        Kind::SpreadAssignment => targets(p.expression(), function, out),
                        _ => {}
                    }
                }
            }
            Kind::SpreadElement => targets(n.expression(), function, out),
            Kind::BinaryExpression if n.operator() == Op::Assign => targets(n.left(), function, out),
            _ => {}
        }
    }
    let mut out = HashSet::new();
    walk(root, None, &mut out);
    out
}

impl<'a> Lower<'a> {
    fn error(&mut self, n: Node, message: impl Into<String>) {
        let Position { line, column } = self.file.position(n.span().start);
        self.errors.push(Error { path: self.path.clone(), line, column, message: message.into() });
    }

    fn unsupported(&mut self, n: Node, what: &str) -> Expr {
        self.error(n, format!("{what} is not supported"));
        Expr::Nil
    }

    fn fact(&self, n: Node) -> Option<&'a Fact> {
        self.facts.get(n.span())
    }

    fn temp(&mut self) -> Name {
        let name = format!("{}{}", self.temp_prefix, self.temps);
        self.temps += 1;
        Cow::Owned(name)
    }

    fn push_temp(&mut self, value: Expr) -> Expr {
        let name = self.temp();
        self.current_block.push(Stmt::Local(vec![name.clone()], vec![value]));
        Expr::Name(name)
    }

    fn push_temp_if_complex(&mut self, value: Expr) -> Expr {
        if value.is_simple() { value } else { self.push_temp(value) }
    }

    /// Stores `value` in a temporary when it may read differently after the statements `later` run, or after
    /// `read_after`, the operands Luau evaluates before it reads a local operand, such as the right side of a binary
    /// operator.
    fn store_before(&mut self, value: Expr, later: &[Stmt], read_after: &[Expr]) -> Expr {
        let moved = !later.is_empty() && !is_stable(&value, later);
        if moved || self.call_may_assign(&value, later, read_after) { self.push_temp(value) } else { value }
    }

    /// Whether `value` names a local that a function assigns, and the later operands may call one.
    fn call_may_assign(&self, value: &Expr, later: &[Stmt], read_after: &[Expr]) -> bool {
        matches!(value, Expr::Name(name) if self.closure_assigned.contains(name.as_ref()))
            && (!later.is_empty() || read_after.iter().any(Expr::may_call))
    }

    fn capture<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> (T, Block) {
        let outer = std::mem::take(&mut self.current_block);
        let value = f(self);
        let inner = std::mem::replace(&mut self.current_block, outer);
        (value, inner)
    }

    /// JavaScript evaluates operands left to right. When a later operand needs statements, the earlier values are
    /// stored first so they are still evaluated first.
    fn ordered(
        &mut self,
        nodes: impl IntoIterator<Item = Node>,
        mut each: impl FnMut(&mut Self, Node) -> Expr,
    ) -> Vec<Expr> {
        let lowered: Vec<(Expr, Block)> = nodes.into_iter().map(|n| self.capture(|l| each(l, n))).collect();
        self.order_values(lowered)
    }

    fn ordered_pair(&mut self, first: Node, second: Node) -> (Expr, Expr) {
        let (first, first_setup) = self.capture(|l| l.expr(first));
        let (second, second_setup) = self.capture(|l| l.expr(second));
        self.current_block.extend(first_setup);
        let first = self.store_before(first, &second_setup, std::slice::from_ref(&second));
        self.current_block.extend(second_setup);
        (first, second)
    }

    fn order_values(&mut self, lowered: Vec<(Expr, Block)>) -> Vec<Expr> {
        let last = lowered.iter().rposition(|(_, b)| !b.is_empty());
        let stable: Vec<bool> = (0..lowered.len())
            .map(|i| {
                let (value, later) = (&lowered[i].0, &lowered[i + 1..]);
                (last.is_none_or(|last| i >= last) || later.iter().all(|(_, block)| is_stable(value, block)))
                    && !later.iter().any(|(_, block)| self.call_may_assign(value, block, &[]))
            })
            .collect();
        let mut values = Vec::with_capacity(lowered.len());
        for ((value, block), stable) in lowered.into_iter().zip(stable) {
            self.current_block.extend(block);
            values.push(if stable { value } else { self.push_temp(value) });
        }
        values
    }

    fn block(&mut self, statements: Nodes) -> Block {
        let (_, block) = self.capture(|l| l.statements(statements));
        block
    }

    fn statements(&mut self, statements: Nodes) {
        self.hoist(statements);
        for s in statements.iter() {
            self.stmt(s);
        }
    }

    fn hoist(&mut self, statements: Nodes) {
        let start = statements.get(0).map_or(0, |s| s.span().start);
        let mut hoisted = Vec::new();
        for s in statements.iter() {
            for declared in declared_names(s) {
                let text = declared.name.text();
                if self.referenced_between(text, start, declared.visible_from, declared.start) {
                    self.hoisted_name_starts.insert(declared.start);
                    hoisted.push(Cow::Borrowed(text));
                }
            }
        }
        if !hoisted.is_empty() {
            self.current_block.push(Stmt::Local(hoisted, Vec::new()));
        }
    }

    fn referenced_between(&self, name: &str, start: u32, end: u32, except: u32) -> bool {
        let first = self.refs.partition_point(|r| r.text < name);
        self.refs[first..]
            .iter()
            .take_while(|r| r.text == name)
            .any(|r| r.start >= start && r.start < end && r.start != except)
    }

    fn is_hoisted(&self, name: Node) -> bool {
        self.hoisted_name_starts.contains(&name.span().start)
    }

    fn local_name(&mut self, n: Node) -> Name {
        let text = n.text();
        if !crate::luau::is_name(text) || EMITTED_GLOBALS.contains(&text) {
            self.error(n, format!("`{text}` cannot be a Luau local name"));
        }
        Cow::Borrowed(text)
    }
}

/// A simple value reads the same after `later` runs unless `later` assigns the name it reads.
fn is_stable(value: &Expr, later: &[Stmt]) -> bool {
    value.is_simple() && !matches!(value, Expr::Name(name) if assigns(later, name))
}

fn assigns(block: &[Stmt], name: &str) -> bool {
    let is_name = |e: &Expr| matches!(e, Expr::Name(n) if n == name);
    block.iter().any(|s| match s {
        Stmt::Assign(targets, _) => targets.iter().any(is_name),
        Stmt::Compound(_, target, _) | Stmt::Function(target, None, _) => is_name(target),
        Stmt::Do(body) | Stmt::While(_, body) | Stmt::Repeat(body, _) => assigns(body, name),
        Stmt::NumericFor(.., body) | Stmt::GenericFor(.., body) => assigns(body, name),
        Stmt::If(_, then, otherwise) => assigns(then, name) || otherwise.as_ref().is_some_and(|b| assigns(b, name)),
        Stmt::Local(..)
        | Stmt::LocalFunction(..)
        | Stmt::Function(..)
        | Stmt::Call(_)
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue => false,
    })
}

const EMITTED_GLOBALS: &[&str] = &[
    "self",
    "super",
    "TS",
    "_G",
    "script",
    "table",
    "string",
    "type",
    "typeof",
    "tostring",
    "tonumber",
    "error",
    "assert",
    "select",
    "setmetatable",
    "getmetatable",
    "next",
    "pairs",
    "ipairs",
    "rawget",
    "rawset",
    "rawequal",
    "bit32",
    "utf8",
    "math",
    "unpack",
];

struct Declared {
    name: Node,
    start: u32,
    visible_from: u32,
}

fn declared_names(s: Node) -> Vec<Declared> {
    let mut out = Vec::new();
    let span = s.span();
    let mut push = |name: Node, visible_from: u32| out.push(Declared { name, start: name.span().start, visible_from });
    match s.kind() {
        Kind::VariableStatement if !s.is_ambient() => {
            for d in s.declarations().iter() {
                if let Some(name) = d.name() {
                    pattern_names(name, &mut |n| push(n, span.end));
                }
            }
        }
        Kind::FunctionDeclaration | Kind::ClassDeclaration | Kind::EnumDeclaration | Kind::ModuleDeclaration
            if !s.is_ambient() =>
        {
            if let Some(name) = s.name() {
                push(name, span.start);
            }
        }
        Kind::ImportDeclaration => {
            if let Some(clause) = s.import_clause() {
                if let Some(name) = clause.name() {
                    push(name, span.start);
                }
                if let Some(bindings) = clause.named_bindings() {
                    match bindings.kind() {
                        Kind::NamespaceImport => {
                            if let Some(name) = bindings.name() {
                                push(name, span.start);
                            }
                        }
                        Kind::NamedImports => {
                            for e in bindings.elements().iter() {
                                if let Some(name) = e.name() {
                                    push(name, span.start);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn pattern_names(n: Node, f: &mut dyn FnMut(Node)) {
    match n.kind() {
        Kind::Identifier => f(n),
        Kind::ObjectBindingPattern | Kind::ArrayBindingPattern => {
            for e in n.elements().iter() {
                if let Some(name) = e.name() {
                    pattern_names(name, f);
                }
            }
        }
        _ => {}
    }
}

fn is_hole(e: Node) -> bool {
    e.kind() == Kind::OmittedExpression || (e.kind() == Kind::BindingElement && e.name().is_none())
}

fn skip_parens_and_assertions(mut n: Node) -> Node {
    while matches!(
        n.kind(),
        Kind::ParenthesizedExpression
            | Kind::AsExpression
            | Kind::NonNullExpression
            | Kind::SatisfiesExpression
            | Kind::TypeAssertionExpression
    ) {
        n = n.expression();
    }
    n
}
