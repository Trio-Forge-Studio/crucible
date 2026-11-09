use super::Extractor;
use super::report::Code;
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

impl Extractor {
    pub(super) fn class_collisions(&mut self, class: P<Node>) {
        let Some(symbol) = self.checker.get_symbol_of_declaration_exported(class) else { return };
        let constructor = self.checker.get_type_of_symbol_exported(symbol);
        let instance = self.checker.get_declared_type_of_symbol_exported(symbol);
        for &member in class.members() {
            if member.kind() != Kind::MethodDeclaration {
                continue;
            }
            let Some(name) = member.name().filter(|n| matches!(n.kind(), Kind::Identifier | Kind::StringLiteral))
            else {
                continue;
            };
            let static_ = tsrs_ast::get_combined_modifier_flags(member).intersects(tsrs_ast::ModifierFlags::Static);
            let opposite = if static_ { instance } else { constructor };
            if self.checker.get_property_of_type_exported(opposite, name.text()).is_some() {
                self.report(
                    member,
                    Code::Place,
                    if static_ {
                        "static methods cannot use the same name as instance properties"
                    } else {
                        "instance methods cannot use the same name as static properties"
                    },
                );
            }
        }
    }
}
