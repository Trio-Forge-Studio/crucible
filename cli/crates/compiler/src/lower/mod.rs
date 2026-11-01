//! The lowering: one source file's syntax tree and its facts in, a Luau chunk out. It never calls the checker. A
//! site the facts do not answer, and any construct outside the supported set, is an error at its source location.

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
use crate::tree::{File, Kind, Node, Nodes};

/// A construct the lowering refuses.
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

/// Lowers one source file. `init` says whether the file is its folder's `init` module, which changes how relative
/// imports start.
pub(crate) fn lower(
    file: File,
    facts: &FileFacts,
    unanswered: &[&Unanswered],
    init: bool,
) -> Result<Block, Vec<Error>> {
    let mut l = Lower {
        file,
        facts,
        path: file.path(),
        init,
        errors: Vec::new(),
        out: Vec::new(),
        temp_prefix: temp_prefix(file.text()),
        temps: 0,
        uses_ts: false,
        refs: references(file.root()),
        hoisted: HashSet::new(),
        exports: Vec::new(),
        container: None,
        breakables: Vec::new(),
    };
    for u in unanswered {
        let (line, column) = file.position(u.span.start);
        let message = format!("the type facts cannot answer this {:?} site ({:?})", u.site, u.reason);
        l.errors.push(Error { path: l.path.clone(), line, column, message });
    }
    let body = l.module(file.root().statements());
    if l.errors.is_empty() { Ok(body) } else { Err(l.errors) }
}

/// What a `break` or `continue` inside the current statement would leave.
#[derive(Clone, PartialEq)]
enum Breakable {
    /// `continue` runs `copy_back` first, to store this pass's copies of the head's variables.
    Loop {
        copy_back: Block,
    },
    Switch,
}

impl Breakable {
    const LOOP: Breakable = Breakable::Loop { copy_back: Vec::new() };
}

pub(crate) struct Lower<'a> {
    file: File,
    facts: &'a FileFacts,
    path: String,
    init: bool,
    errors: Vec<Error>,
    /// The block being built. Expressions push the statements they need before the statement that uses them.
    out: Block,
    temp_prefix: String,
    temps: u32,
    uses_ts: bool,
    refs: Vec<(&'static str, u32)>,
    /// Name spans of declarations whose local is declared at the top of their block.
    hoisted: HashSet<u32>,
    exports: Vec<(Name, Expr)>,
    /// The namespace whose body is being lowered.
    container: Option<Name>,
    breakables: Vec<Breakable>,
}

/// Temporaries are `<prefix><n>`, with a prefix no identifier in the file starts with followed by a digit.
fn temp_prefix(text: &str) -> String {
    let mut prefix = String::from("_");
    loop {
        let clash = text.match_indices(prefix.as_str()).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + prefix.len()..].chars().next();
            !before.is_some_and(|c| c == '_' || c == '$' || c.is_alphanumeric())
                && after.is_some_and(|c| c.is_ascii_digit())
        });
        if !clash {
            return prefix;
        }
        prefix.push('_');
    }
}

/// Every identifier that may refer to a declaration, by text and start offset, sorted. Property names and the names
/// a declaration introduces are left out.
fn references(root: Node) -> Vec<(&'static str, u32)> {
    fn walk(n: Node, out: &mut Vec<(&'static str, u32)>) {
        if n.is_type() {
            return;
        }
        if n.kind() == Kind::Identifier {
            let declares = n.parent().is_some_and(|p| {
                let named = p.name() == Some(n);
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
                    Kind::BindingElement => named || p.property_name() == Some(n),
                    _ => false,
                }
            });
            if !declares {
                out.push((n.text(), n.span().start));
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

impl<'a> Lower<'a> {
    fn error(&mut self, n: Node, message: impl Into<String>) {
        let (line, column) = self.file.position(n.span().start);
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

    /// Declares a temporary holding `value` and returns it.
    fn push_temp(&mut self, value: Expr) -> Expr {
        let name = self.temp();
        self.out.push(Stmt::Local(vec![name.clone()], vec![value]));
        Expr::Name(name)
    }

    fn push_temp_if_complex(&mut self, value: Expr) -> Expr {
        if value.is_simple() { value } else { self.push_temp(value) }
    }

    /// Stores `value` when the statements in `later` run before it is read and could change it.
    fn store_before(&mut self, value: Expr, later: &[Stmt]) -> Expr {
        if later.is_empty() || is_stable(&value, later) { value } else { self.push_temp(value) }
    }

    /// Runs `f` with an empty block and returns what it built alongside its result.
    fn capture<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> (T, Block) {
        let outer = std::mem::take(&mut self.out);
        let value = f(self);
        let inner = std::mem::replace(&mut self.out, outer);
        (value, inner)
    }

    /// Lowers expressions left to right. When a later one needs statements, the earlier values are stored first so
    /// they are still evaluated first.
    fn ordered(
        &mut self,
        nodes: impl IntoIterator<Item = Node>,
        mut each: impl FnMut(&mut Self, Node) -> Expr,
    ) -> Vec<Expr> {
        let lowered: Vec<(Expr, Block)> = nodes.into_iter().map(|n| self.capture(|l| each(l, n))).collect();
        self.order_values(lowered)
    }

    /// `ordered` for two expressions.
    fn ordered_pair(&mut self, first: Node, second: Node) -> (Expr, Expr) {
        let (first, first_setup) = self.capture(|l| l.expr(first));
        let (second, second_setup) = self.capture(|l| l.expr(second));
        self.out.extend(first_setup);
        let first = self.store_before(first, &second_setup);
        self.out.extend(second_setup);
        (first, second)
    }

    /// The same as `ordered` for values already lowered with their statements.
    fn order_values(&mut self, lowered: Vec<(Expr, Block)>) -> Vec<Expr> {
        let last = lowered.iter().rposition(|(_, b)| !b.is_empty());
        let stable: Vec<bool> = (0..lowered.len())
            .map(|i| {
                last.is_none_or(|last| i >= last)
                    || lowered[i + 1..].iter().all(|(_, later)| is_stable(&lowered[i].0, later))
            })
            .collect();
        let mut values = Vec::with_capacity(lowered.len());
        for ((value, block), stable) in lowered.into_iter().zip(stable) {
            self.out.extend(block);
            values.push(if stable { value } else { self.push_temp(value) });
        }
        values
    }

    /// Lowers a statement list into its own block.
    fn block(&mut self, statements: Nodes) -> Block {
        let (_, block) = self.capture(|l| l.statements(statements));
        block
    }

    /// Lowers a statement list into the current block.
    fn statements(&mut self, statements: Nodes) {
        self.hoist(statements);
        for s in statements.iter() {
            self.stmt(s);
        }
    }

    /// Declares at the top of the current block every local in `statements` that one of them refers to before
    /// its declaration runs.
    fn hoist(&mut self, statements: Nodes) {
        let start = statements.get(0).map_or(0, |s| s.span().start);
        let mut hoisted = Vec::new();
        for s in statements.iter() {
            for (name, span, until) in declared_names(s) {
                if self.referenced_between(name.text(), start, until, span) {
                    self.hoisted.insert(span);
                    hoisted.push(Cow::Borrowed(name.text()));
                }
            }
        }
        if !hoisted.is_empty() {
            self.out.push(Stmt::Local(hoisted, Vec::new()));
        }
    }

    fn referenced_between(&self, name: &str, start: u32, end: u32, except: u32) -> bool {
        let first = self.refs.partition_point(|&(n, _)| n < name);
        self.refs[first..]
            .iter()
            .take_while(|&&(n, _)| n == name)
            .any(|&(_, at)| at >= start && at < end && at != except)
    }

    fn is_hoisted(&self, name: Node) -> bool {
        self.hoisted.contains(&name.span().start)
    }

    /// A Luau local for an identifier the source declares.
    fn local_name(&mut self, n: Node) -> Name {
        let text = n.text();
        if !crate::luau::is_name(text) || RESERVED.contains(&text) {
            self.error(n, format!("`{text}` cannot be a Luau local name"));
        }
        Cow::Borrowed(text)
    }
}

/// Whether `value` reads the same after `later` runs: it is simple, and `later` assigns no name it reads.
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

/// Globals the emitted code calls, which a source local must not shadow.
const RESERVED: &[&str] = &[
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

/// The names a statement declares in its block, each with its name's span start and the offset before which a
/// reference means the local must be declared earlier.
fn declared_names(s: Node) -> Vec<(Node, u32, u32)> {
    let mut out = Vec::new();
    let span = s.span();
    match s.kind() {
        Kind::VariableStatement if !s.is_ambient() => {
            for d in s.declarations().iter() {
                if let Some(name) = d.name() {
                    pattern_names(name, &mut |n| out.push((n, n.span().start, span.end)));
                }
            }
        }
        Kind::FunctionDeclaration | Kind::ClassDeclaration | Kind::EnumDeclaration | Kind::ModuleDeclaration
            if !s.is_ambient() =>
        {
            if let Some(name) = s.name() {
                out.push((name, name.span().start, span.start));
            }
        }
        Kind::ImportDeclaration => {
            if let Some(clause) = s.import_clause() {
                if let Some(name) = clause.name() {
                    out.push((name, name.span().start, span.start));
                }
                if let Some(bindings) = clause.named_bindings() {
                    match bindings.kind() {
                        Kind::NamespaceImport => {
                            if let Some(name) = bindings.name() {
                                out.push((name, name.span().start, span.start));
                            }
                        }
                        Kind::NamedImports => {
                            for e in bindings.elements().iter() {
                                if let Some(name) = e.name() {
                                    out.push((name, name.span().start, span.start));
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

/// Calls `f` with each identifier a binding name or pattern declares.
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

/// Whether an array pattern element is a hole, as in `[, b]`.
fn is_hole(e: Node) -> bool {
    e.kind() == Kind::OmittedExpression || (e.kind() == Kind::BindingElement && e.name().is_none())
}

/// Skips parentheses, type assertions and non-null assertions.
fn skip_down(mut n: Node) -> Node {
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
