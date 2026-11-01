use std::borrow::Cow;

use super::{Lower, is_hole, skip_down};
use crate::facts::Receiver;
use crate::luau::{BinOp, Expr, Name, Stmt, UnOp};
use crate::tree::{Kind, Node, Op};

impl Lower<'_> {
    /// `const <pattern> = init`. A `LuaTuple` call binds its values directly.
    pub(super) fn bind_initializer(&mut self, pattern: Node, init: Node) {
        let inner = skip_down(init);
        if pattern.kind() == Kind::ArrayBindingPattern
            && self.fact(pattern).and_then(|f| f.receiver) == Some(Receiver::LuaTuple)
            && inner.kind() == Kind::CallExpression
        {
            let call = self.call(inner, false).unwrap_or(Expr::Nil);
            let mut names = Vec::new();
            let mut later = Vec::new();
            for e in pattern.elements().iter() {
                if is_hole(e) {
                    names.push(self.temp());
                    continue;
                }
                if e.is_rest() {
                    self.unsupported(e, "a rest element over a LuaTuple");
                    continue;
                }
                let Some(name) = e.name() else {
                    self.unsupported(e, "this binding element");
                    continue;
                };
                if name.kind() == Kind::Identifier && e.initializer().is_none() && !self.is_hoisted(name) {
                    names.push(self.local_name(name));
                } else {
                    let temp = self.temp();
                    names.push(temp.clone());
                    later.push((e, temp));
                }
            }
            if names.is_empty() {
                self.discard(call);
                return;
            }
            self.out.push(Stmt::Local(names, vec![call]));
            for (e, temp) in later {
                self.bind_element(e, Expr::Name(temp));
            }
            return;
        }
        let value = self.expr(init);
        self.bind_value(pattern, value);
    }

    /// Declares the names in a binding name or pattern from `value`.
    pub(super) fn bind_value(&mut self, pattern: Node, value: Expr) {
        match pattern.kind() {
            Kind::Identifier => {
                let local = self.local_name(pattern);
                self.declare(pattern, local, Some(value));
            }
            Kind::ArrayBindingPattern => {
                match self.fact(pattern).and_then(|f| f.receiver) {
                    Some(Receiver::Array | Receiver::LuaTuple) => {}
                    Some(_) => {
                        self.unsupported(pattern, "destructuring this value");
                        return;
                    }
                    None => {
                        self.error(pattern, "the type facts do not say what this pattern destructures");
                        return;
                    }
                }
                let value = self.push_temp_if_complex(value);
                for (i, e) in pattern.elements().iter().enumerate() {
                    if is_hole(e) {
                        continue;
                    }
                    let element = if e.is_rest() {
                        let length = Expr::unary(UnOp::Len, value.clone());
                        let start = Expr::Number(i as f64 + 1.0);
                        Expr::global("table.move").call(vec![
                            value.clone(),
                            start,
                            length,
                            Expr::Number(1.0),
                            Expr::Table(Vec::new()),
                        ])
                    } else {
                        value.clone().index(Expr::Number(i as f64 + 1.0))
                    };
                    self.bind_element(e, element);
                }
            }
            Kind::ObjectBindingPattern => {
                let value = self.push_temp_if_complex(value);
                for e in pattern.elements().iter() {
                    if e.is_rest() {
                        self.unsupported(e, "an object rest element");
                        continue;
                    }
                    let Some(key) = e.property_name().or_else(|| e.name()) else {
                        self.unsupported(e, "this binding element");
                        continue;
                    };
                    let key = match key.kind() {
                        Kind::Identifier | Kind::StringLiteral => Expr::String(Cow::Borrowed(key.text())),
                        Kind::NumericLiteral => Expr::Number(key.number()),
                        _ => {
                            self.unsupported(key, "this destructured property name");
                            continue;
                        }
                    };
                    self.bind_element(e, value.clone().index(key));
                }
            }
            _ => {
                self.unsupported(pattern, "this binding");
            }
        }
    }

    fn bind_element(&mut self, e: Node, value: Expr) {
        let Some(name) = e.name() else {
            self.unsupported(e, "this binding element");
            return;
        };
        let default = e.initializer();
        if name.kind() == Kind::Identifier {
            let local = self.local_name(name);
            self.declare(name, local.clone(), Some(value));
            if let Some(default) = default {
                self.default(Expr::Name(local), default);
            }
            return;
        }
        let value = self.push_temp(value);
        if let Some(default) = default {
            self.default(value.clone(), default);
        }
        self.bind_value(name, value);
    }

    /// `if target == nil then target = default end`, evaluating the default only when it applies.
    fn default(&mut self, target: Expr, default: Node) {
        let (value, mut block) = self.capture(|l| l.expr(default));
        block.push(Stmt::Assign(vec![target.clone()], vec![value]));
        self.out.push(Stmt::If(target.binary(BinOp::Eq, Expr::Nil), block, None));
    }

    /// `[a, b] = value` and `({ a, b } = value)`. Returns the assigned value when `used`.
    pub(super) fn assign_pattern(&mut self, target: Node, source: Node, used: bool) -> Option<Expr> {
        let inner = skip_down(source);
        if target.kind() == Kind::ArrayLiteralExpression {
            let receiver = self.fact(target).and_then(|f| f.receiver);
            if receiver == Some(Receiver::LuaTuple) && inner.kind() == Kind::CallExpression {
                if used {
                    self.unsupported(target, "using the value of a LuaTuple assignment");
                }
                let call = self.call(inner, false).unwrap_or(Expr::Nil);
                let elements: Vec<Node> = target.elements().iter().collect();
                let temps: Vec<Name> = elements.iter().map(|_| self.temp()).collect();
                self.out.push(Stmt::Local(temps.clone(), vec![call]));
                for (e, temp) in elements.into_iter().zip(temps) {
                    if e.kind() != Kind::OmittedExpression {
                        self.assign_target(e, Expr::Name(temp));
                    }
                }
                return None;
            }
            match receiver {
                Some(Receiver::Array | Receiver::LuaTuple) => {}
                Some(_) => {
                    self.unsupported(target, "destructuring this value");
                    return None;
                }
                None => {
                    self.error(target, "the type facts do not say what this pattern destructures");
                    return None;
                }
            }
            let value = self.expr(source);
            let value = self.push_temp_if_complex(value);
            for (i, e) in target.elements().iter().enumerate() {
                match e.kind() {
                    Kind::OmittedExpression => {}
                    Kind::SpreadElement => {
                        let length = Expr::unary(UnOp::Len, value.clone());
                        let start = Expr::Number(i as f64 + 1.0);
                        let rest = Expr::global("table.move").call(vec![
                            value.clone(),
                            start,
                            length,
                            Expr::Number(1.0),
                            Expr::Table(Vec::new()),
                        ]);
                        self.assign_target(e.expression(), rest);
                    }
                    _ => self.assign_target(e, value.clone().index(Expr::Number(i as f64 + 1.0))),
                }
            }
            return used.then_some(value);
        }
        let value = self.expr(source);
        let value = self.push_temp_if_complex(value);
        for p in target.properties().iter() {
            match p.kind() {
                Kind::PropertyAssignment => {
                    let (Some(key), Some(target)) = (p.name(), p.initializer()) else {
                        self.unsupported(p, "this destructuring member");
                        continue;
                    };
                    if !matches!(key.kind(), Kind::Identifier | Kind::StringLiteral) {
                        self.unsupported(key, "this destructured property name");
                        continue;
                    }
                    let element = value.clone().field(Cow::Borrowed(key.text()));
                    self.assign_target(target, element);
                }
                Kind::ShorthandPropertyAssignment => {
                    let Some(name) = p.name() else {
                        self.unsupported(p, "this destructuring member");
                        continue;
                    };
                    let element = value.clone().field(Cow::Borrowed(name.text()));
                    let place = self.identifier_place(name);
                    self.out.push(Stmt::Assign(vec![place.clone()], vec![element]));
                    if let Some(default) = p.initializer() {
                        self.default(place, default);
                    }
                }
                _ => {
                    self.unsupported(p, "this destructuring member");
                }
            }
        }
        used.then_some(value)
    }

    fn identifier_place(&mut self, n: Node) -> Expr {
        Expr::Name(self.local_name(n))
    }

    fn assign_target(&mut self, target: Node, value: Expr) {
        if target.kind() == Kind::BinaryExpression && target.operator() == Op::Assign {
            let value = self.push_temp(value);
            self.default(value.clone(), target.right());
            return self.assign_target(target.left(), value);
        }
        if matches!(skip_down(target).kind(), Kind::ArrayLiteralExpression | Kind::ObjectLiteralExpression) {
            self.unsupported(target, "a nested assignment pattern");
            return;
        }
        let place = self.place(target, false);
        self.out.push(Stmt::Assign(vec![place.expr()], vec![value]));
    }
}
