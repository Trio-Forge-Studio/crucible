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
    /// `boolean | undefined`, whose defined `false` is what tells `??` from `||`.
    OptBool,
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
            Ty::OptBool => Ty::Bool,
            Ty::OptObj => Ty::Obj,
            other => panic!("{other:?} is not optional"),
        }
    }

    /// The optional type of an element of this array type.
    pub fn optional_element(self) -> Ty {
        match self {
            Ty::ArrInt => Ty::OptInt,
            Ty::ArrStr => Ty::OptStr,
            Ty::ArrBool => Ty::OptBool,
            Ty::ArrObj => Ty::OptObj,
            other => panic!("{other:?} has no optional element type"),
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
    /// `a % b`, where the generator makes the operands' signs agree: known.md, "% is floored".
    Mod(B, B),
    /// `abs(a)`
    Abs(B),
    /// `a.idiv(b)`, Luau's floor division `//`.
    Idiv(B, B),
    /// `(xs[i] as T | undefined)` over an array variable, with any index, in bounds or not.
    At(Ty, u8, B),
    /// `(maybe(xs, keep)?.[i] as T | undefined)`: an optional element read, whose index runs only when the array is
    /// there.
    OptAt(Ty, u8, bool, B),
    /// `xs.remove(i)` or `xs.unorderedRemove(i)` over an array variable, under a bounds check of a pure index.
    Remove(Ty, u8, B, bool),
    /// `(a === undefined)`
    IsUndefined(B),
    /// `((a as unknown) === (b as unknown))` over operands of different types, or `!==` when the flag is set.
    SameAny(B, B, bool),
    /// `(maybe(i, keep)?.m(n) ?? d)`, or `?.describe()` without `n`, over a class instance.
    OptMethod(B, bool, Option<B>, B),
    /// `(maybe(f, keep)?.(n) ?? d)` over an `FnII` variable.
    OptCallVar(u8, bool, B, B),
    /// `{ a, s, ...o }` with an optional `o`.
    ObjSpreadOpt(B, B, B),
    /// `a op b` as a statement, whose right side runs only when `op` says: `&&`, `||` or `??` over a left operand of
    /// the given type.
    Guard(Logical, Ty, B, B),
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
    /// `(o.a++)` and the like as a value, on an `Obj` variable.
    FieldStep(Step, u8),
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
pub enum Write {
    /// `xs[i] = v`, where `i` may also be the length, which appends.
    Set,
    /// `xs[i] += v` on an `ArrInt` or an `ArrStr`.
    Add,
    /// `xs[i]++` and the like on an `ArrInt`.
    Inc(Step),
    /// `xs[i] ||= v` or `xs[i] &&= v` on an `ArrBool`.
    Logical(Logical),
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
    /// A write to `xs[i]` with a pure index, under a bounds check.
    WriteAt(Ty, u8, Expr, Write, Expr),
    /// `xs.insert(i, v)` with a pure index, under a bounds check.
    Insert(Ty, u8, Expr, Expr),
    /// `xs.remove(i)` or `xs.unorderedRemove(i)` as a statement.
    Remove(Ty, u8, Expr, bool),
    /// `v.field &= k` (`>>>=` when the flag is set) on an `Int` field.
    FieldBit(Ty, Field, u8, bool, u8),
    /// `r.then ??= v`
    ThenCoalesce(u8, Expr),
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
    /// `+=` on an Int or a string, `-=` on an Int. A string's right side may be an Int or a boolean too.
    AddAssign(Ty, u8, Expr),
    SubAssign(u8, Expr),
    MulAssign(u8, u8),
    /// `-=`, `*=`, `/=` or `**=` (with an exponent of `POWS`) on a `Num` variable.
    NumAssign(u8, u8, Expr),
    /// `&= k` on an Int variable, or `>>>= k` when the flag is set.
    BitAssign(u8, bool, u8),
    /// A fresh Int set to the first expression, then a compound assignment by the second, then traced: `%=` of a
    /// value not below 0 by one above it (0) or of two below 0 (1), or `|=`, `^=`, `&=`, `<<=` or `>>=` (2 to 6) of
    /// `abs(a) % 256` by `abs(b) % 256`, or by `abs(b) % 9` for a shift.
    Compound(u8, Expr, Expr),
    /// A fresh string set to the first expression, then `+=` of the boolean, then traced: Luau's `..` turns a number
    /// into a string by itself but refuses a boolean, so only this shows that the boolean went through `tostring`.
    AddBool(Expr, Expr),
    Logical(Logical, Ty, u8, Expr),
    Inc(Step, u8),
    Trace(Ty, Expr),
    Expr(Expr),
    Mutate(Mutation),
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    /// `for (let i = 0; i < count; i += step)`, or counting down when the flag is set. With `capture`, each pass
    /// first stores a closure over `i` in a variable declared before the loop, which is called after it.
    For {
        count: u8,
        step: u8,
        down: bool,
        capture: bool,
        body: Vec<Stmt>,
    },
    /// `for (let i = 0; i < xs.size(); i++)` over an array variable, or from `xs.size() - 1` down to 0. With `peek`,
    /// the body first traces `xs[i + peek]`, which reads past an end on the first or the last pass. With `cull`, a
    /// loop over a variable it may change then removes and traces `xs[i]` when `i % m === 0`, with `unorderedRemove`
    /// when the flag is set, and the array is traced after the loop. Without such a variable, it loops over a copy,
    /// `[...xs, ...extra]`, with the extra elements after it.
    ForIndex {
        ty: Ty,
        source: u8,
        down: bool,
        peek: Option<i8>,
        cull: Option<(bool, u8, Vec<Expr>)>,
        body: Vec<Stmt>,
    },
    /// `for (const x of source)`, or `for (const { a: x, s: y } of source)` over an `ArrObj` with `destructure`.
    ForOf {
        ty: Ty,
        source: Expr,
        destructure: bool,
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
    /// `do { ... } while (guard < count)`, or `while (guard < count && cond)`, whose body then has no `continue`.
    /// With `shadow`, a `let w = 1` before the loop, a `w === 1` in the condition, and a `const w = 2` at the end of
    /// the body, which the condition must not see.
    DoWhile {
        count: u8,
        cond: Option<Expr>,
        shadow: bool,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    /// Each case is a constant, or a pure expression when it has one, then its body and whether it ends in `break`.
    Switch {
        disc: Expr,
        cases: Vec<(u8, Option<Expr>, Vec<Stmt>, bool)>,
        default: Option<Vec<Stmt>>,
    },
    Try {
        body: Vec<Stmt>,
        catch: Option<(bool, Vec<Stmt>)>,
        finally: Option<Vec<Stmt>>,
    },
    Throw(Expr),
    Return(Option<Expr>),
    /// `const [a = d, , b = d, ...rest] = source` over an `ArrInt`, `ArrStr` or `ArrBool`. `None` is a hole; the numbers pick
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
