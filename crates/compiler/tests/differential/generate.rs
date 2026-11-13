//! The proptest strategies that generate programs. Each `prop_oneof` lists its simplest choice first, which is where
//! proptest shrinks to, and strategies are built one level at a time, when a value is drawn.

use std::fmt;
use std::rc::Rc;

use proptest::collection::vec;
use proptest::option;
use proptest::prelude::*;
use proptest::sample::select;
use proptest::strategy::{NewTree, Union, ValueTree};
use proptest::test_runner::TestRunner;

use super::ast::*;

struct Lazy<T>(Rc<dyn Fn() -> BoxedStrategy<T>>);

impl<T> fmt::Debug for Lazy<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Lazy")
    }
}

impl<T: fmt::Debug + 'static> Strategy for Lazy<T> {
    type Tree = Box<dyn ValueTree<Value = T>>;
    type Value = T;

    fn new_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        (self.0)().new_tree(runner)
    }
}

fn lazy<T: fmt::Debug + 'static>(build: impl Fn() -> BoxedStrategy<T> + 'static) -> BoxedStrategy<T> {
    Lazy(Rc::new(build)).boxed()
}

/// Where a statement or expression is generated.
#[derive(Clone, Copy, Debug)]
pub struct Cx {
    /// Levels of expression left.
    depth: u8,
    /// Levels of nested statements left.
    nest: u8,
    /// A `break` has a loop or switch to leave.
    breakable: bool,
    /// A `continue` has a loop, and no switch lies between.
    continuable: bool,
    /// Inside a function that returns this type, or `void` for `Some(None)`.
    ret: Option<Option<Ty>>,
    /// Inside a method of this class.
    own: Option<u8>,
    /// `super.m` exists.
    sup: bool,
    /// Top-level functions below this number may be called.
    calls: u8,
    /// Instances of classes may be made and their methods called.
    classes: bool,
}

const EXPRESSION_DEPTH: u8 = 3;
const NESTING: u8 = 3;

impl Cx {
    pub fn top() -> Cx {
        Cx {
            depth: EXPRESSION_DEPTH,
            nest: NESTING,
            breakable: false,
            continuable: false,
            ret: None,
            own: None,
            sup: false,
            calls: SIGNATURES.len() as u8,
            classes: true,
        }
    }

    fn down(self) -> Cx {
        Cx { depth: self.depth.saturating_sub(1), ..self }
    }

    /// A statement's expressions start from the full depth.
    fn statement(self) -> Cx {
        Cx { depth: EXPRESSION_DEPTH, ..self }
    }

    fn nested(self) -> Cx {
        Cx { nest: self.nest.saturating_sub(1), depth: EXPRESSION_DEPTH, ..self }
    }

    fn in_loop(self) -> Cx {
        Cx { breakable: true, continuable: true, ..self.nested() }
    }

    fn in_switch(self) -> Cx {
        Cx { breakable: true, continuable: false, ..self.nested() }
    }

    /// A function body: loops and switches outside it are out of reach.
    fn function(self, ret: Option<Ty>) -> Cx {
        Cx { breakable: false, continuable: false, ret: Some(ret), ..self.nested() }
    }

    fn inst_types(self) -> Vec<u8> {
        match (self.classes, self.own) {
            (true, _) => vec![0, 1, 2],
            (false, Some(own)) => vec![own],
            (false, None) => Vec::new(),
        }
    }
}

type S<T> = BoxedStrategy<T>;

fn b(s: S<Expr>) -> S<B> {
    s.prop_map(Box::new).boxed()
}

fn var(ty: Ty) -> S<Expr> {
    any::<u8>().prop_map(move |k| Expr::Var(ty, k)).boxed()
}

fn index(len: usize) -> S<u8> {
    (0..len as u8).boxed()
}

pub fn expr(ty: Ty, cx: Cx) -> S<Expr> {
    lazy(move || build(ty, cx))
}

fn lambda(params: Vec<Ty>, ret: Ty, cx: Cx) -> S<Box<Lambda>> {
    let inner = Cx { depth: cx.depth.saturating_sub(1), ..cx.function(Some(ret)) };
    let destructure = if params.first() == Some(&Ty::Obj) { any::<bool>().boxed() } else { Just(false).boxed() };
    (expr(ret, inner), destructure)
        .prop_map(move |(body, destructure)| {
            Box::new(Lambda { params: params.clone(), annotate: None, destructure, body: Body::Expr(Box::new(body)) })
        })
        .boxed()
}

/// A closure stored in a variable, with its types written out, and an expression or block body.
fn closure(params: Vec<Ty>, ret: Ty, cx: Cx) -> S<Expr> {
    let inner = cx.function(Some(ret));
    let expression = expr(ret, inner).prop_map(|body| Body::Expr(Box::new(body)));
    let block = (block(inner), expr(ret, inner)).prop_map(|(stmts, value)| Body::Block(stmts, Some(Box::new(value))));
    let body = if cx.nest > 0 { prop_oneof![expression, block].boxed() } else { expression.boxed() };
    body.prop_map(move |body| {
        Expr::Lambda(Box::new(Lambda { params: params.clone(), annotate: Some(ret), destructure: false, body }))
    })
    .boxed()
}

fn leaves(ty: Ty, arms: &mut Vec<(u32, S<Expr>)>) {
    match ty {
        Ty::Int => arms.push((4, index(INTS.len()).prop_map(Expr::Int).boxed())),
        Ty::Num => {
            arms.push((3, index(NUMS.len()).prop_map(Expr::Num).boxed()));
            arms.push((1, index(INTS.len()).prop_map(Expr::Int).boxed()));
            arms.push((1, var(Ty::Int)));
        }
        Ty::Str => arms.push((4, (index(STRS.len()), 0..3u8).prop_map(|(i, q)| Expr::Str(i, q)).boxed())),
        Ty::Bool => arms.push((2, any::<bool>().prop_map(Expr::Bool).boxed())),
        _ => {}
    }
    arms.push((3, var(ty)));
}

/// The arguments of `push` or `unshift`. The compiler inserts each value before it evaluates the next, and `unshift`'s
/// from the last, so only the argument evaluated first is any expression, and the rest are literals or variables:
/// known.md, "push and unshift insert a value before they evaluate the next".
fn inserted(element: Ty, free: S<Expr>, rest: std::ops::Range<usize>, unshift: bool) -> S<Vec<Expr>> {
    let mut arms = Vec::new();
    leaves(element, &mut arms);
    (free, vec(Union::new_weighted(arms), rest))
        .prop_map(move |(free, mut values)| {
            if unshift {
                values.push(free);
            } else {
                values.insert(0, free);
            }
            values
        })
        .boxed()
}

fn build(ty: Ty, cx: Cx) -> S<Expr> {
    let mut arms: Vec<(u32, S<Expr>)> = Vec::new();
    leaves(ty, &mut arms);
    if cx.depth > 0 {
        composite(ty, cx, &mut arms);
    }
    Union::new_weighted(arms).boxed()
}

fn composite(ty: Ty, cx: Cx, arms: &mut Vec<(u32, S<Expr>)>) {
    let d = cx.down();
    let e = |ty: Ty| expr(ty, d);
    let be = |ty: Ty| b(expr(ty, d));
    let mut push = |w: u32, s: S<Expr>| arms.push((w, s));
    let insts = cx.inst_types();
    let cond = |ty: Ty| (be(Ty::Bool), be(ty), be(ty)).prop_map(|(c, a, b)| Expr::Cond(c, a, b)).boxed();
    match ty {
        Ty::Int => {
            push(3, (be(Ty::Int), be(Ty::Int)).prop_map(|(a, b)| Expr::Add(a, b)).boxed());
            push(2, (be(Ty::Int), be(Ty::Int)).prop_map(|(a, b)| Expr::Sub(a, b)).boxed());
            push(2, (be(Ty::Int), 2..6u8).prop_map(|(a, k)| Expr::MulK(a, k)).boxed());
            // `%` takes `abs` of its left operand and a positive right one: known.md, "% is floored".
            push(2, (be(Ty::Int), 1..10u8).prop_map(|(a, k)| Expr::ModK(a, k)).boxed());
            // The printer takes each operand `abs(a) % 256`, shifts by at most 8 and never writes `~`: known.md,
            // "Bitwise results are unsigned".
            push(
                1,
                (select(vec![Bit::And, Bit::Or, Bit::Xor]), be(Ty::Int), be(Ty::Int))
                    .prop_map(|(op, a, b)| Expr::Bit(op, a, b))
                    .boxed(),
            );
            push(
                1,
                (select(vec![Shift::Left, Shift::Right, Shift::Unsigned]), be(Ty::Int), 0..9u8)
                    .prop_map(|(op, a, k)| Expr::Shift(op, a, k))
                    .boxed(),
            );
            push(1, be(Ty::Int).prop_map(Expr::Plus).boxed());
            push(1, (be(Ty::Int), any::<bool>()).prop_map(|(a, s)| Expr::Cast(a, s)).boxed());
            for sized in [Ty::Str, Ty::ArrInt, Ty::ArrStr, Ty::ArrObj, Ty::ArrBool, Ty::MapSI, Ty::SetS, Ty::SetI] {
                push(1, be(sized).prop_map(Expr::Size).boxed());
            }
            push(
                2,
                // A negative `fromIndex` is left out: known.md, "A negative fromIndex throws".
                (be(Ty::ArrInt), be(Ty::Int), option::of(0..4i8))
                    .prop_map(|(a, v, from)| Expr::IndexOf(a, v, from))
                    .boxed(),
            );
            push(
                1,
                (be(Ty::ArrInt), lambda(vec![Ty::Int, Ty::Int], Ty::Bool, d))
                    .prop_map(|(a, f)| Expr::FindIndex(a, f))
                    .boxed(),
            );
            let ints = prop_oneof![1 => Just(Vec::new()), 2 => inserted(Ty::Int, e(Ty::Int), 0..2, false)];
            push(2, (any::<u8>(), ints).prop_map(|(k, v)| Expr::Push(Ty::ArrInt, k, v)).boxed());
            let strs = inserted(Ty::Str, e(Ty::Str), 0..2, false);
            push(1, (any::<u8>(), strs).prop_map(|(k, v)| Expr::Push(Ty::ArrStr, k, v)).boxed());
            push(
                2,
                (be(Ty::ArrInt), lambda(vec![Ty::Int, Ty::Int, Ty::Int], Ty::Int, d), be(Ty::Int))
                    .prop_map(|(a, f, init)| Expr::Reduce(a, f, Some(init)))
                    .boxed(),
            );
            push(
                1,
                (vec(e(Ty::Int), 1..4), lambda(vec![Ty::Int, Ty::Int, Ty::Int], Ty::Int, d))
                    .prop_map(|(items, f)| Expr::ReduceLit(items, f))
                    .boxed(),
            );
            push(2, (any::<u8>(), 0..4u8, be(Ty::Int)).prop_map(|(k, i, d)| Expr::Index(Ty::ArrInt, k, i, d)).boxed());
            push(
                1,
                (any::<u8>(), any::<u8>(), be(Ty::Int))
                    .prop_map(|(k, j, d)| Expr::IndexAt(Ty::ArrInt, k, j, d))
                    .boxed(),
            );
            push(1, be(Ty::Obj).prop_map(|o| Expr::Bracket(Field::A, o)).boxed());
            push(1, be(Ty::Rec).prop_map(|o| Expr::Bracket(Field::End, o)).boxed());
            push(1, be(Ty::Rec).prop_map(|o| Expr::Bracket(Field::InnerA, o)).boxed());
            push(3, cond(Ty::Int));
            push(2, (be(Ty::OptInt), be(Ty::Int)).prop_map(|(a, b)| Expr::Coalesce(a, b)).boxed());
            push(1, (be(Ty::Int), be(Ty::Int)).prop_map(|(a, b)| Expr::Or(Ty::Int, a, b)).boxed());
            push(1, (be(Ty::Int), be(Ty::Int)).prop_map(|(a, b)| Expr::And(Ty::Int, a, b)).boxed());
            push(2, be(Ty::Obj).prop_map(|o| Expr::Field(Field::A, o)).boxed());
            push(1, be(Ty::Rec).prop_map(|o| Expr::Field(Field::End, o)).boxed());
            push(1, be(Ty::Rec).prop_map(|o| Expr::Field(Field::InnerA, o)).boxed());
            push(1, (be(Ty::OptObj), be(Ty::Int)).prop_map(|(o, d)| Expr::OptField(Field::A, o, d)).boxed());
            push(1, (be(Ty::Rec), be(Ty::Int)).prop_map(|(o, d)| Expr::OptSize(o, d)).boxed());
            push(1, be(Ty::Obj).prop_map(|o| Expr::NonNull(Field::A, o)).boxed());
            for &k in &insts {
                push(1, be(Ty::Inst(k)).prop_map(Expr::InstX).boxed());
                push(1, be(Ty::Inst(k)).prop_map(move |i| Expr::InstY(k, i)).boxed());
                if cx.classes {
                    push(1, (be(Ty::Inst(k)), be(Ty::Int)).prop_map(|(i, n)| Expr::InstM(i, n)).boxed());
                    push(1, Just(Expr::Count(k)).boxed());
                }
            }
            if cx.sup {
                push(2, be(Ty::Int).prop_map(Expr::SuperM).boxed());
            }
            if cx.calls > 0 {
                push(2, e(Ty::Int).prop_map(|a| Expr::Call(0, vec![a], None)).boxed());
            }
            if cx.calls > 2 {
                push(
                    1,
                    (e(Ty::ArrInt), option::of(e(Ty::Int)))
                        .prop_map(|(xs, k)| Expr::Call(2, std::iter::once(xs).chain(k).collect(), None))
                        .boxed(),
                );
            }
            if cx.calls > 3 {
                push(
                    1,
                    (vec(e(Ty::Int), 1..4), option::of(be(Ty::ArrInt)))
                        .prop_map(|(args, spread)| Expr::Call(3, args, spread))
                        .boxed(),
                );
            }
            if cx.calls > 4 {
                push(1, e(Ty::Int).prop_map(|a| Expr::Call(4, vec![a], None)).boxed());
            }
            push(2, (any::<u8>(), e(Ty::Int)).prop_map(|(k, a)| Expr::CallVar(Ty::FnII, k, vec![a])).boxed());
            push(
                1,
                (select(vec![Step::PreInc, Step::PostInc, Step::PreDec, Step::PostDec]), any::<u8>())
                    .prop_map(|(op, k)| Expr::Inc(op, k))
                    .boxed(),
            );
            push(1, (any::<u8>(), be(Ty::Int)).prop_map(|(k, a)| Expr::Assign(Ty::Int, k, a)).boxed());
            push(1, (any::<u8>(), be(Ty::Int)).prop_map(|(k, a)| Expr::AddAssign(Ty::Int, k, a)).boxed());
        }
        Ty::Num => {
            push(4, e(Ty::Int));
            push(2, (be(Ty::Num), be(Ty::Num)).prop_map(|(a, b)| Expr::Add(a, b)).boxed());
            push(2, (be(Ty::Num), be(Ty::Num)).prop_map(|(a, b)| Expr::Sub(a, b)).boxed());
            push(2, (be(Ty::Num), be(Ty::Num)).prop_map(|(a, b)| Expr::Mul(a, b)).boxed());
            push(2, (be(Ty::Num), be(Ty::Num)).prop_map(|(a, b)| Expr::Div(a, b)).boxed());
            push(1, be(Ty::Num).prop_map(Expr::Neg).boxed());
            push(1, (be(Ty::Num), 0..6u8).prop_map(|(a, k)| Expr::Pow(a, k)).boxed());
            push(1, cond(Ty::Num));
            push(1, (any::<u8>(), 0..4u8, be(Ty::Num)).prop_map(|(k, i, d)| Expr::Index(Ty::ArrNum, k, i, d)).boxed());
            push(1, (be(Ty::Num), be(Ty::Num)).prop_map(|(a, b)| Expr::Or(Ty::Num, a, b)).boxed());
        }
        Ty::Str => {
            push(3, (be(Ty::Str), be(Ty::Str)).prop_map(|(a, b)| Expr::Concat(a, b)).boxed());
            // Only integers become strings: known.md, "A number becomes a string through tostring".
            push(2, (be(Ty::Str), be(Ty::Int)).prop_map(|(a, b)| Expr::Concat(a, b)).boxed());
            push(1, (be(Ty::Int), be(Ty::Str)).prop_map(|(a, b)| Expr::Concat(a, b)).boxed());
            push(1, (be(Ty::Str), be(Ty::Bool)).prop_map(|(a, b)| Expr::Concat(a, b)).boxed());
            let part = prop_oneof![e(Ty::Str), e(Ty::Int), e(Ty::Bool)];
            push(
                3,
                (index(STRS.len()), vec((part, index(STRS.len())), 0..3))
                    .prop_map(|(head, parts)| Expr::Template(head, parts))
                    .boxed(),
            );
            push(1, be(Ty::Str).prop_map(Expr::Upper).boxed());
            push(1, be(Ty::Str).prop_map(Expr::Lower).boxed());
            push(1, (be(Ty::Str), 0..4u8).prop_map(|(a, k)| Expr::Rep(a, k)).boxed());
            for joined in [Ty::ArrStr, Ty::ArrInt, Ty::ArrBool] {
                // `join()` with no separator is left out: known.md, "join() separates with a comma and a space".
                push(1, (be(joined), index(JOINS.len())).prop_map(|(a, s)| Expr::Join(a, Some(s))).boxed());
            }
            push(3, cond(Ty::Str));
            push(2, (be(Ty::OptStr), be(Ty::Str)).prop_map(|(a, b)| Expr::Coalesce(a, b)).boxed());
            push(1, (be(Ty::Str), be(Ty::Str)).prop_map(|(a, b)| Expr::Or(Ty::Str, a, b)).boxed());
            push(1, (be(Ty::Str), be(Ty::Str)).prop_map(|(a, b)| Expr::And(Ty::Str, a, b)).boxed());
            push(2, be(Ty::Obj).prop_map(|o| Expr::Field(Field::S, o)).boxed());
            push(1, be(Ty::Rec).prop_map(|o| Expr::Field(Field::InnerS, o)).boxed());
            push(1, (be(Ty::OptObj), be(Ty::Str)).prop_map(|(o, d)| Expr::OptField(Field::S, o, d)).boxed());
            push(1, (any::<u8>(), 0..4u8, be(Ty::Str)).prop_map(|(k, i, d)| Expr::Index(Ty::ArrStr, k, i, d)).boxed());
            push(
                1,
                (any::<u8>(), any::<u8>(), be(Ty::Str))
                    .prop_map(|(k, j, d)| Expr::IndexAt(Ty::ArrStr, k, j, d))
                    .boxed(),
            );
            push(1, be(Ty::Obj).prop_map(|o| Expr::Bracket(Field::S, o)).boxed());
            for &k in &insts {
                push(1, be(Ty::Inst(k)).prop_map(Expr::InstTag).boxed());
                push(1, be(Ty::Inst(k)).prop_map(Expr::InstDescribe).boxed());
            }
            if cx.calls > 1 {
                push(2, (e(Ty::Str), e(Ty::Int)).prop_map(|(s, n)| Expr::Call(1, vec![s, n], None)).boxed());
            }
            push(
                1,
                (any::<u8>(), e(Ty::Str), e(Ty::Int))
                    .prop_map(|(k, s, n)| Expr::CallVar(Ty::FnSIS, k, vec![s, n]))
                    .boxed(),
            );
            push(1, (any::<u8>(), be(Ty::Str)).prop_map(|(k, a)| Expr::Assign(Ty::Str, k, a)).boxed());
            push(1, (any::<u8>(), be(Ty::Str)).prop_map(|(k, a)| Expr::AddAssign(Ty::Str, k, a)).boxed());
        }
        Ty::Bool => {
            let cmp = select(vec![Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge, Cmp::Eq, Cmp::Ne]);
            for operand in [Ty::Int, Ty::Num, Ty::Str] {
                push(
                    2,
                    (cmp.clone(), be(operand), be(operand))
                        .prop_map(move |(op, a, b)| Expr::Cmp(op, operand, a, b))
                        .boxed(),
                );
            }
            push(
                1,
                (select(vec![Cmp::Eq, Cmp::Ne]), be(Ty::Bool), be(Ty::Bool))
                    .prop_map(|(op, a, b)| Expr::Cmp(op, Ty::Bool, a, b))
                    .boxed(),
            );
            for negated in [Ty::Bool, Ty::Int, Ty::Num, Ty::Str] {
                push(
                    if negated == Ty::Bool { 2 } else { 1 },
                    be(negated).prop_map(move |a| Expr::Not(negated, a)).boxed(),
                );
            }
            push(2, (be(Ty::Bool), be(Ty::Bool)).prop_map(|(a, b)| Expr::And(Ty::Bool, a, b)).boxed());
            push(2, (be(Ty::Bool), be(Ty::Bool)).prop_map(|(a, b)| Expr::Or(Ty::Bool, a, b)).boxed());
            push(1, cond(Ty::Bool));
            push(
                1,
                // A negative `fromIndex` is left out: known.md, "A negative fromIndex throws".
                (be(Ty::ArrInt), be(Ty::Int), option::of(0..4i8))
                    .prop_map(|(a, v, from)| Expr::Includes(a, v, from))
                    .boxed(),
            );
            push(
                1,
                (be(Ty::ArrStr), be(Ty::Str), option::of(0..4i8))
                    .prop_map(|(a, v, from)| Expr::Includes(a, v, from))
                    .boxed(),
            );
            // `includes` over `number[]` is left out, because the array may hold NaN: known.md, "includes(NaN) is false".
            for empty in [Ty::ArrInt, Ty::ArrStr, Ty::MapSI, Ty::SetI] {
                push(1, be(empty).prop_map(Expr::IsEmpty).boxed());
            }
            push(
                1,
                (be(Ty::ArrInt), lambda(vec![Ty::Int, Ty::Int], Ty::Bool, d))
                    .prop_map(|(a, f)| Expr::Some(a, f))
                    .boxed(),
            );
            push(
                1,
                (be(Ty::ArrInt), lambda(vec![Ty::Int, Ty::Int], Ty::Bool, d))
                    .prop_map(|(a, f)| Expr::Every(a, f))
                    .boxed(),
            );
            push(1, (be(Ty::MapSI), be(Ty::Str)).prop_map(|(m, k)| Expr::Has(m, k)).boxed());
            push(1, (be(Ty::SetI), be(Ty::Int)).prop_map(|(m, k)| Expr::Has(m, k)).boxed());
            push(1, (be(Ty::SetS), be(Ty::Str)).prop_map(|(m, k)| Expr::Has(m, k)).boxed());
            push(1, (any::<u8>(), be(Ty::Str)).prop_map(|(k, key)| Expr::Delete(Ty::MapSI, k, key)).boxed());
            push(1, (any::<u8>(), be(Ty::Int)).prop_map(|(k, key)| Expr::Delete(Ty::SetI, k, key)).boxed());
            if cx.classes {
                for k in 0..3u8 {
                    push(1, (be(Ty::Inst(k)), 0..3u8).prop_map(|(i, c)| Expr::InstanceOf(i, c)).boxed());
                }
            }
            push(
                1,
                (select(vec![Field::Then, Field::End, Field::Inner]), be(Ty::Rec))
                    .prop_map(|(f, r)| Expr::In(f, r))
                    .boxed(),
            );
            push(1, (var(Ty::Obj), var(Ty::Obj)).prop_map(|(a, b)| Expr::Same(Box::new(a), Box::new(b))).boxed());
            push(1, be(Ty::Num).prop_map(Expr::NaN).boxed());
            push(1, (any::<u8>(), be(Ty::Bool)).prop_map(|(k, a)| Expr::Assign(Ty::Bool, k, a)).boxed());
        }
        Ty::OptInt | Ty::OptStr | Ty::OptObj => {
            let inner = ty.required();
            push(3, (be(inner), any::<bool>()).prop_map(|(v, keep)| Expr::Maybe(v, keep)).boxed());
            let array = match inner {
                Ty::Int => Ty::ArrInt,
                Ty::Str => Ty::ArrStr,
                _ => Ty::ArrObj,
            };
            push(1, any::<u8>().prop_map(move |k| Expr::Pop(array, k)).boxed());
            push(1, any::<u8>().prop_map(move |k| Expr::ShiftArr(array, k)).boxed());
            push(1, (be(array), lambda(vec![inner, Ty::Int], Ty::Bool, d)).prop_map(|(a, f)| Expr::Find(a, f)).boxed());
            push(1, cond(ty));
            match inner {
                Ty::Int => {
                    push(1, (be(Ty::MapSI), be(Ty::Str)).prop_map(|(m, k)| Expr::MapGet(m, k)).boxed());
                    push(1, be(Ty::OptObj).prop_map(|o| Expr::Chain(Field::A, o)).boxed());
                }
                Ty::Str => {
                    push(1, (be(Ty::MapIS), be(Ty::Int)).prop_map(|(m, k)| Expr::MapGet(m, k)).boxed());
                    push(1, be(Ty::OptObj).prop_map(|o| Expr::Chain(Field::S, o)).boxed());
                    push(1, be(Ty::Rec).prop_map(|r| Expr::Field(Field::Then, r)).boxed());
                }
                _ => {}
            }
        }
        Ty::ArrInt | Ty::ArrNum | Ty::ArrStr | Ty::ArrBool | Ty::ArrObj => {
            let element = ty.element();
            let item = prop_oneof![
                4 => e(element).prop_map(Item::One),
                1 => e(ty).prop_map(Item::Spread),
            ];
            push(4, vec(item, 0..5).prop_map(move |items| Expr::Array(ty, items)).boxed());
            for from in [Ty::ArrInt, Ty::ArrStr, Ty::ArrObj] {
                if from == Ty::ArrObj && ty != Ty::ArrInt && ty != Ty::ArrStr {
                    continue;
                }
                push(
                    1,
                    (be(from), lambda(vec![from.element(), Ty::Int], element, d))
                        .prop_map(move |(a, mut f)| {
                            // A written return type keeps TypeScript from inferring a literal element type.
                            f.annotate = Some(element);
                            Expr::Map(a, f)
                        })
                        .boxed(),
                );
            }
            push(
                1,
                (be(ty), lambda(vec![element, Ty::Int], Ty::Bool, d)).prop_map(|(a, f)| Expr::Filter(a, f)).boxed(),
            );
            if ty == Ty::ArrStr {
                push(2, (be(Ty::Str), index(SPLITS.len())).prop_map(|(s, sep)| Expr::Split(s, sep)).boxed());
            }
            push(1, cond(ty));
        }
        Ty::Obj => {
            push(4, (be(Ty::Int), be(Ty::Str)).prop_map(|(a, s)| Expr::Obj(a, s)).boxed());
            push(1, (be(Ty::Obj), option::of(be(Ty::Int))).prop_map(|(o, a)| Expr::ObjSpread(o, a)).boxed());
            push(1, be(Ty::Rec).prop_map(|r| Expr::Field(Field::Inner, r)).boxed());
            push(1, cond(ty));
        }
        Ty::Rec => {
            push(
                4,
                (be(Ty::Int), option::of(be(Ty::Str)), be(Ty::Obj))
                    .prop_map(|(end, then, inner)| Expr::Rec(end, then, inner))
                    .boxed(),
            );
            push(1, (be(Ty::Rec), be(Ty::Int)).prop_map(|(r, end)| Expr::RecSpread(r, end)).boxed());
        }
        Ty::MapSI | Ty::MapIS => {
            let (key, value) = if ty == Ty::MapSI { (Ty::Str, Ty::Int) } else { (Ty::Int, Ty::Str) };
            push(3, vec((e(key), e(value)), 0..4).prop_map(move |entries| Expr::NewMap(ty, entries)).boxed());
        }
        Ty::SetS | Ty::SetI => {
            let element = if ty == Ty::SetS { Ty::Str } else { Ty::Int };
            let array = if ty == Ty::SetS { Ty::ArrStr } else { Ty::ArrInt };
            push(3, vec(e(element), 0..4).prop_map(move |items| Expr::NewSet(ty, items)).boxed());
            push(1, be(array).prop_map(move |a| Expr::NewSetFrom(ty, a)).boxed());
        }
        Ty::FnII => push(3, closure(vec![Ty::Int], Ty::Int, cx)),
        Ty::FnSIS => push(3, closure(vec![Ty::Str, Ty::Int], Ty::Str, cx)),
        Ty::Inst(k) => {
            if cx.own == Some(k) {
                push(4, Just(Expr::This).boxed());
            }
            if cx.classes {
                push(3, be(Ty::Int).prop_map(move |n| Expr::New(k, n)).boxed());
                if k == 0 {
                    push(1, be(Ty::Int).prop_map(Expr::Make).boxed());
                }
            }
        }
    }
}

/// The types a variable or trace may have where `cx` is.
fn types(cx: Cx) -> Vec<Ty> {
    let mut types = vec![
        Ty::Int,
        Ty::Num,
        Ty::Str,
        Ty::Bool,
        Ty::OptInt,
        Ty::OptStr,
        Ty::ArrInt,
        Ty::ArrNum,
        Ty::ArrStr,
        Ty::ArrBool,
        Ty::ArrObj,
        Ty::Obj,
        Ty::Rec,
        Ty::OptObj,
        Ty::MapSI,
        Ty::MapIS,
        Ty::SetS,
        Ty::SetI,
    ];
    types.extend(cx.inst_types().into_iter().map(Ty::Inst));
    types
}

fn typed(cx: Cx) -> S<(Ty, Expr)> {
    select(types(cx)).prop_flat_map(move |ty| expr(ty, cx).prop_map(move |e| (ty, e))).boxed()
}

pub fn block(cx: Cx) -> S<Vec<Stmt>> {
    vec(stmt(cx), 0..6).boxed()
}

pub fn stmt(cx: Cx) -> S<Stmt> {
    lazy(move || build_stmt(cx.statement()))
}

fn build_stmt(cx: Cx) -> S<Stmt> {
    let mut arms: Vec<(u32, S<Stmt>)> = Vec::new();
    let e = |ty: Ty| expr(ty, cx);
    let mut push = |w: u32, s: S<Stmt>| arms.push((w, s));
    push(6, typed(cx).prop_map(|(ty, e)| Stmt::Trace(ty, e)).boxed());
    let mut declared = types(cx);
    declared.extend([Ty::FnII, Ty::FnSIS]);
    push(
        6,
        (select(declared), any::<bool>(), option::weighted(0.15, any::<u8>()))
            .prop_flat_map(move |(ty, mutable, shadow)| {
                expr(ty, cx).prop_map(move |init| Stmt::Let { ty, mutable, shadow, init })
            })
            .boxed(),
    );
    for ty in [Ty::Int, Ty::Str, Ty::Bool, Ty::OptInt, Ty::Num, Ty::FnII] {
        push(1, (any::<u8>(), e(ty)).prop_map(move |(k, v)| Stmt::Assign(ty, k, v)).boxed());
    }
    push(2, (any::<u8>(), e(Ty::Int)).prop_map(|(k, v)| Stmt::AddAssign(Ty::Int, k, v)).boxed());
    push(1, (any::<u8>(), e(Ty::Str)).prop_map(|(k, v)| Stmt::AddAssign(Ty::Str, k, v)).boxed());
    push(1, (any::<u8>(), e(Ty::Int)).prop_map(|(k, v)| Stmt::SubAssign(k, v)).boxed());
    push(1, (any::<u8>(), 2..4u8).prop_map(|(k, m)| Stmt::MulAssign(k, m)).boxed());
    push(1, (any::<u8>(), e(Ty::Int)).prop_map(|(k, v)| Stmt::Logical(Logical::Coalesce, Ty::OptInt, k, v)).boxed());
    push(1, (any::<u8>(), e(Ty::Str)).prop_map(|(k, v)| Stmt::Logical(Logical::Coalesce, Ty::OptStr, k, v)).boxed());
    for ty in [Ty::Int, Ty::Str, Ty::Bool] {
        push(
            1,
            (select(vec![Logical::Or, Logical::And]), any::<u8>(), e(ty))
                .prop_map(move |(op, k, v)| Stmt::Logical(op, ty, k, v))
                .boxed(),
        );
    }
    push(
        2,
        (select(vec![Step::PreInc, Step::PostInc, Step::PreDec, Step::PostDec]), any::<u8>())
            .prop_map(|(op, k)| Stmt::Inc(op, k))
            .boxed(),
    );
    push(2, effect(cx).prop_map(Stmt::Expr).boxed());
    push(4, mutation(cx).prop_map(Stmt::Mutate).boxed());
    if cx.breakable {
        push(1, Just(Stmt::Break).boxed());
    }
    if cx.continuable {
        push(1, Just(Stmt::Continue).boxed());
    }
    push(1, prop_oneof![e(Ty::Str), e(Ty::Int)].prop_map(Stmt::Throw).boxed());
    match cx.ret {
        Some(Some(ty)) => push(1, e(ty).prop_map(|v| Stmt::Return(Some(v))).boxed()),
        Some(None) => push(1, Just(Stmt::Return(None)).boxed()),
        None => {}
    }
    push(
        1,
        (select(vec![Ty::ArrInt, Ty::ArrStr]), vec(option::weighted(0.8, any::<u8>()), 0..4), any::<bool>())
            .prop_flat_map(move |(ty, elements, rest)| {
                expr(ty, cx).prop_map(move |source| Stmt::DestructureArray {
                    ty,
                    source,
                    elements: elements.clone(),
                    rest,
                })
            })
            .boxed(),
    );
    push(
        1,
        (e(Ty::Obj), proptest::sample::subsequence(vec![Field::A, Field::S], 0..=2), any::<u8>())
            .prop_map(|(source, fields, default)| Stmt::DestructureObject { ty: Ty::Obj, source, fields, default })
            .boxed(),
    );
    push(
        1,
        (
            e(Ty::Rec),
            proptest::sample::subsequence(vec![Field::End, Field::Then, Field::InnerA, Field::InnerS], 0..=4),
            any::<u8>(),
        )
            .prop_map(|(source, fields, default)| Stmt::DestructureObject { ty: Ty::Rec, source, fields, default })
            .boxed(),
    );
    push(1, (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Stmt::Swap(a, b)).boxed());
    push(1, (any::<u8>(), any::<u8>(), e(Ty::Obj)).prop_map(|(a, b, o)| Stmt::AssignObject(a, b, o)).boxed());
    push(
        1,
        (select(vec![Ty::MapSI, Ty::MapIS, Ty::SetS, Ty::SetI]), any::<u8>(), any::<u8>())
            .prop_flat_map(|(ty, source, visit)| {
                let visit = match (ty, visit % 4) {
                    (Ty::SetS | Ty::SetI, 0 | 2) => Visit::Values,
                    (_, 0) => Visit::Entries,
                    (_, 1) => Visit::Values,
                    (_, 2) => Visit::Whole,
                    _ => Visit::ForEach,
                };
                term(ty).prop_map(move |term| Stmt::Aggregate { ty, source, visit, term })
            })
            .boxed(),
    );
    if cx.nest > 0 {
        let inner = cx.nested();
        let looped = cx.in_loop();
        push(
            3,
            (e(Ty::Bool), block(inner), option::weighted(0.5, block(inner)))
                .prop_map(|(c, then, otherwise)| Stmt::If(c, then, otherwise))
                .boxed(),
        );
        push(
            2,
            (1..7u8, 1..3u8, any::<bool>(), block(looped))
                .prop_map(|(count, step, down, body)| Stmt::For { count, step, down, body })
                .boxed(),
        );
        push(
            2,
            select(vec![Ty::ArrInt, Ty::ArrStr, Ty::ArrObj, Ty::ArrNum])
                .prop_flat_map(move |ty| {
                    (expr(ty, cx), block(looped)).prop_map(move |(source, body)| Stmt::ForOf { ty, source, body })
                })
                .boxed(),
        );
        push(1, (e(Ty::Str), block(looped)).prop_map(|(s, body)| Stmt::ForOfStr(s, body)).boxed());
        push(
            1,
            select(vec![Ty::ArrInt, Ty::ArrStr])
                .prop_flat_map(move |ty| {
                    (expr(ty, cx), block(cx.function(None))).prop_map(move |(source, body)| Stmt::ForEach {
                        ty,
                        source,
                        body,
                    })
                })
                .boxed(),
        );
        push(
            1,
            (1..7u8, e(Ty::Bool), block(looped))
                .prop_map(|(count, cond, body)| Stmt::While { count, cond, body })
                .boxed(),
        );
        push(1, (1..5u8, block(looped)).prop_map(|(count, body)| Stmt::DoWhile { count, body }).boxed());
        let switched = cx.in_switch();
        push(
            1,
            (e(Ty::Int), vec((0..6u8, block(switched), any::<bool>()), 1..4), option::of(block(switched)))
                .prop_map(|(disc, cases, default)| Stmt::Switch { disc, cases, default })
                .boxed(),
        );
        push(
            2,
            (block(inner), option::of((any::<bool>(), block(inner))), option::weighted(0.5, block(inner)))
                .prop_map(|(body, catch, finally)| Stmt::Try { body, catch, finally })
                .boxed(),
        );
        push(1, block(inner).prop_map(Stmt::Block).boxed());
        // A function declaration has its own `this`, so methods' `this` and `super` stay outside it.
        let function = Cx { own: None, sup: false, ..cx.function(Some(Ty::Int)) };
        push(
            1,
            (block(function), expr(Ty::Int, function)).prop_map(|(body, ret)| Stmt::LocalFunction(body, ret)).boxed(),
        );
    }
    Union::new_weighted(arms).boxed()
}

/// The expressions with an effect, which may stand as statements.
fn effect(cx: Cx) -> S<Expr> {
    let e = |ty: Ty| expr(ty, cx);
    let mut arms: Vec<(u32, S<Expr>)> = vec![
        (2, (any::<u8>(), e(Ty::Int)).prop_map(|(k, a)| Expr::CallVar(Ty::FnII, k, vec![a])).boxed()),
        (
            1,
            (any::<u8>(), e(Ty::Str), e(Ty::Int)).prop_map(|(k, s, n)| Expr::CallVar(Ty::FnSIS, k, vec![s, n])).boxed(),
        ),
        (
            2,
            (any::<u8>(), prop_oneof![1 => Just(Vec::new()), 2 => inserted(Ty::Int, e(Ty::Int), 0..2, false)])
                .prop_map(|(k, v)| Expr::Push(Ty::ArrInt, k, v))
                .boxed(),
        ),
        (1, (any::<u8>(), b(e(Ty::Int))).prop_map(|(k, a)| Expr::Assign(Ty::Int, k, a)).boxed()),
        (1, any::<u8>().prop_map(|k| Expr::Pop(Ty::ArrInt, k)).boxed()),
        (1, any::<u8>().prop_map(|k| Expr::ShiftArr(Ty::ArrStr, k)).boxed()),
    ];
    if cx.calls > 0 {
        arms.push((2, b(e(Ty::Int)).prop_map(|a| Expr::Call(0, vec![*a], None)).boxed()));
    }
    if cx.calls > 1 {
        arms.push((1, (e(Ty::Str), e(Ty::Int)).prop_map(|(s, n)| Expr::Call(1, vec![s, n], None)).boxed()));
    }
    if cx.calls > 4 {
        arms.push((1, e(Ty::Int).prop_map(|a| Expr::Call(4, vec![a], None)).boxed()));
    }
    if cx.classes {
        for k in 0..3u8 {
            arms.push((1, (b(e(Ty::Inst(k))), b(e(Ty::Int))).prop_map(|(i, n)| Expr::InstM(i, n)).boxed()));
        }
    }
    Union::new_weighted(arms).boxed()
}

fn mutation(cx: Cx) -> S<Mutation> {
    let e = |ty: Ty| expr(ty, cx);
    let arrays = [Ty::ArrInt, Ty::ArrStr, Ty::ArrObj, Ty::ArrBool];
    let mut arms: Vec<(u32, S<Mutation>)> = Vec::new();
    for ty in arrays {
        let element = ty.element();
        let pushed = inserted(element, e(element), 0..2, false);
        arms.push((3, (any::<u8>(), pushed).prop_map(move |(k, v)| Mutation::Push(ty, k, v)).boxed()));
        let unshifted = inserted(element, e(element), 0..2, true);
        arms.push((1, (any::<u8>(), unshifted).prop_map(move |(k, v)| Mutation::Unshift(ty, k, v)).boxed()));
        arms.push((1, any::<u8>().prop_map(move |k| Mutation::Pop(ty, k)).boxed()));
        arms.push((1, any::<u8>().prop_map(move |k| Mutation::Shift(ty, k)).boxed()));
        arms.push((
            1,
            (any::<u8>(), 0..4u8, e(element)).prop_map(move |(k, i, v)| Mutation::SetIndex(ty, k, i, v)).boxed(),
        ));
        arms.push((1, (any::<u8>(), e(element)).prop_map(move |(k, v)| Mutation::Append(ty, k, v)).boxed()));
    }
    for ty in [Ty::ArrInt, Ty::MapSI, Ty::SetS] {
        arms.push((1, any::<u8>().prop_map(move |k| Mutation::Clear(ty, k)).boxed()));
    }
    arms.push((
        3,
        (any::<u8>(), e(Ty::Str), e(Ty::Int)).prop_map(|(k, a, b)| Mutation::MapSet(Ty::MapSI, k, a, b)).boxed(),
    ));
    arms.push((
        2,
        (any::<u8>(), e(Ty::Int), e(Ty::Str)).prop_map(|(k, a, b)| Mutation::MapSet(Ty::MapIS, k, a, b)).boxed(),
    ));
    arms.push((1, (any::<u8>(), e(Ty::Str)).prop_map(|(k, a)| Mutation::Delete(Ty::MapSI, k, a)).boxed()));
    arms.push((1, (any::<u8>(), e(Ty::Int)).prop_map(|(k, a)| Mutation::Delete(Ty::MapIS, k, a)).boxed()));
    arms.push((2, (any::<u8>(), e(Ty::Str)).prop_map(|(k, a)| Mutation::SetAdd(Ty::SetS, k, a)).boxed()));
    arms.push((2, (any::<u8>(), e(Ty::Int)).prop_map(|(k, a)| Mutation::SetAdd(Ty::SetI, k, a)).boxed()));
    arms.push((1, (any::<u8>(), e(Ty::Str)).prop_map(|(k, a)| Mutation::Delete(Ty::SetS, k, a)).boxed()));
    arms.push((1, (any::<u8>(), e(Ty::Int)).prop_map(|(k, a)| Mutation::Delete(Ty::SetI, k, a)).boxed()));
    arms.push((
        2,
        (any::<u8>(), e(Ty::Int), any::<bool>())
            .prop_map(|(k, v, c)| Mutation::Field(Ty::Obj, Field::A, k, v, c))
            .boxed(),
    ));
    arms.push((
        1,
        (any::<u8>(), e(Ty::Str), any::<bool>())
            .prop_map(|(k, v, c)| Mutation::Field(Ty::Obj, Field::S, k, v, c))
            .boxed(),
    ));
    arms.push((
        1,
        (any::<u8>(), e(Ty::Int), any::<bool>())
            .prop_map(|(k, v, c)| Mutation::Field(Ty::Rec, Field::End, k, v, c))
            .boxed(),
    ));
    arms.push((
        1,
        (any::<u8>(), e(Ty::Int), any::<bool>())
            .prop_map(|(k, v, c)| Mutation::Field(Ty::Rec, Field::InnerA, k, v, c))
            .boxed(),
    ));
    arms.push((
        1,
        (any::<u8>(), e(Ty::Str)).prop_map(|(k, v)| Mutation::Field(Ty::Rec, Field::Then, k, v, false)).boxed(),
    ));
    arms.push((
        1,
        (any::<u8>(), e(Ty::Obj)).prop_map(|(k, v)| Mutation::Field(Ty::Rec, Field::Inner, k, v, false)).boxed(),
    ));
    arms.push((1, any::<u8>().prop_map(Mutation::DeleteThen).boxed()));
    let inc = || select(vec![Step::PreInc, Step::PostInc, Step::PreDec, Step::PostDec]);
    arms.push((1, (any::<u8>(), inc()).prop_map(|(k, op)| Mutation::FieldInc(Ty::Obj, Field::A, k, op)).boxed()));
    arms.push((1, (any::<u8>(), inc()).prop_map(|(k, op)| Mutation::FieldInc(Ty::Rec, Field::InnerA, k, op)).boxed()));
    arms.push((1, (any::<u8>(), 0..3u8, e(Ty::Int)).prop_map(|(k, i, v)| Mutation::IndexAdd(k, i, v)).boxed()));
    Union::new_weighted(arms).boxed()
}

/// An order-free term of an aggregate: integer arithmetic over the entry's key and value only.
fn term(ty: Ty) -> S<Expr> {
    let leaf = prop_oneof![
        index(INTS.len()).prop_map(Expr::Int),
        any::<u8>().prop_map(|k| Expr::Var(Ty::Int, k)),
        any::<u8>().prop_map(|k| Expr::Size(Box::new(Expr::Var(Ty::Str, k)))),
    ];
    let _ = ty;
    leaf.prop_recursive(2, 8, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::Add(Box::new(a), Box::new(b))),
            (inner.clone(), 2..6u8).prop_map(|(a, k)| Expr::MulK(Box::new(a), k)),
            (inner, 1..10u8).prop_map(|(a, k)| Expr::ModK(Box::new(a), k)),
        ]
    })
    .boxed()
}

fn class(k: u8) -> S<Class> {
    let parent: S<Option<u8>> = match k {
        0 => Just(None).boxed(),
        1 => option::of(Just(0u8)).boxed(),
        _ => option::of(0..k).boxed(),
    };
    parent
        .prop_flat_map(move |parent| {
            let cx = Cx {
                own: Some(k),
                sup: parent.is_some(),
                calls: 0,
                classes: false,
                ..Cx::top().function(Some(Ty::Int))
            };
            let method = (block(cx), expr(Ty::Int, cx));
            let method = if parent.is_some() { option::of(method).boxed() } else { method.prop_map(Some).boxed() };
            (method, any::<bool>(), option::of(0..5u8), any::<bool>()).prop_map(
                move |(method, count, constructor, describe)| Class { parent, method, count, constructor, describe },
            )
        })
        .boxed()
}

fn function(k: u8) -> S<Function> {
    let (_, ret) = SIGNATURES[k as usize];
    let cx = Cx { calls: k, ..Cx::top().function(Some(ret)) };
    (block(cx), expr(ret, cx)).prop_map(|(body, ret)| Function { body, ret }).boxed()
}

pub fn program() -> S<Program> {
    // Proptest shrinks a tuple's members in order, and the body is where a failure usually is.
    (
        vec(stmt(Cx::top()), 1..24),
        (function(0), function(1), function(2), function(3), function(4)),
        (class(0), class(1), class(2)),
        any::<bool>(),
    )
        .prop_map(|(body, (f0, f1, f2, f3, f4), (c0, c1, c2), functions_last)| Program {
            classes: vec![c0, c1, c2],
            functions: vec![f0, f1, f2, f3, f4],
            body,
            functions_last,
        })
        .boxed()
}
