use crucible_compiler::extension::{File, Kind, Node, Op};

use crate::Code;
use crate::declarations::Side;
use crate::network::Standard;
use crate::registration::Walk;

struct StoreCall {
    store: Node,
    member: Option<&'static str>,
}

fn unwrap(mut n: Node) -> Node {
    while matches!(n.kind(), Kind::ParenthesizedExpression | Kind::NonNullExpression) {
        n = n.expression();
    }
    n
}

fn member(n: Node) -> Option<&'static str> {
    match n.kind() {
        Kind::PropertyAccessExpression => n.name().map(|name| name.text()),
        Kind::ElementAccessExpression => {
            let argument = n.argument_expression();
            matches!(argument.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral)
                .then(|| argument.text())
        }
        _ => None,
    }
}

fn calls(n: Node, out: &mut Vec<Node>) {
    if n.kind() == Kind::CallExpression {
        out.push(n);
    }
    n.for_each_child(&mut |child| calls(child, out));
}

impl Walk<'_, '_> {
    fn is_store(&mut self, n: Node) -> bool {
        let t = self.cx.type_at(n);
        self.api(t.symbol()) == Some("ProfileStore")
    }

    fn store_call(&mut self, call: Node) -> Option<StoreCall> {
        let callee = unwrap(call.expression());
        if self.is_store(callee) {
            return Some(StoreCall { store: callee, member: None });
        }
        let name = member(callee)?;
        let object = callee.expression();
        self.is_store(object).then_some(StoreCall { store: object, member: Some(name) })
    }

    fn grants(&mut self) -> bool {
        if let Some(granted) = self.state.network.granted {
            return granted;
        }
        let mut granted = false;
        'files: for file in self.cx.files() {
            if file.is_declaration() || file.is_js() {
                continue;
            }
            let mut all = Vec::new();
            calls(file.root(), &mut all);
            for call in all {
                if self.store_call(call).is_some_and(|c| c.member == Some("grant")) {
                    granted = true;
                    break 'files;
                }
            }
        }
        self.state.network.granted = Some(granted);
        granted
    }

    pub(crate) fn profile_store_call(&mut self, call: Node, file: File) {
        if !self.api.exports_name("ProfileStore") {
            return;
        }
        let Some(StoreCall { store, member }) = self.store_call(call) else { return };
        if member == Some("grant") {
            self.grant(call);
        }
        let client = matches!(member, None | Some("loaded")) && call.arguments().is_empty();
        let required = if client { Side::Client } else { Side::Server };
        if self.file_side(file) == Some(required) {
            return;
        }
        let text = store.source_text();
        let message = if client {
            let loaded = if member.is_some() { ".loaded" } else { "" };
            format!("{text}{loaded}() reads the local player's profile; on the server call {text}{loaded}(player).")
        } else {
            let called = member.map_or_else(|| "(player)".to_owned(), |m| format!(".{m}"));
            format!("{text}{called} is server-only; the client reads {text}().")
        };
        self.report(call, Code::Rule(27), &message);
    }

    pub(crate) fn process_receipt(&mut self, n: Node) {
        if n.operator() != Op::Assign {
            return;
        }
        let target = unwrap(n.left());
        if member(target) != Some("ProcessReceipt") {
            return;
        }
        let object = target.expression();
        let t = self.cx.type_at(object);
        if !t.symbol().is_some_and(|s| s.name() == "MarketplaceService" && self.standard(s, Standard::Types)) {
            return;
        }
        self.state.grants.answers_receipts = true;
        if self.api.exports_name("ProfileStore") && self.grants() {
            self.report(
                n,
                Code::Rule(25),
                "Crucible's data module owns ProcessReceipt; register grants with a profile store's grant(productId, ...).",
            );
        }
    }
}
