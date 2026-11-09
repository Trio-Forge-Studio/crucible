//! Lowers what the transform extracted: `inject` and `attribute` calls, each registered class's `defineClass` call,
//! and the network and data macros with their wire schemas.

use std::borrow::Cow;

use crucible_abi::record::{
    ALL, ATTRIBUTES, BEHAVIOUR, CHECK, DEPS, ID, INSTANCE, KIND, NAME, PROVIDES, REQUIRED, SUBSYSTEM, TAG,
};
use crucible_abi::{ATTRIBUTE, DEFINE_CLASS, N, RECEIVE, RESOLVE, RESOLVE_ALL, T};
use crucible_compiler::extension::{Kind, LowerCx, Node};
use crucible_luau::{Expr, Field, Name, Stmt};

use crate::EMIT;
use crate::facts::{Check, Class, ClassKind, Declaration, Dependency, KeyLiteral, Literal, MapKey, Schema, Tail};

fn string(text: &str) -> Expr {
    Expr::String(Cow::Owned(text.to_owned()))
}

fn literal(value: &Literal) -> Expr {
    match value {
        Literal::Boolean(b) => Expr::Bool(*b),
        Literal::Number(n) => Expr::Number(*n),
        Literal::String(s) => string(s),
    }
}

fn keyed(key: &'static str, value: Expr) -> Field {
    Field::Keyed(Expr::string(key), value)
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

/// `t.<name>` of the runtime's attribute checks.
fn t(cx: &mut LowerCx<'_, '_>, name: &'static str) -> Expr {
    cx.helper(&EMIT, T.name).field(name)
}

pub(crate) fn inject(cx: &mut LowerCx<'_, '_>, call: Node, dependency: &Dependency) -> Expr {
    let (function, id) = match dependency {
        Dependency::One(id) => (RESOLVE.name, id.as_ref()),
        Dependency::All(id) => (RESOLVE_ALL.name, id.as_ref()),
        Dependency::Invalid => return cx.unsupported(call, "inject without a class, interface or All target"),
    };
    cx.helper(&EMIT, function).call(vec![string(id)])
}

pub(crate) fn attribute(cx: &mut LowerCx<'_, '_>, call: Node) -> Expr {
    let field = call.parent().filter(|p| {
        p.kind() == Kind::PropertyDeclaration && p.initializer().is_some_and(|i| skip_parens_and_assertions(i) == call)
    });
    let Some(name) = field.and_then(|f| f.name()).filter(|name| {
        matches!(name.kind(), Kind::Identifier | Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral)
    }) else {
        return cx.unsupported(call, "attribute() outside a behaviour field initializer");
    };
    let mut args = vec![Expr::global("self"), string(name.text())];
    args.extend(cx.arguments(call, Vec::new()));
    cx.helper(&EMIT, ATTRIBUTE.name).call(args)
}

/// `defineClass(<class>, <record>)` after a registered class.
pub(crate) fn register(cx: &mut LowerCx<'_, '_>, class: &Class, local: &Name) -> Stmt {
    let ids = |ids: &[Box<str>]| Expr::Table(ids.iter().map(|id| Field::Item(string(id))).collect());
    let kind = match class.kind {
        ClassKind::Subsystem => SUBSYSTEM,
        ClassKind::Behaviour => BEHAVIOUR,
    };
    let deps = class
        .deps
        .iter()
        .map(|dep| {
            Field::Item(match dep {
                Dependency::One(id) => string(id),
                Dependency::All(id) => Expr::Table(vec![keyed(ALL, string(id))]),
                Dependency::Invalid => Expr::string("invalid"),
            })
        })
        .collect();
    let mut fields = vec![
        keyed(KIND, Expr::string(kind)),
        keyed(ID, string(class.id.as_deref().unwrap_or("invalid"))),
        keyed(DEPS, Expr::Table(deps)),
        keyed(PROVIDES, ids(&class.provides)),
    ];
    if class.kind == ClassKind::Behaviour {
        fields.push(keyed(TAG, string(class.tag.as_deref().unwrap_or("invalid"))));
        fields.push(keyed(INSTANCE, ids(&class.instance)));
        if !class.attributes.is_empty() {
            let mut attributes = Vec::new();
            for attribute in &class.attributes {
                let mut check = check(cx, &attribute.check);
                if !attribute.required {
                    check = t(cx, "optional").call(vec![check]);
                }
                let mut chars = attribute.field.chars();
                let title: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
                let entry = vec![
                    keyed(NAME, Expr::String(Cow::Owned(title))),
                    keyed(CHECK, check),
                    keyed(REQUIRED, Expr::Bool(attribute.required)),
                ];
                attributes.push(Field::Keyed(string(&attribute.field), Expr::Table(entry)));
            }
            fields.push(keyed(ATTRIBUTES, Expr::Table(attributes)));
        }
    }
    let call = cx.helper(&EMIT, DEFINE_CLASS.name).call(vec![Expr::Name(local.clone()), Expr::Table(fields)]);
    Stmt::Call(call)
}

fn check(cx: &mut LowerCx<'_, '_>, check: &Check) -> Expr {
    match check {
        Check::Boolean => t(cx, "boolean"),
        Check::String | Check::Invalid => t(cx, "string"),
        Check::Number => t(cx, "number"),
        Check::Vector3 => t(cx, "vector3"),
        Check::Color3 => t(cx, "color3"),
        Check::Literal(value) => t(cx, "literal").call(vec![literal(value)]),
        Check::Union(parts) => {
            let parts = parts.iter().map(|p| self::check(cx, p)).collect();
            t(cx, "union").call(parts)
        }
    }
}

/// `receive(self, field, ...arguments)` for a receiver field's initializer.
pub(crate) fn receive(cx: &mut LowerCx<'_, '_>, call: Node, field: &str) -> Expr {
    let mut args = vec![Expr::global("self"), string(field)];
    args.extend(cx.arguments(call, Vec::new()));
    cx.helper(&EMIT, RECEIVE.name).call(args)
}

/// `<kind>(name, ...kept arguments, ...schemas, ...tail)`.
pub(crate) fn declaration(cx: &mut LowerCx<'_, '_>, call: Node, declaration: &Declaration) -> Expr {
    let arguments = call.arguments();
    let kept: Vec<Node> = arguments
        .get(0)
        .into_iter()
        .chain(declaration.kind.spec().passed.iter().filter_map(|&i| arguments.get(i)))
        .collect();
    let mut args = cx.some_arguments(&kept);
    for schema in &declaration.schemas {
        args.push(self::schema(cx, schema));
    }
    match &declaration.tail {
        Tail::None => {}
        Tail::Timeout(seconds) => args.push(seconds.map_or(Expr::Nil, Expr::Number)),
        Tail::ServerOnly(paths) => args.push(Expr::Table(
            paths
                .iter()
                .map(|path| Field::Item(Expr::Table(path.iter().map(|s| Field::Item(string(s))).collect())))
                .collect(),
        )),
    }
    cx.helper(&EMIT, declaration.kind.name()).call(args)
}

/// `n.<name>` of the runtime's schema builders.
fn n(cx: &mut LowerCx<'_, '_>, name: &'static str) -> Expr {
    cx.helper(&EMIT, N.name).field(name)
}

fn schema(cx: &mut LowerCx<'_, '_>, schema: &Schema) -> Expr {
    match schema {
        Schema::Nil => n(cx, "nil"),
        Schema::Bool => n(cx, "bool"),
        Schema::F64 => n(cx, "f64"),
        Schema::F32 => n(cx, "f32"),
        Schema::U8 => n(cx, "u8"),
        Schema::U16 => n(cx, "u16"),
        Schema::U32 => n(cx, "u32"),
        Schema::String => n(cx, "string"),
        Schema::Dynamic => n(cx, "dynamic"),
        Schema::Vector3 => n(cx, "vector3"),
        Schema::Color3 => n(cx, "color3"),
        Schema::CFrame => n(cx, "cframe"),
        Schema::Literal(value) => n(cx, "literal").call(vec![literal(value)]),
        Schema::Instance(classes) => {
            let classes = classes.iter().map(|c| Field::Item(string(c))).collect();
            n(cx, "instance").call(vec![Expr::Table(classes)])
        }
        Schema::Object(fields) => {
            let fields = fields
                .iter()
                .map(|f| {
                    Field::Item(Expr::Table(vec![
                        Field::Item(string(&f.name)),
                        Field::Item(self::schema(cx, &f.value)),
                    ]))
                })
                .collect();
            n(cx, "object").call(vec![Expr::Table(fields)])
        }
        Schema::Array(element) => {
            let element = self::schema(cx, element);
            n(cx, "array").call(vec![element])
        }
        Schema::Tuple(elements) => {
            let elements = elements.iter().map(|e| Field::Item(self::schema(cx, e))).collect();
            n(cx, "tuple").call(vec![Expr::Table(elements)])
        }
        Schema::Map(key, value) => {
            let key = match key {
                MapKey::String => n(cx, "string"),
                MapKey::U8 => n(cx, "u8"),
                MapKey::U16 => n(cx, "u16"),
                MapKey::U32 => n(cx, "u32"),
                MapKey::Literals(keys) => {
                    let branches = keys
                        .iter()
                        .map(|k| {
                            let value = match k {
                                KeyLiteral::Number(v) => Expr::Number(*v),
                                KeyLiteral::String(s) => string(s),
                            };
                            Field::Item(n(cx, "literal").call(vec![value]))
                        })
                        .collect();
                    n(cx, "union").call(vec![Expr::Table(branches)])
                }
            };
            let value = self::schema(cx, value);
            n(cx, "map").call(vec![key, value])
        }
        Schema::Union(branches) => {
            let branches = branches.iter().map(|b| Field::Item(self::schema(cx, b))).collect();
            n(cx, "union").call(vec![Expr::Table(branches)])
        }
        Schema::Variant { key, value, body } => {
            let body = self::schema(cx, body);
            n(cx, "variant").call(vec![string(key), literal(value), body])
        }
    }
}
