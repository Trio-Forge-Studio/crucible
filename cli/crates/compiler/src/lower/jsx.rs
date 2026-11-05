use std::borrow::Cow;

use super::Lower;
use crate::facts::{Jsx, Prop};
use crate::luau::{Block, Expr, Field, Stmt};
use crate::tree::{Kind, Node};

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
        node: Node,
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
                    Attribute::Named { node: a, name: a.jsx_attribute_name().jsx_name(), value }
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
            Some(Jsx::Crucible) => self.crucible_element(n, Element::read(n)),
            _ => self.unsupported(
                n,
                "JSX without `jsx: \"react\"` and a `jsxFactory`, or `jsx: \"react-jsx\"` and `jsxImportSource: \"@trioforge/crucible/ui\"`",
            ),
        }
    }

    fn ui(&mut self, name: &'static str) -> Expr {
        self.uses_ui = true;
        Expr::global("__ui").field(name)
    }

    fn crucible_element(&mut self, n: Node, element: Element) -> Expr {
        match element.tag {
            Tag::Intrinsic(_) => self.instance_element(n, element),
            Tag::Component(tag) => self.component_element(n, tag, element),
            Tag::Fragment => {
                let values = self.child_values(n, element.children, false);
                Expr::Table(values.into_iter().map(Field::Item).collect())
            }
        }
    }

    /// Creates the instance, sets, binds or connects each attribute by its `Prop` fact, then parents each child.
    fn instance_element(&mut self, n: Node, element: Element) -> Expr {
        let tag = n.jsx_tag().expect("an intrinsic element has a tag");
        let Some(class) = self.fact(tag).and_then(|f| f.instance_class.as_deref()) else {
            return self.unsupported(tag, "an intrinsic element that is not a creatable instance class");
        };
        let new = Expr::global("Instance.new").call(vec![Expr::String(Cow::Owned(class.to_owned()))]);
        let instance = self.push_temp(new);
        for attribute in element.attributes {
            let (node, name, value) = match attribute {
                Attribute::Named { node, name, value } => (node, name, value),
                Attribute::Spread(source) => {
                    self.unsupported(source, "a spread attribute on an intrinsic element");
                    continue;
                }
            };
            let Some(prop) = self.fact(node).and_then(|f| f.prop) else {
                self.error(node, "the type facts do not say how this attribute lowers");
                continue;
            };
            let value = self.attribute_value(value);
            let statement = match prop {
                Prop::Fixed => Stmt::Assign(vec![instance.clone().field(name)], vec![value]),
                Prop::Reactive => {
                    let property = Expr::String(Cow::Owned(name));
                    Stmt::Call(self.ui("bind").call(vec![instance.clone(), property, value]))
                }
                Prop::Event => Stmt::Call(instance.clone().field(name).method("Connect", vec![value])),
                Prop::Children => Stmt::Call(self.ui("append").call(vec![instance.clone(), value])),
            };
            self.current_block.push(statement);
        }
        for child in element.children {
            match child {
                Child::Text(_) => self.error(n, "text in an intrinsic element is not supported; set its Text"),
                Child::Value(c) if c.jsx_tag().is_some_and(|t| t.is_intrinsic_tag()) => {
                    let value = self.expr(c);
                    self.current_block.push(Stmt::Assign(vec![value.field("Parent")], vec![instance.clone()]));
                }
                Child::Value(c) | Child::Spread(c) => {
                    let value = self.expr(c);
                    let append = self.ui("append").call(vec![instance.clone(), value]);
                    self.current_block.push(Stmt::Call(append));
                }
            }
        }
        instance
    }

    /// `Component(props)`, where `props.children` is the only child, or an array of several.
    fn component_element(&mut self, n: Node, tag: Node, element: Element) -> Expr {
        let mut lowered = vec![self.capture(|l| l.expr(tag)), self.capture(|l| l.attribute_table(&element.attributes))];
        if !element.children.is_empty() {
            lowered.push(self.capture(|l| {
                let mut values = l.child_values(n, element.children, true);
                match values.len() {
                    1 => values.pop().unwrap_or(Expr::Nil),
                    _ => Expr::Table(values.into_iter().map(Field::Item).collect()),
                }
            }));
        }
        let mut values = self.order_values(lowered).into_iter();
        let (callee, props, children) = (values.next().unwrap_or(Expr::Nil), values.next(), values.next());
        let props = match (props.unwrap_or(Expr::Nil), children) {
            (Expr::Table(mut fields), Some(children)) => {
                fields.push(Field::Keyed(Expr::string("children"), children));
                Expr::Table(fields)
            }
            (props, Some(children)) => {
                self.current_block.push(Stmt::Assign(vec![props.clone().field("children")], vec![children]));
                props
            }
            (props, None) => props,
        };
        callee.call(vec![props])
    }

    fn child_values(&mut self, n: Node, children: Vec<Child>, text: bool) -> Vec<Expr> {
        let lowered = children
            .into_iter()
            .map(|child| {
                self.capture(|l| match child {
                    Child::Text(t) if text => Expr::String(Cow::Owned(t)),
                    Child::Text(_) => l.unsupported(n, "text in a fragment"),
                    Child::Value(c) => l.expr(c),
                    Child::Spread(c) => l.unsupported(c, "a spread child here"),
                })
            })
            .collect();
        self.order_values(lowered)
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
            Some(value) if value.kind() == Kind::StringLiteral => {
                Expr::String(Cow::Owned(crate::entities::decode(value.text()).into_owned()))
            }
            Some(value) => self.expr(value),
            None => Expr::Bool(true),
        }
    }
}
