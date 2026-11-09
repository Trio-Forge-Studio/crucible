//! What Crucible's extraction leaves for its lowering, keyed by the site it lowers.

use crate::declarations::DeclarationKind;

/// What a call or class lowers from.
#[derive(Clone, Debug, PartialEq)]
pub enum Fact {
    /// A class that extends `Subsystem` or `CrucibleBehaviour`, registered with `defineClass`.
    Class(Class),
    /// An `inject` call. Without a type argument its target is `Invalid`.
    Inject(Dependency),
    /// An `attribute` call.
    Attribute,
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
