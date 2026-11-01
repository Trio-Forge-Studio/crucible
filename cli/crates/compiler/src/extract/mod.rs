//! The fact extractor. It walks the syntax tree of every source file, finds each site whose lowering depends on a
//! type, and answers it from the checker that ran the full check. A site is a node where roblox-ts 3.0.0 or Crucible's
//! transform asks the checker a question whose answer changes the emitted Luau. Checks that only report an error are
//! not sites yet.

mod crucible;
mod types;

use std::collections::HashMap;

use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{EmitResolver, Type};
use tsrs_compiler::CheckerHandle;
use tsrs_core::{P, jsnum};

use crate::facts::{Callee, Crucible, Fact, Facts, Falsy, FileFacts, Import, Reason, Receiver, Site, Span, Unanswered};
use crate::macros::Macro;
use crate::program::Checked;

pub(crate) fn extract(checked: Checked) -> Facts {
    let Checked { program, mut checker, host } = checked;
    let resolver = checker.new_emit_resolver(tsrs_printer::new_emit_context());
    let globals = types::Globals::new(&mut checker);
    let crucible = crucible::Api::new(program, &mut checker, host);
    let mut x = Extractor {
        checker,
        resolver,
        globals,
        crucible,
        facts: Facts::default(),
        text: "",
        path: String::new(),
        table: Table::default(),
        pending: Vec::new(),
    };
    for &file in program.source_files() {
        if file.is_declaration_file() || file.is_js() {
            continue;
        }
        x.source_file(file);
    }
    x.facts
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
    checker: CheckerHandle,
    resolver: P<EmitResolver>,
    globals: types::Globals,
    crucible: crucible::Api,
    facts: Facts,
    text: &'static str,
    path: String,
    table: Table,
    pending: Vec<ConstEnumImport>,
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
        let table = std::mem::take(&mut self.table);
        for (span, site) in table.conflicts {
            self.facts.unanswered.push(Unanswered { path: self.path.clone(), span, site, reason: Reason::Conflict });
        }
        self.facts.unanswered[first..].sort_by_key(|u| (u.span, u.site));
        let mut facts: Vec<(Span, Fact)> = table.facts.into_iter().collect();
        facts.sort_unstable_by_key(|(span, _)| *span);
        self.facts.files.push(FileFacts { path: std::mem::take(&mut self.path), facts });
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
        match kind {
            Kind::CallExpression => self.call(n),
            Kind::TaggedTemplateExpression => self.tagged_template(n),
            Kind::NewExpression => self.new_expression(n),
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
            Kind::ClassDeclaration => self.crucible_class(n),
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
        let Some(clause) = n.as_import_declaration().import_clause else { return };
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
        for (declaration, name) in bindings {
            self.site();
            let span = self.span(name);
            if self.resolver.is_referenced_alias_declaration(&mut self.checker, declaration) {
                self.table.store(span, Site::Import, |f| &mut f.import, Import::Keep);
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

    fn export_declaration(&mut self, n: P<Node>) {
        if n.is_type_only() {
            return;
        }
        let declaration = n.as_export_declaration();
        let Some(clause) = declaration.export_clause else { return };
        if clause.kind() != Kind::NamedExports {
            return;
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

    fn crucible_class(&mut self, n: P<Node>) {
        if n.modifier_flags().intersects(ModifierFlags::Abstract) {
            return;
        }
        let Some(class) = self.crucible_registration(n) else { return };
        self.site();
        let span = self.span(n);
        self.table.store(span, Site::Crucible, |f| &mut f.crucible, Box::new(Crucible::Class(class)));
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
