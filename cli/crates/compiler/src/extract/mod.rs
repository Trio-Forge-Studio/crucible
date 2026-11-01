//! The fact extractor. It walks the syntax tree of every source file, finds each site whose lowering depends on a
//! type, and answers it from the checker that ran the full check. The predicates port roblox-ts 3.0.0's
//! `util/types.ts`, `isMethod.ts` and `createTruthinessChecks.ts`, and the class metadata ports Crucible's
//! `transformer-src`.

mod crucible;
mod types;

use std::collections::HashMap;

use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{EmitResolver, Type};
use tsrs_compiler::CheckerHandle;
use tsrs_core::P;

use crate::facts::{
    Callee, Constant, Crucible, Fact, Facts, FileFacts, Import, Reason, Receiver, Site, Span, Unanswered,
};
use crate::macros::Macro;
use crate::program::Checked;

pub(crate) fn extract(checked: Checked) -> Facts {
    let Checked { program, mut checker } = checked;
    let resolver = checker.new_emit_resolver(tsrs_printer::new_emit_context());
    let globals = types::Globals::new(&mut checker);
    let crucible = crucible::Api::new(program, &mut checker);
    let mut x = Extractor {
        checker,
        resolver,
        globals,
        crucible,
        facts: Facts::default(),
        text: "",
        path: String::new(),
        file: HashMap::new(),
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

/// An import of a const enum that the checker does not mark as referenced, because TypeScript inlines const enum
/// members. Crucible compiles a const enum as a regular enum, so the import stays if any value refers to it.
struct PendingImport {
    name: &'static str,
    alias: P<Symbol>,
    span: Span,
    used: bool,
}

struct Extractor {
    checker: CheckerHandle,
    resolver: P<EmitResolver>,
    globals: types::Globals,
    crucible: crucible::Api,
    facts: Facts,
    text: &'static str,
    path: String,
    file: HashMap<Span, Fact>,
    pending: Vec<PendingImport>,
}

impl Extractor {
    fn source_file(&mut self, file: P<SourceFile>) {
        self.text = file.text();
        self.path = file.file_name().replace('\\', "/");
        for &statement in file.as_node().statements() {
            self.visit(statement);
        }
        for pending in std::mem::take(&mut self.pending) {
            self.fact(pending.span).import = Some(if pending.used { Import::Keep } else { Import::Elide });
        }
        let mut facts: Vec<(Span, Fact)> = self.file.drain().collect();
        facts.sort_unstable_by_key(|(span, _)| *span);
        self.facts.files.push(FileFacts { path: std::mem::take(&mut self.path), facts });
    }

    fn span(&self, n: P<Node>) -> Span {
        let pos = n.pos().source_offset().unwrap_or(0);
        let end = n.end().source_offset().unwrap_or(pos);
        Span { start: tsrs_scanner::skip_trivia(self.text, pos), end }
    }

    fn fact(&mut self, span: Span) -> &mut Fact {
        self.file.entry(span).or_default()
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
            Kind::ForOfStatement => {
                let for_of = n.as_for_in_or_of_statement();
                self.iterable(for_of.expression, Site::Iterable);
                if for_of.initializer.kind() == Kind::VariableDeclarationList
                    && let Some(&declaration) =
                        for_of.initializer.as_variable_declaration_list().declarations.nodes().first()
                    && declaration.name().is_some_and(|name| name.kind() == Kind::ArrayBindingPattern)
                {
                    self.iterable(declaration.name().unwrap(), Site::Destructure);
                }
            }
            Kind::VariableDeclaration => {
                if n.name().is_some_and(|name| name.kind() == Kind::ArrayBindingPattern)
                    && let Some(initializer) = n.initializer()
                    && n.parent().and_then(|list| list.parent()).is_none_or(|p| p.kind() != Kind::ForOfStatement)
                {
                    self.iterable(initializer, Site::Destructure);
                }
            }
            Kind::SpreadElement => self.iterable(n.as_spread_element().expression, Site::Spread),
            Kind::SpreadAssignment => {
                self.site();
                let expression = n.as_spread_assignment().expression;
                let t = self.type_of(expression);
                let falsy = self.falsy(t);
                let span = self.span(expression);
                set(&mut self.fact(span).falsy, falsy);
            }
            Kind::ReturnStatement => {
                if let Some(expression) = n.expression() {
                    self.value(expression);
                }
            }
            Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::MethodDeclaration
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::Constructor => {
                if n.body().is_some() {
                    self.function(n);
                }
            }
            Kind::ClassDeclaration => self.crucible_class(n),
            Kind::EnumMember => {
                self.site();
                if let Some(value) = self.checker.get_constant_value(n).and_then(types::constant) {
                    let span = self.span(n);
                    set(&mut self.fact(span).constant, value);
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
        let target = types::skip_outer(callee);
        match target.kind() {
            Kind::SuperKeyword | Kind::ImportKeyword => return,
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                let object = target.expression().unwrap();
                if object.kind() == Kind::SuperKeyword {
                    return;
                }
                self.site();
                if let Some(m) = self.crucible.macro_at(&mut self.checker, callee) {
                    let span = self.span(callee);
                    set(&mut self.fact(span).callee, Callee::Macro(m));
                    self.crucible_call(n, m);
                    self.value(object);
                    self.value(n);
                    return;
                }
                let t = self.type_of(callee);
                let t = self.checker.get_non_optional_type(t);
                if types::is_any(t) {
                    self.unanswered(callee, Site::CallCallee, Reason::Any);
                } else {
                    let resolved = match self.first_defined_symbol(t).and_then(|s| self.globals.method_macro(s)) {
                        Some(m) => Callee::Macro(m),
                        None => match self.method_bits(t) {
                            (true, true) => {
                                self.unanswered(callee, Site::CallCallee, Reason::MethodAndFunction);
                                Callee::Method
                            }
                            (true, false) => Callee::Method,
                            _ => Callee::Function,
                        },
                    };
                    let span = self.span(callee);
                    set(&mut self.fact(span).callee, resolved);
                }
                self.value(object);
            }
            _ => {
                self.site();
                let resolved = match self.crucible.macro_at(&mut self.checker, callee) {
                    Some(m) => Some(m),
                    None => {
                        let t = self.type_of(callee);
                        let t = self.checker.get_non_optional_type(t);
                        self.first_defined_symbol(t).and_then(|s| self.globals.call_macro(s))
                    }
                };
                let span = self.span(callee);
                set(&mut self.fact(span).callee, resolved.map_or(Callee::Function, Callee::Macro));
                if let Some(m) = resolved {
                    self.crucible_call(n, m);
                }
            }
        }
        self.value(n);
    }

    fn crucible_call(&mut self, n: P<Node>, m: Macro) {
        if m == Macro::Inject
            && let Some(inject) = self.crucible_inject(n)
        {
            self.site();
            let span = self.span(n);
            self.fact(span).crucible = Some(Box::new(inject));
        }
    }

    fn new_expression(&mut self, n: P<Node>) {
        self.site();
        let callee = n.expression().unwrap();
        let t = self.type_of(callee);
        if let Some(m) = t.symbol().and_then(|s| self.globals.constructor_macro(s)) {
            let span = self.span(callee);
            set(&mut self.fact(span).callee, Callee::Macro(m));
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
            set(&mut self.fact(span).constant, value);
        }
    }

    fn binary(&mut self, n: P<Node>) {
        let binary = n.as_binary_expression();
        let (left, right) = (binary.left, binary.right());
        match binary.operator_token.kind() {
            Kind::PlusToken | Kind::PlusEqualsToken => self.plus(left, right),
            Kind::AmpersandAmpersandToken | Kind::BarBarToken => {
                let op = binary.operator_token.kind();
                for operand in [left, right] {
                    let same = operand.kind() == Kind::BinaryExpression
                        && operand.as_binary_expression().operator_token.kind() == op;
                    if !same {
                        self.truthy(operand);
                    }
                }
            }
            Kind::AmpersandAmpersandEqualsToken | Kind::BarBarEqualsToken => self.truthy(left),
            Kind::QuestionQuestionToken => {
                let inner = n.parent().is_some_and(|p| {
                    p.kind() == Kind::BinaryExpression
                        && p.as_binary_expression().operator_token.kind() == Kind::QuestionQuestionToken
                        && p.as_binary_expression().left == n
                });
                if !inner {
                    self.site();
                    let t = self.type_of(n);
                    let falsy = self.falsy(t);
                    let span = self.span(n);
                    set(&mut self.fact(span).falsy, falsy);
                }
            }
            Kind::EqualsToken if types::skip_outer(left).kind() == Kind::ArrayLiteralExpression => {
                self.iterable(right, Site::Destructure);
            }
            _ => {}
        }
    }

    /// roblox-ts concatenates when either operand is definitely a string, and adds numbers otherwise. An operand is
    /// definitely a string when its receiver is `String` and it is not nullable.
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
            let (receiver, _) = self.value_class(t);
            self.receiver(operand, receiver, t);
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
        let t = self.type_of(n);
        let falsy = self.falsy(t);
        let span = self.span(n);
        set(&mut self.fact(span).falsy, falsy);
    }

    fn for_statement(&mut self, n: P<Node>) {
        let for_statement = n.as_for_statement();
        let mut bounds = Vec::new();
        if let Some(condition) = for_statement.condition {
            self.truthy(condition);
            if condition.kind() == Kind::BinaryExpression {
                bounds.push(condition.as_binary_expression().right());
            }
        }
        if let Some(initializer) = for_statement.initializer
            && initializer.kind() == Kind::VariableDeclarationList
            && let [declaration] = initializer.as_variable_declaration_list().declarations.nodes()
            && let Some(value) = declaration.initializer()
        {
            bounds.push(value);
        }
        for bound in bounds {
            self.site();
            let t = self.type_of(bound);
            if let Some(value) = self.integer_literal(t) {
                let span = self.span(bound);
                set(&mut self.fact(span).constant, Constant::Number(value));
            }
        }
    }

    /// Call receivers, call results and return values.
    fn value(&mut self, n: P<Node>) {
        self.site();
        let t = self.type_of(n);
        let (receiver, _) = self.value_class(t);
        self.receiver(n, receiver, t);
    }

    /// Indexing adds one for arrays, so a receiver whose class is unknown cannot be lowered.
    fn element_receiver(&mut self, n: P<Node>) {
        self.site();
        let t = self.type_of(n);
        if types::is_any(t) {
            self.unanswered(n, Site::ElementReceiver, Reason::Any);
            return;
        }
        let (receiver, mixed) = self.value_class(t);
        if mixed {
            self.unanswered(n, Site::ElementReceiver, Reason::Mixed);
        }
        self.receiver(n, receiver, t);
    }

    fn receiver(&mut self, n: P<Node>, receiver: Receiver, t: P<Type>) {
        let falsy = self.falsy(t);
        let span = self.span(n);
        let fact = self.fact(span);
        set(&mut fact.receiver, receiver);
        set(&mut fact.falsy, falsy);
    }

    /// `for..of` iterables, spreads and destructured values.
    fn iterable(&mut self, n: P<Node>, site: Site) {
        self.site();
        let t = self.type_of(n);
        if types::is_any(t) {
            self.unanswered(n, site, Reason::Any);
            return;
        }
        let (receiver, _) = self.value_class(t);
        if receiver == Receiver::Other {
            if !(site == Site::Iterable && self.is_call_to(n, Macro::Range)) {
                self.unanswered(n, site, Reason::Iteration);
            }
            return;
        }
        self.receiver(n, receiver, t);
    }

    fn is_call_to(&mut self, n: P<Node>, m: Macro) -> bool {
        let n = types::skip_outer(n);
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
        let callee = match self.method_bits(t) {
            (true, true) => {
                self.unanswered(n, Site::Function, Reason::MethodAndFunction);
                Callee::Method
            }
            (true, false) => Callee::Method,
            _ => Callee::Function,
        };
        let span = self.span(n);
        set(&mut self.fact(span).callee, callee);
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
                set(&mut self.fact(span).callee, Callee::Macro(Macro::Promise));
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
                self.fact(span).import = Some(Import::Keep);
                continue;
            }
            let alias = self.checker.get_symbol_of_declaration_exported(declaration);
            let target = alias.map(|a| self.checker.get_aliased_symbol(a));
            let const_enum =
                target.is_some_and(|t| t.flags().intersects(SymbolFlags::ConstEnum | SymbolFlags::ConstEnumOnlyModule));
            match alias {
                Some(alias) if const_enum => {
                    self.pending.push(PendingImport { name: name.as_identifier().text(), alias, span, used: false })
                }
                _ => self.fact(span).import = Some(Import::Elide),
            }
        }
    }

    fn export_declaration(&mut self, n: P<Node>) {
        if n.is_type_only() {
            return;
        }
        let Some(clause) = n.as_export_declaration().export_clause else { return };
        if clause.kind() != Kind::NamedExports {
            return;
        }
        for &specifier in clause.as_named_exports().elements.nodes() {
            if specifier.is_type_only() {
                continue;
            }
            self.site();
            let alias = self.checker.get_symbol_of_declaration_exported(specifier);
            let target = alias.map(|a| self.checker.get_aliased_symbol(a));
            let value = target.is_some_and(|t| self.checker.get_symbol_flags(t).intersects(SymbolFlags::Value));
            let span = self.span(specifier.name().unwrap());
            self.fact(span).import = Some(if value { Import::Keep } else { Import::Elide });
        }
    }

    fn crucible_class(&mut self, n: P<Node>) {
        if n.modifier_flags().intersects(ModifierFlags::Abstract) {
            return;
        }
        let Some(class) = self.crucible_registration(n) else { return };
        self.site();
        let span = self.span(n);
        self.fact(span).crucible = Some(Box::new(Crucible::Class(class)));
    }
}

/// Stores one answer. Sites that share a node ask the same question of the same type, so a second answer must match.
fn set<T: PartialEq + std::fmt::Debug>(slot: &mut Option<T>, value: T) {
    debug_assert!(slot.as_ref().is_none_or(|old| *old == value), "two sites disagree: {slot:?} and {value:?}");
    *slot = Some(value);
}
