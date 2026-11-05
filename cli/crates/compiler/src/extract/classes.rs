use super::crucible::Code;
use super::{Extractor, types};
use crate::classes::Style;
use crate::facts::{ClassStyle, Prop, Site, Span};
use std::collections::BTreeSet;
use tsrs_ast::{Kind, Node, NodeFlags, SymbolFlags};
use tsrs_checker::Type;
use tsrs_core::P;

impl Extractor {
    pub(super) fn component_classes(&mut self, attributes: P<Node>) {
        if self.catalog.is_none() {
            return;
        }
        for &attribute in attributes.as_jsx_attributes().properties().nodes() {
            if attribute.kind() == Kind::JsxAttribute
                && tsrs_ast::entity_name_to_string(attribute.as_jsx_attribute().name, None) == "className"
            {
                self.report(
                    class_value(attribute).unwrap_or(attribute.as_jsx_attribute().name),
                    Code::Rule(29),
                    "className on components is not supported until class forwarding exists",
                );
            }
        }
    }

    fn constant_name(&mut self, n: P<Node>) -> bool {
        if !matches!(n.kind(), Kind::Identifier | Kind::PropertyAccessExpression) {
            return false;
        }
        let Some(mut symbol) = self.checker.get_symbol_at_location_exported(n) else { return false };
        if symbol.flags().intersects(SymbolFlags::Alias) {
            symbol = self.checker.get_aliased_symbol(symbol);
        }
        if symbol.declarations().iter().any(|d| matches!(d.kind(), Kind::GetAccessor | Kind::SetAccessor)) {
            return false;
        }
        if symbol.declarations().iter().any(|d| {
            matches!(d.kind(), Kind::VariableDeclaration | Kind::BindingElement)
                && tsrs_ast::get_combined_node_flags(*d).intersects(NodeFlags::Const)
        }) {
            return true;
        }
        n.kind() == Kind::PropertyAccessExpression
            && self.checker.is_readonly_symbol_exported(symbol)
            && self.constant_name(n.expression().unwrap())
    }

    pub(super) fn class_attribute(
        &mut self,
        element: P<Node>,
        instance: P<Type>,
        class: &str,
        attribute: P<Node>,
        explicit: &BTreeSet<String>,
    ) {
        if self.catalog.is_none() {
            return;
        }
        let Some(value) = class_value(attribute) else {
            self.report(
                attribute.as_jsx_attribute().name,
                Code::Rule(29),
                "className requires a string literal or a constant with one string-literal type",
            );
            return;
        };
        let inline = matches!(value.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral);
        let known = if inline {
            let span = self.span(value);
            let raw = &self.text[span.start as usize..span.end as usize];
            let jsx = attribute.as_jsx_attribute().initializer == Some(value);
            let (text, map) = decode_literal(raw, jsx);
            Some((text, Some(map), span.start))
        } else if self.constant_name(value) {
            let t = self.type_of(value);
            types::literal_string(t).map(|text| (text.to_owned(), None, self.span(value).start))
        } else {
            None
        };
        let Some((text, map, start)) = known else {
            self.report(
                value,
                Code::Rule(29),
                "className requires a string literal or a const name or readonly property with one string-literal type",
            );
            return;
        };
        let mut values = Vec::new();
        let gui = self
            .checker
            .resolve_name_exported("GuiObject", None, SymbolFlags::Type, false)
            .map(|s| self.checker.get_declared_type_of_symbol_exported(s));
        let fits_gui = gui.is_some_and(|gui| self.checker.is_type_assignable_to_exported(instance, gui));
        let mut offset = 0;
        for token in text.split_ascii_whitespace() {
            let at = offset + text[offset..].find(token).unwrap();
            offset = at + token.len();
            let span = match &map {
                Some(map) => Span { start: start + map[at], end: start + map[offset] },
                None => self.span(value),
            };
            let resolved = self.catalog.as_ref().unwrap().lookup(token);
            match resolved {
                Err(message) => {
                    self.report_span(value, span, Code::Rule(28), &format!("{message} (class `{token}`, byte {at})"))
                }
                Ok(writes) => {
                    if !fits_gui {
                        self.report_span(
                            value,
                            span,
                            Code::Rule(31),
                            &format!("`{token}` requires a GuiObject; {class} is not one"),
                        );
                    }
                    let mut missing = BTreeSet::new();
                    for write in &writes {
                        let (owner, property) = write.target().destination();
                        if owner.is_none() && self.checker.get_property_of_type_exported(instance, property).is_none() {
                            missing.insert(property);
                        }
                    }
                    if !missing.is_empty() {
                        self.report_span(
                            value,
                            span,
                            Code::Rule(31),
                            &format!(
                                "`{token}` sets {}, which a {class} does not have",
                                missing.into_iter().collect::<Vec<_>>().join(", ")
                            ),
                        );
                    }
                    values.extend(writes);
                }
            }
        }
        let style = Style::new(values, explicit);
        let kinds: BTreeSet<_> = style.fixed().keys().filter_map(|t| t.destination().0).collect();
        if element.kind() == Kind::JsxElement {
            for &child in element.as_jsx_element().children.nodes() {
                let tag = match child.kind() {
                    Kind::JsxElement => child.as_jsx_element().opening_element.as_jsx_opening_element().tag_name,
                    Kind::JsxSelfClosingElement => child.as_jsx_self_closing_element().tag_name,
                    _ => continue,
                };
                if tag.kind() == Kind::Identifier
                    && let Some(child_class) = self.instance_class(tag.text())
                    && kinds.iter().any(|kind| kind.class() == child_class.name())
                {
                    self.report(
                        tag,
                        Code::Rule(31),
                        &format!("{} is both created by classes and authored as a direct child", child_class.name()),
                    );
                }
            }
        }
        self.table.store(self.span(attribute), Site::Jsx, |f| &mut f.prop, Prop::Class(ClassStyle::new(style)));
    }
}

fn class_value(attribute: P<Node>) -> Option<P<Node>> {
    attribute
        .as_jsx_attribute()
        .initializer
        .and_then(|v| if v.kind() == Kind::JsxExpression { v.as_jsx_expression().expression } else { Some(v) })
}

fn decode_literal(raw: &str, jsx: bool) -> (String, Vec<u32>) {
    let content = &raw[1..raw.len() - 1];
    let mut text = String::new();
    let mut map = Vec::new();
    let mut i = 0;
    while i < content.len() {
        let at = i;
        let rest = &content[i..];
        let mut c = rest.chars().next().unwrap();
        i += c.len_utf8();
        if jsx && c == '&' {
            if let Some(end) = rest.find(';')
                && let Some(decoded) = crate::tree::decode_entity(&rest[1..end])
            {
                c = decoded;
                i = at + end + 1;
            }
        } else if !jsx && c == '\\' {
            let escape = content[i..].chars().next().unwrap();
            i += escape.len_utf8();
            c = match escape {
                '\n' => continue,
                '\r' => {
                    if content[i..].starts_with('\n') {
                        i += 1;
                    }
                    continue;
                }
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'v' => '\u{b}',
                '0' => '\0',
                'x' | 'u' => {
                    let hex = if escape == 'u' && content[i..].starts_with('{') {
                        i += 1;
                        let end = content[i..].find('}').unwrap() + i;
                        let hex = &content[i..end];
                        i = end + 1;
                        hex
                    } else {
                        let len = if escape == 'x' { 2 } else { 4 };
                        let hex = &content[i..i + len];
                        i += len;
                        hex
                    };
                    let mut code = u32::from_str_radix(hex, 16).unwrap();
                    if (0xd800..=0xdbff).contains(&code)
                        && content[i..].starts_with("\\u")
                        && let Some(hex) = content.get(i + 2..i + 6)
                        && let Ok(low @ 0xdc00..=0xdfff) = u32::from_str_radix(hex, 16)
                    {
                        code = 0x10000 + (code - 0xd800) * 0x400 + low - 0xdc00;
                        i += 6;
                    }
                    char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER)
                }
                c => c,
            };
        } else if c == '\r' {
            if content[i..].starts_with('\n') {
                i += 1;
            }
            c = '\n';
        }
        text.push(c);
        map.extend(std::iter::repeat_n((at + 1) as u32, c.len_utf8()));
    }
    map.push((raw.len() - 1) as u32);
    (text, map)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_bytes_keep_their_raw_token_origins() {
        for (raw, jsx, token, column) in [
            (r#""flex&#32;bad""#, true, "bad", 11),
            (r#""flex\u0020bad""#, false, "bad", 12),
            (r#"`flex\x20bad`"#, false, "bad", 10),
            ("`flex\n bad`", false, "bad", 8),
            (r#""\u{1f600} bad""#, false, "bad", 12),
            (r#""\uD83D\uDE00 bad""#, false, "bad", 15),
        ] {
            let (text, map) = decode_literal(raw, jsx);
            assert_eq!(map[text.find(token).unwrap()], column - 1, "{raw}");
        }
    }
}
