//! The syntax tree view the lowering reads. It wraps tsrs nodes behind Crucible's own kinds and accessors, so the
//! lowering never names a tsrs type. Accessors panic on a node of the wrong kind, as tsrs's do.

use tsrs_ast::{Kind as TsKind, ModifierFlags, Node as TsNode, NodeFlags, SourceFile};
use tsrs_core::P;

use crate::facts::Span;

macro_rules! kinds {
    ($($kind:ident,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Kind {
            $($kind,)*
            Other,
        }

        fn kind_of(kind: TsKind) -> Kind {
            match kind {
                $(TsKind::$kind => Kind::$kind,)*
                _ => Kind::Other,
            }
        }
    };
}

kinds! {
    Block,
    EmptyStatement,
    VariableStatement,
    ExpressionStatement,
    IfStatement,
    DoStatement,
    WhileStatement,
    ForStatement,
    ForInStatement,
    ForOfStatement,
    ContinueStatement,
    BreakStatement,
    ReturnStatement,
    WithStatement,
    SwitchStatement,
    LabeledStatement,
    ThrowStatement,
    TryStatement,
    DebuggerStatement,
    VariableDeclaration,
    VariableDeclarationList,
    FunctionDeclaration,
    ClassDeclaration,
    InterfaceDeclaration,
    TypeAliasDeclaration,
    EnumDeclaration,
    ModuleDeclaration,
    ModuleBlock,
    ImportEqualsDeclaration,
    ImportDeclaration,
    ImportClause,
    NamespaceImport,
    NamedImports,
    ImportSpecifier,
    ExportAssignment,
    ExportDeclaration,
    NamedExports,
    NamespaceExport,
    ExportSpecifier,
    NamespaceExportDeclaration,
    Identifier,
    PrivateIdentifier,
    NumericLiteral,
    BigIntLiteral,
    StringLiteral,
    RegularExpressionLiteral,
    NoSubstitutionTemplateLiteral,
    TemplateExpression,
    TemplateSpan,
    TaggedTemplateExpression,
    TrueKeyword,
    FalseKeyword,
    NullKeyword,
    ThisKeyword,
    SuperKeyword,
    ArrayLiteralExpression,
    ObjectLiteralExpression,
    PropertyAccessExpression,
    ElementAccessExpression,
    CallExpression,
    NewExpression,
    TypeAssertionExpression,
    ParenthesizedExpression,
    FunctionExpression,
    ArrowFunction,
    DeleteExpression,
    TypeOfExpression,
    VoidExpression,
    AwaitExpression,
    PrefixUnaryExpression,
    PostfixUnaryExpression,
    BinaryExpression,
    ConditionalExpression,
    YieldExpression,
    SpreadElement,
    ClassExpression,
    OmittedExpression,
    AsExpression,
    NonNullExpression,
    MetaProperty,
    SatisfiesExpression,
    JsxElement,
    JsxSelfClosingElement,
    JsxFragment,
    ComputedPropertyName,
    Parameter,
    Decorator,
    PropertySignature,
    PropertyDeclaration,
    MethodSignature,
    MethodDeclaration,
    ClassStaticBlockDeclaration,
    Constructor,
    GetAccessor,
    SetAccessor,
    IndexSignature,
    SemicolonClassElement,
    ObjectBindingPattern,
    ArrayBindingPattern,
    BindingElement,
    PropertyAssignment,
    ShorthandPropertyAssignment,
    SpreadAssignment,
    EnumMember,
    CaseClause,
    DefaultClause,
    HeritageClause,
    ExpressionWithTypeArguments,
    ImportKeyword,
}

macro_rules! operators {
    ($($token:ident => $op:ident,)*) => {
        /// A binary or assignment operator.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Op {
            $($op,)*
            Other,
        }

        fn op_of(kind: TsKind) -> Op {
            match kind {
                $(TsKind::$token => Op::$op,)*
                _ => Op::Other,
            }
        }
    };
}

operators! {
    EqualsToken => Assign,
    PlusEqualsToken => AddAssign,
    MinusEqualsToken => SubAssign,
    AsteriskEqualsToken => MulAssign,
    SlashEqualsToken => DivAssign,
    PercentEqualsToken => ModAssign,
    AsteriskAsteriskEqualsToken => PowAssign,
    AmpersandEqualsToken => BitAndAssign,
    BarEqualsToken => BitOrAssign,
    CaretEqualsToken => BitXorAssign,
    LessThanLessThanEqualsToken => ShlAssign,
    GreaterThanGreaterThanEqualsToken => ShrAssign,
    GreaterThanGreaterThanGreaterThanEqualsToken => UShrAssign,
    BarBarEqualsToken => OrAssign,
    AmpersandAmpersandEqualsToken => AndAssign,
    QuestionQuestionEqualsToken => CoalesceAssign,
    PlusToken => Add,
    MinusToken => Sub,
    AsteriskToken => Mul,
    SlashToken => Div,
    PercentToken => Mod,
    AsteriskAsteriskToken => Pow,
    AmpersandToken => BitAnd,
    BarToken => BitOr,
    CaretToken => BitXor,
    LessThanLessThanToken => Shl,
    GreaterThanGreaterThanToken => Shr,
    GreaterThanGreaterThanGreaterThanToken => UShr,
    EqualsEqualsEqualsToken => StrictEq,
    ExclamationEqualsEqualsToken => StrictNe,
    EqualsEqualsToken => LooseEq,
    ExclamationEqualsToken => LooseNe,
    LessThanToken => Lt,
    LessThanEqualsToken => Le,
    GreaterThanToken => Gt,
    GreaterThanEqualsToken => Ge,
    AmpersandAmpersandToken => And,
    BarBarToken => Or,
    QuestionQuestionToken => Coalesce,
    CommaToken => Comma,
    InKeyword => In,
    InstanceOfKeyword => InstanceOf,
}

impl Op {
    /// The operator an assignment applies, as `Add` for `+=`.
    pub fn compound(self) -> Option<Op> {
        Some(match self {
            Op::AddAssign => Op::Add,
            Op::SubAssign => Op::Sub,
            Op::MulAssign => Op::Mul,
            Op::DivAssign => Op::Div,
            Op::ModAssign => Op::Mod,
            Op::PowAssign => Op::Pow,
            Op::BitAndAssign => Op::BitAnd,
            Op::BitOrAssign => Op::BitOr,
            Op::BitXorAssign => Op::BitXor,
            Op::ShlAssign => Op::Shl,
            Op::ShrAssign => Op::Shr,
            Op::UShrAssign => Op::UShr,
            Op::OrAssign => Op::Or,
            Op::AndAssign => Op::And,
            Op::CoalesceAssign => Op::Coalesce,
            _ => return None,
        })
    }

    pub fn is_assignment(self) -> bool {
        self == Op::Assign || self.compound().is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    BitNot,
    Increment,
    Decrement,
}

/// One parsed source file.
#[derive(Clone, Copy)]
pub struct File {
    file: P<SourceFile>,
}

impl File {
    pub(crate) fn new(file: P<SourceFile>) -> File {
        File { file }
    }

    /// The absolute path with forward slashes.
    pub fn path(&self) -> String {
        self.file.file_name().replace('\\', "/")
    }

    pub fn text(&self) -> &'static str {
        self.file.text()
    }

    pub fn root(&self) -> Node {
        Node { n: self.file.as_node(), text: self.file.text() }
    }

    /// One-based line and column, in bytes.
    pub fn position(&self, offset: u32) -> (u32, u32) {
        let text = self.file.text();
        let mut offset = (offset as usize).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let before = &text[..offset];
        let line = before.matches('\n').count() + 1;
        let column = offset - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        (line as u32, column as u32)
    }
}

#[derive(Clone, Copy)]
pub struct Node {
    n: P<TsNode>,
    text: &'static str,
}

impl PartialEq for Node {
    fn eq(&self, other: &Node) -> bool {
        self.n == other.n
    }
}

impl Eq for Node {}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let span = self.span();
        write!(f, "{:?}@{}..{}", self.n.kind(), span.start, span.end)
    }
}

#[derive(Clone, Copy)]
pub struct Nodes {
    nodes: &'static [P<TsNode>],
    text: &'static str,
}

impl Nodes {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<Node> {
        self.nodes.get(i).map(|&n| Node { n, text: self.text })
    }

    pub fn last(&self) -> Option<Node> {
        self.nodes.last().map(|&n| Node { n, text: self.text })
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = Node> + ExactSizeIterator + '_ {
        let text = self.text;
        self.nodes.iter().map(move |&n| Node { n, text })
    }
}

impl Node {
    fn wrap(&self, n: P<TsNode>) -> Node {
        Node { n, text: self.text }
    }

    fn wrap_opt(&self, n: Option<P<TsNode>>) -> Option<Node> {
        n.map(|n| self.wrap(n))
    }

    fn list(&self, nodes: &'static [P<TsNode>]) -> Nodes {
        Nodes { nodes, text: self.text }
    }

    pub fn kind(&self) -> Kind {
        kind_of(self.n.kind())
    }

    /// The tsrs kind name, for error messages.
    pub fn kind_name(&self) -> String {
        format!("{:?}", self.n.kind())
    }

    /// The span the fact table keys this node by.
    pub fn span(&self) -> Span {
        let pos = self.n.pos().source_offset().unwrap_or(0);
        let end = self.n.end().source_offset().unwrap_or(pos);
        Span { start: tsrs_scanner::skip_trivia(self.text, pos), end }
    }

    pub fn parent(&self) -> Option<Node> {
        self.wrap_opt(self.n.parent())
    }

    /// The text of an identifier, a string or template literal (its value), or a numeric literal.
    pub fn text(&self) -> &'static str {
        self.n.text()
    }

    /// The value of a numeric literal.
    pub fn number(&self) -> f64 {
        let text = self.n.text();
        text.parse().unwrap_or_else(|_| parse_integer(text).unwrap_or(f64::NAN))
    }

    pub fn expression(&self) -> Node {
        self.wrap(self.n.expression().expect("the node has an expression"))
    }

    pub fn expression_opt(&self) -> Option<Node> {
        self.wrap_opt(self.n.expression())
    }

    pub fn name(&self) -> Option<Node> {
        self.wrap_opt(self.n.name())
    }

    pub fn property_name(&self) -> Option<Node> {
        self.wrap_opt(self.n.property_name())
    }

    pub fn initializer(&self) -> Option<Node> {
        match self.n.kind() {
            TsKind::ForOfStatement | TsKind::ForInStatement => {
                Some(self.wrap(self.n.as_for_in_or_of_statement().initializer))
            }
            TsKind::ForStatement => self.wrap_opt(self.n.as_for_statement().initializer),
            TsKind::ShorthandPropertyAssignment => {
                self.wrap_opt(self.n.as_shorthand_property_assignment().object_assignment_initializer.get())
            }
            _ => self.wrap_opt(self.n.initializer()),
        }
    }

    pub fn body(&self) -> Option<Node> {
        match self.n.kind() {
            TsKind::ModuleDeclaration => self.wrap_opt(self.n.as_module_declaration().body()),
            TsKind::ClassStaticBlockDeclaration => Some(self.wrap(self.n.as_class_static_block_declaration().body)),
            _ => self.wrap_opt(self.n.body()),
        }
    }

    pub fn statements(&self) -> Nodes {
        match self.n.kind() {
            TsKind::CaseClause | TsKind::DefaultClause => {
                self.list(self.n.as_case_or_default_clause().statements.nodes())
            }
            _ => self.list(self.n.statements()),
        }
    }

    pub fn parameters(&self) -> Nodes {
        self.list(self.n.parameters())
    }

    pub fn arguments(&self) -> Nodes {
        self.list(self.n.arguments())
    }

    pub fn elements(&self) -> Nodes {
        match self.n.kind() {
            TsKind::NamedImports => self.list(self.n.as_named_imports().elements.nodes()),
            TsKind::NamedExports => self.list(self.n.as_named_exports().elements.nodes()),
            _ => self.list(self.n.elements()),
        }
    }

    pub fn properties(&self) -> Nodes {
        self.list(self.n.properties())
    }

    pub fn members(&self) -> Nodes {
        self.list(self.n.members())
    }

    pub fn declarations(&self) -> Nodes {
        let list = match self.n.kind() {
            TsKind::VariableStatement => self.n.as_variable_statement().declaration_list,
            _ => self.n,
        };
        self.list(list.as_variable_declaration_list().declarations.nodes())
    }

    /// Whether a variable statement or declaration list declares with `const`.
    pub fn is_const(&self) -> bool {
        self.declaration_list_flags().contains(NodeFlags::Const)
    }

    /// Whether a variable statement or declaration list declares with `var`.
    pub fn is_var(&self) -> bool {
        !self.declaration_list_flags().intersects(NodeFlags::BlockScoped)
    }

    fn declaration_list_flags(&self) -> NodeFlags {
        match self.n.kind() {
            TsKind::VariableStatement => self.n.as_variable_statement().declaration_list.flags(),
            _ => self.n.flags(),
        }
    }

    pub fn left(&self) -> Node {
        self.wrap(self.n.as_binary_expression().left)
    }

    pub fn right(&self) -> Node {
        self.wrap(self.n.as_binary_expression().right())
    }

    pub fn operator(&self) -> Op {
        op_of(self.n.as_binary_expression().operator_token.kind())
    }

    pub fn unary_operator(&self) -> Option<UnaryOp> {
        let kind = match self.n.kind() {
            TsKind::PrefixUnaryExpression => self.n.as_prefix_unary_expression().operator,
            _ => self.n.as_postfix_unary_expression().operator,
        };
        Some(match kind {
            TsKind::PlusToken => UnaryOp::Plus,
            TsKind::MinusToken => UnaryOp::Minus,
            TsKind::ExclamationToken => UnaryOp::Not,
            TsKind::TildeToken => UnaryOp::BitNot,
            TsKind::PlusPlusToken => UnaryOp::Increment,
            TsKind::MinusMinusToken => UnaryOp::Decrement,
            _ => return None,
        })
    }

    pub fn operand(&self) -> Node {
        match self.n.kind() {
            TsKind::PrefixUnaryExpression => self.wrap(self.n.as_prefix_unary_expression().operand),
            _ => self.wrap(self.n.as_postfix_unary_expression().operand),
        }
    }

    /// The condition of a conditional expression or a `for` statement.
    pub fn condition(&self) -> Option<Node> {
        match self.n.kind() {
            TsKind::ConditionalExpression => Some(self.wrap(self.n.as_conditional_expression().condition)),
            TsKind::ForStatement => self.wrap_opt(self.n.as_for_statement().condition),
            _ => None,
        }
    }

    pub fn incrementor(&self) -> Option<Node> {
        self.wrap_opt(self.n.as_for_statement().incrementor)
    }

    pub fn when_true(&self) -> Node {
        self.wrap(self.n.as_conditional_expression().when_true)
    }

    pub fn when_false(&self) -> Node {
        self.wrap(self.n.as_conditional_expression().when_false)
    }

    pub fn then_statement(&self) -> Node {
        self.wrap(self.n.as_if_statement().then_statement)
    }

    pub fn else_statement(&self) -> Option<Node> {
        self.wrap_opt(self.n.as_if_statement().else_statement)
    }

    /// The body of a loop or a labeled statement.
    pub fn statement(&self) -> Node {
        self.wrap(self.n.statement())
    }

    pub fn clauses(&self) -> Nodes {
        let block = self.n.as_switch_statement().case_block;
        self.list(block.as_case_block().clauses.nodes())
    }

    pub fn head(&self) -> Node {
        self.wrap(self.n.as_template_expression().head)
    }

    pub fn template_spans(&self) -> Nodes {
        self.list(self.n.as_template_expression().template_spans.nodes())
    }

    pub fn literal(&self) -> Node {
        self.wrap(self.n.as_template_span().literal)
    }

    pub fn argument_expression(&self) -> Node {
        self.wrap(self.n.as_element_access_expression().argument_expression)
    }

    /// Whether a property access, element access or call has its own `?.`.
    pub fn has_question_dot(&self) -> bool {
        self.n.question_dot_token().is_some()
    }

    /// Whether the node is part of an optional chain.
    pub fn in_optional_chain(&self) -> bool {
        self.n.flags().contains(NodeFlags::OptionalChain)
    }

    /// Whether a parameter or binding element is a rest element.
    pub fn is_rest(&self) -> bool {
        match self.n.kind() {
            TsKind::Parameter => self.n.as_parameter_declaration().dot_dot_dot_token().is_some(),
            TsKind::BindingElement => self.n.as_binding_element().dot_dot_dot_token().is_some(),
            _ => false,
        }
    }

    pub fn is_export(&self) -> bool {
        self.n.modifier_flags().contains(ModifierFlags::Export)
    }

    pub fn is_default(&self) -> bool {
        self.n.modifier_flags().contains(ModifierFlags::Default)
    }

    pub fn is_static(&self) -> bool {
        self.n.modifier_flags().contains(ModifierFlags::Static)
    }

    pub fn is_abstract(&self) -> bool {
        self.n.modifier_flags().contains(ModifierFlags::Abstract)
    }

    /// Whether the declaration is `declare`, or inside an ambient context.
    pub fn is_ambient(&self) -> bool {
        self.n.modifier_flags().contains(ModifierFlags::Ambient) || self.n.flags().contains(NodeFlags::Ambient)
    }

    pub fn is_async(&self) -> bool {
        self.n.modifier_flags().contains(ModifierFlags::Async)
    }

    pub fn is_generator(&self) -> bool {
        tsrs_ast::get_function_flags(Some(self.n)).contains(tsrs_ast::FunctionFlags::Generator)
    }

    /// A constructor parameter that declares a property.
    pub fn is_parameter_property(&self) -> bool {
        self.n.modifier_flags().intersects(ModifierFlags::ParameterPropertyModifier)
    }

    /// Whether a `break` or `continue` names a label.
    pub fn has_label(&self) -> bool {
        self.n.label().is_some()
    }

    pub fn is_type_only(&self) -> bool {
        self.n.is_type_only()
    }

    pub fn has_decorators(&self) -> bool {
        !self.n.decorators().is_empty()
    }

    /// The `extends` expression of a class.
    pub fn extends(&self) -> Option<Node> {
        let clauses = self.n.as_class_declaration().class_like_base.heritage_clauses.get()?;
        clauses
            .nodes()
            .iter()
            .find(|c| c.as_heritage_clause().token == TsKind::ExtendsKeyword)
            .and_then(|c| c.as_heritage_clause().types.get().nodes().first())
            .and_then(|t| t.expression())
            .map(|e| self.wrap(e))
    }

    pub fn module_specifier(&self) -> Option<Node> {
        self.wrap_opt(self.n.module_specifier())
    }

    pub fn import_clause(&self) -> Option<Node> {
        self.wrap_opt(self.n.import_clause())
    }

    pub fn named_bindings(&self) -> Option<Node> {
        self.wrap_opt(self.n.as_import_clause().named_bindings)
    }

    pub fn export_clause(&self) -> Option<Node> {
        self.wrap_opt(self.n.as_export_declaration().export_clause)
    }

    pub fn for_each_child(&self, f: &mut dyn FnMut(Node)) {
        let text = self.text;
        self.n.for_each_child(&mut |child| {
            f(Node { n: child, text });
            false
        });
    }

    /// Whether the node is a type annotation or another node that only exists for the checker.
    pub fn is_type(&self) -> bool {
        tsrs_ast::is_type_node_kind(self.n.kind())
            || matches!(
                self.n.kind(),
                TsKind::InterfaceDeclaration
                    | TsKind::TypeAliasDeclaration
                    | TsKind::TypeParameter
                    | TsKind::HeritageClause
                    | TsKind::IndexSignature
            )
    }
}

fn parse_integer(text: &str) -> Option<f64> {
    let text = text.replace('_', "");
    let (digits, radix) = match text.get(..2) {
        Some("0x" | "0X") => (&text[2..], 16),
        Some("0o" | "0O") => (&text[2..], 8),
        Some("0b" | "0B") => (&text[2..], 2),
        _ => return text.parse().ok(),
    };
    u64::from_str_radix(digits, radix).ok().map(|n| n as f64)
}
