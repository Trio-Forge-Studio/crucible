use std::borrow::Cow;

use super::{Lower, skip_down};
use crate::facts::{Callee, Constant, Falsy, Receiver};
use crate::luau::{BinOp, Block, Expr, Field, Part, Stmt, UnOp};
use crate::macros::Macro;
use crate::tree::{Kind, Node, Op, UnaryOp};

/// An assignable location, lowered once so it can be read and written.
#[derive(Clone)]
pub(super) enum Place {
    Name(Expr),
    Index(Expr, Expr),
}

impl Place {
    pub(super) fn expr(&self) -> Expr {
        match self {
            Place::Name(e) => e.clone(),
            Place::Index(base, key) => base.clone().index(key.clone()),
        }
    }
}

/// `e + delta`, folding a number.
pub(super) fn offset(e: Expr, delta: f64) -> Expr {
    match e {
        Expr::Number(n) => Expr::Number(n + delta),
        Expr::Binary(BinOp::Add, l, r) if matches!(*r, Expr::Number(_)) => {
            let Expr::Number(n) = *r else { unreachable!() };
            offset_number(*l, n + delta)
        }
        Expr::Binary(BinOp::Sub, l, r) if matches!(*r, Expr::Number(_)) => {
            let Expr::Number(n) = *r else { unreachable!() };
            offset_number(*l, delta - n)
        }
        e => offset_number(e, delta),
    }
}

fn offset_number(e: Expr, delta: f64) -> Expr {
    if delta == 0.0 {
        e
    } else if delta > 0.0 {
        e.binary(BinOp::Add, Expr::Number(delta))
    } else {
        e.binary(BinOp::Sub, Expr::Number(-delta))
    }
}

impl Lower<'_> {
    pub(super) fn expr(&mut self, n: Node) -> Expr {
        match n.kind() {
            Kind::Identifier => self.identifier(n),
            Kind::NumericLiteral => Expr::Number(n.number()),
            Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral => Expr::String(Cow::Borrowed(n.text())),
            Kind::TemplateExpression => self.template(n),
            Kind::TrueKeyword => Expr::Bool(true),
            Kind::FalseKeyword => Expr::Bool(false),
            Kind::NullKeyword => Expr::Nil,
            Kind::ThisKeyword => Expr::global("self"),
            Kind::SuperKeyword => Expr::global("super"),
            Kind::ParenthesizedExpression
            | Kind::AsExpression
            | Kind::NonNullExpression
            | Kind::SatisfiesExpression
            | Kind::TypeAssertionExpression => self.expr(n.expression()),
            Kind::PropertyAccessExpression => self.property_access(n),
            Kind::ElementAccessExpression => self.element_access(n),
            Kind::CallExpression => self.call_value(n),
            Kind::NewExpression => self.new_expression(n),
            Kind::BinaryExpression => self.binary(n),
            Kind::PrefixUnaryExpression => self.prefix(n),
            Kind::PostfixUnaryExpression => self.increment(n, true).unwrap_or(Expr::Nil),
            Kind::ConditionalExpression => self.conditional(n),
            Kind::ArrowFunction | Kind::FunctionExpression => {
                if n.kind() == Kind::FunctionExpression && n.name().is_some() {
                    self.error(n, "a named function expression is not supported");
                }
                Expr::Function(Box::new(self.function(n)))
            }
            Kind::ArrayLiteralExpression => self.array_literal(n),
            Kind::ObjectLiteralExpression => self.object_literal(n),
            Kind::RegularExpressionLiteral => self.unsupported(n, "a regular expression literal"),
            Kind::AwaitExpression => self.unsupported(n, "await"),
            Kind::YieldExpression => self.unsupported(n, "yield"),
            Kind::TaggedTemplateExpression => self.unsupported(n, "a tagged template"),
            Kind::ClassExpression => self.unsupported(n, "a class expression"),
            Kind::DeleteExpression => self.unsupported(n, "delete"),
            Kind::TypeOfExpression => {
                self.error(n, "typeof is not supported; use typeOf or typeIs");
                Expr::Nil
            }
            Kind::VoidExpression => self.unsupported(n, "void"),
            Kind::BigIntLiteral => self.unsupported(n, "a bigint literal"),
            Kind::SpreadElement => self.unsupported(n, "a spread here"),
            Kind::JsxElement | Kind::JsxSelfClosingElement | Kind::JsxFragment => self.unsupported(n, "JSX"),
            _ => {
                let what = format!("the expression {}", n.kind_name());
                self.unsupported(n, &what)
            }
        }
    }

    fn identifier(&mut self, n: Node) -> Expr {
        let text = n.text();
        if text == "undefined" {
            return Expr::Nil;
        }
        if self.fact(n).and_then(|f| f.callee) == Some(Callee::Macro(Macro::Promise)) {
            self.uses_ts = true;
            return Expr::global("TS.Promise");
        }
        if !crate::luau::is_name(text) {
            self.error(n, format!("`{text}` cannot be a Luau name"));
        }
        Expr::Name(Cow::Borrowed(text))
    }

    pub(super) fn constant(&self, n: Node) -> Option<Expr> {
        match self.fact(n)?.constant.as_ref()? {
            Constant::Number(v) => Some(Expr::Number(*v)),
            Constant::String(s) => Some(Expr::String(Cow::Owned(s.to_string()))),
        }
    }

    pub(super) fn receiver(&mut self, n: Node) -> Option<Receiver> {
        let receiver = self.fact(n).and_then(|f| f.receiver);
        if receiver.is_none() {
            self.error(n, "the type facts do not say what kind of value this is");
        }
        receiver
    }

    fn property_access(&mut self, n: Node) -> Expr {
        if let Some(c) = self.constant(n) {
            return c;
        }
        if n.in_optional_chain() {
            return self.optional_chain(n, false).unwrap_or(Expr::Nil);
        }
        let name = n.name().expect("a property access has a name");
        if name.kind() == Kind::PrivateIdentifier {
            return self.unsupported(name, "a private name");
        }
        let base = self.expr(n.expression());
        base.field(Cow::Borrowed(name.text()))
    }

    fn element_access(&mut self, n: Node) -> Expr {
        if let Some(c) = self.constant(n) {
            return c;
        }
        if n.in_optional_chain() {
            return self.optional_chain(n, false).unwrap_or(Expr::Nil);
        }
        let object = n.expression();
        let receiver = self.receiver(object);
        let inner = skip_down(object);
        if receiver == Some(Receiver::LuaTuple) && inner.kind() == Kind::CallExpression {
            let call = self.call(inner, false).unwrap_or(Expr::Nil);
            let index = self.expr(n.argument_expression());
            return match index {
                Expr::Number(0.0) => Expr::Paren(Box::new(call)),
                index => Expr::Paren(Box::new(Expr::global("select").call(vec![offset(index, 1.0), call]))),
            };
        }
        let mut values = self.ordered([object, n.argument_expression()], |l, e| l.expr(e));
        let key = values.pop().unwrap();
        let base = values.pop().unwrap();
        let key = if matches!(receiver, Some(Receiver::Array | Receiver::LuaTuple)) { offset(key, 1.0) } else { key };
        base.index(key)
    }

    /// Lowers an assignment target. With `simple`, the base and key are stored so the place can be read and
    /// written without evaluating them twice.
    pub(super) fn place(&mut self, target: Node, simple: bool) -> Place {
        let target = skip_down(target);
        match target.kind() {
            Kind::Identifier => Place::Name(self.identifier(target)),
            Kind::PropertyAccessExpression if !target.in_optional_chain() => {
                let name = target.name().unwrap();
                if name.kind() == Kind::PrivateIdentifier {
                    self.unsupported(name, "a private name");
                }
                let base = self.expr(target.expression());
                let base = if simple { self.push_temp_if_complex(base) } else { base };
                Place::Index(base, Expr::String(Cow::Borrowed(name.text())))
            }
            Kind::ElementAccessExpression if !target.in_optional_chain() => {
                let object = target.expression();
                let receiver = self.receiver(object);
                let mut values = self.ordered([object, target.argument_expression()], |l, e| l.expr(e));
                let key = values.pop().unwrap();
                let base = values.pop().unwrap();
                let key = if receiver == Some(Receiver::Array) { offset(key, 1.0) } else { key };
                if simple {
                    let base = self.push_temp_if_complex(base);
                    let key = self.push_temp_if_complex(key);
                    Place::Index(base, key)
                } else {
                    Place::Index(base, key)
                }
            }
            _ => {
                self.unsupported(target, "this assignment target");
                Place::Name(Expr::Nil)
            }
        }
    }

    fn simplify_place(&mut self, place: Place) -> Place {
        match place {
            Place::Index(base, key) => {
                let base = self.push_temp_if_complex(base);
                let key = self.push_temp_if_complex(key);
                Place::Index(base, key)
            }
            place => place,
        }
    }

    /// Lowers `=` and the compound assignments. Returns the assigned value when `used`.
    pub(super) fn assignment(&mut self, n: Node, used: bool) -> Option<Expr> {
        let op = n.operator();
        let (target, source) = (n.left(), n.right());
        let target_kind = skip_down(target).kind();
        if op == Op::Assign && matches!(target_kind, Kind::ArrayLiteralExpression | Kind::ObjectLiteralExpression) {
            return self.assign_pattern(skip_down(target), source, used);
        }
        let (place, place_block) = self.capture(|l| l.place(target, false));
        self.out.extend(place_block);
        match op {
            Op::Assign => {
                let (value, block) = self.capture(|l| l.expr(source));
                let place = if block.is_empty() { place } else { self.simplify_place(place) };
                self.out.extend(block);
                let value = if used { self.push_temp_if_complex(value) } else { value };
                self.out.push(Stmt::Assign(vec![place.expr()], vec![value.clone()]));
                used.then_some(value)
            }
            Op::CoalesceAssign | Op::OrAssign | Op::AndAssign => {
                let place = self.simplify_place(place);
                let current = place.expr();
                let condition = match op {
                    Op::CoalesceAssign => current.binary(BinOp::Eq, Expr::Nil),
                    Op::OrAssign => {
                        let falsy = self.falsy(target);
                        self.truthy_check(current, falsy).negated()
                    }
                    _ => {
                        let falsy = self.falsy(target);
                        self.truthy_check(current, falsy)
                    }
                };
                let (value, mut block) = self.capture(|l| l.expr(source));
                block.push(Stmt::Assign(vec![place.expr()], vec![value]));
                self.out.push(Stmt::If(condition, block, None));
                used.then(|| place.expr())
            }
            _ => {
                let operator = op.compound().unwrap();
                let (value, block) = self.capture(|l| l.expr(source));
                let place = if block.is_empty() && !used { place } else { self.simplify_place(place) };
                self.out.extend(block);
                let concat = operator == Op::Add && self.is_string_plus(target, source);
                if concat {
                    let value = self.stringify(source, value);
                    self.out.push(Stmt::Compound(BinOp::Concat, place.expr(), value));
                } else if let Some(binop) = arithmetic(operator) {
                    self.out.push(Stmt::Compound(binop, place.expr(), value));
                } else if let Some(function) = bitwise(operator) {
                    let place = self.simplify_place(place.clone());
                    let call = Expr::global(function).call(vec![place.expr(), value]);
                    self.out.push(Stmt::Assign(vec![place.expr()], vec![call]));
                    return used.then(|| place.expr());
                } else {
                    self.error(n, "this compound assignment is not supported");
                }
                used.then(|| place.expr())
            }
        }
    }

    /// Lowers `++` and `--`. Returns the expression's value when `used`.
    pub(super) fn increment(&mut self, n: Node, used: bool) -> Option<Expr> {
        let op = match n.unary_operator() {
            Some(UnaryOp::Increment) => BinOp::Add,
            Some(UnaryOp::Decrement) => BinOp::Sub,
            _ => {
                self.unsupported(n, "this unary operator");
                return None;
            }
        };
        let place = self.place(n.operand(), used);
        if !used {
            self.out.push(Stmt::Compound(op, place.expr(), Expr::Number(1.0)));
            return None;
        }
        if n.kind() == Kind::PrefixUnaryExpression {
            self.out.push(Stmt::Compound(op, place.expr(), Expr::Number(1.0)));
            return Some(place.expr());
        }
        let old = self.push_temp(place.expr());
        self.out.push(Stmt::Assign(vec![place.expr()], vec![old.clone().binary(op, Expr::Number(1.0))]));
        Some(old)
    }

    fn prefix(&mut self, n: Node) -> Expr {
        match n.unary_operator() {
            Some(UnaryOp::Not) => self.condition(n.operand()).negated(),
            Some(UnaryOp::Minus) => {
                let operand = self.expr(n.operand());
                match operand {
                    Expr::Number(v) => Expr::Number(-v),
                    operand => Expr::unary(UnOp::Neg, operand),
                }
            }
            Some(UnaryOp::Plus) => {
                let operand = self.expr(n.operand());
                Expr::global("tonumber").call(vec![operand])
            }
            Some(UnaryOp::BitNot) => {
                let operand = self.expr(n.operand());
                Expr::global("bit32.bnot").call(vec![operand])
            }
            Some(UnaryOp::Increment | UnaryOp::Decrement) => self.increment(n, true).unwrap_or(Expr::Nil),
            None => self.unsupported(n, "this unary operator"),
        }
    }

    fn binary(&mut self, n: Node) -> Expr {
        let op = n.operator();
        if op.is_assignment() {
            return self.assignment(n, true).unwrap_or(Expr::Nil);
        }
        match op {
            Op::And | Op::Or => self.logical(n),
            Op::Coalesce => self.coalesce(n),
            Op::Add => {
                let mut values = self.ordered([n.left(), n.right()], |l, e| l.expr(e));
                let right = values.pop().unwrap();
                let left = values.pop().unwrap();
                if self.is_string_plus(n.left(), n.right()) {
                    let left = self.stringify(n.left(), left);
                    let right = self.stringify(n.right(), right);
                    left.binary(BinOp::Concat, right)
                } else {
                    left.binary(BinOp::Add, right)
                }
            }
            Op::LooseEq | Op::LooseNe => {
                self.error(n, "loose equality is not supported; use === or !==");
                Expr::Nil
            }
            Op::Comma => self.unsupported(n, "the comma operator"),
            Op::In => self.unsupported(n, "the in operator"),
            Op::InstanceOf => self.unsupported(n, "instanceof"),
            op => {
                let mut values = self.ordered([n.left(), n.right()], |l, e| l.expr(e));
                let right = values.pop().unwrap();
                let left = values.pop().unwrap();
                if let Some(binop) = arithmetic(op).or_else(|| comparison(op)) {
                    left.binary(binop, right)
                } else if let Some(function) = bitwise(op) {
                    Expr::global(function).call(vec![left, right])
                } else {
                    self.unsupported(n, "this operator")
                }
            }
        }
    }

    /// Whether `+` concatenates: either operand is a string.
    fn is_string_plus(&mut self, left: Node, right: Node) -> bool {
        let mut string = false;
        for operand in [left, right] {
            match self.fact(operand).and_then(|f| f.receiver) {
                Some(Receiver::String) => string = true,
                Some(_) => {}
                None => self.error(operand, "the type facts do not say whether this operand is a string"),
            }
        }
        string
    }

    /// An operand of `..`, converted with `tostring` unless it is a string that cannot be nil, or a number literal.
    fn stringify(&mut self, n: Node, value: Expr) -> Expr {
        let fact = self.fact(n);
        let string = fact.is_some_and(|f| f.receiver == Some(Receiver::String) && !f.nullable());
        if string || matches!(value, Expr::String(_) | Expr::Number(_) | Expr::Interpolated(_)) {
            value
        } else {
            Expr::global("tostring").call(vec![value])
        }
    }

    /// The JavaScript-falsy values `n` may hold. A chain of `&&` or `||` has facts only on its operands.
    pub(super) fn falsy(&mut self, n: Node) -> Falsy {
        if let Some(falsy) = self.fact(n).and_then(|f| f.falsy) {
            return falsy;
        }
        if n.kind() == Kind::BinaryExpression {
            match n.operator() {
                Op::And => return self.falsy(n.left()) | self.falsy(n.right()),
                Op::Or => return self.falsy(n.right()),
                _ => {}
            }
        }
        if n.kind() == Kind::ParenthesizedExpression {
            return self.falsy(n.expression());
        }
        self.error(n, "the type facts do not say which falsy values this may hold");
        Falsy::ALL
    }

    /// A Luau condition that is true when `value` is JavaScript-truthy.
    pub(super) fn truthy_check(&mut self, value: Expr, falsy: Falsy) -> Expr {
        let mut checks = Vec::new();
        let extra = falsy.contains(Falsy::ZERO) || falsy.contains(Falsy::NAN) || falsy.contains(Falsy::EMPTY_STRING);
        let value = if extra { self.push_temp_if_complex(value) } else { value };
        if falsy.contains(Falsy::ZERO) {
            checks.push(value.clone().binary(BinOp::Ne, Expr::Number(0.0)));
        }
        if falsy.contains(Falsy::NAN) {
            checks.push(value.clone().binary(BinOp::Eq, value.clone()));
        }
        if falsy.contains(Falsy::EMPTY_STRING) {
            checks.push(value.clone().binary(BinOp::Ne, Expr::string("")));
        }
        if checks.is_empty() || falsy.contains(Falsy::NIL) || falsy.contains(Falsy::FALSE) {
            checks.push(value);
        }
        checks.into_iter().reduce(|a, b| a.binary(BinOp::And, b)).unwrap()
    }

    /// Lowers `n` as a condition, where only truthiness matters.
    pub(super) fn condition(&mut self, n: Node) -> Expr {
        match n.kind() {
            Kind::ParenthesizedExpression => {
                let inner = n.expression();
                if is_logic(inner) || self.fact(n).is_none() {
                    return self.condition(inner);
                }
            }
            Kind::PrefixUnaryExpression if n.unary_operator() == Some(UnaryOp::Not) => {
                return self.condition(n.operand()).negated();
            }
            Kind::BinaryExpression if matches!(n.operator(), Op::And | Op::Or) => {
                let left = self.condition(n.left());
                let (right, block) = self.capture(|l| l.condition(n.right()));
                let op = if n.operator() == Op::And { BinOp::And } else { BinOp::Or };
                if block.is_empty() {
                    return left.binary(op, right);
                }
                let result = self.push_temp(left);
                let mut inner = block;
                inner.push(Stmt::Assign(vec![result.clone()], vec![right]));
                let test = if op == BinOp::And { result.clone() } else { result.clone().negated() };
                self.out.push(Stmt::If(test, inner, None));
                return result;
            }
            Kind::BinaryExpression
                if matches!(n.operator(), Op::StrictEq | Op::StrictNe | Op::Lt | Op::Le | Op::Gt | Op::Ge) =>
            {
                return self.expr(n);
            }
            Kind::TrueKeyword | Kind::FalseKeyword => return self.expr(n),
            Kind::CallExpression if self.is_boolean_macro(n) => return self.expr(n),
            _ => {}
        }
        let falsy = self.falsy(n);
        let value = self.expr(n);
        self.truthy_check(value, falsy)
    }

    fn is_boolean_macro(&self, call: Node) -> bool {
        matches!(
            self.fact(call.expression()).and_then(|f| f.callee),
            Some(Callee::Macro(
                Macro::TypeIs
                    | Macro::ClassIs
                    | Macro::ReadonlyArrayIsEmpty
                    | Macro::ReadonlyArrayIncludes
                    | Macro::ReadonlyArrayEvery
                    | Macro::ReadonlyArraySome
                    | Macro::ReadonlySetIsEmpty
                    | Macro::ReadonlySetHas
                    | Macro::ReadonlyMapIsEmpty
                    | Macro::ReadonlyMapHas
            ))
        )
    }

    fn logical(&mut self, n: Node) -> Expr {
        let and = n.operator() == Op::And;
        let falsy = self.falsy(n.left());
        let left = self.expr(n.left());
        let (right, block) = self.capture(|l| l.expr(n.right()));
        let plain = falsy.without(Falsy::NIL).without(Falsy::FALSE).is_empty();
        if plain && block.is_empty() {
            return left.binary(if and { BinOp::And } else { BinOp::Or }, right);
        }
        let result = self.push_temp(left);
        let test = self.truthy_check(result.clone(), falsy);
        let test = if and { test } else { test.negated() };
        let mut inner = block;
        inner.push(Stmt::Assign(vec![result.clone()], vec![right]));
        self.out.push(Stmt::If(test, inner, None));
        result
    }

    /// `a ?? b`. When the whole chain cannot be `false`, no operand's non-nil value is falsy in Luau, so `or`
    /// matches.
    fn coalesce(&mut self, n: Node) -> Expr {
        let mut operands = vec![n.right()];
        let mut left = n.left();
        while left.kind() == Kind::BinaryExpression && left.operator() == Op::Coalesce {
            operands.push(left.right());
            left = left.left();
        }
        operands.push(left);
        operands.reverse();
        let can_be_false = self.falsy(n).contains(Falsy::FALSE);
        let first = self.expr(operands[0]);
        let mut rest = Vec::new();
        for &o in &operands[1..] {
            rest.push(self.capture(|l| l.expr(o)));
        }
        if !can_be_false && rest.iter().all(|(_, b)| b.is_empty()) {
            return rest.into_iter().fold(first, |acc, (e, _)| acc.binary(BinOp::Or, e));
        }
        let result = self.push_temp(first);
        for (value, mut block) in rest {
            block.push(Stmt::Assign(vec![result.clone()], vec![value]));
            self.out.push(Stmt::If(result.clone().binary(BinOp::Eq, Expr::Nil), block, None));
        }
        result
    }

    fn conditional(&mut self, n: Node) -> Expr {
        let condition = self.condition(n.condition().unwrap());
        let (then, then_block) = self.capture(|l| l.expr(n.when_true()));
        let (otherwise, otherwise_block) = self.capture(|l| l.expr(n.when_false()));
        if then_block.is_empty() && otherwise_block.is_empty() {
            return Expr::if_else(condition, then, otherwise);
        }
        let result = self.temp();
        self.out.push(Stmt::Local(vec![result.clone()], Vec::new()));
        let assign = |mut block: Block, value: Expr| {
            block.push(Stmt::Assign(vec![Expr::Name(result.clone())], vec![value]));
            block
        };
        let then = assign(then_block, then);
        let otherwise = assign(otherwise_block, otherwise);
        self.out.push(Stmt::If(condition, then, Some(otherwise)));
        Expr::Name(result)
    }

    fn template(&mut self, n: Node) -> Expr {
        let spans: Vec<Node> = n.template_spans().iter().collect();
        let values = self.ordered(spans.iter().map(|s| s.expression()), |l, e| l.expr(e));
        let mut parts = Vec::new();
        let head = n.head().text();
        if !head.is_empty() {
            parts.push(Part::Text(Cow::Borrowed(head)));
        }
        for (span, value) in spans.iter().zip(values) {
            parts.push(Part::Expr(value));
            let text = span.literal().text();
            if !text.is_empty() {
                parts.push(Part::Text(Cow::Borrowed(text)));
            }
        }
        Expr::Interpolated(parts)
    }

    fn array_literal(&mut self, n: Node) -> Expr {
        let elements: Vec<Node> = n.elements().iter().collect();
        let Some(first_spread) = elements.iter().position(|e| e.kind() == Kind::SpreadElement) else {
            let values = self.ordered(elements.iter().copied(), |l, e| {
                if e.kind() == Kind::OmittedExpression { Expr::Nil } else { l.expr(e) }
            });
            return Expr::Table(values.into_iter().map(Field::Item).collect());
        };
        if elements.len() == 1 {
            return self.spread_copy(elements[0].expression());
        }
        let head = self.ordered(elements[..first_spread].iter().copied(), |l, e| l.expr(e));
        let result = self.push_temp(Expr::Table(head.into_iter().map(Field::Item).collect()));
        for &e in &elements[first_spread..] {
            if e.kind() == Kind::SpreadElement {
                self.spread_append(result.clone(), e.expression());
            } else {
                let value = self.expr(e);
                self.out.push(Stmt::Call(Expr::global("table.insert").call(vec![result.clone(), value])));
            }
        }
        result
    }

    /// A new array holding the values of a spread source.
    fn spread_copy(&mut self, source: Node) -> Expr {
        let receiver = self.receiver(source);
        let inner = skip_down(source);
        if receiver == Some(Receiver::LuaTuple) && inner.kind() == Kind::CallExpression {
            let call = self.call(inner, false).unwrap_or(Expr::Nil);
            return Expr::Table(vec![Field::Item(call)]);
        }
        if receiver == Some(Receiver::Array) {
            let value = self.expr(source);
            let value = self.push_temp_if_complex(value);
            let length = Expr::unary(UnOp::Len, value.clone());
            return Expr::global("table.move").call(vec![
                value,
                Expr::Number(1.0),
                length,
                Expr::Number(1.0),
                Expr::Table(Vec::new()),
            ]);
        }
        let result = self.push_temp(Expr::Table(Vec::new()));
        self.spread_append(result.clone(), source);
        result
    }

    /// Appends a spread source's values to the array in `target`.
    fn spread_append(&mut self, target: Expr, source: Node) {
        let receiver = self.receiver(source);
        let inner = skip_down(source);
        let value = if receiver == Some(Receiver::LuaTuple) && inner.kind() == Kind::CallExpression {
            let call = self.call(inner, false).unwrap_or(Expr::Nil);
            Expr::Table(vec![Field::Item(call)])
        } else {
            self.expr(source)
        };
        let insert = |item: Expr| Stmt::Call(Expr::global("table.insert").call(vec![target.clone(), item]));
        match receiver {
            Some(Receiver::Array | Receiver::LuaTuple) => {
                let value = self.push_temp_if_complex(value);
                let length = Expr::unary(UnOp::Len, value.clone());
                let at = offset(Expr::unary(UnOp::Len, target.clone()), 1.0);
                let call = Expr::global("table.move").call(vec![value, Expr::Number(1.0), length, at, target.clone()]);
                self.out.push(Stmt::Call(call));
            }
            Some(Receiver::Set | Receiver::IterFn) => {
                let v = self.temp();
                let body = vec![insert(Expr::Name(v.clone()))];
                self.out.push(Stmt::GenericFor(vec![v], vec![value], body));
            }
            Some(Receiver::Map) => {
                let (k, v) = (self.temp(), self.temp());
                let pair = Expr::Table(vec![Field::Item(Expr::Name(k.clone())), Field::Item(Expr::Name(v.clone()))]);
                let body = vec![insert(pair)];
                self.out.push(Stmt::GenericFor(vec![k, v], vec![value], body));
            }
            Some(_) => {
                self.unsupported(source, "spreading this value");
            }
            None => {}
        }
    }

    fn property_key(&mut self, name: Node) -> Expr {
        match name.kind() {
            Kind::Identifier | Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral => {
                Expr::String(Cow::Borrowed(name.text()))
            }
            Kind::NumericLiteral => Expr::Number(name.number()),
            Kind::ComputedPropertyName => self.expr(name.expression()),
            _ => self.unsupported(name, "this property name"),
        }
    }

    fn object_literal(&mut self, n: Node) -> Expr {
        let properties: Vec<Node> = n.properties().iter().collect();
        let first_spread = properties.iter().position(|p| p.kind() == Kind::SpreadAssignment);
        let head_end = first_spread.unwrap_or(properties.len());
        let mut lowered = Vec::new();
        for &p in &properties[..head_end] {
            let (key, value) = self.object_property(p);
            lowered.push(key);
            lowered.push(value);
        }
        let values = self.order_values(lowered);
        let fields: Vec<Field> = values.chunks(2).map(|pair| Field::Keyed(pair[0].clone(), pair[1].clone())).collect();
        if first_spread.is_none() {
            return Expr::Table(fields);
        }
        let result = self.push_temp(Expr::Table(fields));
        for &p in &properties[head_end..] {
            if p.kind() == Kind::SpreadAssignment {
                let source = p.expression();
                let falsy = self.falsy(source);
                let value = self.expr(source);
                let value = self.push_temp_if_complex(value);
                let (k, v) = (self.temp(), self.temp());
                let assign =
                    Stmt::Assign(vec![result.clone().index(Expr::Name(k.clone()))], vec![Expr::Name(v.clone())]);
                let merge = Stmt::GenericFor(vec![k, v], vec![value.clone()], vec![assign]);
                if falsy.is_empty() {
                    self.out.push(merge);
                } else {
                    let test = self.truthy_check(value, falsy);
                    self.out.push(Stmt::If(test, vec![merge], None));
                }
            } else {
                let ((key, key_block), (value, value_block)) = self.object_property(p);
                self.out.extend(key_block);
                let key = if value_block.is_empty() { key } else { self.push_temp_if_complex(key) };
                self.out.extend(value_block);
                self.out.push(Stmt::Assign(vec![result.clone().index(key)], vec![value]));
            }
        }
        result
    }

    /// The key and value of an object literal member, each with the statements it needs.
    fn object_property(&mut self, p: Node) -> ((Expr, Block), (Expr, Block)) {
        match p.kind() {
            Kind::PropertyAssignment => {
                let name = p.name().unwrap();
                let key = self.capture(|l| l.property_key(name));
                let value = self.capture(|l| l.expr(p.initializer().unwrap()));
                (key, value)
            }
            Kind::ShorthandPropertyAssignment => {
                let name = p.name().unwrap();
                let value = self.identifier(name);
                ((Expr::String(Cow::Borrowed(name.text())), Vec::new()), (value, Vec::new()))
            }
            Kind::MethodDeclaration => {
                let name = p.name().unwrap();
                let key = self.capture(|l| l.property_key(name));
                let function = self.function(p);
                (key, (Expr::Function(Box::new(function)), Vec::new()))
            }
            _ => {
                self.unsupported(p, "this object member");
                ((Expr::Nil, Vec::new()), (Expr::Nil, Vec::new()))
            }
        }
    }

    /// Lowers an optional chain ending at `n`. Every `?.` tests the value so far and leaves the result nil when
    /// it is nil. With `statement`, a final call runs as a statement and the result is `None`.
    pub(super) fn optional_chain(&mut self, n: Node, statement: bool) -> Option<Expr> {
        let mut links = Vec::new();
        let mut cur = n;
        while cur.in_optional_chain()
            && matches!(
                cur.kind(),
                Kind::PropertyAccessExpression
                    | Kind::ElementAccessExpression
                    | Kind::CallExpression
                    | Kind::NonNullExpression
            )
        {
            links.push(cur);
            cur = cur.expression();
        }
        links.reverse();
        if links[0].kind() == Kind::CallExpression && self.fact(cur).and_then(|f| f.callee) == Some(Callee::Method) {
            self.unsupported(n, "an optional call of a method");
        }
        let base = self.expr(cur);
        let result = self.push_temp(base);
        let mut segment: Vec<Block> = Vec::new();
        let mut open: Option<Block> = None;
        let mut i = 0;
        let mut final_call = None;
        while i < links.len() {
            let link = links[i];
            if link.has_question_dot() {
                if let Some(block) = open.take() {
                    segment.push(block);
                }
                open = Some(Vec::new());
            }
            let next_calls_this = links.get(i + 1).is_some_and(|next| {
                next.kind() == Kind::CallExpression && !next.has_question_dot() && next.expression() == link
            });
            let (value, block) = self.capture(|l| {
                if next_calls_this {
                    l.chain_call(links[i + 1], Some(link), result.clone())
                } else {
                    match link.kind() {
                        Kind::PropertyAccessExpression => {
                            result.clone().field(Cow::Borrowed(link.name().unwrap().text()))
                        }
                        Kind::ElementAccessExpression => {
                            let receiver = l.receiver(link.expression());
                            let key = l.expr(link.argument_expression());
                            let key = if receiver == Some(Receiver::Array) { offset(key, 1.0) } else { key };
                            result.clone().index(key)
                        }
                        Kind::CallExpression => l.chain_call(link, None, result.clone()),
                        _ => result.clone(),
                    }
                }
            });
            let consumed = if next_calls_this { 2 } else { 1 };
            let last = i + consumed == links.len();
            let target = open.get_or_insert_with(Vec::new);
            target.extend(block);
            if last && statement && value.is_multi() {
                final_call = Some(value);
            } else {
                target.push(Stmt::Assign(vec![result.clone()], vec![value]));
            }
            i += consumed;
        }
        if let Some(block) = open.take() {
            segment.push(block);
        }
        let count = segment.len();
        for (k, mut block) in segment.into_iter().enumerate() {
            if k + 1 == count
                && let Some(call) = final_call.take()
            {
                block.push(Stmt::Call(call));
            }
            self.out.push(Stmt::If(result.clone().binary(BinOp::Ne, Expr::Nil), block, None));
        }
        if statement { None } else { Some(result) }
    }

    /// A call inside an optional chain on the value `object`. `callee` is the property or element access the call
    /// goes through, or `None` when `object` is the function itself.
    fn chain_call(&mut self, call: Node, callee: Option<Node>, object: Expr) -> Expr {
        let Some(callee) = callee else {
            let args = self.arguments(call);
            return object.call(args);
        };
        let kind = self.fact(callee).and_then(|f| f.callee);
        match kind {
            Some(Callee::Macro(m)) => self.method_macro(m, call, object, false).unwrap_or(Expr::Nil),
            Some(kind) => {
                let args = self.arguments(call);
                match callee.kind() {
                    Kind::PropertyAccessExpression => {
                        let name = callee.name().unwrap().text();
                        if kind == Callee::Method {
                            self.method_call(object, Cow::Borrowed(name), args)
                        } else {
                            object.field(Cow::Borrowed(name)).call(args)
                        }
                    }
                    _ => {
                        let key = self.expr(callee.argument_expression());
                        let f = object.clone().index(key);
                        if kind == Callee::Method {
                            let mut all = vec![object];
                            all.extend(args);
                            f.call(all)
                        } else {
                            f.call(args)
                        }
                    }
                }
            }
            None => {
                self.error(callee, "the type facts do not say how to call this");
                Expr::Nil
            }
        }
    }
}

fn is_logic(n: Node) -> bool {
    match n.kind() {
        Kind::BinaryExpression => matches!(n.operator(), Op::And | Op::Or),
        Kind::PrefixUnaryExpression => n.unary_operator() == Some(UnaryOp::Not),
        Kind::ParenthesizedExpression => is_logic(n.expression()),
        _ => false,
    }
}

fn arithmetic(op: Op) -> Option<BinOp> {
    Some(match op {
        Op::Add => BinOp::Add,
        Op::Sub => BinOp::Sub,
        Op::Mul => BinOp::Mul,
        Op::Div => BinOp::Div,
        Op::Mod => BinOp::Mod,
        Op::Pow => BinOp::Pow,
        _ => return None,
    })
}

fn comparison(op: Op) -> Option<BinOp> {
    Some(match op {
        Op::StrictEq => BinOp::Eq,
        Op::StrictNe => BinOp::Ne,
        Op::Lt => BinOp::Lt,
        Op::Le => BinOp::Le,
        Op::Gt => BinOp::Gt,
        Op::Ge => BinOp::Ge,
        _ => return None,
    })
}

fn bitwise(op: Op) -> Option<&'static str> {
    Some(match op {
        Op::BitAnd => "bit32.band",
        Op::BitOr => "bit32.bor",
        Op::BitXor => "bit32.bxor",
        Op::Shl => "bit32.lshift",
        Op::Shr => "bit32.arshift",
        Op::UShr => "bit32.rshift",
        _ => return None,
    })
}
