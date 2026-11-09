//! The seam a framework transform plugs into. The compiler calls each extension at fixed points of its passes: once
//! before extraction, after its own extraction of each source file, once after the last file, and while lowering, at
//! each call and after each class declaration. An extension reads syntax through the compiler's tree view and the
//! checker through the handles and queries below, and never names a tsrs type. It keeps its own facts, keyed by
//! `SiteId`. The handles, flags and queries take tsrs's names, without the `_exported` suffix.

use std::ops::BitOr;
use std::path::PathBuf;

use crucible_luau::{Expr, Name, Stmt};
use tsrs_ast::Symbol as TsSymbol;
use tsrs_checker::{LiteralValue, Signature as TsSignature, Type as TsType};
use tsrs_core::P;

pub use crate::extract::extension::{ExtractCx, ProgramCx};
pub use crate::facts::{HelperModule, Span};
pub use crate::lower::extension::LowerCx;
pub use crate::tree::{File, Kind, Node, Nodes, Op, Position};

pub trait Extension: Send {
    /// Tells a host's extensions apart, such as in its logs. The compiler reads it only to name an extension that
    /// breaks a rule of the seam.
    fn name(&self) -> &'static str;

    /// Runs once with the whole program checked, before any file is extracted.
    fn begin_program(&mut self, _cx: &mut ProgramCx<'_>) {}

    /// Runs after the compiler extracted the facts of `file`.
    fn extract_file(&mut self, _cx: &mut ExtractCx<'_>, _file: File) {}

    /// Runs once after the last file was extracted.
    fn finish_program(&mut self, _cx: &mut ProgramCx<'_>) {}

    /// Lowers a call the extension owns in place of the compiler, or returns `None` to leave it to the compiler and
    /// the next extension. Decide before lowering anything: returning `None` after [`LowerCx::arguments`] panics. The
    /// compiler never asks about an optional-chain call, such as `a?.b()`, a `super(..)` or
    /// `super.m(..)` call, or a dynamic `import(..)`.
    fn lower_call(&self, _cx: &mut LowerCx<'_, '_>, _call: Node) -> Option<Expr> {
        None
    }

    /// Statements that follow the declaration of a class bound to the local `local`.
    fn lower_class_tail(&self, _cx: &mut LowerCx<'_, '_>, _class: Node, _local: &Name) -> Vec<Stmt> {
        Vec::new()
    }
}

/// A node of a source file the compiler compiles. Extraction and lowering name the same node with the same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SiteId {
    pub(crate) file: u32,
    pub(crate) span: Span,
}

/// tsrs's flags of one kind, with the constants extensions read under tsrs's names.
macro_rules! flags {
    ($(#[$doc:meta])* $name:ident($ts:path) { $($flag:ident,)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(pub(crate) $ts);

        #[allow(non_upper_case_globals)]
        impl $name {
            $(pub const $flag: $name = $name(<$ts>::$flag);)*

            pub fn intersects(self, other: $name) -> bool {
                self.0.intersects(other.0)
            }

            pub fn contains(self, other: $name) -> bool {
                self.0.contains(other.0)
            }
        }

        impl BitOr for $name {
            type Output = $name;

            fn bitor(self, other: $name) -> $name {
                $name(self.0 | other.0)
            }
        }
    };
}

flags! {
    TypeFlags(tsrs_checker::TypeFlags) {
        Any,
        BigIntLike,
        Boolean,
        BooleanLiteral,
        ESSymbolLike,
        Intersection,
        Never,
        Number,
        NumberLike,
        Object,
        String,
        TemplateLiteral,
        TypeParameter,
        Undefined,
        Union,
        UnionOrIntersection,
        Unknown,
        Void,
    }
}

flags! {
    ObjectFlags(tsrs_checker::ObjectFlags) {
        ClassOrInterface,
        Reference,
    }
}

flags! {
    SymbolFlags(tsrs_ast::SymbolFlags) {
        Alias,
        Class,
        Interface,
        Optional,
        Property,
        Type,
        TypeAlias,
        Variable,
    }
}

flags! {
    ModifierFlags(tsrs_ast::ModifierFlags) {
        Abstract,
        Ambient,
        Async,
        Export,
        Private,
        Protected,
        Readonly,
        Static,
    }
}

flags! {
    NodeFlags(tsrs_ast::NodeFlags) {
        Ambient,
        Const,
        ContainsThis,
    }
}

flags! {
    ElementFlags(tsrs_checker::ElementFlags) {
        Optional,
        Rest,
        Variadic,
    }
}

/// Which signatures of a type to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureKind {
    Call,
    Construct,
}

/// A symbol the checker bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol(pub(crate) P<TsSymbol>);

impl Symbol {
    pub fn name(&self) -> &'static str {
        self.0.name()
    }

    pub fn flags(&self) -> SymbolFlags {
        SymbolFlags(self.0.flags())
    }

    pub fn declarations(&self) -> Vec<Node> {
        self.0.declarations().iter().filter_map(|&d| Node::of(d)).collect()
    }

    pub fn value_declaration(&self) -> Option<Node> {
        self.0.value_declaration().and_then(Node::of)
    }
}

/// A type the checker computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Type(pub(crate) P<TsType>);

impl Type {
    pub fn flags(&self) -> TypeFlags {
        TypeFlags(self.0.flags())
    }

    pub fn object_flags(&self) -> ObjectFlags {
        ObjectFlags(self.0.object_flags())
    }

    pub fn symbol(&self) -> Option<Symbol> {
        self.0.symbol().map(Symbol)
    }

    /// The symbol of the type alias that names the type.
    pub fn alias_symbol(&self) -> Option<Symbol> {
        self.0.alias().and_then(|a| a.symbol()).map(Symbol)
    }

    /// A union's or an intersection's members.
    pub fn types(&self) -> Vec<Type> {
        self.0.types().iter().map(|&t| Type(t)).collect()
    }

    /// The generic type a reference instantiates.
    pub fn target(&self) -> Option<Type> {
        self.0.target().map(Type)
    }

    /// A class's or an interface's type parameters, and none for any other type.
    pub fn type_parameters(&self) -> Vec<Type> {
        self.0.try_as_interface_type().map_or(&[][..], |i| i.type_parameters()).iter().map(|&t| Type(t)).collect()
    }

    pub fn is_tuple_type(&self) -> bool {
        self.0.is_tuple_type()
    }

    /// The flags of each element of a tuple target, such as the `target` of a tuple reference.
    pub fn element_flags(&self) -> Vec<ElementFlags> {
        self.0.as_tuple_type().element_flags().iter().map(|&f| ElementFlags(f)).collect()
    }

    pub fn literal_string(&self) -> Option<&'static str> {
        match self.literal(tsrs_checker::TypeFlags::StringLiteral)? {
            LiteralValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn literal_number(&self) -> Option<f64> {
        match self.literal(tsrs_checker::TypeFlags::NumberLiteral)? {
            LiteralValue::Number(n) => Some(n.0),
            _ => None,
        }
    }

    pub fn literal_bool(&self) -> Option<bool> {
        match self.literal(tsrs_checker::TypeFlags::BooleanLiteral)? {
            LiteralValue::Boolean(b) => Some(b),
            _ => None,
        }
    }

    fn literal(&self, kind: tsrs_checker::TypeFlags) -> Option<LiteralValue> {
        if self.0.flags().intersects(kind) { self.0.as_literal_type().value() } else { None }
    }
}

/// A call or declaration signature the checker resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Signature(pub(crate) P<TsSignature>);

impl Signature {
    pub fn declaration(&self) -> Option<Node> {
        self.0.declaration().and_then(Node::of)
    }

    pub fn parameters(&self) -> Vec<Symbol> {
        self.0.parameters().iter().map(|&p| Symbol(p)).collect()
    }
}

/// An index signature of a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexInfo {
    pub key: Type,
    pub value: Type,
}

/// The number JavaScript's `Number(text)` gives.
pub fn js_number(text: &str) -> f64 {
    tsrs_core::jsnum::from_string(text).0
}

/// Whether TypeScript reads a property name as a number, because it is the text of the number it parses as.
pub fn is_numeric_name(name: &str) -> bool {
    tsrs_core::jsnum::from_string(name).to_string() == name
}

/// A package's `package.json`.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub path: PathBuf,
    pub json: serde_json::Value,
}
