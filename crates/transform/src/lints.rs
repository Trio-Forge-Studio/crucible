use std::collections::HashSet;

use crucible_abi::PACKAGE;
use crucible_compiler::extension::{ExtractCx, File, Kind, Node, NodeFlags, Op, Symbol, SymbolFlags, Type, TypeFlags};

use crate::Code;
use crate::facts::ClassKind;

const CHARM: &str = "@rbxts/charm";

#[derive(Clone, Copy)]
enum Rule {
    FrameHooks,
    SessionLifetime,
}

impl Rule {
    const fn code(self) -> u16 {
        match self {
            Rule::FrameHooks => 35,
            Rule::SessionLifetime => 36,
        }
    }

    const fn oxlint_name(self) -> &'static str {
        match self {
            Rule::FrameHooks => "fragile-delivery/frame-hooks",
            Rule::SessionLifetime => "fragile-delivery/session-lifetime",
        }
    }
}

fn crucible_call(charm: &str) -> Option<&'static str> {
    match charm {
        "effect" => Some("effect"),
        "effectScope" => Some("scope"),
        "onCleanup" => Some("onCleanup"),
        _ => None,
    }
}

/// Whether CRU038 reads the file, which then reports its own imports and exports.
fn linted(file: File) -> bool {
    !file.is_declaration() && !file.path().contains("/node_modules/")
}

fn unwrap(mut n: Node) -> Node {
    while n.kind() == Kind::ParenthesizedExpression {
        n = n.expression();
    }
    n
}

fn parent(n: Node) -> Option<Node> {
    let mut p = n.parent();
    while let Some(outer) = p.filter(|p| p.kind() == Kind::ParenthesizedExpression) {
        p = outer.parent();
    }
    p
}

pub(crate) struct Lints<'w, 'x> {
    pub(crate) cx: &'w mut ExtractCx<'x>,
    pub(crate) charm: bool,
}

impl Lints<'_, '_> {
    pub(crate) fn file(&mut self, file: File) {
        let charm = self.charm && linted(file);
        for statement in file.root().statements().iter() {
            self.visit(statement, charm);
        }
    }

    fn visit(&mut self, n: Node, charm: bool) {
        if n.is_type_node()
            || matches!(n.kind(), Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration)
            || n.flags().intersects(NodeFlags::Ambient)
        {
            return;
        }
        match n.kind() {
            Kind::CallExpression => self.frame(n),
            Kind::ClassDeclaration => self.session(n),
            Kind::ImportDeclaration | Kind::ImportEqualsDeclaration | Kind::ExportDeclaration if charm => {
                self.charm_bindings(n)
            }
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression if charm => self.charm_read(n),
            Kind::BindingElement if charm => self.charm_binding(n),
            Kind::BinaryExpression if charm && n.operator() == Op::Assign => {
                self.charm_assignment(unwrap(n.left()), n.right());
            }
            _ => {}
        }
        n.for_each_child(&mut |child| self.visit(child, charm));
    }

    fn report(&mut self, n: Node, rule: Rule, message: &str) {
        let text = n.file().text();
        let start = n.span().start as usize;
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let previous = text[..line_start]
            .strip_suffix('\n')
            .unwrap_or("")
            .trim_end_matches('\r')
            .rsplit('\n')
            .next()
            .unwrap_or("");
        if let Some((_, rules)) = previous.split_once("oxlint-disable-next-line") {
            let rules = rules.split("--").next().unwrap_or("").trim().trim_end_matches("*/");
            if rules.split(',').any(|r| r.trim() == rule.oxlint_name()) {
                return;
            }
        }
        let (code, message) = Code::Rule(rule.code()).render(message);
        self.cx.report(n, code, &message);
    }

    fn lineage(&mut self, n: Node, chain: &mut Vec<Node>) -> Option<ClassKind> {
        if chain.contains(&n) {
            return None;
        }
        chain.push(n);
        for clause in n.heritage_clauses().iter() {
            if clause.token() != Kind::ExtendsKeyword {
                continue;
            }
            let base = clause.types().get(0)?.expression();
            let symbol = self.cx.symbol_at(base)?;
            let symbol = self.cx.aliased(symbol);
            for declaration in symbol.declarations() {
                if declaration.kind() != Kind::ClassDeclaration {
                    continue;
                }
                if declaration.file().path().contains("/node_modules/@trioforge/crucible/") {
                    match declaration.name().map(|name| name.text()) {
                        Some("Subsystem") => return Some(ClassKind::Subsystem),
                        Some("CrucibleBehaviour") => return Some(ClassKind::Behaviour),
                        _ => {}
                    }
                }
                if let Some(kind) = self.lineage(declaration, chain) {
                    return Some(kind);
                }
            }
        }
        None
    }

    fn frame(&mut self, n: Node) {
        let e = n.expression();
        if e.kind() != Kind::PropertyAccessExpression
            || !e.name().is_some_and(|name| matches!(name.source_text(), "Connect" | "Once" | "ConnectParallel"))
        {
            return;
        }
        let signal = e.expression();
        if signal.kind() != Kind::PropertyAccessExpression
            || signal.expression().kind() != Kind::Identifier
            || signal.expression().source_text() != "RunService"
        {
            return;
        }
        let Some(name) = signal.name().map(|name| name.source_text()) else { return };
        if !matches!(
            name,
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
        while let Some(outer) = p.filter(|p| !p.is_function_like()) {
            p = outer.parent();
        }
        let Some(method) = p else { return };
        if method.kind() != Kind::Constructor
            && (method.kind() != Kind::MethodDeclaration
                || !method.name().is_some_and(|name| matches!(name.source_text(), "awake" | "start")))
        {
            return;
        }
        let Some(owner) = method.parent().filter(|p| p.kind() == Kind::ClassDeclaration) else { return };
        let mut chain = Vec::new();
        let Some(kind) = self.lineage(owner, &mut chain) else { return };
        if kind == ClassKind::Behaviour
            && let Some(p) = parent(n).filter(|p| p.kind() == Kind::BinaryExpression)
        {
            let target = p.left();
            if p.operator() == Op::Assign
                && unwrap(p.right()) == n
                && target.kind() == Kind::PropertyAccessExpression
                && target.expression().kind() == Kind::ThisKeyword
                && let Some(field) = target.name().map(|name| name.source_text())
            {
                let released = chain.iter().flat_map(|c| c.members().iter().collect::<Vec<_>>()).any(|m| {
                    m.kind() == Kind::MethodDeclaration
                        && m.name().is_some_and(|name| name.source_text() == "onDestroy")
                        && (m.source_text().contains(&format!("this.{field}.Disconnect()"))
                            || m.source_text().contains(&format!("this.{field}?.Disconnect()")))
                });
                if released {
                    return;
                }
            }
        }
        let message = format!(
            "This {name} connection lasts the object's whole life. Implement Crucible's update, fixedUpdate, \
             lateUpdate or preRender instead."
        );
        self.report(n, Rule::FrameHooks, &message);
    }

    fn session(&mut self, n: Node) {
        if self.lineage(n, &mut Vec::new()) != Some(ClassKind::Subsystem) {
            return;
        }
        for m in n.members().iter() {
            if m.kind() == Kind::MethodDeclaration && m.name().is_some_and(|name| name.source_text() == "destroy") {
                self.report(
                    m,
                    Rule::SessionLifetime,
                    "Crucible never destroys a subsystem. Do last writes in onShutdown, and give other lifetimes their \
                     own owner.",
                );
            }
            if m.kind() == Kind::PropertyDeclaration
                && let Some(value) = m.initializer().map(unwrap)
                && matches!(value.kind(), Kind::NewExpression | Kind::CallExpression)
                && value.expression().kind() == Kind::Identifier
                && value.expression().source_text() == "Janitor"
            {
                self.report(
                    m,
                    Rule::SessionLifetime,
                    "A subsystem lives for the whole session. Put the Janitor on the lifetime that ends.",
                );
            }
        }
    }

    fn charm_report(&mut self, at: Node, name: &str) {
        let Some(crucible) = crucible_call(name) else { return };
        let message = format!(
            "{name} from {CHARM} belongs to whatever scope is running when it is called, not to the object whose code \
             called it. Use {crucible} from {PACKAGE}, which ends with its owner."
        );
        let (code, message) = Code::Rule(38).render(&message);
        self.cx.report(at, code, &message);
    }

    /// The Charm lifetime call that `symbol` names, through the aliases that lead to it. The search stops at an alias
    /// a linted file declares, because that file reports its own import or export.
    fn charm_lifetime(&mut self, mut symbol: Symbol) -> Option<&'static str> {
        let mut seen = HashSet::new();
        while symbol.flags().intersects(SymbolFlags::Alias) {
            if !seen.insert(symbol) || symbol.declarations().into_iter().any(|d| linted(d.file())) {
                return None;
            }
            symbol = self.cx.immediate_aliased(symbol)?;
        }
        let name = symbol.name();
        crucible_call(name)?;
        let declarations = symbol.declarations();
        declarations
            .into_iter()
            .any(|d| d.kind() == Kind::FunctionDeclaration && self.cx.package_of(d).as_deref() == Some(CHARM))
            .then_some(name)
    }

    fn charm_bindings(&mut self, n: Node) {
        let mut bindings = Vec::new();
        match n.kind() {
            Kind::ImportDeclaration => {
                let Some(clause) = n.import_clause().filter(|c| !c.is_type_only()) else { return };
                if clause.name().is_some() {
                    bindings.push(clause);
                }
                if let Some(named) = clause.named_bindings().filter(|b| b.kind() == Kind::NamedImports) {
                    bindings.extend(named.elements().iter());
                }
            }
            Kind::ExportDeclaration => {
                let Some(named) = n.export_clause().filter(|c| c.kind() == Kind::NamedExports && !n.is_type_only())
                else {
                    return;
                };
                bindings.extend(named.elements().iter());
            }
            Kind::ImportEqualsDeclaration if !n.is_type_only() => bindings.push(n),
            _ => {}
        }
        for binding in bindings.into_iter().filter(|b| !b.is_type_only()) {
            let Some(alias) = self.cx.symbol_of_declaration(binding) else { continue };
            let Some(target) = self.cx.immediate_aliased(alias) else { continue };
            if let Some(name) = self.charm_lifetime(target) {
                self.charm_report(binding, name);
            }
        }
    }

    fn charm_read(&mut self, n: Node) {
        let names = match n.kind() {
            Kind::PropertyAccessExpression => n.name().map(|name| vec![name.text()]).unwrap_or_default(),
            _ => self.literal_names(n.argument_expression()),
        };
        if let Some(name) = self.property_lifetime(n.expression(), names) {
            self.charm_report(n, name);
        }
    }

    fn charm_binding(&mut self, element: Node) {
        let Some(pattern) = element.parent().filter(|p| p.kind() == Kind::ObjectBindingPattern) else { return };
        let Some(key) = element.property_name_or_name().filter(|_| !element.is_rest()) else { return };
        let names = self.key_names(key);
        if let Some(name) = self.property_lifetime(pattern, names) {
            self.charm_report(element, name);
        }
    }

    fn charm_assignment(&mut self, pattern: Node, source: Node) {
        let source = unwrap(source);
        match pattern.kind() {
            Kind::ObjectLiteralExpression => {
                for member in pattern.properties().iter() {
                    let key = match member.kind() {
                        Kind::ShorthandPropertyAssignment | Kind::PropertyAssignment => member.name(),
                        _ => None,
                    };
                    let Some(key) = key else { continue };
                    let names = self.key_names(key);
                    if let Some(name) = self.property_lifetime(source, names) {
                        self.charm_report(member, name);
                    }
                }
            }
            Kind::ArrayLiteralExpression if source.kind() == Kind::ArrayLiteralExpression => {
                for (target, value) in pattern.elements().iter().zip(source.elements().iter()) {
                    if target.kind() == Kind::SpreadElement || value.kind() == Kind::SpreadElement {
                        break;
                    }
                    self.charm_assignment(unwrap(target), value);
                }
            }
            _ => {}
        }
    }

    fn property_lifetime(&mut self, object: Node, names: Vec<&'static str>) -> Option<&'static str> {
        let object = self.cx.type_at(unwrap(object));
        names.into_iter().find_map(|name| {
            let symbol = self.cx.property_of_type(object, name)?;
            self.charm_lifetime(symbol)
        })
    }

    fn key_names(&mut self, key: Node) -> Vec<&'static str> {
        match key.kind() {
            Kind::Identifier | Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral => vec![key.text()],
            Kind::ComputedPropertyName => self.literal_names(key.expression()),
            _ => Vec::new(),
        }
    }

    fn literal_names(&mut self, n: Node) -> Vec<&'static str> {
        let t = self.cx.type_at(n);
        let members = if t.flags().intersects(TypeFlags::Union) { t.types() } else { vec![t] };
        members.iter().filter_map(Type::literal_string).collect()
    }
}
