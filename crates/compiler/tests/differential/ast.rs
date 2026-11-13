//! The generated programs. Every expression has a type fixed when it is generated, and a variable is a reference by
//! type and number that the printer resolves against the variables in scope. Shrinking can then delete any statement
//! and the program still type-checks: a reference whose variable is gone resolves to another, or to a literal.

/// The types a program uses. `Int` and `Num` are both `number` in TypeScript. An `Int` expression is always a finite
/// integer that is not `-0`, so it is the only number that ever becomes a string (see README.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ty {
    Int,
    Num,
    Str,
    Bool,
    OptInt,
    OptStr,
    ArrInt,
    ArrNum,
    ArrStr,
    ArrBool,
    ArrObj,
    /// `{ a: number; s: string }`
    Obj,
    /// `{ end: number; then?: string; inner: Obj }`, whose names are Luau keywords.
    Rec,
    OptObj,
    MapSI,
    MapIS,
    SetS,
    SetI,
    /// `(n: number) => number`
    FnII,
    /// `(s: string, n: number) => string`
    FnSIS,
    /// An instance of class `C<k>`.
    Inst(u8),
}

impl Ty {
    pub fn element(self) -> Ty {
        match self {
            Ty::ArrInt | Ty::SetI => Ty::Int,
            Ty::ArrNum => Ty::Num,
            Ty::ArrStr | Ty::SetS => Ty::Str,
            Ty::ArrBool => Ty::Bool,
            Ty::ArrObj => Ty::Obj,
            other => panic!("{other:?} has no element type"),
        }
    }

    pub fn required(self) -> Ty {
        match self {
            Ty::OptInt => Ty::Int,
            Ty::OptStr => Ty::Str,
            Ty::OptObj => Ty::Obj,
            other => panic!("{other:?} is not optional"),
        }
    }
}

pub type B = Box<Expr>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bit {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Left,
    Right,
    Unsigned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    PreInc,
    PostInc,
    PreDec,
    PostDec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// `o.a` of an `Obj`.
    A,
    /// `o.s` of an `Obj`.
    S,
    /// `r.end` of a `Rec`.
    End,
    /// `r.inner.a`
    InnerA,
    /// `r.inner.s`
    InnerS,
    /// `r.then`, which is optional.
    Then,
    /// `r.inner` itself.
    Inner,
}

/// A function literal. The printer names its parameters and gives them `params`' types.
#[derive(Clone, Debug)]
pub struct Lambda {
    pub params: Vec<Ty>,
    /// Write the parameter and return types, as a closure stored in a variable needs.
    pub annotate: Option<Ty>,
    /// Destructure a first parameter of type `Obj` as `{ a: x, s: y }`.
    pub destructure: bool,
    pub body: Body,
}

#[derive(Clone, Debug)]
pub enum Body {
    Expr(B),
    /// Statements, then the returned value, or nothing for a `void` callback.
    Block(Vec<Stmt>, Option<B>),
}

/// An element of an array literal.
#[derive(Clone, Debug)]
pub enum Item {
    One(Expr),
    Spread(Expr),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(u8),
    Num(u8),
    /// A string from the pool, and how it is quoted.
    Str(u8, u8),
    Bool(bool),
    Var(Ty, u8),

    Add(B, B),
    Sub(B, B),
    /// `a * k`, `k` from 2 to 5.
    MulK(B, u8),
    /// `abs(a) % k`
    ModK(B, u8),
    /// `(abs(a) % 256) op (abs(b) % 256)`
    Bit(Bit, B, B),
    /// `(abs(a) % 256) op k`
    Shift(Shift, B, u8),
    /// `+a`
    Plus(B),
    /// `(a as number)` or `(a satisfies number)`.
    Cast(B, bool),
    /// `.size()` of a string or collection of the given type.
    Size(B),
    IndexOf(B, B, Option<i8>),
    Includes(B, B, Option<i8>),
    FindIndex(B, Box<Lambda>),
    /// `v.push(...)` as a value: the new length.
    Push(Ty, u8, Vec<Expr>),
    /// `xs.reduce(f, init)`, or without an initial value over an array literal that is never empty.
    Reduce(B, Box<Lambda>, Option<B>),
    ReduceLit(Vec<Expr>, Box<Lambda>),
    /// `(xs.size() > k ? xs[k] : d)` over an array variable.
    Index(Ty, u8, u8, B),
    Cond(B, B, B),
    Coalesce(B, B),
    /// `a || b` and `a && b`, whose left operand has the given type.
    Or(Ty, B, B),
    And(Ty, B, B),
    Field(Field, B),
    /// `(o?.a ?? d)` or `(o?.s ?? d)` over an optional object.
    OptField(Field, B, B),
    /// `o?.a`, which is optional.
    Chain(Field, B),
    /// `(r.then?.size() ?? d)`
    OptSize(B, B),
    /// `maybe(o, true)!.a`
    NonNull(Field, B),
    /// `this` inside a method.
    This,
    /// `super.m(n)` inside a subclass's method.
    SuperM(B),
    InstX(B),
    InstY(u8, B),
    InstTag(B),
    InstM(B, B),
    InstDescribe(B),
    /// `C<k>.count`
    Count(u8),
    /// `C0.make(n)`
    Make(B),
    /// `f<k>(args)` for top-level function `k`. A spread argument ends the list of `f3`.
    Call(u8, Vec<Expr>, Option<B>),
    /// A call of a function variable of the given type.
    CallVar(Ty, u8, Vec<Expr>),
    Inc(Step, u8),
    Assign(Ty, u8, B),
    AddAssign(Ty, u8, B),
    MapGet(B, B),

    Div(B, B),
    Mul(B, B),
    Neg(B),
    Pow(B, u8),

    Concat(B, B),
    Template(u8, Vec<(Expr, u8)>),
    Upper(B),
    Lower(B),
    Rep(B, u8),
    /// `xs.join(sep)`, or `xs.join()` for `None`.
    Join(B, Option<u8>),
    /// `o["a"]`: a field read with a string key.
    Bracket(Field, B),
    /// `(j >= 0 && j < xs.size() ? xs[j] : d)` over an array variable and an `Int` variable.
    IndexAt(Ty, u8, u8, B),

    /// A comparison of two operands of the given type.
    Cmp(Cmp, Ty, B, B),
    Not(Ty, B),
    IsEmpty(B),
    Some(B, Box<Lambda>),
    Every(B, Box<Lambda>),
    Has(B, B),
    /// `.delete(key)` as a value, on a map or set variable.
    Delete(Ty, u8, B),
    InstanceOf(B, u8),
    /// `"then" in r` and the like.
    In(Field, B),
    Same(B, B),
    NaN(B),

    /// `maybe(value, keep)`: `value` or `undefined`, in a way TypeScript cannot narrow.
    Maybe(B, bool),
    Pop(Ty, u8),
    ShiftArr(Ty, u8),
    Find(B, Box<Lambda>),

    Array(Ty, Vec<Item>),
    Map(B, Box<Lambda>),
    Filter(B, Box<Lambda>),
    Split(B, u8),

    Obj(B, B),
    ObjSpread(B, Option<B>),
    Rec(B, Option<B>, B),
    RecSpread(B, B),

    NewMap(Ty, Vec<(Expr, Expr)>),
    NewSet(Ty, Vec<Expr>),
    NewSetFrom(Ty, B),

    Lambda(Box<Lambda>),
    New(u8, B),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Logical {
    Coalesce,
    Or,
    And,
}

#[derive(Clone, Debug)]
pub enum Mutation {
    Push(Ty, u8, Vec<Expr>),
    Unshift(Ty, u8, Vec<Expr>),
    Pop(Ty, u8),
    Shift(Ty, u8),
    Clear(Ty, u8),
    /// `if (xs.size() > k) xs[k] = v`
    SetIndex(Ty, u8, u8, Expr),
    /// `xs[xs.size()] = v`
    Append(Ty, u8, Expr),
    MapSet(Ty, u8, Expr, Expr),
    Delete(Ty, u8, Expr),
    SetAdd(Ty, u8, Expr),
    /// `v.field = value` on an object variable, or `+=` when the flag is set.
    Field(Ty, Field, u8, Expr, bool),
    /// `delete r.then`
    DeleteThen(u8),
    /// `v.a++` and the like on an `Int` field.
    FieldInc(Ty, Field, u8, Step),
    /// `if (xs.size() > k) xs[k] += v`
    IndexAdd(u8, u8, Expr),
}

/// How an aggregate visits a map or set. Each adds an order-free term to a fresh sum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visit {
    /// `for (const [k, v] of m)`
    Entries,
    /// `for (const [, v] of m)`, or `for (const k of s)` for a set
    Values,
    /// `for (const e of m)` with `e[0]` and `e[1]`
    Whole,
    ForEach,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        ty: Ty,
        mutable: bool,
        shadow: Option<u8>,
        init: Expr,
    },
    Assign(Ty, u8, Expr),
    /// `+=` on an Int or a string, `-=` on an Int.
    AddAssign(Ty, u8, Expr),
    SubAssign(u8, Expr),
    MulAssign(u8, u8),
    Logical(Logical, Ty, u8, Expr),
    Inc(Step, u8),
    Trace(Ty, Expr),
    Expr(Expr),
    Mutate(Mutation),
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    /// `for (let i = 0; i < count; i += step)`, or counting down when the flag is set.
    For {
        count: u8,
        step: u8,
        down: bool,
        body: Vec<Stmt>,
    },
    ForOf {
        ty: Ty,
        source: Expr,
        body: Vec<Stmt>,
    },
    ForOfStr(Expr, Vec<Stmt>),
    /// A sum over a map or set: a fresh `let` Int, then a loop that adds `term` of each entry.
    Aggregate {
        ty: Ty,
        source: u8,
        visit: Visit,
        term: Expr,
    },
    ForEach {
        ty: Ty,
        source: Expr,
        body: Vec<Stmt>,
    },
    /// `while (guard < count && cond)`, with the guard counted first in the body.
    While {
        count: u8,
        cond: Expr,
        body: Vec<Stmt>,
    },
    DoWhile {
        count: u8,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Switch {
        disc: Expr,
        cases: Vec<(u8, Vec<Stmt>, bool)>,
        default: Option<Vec<Stmt>>,
    },
    Try {
        body: Vec<Stmt>,
        catch: Option<(bool, Vec<Stmt>)>,
        finally: Option<Vec<Stmt>>,
    },
    Throw(Expr),
    Return(Option<Expr>),
    /// `const [a = d, , b = d, ...rest] = source` over an `ArrInt` or `ArrStr`. `None` is a hole; the numbers pick
    /// each default.
    DestructureArray {
        ty: Ty,
        source: Expr,
        elements: Vec<Option<u8>>,
        rest: bool,
    },
    /// `const { a: x, s: y } = o`, or `{ end: x, then: y = d, inner: { a: z } } = r`.
    DestructureObject {
        ty: Ty,
        source: Expr,
        fields: Vec<Field>,
        default: u8,
    },
    /// `[x, y] = [y, x]`
    Swap(u8, u8),
    /// `({ a: x, s: y } = o)`
    AssignObject(u8, u8, Expr),
    Block(Vec<Stmt>),
    /// `function g(n: number): number { ... }` inside a block, which adds an `FnII` variable.
    LocalFunction(Vec<Stmt>, Expr),
}

#[derive(Clone, Debug)]
pub struct Class {
    /// The base class, always a lower number.
    pub parent: Option<u8>,
    /// The body and returned value of `m(n: number): number`. A subclass may call `super.m(n)` in it.
    pub method: Option<(Vec<Stmt>, Expr)>,
    /// Count instances in the static `count` (only `C0` declares it).
    pub count: bool,
    /// A subclass: what its constructor adds to `x` before `super(x + k)`, or no constructor of its own.
    pub constructor: Option<u8>,
    /// A subclass overrides `describe()` and calls `super.describe()`.
    pub describe: bool,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub body: Vec<Stmt>,
    pub ret: Expr,
}

/// The fixed signatures of the top-level functions. A function body calls only lower-numbered functions, so no
/// call cycle exists, except `f4`'s own countdown.
pub const SIGNATURES: [(&[Ty], Ty); 5] = [
    (&[Ty::Int], Ty::Int),
    (&[Ty::Str, Ty::Int], Ty::Str),
    (&[Ty::ArrInt, Ty::Int], Ty::Int),
    (&[Ty::Int, Ty::ArrInt], Ty::Int),
    (&[Ty::Int], Ty::Int),
];

#[derive(Clone, Debug)]
pub struct Program {
    pub classes: Vec<Class>,
    pub functions: Vec<Function>,
    pub body: Vec<Stmt>,
    /// Print the functions after the body, which JavaScript hoists.
    pub functions_last: bool,
}

pub const INTS: &[&str] = &[
    "0", "1", "2", "3", "5", "7", "10", "42", "100", "255", "1000", "-1", "-3", "-42", "0x1f", "1_000", "1e3", "0b101",
    "0o17", "65536",
];
pub const NUMS: &[&str] = &[
    "0.5",
    "1.5",
    "-2.25",
    "0.1",
    "1e-3",
    "3.14159",
    "1e21",
    "2.5e-7",
    "1e300",
    "-0",
    "0.30000000000000004",
    "123.456",
    "4503599627370497",
    "9007199254740993",
];
/// ASCII only: known.md, "A string's size counts bytes".
pub const STRS: &[&str] = &[
    "",
    "a",
    "b",
    "ab",
    "abc",
    "hello world",
    "a,b,c",
    ",",
    "x\ny",
    "tab\there",
    "q\"q",
    "back\\slash",
    "A",
    "end",
    "nil",
    "%d %s",
    "[[x]]",
    "--",
    "]]",
    "it's",
    "`tick`",
    "${x}",
    "\u{1}",
    "a\0b",
];
pub const JOINS: &[&str] = &[",", ", ", "", "-", "|"];
pub const SPLITS: &[&str] = &[",", " ", "ab", "a", ""];
