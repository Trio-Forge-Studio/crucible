mod print;

use std::borrow::Cow;

pub use print::print;

pub type Name = Cow<'static, str>;

pub type Block = Vec<Stmt>;

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `values` may be empty.
    Local(Vec<Name>, Vec<Expr>),
    LocalFunction(Name, Function),
    /// The target is a name or a chain of constant string indexes.
    Function(Expr, Option<Name>, Function),
    Assign(Vec<Expr>, Vec<Expr>),
    Compound(BinOp, Expr, Expr),
    Call(Expr),
    Do(Block),
    If(Expr, Block, Option<Block>),
    While(Expr, Block),
    Repeat(Block, Expr),
    NumericFor(Name, Expr, Expr, Option<Expr>, Block),
    GenericFor(Vec<Name>, Vec<Expr>, Block),
    Return(Vec<Expr>),
    Break,
    Continue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Nil,
    Bool(bool),
    Number(f64),
    String(Name),
    VarArgs,
    Name(Name),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Method(Box<Expr>, Name, Vec<Expr>),
    Function(Box<Function>),
    Table(Vec<Field>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    If {
        condition: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    Interpolated(Vec<Part>),
    /// Truncates a call to its first value.
    Paren(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub params: Vec<Name>,
    pub varargs: bool,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Item(Expr),
    Keyed(Expr, Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(Name),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    Len,
}

impl Expr {
    pub fn name(name: impl Into<Name>) -> Expr {
        Expr::Name(name.into())
    }

    pub fn string(text: impl Into<Name>) -> Expr {
        Expr::String(text.into())
    }

    pub fn global(path: &'static str) -> Expr {
        match path.split_once('.') {
            Some((base, field)) => Expr::Name(Cow::Borrowed(base)).field(field),
            None => Expr::Name(Cow::Borrowed(path)),
        }
    }

    pub fn field(self, name: impl Into<Name>) -> Expr {
        Expr::Index(Box::new(self), Box::new(Expr::String(name.into())))
    }

    pub fn index(self, key: Expr) -> Expr {
        Expr::Index(Box::new(self), Box::new(key))
    }

    pub fn call(self, args: Vec<Expr>) -> Expr {
        Expr::Call(Box::new(self), args)
    }

    pub fn method(self, name: impl Into<Name>, args: Vec<Expr>) -> Expr {
        Expr::Method(Box::new(self), name.into(), args)
    }

    pub fn binary(self, op: BinOp, right: Expr) -> Expr {
        Expr::Binary(op, Box::new(self), Box::new(right))
    }

    pub fn unary(op: UnOp, operand: Expr) -> Expr {
        Expr::Unary(op, Box::new(operand))
    }

    pub fn negated(self) -> Expr {
        match self {
            Expr::Binary(BinOp::Eq, l, r) => Expr::Binary(BinOp::Ne, l, r),
            Expr::Binary(BinOp::Ne, l, r) => Expr::Binary(BinOp::Eq, l, r),
            Expr::Unary(UnOp::Not, e) if e.is_boolean() => *e,
            e => Expr::unary(UnOp::Not, e),
        }
    }

    pub fn if_else(condition: Expr, then: Expr, otherwise: Expr) -> Expr {
        Expr::If { condition: Box::new(condition), then: Box::new(then), otherwise: Box::new(otherwise) }
    }

    fn is_boolean(&self) -> bool {
        match self {
            Expr::Bool(_) => true,
            Expr::Unary(UnOp::Not, _) => true,
            Expr::Binary(op, ..) => {
                matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne)
            }
            _ => false,
        }
    }

    /// Evaluating the expression has no effect, and its value can only change when a statement assigns its name, so
    /// the lowering can repeat it instead of storing it.
    pub fn is_simple(&self) -> bool {
        matches!(self, Expr::Nil | Expr::Bool(_) | Expr::Number(_) | Expr::String(_) | Expr::Name(_) | Expr::VarArgs)
    }

    /// Whether evaluating the expression may run a function the program defines. A function value's body does not
    /// run, and `bit32` and `math` only compute numbers.
    pub fn may_call(&self) -> bool {
        match self {
            Expr::Call(function, args) => match function.as_ref() {
                Expr::Index(library, _) if matches!(library.as_ref(), Expr::Name(n) if n == "bit32" || n == "math") => {
                    args.iter().any(Expr::may_call)
                }
                _ => true,
            },
            Expr::Method(..) => true,
            Expr::Index(a, b) | Expr::Binary(_, a, b) => a.may_call() || b.may_call(),
            Expr::Unary(_, e) | Expr::Paren(e) => e.may_call(),
            Expr::If { condition, then, otherwise } => condition.may_call() || then.may_call() || otherwise.may_call(),
            Expr::Table(fields) => fields.iter().any(|f| match f {
                Field::Item(e) => e.may_call(),
                Field::Keyed(k, v) => k.may_call() || v.may_call(),
            }),
            Expr::Interpolated(parts) => parts.iter().any(|p| matches!(p, Part::Expr(e) if e.may_call())),
            Expr::Nil
            | Expr::Bool(_)
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::VarArgs
            | Expr::Name(_)
            | Expr::Function(_) => false,
        }
    }

    /// A call that may return several values or none.
    pub fn is_multi(&self) -> bool {
        matches!(self, Expr::Call(..) | Expr::Method(..) | Expr::VarArgs)
    }
}

pub fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return false };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !is_keyword(text)
}

pub fn is_keyword(text: &str) -> bool {
    matches!(
        text,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}
