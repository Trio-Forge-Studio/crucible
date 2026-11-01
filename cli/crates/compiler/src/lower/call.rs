use std::borrow::Cow;

use super::expr::offset;
use super::{Lower, skip_down};
use crate::facts::{Callee, Receiver};
use crate::luau::{BinOp, Expr, Field, Name, Stmt, UnOp, is_name};
use crate::macros::{Macro, MacroKind};
use crate::tree::{Kind, Node};

/// Luau types `type()` reports. Any other name needs `typeof()`.
const PRIMITIVE_TYPES: &[&str] =
    &["nil", "boolean", "string", "number", "table", "userdata", "function", "thread", "vector", "buffer"];

impl Lower<'_> {
    /// A call used as a value. A `LuaTuple` result becomes an array, as the source's type says.
    pub(super) fn call_value(&mut self, n: Node) -> Expr {
        let value = self.call(n, false).unwrap_or(Expr::Nil);
        if value.is_multi() && self.fact(n).and_then(|f| f.receiver) == Some(Receiver::LuaTuple) {
            Expr::Table(vec![Field::Item(value)])
        } else {
            value
        }
    }

    /// Lowers a call. With `statement`, a macro may lower to statements alone and return `None`.
    pub(super) fn call(&mut self, n: Node, statement: bool) -> Option<Expr> {
        if n.in_optional_chain() {
            return self.optional_chain(n, statement);
        }
        let callee = n.expression();
        let target = skip_down(callee);
        match target.kind() {
            Kind::SuperKeyword => {
                let args = self.arguments(n);
                let mut all = vec![Expr::global("self")];
                all.extend(args);
                return Some(Expr::global("super").field("constructor").call(all));
            }
            Kind::ImportKeyword => return Some(self.unsupported(n, "dynamic import")),
            _ => {}
        }
        if target.kind() == Kind::PropertyAccessExpression
            && skip_down(target.expression()).kind() == Kind::SuperKeyword
        {
            let name: Name = Cow::Borrowed(target.name().unwrap().text());
            let args = self.arguments(n);
            let mut all = vec![Expr::global("self")];
            all.extend(args);
            return Some(Expr::global("super").field(name).call(all));
        }
        let Some(kind) = self.fact(callee).and_then(|f| f.callee) else {
            self.error(callee, "the type facts do not say how to call this");
            return Some(Expr::Nil);
        };
        if let Callee::Macro(m) = kind {
            return self.macro_call(m, n, statement);
        }
        match target.kind() {
            Kind::PropertyAccessExpression => {
                let object = target.expression();
                let name: Name = Cow::Borrowed(target.name().unwrap().text());
                let base = self.expr(object);
                let (args, block) = self.capture(|l| l.arguments(n));
                let base = if block.is_empty() { base } else { self.push_temp_if_complex(base) };
                self.out.extend(block);
                Some(match kind {
                    Callee::Method => self.method_call(base, name, args),
                    _ => base.field(name).call(args),
                })
            }
            Kind::ElementAccessExpression => {
                let object = target.expression();
                let receiver = self.receiver(object);
                let mut values = self.ordered([object, target.argument_expression()], |l, e| l.expr(e));
                let key = values.pop().unwrap();
                let base = values.pop().unwrap();
                let key = if receiver == Some(Receiver::Array) { offset(key, 1.0) } else { key };
                if kind == Callee::Method {
                    let base = self.push_temp_if_complex(base);
                    let key = self.push_temp_if_complex(key);
                    let (args, block) = self.capture(|l| l.arguments(n));
                    self.out.extend(block);
                    let mut all = vec![base.clone()];
                    all.extend(args);
                    Some(base.index(key).call(all))
                } else {
                    let (args, block) = self.capture(|l| l.arguments(n));
                    let (base, key) = if block.is_empty() {
                        (base, key)
                    } else {
                        (self.push_temp_if_complex(base), self.push_temp_if_complex(key))
                    };
                    self.out.extend(block);
                    Some(base.index(key).call(args))
                }
            }
            _ => {
                let function = self.expr(callee);
                let (args, block) = self.capture(|l| l.arguments(n));
                let function = if block.is_empty() { function } else { self.push_temp_if_complex(function) };
                self.out.extend(block);
                Some(function.call(args))
            }
        }
    }

    /// `base:name(args)`, or `base["name"](base, args)` when the name is not a Luau name.
    pub(super) fn method_call(&mut self, base: Expr, name: Name, args: Vec<Expr>) -> Expr {
        if is_name(&name) {
            return base.method(name, args);
        }
        let base = self.push_temp_if_complex(base);
        let mut all = vec![base.clone()];
        all.extend(args);
        base.field(name).call(all)
    }

    /// A call's arguments in order. A spread must come last.
    pub(super) fn arguments(&mut self, call: Node) -> Vec<Expr> {
        let args: Vec<Node> = call.arguments().iter().collect();
        let roblox = self.fact(call.expression()).and_then(|f| f.roblox_api) == Some(true);
        let count = args.len();
        let mut values = self.ordered(args.iter().copied(), |l, a| {
            if a.kind() == Kind::SpreadElement { l.spread_argument(a.expression()) } else { l.expr(a) }
        });
        for (i, a) in args.iter().enumerate() {
            if a.kind() == Kind::SpreadElement && i + 1 != count {
                self.unsupported(*a, "a spread before the last argument");
            }
        }
        if roblox
            && let Some(last) = values.last_mut()
            && last.is_multi()
            && args.last().is_some_and(|a| a.kind() != Kind::SpreadElement)
        {
            let value = std::mem::replace(last, Expr::Nil);
            *last = Expr::Paren(Box::new(value));
        }
        values
    }

    fn spread_argument(&mut self, source: Node) -> Expr {
        let receiver = self.receiver(source);
        let inner = skip_down(source);
        match receiver {
            Some(Receiver::LuaTuple) if inner.kind() == Kind::CallExpression => {
                self.call(inner, false).unwrap_or(Expr::Nil)
            }
            Some(Receiver::Array | Receiver::LuaTuple) => {
                let value = self.expr(source);
                Expr::global("table.unpack").call(vec![value])
            }
            Some(_) => self.unsupported(source, "spreading this value into a call"),
            None => Expr::Nil,
        }
    }

    pub(super) fn new_expression(&mut self, n: Node) -> Expr {
        let callee = n.expression();
        if let Some(Callee::Macro(m)) = self.fact(callee).and_then(|f| f.callee) {
            return self.constructor_macro(m, n);
        }
        let class = self.expr(callee);
        let (args, block) = self.capture(|l| l.arguments(n));
        let class = if block.is_empty() { class } else { self.push_temp_if_complex(class) };
        self.out.extend(block);
        class.field("new").call(args)
    }

    fn constructor_macro(&mut self, m: Macro, n: Node) -> Expr {
        let args: Vec<Node> = n.arguments().iter().collect();
        let value = match m {
            Macro::NewArray => {
                if args.is_empty() {
                    Expr::Table(Vec::new())
                } else {
                    let values = self.ordered(args.iter().copied(), |l, a| l.expr(a));
                    Expr::global("table.create").call(values)
                }
            }
            Macro::NewSet | Macro::NewReadonlySet | Macro::NewWeakSet => self.new_set(args.first().copied()),
            Macro::NewMap | Macro::NewReadonlyMap | Macro::NewWeakMap => self.new_map(args.first().copied()),
            _ => return self.unsupported(n, "this constructor macro"),
        };
        if matches!(m, Macro::NewWeakSet | Macro::NewWeakMap) {
            let mode = Expr::Table(vec![Field::Keyed(Expr::string("__mode"), Expr::string("k"))]);
            return Expr::global("setmetatable").call(vec![value, mode]);
        }
        value
    }

    fn new_set(&mut self, source: Option<Node>) -> Expr {
        let Some(source) = source else { return Expr::Table(Vec::new()) };
        let inner = skip_down(source);
        if inner.kind() == Kind::ArrayLiteralExpression
            && !inner.elements().iter().any(|e| e.kind() == Kind::SpreadElement)
        {
            let values = self.ordered(inner.elements().iter(), |l, e| l.expr(e));
            return Expr::Table(values.into_iter().map(|v| Field::Keyed(v, Expr::Bool(true))).collect());
        }
        let value = self.expr(source);
        let set = self.push_temp(Expr::Table(Vec::new()));
        let (k, v) = (self.temp(), self.temp());
        let body = vec![Stmt::Assign(vec![set.clone().index(Expr::Name(v.clone()))], vec![Expr::Bool(true)])];
        self.out.push(Stmt::GenericFor(vec![k, v], vec![value], body));
        set
    }

    fn new_map(&mut self, source: Option<Node>) -> Expr {
        let Some(source) = source else { return Expr::Table(Vec::new()) };
        let inner = skip_down(source);
        let pairs = inner.kind() == Kind::ArrayLiteralExpression
            && inner.elements().iter().all(|e| {
                let e = skip_down(e);
                e.kind() == Kind::ArrayLiteralExpression && e.elements().len() == 2
            });
        if pairs {
            let mut nodes = Vec::new();
            for e in inner.elements().iter() {
                let e = skip_down(e);
                nodes.push(e.elements().get(0).unwrap());
                nodes.push(e.elements().get(1).unwrap());
            }
            let values = self.ordered(nodes, |l, e| l.expr(e));
            return Expr::Table(values.chunks(2).map(|p| Field::Keyed(p[0].clone(), p[1].clone())).collect());
        }
        let value = self.expr(source);
        let map = self.push_temp(Expr::Table(Vec::new()));
        let (k, v) = (self.temp(), self.temp());
        let entry = Expr::Name(v.clone());
        let body = vec![Stmt::Assign(
            vec![map.clone().index(entry.clone().index(Expr::Number(1.0)))],
            vec![entry.index(Expr::Number(2.0))],
        )];
        self.out.push(Stmt::GenericFor(vec![k, v], vec![value], body));
        map
    }

    fn macro_call(&mut self, m: Macro, n: Node, statement: bool) -> Option<Expr> {
        match m.kind() {
            MacroKind::Call => Some(self.call_macro(m, n)),
            MacroKind::Method => {
                let target = skip_down(n.expression());
                if !matches!(target.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
                    return Some(self.unsupported(n, "a method macro without its object"));
                }
                let object = self.expr(target.expression());
                self.method_macro(m, n, object, statement)
            }
            MacroKind::Crucible => {
                Some(self.unsupported(
                    n,
                    &format!("Crucible's `{}` macro, which lands with the Crucible transform", m.name()),
                ))
            }
            MacroKind::Constructor | MacroKind::Identifier => Some(self.unsupported(n, "this macro call")),
        }
    }

    fn call_macro(&mut self, m: Macro, n: Node) -> Expr {
        let args: Vec<Node> = n.arguments().iter().collect();
        match m {
            Macro::Assert => {
                let Some(&first) = args.first() else { return self.unsupported(n, "assert without arguments") };
                let mut lowered = vec![self.capture(|l| l.condition(first))];
                for &a in &args[1..] {
                    lowered.push(self.capture(|l| l.expr(a)));
                }
                let values = self.order_values(lowered);
                Expr::global("assert").call(values)
            }
            Macro::TypeOf => {
                let values = self.ordered(args, |l, a| l.expr(a));
                Expr::global("typeof").call(values)
            }
            Macro::TypeIs => {
                let mut values = self.ordered(args, |l, a| l.expr(a));
                if values.len() != 2 {
                    return self.unsupported(n, "typeIs without two arguments");
                }
                let name = values.pop().unwrap();
                let value = values.pop().unwrap();
                let function = match &name {
                    Expr::String(s) if PRIMITIVE_TYPES.contains(&s.as_ref()) => "type",
                    _ => "typeof",
                };
                Expr::global(function).call(vec![value]).binary(BinOp::Eq, name)
            }
            Macro::ClassIs => {
                let mut values = self.ordered(args, |l, a| l.expr(a));
                if values.len() != 2 {
                    return self.unsupported(n, "classIs without two arguments");
                }
                let name = values.pop().unwrap();
                let value = values.pop().unwrap();
                value.field("ClassName").binary(BinOp::Eq, name)
            }
            Macro::Identity => {
                let mut values = self.ordered(args, |l, a| l.expr(a));
                values.pop().unwrap_or(Expr::Nil)
            }
            Macro::Range => self.unsupported(n, "$range outside a for-of"),
            Macro::Tuple => self.unsupported(n, "$tuple outside a return"),
            _ => self.unsupported(n, &format!("the `{}` macro", m.name())),
        }
    }

    /// Lowers a method macro on an already lowered receiver.
    pub(super) fn method_macro(&mut self, m: Macro, n: Node, object: Expr, statement: bool) -> Option<Expr> {
        let arg_nodes: Vec<Node> = n.arguments().iter().collect();
        let (args, block) = self.capture(|l| l.ordered(arg_nodes.iter().copied(), |l, a| l.expr(a)));
        let object = if block.is_empty() { object } else { self.push_temp_if_complex(object) };
        self.out.extend(block);
        let mut args = args.into_iter();
        let mut arg = || args.next().unwrap_or(Expr::Nil);
        let length = |e: &Expr| Expr::unary(UnOp::Len, e.clone());
        let value = match m {
            Macro::ArrayLikeSize | Macro::StringSize => length(&object),
            Macro::StringByte
            | Macro::StringFind
            | Macro::StringFormat
            | Macro::StringGmatch
            | Macro::StringGsub
            | Macro::StringLower
            | Macro::StringMatch
            | Macro::StringRep
            | Macro::StringReverse
            | Macro::StringSplit
            | Macro::StringSub
            | Macro::StringUpper => {
                let mut values = vec![object];
                values.extend(std::iter::from_fn(|| args.next()));
                let function: &'static str = match m {
                    Macro::StringByte => "string.byte",
                    Macro::StringFind => "string.find",
                    Macro::StringFormat => "string.format",
                    Macro::StringGmatch => "string.gmatch",
                    Macro::StringGsub => "string.gsub",
                    Macro::StringLower => "string.lower",
                    Macro::StringMatch => "string.match",
                    Macro::StringRep => "string.rep",
                    Macro::StringReverse => "string.reverse",
                    Macro::StringSplit => "string.split",
                    Macro::StringSub => "string.sub",
                    _ => "string.upper",
                };
                Expr::global(function).call(values)
            }
            Macro::ArrayPush | Macro::ArrayUnshift => {
                let values: Vec<Expr> = std::iter::from_fn(|| args.next()).collect();
                if values.is_empty() {
                    return Some(length(&object));
                }
                let single = values.len() == 1 && statement;
                let object = if single { object } else { self.push_temp_if_complex(object) };
                let front = m == Macro::ArrayUnshift;
                let mut inserts: Vec<Expr> = values
                    .into_iter()
                    .map(|v| {
                        let args =
                            if front { vec![object.clone(), Expr::Number(1.0), v] } else { vec![object.clone(), v] };
                        Expr::global("table.insert").call(args)
                    })
                    .collect();
                if front {
                    inserts.reverse();
                }
                if statement {
                    let last = inserts.pop();
                    for call in inserts {
                        self.out.push(Stmt::Call(call));
                    }
                    return last;
                }
                for call in inserts {
                    self.out.push(Stmt::Call(call));
                }
                length(&object)
            }
            Macro::ArrayPop => {
                let object = self.push_temp_if_complex(object);
                if statement {
                    self.out.push(Stmt::Assign(vec![object.clone().index(length(&object))], vec![Expr::Nil]));
                    return None;
                }
                let size = self.push_temp(length(&object));
                let value = self.push_temp(object.clone().index(size.clone()));
                self.out.push(Stmt::Assign(vec![object.index(size)], vec![Expr::Nil]));
                value
            }
            Macro::ArrayShift => Expr::global("table.remove").call(vec![object, Expr::Number(1.0)]),
            Macro::ArrayInsert => {
                let index = offset(arg(), 1.0);
                Expr::global("table.insert").call(vec![object, index, arg()])
            }
            Macro::ArrayRemove => Expr::global("table.remove").call(vec![object, offset(arg(), 1.0)]),
            Macro::ArrayUnorderedRemove => {
                let index = offset(arg(), 1.0);
                let index = self.push_temp_if_complex(index);
                let object = self.push_temp_if_complex(object);
                let size = self.push_temp(length(&object));
                let value = self.push_temp(object.clone().index(index.clone()));
                let body = vec![
                    Stmt::Assign(vec![object.clone().index(index)], vec![object.clone().index(size.clone())]),
                    Stmt::Assign(vec![object.index(size)], vec![Expr::Nil]),
                ];
                self.out.push(Stmt::If(value.clone().binary(BinOp::Ne, Expr::Nil), body, None));
                value
            }
            Macro::ArraySort => {
                let object = if statement { object } else { self.push_temp_if_complex(object) };
                let mut values = vec![object.clone()];
                values.extend(std::iter::from_fn(|| args.next()));
                let call = Expr::global("table.sort").call(values);
                if statement {
                    return Some(call);
                }
                self.out.push(Stmt::Call(call));
                object
            }
            Macro::ArrayClear | Macro::SetClear | Macro::MapClear => {
                let call = Expr::global("table.clear").call(vec![object]);
                if statement {
                    return Some(call);
                }
                self.out.push(Stmt::Call(call));
                Expr::Nil
            }
            Macro::ReadonlyArrayIsEmpty => length(&object).binary(BinOp::Eq, Expr::Number(0.0)),
            Macro::ReadonlyArrayJoin => {
                let separator = match arg() {
                    Expr::Nil => Expr::string(", "),
                    e if e.is_simple() => e,
                    e => {
                        let separator = self.push_temp(e);
                        let default = vec![Stmt::Assign(vec![separator.clone()], vec![Expr::string(", ")])];
                        self.out.push(Stmt::If(separator.clone().binary(BinOp::Eq, Expr::Nil), default, None));
                        separator
                    }
                };
                let object = self.push_temp_if_complex(object);
                let strings = self.push_temp(Expr::global("table.create").call(vec![length(&object)]));
                let (k, v) = (self.temp(), self.temp());
                let body = vec![Stmt::Assign(
                    vec![strings.clone().index(Expr::Name(k.clone()))],
                    vec![Expr::global("tostring").call(vec![Expr::Name(v.clone())])],
                )];
                self.out.push(Stmt::GenericFor(vec![k, v], vec![object], body));
                Expr::global("table.concat").call(vec![strings, separator])
            }
            Macro::ReadonlyArrayMove => {
                let (from, to, at) = (offset(arg(), 1.0), offset(arg(), 1.0), offset(arg(), 1.0));
                let mut values = vec![object, from, to, at];
                if let Some(destination) = args.next() {
                    values.push(destination);
                }
                Expr::global("table.move").call(values)
            }
            Macro::ReadonlyArrayIncludes => {
                let mut values = vec![object, arg()];
                if let Some(from) = args.next() {
                    values.push(offset(from, 1.0));
                }
                Expr::global("table.find").call(values).binary(BinOp::Ne, Expr::Nil)
            }
            Macro::ReadonlyArrayIndexOf => {
                let mut values = vec![object, arg()];
                if let Some(from) = args.next() {
                    values.push(offset(from, 1.0));
                }
                let found = Expr::global("table.find").call(values).binary(BinOp::Or, Expr::Number(0.0));
                found.binary(BinOp::Sub, Expr::Number(1.0))
            }
            Macro::ReadonlyArrayEvery | Macro::ReadonlyArraySome => {
                let every = m == Macro::ReadonlyArrayEvery;
                let object = self.push_temp_if_complex(object);
                let result = self.push_temp(Expr::Bool(every));
                let callback = self.callback(arg());
                let (k, v) = (self.temp(), self.temp());
                let call = callback.call(self.element_args(&k, &v, &object));
                let test = if every { call.negated() } else { call };
                let body = vec![Stmt::If(
                    test,
                    vec![Stmt::Assign(vec![result.clone()], vec![Expr::Bool(!every)]), Stmt::Break],
                    None,
                )];
                self.out.push(Stmt::GenericFor(vec![k, v], vec![object], body));
                result
            }
            Macro::ReadonlyArrayForEach => {
                let object = self.push_temp_if_complex(object);
                let callback = self.callback(arg());
                let (k, v) = (self.temp(), self.temp());
                let body = vec![Stmt::Call(callback.call(self.element_args(&k, &v, &object)))];
                self.out.push(Stmt::GenericFor(vec![k, v], vec![object], body));
                if statement {
                    return None;
                }
                Expr::Nil
            }
            Macro::ReadonlyArrayMap => {
                let object = self.push_temp_if_complex(object);
                let result = self.push_temp(Expr::global("table.create").call(vec![length(&object)]));
                let callback = self.callback(arg());
                let (k, v) = (self.temp(), self.temp());
                let body = vec![Stmt::Assign(
                    vec![result.clone().index(Expr::Name(k.clone()))],
                    vec![callback.call(self.element_args(&k, &v, &object))],
                )];
                self.out.push(Stmt::GenericFor(vec![k, v], vec![object], body));
                result
            }
            Macro::ReadonlyArrayMapFiltered | Macro::ReadonlyArrayFilter => {
                let filter = m == Macro::ReadonlyArrayFilter;
                let object = self.push_temp_if_complex(object);
                let result = self.push_temp(Expr::Table(Vec::new()));
                let callback = self.callback(arg());
                let size = self.push_temp(Expr::Number(0.0));
                let (k, v) = (self.temp(), self.temp());
                let call = callback.call(self.element_args(&k, &v, &object));
                let append = |value: Expr| {
                    vec![
                        Stmt::Compound(BinOp::Add, size.clone(), Expr::Number(1.0)),
                        Stmt::Assign(vec![result.clone().index(size.clone())], vec![value]),
                    ]
                };
                let body = if filter {
                    vec![Stmt::If(call.binary(BinOp::Eq, Expr::Bool(true)), append(Expr::Name(v.clone())), None)]
                } else {
                    let mapped = self.temp();
                    vec![
                        Stmt::Local(vec![mapped.clone()], vec![call]),
                        Stmt::If(
                            Expr::Name(mapped.clone()).binary(BinOp::Ne, Expr::Nil),
                            append(Expr::Name(mapped)),
                            None,
                        ),
                    ]
                };
                self.out.push(Stmt::GenericFor(vec![k, v], vec![object], body));
                result
            }
            Macro::ReadonlyArrayFilterUndefined => {
                let object = self.push_temp_if_complex(object);
                let size = self.push_temp(Expr::Number(0.0));
                let i = self.temp();
                let index = Expr::Name(i.clone());
                let widen = Stmt::If(
                    index.clone().binary(BinOp::Gt, size.clone()),
                    vec![Stmt::Assign(vec![size.clone()], vec![index])],
                    None,
                );
                self.out.push(Stmt::GenericFor(vec![i], vec![object.clone()], vec![widen]));
                let result = self.push_temp(Expr::Table(Vec::new()));
                let count = self.push_temp(Expr::Number(0.0));
                let (i, v) = (self.temp(), self.temp());
                let body = vec![
                    Stmt::Local(vec![v.clone()], vec![object.index(Expr::Name(i.clone()))]),
                    Stmt::If(
                        Expr::Name(v.clone()).binary(BinOp::Ne, Expr::Nil),
                        vec![
                            Stmt::Compound(BinOp::Add, count.clone(), Expr::Number(1.0)),
                            Stmt::Assign(vec![result.clone().index(count)], vec![Expr::Name(v)]),
                        ],
                        None,
                    ),
                ];
                self.out.push(Stmt::NumericFor(i, Expr::Number(1.0), size, None, body));
                result
            }
            Macro::ReadonlyArrayReduce => {
                let object = self.push_temp_if_complex(object);
                let callback = self.callback(arg());
                let initial = args.next();
                let (result, start) = match initial {
                    Some(initial) => (self.push_temp(initial), Expr::Number(1.0)),
                    None => {
                        let empty = Stmt::Call(Expr::global("error").call(vec![Expr::string(
                            "Attempted to call `ReadonlyArray.reduce()` on an empty array without an initialValue.",
                        )]));
                        self.out.push(Stmt::If(
                            length(&object).binary(BinOp::Eq, Expr::Number(0.0)),
                            vec![empty],
                            None,
                        ));
                        (self.push_temp(object.clone().index(Expr::Number(1.0))), Expr::Number(2.0))
                    }
                };
                let i = self.temp();
                let index = Expr::Name(i.clone());
                let call = callback.call(vec![
                    result.clone(),
                    object.clone().index(index.clone()),
                    offset(index, -1.0),
                    object.clone(),
                ]);
                let body = vec![Stmt::Assign(vec![result.clone()], vec![call])];
                self.out.push(Stmt::NumericFor(i, start, length(&object), None, body));
                result
            }
            Macro::ReadonlyArrayFind | Macro::ReadonlyArrayFindIndex => {
                let index = m == Macro::ReadonlyArrayFindIndex;
                let object = self.push_temp_if_complex(object);
                let callback = self.callback(arg());
                let result = if index {
                    self.push_temp(Expr::Number(-1.0))
                } else {
                    let name = self.temp();
                    self.out.push(Stmt::Local(vec![name.clone()], Vec::new()));
                    Expr::Name(name)
                };
                let (k, v) = (self.temp(), self.temp());
                let call = callback.call(self.element_args(&k, &v, &object));
                let found = if index { offset(Expr::Name(k.clone()), -1.0) } else { Expr::Name(v.clone()) };
                let body = vec![Stmt::If(
                    call.binary(BinOp::Eq, Expr::Bool(true)),
                    vec![Stmt::Assign(vec![result.clone()], vec![found]), Stmt::Break],
                    None,
                )];
                self.out.push(Stmt::GenericFor(vec![k, v], vec![object], body));
                result
            }
            Macro::ReadonlySetIsEmpty | Macro::ReadonlyMapIsEmpty => {
                Expr::global("next").call(vec![object]).binary(BinOp::Eq, Expr::Nil)
            }
            Macro::ReadonlySetSize | Macro::ReadonlyMapSize => {
                let size = self.push_temp(Expr::Number(0.0));
                let k = self.temp();
                let body = vec![Stmt::Compound(BinOp::Add, size.clone(), Expr::Number(1.0))];
                self.out.push(Stmt::GenericFor(vec![k], vec![object], body));
                size
            }
            Macro::ReadonlySetHas | Macro::ReadonlyMapHas => object.index(arg()).binary(BinOp::Ne, Expr::Nil),
            Macro::ReadonlyMapGet => object.index(arg()),
            Macro::ReadonlySetForEach | Macro::ReadonlyMapForEach => {
                let object = self.push_temp_if_complex(object);
                let callback = self.callback(arg());
                let (k, v) = (self.temp(), self.temp());
                let (names, call_args) = if m == Macro::ReadonlySetForEach {
                    (vec![v.clone()], vec![Expr::Name(v.clone()), Expr::Name(v), object.clone()])
                } else {
                    (vec![k.clone(), v.clone()], vec![Expr::Name(v), Expr::Name(k), object.clone()])
                };
                let body = vec![Stmt::Call(callback.call(call_args))];
                self.out.push(Stmt::GenericFor(names, vec![object], body));
                if statement {
                    return None;
                }
                Expr::Nil
            }
            Macro::SetAdd | Macro::MapSet => {
                let object = if statement { object } else { self.push_temp_if_complex(object) };
                let key = arg();
                let value = if m == Macro::SetAdd { Expr::Bool(true) } else { arg() };
                self.out.push(Stmt::Assign(vec![object.clone().index(key)], vec![value]));
                if statement {
                    return None;
                }
                object
            }
            Macro::SetDelete | Macro::MapDelete => {
                let key = self.push_temp_if_complex(arg());
                if statement {
                    self.out.push(Stmt::Assign(vec![object.index(key)], vec![Expr::Nil]));
                    return None;
                }
                let object = self.push_temp_if_complex(object);
                let existed = self.push_temp(object.clone().index(key.clone()).binary(BinOp::Ne, Expr::Nil));
                self.out.push(Stmt::Assign(vec![object.index(key)], vec![Expr::Nil]));
                existed
            }
            Macro::PromiseThen => {
                let values: Vec<Expr> = std::iter::from_fn(|| args.next()).collect();
                object.method("andThen", values)
            }
            Macro::NumberIdiv => object.binary(BinOp::FloorDiv, arg()),
            m => {
                let operator = match m.name() {
                    "add" => BinOp::Add,
                    "sub" => BinOp::Sub,
                    "mul" => BinOp::Mul,
                    "div" => BinOp::Div,
                    "idiv" => BinOp::FloorDiv,
                    _ => return Some(self.unsupported(n, &format!("the `{}` macro", m.name()))),
                };
                object.binary(operator, arg())
            }
        };
        Some(value)
    }

    /// A callback the macro loops call, stored unless it is already a name.
    fn callback(&mut self, value: Expr) -> Expr {
        if matches!(value, Expr::Name(_)) { value } else { self.push_temp(value) }
    }

    /// The `(value, index, array)` arguments an array callback receives.
    fn element_args(&self, k: &Name, v: &Name, object: &Expr) -> Vec<Expr> {
        vec![Expr::Name(v.clone()), offset(Expr::Name(k.clone()), -1.0), object.clone()]
    }
}
