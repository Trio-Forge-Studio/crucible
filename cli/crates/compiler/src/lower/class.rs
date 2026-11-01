use std::borrow::Cow;

use super::Lower;
use crate::luau::{BinOp, Expr, Field, Function, Name, Stmt};
use crate::tree::{Kind, Node};

/// Whether `n` uses `this` outside a function that has its own.
fn uses_this(n: Node) -> bool {
    match n.kind() {
        Kind::ThisKeyword => true,
        Kind::FunctionExpression | Kind::FunctionDeclaration | Kind::MethodDeclaration | Kind::ClassDeclaration => {
            false
        }
        _ => {
            let mut found = false;
            n.for_each_child(&mut |c| found = found || uses_this(c));
            found
        }
    }
}

fn is_super_call(s: Node) -> bool {
    s.kind() == Kind::ExpressionStatement && {
        let e = super::skip_down(s.expression());
        e.kind() == Kind::CallExpression && e.expression().kind() == Kind::SuperKeyword
    }
}

impl Lower<'_> {
    /// Lowers a class to roblox-ts's class shape, which the sample game's `rbxtsc` output extends: a table whose
    /// metatable's `__index` is the base class, a `constructor` method, and a `new` function unless abstract.
    pub(super) fn class_declaration(&mut self, s: Node) {
        if s.is_ambient() {
            return;
        }
        if s.has_decorators() {
            self.unsupported(s, "a decorator");
        }
        if s.is_default() {
            self.unsupported(s, "export default");
        }
        let Some(name) = s.name() else {
            self.unsupported(s, "an anonymous class");
            return;
        };
        let local = self.local_name(name);
        if !self.is_hoisted(name) {
            self.out.push(Stmt::Local(vec![local.clone()], Vec::new()));
        }
        let class = Expr::Name(local.clone());
        let abstract_ = s.is_abstract();
        let extends = s.extends();
        let members: Vec<Node> = s.members().iter().collect();
        let outer_container = self.container.take();
        let (_, block) = self.capture(|l| {
            if abstract_ && extends.is_none() {
                l.out.push(Stmt::Assign(vec![class.clone()], vec![Expr::Table(Vec::new())]));
            } else {
                let tostring = Function {
                    params: Vec::new(),
                    varargs: false,
                    body: vec![Stmt::Return(vec![Expr::String(local.clone())])],
                };
                let mut metatable = vec![Field::Keyed(Expr::string("__tostring"), Expr::Function(Box::new(tostring)))];
                if let Some(base) = extends {
                    let base = l.expr(base);
                    l.out.push(Stmt::Local(vec![Cow::Borrowed("super")], vec![base]));
                    metatable.push(Field::Keyed(Expr::string("__index"), Expr::global("super")));
                }
                let table = Expr::global("setmetatable").call(vec![Expr::Table(Vec::new()), Expr::Table(metatable)]);
                l.out.push(Stmt::Assign(vec![class.clone()], vec![table]));
                l.out.push(Stmt::Assign(vec![class.clone().field("__index")], vec![class.clone()]));
            }
            if !abstract_ {
                let construct = Expr::global("self").method("constructor", vec![Expr::VarArgs]);
                let body = vec![
                    Stmt::Local(
                        vec![Cow::Borrowed("self")],
                        vec![Expr::global("setmetatable").call(vec![Expr::Table(Vec::new()), class.clone()])],
                    ),
                    Stmt::Return(vec![construct.binary(BinOp::Or, Expr::global("self"))]),
                ];
                let new = Function { params: Vec::new(), varargs: true, body };
                l.out.push(Stmt::Function(class.clone().field("new"), None, new));
            }
            let constructor = members.iter().copied().find(|m| m.kind() == Kind::Constructor && m.body().is_some());
            l.constructor(&class, s, constructor, extends.is_some());
            let mut has_to_string = false;
            for &m in &members {
                match m.kind() {
                    Kind::MethodDeclaration => {
                        if m.body().is_none() {
                            continue;
                        }
                        let Some(key) = l.member_name(m) else { continue };
                        if key == "toString" {
                            has_to_string = true;
                        }
                        let function = l.function(m);
                        l.out.push(method_statement(class.clone(), key, function));
                    }
                    Kind::GetAccessor | Kind::SetAccessor => {
                        l.unsupported(m, "a getter or setter");
                    }
                    Kind::PropertyDeclaration if m.name().is_some_and(|n| n.kind() == Kind::PrivateIdentifier) => {
                        l.unsupported(m, "a private name");
                    }
                    _ => {}
                }
            }
            if has_to_string {
                let body = vec![Stmt::Return(vec![Expr::global("self").method("toString", Vec::new())])];
                let function = Function { params: vec![Cow::Borrowed("self")], varargs: false, body };
                l.out.push(method_statement(class.clone(), Cow::Borrowed("__tostring"), function));
            }
            for &m in &members {
                match m.kind() {
                    Kind::PropertyDeclaration if m.is_static() && !m.is_ambient() => {
                        let Some(initializer) = m.initializer() else { continue };
                        let Some(key) = l.member_name(m) else { continue };
                        if uses_this(initializer) {
                            l.unsupported(initializer, "this in a static property initializer");
                        }
                        let value = l.expr(initializer);
                        l.out.push(Stmt::Assign(vec![class.clone().field(key)], vec![value]));
                    }
                    Kind::ClassStaticBlockDeclaration => {
                        let Some(body) = m.body() else { continue };
                        let mut block = l.block(body.statements());
                        if uses_this(body) {
                            block.insert(0, Stmt::Local(vec![Cow::Borrowed("self")], vec![class.clone()]));
                        }
                        l.out.push(Stmt::Do(block));
                    }
                    _ => {}
                }
            }
        });
        self.container = outer_container;
        self.out.push(Stmt::Do(block));
        if s.is_export() {
            self.export(local);
        }
    }

    fn member_name(&mut self, m: Node) -> Option<Name> {
        let name = m.name()?;
        match name.kind() {
            Kind::Identifier | Kind::StringLiteral => Some(Cow::Borrowed(name.text())),
            _ => {
                self.unsupported(name, "this member name");
                None
            }
        }
    }

    fn constructor(&mut self, class: &Expr, s: Node, constructor: Option<Node>, extends: bool) {
        let (function, block) = self.capture(|l| {
            let Some(c) = constructor else {
                if extends {
                    let call =
                        Expr::global("super").field("constructor").call(vec![Expr::global("self"), Expr::VarArgs]);
                    l.out.push(Stmt::Call(call));
                }
                l.property_initializers(s);
                return (Vec::new(), extends);
            };
            let (params, varargs) = l.parameters(c.parameters());
            let Some(body) = c.body() else { return (params, varargs) };
            let statements = body.statements();
            l.hoist(statements);
            let split = statements.iter().position(is_super_call).map_or(0, |i| i + 1);
            for st in statements.iter().take(split) {
                l.stmt(st);
            }
            for p in c.parameters().iter() {
                if p.is_parameter_property()
                    && let Some(name) = p.name()
                {
                    let local = l.local_name(name);
                    l.out.push(Stmt::Assign(vec![Expr::global("self").field(local.clone())], vec![Expr::Name(local)]));
                }
            }
            l.property_initializers(s);
            for st in statements.iter().skip(split) {
                l.stmt(st);
            }
            (params, varargs)
        });
        let (params, varargs) = function;
        let function = Function { params, varargs, body: block };
        self.out.push(Stmt::Function(class.clone(), Some(Cow::Borrowed("constructor")), function));
    }

    fn property_initializers(&mut self, s: Node) {
        for m in s.members().iter() {
            if m.kind() != Kind::PropertyDeclaration || m.is_static() || m.is_ambient() || m.is_abstract() {
                continue;
            }
            let Some(initializer) = m.initializer() else { continue };
            let Some(key) = self.member_name(m) else { continue };
            let value = self.expr(initializer);
            self.out.push(Stmt::Assign(vec![Expr::global("self").field(key)], vec![value]));
        }
    }
}

/// `function Class:name(...)` for a function that takes `self`, else `function Class.name(...)`.
fn method_statement(class: Expr, name: Name, function: Function) -> Stmt {
    if function.params.first().is_some_and(|p| p == "self") {
        Stmt::Function(class, Some(name), function.without_self())
    } else {
        Stmt::Function(class.field(name), None, function)
    }
}

impl Function {
    /// The function without its leading `self` parameter, for `function a:b()` syntax.
    fn without_self(mut self) -> Function {
        if self.params.first().is_some_and(|p| p == "self") {
            self.params.remove(0);
        }
        self
    }
}
