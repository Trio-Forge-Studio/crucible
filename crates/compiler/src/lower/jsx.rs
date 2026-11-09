use std::borrow::Cow;

use super::Lower;
use crate::facts::Jsx;
use crate::tree::{Kind, Node};
use crucible_luau::{Block, Expr, Field, Stmt};

/// A JSX element or fragment, as each lowering reads it.
struct Element {
    tag: Tag,
    attributes: Vec<Attribute>,
    children: Vec<Child>,
}

enum Tag {
    /// A lowercase or namespaced name, such as `frame`.
    Intrinsic(String),
    /// A value, such as `Button` or `ui.Button`.
    Component(Node),
    Fragment,
}

enum Attribute {
    /// `name="text"`, `name={value}`, or a bare `name`, which is `true`.
    Named {
        name: String,
        value: Option<Node>,
    },
    Spread(Node),
}

enum Child {
    Text(String),
    Value(Node),
    /// `{...values}`, which passes each value as a child.
    Spread(Node),
}

impl Element {
    fn read(n: Node) -> Element {
        let tag = match n.jsx_tag() {
            None => Tag::Fragment,
            Some(tag) if tag.is_intrinsic_tag() => Tag::Intrinsic(tag.jsx_name()),
            Some(tag) => Tag::Component(tag),
        };
        let attributes = n
            .jsx_attributes()
            .iter()
            .map(|a| match a.kind() {
                Kind::JsxSpreadAttribute => Attribute::Spread(a.jsx_spread_expression()),
                _ => {
                    let value = a.jsx_attribute_value().map(|v| match v.kind() {
                        Kind::JsxExpression => v.jsx_expression().unwrap_or(v),
                        _ => v,
                    });
                    Attribute::Named { name: a.jsx_attribute_name().jsx_name(), value }
                }
            })
            .collect();
        let children = n
            .jsx_children()
            .iter()
            .filter_map(|c| match c.kind() {
                Kind::JsxText => c.jsx_text().map(Child::Text),
                Kind::JsxExpression => {
                    let value = c.jsx_expression()?;
                    Some(if c.is_jsx_spread_child() { Child::Spread(value) } else { Child::Value(value) })
                }
                _ => Some(Child::Value(c)),
            })
            .collect();
        Element { tag, attributes, children }
    }
}

/// `Vide.jsx` as a field chain.
fn entity(name: &str) -> Expr {
    let mut parts = name.split('.');
    let first = Expr::Name(Cow::Owned(parts.next().unwrap_or_default().to_owned()));
    parts.fold(first, |e, part| e.field(part.to_owned()))
}

impl Lower<'_> {
    pub(super) fn jsx(&mut self, n: Node) -> Expr {
        match self.facts.jsx.as_ref() {
            Some(Jsx::Factory { factory, fragment }) => {
                self.factory_element(n, Element::read(n), factory, fragment.as_deref())
            }
            _ => self.unsupported(n, "JSX without `jsx: \"react\"` and a `jsxFactory`"),
        }
    }

    /// `factory(tag, attributes, ...children)`, with `nil` for the attributes when there are none but there are
    /// children.
    fn factory_element(&mut self, n: Node, element: Element, factory: &str, fragment: Option<&str>) -> Expr {
        let tag = match element.tag {
            Tag::Intrinsic(name) => (Expr::String(Cow::Owned(name)), Block::new()),
            Tag::Component(tag) => self.capture(|l| l.expr(tag)),
            Tag::Fragment => match fragment {
                Some(fragment) => (entity(fragment), Block::new()),
                None => (self.unsupported(n, "a fragment without a `jsxFragmentFactory`"), Block::new()),
            },
        };
        let mut lowered = vec![tag];
        if !element.attributes.is_empty() {
            lowered.push(self.capture(|l| l.attribute_table(&element.attributes)));
        } else if !element.children.is_empty() {
            lowered.push((Expr::Nil, Block::new()));
        }
        let count = element.children.len();
        for (i, child) in element.children.into_iter().enumerate() {
            lowered.push(self.capture(|l| match child {
                Child::Text(text) => Expr::String(Cow::Owned(text)),
                Child::Value(value) => l.expr(value),
                Child::Spread(value) if i + 1 == count => {
                    let value = l.expr(value);
                    Expr::global("table.unpack").call(vec![value])
                }
                Child::Spread(value) => l.unsupported(value, "a spread child before the last child"),
            }));
        }
        let args = self.order_values(lowered);
        entity(factory).call(args)
    }

    /// The attributes as one table, in source order, with each spread copied in at its place.
    fn attribute_table(&mut self, attributes: &[Attribute]) -> Expr {
        let head = attributes.iter().position(|a| matches!(a, Attribute::Spread(_))).unwrap_or(attributes.len());
        let mut lowered = Vec::new();
        let mut names = Vec::new();
        for attribute in &attributes[..head] {
            if let Attribute::Named { name, value, .. } = attribute {
                names.push(name.clone());
                lowered.push(self.capture(|l| l.attribute_value(*value)));
            }
        }
        let values = self.order_values(lowered);
        let fields =
            names.into_iter().zip(values).map(|(name, value)| Field::Keyed(Expr::String(Cow::Owned(name)), value));
        let table = Expr::Table(fields.collect());
        if head == attributes.len() {
            return table;
        }
        let result = self.push_temp(table);
        for attribute in &attributes[head..] {
            match attribute {
                Attribute::Spread(source) => self.merge_spread(&result, *source),
                Attribute::Named { name, value, .. } => {
                    let value = self.attribute_value(*value);
                    let key = Expr::String(Cow::Owned(name.clone()));
                    self.current_block.push(Stmt::Assign(vec![result.clone().index(key)], vec![value]));
                }
            }
        }
        result
    }

    fn attribute_value(&mut self, value: Option<Node>) -> Expr {
        match value {
            Some(value) if value.kind() == Kind::StringLiteral => self
                .extension_expression(value)
                .unwrap_or_else(|| Expr::String(Cow::Owned(crate::entities::decode(value.text()).into_owned()))),
            Some(value) => self.expr(value),
            None => Expr::Bool(true),
        }
    }
}
