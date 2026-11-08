use std::borrow::Cow;

use crucible_compiler::ext::{LowerCx, skip_parens_and_assertions};
use crucible_compiler::tree::{Kind, Node};
use crucible_luau::{Expr, Field, Name, Stmt};

use crate::{Check, Class, ClassKind, Dependency, EMIT, Fact, Literal};

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

pub(crate) fn call(cx: &mut LowerCx<'_, '_>, n: Node, fact: &Fact) -> Option<Expr> {
    if n.arguments().iter().last().is_some_and(|a| a.kind() == Kind::SpreadElement) {
        return Some(cx.unsupported(n, "a spread argument to a Crucible macro"));
    }
    Some(match fact {
        Fact::Attribute => {
            let field = n.parent().filter(|p| {
                p.kind() == Kind::PropertyDeclaration
                    && p.initializer().is_some_and(|i| skip_parens_and_assertions(i) == n)
            });
            let Some(name) = field.and_then(|f| f.name()).filter(|name| {
                matches!(name.kind(), Kind::Identifier | Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral)
            }) else {
                return Some(cx.unsupported(n, "attribute() outside a behaviour field initializer"));
            };
            let mut args = vec![Expr::global("self"), string(name.text())];
            args.extend(cx.macro_arguments(n));
            cx.helper(EMIT, "attribute").call(args)
        }
        Fact::Inject(dependency) => {
            let (function, id) = match dependency {
                Dependency::One(id) => ("resolve", id.as_ref()),
                Dependency::All(id) => ("resolveAll", id.as_ref()),
                Dependency::Invalid => return Some(cx.unsupported(n, "inject without a class, interface or All target")),
            };
            cx.helper(EMIT, function).call(vec![string(id)])
        }
        Fact::Boot(loads) => {
            let callee = cx.expr(n.expression());
            for (span, glob) in loads {
                cx.substitute(*span, string(glob));
            }
            let args = cx.macro_arguments(n);
            callee.call(args)
        }
        Fact::Class(_) => return None,
    })
}

/// `__crucible.defineClass(Class, { kind, id, deps, provides, tag, instance, attributes })`, which the runtime's
/// registry reads, after a registered class.
pub(crate) fn register(cx: &mut LowerCx<'_, '_>, class: &Class, name: Name) -> Stmt {
    let ids = |ids: &[Box<str>]| Expr::Table(ids.iter().map(|id| Field::Item(string(id))).collect());
    let kind = match class.kind {
        ClassKind::Subsystem => "subsystem",
        ClassKind::Behaviour => "behaviour",
    };
    let deps = class
        .deps
        .iter()
        .map(|dep| {
            Field::Item(match dep {
                Dependency::One(id) => string(id),
                Dependency::All(id) => Expr::Table(vec![keyed("all", string(id))]),
                Dependency::Invalid => Expr::string("invalid"),
            })
        })
        .collect();
    let mut fields = vec![
        keyed("kind", Expr::string(kind)),
        keyed("id", string(class.id.as_deref().unwrap_or("invalid"))),
        keyed("deps", Expr::Table(deps)),
        keyed("provides", ids(&class.provides)),
    ];
    if class.kind == ClassKind::Behaviour {
        fields.push(keyed("tag", string(class.tag.as_deref().unwrap_or("invalid"))));
        fields.push(keyed("instance", ids(&class.instance)));
        if !class.attributes.is_empty() {
            let mut attributes = Vec::new();
            for attribute in &class.attributes {
                let mut check = self::check(cx, &attribute.check);
                if !attribute.required {
                    check = cx.helper(EMIT, "t").field("optional").call(vec![check]);
                }
                let mut chars = attribute.field.chars();
                let title: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
                let entry = vec![
                    keyed("name", Expr::String(Cow::Owned(title))),
                    keyed("check", check),
                    keyed("required", Expr::Bool(attribute.required)),
                ];
                attributes.push(Field::Keyed(string(&attribute.field), Expr::Table(entry)));
            }
            fields.push(keyed("attributes", Expr::Table(attributes)));
        }
    }
    let call = cx.helper(EMIT, "defineClass").call(vec![Expr::Name(name), Expr::Table(fields)]);
    Stmt::Call(call)
}

fn check(cx: &mut LowerCx<'_, '_>, check: &Check) -> Expr {
    let t = |cx: &mut LowerCx<'_, '_>, name: &'static str| cx.helper(EMIT, "t").field(name);
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
