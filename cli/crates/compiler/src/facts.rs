//! The fact table. Each answer the lowering needs from the checker is stored ahead of time as plain data, keyed by the
//! syntax node it is about, so the lowering never calls the checker. Nothing here refers to tsrs.

use std::fmt;
use std::ops::BitOr;

use crate::macros::Macro;

/// A node's source range in UTF-8 bytes, from the start of its first token to its end. Leading trivia is excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// What the lowering of one node needs from the checker. A field is set only at the sites that ask for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fact {
    /// The class of a value whose lowering depends on it. Computed over the non-nullable part of the type, with
    /// `falsy` saying whether the value may also be nil.
    pub receiver: Option<Receiver>,
    /// What a call, a `new` callee or an identifier macro resolves to, and whether a function takes `self`.
    pub callee: Option<Callee>,
    /// The JavaScript-falsy values the expression may hold.
    pub falsy: Option<Falsy>,
    /// The value of an enum member, an enum member access or an integer literal type.
    pub constant: Option<Constant>,
    /// Whether an import or re-export binding must stay in the output.
    pub import: Option<Import>,
    /// What Crucible's transform reads at a class, an `inject` call or an `attribute` call.
    pub crucible: Option<Box<Crucible>>,
}

impl Fact {
    /// Whether the value may be `undefined`, `null` or `void`. False where the site does not ask.
    pub fn nullable(&self) -> bool {
        self.falsy.is_some_and(|f| f.contains(Falsy::NIL))
    }
}

/// The class of a value, for method calls, indexing, iteration, spreads, destructuring, call results and `+`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Receiver {
    Array,
    Map,
    Set,
    String,
    /// A `LuaTuple`, which lowers to multiple values.
    LuaTuple,
    /// An `IterableFunction`, which a `for..of` calls as a Luau iterator. Its binding pattern says whether each step
    /// yields a `LuaTuple`.
    IterFn,
    Other,
}

/// What a call lowers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Callee {
    Macro(Macro),
    /// Called with `:`, or a function that takes `self`.
    Method,
    /// Called with `.`, or a function that takes no `self`.
    Function,
}

/// A set of JavaScript-falsy values. Luau treats only `nil` and `false` as falsy, so a truthiness check tests the
/// other bits explicitly.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Falsy(u8);

impl Falsy {
    /// `undefined`, `null` or `void`. This is the fact's `nullable`.
    pub const NIL: Falsy = Falsy(1);
    pub const FALSE: Falsy = Falsy(1 << 1);
    pub const ZERO: Falsy = Falsy(1 << 2);
    pub const NAN: Falsy = Falsy(1 << 3);
    pub const EMPTY_STRING: Falsy = Falsy(1 << 4);
    pub const ALL: Falsy = Falsy(0b1_1111);
    pub const NONE: Falsy = Falsy(0);

    pub const fn contains(self, other: Falsy) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn without(self, other: Falsy) -> Falsy {
        Falsy(self.0 & !other.0)
    }
}

impl BitOr for Falsy {
    type Output = Falsy;

    fn bitor(self, other: Falsy) -> Falsy {
        Falsy(self.0 | other.0)
    }
}

impl fmt::Debug for Falsy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (Falsy::NIL, "NIL"),
            (Falsy::FALSE, "FALSE"),
            (Falsy::ZERO, "ZERO"),
            (Falsy::NAN, "NAN"),
            (Falsy::EMPTY_STRING, "EMPTY_STRING"),
        ];
        let set: Vec<&str> = names.iter().filter(|(bit, _)| self.contains(*bit)).map(|(_, name)| *name).collect();
        if set.is_empty() { f.write_str("NONE") } else { f.write_str(&set.join(" | ")) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Number(f64),
    String(Box<str>),
}

/// Whether an import or re-export binding stays. Elided bindings are used only as types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Import {
    Keep,
    Elide,
}

/// What Crucible's transform needs from the checker.
#[derive(Clone, Debug, PartialEq)]
pub enum Crucible {
    /// A class that extends `Subsystem` or `CrucibleBehaviour`, registered with `defineClass`.
    Class(Class),
    /// `inject<T>()` or `inject<All<T>>()`.
    Inject(Dependency),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassKind {
    Subsystem,
    Behaviour,
}

/// The registration data of a concrete Crucible class. A type id is `<package>/<module path>:<name>`, and `None` where the
/// transform reports that it cannot compute one.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub kind: ClassKind,
    pub id: Option<Box<str>>,
    /// The constructor's parameters in order.
    pub deps: Vec<Dependency>,
    /// Every base class and implemented interface below Crucible's own classes.
    pub provides: Vec<Box<str>>,
    /// The behaviour's tag literal.
    pub tag: Option<Box<str>>,
    /// The behaviour's `Instances` class names.
    pub instance: Vec<Box<str>>,
    pub attributes: Vec<Attribute>,
}

/// A constructor parameter or `inject` target, by type id.
#[derive(Clone, Debug, PartialEq)]
pub enum Dependency {
    One(Box<str>),
    /// `All<T>`.
    All(Box<str>),
    /// A target the transform rejects.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub field: Box<str>,
    pub check: Check,
    /// False when `attribute()` has a default value.
    pub required: bool,
}

/// A runtime type check for an attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Boolean,
    String,
    Number,
    Vector3,
    Color3,
    Literal(Literal),
    Union(Vec<Check>),
    /// A type the transform rejects.
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Boolean(bool),
    Number(f64),
    String(Box<str>),
}

/// The facts of one source file, sorted by span.
#[derive(Clone, Debug)]
pub struct FileFacts {
    /// The absolute path with forward slashes.
    pub path: String,
    pub facts: Vec<(Span, Fact)>,
}

impl FileFacts {
    pub fn get(&self, span: Span) -> Option<&Fact> {
        self.facts.binary_search_by_key(&span, |(s, _)| *s).ok().map(|i| &self.facts[i].1)
    }
}

/// A type-dependent site the extractor could not answer. The lowering would have to report it as an error.
#[derive(Clone, Debug, PartialEq)]
pub struct Unanswered {
    pub path: String,
    pub span: Span,
    pub site: Site,
    pub reason: Reason,
}

/// The kind of site an unanswered entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Site {
    CallCallee,
    ElementReceiver,
    PlusOperand,
    Iterable,
    Spread,
    Destructure,
    Function,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// The type is `any`.
    Any,
    /// A union whose members lower differently, such as `string | string[]`.
    Mixed,
    /// A callee whose signatures mix methods and plain functions.
    MethodAndFunction,
    /// An iterable that is not an array, set, map, string, `LuaTuple` or `IterableFunction`.
    Iteration,
}

/// The fact table of a checked program.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub files: Vec<FileFacts>,
    pub unanswered: Vec<Unanswered>,
    /// Sites visited, answered or not.
    pub sites: usize,
}

impl Facts {
    /// The facts of the file at `path`, an absolute path in either slash direction.
    pub fn file(&self, path: &str) -> Option<&FileFacts> {
        let path = path.replace('\\', "/");
        self.files.iter().find(|f| f.path == path)
    }

    /// Facts stored, one per node.
    pub fn len(&self) -> usize {
        self.files.iter().map(|f| f.facts.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
