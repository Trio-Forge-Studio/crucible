//! The fact extractor. It walks the syntax tree of every source file, finds each site whose lowering depends on a
//! type, and answers it from the checker that ran the full check. A site is a node where roblox-ts 3.0.0 or Crucible's
//! transform asks the checker a question whose answer changes the emitted Luau. Checks that only report an error are
//! not sites yet.

mod classes;
mod crucible;
mod data;
mod network;
mod typed_lint;
mod types;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use tsrs_ast::{Kind, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::SignatureKind;
use tsrs_checker::{EmitResolver, Type};
use tsrs_compiler::CheckerHandle;
use tsrs_core::{CompilerOptions, JsxEmit, P, jsnum};

use crate::Diagnostic;
use crate::facts::{
    Callee, Fact, Facts, Falsy, FileFacts, Import, Jsx, Prop, Reason, Receiver, RuntimeLib, Site, Span, Unanswered,
};
use crate::macros::Macro;
use crate::program::Checked;
use crate::project::{Game, absolute, installed_module};
use crucible::Code;

const UI_ENTRY: &str = "@trioforge/crucible/ui";

pub(crate) fn extract(checked: Checked, target: crate::Target<'_>) -> Facts {
    let root = checked.source_root();
    let out_dir = checked.out_dir();
    let dir = checked.config_dir();
    let Checked { program, mut checker, host } = checked;
    let resolver = checker.new_emit_resolver(tsrs_printer::new_emit_context());
    let options = program.options();
    let globals = types::Globals::new(&mut checker);
    let crucible = crucible::Api::new(program, &mut checker, host);
    let mut facts = Facts::default();
    let catalog = match crate::classes::Catalog::load(Path::new(&dir)) {
        Ok(catalog) => Some(catalog),
        Err(e) => {
            let (code, message) = crucible::Code::Rule(30).render(&e.message);
            facts.diagnostics.push(Diagnostic { path: Some(e.path), line: e.line, column: e.column, code, message });
            None
        }
    };
    let game = match target {
        crate::Target::Package => None,
        crate::Target::Game { project, include } => {
            match Game::load(project, include, Path::new(&dir), Path::new(&root), out_dir.as_deref().map(Path::new)) {
                Ok(game) => Some(game),
                Err(message) => {
                    facts.diagnostics.push(Diagnostic { path: None, line: 0, column: 0, code: 0, message });
                    None
                }
            }
        }
    };
    if let Some(game) = &game {
        match absolute(&game.runtime_lib) {
            Some(path) => facts.place.runtime_lib = RuntimeLib::Require(path),
            None => facts.diagnostics.push(Diagnostic {
                path: None,
                line: 0,
                column: 0,
                code: 0,
                message: "roblox-ts's runtime library has no instance path".to_owned(),
            }),
        }
        facts.place.crucible_emit =
            crucible.emit.as_deref().and_then(|emit| game.module(emit)).and_then(|r| absolute(&r));
    }
    facts.place.crucible_ui = crucible.ui.as_deref().and_then(|ui| match &game {
        Some(game) => game.module(ui).and_then(|r| absolute(&r)),
        None => installed_module(&ui.to_string_lossy().replace('\\', "/")),
    });
    let mut x = Extractor {
        checker,
        resolver,
        globals,
        crucible,
        game,
        config_dir: PathBuf::from(&dir),
        network: network::Program::default(),
        facts,
        text: "",
        path: String::new(),
        table: Table::default(),
        pending: Vec::new(),
        modules: Vec::new(),
        reported: HashSet::new(),
        reported_classes: HashSet::new(),
        options,
        jsx: None,
        instance_classes: None,
        catalog,
    };
    for &file in program.source_files() {
        if file.is_declaration_file() || file.is_js() {
            continue;
        }
        x.source_file(file);
    }
    x.facts.diagnostics.sort_by(|a, b| (&a.path, a.line, a.column, a.code).cmp(&(&b.path, b.line, b.column, b.code)));
    x.facts
}

/// When an import's module must be reachable in the place.
enum Kept {
    Always,
    /// When any of these bindings stays.
    AnyOf(Vec<Span>),
}

struct ModuleUse {
    specifier: P<Node>,
    kept: Kept,
}

/// The checker does not mark a const enum import as referenced, because TypeScript inlines const enum members.
/// Crucible compiles a const enum as a regular enum, so the import stays if any value refers to it.
struct ConstEnumImport {
    name: &'static str,
    alias: P<Symbol>,
    span: Span,
    used: bool,
}

#[derive(Default)]
struct Table {
    facts: HashMap<Span, Fact>,
    conflicts: Vec<(Span, Site)>,
}

impl Table {
    fn store<T: PartialEq>(&mut self, span: Span, site: Site, field: fn(&mut Fact) -> &mut Option<T>, value: T) {
        let slot = field(self.facts.entry(span).or_default());
        match slot {
            Some(old) if *old != value => self.conflicts.push((span, site)),
            _ => *slot = Some(value),
        }
    }
}

enum Pattern {
    Assigned(P<Node>),
    ForOf,
    Nested,
}

/// How an array or object literal destructures, or `None` where it builds a value.
fn pattern_role(n: P<Node>) -> Option<Pattern> {
    let parent = n.parent()?;
    match parent.kind() {
        Kind::BinaryExpression => {
            let binary = parent.as_binary_expression();
            if binary.operator_token.kind() != Kind::EqualsToken || binary.left != n {
                return None;
            }
            Some(if tsrs_ast::is_assignment_target(parent) {
                Pattern::Nested
            } else {
                Pattern::Assigned(binary.right())
            })
        }
        Kind::ForOfStatement if parent.initializer() == Some(n) => Some(Pattern::ForOf),
        _ => tsrs_ast::is_assignment_target(n).then_some(Pattern::Nested),
    }
}

/// Whether an argument is a call, which can pass several values or none.
fn returns_values(argument: P<Node>) -> bool {
    types::skip_downwards(argument).kind() == Kind::CallExpression
}

fn computed_key(n: P<Node>) -> bool {
    let computed = |key: Option<P<Node>>| key.is_some_and(|key| key.kind() == Kind::ComputedPropertyName);
    match n.kind() {
        Kind::ObjectBindingPattern => n.elements().iter().any(|e| computed(e.property_name())),
        Kind::ObjectLiteralExpression => {
            n.properties().iter().any(|p| p.kind() == Kind::PropertyAssignment && computed(p.name()))
        }
        _ => false,
    }
}

struct Extractor {
    catalog: Option<crate::classes::Catalog>,
    checker: CheckerHandle,
    resolver: P<EmitResolver>,
    globals: types::Globals,
    crucible: crucible::Api,
    game: Option<Game>,
    config_dir: PathBuf,
    network: network::Program,
    facts: Facts,
    text: &'static str,
    path: String,
    table: Table,
    pending: Vec<ConstEnumImport>,
    modules: Vec<ModuleUse>,
    reported: HashSet<(String, u32, Code)>,
    reported_classes: HashSet<(String, Span, Code, String)>,
    options: P<CompilerOptions>,
    jsx: Option<Jsx>,
    /// `CreatableInstances`'s classes by their lowercase name, read at the first intrinsic element.
    instance_classes: Option<HashMap<String, P<Symbol>>>,
}

impl Extractor {
    fn source_file(&mut self, file: P<SourceFile>) {
        self.text = file.text();
        self.path = file.file_name().replace('\\', "/");
        let first = self.facts.unanswered.len();
        let statements = file.as_node().statements();
        let (imports, rest): (Vec<P<Node>>, Vec<P<Node>>) =
            statements.iter().copied().partition(|s| s.kind() == Kind::ImportDeclaration);
        for statement in imports.into_iter().chain(rest) {
            self.visit(statement);
        }
        for pending in std::mem::take(&mut self.pending) {
            let import = if pending.used { Import::Keep } else { Import::Elide };
            self.table.store(pending.span, Site::Import, |f| &mut f.import, import);
        }
        for module in std::mem::take(&mut self.modules) {
            let kept = match &module.kept {
                Kept::Always => true,
                Kept::AnyOf(bindings) => {
                    bindings.iter().any(|b| self.table.facts.get(b).and_then(|f| f.import).is_some_and(Import::kept))
                }
            };
            if kept {
                self.module_path(module.specifier);
            }
        }
        self.crucible_file(file);
        let table = std::mem::take(&mut self.table);
        for (span, site) in table.conflicts {
            self.facts.unanswered.push(Unanswered { path: self.path.clone(), span, site, reason: Reason::Conflict });
        }
        self.facts.unanswered[first..].sort_by_key(|u| (u.span, u.site));
        let mut facts: Vec<(Span, Fact)> = table.facts.into_iter().collect();
        facts.sort_unstable_by_key(|(span, _)| *span);
        self.facts.files.push(FileFacts { path: std::mem::take(&mut self.path), facts, jsx: self.jsx.take() });
    }

    fn span(&self, n: P<Node>) -> Span {
        crate::tree::span(n, self.text)
    }

    fn site(&mut self) {
        self.facts.sites += 1;
    }

    fn unanswered(&mut self, n: P<Node>, site: Site, reason: Reason) {
        let span = self.span(n);
        self.facts.unanswered.push(Unanswered { path: self.path.clone(), span, site, reason });
    }

    fn type_of(&mut self, n: P<Node>) -> P<Type> {
        self.checker.get_type_at_location(n)
    }

    fn visit(&mut self, n: P<Node>) {
        let kind = n.kind();
        if tsrs_ast::is_type_node_kind(kind)
            || matches!(kind, Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration)
            || n.flags().intersects(NodeFlags::Ambient)
        {
            return;
        }
        self.typed_lint(n);
        match kind {
            Kind::CallExpression => self.call(n),
            Kind::TaggedTemplateExpression => self.tagged_template(n),
            Kind::NewExpression => self.new_expression(n),
            Kind::DeleteExpression => {
                let target = types::skip_downwards(n.expression().unwrap());
                if matches!(target.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
                    self.element_receiver(target.expression().unwrap());
                }
            }
            Kind::PropertyAccessExpression => self.constant_ref(n),
            Kind::ElementAccessExpression => {
                let object = n.as_element_access_expression().expression;
                self.element_receiver(object);
                self.constant_ref(n);
            }
            Kind::BinaryExpression => self.binary(n),
            Kind::PrefixUnaryExpression => {
                let unary = n.as_prefix_unary_expression();
                if unary.operator == Kind::ExclamationToken {
                    self.truthy(unary.operand);
                }
            }
            Kind::IfStatement => self.truthy(n.as_if_statement().expression),
            Kind::WhileStatement => self.truthy(n.as_while_statement().expression),
            Kind::DoStatement => self.truthy(n.as_do_statement().expression),
            Kind::ConditionalExpression => self.truthy(n.as_conditional_expression().condition),
            Kind::ForStatement => self.for_statement(n),
            Kind::ForOfStatement => self.for_of(n),
            Kind::ArrayBindingPattern => {
                self.iterable(n, Site::Destructure);
            }
            Kind::ObjectBindingPattern if computed_key(n) => {
                let t = self.type_of(n);
                self.element_receiver_of(n, t);
            }
            Kind::ArrayLiteralExpression | Kind::ObjectLiteralExpression => self.assignment_pattern(n),
            Kind::SpreadElement => {
                let expression = n.as_spread_element().expression;
                if !tsrs_ast::is_assignment_target(expression) {
                    self.iterable(expression, Site::Spread);
                }
            }
            Kind::SpreadAssignment => {
                let expression = n.as_spread_assignment().expression;
                if !tsrs_ast::is_assignment_target(expression) {
                    self.object_spread(expression);
                }
            }
            Kind::JsxSpreadAttribute => self.object_spread(n.as_jsx_spread_attribute().expression),
            Kind::JsxElement | Kind::JsxSelfClosingElement | Kind::JsxFragment => {
                if self.jsx.is_none() {
                    self.jsx = Some(self.jsx_lowering(n));
                }
                if self.jsx == Some(Jsx::Crucible) && kind != Kind::JsxFragment {
                    self.intrinsic_element(n);
                }
            }
            Kind::ReturnStatement => {
                if let Some(expression) = n.expression() {
                    self.call_or_return(expression);
                }
            }
            Kind::ArrowFunction => {
                self.function(n);
                if let Some(body) = n.body()
                    && body.kind() != Kind::Block
                {
                    self.call_or_return(body);
                }
            }
            Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::MethodDeclaration
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::Constructor => {
                if n.body().is_none() {
                    return;
                }
                self.function(n);
            }
            Kind::EnumMember => {
                self.site();
                if let Some(value) = self.checker.get_constant_value(n).and_then(types::constant) {
                    let span = self.span(n);
                    self.table.store(span, Site::Constant, |f| &mut f.constant, value);
                }
            }
            Kind::Identifier => self.identifier(n),
            Kind::ImportDeclaration => self.import_declaration(n),
            Kind::ExportDeclaration => self.export_declaration(n),
            Kind::HeritageClause if n.as_heritage_clause().token == Kind::ImplementsKeyword => return,
            _ => {}
        }
        n.for_each_child(&mut |child| {
            self.visit(child);
            false
        });
    }

    fn call(&mut self, n: P<Node>) {
        let callee = n.as_call_expression().expression;
        let target = types::skip_downwards(callee);
        let object = match target.kind() {
            Kind::ImportKeyword => return,
            Kind::SuperKeyword => {
                self.call_or_return(n);
                return;
            }
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression => target.expression(),
            _ => None,
        };
        if object.is_some_and(|o| o.kind() == Kind::SuperKeyword) {
            self.call_or_return(n);
            return;
        }
        self.site();
        let span = self.span(callee);
        if let Some(m) = self.crucible.macro_at(&mut self.checker, n) {
            self.table.store(span, Site::CallCallee, |f| &mut f.callee, Callee::Macro(m));
            if m == Macro::Inject {
                self.site();
                let inject = Box::new(self.crucible_inject(n));
                let call = self.span(n);
                self.table.store(call, Site::Crucible, |f| &mut f.crucible, inject);
            }
        } else {
            let t = self.type_of(callee);
            let t = self.checker.get_non_optional_type(t);
            if object.is_some() && types::is_any(t) {
                self.unanswered(callee, Site::CallCallee, Reason::Any);
            } else {
                let symbol = self.first_defined_symbol(t);
                let found = match symbol {
                    Some(s) if object.is_some() => self.globals.method_macro(s),
                    Some(s) => self.globals.call_macro(s),
                    None => None,
                };
                let resolved = match found {
                    Some(m) => Callee::Macro(m),
                    None if object.is_some() => self.callee_kind(callee, Site::CallCallee, t),
                    None => Callee::Function,
                };
                self.table.store(span, Site::CallCallee, |f| &mut f.callee, resolved);
                let arguments = n.arguments();
                if resolved == Callee::Macro(Macro::Assert)
                    && let Some(&first) = arguments.first()
                {
                    self.truthy(first);
                }
                if let Callee::Macro(_) = resolved {
                    if let Some(&last) = arguments.last()
                        && last.kind() == Kind::SpreadElement
                    {
                        self.unanswered(last, Site::Spread, Reason::MacroSpread);
                    }
                } else if arguments.iter().any(|&a| returns_values(a)) {
                    let roblox = self.possibly_roblox(t);
                    self.table.store(span, Site::CallCallee, |f| &mut f.roblox_api, roblox);
                }
            }
        }
        if let Some(object) = object {
            self.call_or_return(object);
        }
        self.call_or_return(n);
    }

    fn tagged_template(&mut self, n: P<Node>) {
        let tag = n.as_tagged_template_expression().tag;
        let property =
            matches!(types::skip_downwards(tag).kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression);
        self.site();
        let t = self.type_of(tag);
        let t = self.checker.get_non_optional_type(t);
        if property && types::is_any(t) {
            self.unanswered(tag, Site::CallCallee, Reason::Any);
        } else {
            let callee = if property { self.callee_kind(tag, Site::CallCallee, t) } else { Callee::Function };
            let span = self.span(tag);
            self.table.store(span, Site::CallCallee, |f| &mut f.callee, callee);
        }
        self.call_or_return(n);
    }

    fn callee_kind(&mut self, n: P<Node>, site: Site, t: P<Type>) -> Callee {
        let signatures = self.signatures(t);
        if signatures.method && signatures.function {
            self.unanswered(n, site, Reason::MethodAndFunction);
        }
        if signatures.method { Callee::Method } else { Callee::Function }
    }

    fn new_expression(&mut self, n: P<Node>) {
        self.site();
        let callee = n.expression().unwrap();
        let t = self.type_of(callee);
        let span = self.span(callee);
        if let Some(m) = t.symbol().and_then(|s| self.globals.constructor_macro(s)) {
            self.table.store(span, Site::NewCallee, |f| &mut f.callee, Callee::Macro(m));
        } else if n.arguments().iter().any(|&a| returns_values(a)) {
            let roblox = self.possibly_roblox(t);
            self.table.store(span, Site::NewCallee, |f| &mut f.roblox_api, roblox);
        }
    }

    fn constant_ref(&mut self, n: P<Node>) {
        if n.parent().is_some_and(|p| p.kind() == Kind::CallExpression && p.as_call_expression().expression == n)
            || n.expression().is_some_and(|e| e.kind() == Kind::SuperKeyword)
        {
            return;
        }
        self.site();
        if let Some(value) = self.checker.get_constant_value(n).and_then(types::constant) {
            let span = self.span(n);
            self.table.store(span, Site::Constant, |f| &mut f.constant, value);
        }
    }

    fn binary(&mut self, n: P<Node>) {
        let binary = n.as_binary_expression();
        let (left, right) = (binary.left, binary.right());
        match binary.operator_token.kind() {
            Kind::InKeyword => self.element_receiver(right),
            Kind::InstanceOfKeyword => {
                self.site();
                let t = self.type_of(right);
                let class = t.symbol().is_some_and(|s| self.checker.get_symbol_flags(s).intersects(SymbolFlags::Class))
                    && !self.possibly_roblox(t);
                let span = self.span(right);
                self.table.store(span, Site::InstanceOfClass, |f| &mut f.class_constructor, class);
            }
            Kind::PlusToken | Kind::PlusEqualsToken => self.plus(left, right),
            Kind::AmpersandAmpersandToken | Kind::BarBarToken => {
                self.truthy(left);
                self.truthy(right);
            }
            Kind::AmpersandAmpersandEqualsToken | Kind::BarBarEqualsToken => self.truthy(left),
            Kind::QuestionQuestionToken => {
                let inner = n.parent().is_some_and(|p| {
                    p.kind() == Kind::BinaryExpression
                        && p.as_binary_expression().operator_token.kind() == Kind::QuestionQuestionToken
                        && p.as_binary_expression().left == n
                });
                if !inner {
                    self.truthy(n);
                }
            }
            _ => {}
        }
    }

    /// `+` concatenates when either operand is definitely a string, so an operand that only may be one leaves the
    /// operator unanswered.
    fn plus(&mut self, left: P<Node>, right: P<Node>) {
        let mut strings = [false; 2];
        let mut maybe = [false; 2];
        for (i, operand) in [left, right].into_iter().enumerate() {
            self.site();
            let t = self.type_of(operand);
            if types::is_any(t) {
                self.unanswered(operand, Site::PlusOperand, Reason::Any);
                continue;
            }
            strings[i] = self.definitely(t, types::is_string);
            maybe[i] = !strings[i] && self.possibly_string(t);
            let class = self.value_class(t);
            self.receiver(operand, class.receiver, t, Site::PlusOperand);
        }
        if !strings.contains(&true) {
            for (i, operand) in [left, right].into_iter().enumerate() {
                if maybe[i] {
                    self.unanswered(operand, Site::PlusOperand, Reason::Mixed);
                }
            }
        }
    }

    fn truthy(&mut self, n: P<Node>) {
        self.site();
        let falsy = self.falsy_of(n);
        let span = self.span(n);
        self.table.store(span, Site::Truthiness, |f| &mut f.falsy, falsy);
    }

    /// `&&`, `||` and parentheses answer from their operands. The checker types `a && b` with only `a`'s
    /// definitely falsy part, which is `0` for a number, so its type drops NaN.
    fn falsy_of(&mut self, n: P<Node>) -> Falsy {
        match n.kind() {
            Kind::ParenthesizedExpression => self.falsy_of(n.as_parenthesized_expression().expression.get()),
            Kind::BinaryExpression => {
                let binary = n.as_binary_expression();
                match binary.operator_token.kind() {
                    Kind::AmpersandAmpersandToken => self.falsy_of(binary.left) | self.falsy_of(binary.right()),
                    Kind::BarBarToken => self.falsy_of(binary.right()),
                    _ => {
                        let t = self.type_of(n);
                        self.falsy(t)
                    }
                }
            }
            _ => {
                let t = self.type_of(n);
                self.falsy(t)
            }
        }
    }

    /// The checker accepts only object types and falsy values in a spread, so no falsy values means an object.
    fn object_spread(&mut self, n: P<Node>) {
        self.site();
        let t = self.type_of(n);
        let falsy = self.falsy(t);
        let span = self.span(n);
        self.table.store(span, Site::Spread, |f| &mut f.falsy, falsy);
    }

    fn for_statement(&mut self, n: P<Node>) {
        let for_statement = n.as_for_statement();
        if let Some(condition) = for_statement.condition {
            self.truthy(condition);
            if condition.kind() == Kind::BinaryExpression {
                self.integer_bound(condition.as_binary_expression().right());
            }
        }
        if let Some(initializer) = for_statement.initializer
            && initializer.kind() == Kind::VariableDeclarationList
            && let [declaration] = initializer.as_variable_declaration_list().declarations.nodes()
            && let Some(value) = declaration.initializer()
        {
            self.integer_bound(value);
        }
    }

    /// A numeric `for` loop needs integer bounds. Sums, differences, products and powers of integers are integers,
    /// so only their leaves are answered: a literal by its value, a `size` macro call as an integer, and any other
    /// expression by its type.
    fn integer_bound(&mut self, n: P<Node>) {
        match n.kind() {
            Kind::NumericLiteral => {
                self.site();
                let integer = jsnum::from_string(n.as_numeric_literal().text()).0.fract() == 0.0;
                let span = self.span(n);
                self.table.store(span, Site::Constant, |f| &mut f.integer, integer);
            }
            Kind::BinaryExpression => {
                let binary = n.as_binary_expression();
                if matches!(
                    binary.operator_token.kind(),
                    Kind::PlusToken | Kind::MinusToken | Kind::AsteriskToken | Kind::AsteriskAsteriskToken
                ) {
                    self.integer_bound(binary.left);
                    self.integer_bound(binary.right());
                }
            }
            Kind::PrefixUnaryExpression => {
                let unary = n.as_prefix_unary_expression();
                if matches!(unary.operator, Kind::PlusToken | Kind::MinusToken) {
                    self.integer_bound(unary.operand);
                }
            }
            _ => {
                self.site();
                let integer = self.is_size_macro_call(n) || {
                    let t = self.type_of(n);
                    self.definitely(t, types::is_integer_literal)
                };
                let span = self.span(n);
                self.table.store(span, Site::Constant, |f| &mut f.integer, integer);
            }
        }
    }

    fn is_size_macro_call(&mut self, n: P<Node>) -> bool {
        if n.kind() != Kind::CallExpression {
            return false;
        }
        let t = self.type_of(n.as_call_expression().expression);
        let t = self.checker.get_non_optional_type(t);
        let symbol = self.first_defined_symbol(t);
        symbol.and_then(|s| self.globals.method_macro(s)).is_some_and(|m| m.name() == "size")
    }

    fn for_of(&mut self, n: P<Node>) {
        let for_of = n.as_for_in_or_of_statement();
        let receiver = self.iterable(for_of.expression, Site::Iterable);
        if receiver == Some(Receiver::LuaTupleIterFn) {
            let t = self.type_of(for_of.expression);
            if let Some(length) = self.step_length(t) {
                let span = self.span(for_of.expression);
                self.table.store(span, Site::Iterable, |f| &mut f.step_length, length);
            }
        }
        let pattern = for_of.initializer;
        let inline = pattern.kind() == Kind::ArrayLiteralExpression
            && matches!(receiver, Some(Receiver::Map | Receiver::LuaTupleIterFn));
        if !inline && (pattern.kind() == Kind::ArrayLiteralExpression || computed_key(pattern)) {
            self.site();
            self.unanswered(pattern, Site::Destructure, Reason::AssignmentPattern);
        }
    }

    fn assignment_pattern(&mut self, n: P<Node>) {
        let object = n.kind() == Kind::ObjectLiteralExpression;
        if object && !computed_key(n) {
            return;
        }
        match pattern_role(n) {
            None | Some(Pattern::ForOf) => {}
            Some(Pattern::Assigned(source)) => {
                let t = self.type_of(source);
                if object {
                    self.element_receiver_of(n, t);
                } else {
                    self.iterable_of(n, t, Site::Destructure);
                }
            }
            Some(Pattern::Nested) => {
                self.site();
                self.unanswered(n, Site::Destructure, Reason::AssignmentPattern);
            }
        }
    }

    fn call_or_return(&mut self, n: P<Node>) {
        self.site();
        let t = self.type_of(n);
        let class = self.value_class(t);
        self.receiver(n, class.receiver, t, Site::CallOrReturn);
    }

    fn element_receiver(&mut self, n: P<Node>) {
        let t = self.type_of(n);
        self.element_receiver_of(n, t);
    }

    /// Indexing adds one for arrays, so a receiver whose class is unknown cannot be lowered.
    fn element_receiver_of(&mut self, n: P<Node>, t: P<Type>) {
        self.site();
        if types::is_any(t) {
            self.unanswered(n, Site::ElementReceiver, Reason::Any);
            return;
        }
        let class = self.value_class(t);
        if class.mixed {
            self.unanswered(n, Site::ElementReceiver, Reason::Mixed);
        }
        self.receiver(n, class.receiver, t, Site::ElementReceiver);
    }

    fn receiver(&mut self, n: P<Node>, receiver: Receiver, t: P<Type>, site: Site) {
        let falsy = self.falsy(t);
        let span = self.span(n);
        self.table.store(span, site, |f| &mut f.receiver, receiver);
        self.table.store(span, site, |f| &mut f.falsy, falsy);
    }

    fn iterable(&mut self, n: P<Node>, site: Site) -> Option<Receiver> {
        let t = self.type_of(n);
        self.iterable_of(n, t, site)
    }

    fn iterable_of(&mut self, n: P<Node>, t: P<Type>, site: Site) -> Option<Receiver> {
        self.site();
        if types::is_any(t) {
            self.unanswered(n, site, Reason::Any);
            return None;
        }
        let receiver = self.value_class(t).receiver;
        if receiver == Receiver::Other {
            if !(site == Site::Iterable && self.is_call_to(n, Macro::Range)) {
                self.unanswered(n, site, Reason::Iteration);
            }
            return None;
        }
        self.receiver(n, receiver, t, site);
        Some(receiver)
    }

    fn is_call_to(&mut self, n: P<Node>, m: Macro) -> bool {
        let n = types::skip_downwards(n);
        if n.kind() != Kind::CallExpression {
            return false;
        }
        let callee = n.as_call_expression().expression;
        let t = self.type_of(callee);
        self.first_defined_symbol(t).and_then(|s| self.globals.call_macro(s)) == Some(m)
    }

    fn function(&mut self, n: P<Node>) {
        self.site();
        let t = self.type_of(n);
        let callee = self.callee_kind(n, Site::Function, t);
        let span = self.span(n);
        self.table.store(span, Site::Function, |f| &mut f.callee, callee);
    }

    fn identifier(&mut self, n: P<Node>) {
        let text = n.as_identifier().text();
        if let Some(i) = self.pending.iter().position(|p| p.name == text && !p.used)
            && !n.parent().is_some_and(|p| {
                matches!(
                    p.kind(),
                    Kind::ImportSpecifier | Kind::ImportClause | Kind::NamespaceImport | Kind::ExportSpecifier
                )
            })
            && self.checker.get_symbol_at_location_exported(n) == Some(self.pending[i].alias)
        {
            self.pending[i].used = true;
        }
        if text == Macro::Promise.name() && self.globals.identifier_macro.is_some() {
            let symbol = self.checker.get_symbol_at_location_exported(n);
            if symbol.is_some() && symbol == self.globals.identifier_macro {
                self.site();
                let span = self.span(n);
                self.table.store(span, Site::Identifier, |f| &mut f.callee, Callee::Macro(Macro::Promise));
            }
        }
    }

    fn import_declaration(&mut self, n: P<Node>) {
        let declaration = n.as_import_declaration();
        let Some(clause) = declaration.import_clause else {
            self.modules.push(ModuleUse { specifier: declaration.module_specifier, kept: Kept::Always });
            return;
        };
        if clause.is_type_only() {
            return;
        }
        let mut bindings = Vec::new();
        if let Some(name) = clause.as_import_clause().name {
            bindings.push((clause, name));
        }
        if let Some(named) = clause.as_import_clause().named_bindings {
            match named.kind() {
                Kind::NamespaceImport => bindings.push((named, named.name().unwrap())),
                Kind::NamedImports => {
                    for &specifier in named.as_named_imports().elements.nodes() {
                        if !specifier.is_type_only() {
                            bindings.push((specifier, specifier.name().unwrap()));
                        }
                    }
                }
                _ => {}
            }
        }
        let spans = bindings.iter().map(|&(_, name)| self.span(name)).collect();
        self.modules.push(ModuleUse { specifier: declaration.module_specifier, kept: Kept::AnyOf(spans) });
        for (declaration, name) in bindings {
            self.site();
            let span = self.span(name);
            if self.resolver.is_referenced_alias_declaration(&mut self.checker, declaration) {
                let whole = declaration == clause && self.exports_equals(n.as_import_declaration().module_specifier);
                let import = if whole { Import::KeepModule } else { Import::Keep };
                self.table.store(span, Site::Import, |f| &mut f.import, import);
                continue;
            }
            let alias = self.checker.get_symbol_of_declaration_exported(declaration);
            let target = alias.map(|a| self.checker.get_aliased_symbol(a));
            let const_enum =
                target.is_some_and(|t| t.flags().intersects(SymbolFlags::ConstEnum | SymbolFlags::ConstEnumOnlyModule));
            match alias {
                Some(alias) if const_enum => {
                    self.pending.push(ConstEnumImport { name: name.as_identifier().text(), alias, span, used: false })
                }
                _ => self.table.store(span, Site::Import, |f| &mut f.import, Import::Elide),
            }
        }
    }

    fn exports_equals(&mut self, specifier: P<Node>) -> bool {
        let module = self.checker.resolve_external_module_name_exported(specifier, None);
        module
            .and_then(|m| m.exports())
            .is_some_and(|e| e.lookup(tsrs_ast::INTERNAL_SYMBOL_NAME_EXPORT_EQUALS).is_some())
    }

    fn jsx_lowering(&mut self, n: P<Node>) -> Jsx {
        let jsx = self.options.jsx;
        if jsx == JsxEmit::REACT_JSX || jsx == JsxEmit::REACT_JSX_DEV {
            let source = tsrs_ast::get_jsx_implicit_import_base(&self.options, tsrs_ast::get_source_file_of_node(n));
            return if source == UI_ENTRY { Jsx::Crucible } else { Jsx::Unsupported };
        }
        if jsx != JsxEmit::REACT {
            return Jsx::Unsupported;
        }
        let name = |entity: P<Node>| tsrs_ast::entity_name_to_string(entity, None).into_boxed_str();
        match self.resolver.get_jsx_factory_entity(&mut self.checker, n) {
            Some(factory) => Jsx::Factory {
                factory: name(factory),
                fragment: self.resolver.get_jsx_fragment_factory_entity(&mut self.checker, n).map(name),
            },
            None => Jsx::Unsupported,
        }
    }

    /// Stores the class an intrinsic element creates and how each of its attributes lowers.
    fn intrinsic_element(&mut self, n: P<Node>) {
        let (tag, attributes) = match n.kind() {
            Kind::JsxElement => {
                let opening = n.as_jsx_element().opening_element.as_jsx_opening_element();
                (opening.tag_name, opening.attributes)
            }
            _ => {
                let element = n.as_jsx_self_closing_element();
                (element.tag_name, element.attributes)
            }
        };
        if tag.kind() != Kind::Identifier || !tsrs_scanner::is_intrinsic_jsx_name(tag.text()) {
            self.component_classes(attributes);
            return;
        }
        self.site();
        let Some(class) = self.instance_class(tag.text()) else { return };
        let span = self.span(tag);
        self.table.store(span, Site::Jsx, |f| &mut f.instance_class, class.name().into());
        let instance = self.checker.get_type_of_symbol_exported(class);
        let mut explicit = BTreeSet::new();
        let mut classes = Vec::new();
        for &attribute in attributes.as_jsx_attributes().properties().nodes() {
            if attribute.kind() != Kind::JsxAttribute {
                continue;
            }
            self.site();
            let name = tsrs_ast::entity_name_to_string(attribute.as_jsx_attribute().name, None);
            if name == "className" {
                classes.push(attribute);
                continue;
            }
            match self.prop(instance, attribute) {
                Ok(prop) => {
                    if matches!(prop, Prop::Fixed { optional: false } | Prop::Reactive { optional: false }) {
                        explicit.insert(name);
                    }
                    let span = self.span(attribute);
                    self.table.store(span, Site::Jsx, |f| &mut f.prop, prop);
                }
                Err(reason) => self.unanswered(attribute, Site::Jsx, reason),
            }
        }
        for attribute in classes {
            self.class_attribute(n, instance, class.name(), attribute, &explicit);
        }
    }

    fn instance_class(&mut self, tag: &str) -> Option<P<Symbol>> {
        if self.instance_classes.is_none() {
            let mut classes = HashMap::new();
            if let Some(creatable) = self.globals.creatable_instances {
                let t = self.checker.get_declared_type_of_symbol_exported(creatable);
                for &class in self.checker.get_properties_of_type_exported(t) {
                    classes.insert(class.name().to_ascii_lowercase(), class);
                }
            }
            self.instance_classes = Some(classes);
        }
        self.instance_classes.as_ref()?.get(tag).copied()
    }

    /// An event by the instance's declared property, and otherwise a `Readable` when every member of the value's type
    /// is callable. A value that mixes callable and plain members has no answer. A value that may be nil is optional,
    /// so the element applies it only when it is not nil, as Vide skips a nil prop.
    fn prop(&mut self, instance: P<Type>, attribute: P<Node>) -> Result<Prop, Reason> {
        let jsx = attribute.as_jsx_attribute();
        let name = tsrs_ast::entity_name_to_string(jsx.name, None);
        if name == "children" {
            return Ok(Prop::Children);
        }
        let value = match jsx.initializer {
            Some(v) if v.kind() == Kind::JsxExpression => v.as_jsx_expression().expression,
            value => value,
        };
        let Some(value) = value.filter(|v| v.kind() != Kind::StringLiteral) else {
            return Ok(Prop::Fixed { optional: false });
        };
        let t = self.type_of(value);
        if types::is_any(t) {
            return Err(Reason::Any);
        }
        let optional = self.falsy(t).contains(Falsy::NIL);
        if let Some(property) = self.checker.get_property_of_type_exported(instance, &name) {
            let declared = self.checker.get_type_of_symbol_exported(property);
            if declared.symbol().is_some() && declared.symbol() == self.globals.signal {
                return Ok(Prop::Event { optional });
            }
        }
        let t = self.checker.get_non_nullable_type(t);
        let mut parts = Vec::new();
        self.walk_types(t, &mut parts);
        let callable = parts
            .iter()
            .filter(|&&part| !self.checker.get_signatures_of_type_exported(part, SignatureKind::Call).is_empty())
            .count();
        match callable {
            0 => Ok(Prop::Fixed { optional }),
            n if n == parts.len() => Ok(Prop::Reactive { optional }),
            _ => Err(Reason::Mixed),
        }
    }

    fn export_declaration(&mut self, n: P<Node>) {
        if n.is_type_only() {
            return;
        }
        let declaration = n.as_export_declaration();
        let Some(clause) = declaration.export_clause else {
            if let Some(specifier) = declaration.module_specifier {
                self.site();
                let module = self.checker.get_symbol_at_location_exported(specifier);
                let value = module.is_some_and(|m| {
                    self.checker.get_exports_of_module_exported(m).into_iter().any(|symbol| {
                        let target = if self.checker.get_symbol_flags(symbol).intersects(SymbolFlags::Alias) {
                            self.checker.get_aliased_symbol(symbol)
                        } else {
                            symbol
                        };
                        self.checker.get_symbol_flags(target).intersects(SymbolFlags::Value)
                    })
                });
                let span = self.span(specifier);
                self.table.store(
                    span,
                    Site::Import,
                    |f| &mut f.import,
                    if value { Import::Keep } else { Import::Elide },
                );
                if value {
                    self.modules.push(ModuleUse { specifier, kept: Kept::Always });
                }
            }
            return;
        };
        if clause.kind() != Kind::NamedExports {
            return;
        }
        if let Some(module) = declaration.module_specifier {
            let spans = clause
                .as_named_exports()
                .elements
                .nodes()
                .iter()
                .filter(|s| !s.is_type_only())
                .map(|s| self.span(s.name().unwrap()))
                .collect();
            self.modules.push(ModuleUse { specifier: module, kept: Kept::AnyOf(spans) });
        }
        for &specifier in clause.as_named_exports().elements.nodes() {
            if specifier.is_type_only() {
                continue;
            }
            self.site();
            let alias = self.checker.get_symbol_of_declaration_exported(specifier);
            if declaration.module_specifier.is_none()
                && let Some(local) = alias.and_then(|a| self.checker.get_immediate_aliased_symbol_exported(a))
            {
                for pending in self.pending.iter_mut().filter(|p| p.alias == local) {
                    pending.used = true;
                }
            }
            let target = alias.map(|a| self.checker.get_aliased_symbol(a));
            let value = target.is_some_and(|t| self.checker.get_symbol_flags(t).intersects(SymbolFlags::Value));
            let span = self.span(specifier.name().unwrap());
            let import = if value { Import::Keep } else { Import::Elide };
            self.table.store(span, Site::Import, |f| &mut f.import, import);
        }
    }

    /// Where a kept import's module is in a game, from the importing script. In a package, only an import of another
    /// installed package needs one; relative imports lower from their specifier.
    fn module_path(&mut self, specifier: P<Node>) {
        if self.game.is_none() && specifier.text().starts_with('.') {
            return;
        }
        self.site();
        let target = self
            .checker
            .get_symbol_at_location_exported(specifier)
            .and_then(|s| s.declarations().first().copied())
            .and_then(tsrs_ast::get_source_file_of_node)
            .map(|f| f.file_name().replace('\\', "/"));
        let Some(game) = &self.game else {
            if let Some(path) = target.as_deref().and_then(installed_module) {
                let span = self.span(specifier);
                self.table.store(span, Site::Import, |f| &mut f.module, Box::new(path));
            }
            return;
        };
        let Some(target) = target else {
            return self.report(specifier, Code::Place, "the module this names has no source file");
        };
        if target.ends_with(".server.ts") || target.ends_with(".client.ts") {
            return self.report(specifier, Code::Place, "a script is not a module and cannot be imported");
        }
        let Some(script) = game.module(Path::new(&self.path)) else {
            return self.report(specifier, Code::Place, "the Rojo project maps no instance for this file's output");
        };
        let Some(module) = game.module(Path::new(&target)) else {
            return self.report(specifier, Code::Place, "the Rojo project maps no instance for this module");
        };
        match game.rojo.import(&script, &module) {
            Ok(path) => {
                let span = self.span(specifier);
                self.table.store(span, Site::Import, |f| &mut f.module, Box::new(path));
            }
            Err(message) => self.report(specifier, Code::Place, message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::Falsy;

    #[test]
    fn a_second_different_answer_is_a_conflict() {
        let span = Span { start: 3, end: 9 };
        let mut table = Table::default();
        table.store(span, Site::CallOrReturn, |f| &mut f.receiver, Receiver::Array);
        table.store(span, Site::CallOrReturn, |f| &mut f.receiver, Receiver::Array);
        table.store(span, Site::Truthiness, |f| &mut f.falsy, Falsy::NIL);
        assert!(table.conflicts.is_empty(), "equal answers agree");
        table.store(span, Site::Iterable, |f| &mut f.receiver, Receiver::Map);
        assert_eq!(table.conflicts, [(span, Site::Iterable)], "a different answer is recorded");
        assert_eq!(table.facts[&span].receiver, Some(Receiver::Array), "the first answer stays");
    }
}
