//! The seam between the compiler and a framework transform. An extension sees the program through compiler-owned
//! handles and the narrow set of queries below, and never names a tsrs type. Each handle is a pointer into the one
//! program a compile builds, so a node seen while extracting is the same handle at lowering time.

use tsrs_ast::{Kind as TsKind, ModifierFlags, Node as TsNode, NodeFlags, SourceFile as TsSourceFile};
use tsrs_ast::{Symbol as TsSymbol, SymbolFlags as TsSymbolFlags};
use tsrs_checker::TypeFlags as TsTypeFlags;
use tsrs_checker::{LiteralValue as TsLiteral, ObjectFlags, Signature as TsSignature, Type as TsType};
use tsrs_core::P;

use crucible_luau::{Expr, Name, Stmt};

pub use crate::extract::crucible::Code;
pub use crate::extract::extension::ExtractCx;
pub use crate::lower::extension::LowerCx;

/// A framework transform. The compiler calls each registered extension at fixed points of its passes.
pub trait Extension: Send {
    fn name(&self) -> &'static str;
    /// Before any file is extracted, with the whole program built and checked.
    fn begin_program(&mut self, cx: &mut ExtractCx<'_>);
    /// After the compiler extracted its own facts for `file`.
    fn extract_file(&mut self, cx: &mut ExtractCx<'_>, file: SourceFile);
    /// Lowers a call the extension owns, in place of the compiler's own call lowering.
    fn lower_call(&self, cx: &mut LowerCx<'_, '_>, call: Node) -> Option<Expr>;
    /// Statements that follow a class declaration whose instance lives in `local`.
    fn lower_class_tail(&self, cx: &mut LowerCx<'_, '_>, class: Node, local: &Name) -> Vec<Stmt>;
}

/// A module a lowering calls into, which the compiler imports once per file into `local` on first use.
#[derive(Debug)]
pub struct HelperModule {
    /// The npm package and the export of its `exports` map whose declarations name the module.
    pub package: &'static str,
    pub entry: &'static str,
    pub local: &'static str,
    /// The error when the program's place cannot reach the module.
    pub missing: &'static str,
}

macro_rules! kinds {
    ($($kind:ident,)*) => {
        /// The syntax kinds an extension tells apart. Every other kind is `Other`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum SyntaxKind {
            $($kind,)*
            Other,
        }

        fn kind_of(kind: TsKind) -> SyntaxKind {
            match kind {
                $(TsKind::$kind => SyntaxKind::$kind,)*
                _ => SyntaxKind::Other,
            }
        }
    };
}

kinds! {
    SourceFile,
    Identifier,
    StringLiteral,
    NumericLiteral,
    NoSubstitutionTemplateLiteral,
    CallExpression,
    ParenthesizedExpression,
    AsExpression,
    NonNullExpression,
    SatisfiesExpression,
    TypeAssertionExpression,
    ObjectLiteralExpression,
    ArrayLiteralExpression,
    PropertyAssignment,
    SpreadAssignment,
    ComputedPropertyName,
    ClassDeclaration,
    ClassExpression,
    InterfaceDeclaration,
    TypeAliasDeclaration,
    HeritageClause,
    TypeReference,
    Constructor,
    MethodDeclaration,
    PropertyDeclaration,
    Parameter,
}

macro_rules! flags {
    ($(#[$doc:meta])* $name:ident($raw:ty) { $($flag:ident = $value:expr,)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name($raw);

        impl $name {
            $(pub const $flag: $name = $name($value);)*

            pub const fn union(self, other: $name) -> $name {
                $name(self.0.union(other.0))
            }

            pub fn intersects(self, other: $name) -> bool {
                self.0.intersects(other.0)
            }
        }
    };
}

flags! {
    /// A type's flags. Equality compares every flag, including those without a constant here.
    TypeFlags(TsTypeFlags) {
        BOOLEAN = TsTypeFlags::Boolean,
        BOOLEAN_LITERAL = TsTypeFlags::BooleanLiteral,
        NUMBER = TsTypeFlags::Number,
        STRING = TsTypeFlags::String,
        UNDEFINED = TsTypeFlags::Undefined,
        UNION = TsTypeFlags::Union,
        INTERSECTION = TsTypeFlags::Intersection,
        UNION_OR_INTERSECTION = TsTypeFlags::UnionOrIntersection,
    }
}

flags! {
    SymbolFlags(TsSymbolFlags) {
        CLASS = TsSymbolFlags::Class,
        INTERFACE = TsSymbolFlags::Interface,
        TYPE_ALIAS = TsSymbolFlags::TypeAlias,
    }
}

flags! {
    /// A declaration's modifiers, combined with those its declaration list carries.
    Modifiers(ModifierFlags) {
        ABSTRACT = ModifierFlags::Abstract,
        AMBIENT = ModifierFlags::Ambient,
        ASYNC = ModifierFlags::Async,
        PRIVATE = ModifierFlags::Private,
        PROTECTED = ModifierFlags::Protected,
        STATIC = ModifierFlags::Static,
    }
}

/// A literal type's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiteralValue {
    String(&'static str),
    Number(f64),
    Boolean(bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Node(pub(crate) P<TsNode>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceFile(pub(crate) P<TsSourceFile>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(pub(crate) P<TsSymbol>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Type(pub(crate) P<TsType>);

#[derive(Clone, Copy)]
pub struct Signature(pub(crate) P<TsSignature>);

fn nodes(list: &[P<TsNode>]) -> Vec<Node> {
    list.iter().map(|&n| Node(n)).collect()
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0.kind())
    }
}

impl Node {
    pub fn kind(self) -> SyntaxKind {
        kind_of(self.0.kind())
    }

    pub fn parent(self) -> Option<Node> {
        self.0.parent().map(Node)
    }

    pub fn for_each_child(self, f: &mut dyn FnMut(Node)) {
        self.0.for_each_child(&mut |child| {
            f(Node(child));
            false
        });
    }

    pub fn source_file(self) -> Option<SourceFile> {
        tsrs_ast::get_source_file_of_node(self.0).map(SourceFile)
    }

    /// An identifier's name, or a string or template literal's cooked value.
    pub fn text(self) -> &'static str {
        self.0.text()
    }

    pub fn name(self) -> Option<Node> {
        self.0.name().map(Node)
    }

    pub fn expression(self) -> Option<Node> {
        self.0.expression().map(Node)
    }

    pub fn initializer(self) -> Option<Node> {
        self.0.initializer().map(Node)
    }

    /// A declaration's type annotation, or the aliased type of a type alias.
    pub fn type_node(self) -> Option<Node> {
        self.0.type_node().map(Node)
    }

    /// A type reference's name.
    pub fn type_name(self) -> Option<Node> {
        (self.0.kind() == TsKind::TypeReference).then(|| Node(self.0.as_type_reference_node().type_name))
    }

    pub fn arguments(self) -> Vec<Node> {
        nodes(self.0.arguments())
    }

    pub fn type_arguments(self) -> Vec<Node> {
        nodes(self.0.type_arguments())
    }

    pub fn has_type_parameters(self) -> bool {
        !self.0.type_parameters().is_empty()
    }

    pub fn members(self) -> Vec<Node> {
        nodes(self.0.members())
    }

    pub fn properties(self) -> Vec<Node> {
        nodes(self.0.properties())
    }

    pub fn elements(self) -> Vec<Node> {
        nodes(self.0.elements())
    }

    pub fn modifiers(self) -> Modifiers {
        Modifiers(tsrs_ast::get_combined_modifier_flags(self.0))
    }

    /// Inside a `declare` context.
    pub fn is_ambient_context(self) -> bool {
        self.0.flags().intersects(NodeFlags::Ambient)
    }

    pub fn contains_this(self) -> bool {
        self.0.flags().intersects(NodeFlags::ContainsThis)
    }

    /// A parameter's `?`.
    pub fn has_question_token(self) -> bool {
        self.0.kind() == TsKind::Parameter && self.0.as_parameter_declaration().question_token.get().is_some()
    }

    /// A rest parameter.
    pub fn is_rest(self) -> bool {
        self.0.kind() == TsKind::Parameter && self.0.as_parameter_declaration().dot_dot_dot_token().is_some()
    }

    /// The types a class or interface declaration lists after `implements`.
    pub fn implemented_types(self) -> Vec<Node> {
        let clauses = match self.0.kind() {
            TsKind::ClassDeclaration => self.0.as_class_declaration().heritage_clauses(),
            TsKind::ClassExpression => self.0.as_class_expression().heritage_clauses(),
            TsKind::InterfaceDeclaration => self.0.as_interface_declaration().heritage_clauses(),
            _ => None,
        };
        clauses
            .map_or(&[][..], |l| l.nodes())
            .iter()
            .filter(|c| c.as_heritage_clause().token == TsKind::ImplementsKeyword)
            .flat_map(|c| c.as_heritage_clause().types.get().nodes().iter().map(|&n| Node(n)))
            .collect()
    }

    /// The types a declaration lists after `extends`.
    pub fn extends_types(self) -> Vec<Node> {
        nodes(tsrs_ast::get_extends_heritage_clause_elements(self.0))
    }

    /// Whether a heritage clause is an `extends` clause.
    pub fn is_extends_clause(self) -> bool {
        self.0.kind() == TsKind::HeritageClause && self.0.as_heritage_clause().token == TsKind::ExtendsKeyword
    }

    /// The name expression of an element of a heritage clause.
    pub fn heritage_name(self) -> Option<Node> {
        Some(Node(tsrs_ast::get_heritage_clause_element_name(self.0)))
    }

    /// A declaration in a global script, or in a `declare global` block of a module.
    pub fn is_global(self) -> bool {
        crate::extract::crucible::is_global(self.0)
    }

    /// The start of the node's first token, in bytes.
    #[cfg(feature = "test-support")]
    pub fn start(self) -> u32 {
        self.source_file().map_or(0, |file| crate::tree::span(self.0, file.0.text()).start)
    }
}

impl SourceFile {
    /// The absolute path with forward slashes.
    pub fn path(self) -> String {
        self.0.file_name().replace('\\', "/")
    }

    pub fn is_declaration(self) -> bool {
        self.0.is_declaration_file()
    }

    pub fn is_js(self) -> bool {
        self.0.is_js()
    }

    pub fn root(self) -> Node {
        Node(self.0.as_node())
    }
}

impl Symbol {
    pub fn name(self) -> &'static str {
        self.0.name()
    }

    pub fn flags(self) -> SymbolFlags {
        SymbolFlags(self.0.flags())
    }

    pub fn declarations(self) -> Vec<Node> {
        nodes(self.0.declarations())
    }

    pub fn value_declaration(self) -> Option<Node> {
        self.0.value_declaration().map(Node)
    }
}

impl Type {
    pub fn flags(self) -> TypeFlags {
        TypeFlags(self.0.flags())
    }

    pub fn symbol(self) -> Option<Symbol> {
        self.0.symbol().map(Symbol)
    }

    /// The symbol of the type alias that names this type.
    pub fn alias_symbol(self) -> Option<Symbol> {
        self.0.alias().and_then(|a| a.symbol()).map(Symbol)
    }

    /// A union's or intersection's members.
    pub fn types(self) -> Vec<Type> {
        self.0.types().iter().map(|&t| Type(t)).collect()
    }

    /// An instantiation of a generic class or interface, such as `All<T>`.
    pub fn is_reference(self) -> bool {
        self.0.flags().intersects(TsTypeFlags::Object) && self.0.object_flags().intersects(ObjectFlags::Reference)
    }

    /// The generic type a reference instantiates, or the type itself.
    pub fn target(self) -> Type {
        if self.is_reference() { Type(self.0.target().unwrap_or(self.0)) } else { self }
    }

    pub fn is_class_or_interface(self) -> bool {
        self.0.object_flags().intersects(ObjectFlags::ClassOrInterface)
    }

    /// A generic class or interface's type parameters.
    pub fn type_parameters(self) -> Vec<Type> {
        self.0.try_as_interface_type().map_or_else(Vec::new, |i| i.type_parameters().iter().map(|&t| Type(t)).collect())
    }

    pub fn literal(self) -> Option<LiteralValue> {
        let flags = self.0.flags();
        if !flags.intersects(TsTypeFlags::StringLiteral | TsTypeFlags::NumberLiteral | TsTypeFlags::BooleanLiteral) {
            return None;
        }
        match self.0.as_literal_type().value() {
            Some(TsLiteral::String(s)) if flags.intersects(TsTypeFlags::StringLiteral) => Some(LiteralValue::String(s)),
            Some(TsLiteral::Number(n)) if flags.intersects(TsTypeFlags::NumberLiteral) => {
                Some(LiteralValue::Number(n.0))
            }
            Some(TsLiteral::Boolean(b)) if flags.intersects(TsTypeFlags::BooleanLiteral) => {
                Some(LiteralValue::Boolean(b))
            }
            _ => None,
        }
    }
}

impl Signature {
    pub fn parameters(self) -> Vec<Symbol> {
        self.0.parameters().iter().map(|&s| Symbol(s)).collect()
    }
}
