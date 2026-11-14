//! Shrinks a failing program. Each round prints every candidate one edit away, runs them all in one batch, and keeps
//! the edits that still fail the same way: together when the program with all of them still fails, else the one that
//! removes the most text. A compile costs about as much for a batch as for one program, so a round costs about one
//! compile, where proptest's own shrinking compiles once per step. The edits delete a statement, replace a statement
//! with a block of its body, and replace an expression with one of its operands. Every variable stays a reference by
//! type, so a deleted declaration leaves a program that still type-checks; an edit that does not is rejected and
//! dropped.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

use super::ast::*;
use super::print;
use super::run::{Failure, Outcome, Runner};

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

pub fn shrink(
    program: Program,
    kind: Failure,
    outcome: Outcome,
    batch: usize,
    limit: Duration,
    runner: &mut Runner,
) -> (String, Outcome, usize) {
    let started = Instant::now();
    let same = |o: &Outcome| o.failure() == Some(kind) && same_rejection(o, &outcome);
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
                let sources: Vec<String> = candidates.iter().map(|(_, _, s)| s.clone()).collect();
                let outcomes = runner.run(&sources);
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
                        let o = runner.run(std::slice::from_ref(&source)).remove(0);
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

/// A rejected program shrinks to one rejected with the same first error, not to any program that fails to compile.
fn same_rejection(a: &Outcome, b: &Outcome) -> bool {
    let code = |o: &Outcome| match o {
        Outcome::Rejected(text) => text.lines().next().and_then(|l| l.split(": ").nth(1)).map(str::to_owned),
        _ => None,
    };
    code(a) == code(b)
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
            .flat_map(|&(n, count)| {
                (0..count).map(move |j| Edits { replace: BTreeMap::from([(n, j)]), ..Edits::default() })
            })
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
        let count = operands(e).len();
        if count > 0 {
            self.operands.push((n, count));
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

/// The direct operands of an expression, which may replace it.
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
