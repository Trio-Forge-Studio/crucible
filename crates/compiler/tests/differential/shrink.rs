//! Shrinks a failing program. Each round prints every candidate one edit away, runs them all in one batch, and keeps
//! the edits that still fail the same way: together when the program with all of them still fails, else the one that
//! removes the most text. A compile costs about as much for a batch as for one program, so a round costs about one
//! compile, where proptest's own shrinking compiles once per step. The edits delete a statement, replace a statement
//! with a block of its body, and replace an expression with one of its operands. Every variable stays a reference by
//! type, so a deleted declaration leaves a program that still type-checks; an edit that does not is rejected and
//! dropped.
//!
//! A shrunk program must show the same bug as the original, not another difference an edit brought in. So an operand
//! replaces an expression only when it has the expression's own type and keeps what the generator knows of an Int's
//! sign, and a candidate is kept only when it fails exactly as the original did: for traces that differ, with the same
//! pair of lines where they first part.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

use super::ast::*;
use super::print;
use super::run::Outcome;

#[derive(Clone, Copy)]
enum Pass {
    Delete,
    Flatten,
    Replace,
}

/// What a candidate changes, by the numbers a walk of the original program gives its statements and expressions.
#[derive(Clone, Default)]
struct Edits {
    statements: BTreeSet<usize>,
    flatten: BTreeSet<usize>,
    replace: BTreeMap<usize, usize>,
}

/// Runs candidates, given as programs and as their printed sources, and returns their outcomes in order. The test runs
/// the sources; the shrinker's own test also prints an edited program for the compiler.
pub type Run<'r> = dyn FnMut(&[Program], &[String]) -> Vec<Outcome> + 'r;

pub fn shrink(
    program: Program,
    outcome: Outcome,
    batch: usize,
    limit: Duration,
    run: &mut Run,
) -> (String, Outcome, usize) {
    let started = Instant::now();
    let failure = outcome.failure();
    let same = |o: &Outcome| o.failure() == failure;
    let mut best = (program.clone(), print::print(&program), outcome.clone());
    let mut seen: HashSet<String> = HashSet::from([best.1.clone()]);
    let mut rounds = 0;
    'passes: loop {
        let mut progress = false;
        for pass in [Pass::Delete, Pass::Flatten, Pass::Replace] {
            loop {
                if started.elapsed() > limit {
                    break 'passes;
                }
                let candidates: Vec<(Edits, Program, String)> = edits(&best.0, pass)
                    .into_iter()
                    .filter_map(|e| {
                        let p = apply(&best.0, &e);
                        let source = print::print(&p);
                        seen.insert(source.clone()).then_some((e, p, source))
                    })
                    .take(batch)
                    .collect();
                if candidates.is_empty() {
                    break;
                }
                rounds += 1;
                let programs: Vec<Program> = candidates.iter().map(|(_, p, _)| p.clone()).collect();
                let sources: Vec<String> = candidates.iter().map(|(_, _, s)| s.clone()).collect();
                let outcomes = run(&programs, &sources);
                let mut kept: Vec<(Edits, Program, String, Outcome)> = candidates
                    .into_iter()
                    .zip(outcomes)
                    .filter(|(_, o)| same(o))
                    .map(|((e, p, s), o)| (e, p, s, o))
                    .collect();
                if kept.is_empty() {
                    break;
                }
                progress = true;
                kept.sort_by_key(|(_, _, s, _)| s.len());
                if kept.len() > 1 {
                    let mut all = Edits::default();
                    for (e, ..) in &kept {
                        all.statements.extend(&e.statements);
                        all.flatten.extend(&e.flatten);
                        all.replace.extend(&e.replace);
                    }
                    let p = apply(&best.0, &all);
                    let source = print::print(&p);
                    if seen.insert(source.clone()) {
                        let o = run(std::slice::from_ref(&p), std::slice::from_ref(&source)).remove(0);
                        if same(&o) && source.len() < kept[0].2.len() {
                            best = (p, source, o);
                            continue;
                        }
                    }
                }
                let (_, p, s, o) = kept.swap_remove(0);
                best = (p, s, o);
            }
        }
        if !progress {
            break;
        }
    }
    (best.1, best.2, rounds)
}

fn edits(p: &Program, pass: Pass) -> Vec<Edits> {
    let mut counter = Counter::default();
    let mut p = p.clone();
    counter.program(&mut p);
    match pass {
        // Outer statements first, so a round removes the most.
        Pass::Delete => {
            counter.statements.iter().map(|&n| Edits { statements: BTreeSet::from([n]), ..Edits::default() }).collect()
        }
        Pass::Flatten => {
            counter.nested.iter().map(|&n| Edits { flatten: BTreeSet::from([n]), ..Edits::default() }).collect()
        }
        Pass::Replace => counter
            .operands
            .iter()
            .map(|&(n, j)| Edits { replace: BTreeMap::from([(n, j)]), ..Edits::default() })
            .collect(),
    }
}

fn apply(p: &Program, edits: &Edits) -> Program {
    let mut p = p.clone();
    let mut editor = Editor { edits, statement: 0, expression: 0 };
    editor.program(&mut p);
    let mut sweep = Sweep;
    sweep.program(&mut p);
    p
}

/// Walks a program: each statement and expression gets the next number before its parts are walked, and `post` sees
/// it after, so an edit of a part never moves a number.
trait Walk {
    fn post_statement(&mut self, _n: usize, _s: &mut Stmt) {}
    fn post_expression(&mut self, _n: usize, _e: &mut Expr) {}
    fn next_statement(&mut self) -> usize;
    fn next_expression(&mut self) -> usize;

    fn program(&mut self, p: &mut Program) {
        self.list(&mut p.body);
        for f in &mut p.functions {
            self.list(&mut f.body);
            self.expr(&mut f.ret);
        }
        for c in &mut p.classes {
            if let Some((body, ret)) = &mut c.method {
                self.list(body);
                self.expr(ret);
            }
        }
    }

    fn list(&mut self, list: &mut Vec<Stmt>) {
        for s in list.iter_mut() {
            let n = self.next_statement();
            self.statement(s);
            self.post_statement(n, s);
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        let n = self.next_expression();
        self.parts(e);
        self.post_expression(n, e);
    }

    fn lambda(&mut self, f: &mut Lambda) {
        match &mut f.body {
            Body::Expr(e) => self.expr(e),
            Body::Block(list, ret) => {
                self.list(list);
                if let Some(ret) = ret {
                    self.expr(ret);
                }
            }
        }
    }

    fn statement(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { init: e, .. }
            | Stmt::Assign(_, _, e)
            | Stmt::AddAssign(_, _, e)
            | Stmt::SubAssign(_, e)
            | Stmt::NumAssign(_, _, e)
            | Stmt::Logical(_, _, _, e)
            | Stmt::Trace(_, e)
            | Stmt::Expr(e)
            | Stmt::Throw(e)
            | Stmt::Return(Some(e))
            | Stmt::DestructureArray { source: e, .. }
            | Stmt::DestructureObject { source: e, .. }
            | Stmt::AssignObject(_, _, e)
            | Stmt::Aggregate { term: e, .. } => self.expr(e),
            Stmt::Compound(_, a, b) | Stmt::AddBool(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Stmt::MulAssign(..)
            | Stmt::BitAssign(..)
            | Stmt::Inc(..)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Return(None)
            | Stmt::Swap(..) => {}
            Stmt::Mutate(m) => self.mutation(m),
            Stmt::If(c, then, otherwise) => {
                self.expr(c);
                self.list(then);
                if let Some(otherwise) = otherwise {
                    self.list(otherwise);
                }
            }
            Stmt::For { body, .. } | Stmt::ForIndex { body, .. } | Stmt::Block(body) => self.list(body),
            Stmt::DoWhile { cond, body, .. } => {
                self.list(body);
                if let Some(cond) = cond {
                    self.expr(cond);
                }
            }
            Stmt::ForOf { source, body, .. }
            | Stmt::ForOfStr(source, body)
            | Stmt::ForEach { source, body, .. }
            | Stmt::While { cond: source, body, .. } => {
                self.expr(source);
                self.list(body);
            }
            Stmt::Switch { disc, cases, default } => {
                self.expr(disc);
                for (_, label, body, _) in cases {
                    if let Some(label) = label {
                        self.expr(label);
                    }
                    self.list(body);
                }
                if let Some(default) = default {
                    self.list(default);
                }
            }
            Stmt::Try { body, catch, finally } => {
                self.list(body);
                if let Some((_, body)) = catch {
                    self.list(body);
                }
                if let Some(body) = finally {
                    self.list(body);
                }
            }
            Stmt::LocalFunction(body, ret) => {
                self.list(body);
                self.expr(ret);
            }
        }
    }

    fn mutation(&mut self, m: &mut Mutation) {
        match m {
            Mutation::Push(_, _, values) | Mutation::Unshift(_, _, values) => {
                for v in values {
                    self.expr(v);
                }
            }
            Mutation::SetIndex(_, _, _, e)
            | Mutation::Append(_, _, e)
            | Mutation::Delete(_, _, e)
            | Mutation::SetAdd(_, _, e)
            | Mutation::Field(_, _, _, e, _)
            | Mutation::IndexAdd(_, _, e)
            | Mutation::Remove(_, _, e, _)
            | Mutation::ThenCoalesce(_, e) => self.expr(e),
            Mutation::MapSet(_, _, a, b) | Mutation::WriteAt(_, _, a, _, b) | Mutation::Insert(_, _, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Mutation::Pop(..)
            | Mutation::Shift(..)
            | Mutation::Clear(..)
            | Mutation::DeleteThen(_)
            | Mutation::FieldInc(..)
            | Mutation::FieldBit(..) => {}
        }
    }

    fn parts(&mut self, e: &mut Expr) {
        use Expr as E;
        match e {
            E::Int(_) | E::Num(_) | E::Str(..) | E::Bool(_) | E::Var(..) | E::This | E::Count(_) => {}
            E::Inc(..) | E::FieldStep(..) | E::Pop(..) | E::ShiftArr(..) => {}
            E::MulK(a, _)
            | E::ModK(a, _)
            | E::Abs(a)
            | E::At(_, _, a)
            | E::OptAt(_, _, _, a)
            | E::Remove(_, _, a, _)
            | E::IsUndefined(a)
            | E::Shift(_, a, _)
            | E::Plus(a)
            | E::Cast(a, _)
            | E::Size(a)
            | E::Index(_, _, _, a)
            | E::Field(_, a)
            | E::Chain(_, a)
            | E::NonNull(_, a)
            | E::SuperM(a)
            | E::InstX(a)
            | E::InstY(_, a)
            | E::InstTag(a)
            | E::InstDescribe(a)
            | E::Make(a)
            | E::Assign(_, _, a)
            | E::AddAssign(_, _, a)
            | E::Neg(a)
            | E::Pow(a, _)
            | E::Upper(a)
            | E::Lower(a)
            | E::Rep(a, _)
            | E::Join(a, _)
            | E::Bracket(_, a)
            | E::IndexAt(_, _, _, a)
            | E::Not(_, a)
            | E::IsEmpty(a)
            | E::Delete(_, _, a)
            | E::InstanceOf(a, _)
            | E::In(_, a)
            | E::NaN(a)
            | E::Maybe(a, _)
            | E::Split(a, _)
            | E::NewSetFrom(_, a)
            | E::New(_, a) => self.expr(a),
            E::Add(a, b)
            | E::Sub(a, b)
            | E::Mod(a, b)
            | E::Idiv(a, b)
            | E::SameAny(a, b, _)
            | E::Guard(_, _, a, b)
            | E::Bit(_, a, b)
            | E::IndexOf(a, b, _)
            | E::Includes(a, b, _)
            | E::Coalesce(a, b)
            | E::Or(_, a, b)
            | E::And(_, a, b)
            | E::OptField(_, a, b)
            | E::OptSize(a, b)
            | E::InstM(a, b)
            | E::OptCallVar(_, _, a, b)
            | E::MapGet(a, b)
            | E::Div(a, b)
            | E::Mul(a, b)
            | E::Concat(a, b)
            | E::Cmp(_, _, a, b)
            | E::Has(a, b)
            | E::Same(a, b)
            | E::Obj(a, b)
            | E::RecSpread(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            E::Cond(a, b, c) | E::ObjSpreadOpt(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            E::OptMethod(a, _, n, d) => {
                self.expr(a);
                if let Some(n) = n {
                    self.expr(n);
                }
                self.expr(d);
            }
            E::FindIndex(a, f) | E::Some(a, f) | E::Every(a, f) | E::Find(a, f) | E::Map(a, f) | E::Filter(a, f) => {
                self.expr(a);
                self.lambda(f);
            }
            E::Reduce(a, f, init) => {
                self.expr(a);
                self.lambda(f);
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            E::ReduceLit(items, f) => {
                for i in items {
                    self.expr(i);
                }
                self.lambda(f);
            }
            E::Push(_, _, items) | E::CallVar(_, _, items) | E::NewSet(_, items) => {
                for i in items {
                    self.expr(i);
                }
            }
            E::Call(_, args, spread) => {
                for a in args {
                    self.expr(a);
                }
                if let Some(s) = spread {
                    self.expr(s);
                }
            }
            E::Template(_, parts) => {
                for (p, _) in parts {
                    self.expr(p);
                }
            }
            E::Array(_, items) => {
                for i in items {
                    match i {
                        Item::One(e) | Item::Spread(e) => self.expr(e),
                    }
                }
            }
            E::ObjSpread(o, a) => {
                self.expr(o);
                if let Some(a) = a {
                    self.expr(a);
                }
            }
            E::Rec(end, then, inner) => {
                self.expr(end);
                if let Some(then) = then {
                    self.expr(then);
                }
                self.expr(inner);
            }
            E::NewMap(_, entries) => {
                for (k, v) in entries {
                    self.expr(k);
                    self.expr(v);
                }
            }
            E::Lambda(f) => self.lambda(f),
        }
    }
}

/// Numbers the statements and expressions, and lists the edits a pass may try.
#[derive(Default)]
struct Counter {
    statement: usize,
    expression: usize,
    statements: Vec<usize>,
    nested: Vec<usize>,
    /// Each expression with the number of an operand that may replace it.
    operands: Vec<(usize, usize)>,
}

impl Walk for Counter {
    fn next_statement(&mut self) -> usize {
        self.statement += 1;
        self.statement - 1
    }

    fn next_expression(&mut self) -> usize {
        self.expression += 1;
        self.expression - 1
    }

    fn post_statement(&mut self, n: usize, s: &mut Stmt) {
        if !matches!(s, Stmt::Block(body) if body.is_empty()) {
            self.statements.push(n);
        }
        if body(s).is_some() {
            self.nested.push(n);
        }
    }

    fn post_expression(&mut self, n: usize, e: &mut Expr) {
        for (j, operand) in operands(e).iter().enumerate() {
            if fits(operand, e) {
                self.operands.push((n, j));
            }
        }
    }
}

/// The walk was post-order, so the edits list inner parts first: put outer ones first.
impl Counter {
    fn program(&mut self, p: &mut Program) {
        Walk::program(self, p);
        self.statements.sort_unstable();
        self.nested.sort_unstable();
        self.operands.sort_unstable();
    }
}

struct Editor<'e> {
    edits: &'e Edits,
    statement: usize,
    expression: usize,
}

impl Walk for Editor<'_> {
    fn next_statement(&mut self) -> usize {
        self.statement += 1;
        self.statement - 1
    }

    fn next_expression(&mut self) -> usize {
        self.expression += 1;
        self.expression - 1
    }

    fn post_statement(&mut self, n: usize, s: &mut Stmt) {
        if self.edits.statements.contains(&n) {
            *s = Stmt::Block(Vec::new());
        } else if self.edits.flatten.contains(&n)
            && let Some(body) = body(s)
        {
            *s = Stmt::Block(body);
        }
    }

    fn post_expression(&mut self, n: usize, e: &mut Expr) {
        if let Some(&j) = self.edits.replace.get(&n)
            && let Some(operand) = operands(e).into_iter().nth(j)
        {
            *e = operand;
        }
    }
}

/// The program with `edit` applied to each expression, inner ones first, as the shrinker's own test edits one.
pub fn map_expressions(p: &Program, edit: fn(&mut Expr)) -> Program {
    struct Map(fn(&mut Expr));
    impl Walk for Map {
        fn next_statement(&mut self) -> usize {
            0
        }
        fn next_expression(&mut self) -> usize {
            0
        }
        fn post_expression(&mut self, _n: usize, e: &mut Expr) {
            (self.0)(e)
        }
    }
    let mut p = p.clone();
    Map(edit).program(&mut p);
    p
}

/// Drops the empty blocks deleted statements leave.
struct Sweep;

impl Walk for Sweep {
    fn next_statement(&mut self) -> usize {
        0
    }

    fn next_expression(&mut self) -> usize {
        0
    }

    fn list(&mut self, list: &mut Vec<Stmt>) {
        list.retain(|s| !matches!(s, Stmt::Block(body) if body.is_empty()));
        for s in list.iter_mut() {
            self.statement(s);
        }
    }
}

/// The statements a statement runs as its body, which a flattened statement keeps in a block.
fn body(s: &Stmt) -> Option<Vec<Stmt>> {
    match s {
        Stmt::If(_, body, _)
        | Stmt::For { body, .. }
        | Stmt::ForIndex { body, .. }
        | Stmt::ForOf { body, .. }
        | Stmt::ForOfStr(_, body)
        | Stmt::ForEach { body, .. }
        | Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::Try { body, .. }
        | Stmt::LocalFunction(body, _) => Some(body.clone()),
        Stmt::Switch { cases, .. } => cases.first().map(|(_, _, body, _)| body.clone()),
        _ => None,
    }
}

/// The direct operands of an expression, which may replace it when they fit.
fn operands(e: &Expr) -> Vec<Expr> {
    struct Collect(Vec<Expr>);
    impl Walk for Collect {
        fn next_statement(&mut self) -> usize {
            0
        }
        fn next_expression(&mut self) -> usize {
            0
        }
        fn expr(&mut self, e: &mut Expr) {
            self.0.push(e.clone());
        }
        fn lambda(&mut self, _f: &mut Lambda) {}
    }
    let mut collect = Collect(Vec::new());
    collect.parts(&mut e.clone());
    collect.0
}

/// What is known of an Int's sign. `nat`, `pos` and `neg` in the generator build `%`'s operands so that their signs
/// agree, so an edit must not swap one of those for an operand that may have another sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sign {
    Pos,
    /// At least 0.
    Nat,
    Neg,
    Any,
}

impl Sign {
    fn within(self, other: Sign) -> bool {
        self == other || other == Sign::Any || (self == Sign::Pos && other == Sign::Nat)
    }

    fn join(self, other: Sign) -> Sign {
        match (self, other) {
            _ if self.within(other) => other,
            _ if other.within(self) => self,
            _ => Sign::Any,
        }
    }
}

/// An operand may replace an expression when it has the same type, or is an Int where a `Num` was, as the generator
/// writes one, and keeps what is known of an Int's sign. The printer's casts would make TypeScript accept an operand
/// of another type, such as a string in a `boolean[]`, and that program can differ for a reason that has nothing to do
/// with the failure it shrinks.
fn fits(operand: &Expr, e: &Expr) -> bool {
    match (ty(operand), ty(e)) {
        (Some(Ty::Int), Some(Ty::Int)) => sign(operand).within(sign(e)),
        (Some(Ty::Int), Some(Ty::Num)) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn optional(ty: Ty) -> Option<Ty> {
    Some(match ty {
        Ty::Int => Ty::OptInt,
        Ty::Str => Ty::OptStr,
        Ty::Bool => Ty::OptBool,
        Ty::Obj => Ty::OptObj,
        _ => return None,
    })
}

fn optional_element(array: Ty) -> Option<Ty> {
    match array {
        Ty::ArrInt | Ty::ArrStr | Ty::ArrBool | Ty::ArrObj => Some(array.optional_element()),
        _ => None,
    }
}

fn element(array: Ty) -> Option<Ty> {
    match array {
        Ty::ArrInt | Ty::ArrNum | Ty::ArrStr | Ty::ArrBool | Ty::ArrObj => Some(array.element()),
        _ => None,
    }
}

fn numeric(a: Option<Ty>, b: Option<Ty>) -> Option<Ty> {
    match (a?, b?) {
        (Ty::Int, Ty::Int) => Some(Ty::Int),
        (Ty::Int | Ty::Num, Ty::Int | Ty::Num) => Some(Ty::Num),
        _ => None,
    }
}

fn field_ty(f: Field) -> Ty {
    match f {
        Field::A | Field::End | Field::InnerA => Ty::Int,
        Field::S | Field::InnerS => Ty::Str,
        Field::Then => Ty::OptStr,
        Field::Inner => Ty::Obj,
    }
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Int(i) if INTS[*i as usize % INTS.len()] == "0")
}

/// The type of an expression's value, or `None` where the shrinker cannot tell, which no edit then replaces. An Int is
/// a finite integer that is not `-0`, so a `-a`, `a * b` or `%` of negative numbers that may be `-0` is a `Num`.
pub fn ty(e: &Expr) -> Option<Ty> {
    use Expr as E;
    Some(match e {
        E::Int(_) => Ty::Int,
        E::Num(_) => Ty::Num,
        E::Str(..) => Ty::Str,
        E::Bool(_) => Ty::Bool,
        E::Var(t, _) => *t,
        // `a % b + 0` of two negative Ints, which the generator writes so that `-0` becomes `0`.
        E::Add(a, b) if matches!(**a, E::Mod(..)) && is_zero(b) => numeric(ty(a).map(|_| Ty::Int), ty(b))?,
        E::Add(a, b) | E::Sub(a, b) => numeric(ty(a), ty(b))?,
        E::Mod(a, b) => match numeric(ty(a), ty(b))? {
            Ty::Int if sign(a).within(Sign::Nat) => Ty::Int,
            _ => Ty::Num,
        },
        E::MulK(a, _) | E::Plus(a) | E::Cast(a, _) => numeric(ty(a), Some(Ty::Int))?,
        E::Neg(a) => match numeric(ty(a), Some(Ty::Int))? {
            Ty::Int if matches!(sign(a), Sign::Pos | Sign::Neg) => Ty::Int,
            _ => Ty::Num,
        },
        E::Abs(_) | E::ModK(..) | E::Bit(..) | E::Shift(..) | E::Size(_) | E::IndexOf(..) | E::FindIndex(..) => Ty::Int,
        E::Push(..) | E::Reduce(..) | E::ReduceLit(..) | E::OptSize(..) | E::SuperM(_) | E::InstX(_) => Ty::Int,
        E::InstY(..) | E::InstM(..) | E::Count(_) | E::Inc(..) | E::FieldStep(..) | E::OptCallVar(..) => Ty::Int,
        E::Idiv(..) | E::Div(..) | E::Mul(..) | E::Pow(..) => Ty::Num,
        E::Concat(..) | E::Template(..) | E::Upper(_) | E::Lower(_) | E::Rep(..) | E::Join(..) => Ty::Str,
        E::InstTag(_) | E::InstDescribe(_) => Ty::Str,
        E::IsUndefined(_) | E::SameAny(..) | E::Includes(..) | E::Cmp(..) | E::Not(..) | E::IsEmpty(_) => Ty::Bool,
        E::Some(..) | E::Every(..) | E::Has(..) | E::Delete(..) | E::InstanceOf(..) | E::In(..) | E::Same(..) => {
            Ty::Bool
        }
        E::NaN(_) => Ty::Bool,
        E::At(t, ..) | E::OptAt(t, ..) | E::Remove(t, ..) | E::Pop(t, _) | E::ShiftArr(t, _) => optional_element(*t)?,
        E::Index(t, ..) | E::IndexAt(t, ..) => element(*t)?,
        E::Find(a, _) => optional_element(ty(a)?)?,
        E::OptMethod(_, _, Some(_), _) => Ty::Int,
        E::OptMethod(_, _, None, _) => Ty::Str,
        E::ObjSpreadOpt(..) | E::Obj(..) | E::ObjSpread(..) => Ty::Obj,
        E::Rec(..) | E::RecSpread(..) => Ty::Rec,
        E::Cond(_, a, b) => match (ty(a)?, ty(b)?) {
            (a, b) if a == b => a,
            (Ty::Int | Ty::Num, Ty::Int | Ty::Num) => Ty::Num,
            _ => return None,
        },
        E::Coalesce(a, b) => {
            let (a, b) = (ty(a)?, ty(b)?);
            if a != b && optional(b) != Some(a) {
                return None;
            }
            b
        }
        E::Or(t, ..) | E::And(t, ..) => *t,
        E::Field(f, _) | E::Bracket(f, _) | E::NonNull(f, _) | E::OptField(f, ..) => match f {
            // `o["inner"]["a"]` is what `Bracket` prints for every field but these.
            _ if matches!(e, E::Bracket(..)) && !matches!(f, Field::A | Field::S | Field::End) => Ty::Int,
            f => field_ty(*f),
        },
        E::Chain(f, _) => optional(field_ty(*f))?,
        E::Make(_) => Ty::Inst(0),
        E::New(k, _) => Ty::Inst(*k),
        E::Call(k, ..) => SIGNATURES.get(*k as usize)?.1,
        E::CallVar(t, ..) => match t {
            Ty::FnII => Ty::Int,
            Ty::FnSIS => Ty::Str,
            _ => return None,
        },
        E::Assign(t, ..) | E::AddAssign(t, ..) => *t,
        E::MapGet(m, _) => match ty(m)? {
            Ty::MapSI => Ty::OptInt,
            Ty::MapIS => Ty::OptStr,
            _ => return None,
        },
        E::Maybe(v, _) => optional(ty(v)?)?,
        E::Array(t, _) | E::NewMap(t, _) | E::NewSet(t, _) | E::NewSetFrom(t, _) => *t,
        E::Filter(a, _) => ty(a)?,
        E::Map(_, f) => match f.annotate? {
            Ty::Int => Ty::ArrInt,
            Ty::Str => Ty::ArrStr,
            Ty::Bool => Ty::ArrBool,
            Ty::Num => Ty::ArrNum,
            Ty::Obj => Ty::ArrObj,
            _ => return None,
        },
        E::Split(..) => Ty::ArrStr,
        E::Lambda(f) => match (f.params.as_slice(), f.annotate?) {
            ([Ty::Int], Ty::Int) => Ty::FnII,
            ([Ty::Str, Ty::Int], Ty::Str) => Ty::FnSIS,
            _ => return None,
        },
        E::This | E::Guard(..) => return None,
    })
}

/// What is known of an Int expression's sign, from the shapes `nat`, `pos` and `neg` build.
fn sign(e: &Expr) -> Sign {
    use Expr as E;
    match e {
        E::Int(i) => match INTS[*i as usize % INTS.len()] {
            "0" => Sign::Nat,
            text if text.starts_with('-') => Sign::Neg,
            _ => Sign::Pos,
        },
        E::Abs(_) | E::Size(_) => Sign::Nat,
        E::Add(a, b) => match (sign(a), sign(b)) {
            (Sign::Neg, Sign::Neg) => Sign::Neg,
            (a, b) if a.within(Sign::Nat) && b.within(Sign::Nat) => {
                if a == Sign::Pos || b == Sign::Pos {
                    Sign::Pos
                } else {
                    Sign::Nat
                }
            }
            _ => Sign::Any,
        },
        E::Sub(a, b) => match (sign(a), sign(b)) {
            (Sign::Neg, b) if b.within(Sign::Nat) => Sign::Neg,
            (Sign::Pos, Sign::Neg) => Sign::Pos,
            _ => Sign::Any,
        },
        E::MulK(a, _) | E::Plus(a) | E::Cast(a, _) => sign(a),
        E::Mod(a, b) if sign(a).within(Sign::Nat) && sign(b) == Sign::Pos => Sign::Nat,
        E::Neg(a) => match sign(a) {
            Sign::Pos => Sign::Neg,
            Sign::Neg => Sign::Pos,
            _ => Sign::Any,
        },
        E::Cond(_, a, b) => sign(a).join(sign(b)),
        _ => Sign::Any,
    }
}
