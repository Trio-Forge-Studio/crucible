//! Crucible's data rules. CRU025 leaves `ProcessReceipt` to the data module once a profile store registers a grant, and
//! CRU027 keeps each profile store call on the side that may make it. The diagnostics equal the ones
//! `packages/crucible/transformer-src/data.cjs` reports.

use tsrs_ast::{Kind, Node, SourceFile};
use tsrs_core::P;

use super::Extractor;
use super::crucible::Code;
use super::network::Standard;
use crate::declarations::Side;

/// A call to a profile store, or to one of its methods.
struct StoreCall {
    store: P<Node>,
    member: Option<&'static str>,
}

fn unwrap(mut n: P<Node>) -> P<Node> {
    while matches!(n.kind(), Kind::ParenthesizedExpression | Kind::NonNullExpression) {
        let Some(inner) = n.expression() else { break };
        n = inner;
    }
    n
}

/// The property a constant property or element access reads.
fn member(n: P<Node>) -> Option<&'static str> {
    match n.kind() {
        Kind::PropertyAccessExpression => Some(n.as_property_access_expression().name.text()),
        Kind::ElementAccessExpression => {
            let argument = n.as_element_access_expression().argument_expression;
            matches!(argument.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral)
                .then(|| argument.text())
        }
        _ => None,
    }
}

fn calls(n: P<Node>, out: &mut Vec<P<Node>>) {
    if n.kind() == Kind::CallExpression {
        out.push(n);
    }
    n.for_each_child(&mut |child| {
        calls(child, out);
        false
    });
}

impl Extractor {
    fn is_store(&mut self, n: P<Node>) -> bool {
        let t = self.checker.get_type_at_location(n);
        self.api(t.symbol()) == Some("ProfileStore")
    }

    fn store_call(&mut self, call: P<Node>) -> Option<StoreCall> {
        let callee = unwrap(call.as_call_expression().expression);
        if self.is_store(callee) {
            return Some(StoreCall { store: callee, member: None });
        }
        let name = member(callee)?;
        let object = callee.expression()?;
        self.is_store(object).then_some(StoreCall { store: object, member: Some(name) })
    }

    /// Whether any profile store call in the program registers a `grant`.
    fn grants(&mut self) -> bool {
        if let Some(granted) = self.network.granted {
            return granted;
        }
        let mut granted = false;
        'files: for &file in self.crucible.program.source_files() {
            if file.is_declaration_file() || file.is_js() {
                continue;
            }
            let mut all = Vec::new();
            calls(file.as_node(), &mut all);
            for call in all {
                if self.store_call(call).is_some_and(|c| c.member == Some("grant")) {
                    granted = true;
                    break 'files;
                }
            }
        }
        self.network.granted = Some(granted);
        granted
    }

    /// CRU027: reading the local player's profile is the client's, and every other store call is the server's.
    pub(super) fn profile_store_call(&mut self, call: P<Node>, file: P<SourceFile>) {
        if !self.crucible.exports_name("ProfileStore") {
            return;
        }
        let Some(StoreCall { store, member }) = self.store_call(call) else { return };
        let client = matches!(member, None | Some("loaded")) && call.arguments().is_empty();
        let required = if client { Side::Client } else { Side::Server };
        if self.file_side(file) == Some(required) {
            return;
        }
        let span = self.span(store);
        let text = &self.text[span.start as usize..span.end as usize];
        let message = if client {
            let loaded = if member.is_some() { ".loaded" } else { "" };
            format!("{text}{loaded}() reads the local player's profile; on the server call {text}{loaded}(player).")
        } else {
            let called = member.map_or_else(|| "(player)".to_owned(), |m| format!(".{m}"));
            format!("{text}{called} is server-only; the client reads {text}().")
        };
        self.report(call, Code::Rule(27), &message);
    }

    /// CRU025 at an assignment to `MarketplaceService.ProcessReceipt` in a program whose profile stores grant products.
    pub(super) fn process_receipt(&mut self, n: P<Node>) {
        let binary = n.as_binary_expression();
        if binary.operator_token.kind() != Kind::EqualsToken || !self.crucible.exports_name("ProfileStore") {
            return;
        }
        let target = unwrap(binary.left);
        if member(target) != Some("ProcessReceipt") {
            return;
        }
        let Some(object) = target.expression() else { return };
        let t = self.checker.get_type_at_location(object);
        let marketplace =
            t.symbol().is_some_and(|s| s.name() == "MarketplaceService" && self.standard(s, Standard::Types));
        if marketplace && self.grants() {
            self.report(
                n,
                Code::Rule(25),
                "Crucible's data module owns ProcessReceipt; register grants with a profile store's grant(productId, ...).",
            );
        }
    }
}
