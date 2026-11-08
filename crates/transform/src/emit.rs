use super::EMIT;
use super::{Check, Class, Dependency};
use crucible_compiler::extension::LowerCx;
use crucible_compiler::extension::{ClassKind, Literal};
use crucible_luau::{Expr, Field, Name, Stmt};
use std::borrow::Cow;
fn string(text: &str) -> Expr {
    Expr::String(Cow::Owned(text.to_owned()))
}
fn keyed(key: &'static str, value: Expr) -> Field {
    Field::Keyed(Expr::string(key), value)
}
fn literal(value: &Literal) -> Expr {
    match value {
        Literal::Boolean(b) => Expr::Bool(*b),
        Literal::Number(n) => Expr::Number(*n),
        Literal::String(s) => string(s),
    }
}
struct Emitter<'a, 'b, 'c> {
    cx: &'a mut LowerCx<'b, 'c>,
}
pub(super) fn register(cx: &mut LowerCx<'_, '_>, class: &Class, name: Name) {
    Emitter { cx }.register(class, name);
}
impl Emitter<'_, '_, '_> {
    fn runtime(&mut self, name: &'static str) -> Expr {
        self.cx.helper(EMIT, name)
    }
    /// `__crucible.defineClass(Class, { kind, id, deps, provides, tag, instance, attributes })`, which the runtime's
    /// registry reads, after a registered class.
    fn register(&mut self, class: &Class, name: Name) {
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
        self.cx.emit(Stmt::Call(call));
    }

    fn check(&mut self, check: &Check) -> Expr {
        let t = |l: &mut Self, name: &'static str| l.runtime("t").field(name);
        match check {
            Check::Boolean => t(self, "boolean"),
            Check::String | Check::Invalid => t(self, "string"),
            Check::Number => t(self, "number"),
            Check::Vector3 => t(self, "vector3"),
            Check::Color3 => t(self, "color3"),
            Check::Literal(value) => t(self, "literal").call(vec![literal(value)]),
            Check::Union(parts) => {
                let parts = parts.iter().map(|p| self.check(p)).collect();
                t(self, "union").call(parts)
            }
        }
    }
}
