use std::borrow::Cow;

use super::{Lower, skip_parens_and_assertions};
use crate::declarations::DeclarationKind;
use crate::facts::{
    Check, Class, ClassKind, Crucible, Declaration, Dependency, KeyLiteral, Literal, MapKey, Schema, Tail,
};
use crate::macros::Macro;
use crate::tree::{Kind, Node};
use crucible_luau::{Expr, Field, Name, Stmt};

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

impl Lower<'_> {
    fn runtime(&mut self, name: &'static str) -> Expr {
        self.helper(&super::CRUCIBLE_EMIT, name)
    }

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
            Macro::Receive => {
                let Some(Crucible::Receive { field }) = self.fact(n).and_then(|f| f.crucible.as_deref()) else {
                    return self.unsupported(n, "receive() outside a receiver field initializer");
                };
                let mut args = vec![Expr::global("self"), string(field)];
                args.extend(self.macro_arguments(n));
                self.runtime("receive").call(args)
            }
            _ if DeclarationKind::of(m).is_some() => {
                let Some(Crucible::Declaration(declaration)) = self.fact(n).and_then(|f| f.crucible.as_deref()) else {
                    return self.unsupported(n, &format!("`{}` without its declaration facts", m.name()));
                };
                self.declaration(declaration, n)
            }
            _ => self.unsupported(n, &format!("the `{}` macro", m.name())),
        }
    }

    fn declaration(&mut self, declaration: &Declaration, n: Node) -> Expr {
        let arguments = n.arguments();
        let kept =
            arguments.get(0).into_iter().chain(declaration.kind.spec().passed.iter().filter_map(|&i| arguments.get(i)));
        let mut args = self.ordered(kept, |l, a| l.expr(a));
        for schema in &declaration.schemas {
            args.push(self.schema(schema));
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
        self.runtime(declaration.kind.name()).call(args)
    }

    fn schema(&mut self, schema: &Schema) -> Expr {
        let n = |l: &mut Self, name: &'static str| l.runtime("n").field(name);
        match schema {
            Schema::Nil => n(self, "nil"),
            Schema::Bool => n(self, "bool"),
            Schema::F64 => n(self, "f64"),
            Schema::F32 => n(self, "f32"),
            Schema::U8 => n(self, "u8"),
            Schema::U16 => n(self, "u16"),
            Schema::U32 => n(self, "u32"),
            Schema::String => n(self, "string"),
            Schema::Dynamic => n(self, "dynamic"),
            Schema::Vector3 => n(self, "vector3"),
            Schema::Color3 => n(self, "color3"),
            Schema::CFrame => n(self, "cframe"),
            Schema::Literal(value) => n(self, "literal").call(vec![literal(value)]),
            Schema::Instance(classes) => {
                let classes = classes.iter().map(|c| Field::Item(string(c))).collect();
                n(self, "instance").call(vec![Expr::Table(classes)])
            }
            Schema::Object(fields) => {
                let fields = fields
                    .iter()
                    .map(|f| {
                        Field::Item(Expr::Table(vec![Field::Item(string(&f.name)), Field::Item(self.schema(&f.value))]))
                    })
                    .collect();
                n(self, "object").call(vec![Expr::Table(fields)])
            }
            Schema::Array(element) => {
                let element = self.schema(element);
                n(self, "array").call(vec![element])
            }
            Schema::Tuple(elements) => {
                let elements = elements.iter().map(|e| Field::Item(self.schema(e))).collect();
                n(self, "tuple").call(vec![Expr::Table(elements)])
            }
            Schema::Map(key, value) => {
                let key = match key {
                    MapKey::String => n(self, "string"),
                    MapKey::U8 => n(self, "u8"),
                    MapKey::U16 => n(self, "u16"),
                    MapKey::U32 => n(self, "u32"),
                    MapKey::Literals(keys) => {
                        let branches = keys
                            .iter()
                            .map(|k| {
                                let value = match k {
                                    KeyLiteral::Number(v) => Expr::Number(*v),
                                    KeyLiteral::String(s) => string(s),
                                };
                                Field::Item(n(self, "literal").call(vec![value]))
                            })
                            .collect();
                        n(self, "union").call(vec![Expr::Table(branches)])
                    }
                };
                let value = self.schema(value);
                n(self, "map").call(vec![key, value])
            }
            Schema::Union(branches) => {
                let branches = branches.iter().map(|b| Field::Item(self.schema(b))).collect();
                n(self, "union").call(vec![Expr::Table(branches)])
            }
            Schema::Variant { key, value, body } => {
                let body = self.schema(body);
                n(self, "variant").call(vec![string(key), literal(value), body])
            }
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
            Check::Literal(value) => t(self, "literal").call(vec![literal(value)]),
            Check::Union(parts) => {
                let parts = parts.iter().map(|p| self.check(p)).collect();
                t(self, "union").call(parts)
            }
        }
    }
}
