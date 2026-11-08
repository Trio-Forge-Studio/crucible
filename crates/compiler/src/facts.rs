//! The fact table. Each answer the lowering needs from the checker is stored ahead of time as plain data, keyed by the
//! syntax node it is about, so the lowering never calls the checker. Nothing here refers to tsrs.

use std::fmt;
use std::ops::BitOr;

use crate::Diagnostic;
use crate::declarations::DeclarationKind;
use crate::macros::Macro;

/// A node's source range in UTF-8 bytes, from the start of its first token to its end. Leading trivia is excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// `None` in a field means no site on the node asks for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fact {
    /// Computed over the non-nullable part of the type, with `falsy` saying whether the value may also be nil.
    pub receiver: Option<Receiver>,
    /// What a call or tagged template callee, a `new` callee or an identifier macro resolves to, and whether a
    /// function takes `self`.
    pub callee: Option<Callee>,
    /// Whether a callee may be a Roblox API, which tells a missing argument from `nil`. Asked at calls with a call
    /// argument, which the lowering wraps in parentheses when it may return nothing.
    pub roblox_api: Option<bool>,
    pub class_constructor: Option<bool>,
    pub falsy: Option<Falsy>,
    /// The value of an enum member or a const enum member access.
    pub constant: Option<Constant>,
    /// Whether a numeric `for` loop bound is definitely an integer.
    pub integer: Option<bool>,
    /// How many values each step of an iterated `LuaTuple` iterator has. Absent where the tuple's length varies.
    pub step_length: Option<u32>,
    pub import: Option<Import>,
    /// Set at a registered class, an `inject`, declaration or `receive` call, and a `boot` load glob.
    pub crucible: Option<Box<Crucible>>,
    /// Where an imported module's instance is, set at an import's module specifier in a game.
    pub module: Option<Box<ModulePath>>,
}

impl Fact {
    /// Whether the value may be `undefined`, `null` or `void`. False where the site does not ask.
    pub fn nullable(&self) -> bool {
        self.falsy.is_some_and(|f| f.contains(Falsy::NIL))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Receiver {
    Array,
    Map,
    Set,
    String,
    /// A `LuaTuple`, which lowers to multiple values.
    LuaTuple,
    /// An `IterableFunction`, which a `for..of` calls as a Luau iterator.
    IterFn,
    /// An `IterableFunction` whose steps are `LuaTuple`s, so each step is several values.
    LuaTupleIterFn,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Callee {
    Macro(Macro),
    /// Called with `:`, or a function that takes `self`.
    Method,
    /// Called with `.`, or a function that takes no `self`.
    Function,
}

/// The JavaScript-falsy values an expression may hold. Luau treats only `nil` and `false` as falsy, so a truthiness
/// check tests the other bits explicitly.
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
    /// A default import of a module that assigns `export =`, which binds the whole module rather than its `default`.
    KeepModule,
    Elide,
}

impl Import {
    pub fn kept(self) -> bool {
        self != Import::Elide
    }
}

/// How a file's JSX lowers, from the compiler options and the file's pragmas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Jsx {
    /// `jsx: "react"`. Each element calls `factory`, such as `Vide.jsx`, with its tag, its attributes and its
    /// children, as roblox-ts emits it, and a fragment passes `fragment` as its tag. Both are dotted names.
    Factory { factory: Box<str>, fragment: Option<Box<str>> },
    /// Any other `jsx` setting, which the lowering refuses.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Crucible {
    /// A class that extends `Subsystem` or `CrucibleBehaviour`, registered with `defineClass`.
    Class(Class),
    /// An `inject` call. Without a type argument its target is `Invalid`.
    Inject(Dependency),
    /// A `boot` load glob, as the instance path pattern the runtime matches, such as
    /// `crucible-path:ServerScriptService/TS/systems/**`.
    Load(Box<str>),
    /// A declaration macro call, which lowers to the `@trioforge/crucible/emit` function of the same name.
    Declaration(Declaration),
    /// A `receive` call, which lowers to `receive(self, field, target, callback)`.
    Receive { field: Box<str> },
}

/// `__crucible.<kind>(name, ...kept arguments, ...schemas, ...tail)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub kind: DeclarationKind,
    /// A message's payload, an ask's request and response, or a state's value. A profile store has the saved value
    /// and then the replica its owner's client receives.
    pub schemas: Vec<Schema>,
    pub tail: Tail,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tail {
    None,
    /// An ask's timeout in seconds, or `None` for the runtime's default.
    Timeout(Option<f64>),
    /// The paths of a profile store's `serverOnly` fields, each from the value's root.
    ServerOnly(Vec<Vec<Box<str>>>),
}

/// A value's wire schema, which the runtime's `n` builders describe.
#[derive(Clone, Debug, PartialEq)]
pub enum Schema {
    Nil,
    Bool,
    F64,
    F32,
    U8,
    U16,
    U32,
    String,
    /// `unknown` in a payload, which the codec checks at run time.
    Dynamic,
    Vector3,
    Color3,
    CFrame,
    Literal(Literal),
    /// `Instances` class names, sorted by their bytes.
    Instance(Vec<Box<str>>),
    /// Sorted by the bytes of their names.
    Object(Vec<Field>),
    Array(Box<Schema>),
    Tuple(Vec<Schema>),
    Map(MapKey, Box<Schema>),
    Union(Vec<Schema>),
    /// One member of a union of objects: its discriminant property, one literal value of it, and the object without it.
    Variant {
        key: Box<str>,
        value: Literal,
        body: Box<Schema>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Box<str>,
    pub value: Schema,
    /// A `serverOnly` profile store field, which the replica leaves out.
    pub server_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapKey {
    String,
    U8,
    U16,
    U32,
    /// A union of literals, numbers before strings, each group sorted.
    Literals(Vec<KeyLiteral>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyLiteral {
    Number(f64),
    String(Box<str>),
}

/// An instance path a module reaches another module by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModulePath {
    /// Up `parents` levels from the script, then down `path`.
    Relative { parents: u32, path: Vec<Box<str>> },
    /// From a service of the place, then down `path`.
    Absolute { service: Box<str>, path: Vec<Box<str>> },
    /// From the installed package `scope/name` that `TS.getModule` finds above the script, then down `path`. A
    /// package's import of another package, as roblox-ts emits it.
    Package { scope: Box<str>, name: Box<str>, path: Vec<Box<str>> },
}

/// How a module gets roblox-ts's runtime library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RuntimeLib {
    /// From `_G[script]`, which `TS.import` sets for each module it loads, as in a package.
    #[default]
    Global,
    /// By requiring it at its instance path. A game's scripts and the modules `boot` requires have no `_G[script]`.
    Require(ModulePath),
}

/// What every module of a program shares about the place it runs in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub runtime_lib: RuntimeLib,
    /// The modules lowerings call into, in the order they were declared.
    pub helpers: Vec<Helper>,
}

/// A runtime module that compiled code calls into through one local, such as `__crucible`. A file that uses it imports
/// it at its top, after roblox-ts's runtime library and in the order of first use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelperModule {
    /// The npm package that has the module.
    pub package: &'static str,
    /// The key of the package's `exports` whose declarations are the module's, such as `./emit`.
    pub entry: &'static str,
    /// The program cannot declare this name.
    pub local: &'static str,
    /// The error in a file that uses the module where the place cannot reach it.
    pub missing: &'static str,
}

/// A helper module, resolved for the program's place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Helper {
    pub module: HelperModule,
    /// `None` outside a game, or where the game has no instance for the module.
    pub path: Option<ModulePath>,
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
    /// A target CRU001 or CRU005 rejects, or one without a type id. The pass reports it and the compile fails.
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
    /// A type CRU007 rejects. The pass reports it and the compile fails.
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
    /// `None` when the file has no JSX.
    pub jsx: Option<Jsx>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Site {
    /// The callee of a call or a tagged template.
    CallCallee,
    NewCallee,
    InstanceOfClass,
    /// A reference to an identifier macro.
    Identifier,
    /// An indexed value, or an object pattern with a computed key.
    ElementReceiver,
    PlusOperand,
    /// A condition, a logical operand or a `??` chain.
    Truthiness,
    Iterable,
    /// An array, call, object or JSX spread.
    Spread,
    Destructure,
    /// A call receiver, a call result or a returned value.
    CallOrReturn,
    Function,
    Constant,
    Import,
    Crucible,
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
    /// An array assignment pattern, or an object one with a computed key, nested in another pattern or in a `for..of`
    /// head that needs its source type. tsrs keeps the checker's assignment pattern type private.
    AssignmentPattern,
    /// A spread argument to a macro. roblox-ts rejects it for a variadic macro, and otherwise needs the tuple's length.
    MacroSpread,
    /// Two sites on one node got different answers.
    Conflict,
}

#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub files: Vec<FileFacts>,
    pub unanswered: Vec<Unanswered>,
    /// Sites visited, answered or not.
    pub sites: usize,
    /// Errors in code that type-checks: Crucible's rules, and modules or place paths the game cannot reach. Sorted
    /// by path and position.
    pub diagnostics: Vec<Diagnostic>,
    pub place: Place,
}

#[cfg(feature = "test-support")]
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
