use super::{Extractor, report::Code, types};
use tsrs_ast::{Kind, Node};
use tsrs_checker::{Type, TypeFlags};
use tsrs_core::P;

#[derive(Clone, Copy)]
enum Rule {
    Truthiness = 32,
    LuaTuple = 33,
    ArrayPairs = 34,
}
impl Rule {
    fn legacy(self) -> &'static str {
        match self {
            Self::Truthiness => "roblox-ts/lua-truthiness",
            Self::LuaTuple => "roblox-ts/misleading-lua-tuple-checks",
            Self::ArrayPairs => "roblox-ts/no-array-pairs",
        }
    }
}

fn unwrap(mut n: P<Node>) -> P<Node> {
    while n.kind() == Kind::ParenthesizedExpression {
        n = n.expression().unwrap();
    }
    n
}
fn parent(n: P<Node>) -> Option<P<Node>> {
    let mut p = n.parent();
    while p.is_some_and(|p| p.kind() == Kind::ParenthesizedExpression) {
        p = p.unwrap().parent();
    }
    p
}
fn logical(k: Kind) -> bool {
    matches!(k, Kind::AmpersandAmpersandToken | Kind::BarBarToken | Kind::QuestionQuestionToken)
}
fn is_logical(n: P<Node>) -> bool {
    n.kind() == Kind::BinaryExpression && logical(n.as_binary_expression().operator_token.kind())
}

impl Extractor {
    fn lint_text(&self, n: P<Node>) -> &str {
        let text = tsrs_ast::get_source_file_of_node(n).unwrap().text();
        let span = crate::tree::span(n, text);
        &text[span.start as usize..span.end as usize]
    }
    fn lint_report(&mut self, n: P<Node>, rule: Rule, message: &str) {
        let start = self.span(n).start as usize;
        let before = &self.file.text[..start];
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let previous = self.file.text[..line_start]
            .strip_suffix('\n')
            .unwrap_or("")
            .trim_end_matches('\r')
            .rsplit('\n')
            .next()
            .unwrap_or("");
        if let Some((_, rules)) = previous.split_once("oxlint-disable-next-line") {
            let rules = rules.split("--").next().unwrap().trim().trim_end_matches("*/");
            if rules.split(',').any(|r| r.trim() == rule.legacy()) {
                return;
            }
        }
        self.report(n, Code::Rule(rule as u16), message);
    }
    fn lint_truthy_type(&mut self, t: P<Type>) -> bool {
        let t = self.checker.get_base_constraint_of_type_exported(t).unwrap_or(t);
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().iter().any(|&m| self.lint_truthy_type(m));
        }
        t.flags().intersects(TypeFlags::AnyOrUnknown | TypeFlags::TypeVariable)
            || self.is_defined_type(t)
            || (t.flags().intersects(TypeFlags::NumberLike) && types::literal_number(t).is_none_or(|v| v == 0.0))
            || (t.flags().intersects(TypeFlags::StringLike) && types::literal_string(t).is_none_or(str::is_empty))
    }
    fn lint_truthy(&mut self, n: P<Node>) {
        let t = self.type_of(n);
        if self.lint_truthy_type(t) {
            self.lint_report(
                n,
                Rule::Truthiness,
                "0, NaN, and \"\" are falsy in TS. Compare explicitly, because Luau treats them as truthy.",
            );
        }
    }
    fn lint_tuple(&mut self, n: P<Node>) -> bool {
        let n = unwrap(n);
        let candidate = match n.kind() {
            Kind::Identifier => self.lint_text(n) != "undefined",
            Kind::BinaryExpression => {
                let k = n.as_binary_expression().operator_token.kind();
                logical(k) || k == Kind::CommaToken || tsrs_ast::is_assignment_operator(k)
            }
            Kind::AwaitExpression
            | Kind::CallExpression
            | Kind::ConditionalExpression
            | Kind::PropertyAccessExpression
            | Kind::ElementAccessExpression
            | Kind::TaggedTemplateExpression
            | Kind::AsExpression
            | Kind::NonNullExpression
            | Kind::SatisfiesExpression
            | Kind::TypeAssertionExpression
            | Kind::YieldExpression => true,
            _ => false,
        };
        let t = self.type_of(n);
        let t = self.checker.get_base_constraint_of_type_exported(t).unwrap_or(t);
        candidate && t.alias().and_then(|a| a.symbol()).is_some_and(|s| s.name() == "LuaTuple")
    }
    fn lint_tuple_use(&mut self, n: P<Node>) {
        if self.lint_tuple(n) {
            self.lint_report(n, Rule::LuaTuple, "Unexpected LuaTuple in conditional expression. Add [0].");
        }
    }
    fn lint_tuple_declaration(&mut self, n: P<Node>) {
        if n.kind() == Kind::Identifier {
            self.lint_report(n, Rule::LuaTuple, "Unexpected LuaTuple in declaration, use array destructuring.");
        }
    }
    fn lint_condition(&mut self, n: P<Node>) {
        let n = unwrap(n);
        if !is_logical(n) {
            self.lint_truthy(n);
            self.lint_tuple_use(n);
        }
    }
    fn lint_array(&mut self, t: P<Type>) -> bool {
        let t = self.checker.get_base_constraint_of_type_exported(t).unwrap_or(t);
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().iter().any(|&t| self.lint_array(t));
        }
        !t.flags().intersects(TypeFlags::AnyOrUnknown | TypeFlags::TypeVariable)
            && self.checker.is_array_like_type_exported(t)
    }
    pub(super) fn typed_lint(&mut self, n: P<Node>) {
        match n.kind() {
            Kind::IfStatement | Kind::WhileStatement | Kind::DoStatement => {
                self.lint_condition(n.expression().unwrap())
            }
            Kind::ConditionalExpression => self.lint_condition(n.as_conditional_expression().condition),
            Kind::ForStatement => {
                if let Some(c) = n.as_for_statement().condition {
                    self.lint_condition(c);
                }
            }
            Kind::PrefixUnaryExpression if n.as_prefix_unary_expression().operator == Kind::ExclamationToken => {
                let operand = unwrap(n.as_prefix_unary_expression().operand);
                if !parent(n).is_some_and(|p| {
                    p.kind() == Kind::PrefixUnaryExpression
                        && p.as_prefix_unary_expression().operator == Kind::ExclamationToken
                }) {
                    self.lint_truthy(operand);
                }
                self.lint_tuple_use(operand);
            }
            Kind::BinaryExpression => {
                let b = n.as_binary_expression();
                let k = b.operator_token.kind();
                let l = unwrap(b.left);
                let r = unwrap(b.right());
                if logical(k) {
                    if k != Kind::QuestionQuestionToken {
                        self.lint_truthy(l);
                    } else if parent(n).is_some_and(|p| p.kind() == Kind::IfStatement) {
                        self.lint_truthy(r);
                    }
                    self.lint_tuple_use(l);
                    self.lint_tuple_use(r);
                } else if matches!(
                    k,
                    Kind::EqualsEqualsToken
                        | Kind::EqualsEqualsEqualsToken
                        | Kind::ExclamationEqualsToken
                        | Kind::ExclamationEqualsEqualsToken
                ) {
                    self.lint_tuple_use(l);
                    self.lint_tuple_use(r);
                } else if k == Kind::EqualsToken
                    && l.kind() == Kind::Identifier
                    && self.lint_tuple(r)
                    && !self.lint_tuple(l)
                {
                    self.lint_tuple_declaration(l);
                }
            }
            Kind::VariableDeclaration => {
                if let Some(v) = n.initializer()
                    && self.lint_tuple(v)
                {
                    self.lint_tuple_declaration(n.name().unwrap());
                }
            }
            Kind::ForOfStatement => {
                let e = n.expression().unwrap();
                let t = self.type_of(e);
                if Self::is_lua_tuple_iterable_function(self, t) {
                    let init = n.initializer().unwrap();
                    let name = if init.kind() == Kind::VariableDeclarationList {
                        init.as_variable_declaration_list().declarations().nodes()[0].name().unwrap()
                    } else {
                        init
                    };
                    if name.kind() != Kind::Identifier
                        || name.as_identifier().text() != "_"
                        || self.lint_identifier_used(n, name)
                    {
                        self.lint_tuple_declaration(name);
                    }
                }
            }
            Kind::CallExpression => {
                let c = n.as_call_expression();
                if c.expression.kind() == Kind::Identifier
                    && matches!(self.lint_text(c.expression), "pairs" | "ipairs")
                    && let Some(&a) = c.arguments().nodes().first()
                {
                    let t = self.type_of(a);
                    if self.lint_array(t) {
                        self.lint_report(n, Rule::ArrayPairs, &format!("Do not use Array<T> with {}(). Key values will not be shifted from 1-indexed to 0-indexed.", self.lint_text(c.expression)));
                    }
                }
            }
            _ => {}
        }
    }
    fn lint_identifier_used(&mut self, root: P<Node>, name: P<Node>) -> bool {
        let symbol = self.checker.get_symbol_at_location_exported(name);
        fn walk(x: &mut Extractor, n: P<Node>, name: P<Node>, symbol: Option<P<tsrs_ast::Symbol>>) -> bool {
            if n != name && n.kind() == Kind::Identifier && x.checker.get_symbol_at_location_exported(n) == symbol {
                return true;
            }
            let mut used = false;
            n.for_each_child(&mut |c| {
                used |= walk(x, c, name, symbol);
                used
            });
            used
        }
        walk(self, root, name, symbol)
    }
}
