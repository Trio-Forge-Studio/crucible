use super::{Extractor, crucible::Code, types};
use crate::facts::ClassKind;
use tsrs_ast::{Kind, Node, SymbolFlags};
use tsrs_checker::{ContextFlags, SignatureKind, Type, TypeFlags};
use tsrs_core::P;

#[derive(Clone, Copy)]
enum Rule {
    Truthiness = 32,
    LuaTuple = 33,
    ArrayPairs = 34,
    FrameHooks = 35,
    SessionLifetime = 36,
    StateAsFunction = 37,
}
impl Rule {
    fn legacy(self) -> &'static str {
        match self {
            Self::Truthiness => "roblox-ts/lua-truthiness",
            Self::LuaTuple => "roblox-ts/misleading-lua-tuple-checks",
            Self::ArrayPairs => "roblox-ts/no-array-pairs",
            Self::FrameHooks => "fragile-delivery/frame-hooks",
            Self::SessionLifetime => "fragile-delivery/session-lifetime",
            Self::StateAsFunction => "fragile-delivery/state-as-function",
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
        let before = &self.text[..start];
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let previous = self.text[..line_start]
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
                self.lint_frame(n);
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
            Kind::ClassDeclaration => self.lint_session(n),
            _ => {}
        }
        if matches!(n.kind(), Kind::Identifier | Kind::PropertyAccessExpression) {
            self.lint_state(n);
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
    fn lint_lineage(&mut self, n: P<Node>, seen: &mut Vec<P<Node>>) -> Option<ClassKind> {
        if seen.contains(&n) {
            return None;
        }
        seen.push(n);
        for &h in n.as_class_declaration().heritage_clauses().map_or(&[][..], |l| l.nodes()) {
            if h.as_heritage_clause().token != Kind::ExtendsKeyword {
                continue;
            }
            let e = h.as_heritage_clause().types.get().nodes().first()?.expression()?;
            let mut s = self.checker.get_symbol_at_location_exported(e)?;
            if s.flags().intersects(SymbolFlags::Alias) {
                s = self.checker.get_aliased_symbol(s);
            }
            for &d in s.declarations() {
                if d.kind() != Kind::ClassDeclaration {
                    continue;
                }
                let file = tsrs_ast::get_source_file_of_node(d)?.file_name().replace('\\', "/");
                if file.contains("/node_modules/@trioforge/crucible/") {
                    match d.name().map(|n| n.as_identifier().text()) {
                        Some("Subsystem") => return Some(ClassKind::Subsystem),
                        Some("CrucibleBehaviour") => return Some(ClassKind::Behaviour),
                        _ => {}
                    }
                }
                if let Some(k) = self.lint_lineage(d, seen) {
                    return Some(k);
                }
            }
        }
        None
    }
    fn lint_frame(&mut self, n: P<Node>) {
        let e = n.expression().unwrap();
        if e.kind() != Kind::PropertyAccessExpression
            || !matches!(self.lint_text(e.name().unwrap()), "Connect" | "Once" | "ConnectParallel")
        {
            return;
        }
        let signal = e.expression().unwrap();
        if signal.kind() != Kind::PropertyAccessExpression
            || signal.expression().unwrap().kind() != Kind::Identifier
            || self.lint_text(signal.expression().unwrap()) != "RunService"
        {
            return;
        }
        let name = self.lint_text(signal.name().unwrap()).to_owned();
        if !matches!(
            name.as_str(),
            "Heartbeat"
                | "PostSimulation"
                | "Stepped"
                | "PreSimulation"
                | "RenderStepped"
                | "PreRender"
                | "PreAnimation"
        ) {
            return;
        }
        let mut p = n.parent();
        while p.is_some_and(|p| !tsrs_ast::is_function_like(p)) {
            p = p.unwrap().parent();
        }
        let Some(method) = p else {
            return;
        };
        if method.kind() != Kind::Constructor
            && (method.kind() != Kind::MethodDeclaration
                || !matches!(self.lint_text(method.name().unwrap()), "awake" | "start"))
        {
            return;
        }
        let Some(owner) = method.parent().filter(|p| p.kind() == Kind::ClassDeclaration) else {
            return;
        };
        let mut chain = Vec::new();
        let Some(kind) = self.lint_lineage(owner, &mut chain) else {
            return;
        };
        if kind == ClassKind::Behaviour
            && let Some(p) = parent(n).filter(|p| p.kind() == Kind::BinaryExpression)
        {
            let b = p.as_binary_expression();
            let target = b.left;
            if b.operator_token.kind() == Kind::EqualsToken
                && unwrap(b.right()) == n
                && target.kind() == Kind::PropertyAccessExpression
                && target.expression().unwrap().kind() == Kind::ThisKeyword
            {
                let field = self.lint_text(target.name().unwrap());
                if chain.iter().flat_map(|c| c.members()).any(|m| {
                    m.kind() == Kind::MethodDeclaration && self.lint_text(m.name().unwrap()) == "onDestroy" && {
                        let text = self.lint_text(*m);
                        text.contains(&format!("this.{field}.Disconnect()"))
                            || text.contains(&format!("this.{field}?.Disconnect()"))
                    }
                }) {
                    return;
                }
            }
        }
        self.lint_report(n, Rule::FrameHooks, &format!("This {name} connection lasts the object's whole life. Implement Crucible's update, fixedUpdate, lateUpdate or preRender instead."));
    }
    fn lint_session(&mut self, n: P<Node>) {
        if self.lint_lineage(n, &mut Vec::new()) != Some(ClassKind::Subsystem) {
            return;
        }
        for &m in n.members() {
            if m.kind() == Kind::MethodDeclaration && self.lint_text(m.name().unwrap()) == "destroy" {
                self.lint_report(m, Rule::SessionLifetime, "Crucible never destroys a subsystem. Do last writes in onShutdown, and give other lifetimes their own owner.");
            }
            if m.kind() == Kind::PropertyDeclaration
                && let Some(v) = m.initializer().map(unwrap)
                && matches!(v.kind(), Kind::NewExpression | Kind::CallExpression)
                && let Some(e) = v.expression()
                && e.kind() == Kind::Identifier
                && self.lint_text(e) == "Janitor"
            {
                self.lint_report(
                    m,
                    Rule::SessionLifetime,
                    "A subsystem lives for the whole session. Put the Janitor on the lifetime that ends.",
                );
            }
        }
    }
    fn lint_crucible_type(t: P<Type>) -> Option<&'static str> {
        let s = t.alias().and_then(|a| a.symbol()).or_else(|| t.symbol())?;
        let d = *s.declarations().first()?;
        let f = tsrs_ast::get_source_file_of_node(d)?.file_name().replace('\\', "/");
        f.contains("/node_modules/@trioforge/crucible/").then(|| s.name())
    }
    fn lint_state(&mut self, n: P<Node>) {
        let Some(p) = n.parent() else {
            return;
        };
        if (matches!(p.kind(), Kind::CallExpression | Kind::PropertyAccessExpression) && p.expression() == Some(n))
            || (matches!(p.kind(), Kind::PropertyAssignment | Kind::JsxAttribute) && p.name() == Some(n))
            || (p.kind() == Kind::PropertyAccessExpression && p.expression() != Some(n))
        {
            return;
        }
        let t = self.type_of(n);
        if Self::lint_crucible_type(t) != Some("State") {
            return;
        }
        let Some(t) = self.checker.get_contextual_type_exported(n, ContextFlags::None) else {
            return;
        };
        let members = if t.flags().intersects(TypeFlags::Union) { t.types().to_vec() } else { vec![t] };
        if members.iter().any(|&t| Self::lint_crucible_type(t).is_some()) {
            return;
        }
        if members.iter().any(|&t| !self.checker.get_signatures_of_type_exported(t, SignatureKind::Call).is_empty()) {
            self.lint_report(n, Rule::StateAsFunction, "A Crucible State is a callable table, not a function, so code that tests type(x) == \"function\", such as Vide, treats it as a constant. Pass () => state(), or type the receiver as Readable.");
        }
    }
}
