use crucible_abi::PACKAGE;
use crucible_compiler::extension::{ExtractCx, File, Kind, Node, NodeFlags, Op, Symbol};

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
    pub(crate) uses_crucible: bool,
}

impl Lints<'_, '_> {
    pub(crate) fn file(&mut self, file: File) {
        let charm = self.uses_crucible && !file.path().contains("/node_modules/");
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
            Kind::ImportDeclaration | Kind::ExportDeclaration if charm => self.charm_specifiers(n),
            Kind::PropertyAccessExpression if charm => self.charm_access(n),
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

    fn charm_lifetime(&mut self, at: Node, name: &str) {
        let Some(crucible) = crucible_call(name) else { return };
        let message = format!(
            "{name} from {CHARM} belongs to whatever scope is running when it is called, not to the object whose code \
             called it. Use {crucible} from {PACKAGE}, which ends with its owner."
        );
        let (code, message) = Code::Rule(38).render(&message);
        self.cx.report(at, code, &message);
    }

    fn is_charm(&mut self, module: Option<Symbol>) -> bool {
        let Some(module) = module else { return false };
        module.declarations().into_iter().any(|d| self.cx.package_of(d).as_deref() == Some(CHARM))
    }

    fn charm_specifiers(&mut self, n: Node) {
        let Some(specifier) = n.module_specifier().filter(|s| s.kind() == Kind::StringLiteral) else { return };
        let module = self.cx.resolve_external_module(specifier);
        if !self.is_charm(module) {
            return;
        }
        let named = if n.kind() == Kind::ImportDeclaration {
            n.import_clause().filter(|c| !c.is_type_only()).and_then(|c| c.named_bindings())
        } else {
            n.export_clause().filter(|_| !n.is_type_only())
        };
        let Some(named) = named.filter(|b| matches!(b.kind(), Kind::NamedImports | Kind::NamedExports)) else {
            return;
        };
        for element in named.elements().iter() {
            if element.is_type_only() {
                continue;
            }
            if let Some(name) = element.property_name_or_name() {
                self.charm_lifetime(element, name.text());
            }
        }
    }

    fn charm_access(&mut self, n: Node) {
        let Some(name) = n.name() else { return };
        if crucible_call(name.text()).is_none() {
            return;
        }
        let Some(symbol) = self.cx.symbol_at(n.expression()) else { return };
        let module = self.cx.aliased(symbol);
        let is_module = module.declarations().iter().any(|d| d.kind() == Kind::SourceFile);
        if is_module && self.is_charm(Some(module)) {
            self.charm_lifetime(n, name.text());
        }
    }
}
