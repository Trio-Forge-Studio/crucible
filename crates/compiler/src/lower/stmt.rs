use std::borrow::Cow;

use super::{Breakable, Exit, ImportBase, Lower, is_hole, skip_parens_and_assertions};
use crate::facts::{Callee, Constant, Import, Receiver, RuntimeLib};
use crate::macros::Macro;
use crate::tree::{Kind, Node, Nodes, Op, UnaryOp};
use crucible_luau::{BinOp, Block, Expr, Field, Function, Name, Stmt, UnOp};

impl Lower<'_> {
    pub(super) fn module(&mut self, statements: Nodes) -> Block {
        self.statements(statements);
        let mut body = std::mem::take(&mut self.current_block);
        let exports = std::mem::take(&mut self.exports);
        let value = if let Some(value) = self.export_assignment.take() {
            value
        } else if exports.is_empty() && self.export_stars.is_empty() {
            Expr::Nil
        } else {
            Expr::Table(exports.into_iter().map(|(name, value)| Field::Keyed(Expr::String(name), value)).collect())
        };
        let value = if self.export_stars.is_empty() {
            value
        } else {
            let name = self.temp();
            body.push(Stmt::Local(vec![name.clone()], vec![value]));
            for module in std::mem::take(&mut self.export_stars) {
                let (key, item) = (self.temp(), self.temp());
                let k = Expr::Name(key.clone());
                let target = Expr::Name(name.clone()).index(k.clone());
                let condition = k
                    .binary(BinOp::Ne, Expr::string("default"))
                    .binary(BinOp::And, target.clone().binary(BinOp::Eq, Expr::Nil));
                body.push(Stmt::GenericFor(
                    vec![key, item.clone()],
                    vec![module],
                    vec![Stmt::If(condition, vec![Stmt::Assign(vec![target], vec![Expr::Name(item)])], None)],
                ));
            }
            Expr::Name(name)
        };
        body.push(Stmt::Return(vec![value]));
        for local in std::mem::take(&mut self.helpers_used).into_iter().rev() {
            let Some(module) = self.place.helpers.iter().find(|m| m.local == local) else {
                let root = self.file.root();
                self.error(root, format!("no extension declares the helper module {local}"));
                continue;
            };
            match &module.path {
                Some(path) => {
                    self.uses_ts = true;
                    let import = Expr::global("TS.import").call(super::import_arguments(path));
                    body.insert(0, Stmt::Local(vec![Cow::Borrowed(local)], vec![import]));
                }
                None => {
                    let root = self.file.root();
                    self.error(root, module.missing.to_string());
                }
            }
        }
        if self.uses_ui {
            match &self.place.crucible_ui {
                Some(ui) => {
                    self.uses_ts = true;
                    let import = Expr::global("TS.import").call(super::import_arguments(ui));
                    body.insert(0, Stmt::Local(vec![Cow::Borrowed("__ui")], vec![import]));
                }
                None => {
                    let root = self.file.root();
                    self.error(root, "Crucible UI's JSX needs @trioforge/crucible/ui where the program can reach it");
                }
            }
        }
        if self.uses_ts {
            let runtime = match &self.place.runtime_lib {
                RuntimeLib::Global => Expr::global("_G").index(Expr::global("script")),
                RuntimeLib::Require(path) => Expr::global("require").call(vec![super::instance_at(path)]),
            };
            body.insert(0, Stmt::Local(vec![Cow::Borrowed("TS")], vec![runtime]));
        }
        body
    }

    pub(super) fn stmt(&mut self, s: Node) {
        match s.kind() {
            Kind::Block => {
                let block = self.block(s.statements());
                self.current_block.push(Stmt::Do(block));
            }
            Kind::EmptyStatement | Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration => {}
            Kind::VariableStatement => self.variable_statement(s),
            Kind::ExpressionStatement => self.expr_stmt(s.expression()),
            Kind::IfStatement => self.if_statement(s),
            Kind::ReturnStatement => self.return_statement(s),
            Kind::ForStatement => self.for_statement(s),
            Kind::ForOfStatement => self.for_of(s),
            Kind::WhileStatement => self.while_statement(s),
            Kind::DoStatement => self.do_statement(s),
            Kind::BreakStatement => {
                if s.has_label() {
                    self.error(s, "labeled break is not supported");
                }
                self.leave(Exit::Break, Vec::new());
            }
            Kind::ContinueStatement => {
                if s.has_label() {
                    self.error(s, "labeled continue is not supported");
                }
                let target = self.breakables.iter().rev().find(|b| !matches!(b, Breakable::Try { .. }));
                if target == Some(&Breakable::Switch) {
                    self.error(s, "continue inside a switch is not supported");
                }
                self.leave(Exit::Continue, Vec::new());
            }
            Kind::SwitchStatement => self.switch_statement(s),
            Kind::FunctionDeclaration => self.function_declaration(s),
            Kind::ClassDeclaration => self.class_declaration(s),
            Kind::EnumDeclaration => self.enum_declaration(s),
            Kind::ModuleDeclaration => self.namespace(s),
            Kind::ImportDeclaration => self.import_declaration(s),
            Kind::ExportDeclaration => self.export_declaration(s),
            Kind::ThrowStatement => {
                let value = match s.expression_opt() {
                    Some(e) => self.expr(e),
                    None => Expr::Nil,
                };
                self.current_block.push(Stmt::Call(throw(value)));
            }
            Kind::TryStatement => self.try_statement(s),
            Kind::LabeledStatement => {
                self.unsupported(s, "a labeled statement");
            }
            Kind::ExportAssignment => {
                let value = self.expr(s.expression());
                if s.is_export_equals() {
                    self.export_assignment = Some(value);
                } else {
                    let value = self.push_temp(value);
                    self.exports.push((Cow::Borrowed("default"), value));
                }
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

    pub(super) fn body_block(&mut self, s: Node) -> Block {
        if s.kind() == Kind::Block {
            self.block(s.statements())
        } else {
            let (_, block) = self.capture(|l| l.stmt(s));
            block
        }
    }

    pub(super) fn export(&mut self, name: Name) {
        match &self.namespace {
            Some(namespace) => {
                let target = Expr::Name(namespace.clone()).field(name.clone());
                self.current_block.push(Stmt::Assign(vec![target], vec![Expr::Name(name)]));
            }
            None => self.exports.push((name.clone(), Expr::Name(name))),
        }
    }

    pub(super) fn declare(&mut self, name_node: Node, name: Name, value: Option<Expr>) {
        if self.is_hoisted(name_node) {
            if let Some(value) = value {
                self.current_block.push(Stmt::Assign(vec![Expr::Name(name)], vec![value]));
            }
        } else {
            self.current_block.push(Stmt::Local(vec![name], value.into_iter().collect()));
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
        self.current_block.push(Stmt::If(condition, then, otherwise));
    }

    /// The values a `return` or an arrow body returns. A `LuaTuple` returns its members as several values.
    pub(super) fn return_values(&mut self, e: Node) -> Vec<Expr> {
        let inner = skip_parens_and_assertions(e);
        if inner.kind() == Kind::CallExpression
            && self.fact(inner.expression()).and_then(|f| f.callee) == Some(Callee::Macro(Macro::Tuple))
        {
            return self.ordered(inner.arguments().iter(), |l, a| l.expr(a));
        }
        match self.fact(e).and_then(|f| f.receiver) {
            Some(Receiver::LuaTuple) => {}
            Some(_) => return vec![self.expr(e)],
            None => {
                self.error(e, "the type facts do not say whether this returns a LuaTuple");
                return vec![self.expr(e)];
            }
        }
        match inner.kind() {
            Kind::CallExpression => vec![self.call(inner)],
            Kind::ArrayLiteralExpression if !inner.elements().iter().any(|e| e.kind() == Kind::SpreadElement) => {
                self.ordered(inner.elements().iter(), |l, a| l.expr(a))
            }
            _ => {
                let value = self.expr(e);
                vec![Expr::global("table.unpack").call(vec![value])]
            }
        }
    }

    fn return_statement(&mut self, s: Node) {
        let Some(e) = s.expression_opt() else {
            return self.leave(Exit::ReturnNothing, Vec::new());
        };
        let values = self.return_values(e);
        let several = values.len() > 1
            || (values.last().is_some_and(Expr::is_multi)
                && self.fact(e).and_then(|f| f.receiver) == Some(Receiver::LuaTuple));
        let exit = if values.is_empty() {
            Exit::ReturnNothing
        } else if several {
            Exit::ReturnSeveral
        } else {
            Exit::Return
        };
        self.leave(exit, values);
    }

    /// Lowers a `return`, `break` or `continue`. One that leaves the function of a `try` or `catch` body returns the
    /// exit's code from that function instead, and the code after its `pcall` leaves the same way.
    fn leave(&mut self, exit: Exit, values: Vec<Expr>) {
        let mut crossed = None;
        for b in self.breakables.iter_mut().rev() {
            match b {
                Breakable::Try { exits } => {
                    crossed = Some(exits);
                    break;
                }
                Breakable::Loop { .. } | Breakable::Switch if matches!(exit, Exit::Break | Exit::Continue) => break,
                Breakable::Loop { .. } | Breakable::Switch => {}
            }
        }
        if let Some(exits) = crossed {
            if !exits.contains(&exit) {
                exits.push(exit);
            }
            let mut returned = vec![Expr::Number(f64::from(exit as u8))];
            match exit {
                Exit::Return => returned.extend(values),
                Exit::ReturnSeveral => returned.push(Expr::global("table.pack").call(values)),
                Exit::Break | Exit::Continue | Exit::ReturnNothing => {}
            }
            self.current_block.push(Stmt::Return(returned));
            return;
        }
        match exit {
            Exit::Return | Exit::ReturnSeveral | Exit::ReturnNothing => self.current_block.push(Stmt::Return(values)),
            Exit::Break => self.current_block.push(Stmt::Break),
            Exit::Continue => {
                if let Some(Breakable::Loop { copy_back }) = self.breakables.last() {
                    let copy_back = copy_back.clone();
                    self.current_block.extend(copy_back);
                }
                self.current_block.push(Stmt::Continue);
            }
        }
    }

    /// `pcall` runs the `try` body in a function. Without a `finally` body the `catch` body runs inline after it. With
    /// one, the `catch` body runs in a second `pcall` and the `finally` body runs inline after both, so a `return`,
    /// `break` or throw in it replaces the pending one, as in JavaScript.
    fn try_statement(&mut self, s: Node) {
        let mut exits = Vec::new();
        let body = self.guarded(Vec::new(), &mut exits, |l| l.statements(s.try_block().statements()));
        let ok = self.temp();
        let thrown = self.temp();
        let value = self.temp();
        let failed = Expr::unary(UnOp::Not, Expr::Name(ok.clone()));
        if let (Some(clause), None) = (s.catch_clause(), s.finally_block()) {
            let (_, handler) = self.capture(|l| {
                if let Some(name) = clause.catch_variable().and_then(|d| d.name()) {
                    l.bind_value(name, Expr::Name(thrown.clone()));
                }
                l.statements(clause.catch_block().statements());
            });
            let dispatch = self.repeat_exit(&exits, &thrown, &value);
            if handler.is_empty() && dispatch.is_empty() {
                self.current_block.push(Stmt::Call(pcall(vec![body])));
                return;
            }
            let names = pcall_results(&exits, ok, thrown, value);
            self.current_block.push(Stmt::Local(names, vec![pcall(vec![body])]));
            self.current_block.push(Stmt::If(failed, handler, (!dispatch.is_empty()).then_some(dispatch)));
            return;
        }
        let retry = s.catch_clause().map(|clause| self.catch_call(clause, &thrown, &mut exits));
        let names = pcall_results(&exits, ok, thrown.clone(), value.clone());
        let targets = names.iter().cloned().map(Expr::Name).collect();
        self.current_block.push(Stmt::Local(names, vec![pcall(vec![body])]));
        if let Some(call) = retry {
            self.current_block.push(Stmt::If(failed.clone(), vec![Stmt::Assign(targets, vec![call])], None));
        }
        if let Some(finally) = s.finally_block() {
            let block = self.block(finally.statements());
            self.current_block.extend(scoped(block));
        }
        self.current_block.push(Stmt::If(failed, vec![Stmt::Call(throw(Expr::Name(thrown.clone())))], None));
        let dispatch = self.repeat_exit(&exits, &thrown, &value);
        self.current_block.extend(dispatch);
    }

    /// `pcall` on the `catch` body's function, which takes the thrown value when the clause has a variable.
    fn catch_call(&mut self, clause: Node, thrown: &Name, exits: &mut Vec<Exit>) -> Expr {
        let variable = clause.catch_variable().and_then(|d| d.name());
        let param =
            variable.map(|name| if name.kind() == Kind::Identifier { self.local_name(name) } else { self.temp() });
        let function = self.guarded(param.iter().cloned().collect(), exits, |l| {
            if let (Some(pattern), Some(param)) = (variable, &param)
                && pattern.kind() != Kind::Identifier
            {
                l.bind_value(pattern, Expr::Name(param.clone()));
            }
            l.statements(clause.catch_block().statements());
        });
        let mut args = vec![function];
        if param.is_some() {
            args.push(Expr::Name(thrown.clone()));
        }
        pcall(args)
    }

    /// Lowers a `try` or `catch` body as a function for `pcall`, and adds the exits that leave it to `exits`.
    fn guarded(&mut self, params: Vec<Name>, exits: &mut Vec<Exit>, body: impl FnOnce(&mut Self)) -> Expr {
        self.breakables.push(Breakable::Try { exits: Vec::new() });
        let (_, block) = self.capture(body);
        if let Some(Breakable::Try { exits: left }) = self.breakables.pop() {
            for exit in left {
                if !exits.contains(&exit) {
                    exits.push(exit);
                }
            }
        }
        Expr::Function(Box::new(Function { params, varargs: false, body: block }))
    }

    /// Leaves the way a guarded function said it left, from the exit's code in `code` and the returned value in
    /// `value`.
    fn repeat_exit(&mut self, exits: &[Exit], code: &Name, value: &Name) -> Block {
        let mut chain = None;
        for exit in Exit::ALL.into_iter().rev().filter(|exit| exits.contains(exit)) {
            let values = match exit {
                Exit::Return => vec![Expr::Name(value.clone())],
                Exit::ReturnSeveral => {
                    let packed = Expr::Name(value.clone());
                    vec![Expr::global("table.unpack").call(vec![packed.clone(), Expr::Number(1.0), packed.field("n")])]
                }
                Exit::Break | Exit::Continue | Exit::ReturnNothing => Vec::new(),
            };
            let (_, then) = self.capture(|l| l.leave(exit, values));
            let condition = Expr::Name(code.clone()).binary(BinOp::Eq, Expr::Number(f64::from(exit as u8)));
            chain = Some(vec![Stmt::If(condition, then, chain)]);
        }
        chain.unwrap_or_default()
    }

    fn with_breakable<T>(&mut self, kind: Breakable, f: impl FnOnce(&mut Self) -> T) -> T {
        self.breakables.push(kind);
        let value = f(self);
        self.breakables.pop();
        value
    }

    fn while_statement(&mut self, s: Node) {
        let (condition, prereqs) = self.capture(|l| l.condition(s.expression()));
        let mut body = self.with_breakable(Breakable::LOOP, |l| l.body_block(s.loop_body()));
        if prereqs.is_empty() {
            self.current_block.push(Stmt::While(condition, body));
        } else {
            let mut block = prereqs;
            block.push(Stmt::If(condition.negated(), vec![Stmt::Break], None));
            block.append(&mut body);
            self.current_block.push(Stmt::While(Expr::Bool(true), block));
        }
    }

    fn do_statement(&mut self, s: Node) {
        let body = self.with_breakable(Breakable::LOOP, |l| l.body_block(s.loop_body()));
        let (condition, prereqs) = self.capture(|l| l.condition(s.expression()));
        if !prereqs.is_empty() && has_own_continue(s.loop_body()) {
            self.error(s, "continue in a do-while whose condition needs statements is not supported");
        }
        let mut body = scoped(body);
        body.extend(prereqs);
        self.current_block.push(Stmt::Repeat(body, condition.negated()));
    }

    fn captured_head_names(&self, s: Node) -> Vec<Node> {
        let Some(init) = s.initializer() else { return Vec::new() };
        if init.kind() != Kind::VariableDeclarationList || init.is_const() {
            return Vec::new();
        }
        let mut names = Vec::new();
        for d in init.declarations().iter() {
            if let Some(name) = d.name() {
                super::pattern_names(name, &mut |n| names.push(n));
            }
        }
        let parts = [s.condition(), s.incrementor(), Some(s.loop_body())];
        names.retain(|&name| parts.iter().flatten().any(|&part| closure_refers_to(part, name)));
        names
    }

    /// JavaScript gives each pass of a `for` its own copy of the `let` variables in its head. When a closure
    /// captures one, each pass declares its own local from a relay, and stores it back at the end of the pass and
    /// before each `continue`.
    fn for_statement(&mut self, s: Node) {
        let captured = self.captured_head_names(s);
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
        let mut copy_in = Vec::new();
        let mut copy_back = Vec::new();
        if !captured.is_empty() {
            let names: Vec<Name> = captured.iter().map(|n| Cow::Borrowed(n.text())).collect();
            let relays: Vec<Name> = names.iter().map(|_| self.temp()).collect();
            let as_exprs = |names: &[Name]| names.iter().cloned().map(Expr::Name).collect::<Vec<_>>();
            block.push(Stmt::Local(relays.clone(), as_exprs(&names)));
            copy_in.push(Stmt::Local(names.clone(), as_exprs(&relays)));
            copy_back.push(Stmt::Assign(as_exprs(&relays), as_exprs(&names)));
        }
        let (condition, prereqs) = match s.condition() {
            Some(c) => self.capture(|l| l.condition(c)),
            None => (Expr::Bool(true), Vec::new()),
        };
        let increment = match s.incrementor() {
            Some(i) => self.capture(|l| l.expr_stmt(i)).1,
            None => Vec::new(),
        };
        let copy_at_continue = if continue_hides(s.loop_body(), &captured) {
            let store = self.temp();
            let function = Function { params: Vec::new(), varargs: false, body: copy_back.clone() };
            copy_in.push(Stmt::LocalFunction(store.clone(), function));
            vec![Stmt::Call(Expr::Name(store).call(Vec::new()))]
        } else {
            copy_back.clone()
        };
        let loop_kind = Breakable::Loop { copy_back: copy_at_continue };
        let mut body = self.with_breakable(loop_kind, |l| l.body_block(s.loop_body()));
        if !copy_in.is_empty() || (has_own_continue(s.loop_body()) && !increment.is_empty()) {
            // `continue` jumps to the loop's start, so the increment runs there on every pass but the first.
            let mut inner = copy_in;
            if !increment.is_empty() {
                let flag = self.temp();
                block.push(Stmt::Local(vec![flag.clone()], vec![Expr::Bool(false)]));
                inner.push(Stmt::If(
                    Expr::Name(flag.clone()),
                    increment,
                    Some(vec![Stmt::Assign(vec![Expr::Name(flag)], vec![Expr::Bool(true)])]),
                ));
            }
            inner.extend(prereqs);
            if condition != Expr::Bool(true) {
                inner.push(Stmt::If(condition.negated(), vec![Stmt::Break], None));
            }
            if copy_back.is_empty() {
                inner.append(&mut body);
            } else {
                inner.extend(scoped(body));
                inner.extend(copy_back);
            }
            block.push(Stmt::While(Expr::Bool(true), inner));
        } else if prereqs.is_empty() {
            let mut body = if increment.is_empty() { body } else { scoped(body) };
            body.extend(increment);
            block.push(Stmt::While(condition, body));
        } else {
            let mut inner = prereqs;
            inner.push(Stmt::If(condition.negated(), vec![Stmt::Break], None));
            inner.extend(if increment.is_empty() { body } else { scoped(body) });
            inner.extend(increment);
            block.push(Stmt::While(Expr::Bool(true), inner));
        }
        self.current_block.push(Stmt::Do(block));
    }

    fn for_of(&mut self, s: Node) {
        let iterable = s.expression();
        let Some(initializer) = s.initializer() else {
            self.unsupported(s, "this for-of");
            return;
        };
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
        let inner = skip_parens_and_assertions(iterable);
        if inner.kind() == Kind::CallExpression
            && self.fact(inner.expression()).and_then(|f| f.callee) == Some(Callee::Macro(Macro::Range))
        {
            return self.for_range(s, inner, binding);
        }
        let Some(receiver) = self.fact(iterable).and_then(|f| f.receiver) else {
            self.error(iterable, "the type facts do not say how to iterate this value");
            return;
        };
        let receiver = if receiver == Receiver::LuaTuple { Receiver::Array } else { receiver };
        let body = s.loop_body().span();
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
                } else if binding.kind() == Kind::Identifier
                    && !self.referenced_between(binding.text(), body.start, body.end, u32::MAX)
                {
                    (vec![self.temp()], vec![source], None)
                } else {
                    let length = match receiver {
                        Receiver::Map => Some(2),
                        _ => self.fact(iterable).and_then(|f| f.step_length),
                    };
                    let Some(length) = length else {
                        self.unsupported(
                            binding,
                            "binding a whole step of a LuaTuple iterator whose length is not fixed",
                        );
                        return;
                    };
                    let values: Vec<Name> = (0..length.max(1)).map(|_| self.temp()).collect();
                    let step = Expr::Table(values.iter().map(|v| Field::Item(Expr::Name(v.clone()))).collect());
                    (values, vec![source], Some((binding, step)))
                }
            }
            Receiver::String => {
                let name = match simple_name(self, binding) {
                    Some(name) => name,
                    None => {
                        self.unsupported(binding, "destructuring a string character");
                        return;
                    }
                };
                let pattern = Expr::global("utf8.charpattern");
                (vec![name], vec![Expr::global("string.gmatch").call(vec![source, pattern])], None)
            }
            Receiver::LuaTuple | Receiver::Other => {
                self.unsupported(iterable, "iterating this value");
                return;
            }
        };
        let body = self.with_breakable(Breakable::LOOP, |l| {
            let (_, mut block) = l.capture(|l| {
                if let Some((pattern, value)) = bind {
                    l.bind_value(pattern, value);
                }
            });
            block.extend(l.body_block(s.loop_body()));
            block
        });
        self.current_block.push(Stmt::GenericFor(names, iter, body));
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
        let body = self.with_breakable(Breakable::LOOP, |l| l.body_block(s.loop_body()));
        self.current_block.push(Stmt::NumericFor(name, start, end, step, body));
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
            .any(|(i, c)| i + 1 < clauses.len() && !c.statements().is_empty() && !clause_always_exits(c.statements()));
        let fallthrough = falls.then(|| self.temp());
        let body = self.with_breakable(Breakable::Switch, |l| {
            let (_, block) = l.capture(|l| {
                if let Some(flag) = &fallthrough {
                    l.current_block.push(Stmt::Local(vec![flag.clone()], vec![Expr::Bool(false)]));
                }
                let mut tests: Vec<Expr> = Vec::new();
                let mut after_fallthrough = false;
                for clause in &clauses {
                    if clause.kind() == Kind::DefaultClause {
                        let statements = l.block(clause.statements());
                        l.current_block.extend(statements);
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
                    let Some(mut condition) = tests.drain(..).reduce(|a, b| a.binary(BinOp::Or, b)) else {
                        continue;
                    };
                    if after_fallthrough && let Some(flag) = &fallthrough {
                        condition = Expr::Name(flag.clone()).binary(BinOp::Or, condition);
                    }
                    let mut block = l.block(clause.statements());
                    let ends = clause_always_exits(clause.statements());
                    if !ends && let Some(flag) = &fallthrough {
                        block.push(Stmt::Assign(vec![Expr::Name(flag.clone())], vec![Expr::Bool(true)]));
                    }
                    after_fallthrough = !ends;
                    l.current_block.push(Stmt::If(condition, block, None));
                }
            });
            block
        });
        self.current_block.push(Stmt::Repeat(body, Expr::Bool(true)));
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
            self.current_block.push(Stmt::Function(Expr::Name(local.clone()), None, function));
        } else {
            self.current_block.push(Stmt::LocalFunction(local.clone(), function));
        }
        if s.is_export() {
            self.export(local);
        }
    }

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
        let namespace = self.namespace.take();
        let (Parameters { names: mut params, rest: varargs }, body) = self.capture(|l| {
            let params = l.parameters(f.parameters());
            match f.body() {
                Some(body) if body.kind() == Kind::Block => l.statements(body.statements()),
                Some(body) => {
                    let values = l.return_values(body);
                    l.current_block.push(Stmt::Return(values));
                }
                None => {}
            }
            params
        });
        self.breakables = outer;
        self.namespace = namespace;
        if method {
            params.insert(0, Cow::Borrowed("self"));
        }
        Function { params, varargs, body }
    }

    pub(super) fn parameters(&mut self, params: Nodes) -> Parameters {
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
                    self.current_block.push(Stmt::Local(vec![local], vec![rest]));
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
                self.current_block.push(Stmt::If(Expr::Name(local.clone()).binary(BinOp::Eq, Expr::Nil), block, None));
            }
            if name.kind() != Kind::Identifier {
                self.bind_value(name, Expr::Name(local));
            }
        }
        Parameters { names, rest: varargs }
    }

    fn enum_declaration(&mut self, s: Node) {
        if s.is_ambient() {
            return;
        }
        let Some(name) = s.name() else {
            self.unsupported(s, "an anonymous enum");
            return;
        };
        let local = self.local_name(name);
        if !self.is_hoisted(name) {
            self.current_block.push(Stmt::Local(vec![local.clone()], Vec::new()));
        }
        let members: Vec<Node> = s.members().iter().collect();
        let (_, block) = self.capture(|l| {
            let mut values = Vec::new();
            for m in &members {
                let Some(key) = m.name() else {
                    l.unsupported(*m, "this enum member");
                    continue;
                };
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
                    l.current_block.push(Stmt::Local(vec![inverse.clone()], vec![Expr::Table(Vec::new())]));
                    vec![Field::Keyed(Expr::string("__index"), Expr::Name(inverse.clone()))]
                }
                None => Vec::new(),
            };
            let table = Expr::global("setmetatable").call(vec![Expr::Table(Vec::new()), Expr::Table(metatable)]);
            l.current_block.push(Stmt::Assign(vec![Expr::Name(local.clone())], vec![table]));
            for (key, value) in values {
                let string = matches!(value, Expr::String(_));
                let simple = value.is_simple();
                let value = if simple || inverse.is_none() || string { value } else { l.push_temp(value) };
                l.current_block.push(Stmt::Assign(vec![Expr::Name(local.clone()).field(key)], vec![value.clone()]));
                if let Some(inverse) = &inverse
                    && !string
                {
                    l.current_block
                        .push(Stmt::Assign(vec![Expr::Name(inverse.clone()).index(value)], vec![Expr::string(key)]));
                }
            }
        });
        self.current_block.push(Stmt::Do(block));
        if s.is_export() {
            self.export(local);
        }
    }

    fn namespace(&mut self, s: Node) {
        if s.is_ambient() {
            return;
        }
        let Some(name) = s.name() else {
            self.unsupported(s, "an anonymous namespace");
            return;
        };
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
        let outer = self.namespace.replace(local.clone());
        let block = self.block(body.statements());
        self.namespace = outer;
        self.current_block.push(Stmt::Do(block));
        if s.is_export() {
            self.export(local);
        }
    }

    /// `TS.import(script, <folder>, <names>...)`, from the module's place path in a game, or else relative to the
    /// importing module.
    fn import_call(&mut self, specifier: Node) -> Expr {
        if let Some(path) = self.fact(specifier).and_then(|f| f.module.as_deref()) {
            self.uses_ts = true;
            return Expr::global("TS.import").call(super::import_arguments(path));
        }
        let text = specifier.text();
        if !text.starts_with('.') {
            self.error(
                specifier,
                format!(
                    "`{text}` is not a relative import or a scoped package under node_modules, which a package needs"
                ),
            );
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
        if self.import_base == ImportBase::ScriptParent {
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

    fn import_fact(&mut self, name: Node) -> Import {
        match self.fact(name).and_then(|f| f.import) {
            Some(import) => import,
            None => {
                self.error(name, "the type facts do not say whether this import is a value");
                Import::Elide
            }
        }
    }

    fn import_declaration(&mut self, s: Node) {
        let Some(specifier) = s.module_specifier() else {
            self.unsupported(s, "this import");
            return;
        };
        let Some(clause) = s.import_clause() else {
            let call = self.import_call(specifier);
            self.current_block.push(Stmt::Call(call));
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
                Kind::NamespaceImport => {
                    if let Some(name) = named.name() {
                        bindings.push((name, None));
                    }
                }
                _ => {
                    for e in named.elements().iter() {
                        if e.is_type_only() {
                            continue;
                        }
                        let Some(name) = e.name() else { continue };
                        let key = e.property_name().unwrap_or(name);
                        bindings.push((name, Some(key.text())));
                    }
                }
            }
        }
        let kept: Vec<(Node, Option<&'static str>)> = bindings
            .into_iter()
            .filter_map(|(n, key)| match self.import_fact(n) {
                Import::Keep => Some((n, key)),
                Import::KeepModule => Some((n, None)),
                Import::Elide => None,
            })
            .collect();
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
            if let Some(specifier) = s.module_specifier() {
                if !self.import_fact(specifier).kept() {
                    return;
                }
                let module = self.import_call(specifier);
                let module = self.push_temp(module);
                self.export_stars.push(module);
            }
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
            let Some(name) = e.name() else { continue };
            if self.import_fact(name).kept() {
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

    pub(super) fn expr_stmt(&mut self, n: Node) {
        let n = skip_parens_and_assertions(n);
        match n.kind() {
            Kind::CallExpression => {
                self.call_statement(n);
            }
            Kind::BinaryExpression if n.operator().is_assignment() => {
                self.assignment(n);
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
                self.current_block.push(Stmt::If(condition, block, None));
            }
            Kind::PrefixUnaryExpression | Kind::PostfixUnaryExpression
                if matches!(n.unary_operator(), Some(UnaryOp::Increment | UnaryOp::Decrement)) =>
            {
                self.increment(n);
            }
            Kind::ConditionalExpression => {
                let Some(test) = n.condition() else {
                    self.unsupported(n, "this conditional");
                    return;
                };
                let condition = self.condition(test);
                let (_, then) = self.capture(|l| l.expr_stmt(n.when_true()));
                let (_, otherwise) = self.capture(|l| l.expr_stmt(n.when_false()));
                self.current_block.push(Stmt::If(condition, then, Some(otherwise)));
            }
            Kind::Identifier | Kind::NumericLiteral | Kind::StringLiteral | Kind::TrueKeyword | Kind::FalseKeyword => {}
            _ => {
                let e = self.expr(n);
                self.discard(e);
            }
        }
    }

    pub(super) fn discard(&mut self, e: Expr) {
        match e {
            Expr::Call(..) | Expr::Method(..) => self.current_block.push(Stmt::Call(e)),
            Expr::Paren(inner) if inner.is_multi() => self.current_block.push(Stmt::Call(*inner)),
            e if e.is_simple() => {}
            e => {
                let name = self.temp();
                self.current_block.push(Stmt::Local(vec![name], vec![e]));
            }
        }
    }
}

pub(super) struct Parameters {
    pub(super) names: Vec<Name>,
    pub(super) rest: bool,
}

/// Level 0 keeps a thrown string as it is, without the position Luau would prepend, so a `catch` sees the value the
/// `throw` gave.
fn throw(value: Expr) -> Expr {
    Expr::global("error").call(vec![value, Expr::Number(0.0)])
}

fn pcall(args: Vec<Expr>) -> Expr {
    Expr::global("pcall").call(args)
}

/// The names `pcall`'s results go to: whether the body finished, then the thrown value or the exit's code, then the
/// returned value when a `return` left the body.
fn pcall_results(exits: &[Exit], ok: Name, thrown: Name, value: Name) -> Vec<Name> {
    let mut names = vec![ok, thrown];
    if exits.iter().any(|exit| matches!(exit, Exit::Return | Exit::ReturnSeveral)) {
        names.push(value);
    }
    names
}

fn has_own_continue(s: Node) -> bool {
    any_own_continue(s, &|_| true)
}

/// Whether some `continue` of the loop whose body is `s` matches `test`.
fn any_own_continue(s: Node, test: &dyn Fn(Node) -> bool) -> bool {
    match s.kind() {
        Kind::ContinueStatement => test(s),
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
            s.for_each_child(&mut |c| found = found || any_own_continue(c, test));
            found
        }
    }
}

/// Whether a `continue` in `body` sits where a declaration in the body hides one of the head's `names`, so a store
/// written there would read the body's variable.
fn continue_hides(body: Node, names: &[Node]) -> bool {
    !names.is_empty() && any_own_continue(body, &|c| names.iter().any(|name| c.lookup(name.text()) != name.parent()))
}

/// Whether a function inside `n` refers to the variable that the identifier `name` declares.
fn closure_refers_to(n: Node, name: Node) -> bool {
    if matches!(n.kind(), Kind::ClassDeclaration | Kind::ClassExpression) || super::runs_later(n) {
        return refers_to(n, name);
    }
    let mut found = false;
    n.for_each_child(&mut |c| found = found || closure_refers_to(c, name));
    found
}

fn refers_to(n: Node, name: Node) -> bool {
    if n.kind() == Kind::Identifier {
        return n.text() == name.text() && super::is_reference(n) && n.declaration() == name.parent();
    }
    let mut found = false;
    n.for_each_child(&mut |c| found = found || refers_to(c, name));
    found
}

/// Wraps a loop body that declares locals in `do ... end`, so the statements after it in the loop see the names
/// the loop's head declared.
fn scoped(body: Block) -> Block {
    if body.iter().any(|s| matches!(s, Stmt::Local(..) | Stmt::LocalFunction(..))) {
        vec![Stmt::Do(body)]
    } else {
        body
    }
}

fn clause_always_exits(statements: Nodes) -> bool {
    statements.last().is_some_and(|s| match s.kind() {
        Kind::BreakStatement | Kind::ReturnStatement | Kind::ContinueStatement | Kind::ThrowStatement => true,
        Kind::Block => clause_always_exits(s.statements()),
        _ => false,
    })
}
