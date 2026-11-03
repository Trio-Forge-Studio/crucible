use std::borrow::Cow;

use super::{Lower, skip_parens_and_assertions};
use crate::facts::{Check, Class, ClassKind, Crucible, Dependency, Literal};
use crate::luau::{Expr, Field, Name, Stmt};
use crate::macros::Macro;
use crate::tree::{Kind, Node};

fn string(text: &str) -> Expr {
    Expr::String(Cow::Owned(text.to_owned()))
}

fn keyed(key: &'static str, value: Expr) -> Field {
    Field::Keyed(Expr::string(key), value)
}

impl Lower<'_> {
    fn runtime(&mut self, name: &'static str) -> Expr {
        self.uses_crucible = true;
        Expr::global("__crucible").field(name)
    }

    /// Crucible's `attribute`, `inject` and `boot`, which call `@trioforge/crucible/emit` or the side entry's `boot`.
    pub(super) fn crucible_macro(&mut self, m: Macro, n: Node) -> Expr {
        match m {
            Macro::Attribute => {
                let field = n.parent().filter(|p| {
                    p.kind() == Kind::PropertyDeclaration
                        && p.initializer().is_some_and(|i| skip_parens_and_assertions(i) == n)
                });
                let Some(name) = field.and_then(|f| f.name()).filter(|name| {
                    matches!(name.kind(), Kind::Identifier | Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral)
                }) else {
                    return self.unsupported(n, "attribute() outside a behaviour field initializer");
                };
                let mut args = vec![Expr::global("self"), string(name.text())];
                args.extend(self.macro_arguments(n));
                self.runtime("attribute").call(args)
            }
            Macro::Inject => {
                let (function, id) = match self.fact(n).and_then(|f| f.crucible.as_deref()) {
                    Some(Crucible::Inject(Dependency::One(id))) => ("resolve", id.as_ref()),
                    Some(Crucible::Inject(Dependency::All(id))) => ("resolveAll", id.as_ref()),
                    _ => return self.unsupported(n, "inject without a class, interface or All target"),
                };
                self.runtime(function).call(vec![string(id)])
            }
            Macro::Boot => {
                let callee = self.expr(n.expression());
                let args = self.macro_arguments(n);
                callee.call(args)
            }
            _ => self.unsupported(n, &format!("the `{}` macro", m.name())),
        }
    }

    fn macro_arguments(&mut self, n: Node) -> Vec<Expr> {
        let mut values = self.ordered(n.arguments().iter(), |l, a| l.expr(a));
        if let Some(last) = values.pop() {
            values.push(if last.is_multi() { Expr::Paren(Box::new(last)) } else { last });
        }
        values
    }

    /// `__crucible.defineClass(Class, { kind, id, deps, provides, tag, instance, attributes })`, which the runtime's
    /// registry reads, after a registered class.
    pub(super) fn register(&mut self, class: &Class, name: Name) {
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
                    let mut check = self.check(&attribute.check);
                    if !attribute.required {
                        check = self.runtime("t").field("optional").call(vec![check]);
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
        let call = self.runtime("defineClass").call(vec![Expr::Name(name), Expr::Table(fields)]);
        self.current_block.push(Stmt::Call(call));
    }

    fn check(&mut self, check: &Check) -> Expr {
        let t = |l: &mut Self, name: &'static str| l.runtime("t").field(name);
        match check {
            Check::Boolean => t(self, "boolean"),
            Check::String | Check::Invalid => t(self, "string"),
            Check::Number => t(self, "number"),
            Check::Vector3 => t(self, "vector3"),
            Check::Color3 => t(self, "color3"),
            Check::Literal(literal) => {
                let value = match literal {
                    Literal::Boolean(b) => Expr::Bool(*b),
                    Literal::Number(n) => Expr::Number(*n),
                    Literal::String(s) => string(s),
                };
                t(self, "literal").call(vec![value])
            }
            Check::Union(parts) => {
                let parts = parts.iter().map(|p| self.check(p)).collect();
                t(self, "union").call(parts)
            }
        }
    }
}
