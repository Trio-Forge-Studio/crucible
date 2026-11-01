use std::borrow::Cow;

use super::{Breakable, Lower, is_hole, skip_down};
use crate::facts::{Callee, Constant, Import, Receiver};
use crate::luau::{BinOp, Block, Expr, Field, Function, Name, Stmt};
use crate::macros::Macro;
use crate::tree::{Kind, Node, Nodes, Op, UnaryOp};

impl Lower<'_> {
    pub(super) fn module(&mut self, statements: Nodes) -> Block {
        self.statements(statements);
        let mut body = std::mem::take(&mut self.out);
        let exports = std::mem::take(&mut self.exports);
        let value = if exports.is_empty() {
            Expr::Nil
        } else {
            Expr::Table(exports.into_iter().map(|(name, value)| Field::Keyed(Expr::String(name), value)).collect())
        };
        body.push(Stmt::Return(vec![value]));
        if self.uses_ts {
            body.insert(
                0,
                Stmt::Local(vec![Cow::Borrowed("TS")], vec![Expr::global("_G").index(Expr::global("script"))]),
            );
        }
        body
    }

    pub(super) fn stmt(&mut self, s: Node) {
        match s.kind() {
            Kind::Block => {
                let block = self.block(s.statements());
                self.out.push(Stmt::Do(block));
            }
            Kind::EmptyStatement | Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration => {}
            Kind::VariableStatement => self.variable_statement(s),
            Kind::ExpressionStatement => self.expr_stmt(s.expression()),
            Kind::IfStatement => self.if_statement(s),
            Kind::ReturnStatement => {
                let values = match s.expression_opt() {
                    Some(e) => self.return_values(e),
                    None => Vec::new(),
                };
                self.out.push(Stmt::Return(values));
            }
            Kind::ForStatement => self.for_statement(s),
            Kind::ForOfStatement => self.for_of(s),
            Kind::WhileStatement => self.while_statement(s),
            Kind::DoStatement => self.do_statement(s),
            Kind::BreakStatement => {
                if s.has_label() {
                    self.error(s, "labeled break is not supported");
                }
                self.out.push(Stmt::Break);
            }
            Kind::ContinueStatement => {
                if s.has_label() {
                    self.error(s, "labeled continue is not supported");
                }
                if self.breakables.last() == Some(&Breakable::Switch) {
                    self.error(s, "continue inside a switch is not supported");
                }
                self.out.push(Stmt::Continue);
            }
            Kind::SwitchStatement => self.switch_statement(s),
            Kind::FunctionDeclaration => self.function_declaration(s),
            Kind::ClassDeclaration => self.class_declaration(s),
            Kind::EnumDeclaration => self.enum_declaration(s),
            Kind::ModuleDeclaration => self.namespace(s),
            Kind::ImportDeclaration => self.import_declaration(s),
            Kind::ExportDeclaration => self.export_declaration(s),
            Kind::ThrowStatement => {
                self.unsupported(s, "throw");
            }
            Kind::TryStatement => {
                self.unsupported(s, "try");
            }
            Kind::LabeledStatement => {
                self.unsupported(s, "a labeled statement");
            }
            Kind::ExportAssignment => {
                self.unsupported(s, "export assignment");
            }
            Kind::ImportEqualsDeclaration => {
                self.unsupported(s, "import assignment");
            }
            _ => {
                let what = format!("the statement {}", s.kind_name());
                self.unsupported(s, &what);
            }
        }
    }

    /// Lowers the body of a branch or loop, which may be a single statement.
    pub(super) fn body_block(&mut self, s: Node) -> Block {
        if s.kind() == Kind::Block {
            self.block(s.statements())
        } else {
            let (_, block) = self.capture(|l| l.stmt(s));
            block
        }
    }

    pub(super) fn export(&mut self, name: Name) {
        match &self.container {
            Some(container) => {
                let target = Expr::Name(container.clone()).field(name.clone());
                self.out.push(Stmt::Assign(vec![target], vec![Expr::Name(name)]));
            }
            None => self.exports.push((name.clone(), Expr::Name(name))),
        }
    }

    /// Declares `name` with `value`, or assigns it when the block declared it first.
    pub(super) fn declare(&mut self, name_node: Node, name: Name, value: Option<Expr>) {
        if self.is_hoisted(name_node) {
            if let Some(value) = value {
                self.out.push(Stmt::Assign(vec![Expr::Name(name)], vec![value]));
            }
        } else {
            self.out.push(Stmt::Local(vec![name], value.into_iter().collect()));
        }
    }

    fn variable_statement(&mut self, s: Node) {
        if s.is_ambient() {
            return;
        }
        if s.is_var() {
            self.error(s, "var is not supported; use let or const");
        }
        let export = s.is_export();
        if export && !s.is_const() {
            self.error(s, "export let is not supported; export a const");
        }
        for d in s.declarations().iter() {
            self.variable_declaration(d, export);
        }
    }

    pub(super) fn variable_declaration(&mut self, d: Node, export: bool) {
        let Some(name) = d.name() else { return };
        match name.kind() {
            Kind::Identifier => {
                let local = self.local_name(name);
                let value = d.initializer().map(|i| self.expr(i));
                self.declare(name, local.clone(), value);
                if export {
                    self.export(local);
                }
            }
            Kind::ArrayBindingPattern | Kind::ObjectBindingPattern => {
                let Some(init) = d.initializer() else {
                    self.error(d, "a destructuring declaration needs an initializer");
                    return;
                };
                self.bind_initializer(name, init);
                if export {
                    let mut names = Vec::new();
                    super::pattern_names(name, &mut |n| names.push(n));
                    for n in names {
                        let local = self.local_name(n);
                        self.export(local);
                    }
                }
            }
            _ => {
                self.unsupported(name, "this declaration name");
            }
        }
    }

    fn if_statement(&mut self, s: Node) {
        let condition = self.condition(s.expression());
        let then = self.body_block(s.then_statement());
        let otherwise = s.else_statement().map(|e| {
            if e.kind() == Kind::IfStatement {
                let (_, block) = self.capture(|l| l.if_statement(e));
                block
            } else {
                self.body_block(e)
            }
        });
        self.out.push(Stmt::If(condition, then, otherwise));
    }

    /// The values a `return` or an arrow body returns. A `LuaTuple` returns its members as several values.
    pub(super) fn return_values(&mut self, e: Node) -> Vec<Expr> {
        let inner = skip_down(e);
        if inner.kind() == Kind::CallExpression
            && self.fact(inner.expression()).and_then(|f| f.callee) == Some(Callee::Macro(Macro::Tuple))
        {
            return self.ordered(inner.arguments().iter(), |l, a| l.expr(a));
        }
        let tuple = self.fact(e).and_then(|f| f.receiver) == Some(Receiver::LuaTuple);
        if !tuple {
            return vec![self.expr(e)];
        }
        match inner.kind() {
            Kind::CallExpression => vec![self.call(inner, false).unwrap_or(Expr::Nil)],
            Kind::ArrayLiteralExpression if !inner.elements().iter().any(|e| e.kind() == Kind::SpreadElement) => {
                self.ordered(inner.elements().iter(), |l, a| l.expr(a))
            }
            _ => {
                let value = self.expr(e);
                vec![Expr::global("table.unpack").call(vec![value])]
            }
        }
    }

    fn with_breakable<T>(&mut self, kind: Breakable, f: impl FnOnce(&mut Self) -> T) -> T {
        self.breakables.push(kind);
        let value = f(self);
        self.breakables.pop();
        value
    }

    fn while_statement(&mut self, s: Node) {
        let (condition, prereqs) = self.capture(|l| l.condition(s.expression()));
        let mut body = self.with_breakable(Breakable::Loop, |l| l.body_block(s.statement()));
        if prereqs.is_empty() {
            self.out.push(Stmt::While(condition, body));
        } else {
            let mut block = prereqs;
            block.push(Stmt::If(condition.negated(), vec![Stmt::Break], None));
            block.append(&mut body);
            self.out.push(Stmt::While(Expr::Bool(true), block));
        }
    }

    fn do_statement(&mut self, s: Node) {
        let mut body = self.with_breakable(Breakable::Loop, |l| l.body_block(s.statement()));
        let (condition, prereqs) = self.capture(|l| l.condition(s.expression()));
        if !prereqs.is_empty() && has_continue(s.statement()) {
            self.error(s, "continue in a do-while whose condition needs statements is not supported");
        }
        body.extend(prereqs);
        self.out.push(Stmt::Repeat(body, condition.negated()));
    }

    fn for_statement(&mut self, s: Node) {
        let (_, mut block) = self.capture(|l| {
            if let Some(init) = s.initializer() {
                if init.kind() == Kind::VariableDeclarationList {
                    if init.is_var() {
                        l.error(init, "var is not supported; use let or const");
                    }
                    for d in init.declarations().iter() {
                        l.variable_declaration(d, false);
                    }
                } else {
                    l.expr_stmt(init);
                }
            }
        });
        let (condition, prereqs) = match s.condition() {
            Some(c) => self.capture(|l| l.condition(c)),
            None => (Expr::Bool(true), Vec::new()),
        };
        let increment = match s.incrementor() {
            Some(i) => self.capture(|l| l.expr_stmt(i)).1,
            None => Vec::new(),
        };
        let mut body = self.with_breakable(Breakable::Loop, |l| l.body_block(s.statement()));
        if has_continue(s.statement()) && !increment.is_empty() {
            // `continue` jumps to the loop's start, so the increment runs there on every pass but the first.
            let flag = self.temp();
            block.push(Stmt::Local(vec![flag.clone()], vec![Expr::Bool(false)]));
            let mut inner = vec![Stmt::If(
                Expr::Name(flag.clone()),
                increment,
                Some(vec![Stmt::Assign(vec![Expr::Name(flag)], vec![Expr::Bool(true)])]),
            )];
            inner.extend(prereqs);
            if condition != Expr::Bool(true) {
                inner.push(Stmt::If(condition.negated(), vec![Stmt::Break], None));
            }
            inner.append(&mut body);
            block.push(Stmt::While(Expr::Bool(true), inner));
        } else if prereqs.is_empty() {
            body.extend(increment);
            block.push(Stmt::While(condition, body));
        } else {
            let mut inner = prereqs;
            inner.push(Stmt::If(condition.negated(), vec![Stmt::Break], None));
            inner.append(&mut body);
            inner.extend(increment);
            block.push(Stmt::While(Expr::Bool(true), inner));
        }
        self.out.push(Stmt::Do(block));
    }

    fn for_of(&mut self, s: Node) {
        let iterable = s.expression();
        let initializer = s.initializer().expect("a for-of has an initializer");
        let binding = if initializer.kind() == Kind::VariableDeclarationList {
            if initializer.is_var() {
                self.error(initializer, "var is not supported; use let or const");
            }
            match initializer.declarations().get(0).and_then(|d| d.name()) {
                Some(name) => name,
                None => return,
            }
        } else {
            self.unsupported(initializer, "a for-of over an existing variable");
            return;
        };
        let inner = skip_down(iterable);
        if inner.kind() == Kind::CallExpression
            && self.fact(inner.expression()).and_then(|f| f.callee) == Some(Callee::Macro(Macro::Range))
        {
            return self.for_range(s, inner, binding);
        }
        let Some(receiver) = self.fact(iterable).and_then(|f| f.receiver) else {
            self.error(iterable, "the type facts do not say how to iterate this value");
            return;
        };
        let source = self.expr(iterable);
        let simple_name =
            |l: &mut Self, n: Node| -> Option<Name> { (n.kind() == Kind::Identifier).then(|| l.local_name(n)) };
        let (names, iter, bind): (Vec<Name>, Vec<Expr>, Option<(Node, Expr)>) = match receiver {
            Receiver::Array => {
                let skip = self.temp();
                match simple_name(self, binding) {
                    Some(name) => (vec![skip, name], vec![source], None),
                    None => {
                        let value = self.temp();
                        (vec![skip, value.clone()], vec![source], Some((binding, Expr::Name(value))))
                    }
                }
            }
            Receiver::Set | Receiver::IterFn => match simple_name(self, binding) {
                Some(name) => (vec![name], vec![source], None),
                None => {
                    let value = self.temp();
                    (vec![value.clone()], vec![source], Some((binding, Expr::Name(value))))
                }
            },
            Receiver::Map | Receiver::LuaTupleIterFn => {
                let inline = binding.kind() == Kind::ArrayBindingPattern
                    && binding.elements().iter().all(|e| {
                        is_hole(e)
                            || (!e.is_rest()
                                && e.initializer().is_none()
                                && e.name().is_some_and(|n| n.kind() == Kind::Identifier))
                    });
                if inline {
                    let mut names = Vec::new();
                    for e in binding.elements().iter() {
                        let name = match e.name() {
                            Some(name) if !is_hole(e) => self.local_name(name),
                            _ => self.temp(),
                        };
                        names.push(name);
                    }
                    if names.is_empty() {
                        names.push(self.temp());
                    }
                    (names, vec![source], None)
                } else if receiver == Receiver::Map {
                    let (k, v) = (self.temp(), self.temp());
                    let entry =
                        Expr::Table(vec![Field::Item(Expr::Name(k.clone())), Field::Item(Expr::Name(v.clone()))]);
                    (vec![k, v], vec![source], Some((binding, entry)))
                } else {
                    self.unsupported(binding, "this binding over a LuaTuple iterator");
                    return;
                }
            }
            Receiver::String => {
                let skip = self.temp();
                let name = match simple_name(self, binding) {
                    Some(name) => name,
                    None => {
                        self.unsupported(binding, "destructuring a string character");
                        return;
                    }
                };
                let pattern = Expr::global("utf8.charpattern");
                (vec![skip, name], vec![Expr::global("string.gmatch").call(vec![source, pattern])], None)
            }
            Receiver::LuaTuple | Receiver::Other => {
                self.unsupported(iterable, "iterating this value");
                return;
            }
        };
        let body = self.with_breakable(Breakable::Loop, |l| {
            let (_, mut block) = l.capture(|l| {
                if let Some((pattern, value)) = bind {
                    l.bind_value(pattern, value);
                }
            });
            block.extend(l.body_block(s.statement()));
            block
        });
        self.out.push(Stmt::GenericFor(names, iter, body));
    }

    fn for_range(&mut self, s: Node, call: Node, binding: Node) {
        if binding.kind() != Kind::Identifier {
            self.unsupported(binding, "destructuring a $range value");
            return;
        }
        let name = self.local_name(binding);
        let mut args = self.ordered(call.arguments().iter(), |l, a| l.expr(a));
        if args.len() < 2 {
            self.error(call, "$range needs a start and an end");
            return;
        }
        let step = (args.len() > 2).then(|| args.remove(2));
        let end = args.remove(1);
        let start = args.remove(0);
        let body = self.with_breakable(Breakable::Loop, |l| l.body_block(s.statement()));
        self.out.push(Stmt::NumericFor(name, start, end, step, body));
    }

    fn switch_statement(&mut self, s: Node) {
        let discriminant = self.expr(s.expression());
        let value = self.push_temp_if_complex(discriminant);
        let clauses: Vec<Node> = s.clauses().iter().collect();
        if let Some(i) = clauses.iter().position(|c| c.kind() == Kind::DefaultClause)
            && i + 1 != clauses.len()
        {
            self.error(clauses[i], "a default clause before other cases is not supported");
        }
        let falls = clauses
            .iter()
            .enumerate()
            .any(|(i, c)| i + 1 < clauses.len() && !c.statements().is_empty() && !ends_flow(c.statements()));
        let fallthrough = falls.then(|| self.temp());
        let body = self.with_breakable(Breakable::Switch, |l| {
            let (_, block) = l.capture(|l| {
                if let Some(flag) = &fallthrough {
                    l.out.push(Stmt::Local(vec![flag.clone()], vec![Expr::Bool(false)]));
                }
                let mut tests: Vec<Expr> = Vec::new();
                let mut after_fallthrough = false;
                for clause in &clauses {
                    if clause.kind() == Kind::DefaultClause {
                        let statements = l.block(clause.statements());
                        l.out.extend(statements);
                        tests.clear();
                        continue;
                    }
                    let (test, prereqs) = l.capture(|l| l.expr(clause.expression()));
                    if !prereqs.is_empty() {
                        l.error(clause.expression(), "a case expression that needs statements is not supported");
                    }
                    tests.push(value.clone().binary(BinOp::Eq, test));
                    if clause.statements().is_empty() {
                        continue;
                    }
                    let mut condition = tests.drain(..).reduce(|a, b| a.binary(BinOp::Or, b)).unwrap();
                    if after_fallthrough && let Some(flag) = &fallthrough {
                        condition = Expr::Name(flag.clone()).binary(BinOp::Or, condition);
                    }
                    let mut block = l.block(clause.statements());
                    let ends = ends_flow(clause.statements());
                    if !ends && let Some(flag) = &fallthrough {
                        block.push(Stmt::Assign(vec![Expr::Name(flag.clone())], vec![Expr::Bool(true)]));
                    }
                    after_fallthrough = !ends;
                    l.out.push(Stmt::If(condition, block, None));
                }
            });
            block
        });
        self.out.push(Stmt::Repeat(body, Expr::Bool(true)));
    }

    fn function_declaration(&mut self, s: Node) {
        if s.body().is_none() || s.is_ambient() {
            return;
        }
        if s.is_default() {
            self.unsupported(s, "export default");
            return;
        }
        let Some(name) = s.name() else {
            self.unsupported(s, "an anonymous function declaration");
            return;
        };
        let local = self.local_name(name);
        let function = self.function(s);
        if self.is_hoisted(name) {
            self.out.push(Stmt::Function(Expr::Name(local.clone()), None, function));
        } else {
            self.out.push(Stmt::LocalFunction(local.clone(), function));
        }
        if s.is_export() {
            self.export(local);
        }
    }

    /// Lowers a function-like node: its parameters, then its body.
    pub(super) fn function(&mut self, f: Node) -> Function {
        if f.is_async() {
            self.unsupported(f, "an async function");
        }
        if f.is_generator() {
            self.unsupported(f, "a generator");
        }
        let method = match self.fact(f).and_then(|fact| fact.callee) {
            Some(Callee::Method) => true,
            Some(_) => false,
            None => {
                self.error(f, "the type facts do not say whether this function takes self");
                false
            }
        };
        let outer = std::mem::take(&mut self.breakables);
        let container = self.container.take();
        let ((mut params, varargs), body) = self.capture(|l| {
            let params = l.parameters(f.parameters());
            match f.body() {
                Some(body) if body.kind() == Kind::Block => l.statements(body.statements()),
                Some(body) => {
                    let values = l.return_values(body);
                    l.out.push(Stmt::Return(values));
                }
                None => {}
            }
            params
        });
        self.breakables = outer;
        self.container = container;
        if method {
            params.insert(0, Cow::Borrowed("self"));
        }
        Function { params, varargs, body }
    }

    /// The parameter names and whether the last is a rest parameter. Default values and destructuring become
    /// statements in the current block.
    pub(super) fn parameters(&mut self, params: Nodes) -> (Vec<Name>, bool) {
        let mut names = Vec::new();
        let mut varargs = false;
        for p in params.iter() {
            let Some(name) = p.name() else { continue };
            if name.kind() == Kind::Identifier && name.text() == "this" {
                continue;
            }
            if p.is_rest() {
                varargs = true;
                let rest = Expr::Table(vec![Field::Item(Expr::VarArgs)]);
                if name.kind() == Kind::Identifier {
                    let local = self.local_name(name);
                    self.out.push(Stmt::Local(vec![local], vec![rest]));
                } else {
                    let value = self.push_temp(rest);
                    self.bind_value(name, value);
                }
                continue;
            }
            let local = if name.kind() == Kind::Identifier { self.local_name(name) } else { self.temp() };
            names.push(local.clone());
            if let Some(default) = p.initializer() {
                let (value, mut block) = self.capture(|l| l.expr(default));
                block.push(Stmt::Assign(vec![Expr::Name(local.clone())], vec![value]));
                self.out.push(Stmt::If(Expr::Name(local.clone()).binary(BinOp::Eq, Expr::Nil), block, None));
            }
            if name.kind() != Kind::Identifier {
                self.bind_value(name, Expr::Name(local));
            }
        }
        (names, varargs)
    }

    fn enum_declaration(&mut self, s: Node) {
        if s.is_ambient() {
            return;
        }
        let name = s.name().expect("an enum has a name");
        let local = self.local_name(name);
        if !self.is_hoisted(name) {
            self.out.push(Stmt::Local(vec![local.clone()], Vec::new()));
        }
        let members: Vec<Node> = s.members().iter().collect();
        let (_, block) = self.capture(|l| {
            let mut values = Vec::new();
            for m in &members {
                let key = m.name().expect("an enum member has a name");
                if !matches!(key.kind(), Kind::Identifier | Kind::StringLiteral) {
                    l.unsupported(key, "this enum member name");
                    continue;
                }
                let value = match l.fact(*m).and_then(|f| f.constant.clone()) {
                    Some(Constant::Number(n)) => Expr::Number(n),
                    Some(Constant::String(s)) => Expr::String(Cow::Owned(s.into())),
                    None => match m.initializer() {
                        Some(i) => l.expr(i),
                        None => {
                            l.error(*m, "the type facts have no value for this enum member");
                            Expr::Nil
                        }
                    },
                };
                values.push((key.text(), value));
            }
            let numeric = values.iter().any(|(_, v)| !matches!(v, Expr::String(_)));
            let inverse = numeric.then(|| l.temp());
            let metatable = match &inverse {
                Some(inverse) => {
                    l.out.push(Stmt::Local(vec![inverse.clone()], vec![Expr::Table(Vec::new())]));
                    vec![Field::Keyed(Expr::string("__index"), Expr::Name(inverse.clone()))]
                }
                None => Vec::new(),
            };
            let table = Expr::global("setmetatable").call(vec![Expr::Table(Vec::new()), Expr::Table(metatable)]);
            l.out.push(Stmt::Assign(vec![Expr::Name(local.clone())], vec![table]));
            for (key, value) in values {
                let string = matches!(value, Expr::String(_));
                let simple = value.is_simple();
                let value = if simple || inverse.is_none() || string { value } else { l.push_temp(value) };
                l.out.push(Stmt::Assign(vec![Expr::Name(local.clone()).field(key)], vec![value.clone()]));
                if let Some(inverse) = &inverse
                    && !string
                {
                    l.out.push(Stmt::Assign(vec![Expr::Name(inverse.clone()).index(value)], vec![Expr::string(key)]));
                }
            }
        });
        self.out.push(Stmt::Do(block));
        if s.is_export() {
            self.export(local);
        }
    }

    fn namespace(&mut self, s: Node) {
        if s.is_ambient() {
            return;
        }
        let name = s.name().expect("a namespace has a name");
        if name.kind() != Kind::Identifier {
            return;
        }
        let Some(body) = s.body() else { return };
        if body.kind() != Kind::ModuleBlock {
            self.unsupported(body, "a dotted namespace");
            return;
        }
        let local = self.local_name(name);
        self.declare(name, local.clone(), Some(Expr::Table(Vec::new())));
        let outer = self.container.replace(local.clone());
        let block = self.block(body.statements());
        self.container = outer;
        self.out.push(Stmt::Do(block));
        if s.is_export() {
            self.export(local);
        }
    }

    /// `TS.import(script, <folder>, <names>...)` for a relative module specifier.
    fn import_call(&mut self, specifier: Node) -> Expr {
        let text = specifier.text();
        if !text.starts_with('.') {
            self.error(specifier, format!("`{text}` is a package import; only relative imports are supported"));
            return Expr::Nil;
        }
        let mut up = 0;
        let mut names: Vec<&'static str> = Vec::new();
        for segment in text.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    if names.pop().is_none() {
                        up += 1;
                    }
                }
                name => names.push(name),
            }
        }
        if names.last() == Some(&"index") {
            names.pop();
        }
        let mut base = Expr::global("script");
        if !self.init {
            base = base.field("Parent");
        }
        for _ in 0..up {
            base = base.field("Parent");
        }
        self.uses_ts = true;
        let mut args = vec![Expr::global("script"), base];
        args.extend(names.into_iter().map(|n| Expr::String(Cow::Borrowed(n))));
        Expr::global("TS.import").call(args)
    }

    fn import_kept(&mut self, name: Node) -> bool {
        match self.fact(name).and_then(|f| f.import) {
            Some(Import::Keep) => true,
            Some(Import::Elide) => false,
            None => {
                self.error(name, "the type facts do not say whether this import is a value");
                false
            }
        }
    }

    fn import_declaration(&mut self, s: Node) {
        let specifier = s.module_specifier().expect("an import has a module specifier");
        let Some(clause) = s.import_clause() else {
            let call = self.import_call(specifier);
            self.out.push(Stmt::Call(call));
            return;
        };
        if clause.is_type_only() {
            return;
        }
        let mut bindings: Vec<(Node, Option<&'static str>)> = Vec::new();
        if let Some(name) = clause.name() {
            bindings.push((name, Some("default")));
        }
        if let Some(named) = clause.named_bindings() {
            match named.kind() {
                Kind::NamespaceImport => bindings.push((named.name().unwrap(), None)),
                _ => {
                    for e in named.elements().iter() {
                        if e.is_type_only() {
                            continue;
                        }
                        let name = e.name().unwrap();
                        let key = e.property_name().unwrap_or(name);
                        bindings.push((name, Some(key.text())));
                    }
                }
            }
        }
        let kept: Vec<(Node, Option<&'static str>)> =
            bindings.into_iter().filter(|(n, _)| self.import_kept(*n)).collect();
        if kept.is_empty() {
            return;
        }
        let module = self.import_call(specifier);
        let module = if kept.len() == 1 { module } else { self.push_temp(module) };
        for (name, key) in kept {
            let local = self.local_name(name);
            let value = match key {
                Some(key) => module.clone().field(key),
                None => module.clone(),
            };
            self.declare(name, local, Some(value));
        }
    }

    fn export_declaration(&mut self, s: Node) {
        if s.is_type_only() {
            return;
        }
        let Some(clause) = s.export_clause() else {
            self.unsupported(s, "export *");
            return;
        };
        if clause.kind() != Kind::NamedExports {
            self.unsupported(s, "export * as");
            return;
        }
        let mut kept = Vec::new();
        for e in clause.elements().iter() {
            if e.is_type_only() {
                continue;
            }
            let name = e.name().unwrap();
            if self.import_kept(name) {
                let local = e.property_name().unwrap_or(name);
                kept.push((name.text(), local.text()));
            }
        }
        if kept.is_empty() {
            return;
        }
        match s.module_specifier() {
            Some(specifier) => {
                let module = self.import_call(specifier);
                let module = self.push_temp(module);
                for (name, key) in kept {
                    self.exports.push((Cow::Borrowed(name), module.clone().field(key)));
                }
            }
            None => {
                for (name, local) in kept {
                    self.exports.push((Cow::Borrowed(name), Expr::Name(Cow::Borrowed(local))));
                }
            }
        }
    }

    /// Lowers an expression whose value is unused.
    pub(super) fn expr_stmt(&mut self, n: Node) {
        let n = skip_down(n);
        match n.kind() {
            Kind::CallExpression => {
                if let Some(e) = self.call(n, true) {
                    self.discard(e);
                }
            }
            Kind::BinaryExpression if n.operator().is_assignment() => {
                self.assignment(n, false);
            }
            Kind::BinaryExpression if matches!(n.operator(), Op::And | Op::Or | Op::Coalesce) => {
                let right = n.right();
                let condition = match n.operator() {
                    Op::And => self.condition(n.left()),
                    Op::Or => self.condition(n.left()).negated(),
                    _ => {
                        let left = self.expr(n.left());
                        left.binary(BinOp::Eq, Expr::Nil)
                    }
                };
                let (_, block) = self.capture(|l| l.expr_stmt(right));
                self.out.push(Stmt::If(condition, block, None));
            }
            Kind::PrefixUnaryExpression | Kind::PostfixUnaryExpression
                if matches!(n.unary_operator(), Some(UnaryOp::Increment | UnaryOp::Decrement)) =>
            {
                self.increment(n, false);
            }
            Kind::ConditionalExpression => {
                let condition = self.condition(n.condition().unwrap());
                let (_, then) = self.capture(|l| l.expr_stmt(n.when_true()));
                let (_, otherwise) = self.capture(|l| l.expr_stmt(n.when_false()));
                self.out.push(Stmt::If(condition, then, Some(otherwise)));
            }
            Kind::Identifier | Kind::NumericLiteral | Kind::StringLiteral | Kind::TrueKeyword | Kind::FalseKeyword => {}
            _ => {
                let e = self.expr(n);
                self.discard(e);
            }
        }
    }

    /// Evaluates `e` for its effects.
    pub(super) fn discard(&mut self, e: Expr) {
        match e {
            Expr::Call(..) | Expr::Method(..) => self.out.push(Stmt::Call(e)),
            Expr::Paren(inner) if inner.is_multi() => self.out.push(Stmt::Call(*inner)),
            e if e.is_simple() => {}
            e => {
                let name = self.temp();
                self.out.push(Stmt::Local(vec![name], vec![e]));
            }
        }
    }
}

/// Whether a `continue` in `s` belongs to the loop around `s`.
fn has_continue(s: Node) -> bool {
    match s.kind() {
        Kind::ContinueStatement => true,
        Kind::ForStatement
        | Kind::ForOfStatement
        | Kind::ForInStatement
        | Kind::WhileStatement
        | Kind::DoStatement
        | Kind::FunctionDeclaration
        | Kind::FunctionExpression
        | Kind::ArrowFunction
        | Kind::MethodDeclaration
        | Kind::ClassDeclaration => false,
        _ => {
            let mut found = false;
            s.for_each_child(&mut |c| found = found || has_continue(c));
            found
        }
    }
}

/// Whether a clause's statements always leave the switch.
fn ends_flow(statements: Nodes) -> bool {
    statements.last().is_some_and(|s| match s.kind() {
        Kind::BreakStatement | Kind::ReturnStatement | Kind::ContinueStatement | Kind::ThrowStatement => true,
        Kind::Block => ends_flow(s.statements()),
        _ => false,
    })
}
