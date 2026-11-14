//! Prints a program as one standalone TypeScript file. Variable references resolve here, against the variables in
//! scope, and the printer also keeps every program safe to run in both worlds: see "What a program never does" in
//! README.md for the rules it enforces.

use std::collections::HashSet;

use super::ast::*;

// The exponents both sides compute exactly: known.md, "** is C's pow".
const POWS: &[&str] = &["0", "1", "2", "-1"];

#[derive(Clone, Debug)]
struct Var {
    name: String,
    ty: Ty,
    mutable: bool,
    /// Never assigned or structurally changed: loop counters, guards, parameters and iteration values.
    frozen: bool,
    /// How many functions deep it was declared.
    level: u32,
    /// Initialized with a new collection, so nothing else refers to it.
    fresh: bool,
}

#[derive(Default)]
struct Frame {
    vars: Vec<Var>,
    /// Names referred to while this frame was open. A `let` in this frame may not hide them, because the reference
    /// would then fall in the new variable's temporal dead zone.
    used: HashSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    Read,
    /// Assign: a `let` that is not frozen.
    Write,
    /// Change a collection's contents.
    Mutate,
}

struct Printer<'p> {
    p: &'p Program,
    out: String,
    indent: usize,
    frames: Vec<Frame>,
    level: u32,
    /// Collection types being iterated by an enclosing loop: no variable of these types changes, which also covers
    /// aliases.
    frozen: Vec<Ty>,
    /// Names whose `let` is being initialized.
    tdz: Vec<String>,
    /// Only the innermost frame is visible, as in an aggregate's term.
    pure: bool,
    next: usize,
    functions: [bool; 5],
    classes: [bool; 3],
    abs: bool,
    maybe: bool,
    id: bool,
    /// The class whose method is being printed.
    class: Option<u8>,
    /// Inside a stored closure or a local function, which never calls a function variable: variables can be
    /// reassigned, so such a call could reach the closure itself and never end.
    closures: u32,
}

pub fn print(p: &Program) -> String {
    let mut pr = Printer {
        p,
        out: String::new(),
        indent: 0,
        frames: vec![Frame::default()],
        level: 0,
        frozen: Vec::new(),
        tdz: Vec::new(),
        pure: false,
        next: 0,
        functions: [false; 5],
        classes: [false; 3],
        abs: false,
        maybe: false,
        id: false,
        class: None,
        closures: 0,
    };
    pr.stmts(&p.body);
    let body = std::mem::take(&mut pr.out);
    let mut functions = vec![String::new(); SIGNATURES.len()];
    for k in (0..SIGNATURES.len()).rev() {
        if pr.functions[k] {
            pr.function(k);
            functions[k] = std::mem::take(&mut pr.out);
        }
    }
    let mut classes = vec![String::new(); 3];
    for k in (0..3).rev() {
        if pr.classes[k] {
            pr.class(k as u8);
            classes[k] = std::mem::take(&mut pr.out);
        }
    }
    let mut text = String::from("declare function trace(value: unknown, shape: string): void;\n");
    if pr.abs {
        text.push_str("function abs(n: number): number {\n\treturn n < 0 ? -n : n;\n}\n");
    }
    if pr.maybe {
        text.push_str(
            "function maybe<T>(value: T, keep: boolean): T | undefined {\n\treturn keep ? value : undefined;\n}\n",
        );
    }
    if pr.id {
        text.push_str("function id<T>(value: T): T {\n\treturn value;\n}\n");
    }
    for class in &classes {
        text.push_str(class);
    }
    let functions = functions.concat();
    if !p.functions_last {
        text.push_str(&functions);
    }
    text.push_str(&body);
    if p.functions_last {
        text.push_str(&functions);
    }
    text
}

pub fn ty_text(ty: Ty) -> String {
    match ty {
        Ty::Int | Ty::Num => "number".into(),
        Ty::Str => "string".into(),
        Ty::Bool => "boolean".into(),
        Ty::OptInt => "number | undefined".into(),
        Ty::OptStr => "string | undefined".into(),
        Ty::OptBool => "boolean | undefined".into(),
        Ty::ArrInt | Ty::ArrNum => "number[]".into(),
        Ty::ArrStr => "string[]".into(),
        Ty::ArrBool => "boolean[]".into(),
        Ty::ArrObj => "{ a: number; s: string }[]".into(),
        Ty::Obj => "{ a: number; s: string }".into(),
        Ty::Rec => "{ end: number; then?: string; inner: { a: number; s: string } }".into(),
        Ty::OptObj => "{ a: number; s: string } | undefined".into(),
        Ty::MapSI => "Map<string, number>".into(),
        Ty::MapIS => "Map<number, string>".into(),
        Ty::SetS => "Set<string>".into(),
        Ty::SetI => "Set<number>".into(),
        Ty::FnII => "(n: number) => number".into(),
        Ty::FnSIS => "(s: string, n: number) => string".into(),
        Ty::Inst(k) => format!("C{k}"),
    }
}

impl Printer<'_> {
    fn shape(&self, ty: Ty) -> String {
        match ty {
            Ty::Int | Ty::Num => "n".into(),
            Ty::Str => "s".into(),
            Ty::Bool => "b".into(),
            Ty::OptInt => "?n".into(),
            Ty::OptStr => "?s".into(),
            Ty::OptBool => "?b".into(),
            Ty::ArrInt | Ty::ArrNum => "[n]".into(),
            Ty::ArrStr => "[s]".into(),
            Ty::ArrBool => "[b]".into(),
            Ty::ArrObj => "[{a:n,s:s}]".into(),
            Ty::Obj => "{a:n,s:s}".into(),
            Ty::Rec => "{end:n,inner:{a:n,s:s},then:?s}".into(),
            Ty::OptObj => "?{a:n,s:s}".into(),
            Ty::MapSI => "M(s,n)".into(),
            Ty::MapIS => "M(n,s)".into(),
            Ty::SetS => "S(s)".into(),
            Ty::SetI => "S(n)".into(),
            Ty::FnII | Ty::FnSIS => "x".into(),
            Ty::Inst(k) => {
                if self.has_y(k) {
                    "{tag:s,x:n,y:n}".into()
                } else {
                    "{tag:s,x:n}".into()
                }
            }
        }
    }

    fn has_y(&self, k: u8) -> bool {
        self.p.classes[k as usize].parent.is_some()
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push('\t');
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn declare(&mut self, name: String, ty: Ty, mutable: bool, frozen: bool, fresh: bool) {
        let level = self.level;
        self.frames.last_mut().expect("a frame is open").vars.push(Var { name, ty, mutable, frozen, level, fresh });
    }

    fn open(&mut self) {
        self.frames.push(Frame::default());
    }

    fn close(&mut self) {
        self.frames.pop();
    }

    fn visible(&self) -> Vec<Var> {
        let frames = if self.pure { &self.frames[self.frames.len() - 1..] } else { &self.frames[..] };
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for frame in frames.iter().rev() {
            for v in frame.vars.iter().rev() {
                if seen.insert(v.name.clone()) && !self.tdz.contains(&v.name) {
                    found.push(v.clone());
                }
            }
        }
        found.reverse();
        found
    }

    fn allowed(&self, v: &Var, need: Need) -> bool {
        match need {
            Need::Read => true,
            Need::Write => !self.pure && v.mutable && !v.frozen,
            Need::Mutate => {
                !self.pure
                    && !v.frozen
                    && v.level == self.level
                    && (self.level == 0 || v.fresh)
                    && !self.frozen.contains(&v.ty)
            }
        }
    }

    fn candidates(&self, ty: Ty, need: Need) -> Vec<Var> {
        self.visible().into_iter().filter(|v| v.ty == ty && self.allowed(v, need)).collect()
    }

    fn mark(&mut self, name: &str) {
        for frame in &mut self.frames {
            frame.used.insert(name.to_owned());
        }
    }

    fn resolve(&mut self, ty: Ty, k: u8, need: Need) -> Option<String> {
        let found = self.candidates(ty, need);
        if found.is_empty() {
            return None;
        }
        let name = found[k as usize % found.len()].name.clone();
        self.mark(&name);
        Some(name)
    }

    /// A variable of an outer frame that a new `let` here may hide.
    fn shadowable(&self, k: u8) -> Option<String> {
        if self.pure || self.frames.len() < 2 {
            return None;
        }
        let current = self.frames.last().expect("a frame is open");
        let own: HashSet<&String> = current.vars.iter().map(|v| &v.name).collect();
        let outer: Vec<&Var> = self.frames[..self.frames.len() - 1]
            .iter()
            .flat_map(|f| f.vars.iter())
            .filter(|v| !own.contains(&v.name) && !current.used.contains(&v.name) && !v.name.contains('['))
            .collect();
        (!outer.is_empty()).then(|| outer[k as usize % outer.len()].name.clone())
    }

    fn default(&mut self, ty: Ty) -> String {
        match ty {
            Ty::Int | Ty::Num => "0".into(),
            Ty::Str => "\"\"".into(),
            Ty::Bool => "false".into(),
            Ty::OptInt => {
                self.maybe = true;
                "maybe(0, false)".into()
            }
            Ty::OptStr => {
                self.maybe = true;
                "maybe(\"\", false)".into()
            }
            Ty::OptBool => {
                self.maybe = true;
                "maybe(false, false)".into()
            }
            Ty::OptObj => {
                self.maybe = true;
                "maybe({ a: 0, s: \"\" }, false)".into()
            }
            Ty::ArrInt | Ty::ArrNum | Ty::ArrStr | Ty::ArrBool | Ty::ArrObj => format!("([] as {})", ty_text(ty)),
            Ty::Obj => "{ a: 0, s: \"\" }".into(),
            Ty::Rec => format!("({{ end: 0, inner: {{ a: 0, s: \"\" }} }} as {})", ty_text(Ty::Rec)),
            Ty::MapSI | Ty::MapIS | Ty::SetS | Ty::SetI => format!("new {}()", ty_text(ty)),
            Ty::FnII => {
                let n = self.fresh("x");
                format!("(({n}: number): number => {n})")
            }
            Ty::FnSIS => {
                let (s, n) = (self.fresh("x"), self.fresh("x"));
                format!("(({s}: string, {n}: number): string => {s})")
            }
            Ty::Inst(k) => {
                self.classes[k as usize] = true;
                format!("new C{k}(0)")
            }
        }
    }

    fn var(&mut self, ty: Ty, k: u8) -> String {
        match self.resolve(ty, k, Need::Read) {
            Some(name) => name,
            // A literal here could make a condition constant, which TypeScript rejects.
            None if ty == Ty::Str => {
                self.id = true;
                "id(\"\")".into()
            }
            None => self.default(ty),
        }
    }

    /// Hides a literal from TypeScript's checks of constant conditions, which are errors. A fallback that would print
    /// a literal in place of a missing variable goes through here too.
    fn opaque(&mut self, text: String, e: &Expr) -> String {
        if constant(e) {
            self.id = true;
            format!("id({text})")
        } else {
            text
        }
    }

    /// Keeps TypeScript from narrowing a variable, which can make it `never` in a branch.
    fn unnarrowed(&mut self, text: String) -> String {
        if is_name(&text) {
            self.id = true;
            format!("id({text})")
        } else {
            text
        }
    }

    fn e(&mut self, x: &Expr) -> String {
        match x {
            Expr::Int(i) => number(INTS[*i as usize % INTS.len()]),
            Expr::Num(i) => number(NUMS[*i as usize % NUMS.len()]),
            Expr::Str(i, q) => quote(STRS[*i as usize % STRS.len()], *q),
            Expr::Bool(v) => v.to_string(),
            Expr::Var(ty, k) => self.var(*ty, *k),
            Expr::Add(..)
            | Expr::Sub(..)
            | Expr::Mul(..)
            | Expr::Div(..)
            | Expr::Concat(..)
            | Expr::MulK(..)
            | Expr::Mod(..)
            | Expr::Coalesce(..)
            | Expr::Or(Ty::Bool, ..)
            | Expr::And(Ty::Bool, ..) => {
                let text = self.bare(x);
                format!("({text})")
            }
            Expr::Abs(a) => {
                self.abs = true;
                format!("abs({})", self.e(a))
            }
            Expr::Idiv(a, b) => {
                let a = self.recv(a);
                format!("{a}.idiv({})", self.e(b))
            }
            Expr::At(ty, k, i) => {
                let optional = ty.optional_element();
                match self.resolve(*ty, *k, Need::Read) {
                    Some(xs) => format!("({xs}[{}] as {})", self.operand(i, 0), ty_text(optional)),
                    None => self.default(optional),
                }
            }
            Expr::OptAt(ty, k, keep, i) => {
                let optional = ty.optional_element();
                match self.resolve(*ty, *k, Need::Read) {
                    Some(xs) => {
                        self.maybe = true;
                        format!("(maybe({xs}, {keep})?.[{}] as {})", self.operand(i, 0), ty_text(optional))
                    }
                    None => self.default(optional),
                }
            }
            Expr::Remove(ty, k, i, unordered) => {
                let optional = ty.optional_element();
                match self.resolve(*ty, *k, Need::Mutate) {
                    Some(xs) => {
                        let i = self.operand(i, INDEX);
                        let method = if *unordered { "unorderedRemove" } else { "remove" };
                        format!("({i} >= 0 && {i} < {xs}.size() ? {xs}.{method}({i}) : undefined)")
                    }
                    None => self.default(optional),
                }
            }
            Expr::IsUndefined(a) => {
                let a = self.e(a);
                format!("({} === undefined)", self.unnarrowed(a))
            }
            Expr::SameAny(a, b, ne) => {
                let (a, b) = (self.e(a), self.e(b));
                let op = if *ne { "!==" } else { "===" };
                format!("(({a} as unknown) {op} ({b} as unknown))")
            }
            Expr::OptMethod(i, keep, n, d) => {
                self.maybe = true;
                let i = self.e(i);
                let call = match n {
                    Some(n) => format!("m({})", self.e(n)),
                    None => "describe()".into(),
                };
                format!("(maybe({i}, {keep})?.{call} ?? {})", self.e(d))
            }
            Expr::OptCallVar(k, keep, n, d) => {
                self.maybe = true;
                let found = if self.closures > 0 { None } else { self.resolve(Ty::FnII, *k, Need::Read) };
                let callee = match found {
                    Some(name) => name,
                    None => self.default(Ty::FnII),
                };
                let n = self.e(n);
                format!("(maybe({callee}, {keep})?.({n}) ?? {})", self.e(d))
            }
            Expr::ObjSpreadOpt(a, s, o) => {
                let (a, s) = (self.e(a), self.e(s));
                format!("{{ a: {a}, s: {s}, ...{} }}", self.e(o))
            }
            Expr::Guard(op, ty, a, f) => {
                let left = self.e(a);
                let left = self.opaque(left, a);
                let (op, left) = match op {
                    Logical::And => ("&&", truthy(left, *ty)),
                    Logical::Or => ("||", truthy(left, *ty)),
                    Logical::Coalesce => ("??", left),
                };
                format!("{left} {op} {}", self.e(f))
            }
            Expr::ModK(a, k) => {
                self.abs = true;
                format!("(abs({}) % {k})", self.e(a))
            }
            Expr::Bit(op, a, b) => {
                self.abs = true;
                let op = match op {
                    Bit::And => "&",
                    Bit::Or => "|",
                    Bit::Xor => "^",
                };
                let (a, b) = (self.e(a), self.e(b));
                format!("((abs({a}) % 256) {op} (abs({b}) % 256))")
            }
            Expr::Shift(op, a, k) => {
                self.abs = true;
                let op = match op {
                    Shift::Left => "<<",
                    Shift::Right => ">>",
                    Shift::Unsigned => ">>>",
                };
                format!("((abs({}) % 256) {op} {k})", self.e(a))
            }
            Expr::Plus(a) => format!("(+{})", self.e(a)),
            Expr::Neg(a) => format!("(-{})", self.e(a)),
            Expr::Pow(a, k) => format!("({} ** {})", self.e(a), POWS[*k as usize % POWS.len()]),
            Expr::Cast(a, satisfies) => {
                let a = self.e(a);
                if *satisfies { format!("({a} satisfies number)") } else { format!("({a} as number)") }
            }
            Expr::Size(a) => format!("{}.size()", self.recv(a)),
            Expr::IndexOf(a, v, from) => {
                let (a, v) = (self.recv(a), self.e(v));
                match from {
                    Some(from) => format!("{a}.indexOf({v}, {})", number(&from.to_string())),
                    None => format!("{a}.indexOf({v})"),
                }
            }
            Expr::Includes(a, v, from) => {
                let (a, v) = (self.recv(a), self.e(v));
                match from {
                    Some(from) => format!("{a}.includes({v}, {})", number(&from.to_string())),
                    None => format!("{a}.includes({v})"),
                }
            }
            Expr::FindIndex(a, f) => self.method(a, "findIndex", f),
            Expr::Some(a, f) => self.method(a, "some", f),
            Expr::Every(a, f) => self.method(a, "every", f),
            Expr::Find(a, f) => self.method(a, "find", f),
            Expr::Map(a, f) => self.method(a, "map", f),
            Expr::Filter(a, f) => self.method(a, "filter", f),
            Expr::Push(ty, k, values) => {
                let target = self.resolve(*ty, *k, Need::Mutate);
                let values = self.list(values);
                match target {
                    Some(name) => format!("{name}.push({values})"),
                    None => format!("([] as {}).push({values})", ty_text(*ty)),
                }
            }
            Expr::Reduce(a, f, init) => {
                let a = self.recv(a);
                let f = self.lambda(f);
                match init {
                    // The cast keeps TypeScript from inferring a literal accumulator type.
                    Some(init) => format!("{a}.reduce({f}, {} as number)", self.e(init)),
                    None => format!("{a}.reduce({f})"),
                }
            }
            Expr::ReduceLit(items, f) => {
                let items = self.list(items);
                format!("[{items}].reduce({})", self.lambda(f))
            }
            Expr::Index(ty, k, i, d) => match self.resolve(*ty, *k, Need::Read) {
                Some(name) => format!("({name}.size() > {i} ? {name}[{i}] : {})", self.e(d)),
                None => {
                    let text = self.e(d);
                    self.opaque(text, d)
                }
            },
            Expr::Cond(c, a, b) => {
                let c = self.e(c);
                let c = self.opaque(c, x.cond());
                format!("({c} ? {} : {})", self.e(a), self.e(b))
            }
            Expr::Or(ty, a, b) | Expr::And(ty, a, b) => {
                let op = if matches!(x, Expr::Or(..)) { "||" } else { "&&" };
                let left = self.e(a);
                let left = self.opaque(left, a);
                let left = truthy(left, *ty);
                // TypeScript narrows `n && 0` to the type `0`, which an array literal would take as its element.
                format!("(({left} {op} {}) as {})", self.e(b), ty_text(*ty))
            }
            Expr::Field(f, o) => format!("{}{}", self.recv(o), field(*f)),
            Expr::Chain(f, o) => format!("{}?{}", self.recv(o), field(*f)),
            Expr::OptField(f, o, d) => {
                let o = self.recv(o);
                format!("({o}?{} ?? {})", field(*f), self.e(d))
            }
            Expr::OptSize(r, d) => {
                let r = self.recv(r);
                format!("({r}.then?.size() ?? {})", self.e(d))
            }
            Expr::NonNull(f, o) => {
                self.maybe = true;
                format!("maybe({}, true)!{}", self.e(o), field(*f))
            }
            Expr::This => "this".into(),
            Expr::SuperM(n) => format!("super.m({})", self.e(n)),
            Expr::InstX(i) => format!("{}.x", self.recv(i)),
            Expr::InstY(k, i) => {
                let text = self.recv(i);
                let class = if matches!(**i, Expr::This) { self.class.unwrap_or(*k) } else { *k };
                if self.has_y(class) { format!("{text}.y") } else { "0".into() }
            }
            Expr::InstTag(i) => format!("{}.tag", self.recv(i)),
            Expr::InstM(i, n) => {
                let i = self.recv(i);
                format!("{i}.m({})", self.e(n))
            }
            Expr::InstDescribe(i) => format!("{}.describe()", self.recv(i)),
            Expr::Count(k) => {
                self.classes[*k as usize] = true;
                format!("C{k}.count")
            }
            Expr::Make(n) => {
                self.classes[0] = true;
                format!("C0.make({})", self.e(n))
            }
            Expr::New(k, n) => {
                self.classes[*k as usize] = true;
                format!("new C{k}({})", self.e(n))
            }
            Expr::Call(k, args, spread) => {
                self.functions[*k as usize] = true;
                if *k == 4 {
                    self.abs = true;
                    return format!("f4(abs({}) % 6)", self.e(&args[0]));
                }
                let mut parts: Vec<String> = args.iter().map(|a| self.e(a)).collect();
                if let Some(spread) = spread {
                    parts.push(format!("...{}", self.e(spread)));
                }
                format!("f{k}({})", parts.join(", "))
            }
            Expr::CallVar(ty, k, args) => {
                let found = if self.closures > 0 { None } else { self.resolve(*ty, *k, Need::Read) };
                let callee = match found {
                    Some(name) => name,
                    None => self.default(*ty),
                };
                format!("{callee}({})", self.list(args))
            }
            Expr::Inc(op, k) => match self.resolve(Ty::Int, *k, Need::Write) {
                Some(name) => match op {
                    Step::PreInc => format!("(++{name})"),
                    Step::PostInc => format!("({name}++)"),
                    Step::PreDec => format!("(--{name})"),
                    Step::PostDec => format!("({name}--)"),
                },
                None => "0".into(),
            },
            Expr::FieldStep(op, k) => match self.resolve(Ty::Obj, *k, Need::Read) {
                Some(name) => match op {
                    Step::PreInc => format!("(++{name}.a)"),
                    Step::PostInc => format!("({name}.a++)"),
                    Step::PreDec => format!("(--{name}.a)"),
                    Step::PostDec => format!("({name}.a--)"),
                },
                None => "0".into(),
            },
            Expr::Assign(ty, k, v) | Expr::AddAssign(ty, k, v) => match self.resolve(*ty, *k, Need::Write) {
                Some(name) => {
                    let op = if matches!(x, Expr::Assign(..)) { "=" } else { "+=" };
                    match ty {
                        Ty::Str => format!("({} ? ({name} {op} {}) : {name})", short(&name), self.e(v)),
                        _ => format!("({name} {op} {})", self.e(v)),
                    }
                }
                // A string's `+=` may add a number, so the value goes through a template.
                None if *ty == Ty::Str => format!("`${{{}}}`", self.e(v)),
                None => {
                    let text = self.e(v);
                    self.opaque(text, v)
                }
            },
            Expr::MapGet(m, key) => {
                let m = self.recv(m);
                format!("{m}.get({})", self.e(key))
            }
            Expr::Template(head, parts) => {
                let mut text = format!("`{}", template_text(STRS[*head as usize % STRS.len()]));
                for (part, tail) in parts {
                    let part = self.e(part);
                    text.push_str(&format!("${{{part}}}{}", template_text(STRS[*tail as usize % STRS.len()])));
                }
                text.push('`');
                text
            }
            Expr::Upper(a) => format!("{}.upper()", self.recv(a)),
            Expr::Lower(a) => format!("{}.lower()", self.recv(a)),
            Expr::Rep(a, k) => format!("{}.rep({k})", self.recv(a)),
            Expr::Join(a, sep) => match sep {
                Some(sep) => format!("{}.join({})", self.recv(a), quote(JOINS[*sep as usize % JOINS.len()], 0)),
                None => format!("{}.join()", self.recv(a)),
            },
            Expr::Bracket(f, o) => {
                let key = match f {
                    Field::A => "[\"a\"]",
                    Field::S => "[\"s\"]",
                    Field::End => "[\"end\"]",
                    _ => "[\"inner\"][\"a\"]",
                };
                format!("{}{key}", self.recv(o))
            }
            Expr::IndexAt(ty, k, j, d) => {
                let array = self.resolve(*ty, *k, Need::Read);
                let index = self.resolve(Ty::Int, *j, Need::Read);
                match (array, index) {
                    (Some(xs), Some(j)) => {
                        let d = self.e(d);
                        format!("({j} >= 0 && {j} < {xs}.size() ? {xs}[{j}] : {d})")
                    }
                    _ => {
                        let text = self.e(d);
                        self.opaque(text, d)
                    }
                }
            }
            Expr::Cmp(op, ty, a, b) => {
                let op = match op {
                    Cmp::Lt => "<",
                    Cmp::Le => "<=",
                    Cmp::Gt => ">",
                    Cmp::Ge => ">=",
                    Cmp::Eq => "===",
                    Cmp::Ne => "!==",
                };
                // A cast keeps TypeScript from rejecting a comparison of two literals or of a narrowed variable.
                let t = ty_text(*ty);
                let (a, b) = (self.e(a), self.e(b));
                format!("(({a} as {t}) {op} ({b} as {t}))")
            }
            Expr::Not(ty, a) => {
                let text = self.e(a);
                let text = self.opaque(text, a);
                format!("(!{})", truthy(text, *ty))
            }
            Expr::IsEmpty(a) => format!("{}.isEmpty()", self.recv(a)),
            Expr::Has(m, key) => {
                let m = self.recv(m);
                format!("{m}.has({})", self.e(key))
            }
            Expr::Delete(ty, k, key) => {
                let target = self.resolve(*ty, *k, Need::Mutate);
                let key = self.e(key);
                match target {
                    Some(name) => format!("{name}.delete({key})"),
                    None => format!("{}.delete({key})", self.default(*ty)),
                }
            }
            Expr::InstanceOf(i, k) => {
                self.classes[*k as usize] = true;
                let i = self.e(i);
                format!("({} instanceof C{k})", self.unnarrowed(i))
            }
            Expr::In(f, r) => {
                let name = match f {
                    Field::Then => "then",
                    Field::End => "end",
                    _ => "inner",
                };
                let r = self.e(r);
                format!("(\"{name}\" in {})", self.unnarrowed(r))
            }
            Expr::Same(a, b) => {
                let (Expr::Var(_, a), Expr::Var(_, b)) = (&**a, &**b) else { return "false".into() };
                match (self.resolve(Ty::Obj, *a, Need::Read), self.resolve(Ty::Obj, *b, Need::Read)) {
                    (Some(a), Some(b)) => format!("({a} === {b})"),
                    _ => "false".into(),
                }
            }
            Expr::NaN(a) => {
                let a = self.e(a);
                format!("(({a} as number) !== ({a} as number))")
            }
            Expr::Maybe(v, keep) => {
                self.maybe = true;
                format!("maybe({}, {keep})", self.e(v))
            }
            Expr::Pop(ty, k) | Expr::ShiftArr(ty, k) => {
                let method = if matches!(x, Expr::Pop(..)) { "pop" } else { "shift" };
                match self.resolve(*ty, *k, Need::Mutate) {
                    Some(name) => format!("{name}.{method}()"),
                    None => format!("{}.{method}()", self.default(*ty)),
                }
            }
            Expr::Array(ty, items) => {
                if items.is_empty() {
                    return self.default(*ty);
                }
                let items: Vec<String> = items
                    .iter()
                    .map(|item| match item {
                        Item::One(e) => self.e(e),
                        Item::Spread(e) => format!("...{}", self.e(e)),
                    })
                    .collect();
                format!("[{}]", items.join(", "))
            }
            Expr::Split(s, sep) => {
                format!("{}.split({})", self.recv(s), quote(SPLITS[*sep as usize % SPLITS.len()], 0))
            }
            Expr::Obj(a, s) => {
                let a = self.e(a);
                format!("{{ a: {a}, s: {} }}", self.e(s))
            }
            Expr::ObjSpread(o, a) => {
                let o = self.e(o);
                match a {
                    Some(a) => format!("{{ ...{o}, a: {} }}", self.e(a)),
                    None => format!("{{ ...{o} }}"),
                }
            }
            // The casts keep `then` in the type when the literal leaves it out.
            Expr::Rec(end, then, inner) => {
                let end = self.e(end);
                let then = then.as_ref().map(|t| format!(", then: {}", self.e(t))).unwrap_or_default();
                format!("({{ end: {end}{then}, inner: {} }} as {})", self.e(inner), ty_text(Ty::Rec))
            }
            Expr::RecSpread(r, end) => {
                let r = self.e(r);
                format!("({{ ...{r}, end: {} }} as {})", self.e(end), ty_text(Ty::Rec))
            }
            Expr::NewMap(ty, entries) => {
                if entries.is_empty() {
                    return format!("new {}()", ty_text(*ty));
                }
                let entries: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| {
                        let k = self.e(k);
                        format!("[{k}, {}]", self.e(v))
                    })
                    .collect();
                format!("new {}([{}])", ty_text(*ty), entries.join(", "))
            }
            Expr::NewSet(ty, items) => {
                if items.is_empty() {
                    return format!("new {}()", ty_text(*ty));
                }
                format!("new {}([{}])", ty_text(*ty), self.list(items))
            }
            Expr::NewSetFrom(ty, a) => format!("new {}({})", ty_text(*ty), self.e(a)),
            Expr::Lambda(f) => format!("({})", self.lambda(f)),
        }
    }

    /// A binary expression without its outer parentheses, and with only the parentheses its operands need, so the
    /// compiler sees chains such as `a ?? b ?? c` and `a - b + c` as TypeScript parses them.
    fn bare(&mut self, x: &Expr) -> String {
        match x {
            Expr::Add(a, b) | Expr::Concat(a, b) => self.infix(a, "+", b, ADD),
            Expr::Sub(a, b) => self.infix(a, "-", b, ADD),
            Expr::Mul(a, b) => self.infix(a, "*", b, MUL),
            Expr::Div(a, b) => self.infix(a, "/", b, MUL),
            Expr::Mod(a, b) => self.infix(a, "%", b, MUL),
            Expr::MulK(a, k) => format!("{} * {k}", self.operand(a, MUL)),
            // `??` takes no `||` or `&&` operand without parentheses, so only a `??` stays bare on its left.
            Expr::Coalesce(a, b) => {
                let left = if matches!(**a, Expr::Coalesce(..)) { self.bare(a) } else { self.e(a) };
                format!("{left} ?? {}", self.e(b))
            }
            Expr::Or(Ty::Bool, a, b) | Expr::And(Ty::Bool, a, b) => {
                let (op, p) = if matches!(x, Expr::Or(..)) { ("||", OR) } else { ("&&", AND) };
                let left = self.operand(a, p);
                let left = self.opaque(left, a);
                format!("{left} {op} {}", self.operand(b, p + 1))
            }
            other => self.e(other),
        }
    }

    fn infix(&mut self, a: &Expr, op: &str, b: &Expr, p: u8) -> String {
        let a = self.operand(a, p);
        format!("{a} {op} {}", self.operand(b, p + 1))
    }

    /// An operand written bare when its operator binds at least as tightly as `min`.
    fn operand(&mut self, x: &Expr, min: u8) -> String {
        match ts_precedence(x) {
            Some(p) if p >= min => self.bare(x),
            _ => self.e(x),
        }
    }

    fn list(&mut self, items: &[Expr]) -> String {
        let items: Vec<String> = items.iter().map(|e| self.e(e)).collect();
        items.join(", ")
    }

    /// The receiver of a member access, in parentheses unless it is a name.
    fn recv(&mut self, x: &Expr) -> String {
        let text = self.e(x);
        if is_name(&text) || text == "this" { text } else { format!("({text})") }
    }

    fn method(&mut self, a: &Expr, name: &str, f: &Lambda) -> String {
        let a = self.recv(a);
        format!("{a}.{name}({})", self.lambda(f))
    }

    fn lambda(&mut self, f: &Lambda) -> String {
        self.level += 1;
        self.open();
        let closure = u32::from(f.annotate.is_some());
        self.closures += closure;
        let saved_pure = std::mem::replace(&mut self.pure, false);
        let mut params = Vec::new();
        for (i, &ty) in f.params.iter().enumerate() {
            if i == 0 && f.destructure {
                let (a, s) = (self.fresh("x"), self.fresh("x"));
                params.push(format!("{{ a: {a}, s: {s} }}"));
                self.declare(a, Ty::Int, false, true, false);
                self.declare(s, Ty::Str, false, true, false);
                continue;
            }
            let name = self.fresh("x");
            params.push(match f.annotate {
                Some(_) => format!("{name}: {}", ty_text(ty)),
                None => name.clone(),
            });
            self.declare(name, ty, false, true, false);
        }
        let head = match f.annotate {
            Some(ret) => format!("({}): {} =>", params.join(", "), ty_text(ret)),
            None => format!("({}) =>", params.join(", ")),
        };
        let text = match &f.body {
            Body::Expr(e) => {
                let body = self.e(e);
                if body.starts_with('{') { format!("{head} ({body})") } else { format!("{head} {body}") }
            }
            Body::Block(stmts, ret) => {
                let saved = std::mem::take(&mut self.out);
                self.indent += 1;
                let ended = self.stmts(stmts);
                if let (false, Some(ret)) = (ended, ret) {
                    let ret = self.e(ret);
                    self.line(&format!("return {ret};"));
                }
                self.indent -= 1;
                let body = std::mem::replace(&mut self.out, saved);
                let close = "\t".repeat(self.indent);
                format!("{head} {{\n{body}{close}}}")
            }
        };
        self.pure = saved_pure;
        self.closures -= closure;
        self.close();
        self.level -= 1;
        text
    }

    /// Prints statements until one that always jumps, and says whether it found one.
    fn stmts(&mut self, list: &[Stmt]) -> bool {
        for s in list {
            self.stmt(s);
            if matches!(s, Stmt::Break | Stmt::Continue | Stmt::Return(_) | Stmt::Throw(_)) {
                return true;
            }
        }
        false
    }

    fn block(&mut self, list: &[Stmt]) -> bool {
        self.indent += 1;
        self.open();
        let ended = self.stmts(list);
        self.close();
        self.indent -= 1;
        ended
    }

    /// A loop body whose first statement counts `guard`, so no `let` in it may hide the guard.
    fn guarded_block(&mut self, list: &[Stmt], guard: &str) -> bool {
        self.indent += 1;
        self.open();
        self.mark(guard);
        let ended = self.stmts(list);
        self.close();
        self.indent -= 1;
        ended
    }

    /// A loop body after its `first` lines, which read `names`, so no `let` in the body may hide them.
    fn body_after(&mut self, first: &[String], names: &[&str], list: &[Stmt], frozen: Option<Ty>) {
        self.frozen.extend(frozen);
        self.indent += 1;
        self.open();
        for name in names {
            self.mark(name);
        }
        for line in first {
            self.line(line);
        }
        self.stmts(list);
        self.close();
        self.indent -= 1;
        if frozen.is_some() {
            self.frozen.pop();
        }
    }

    /// Statements in a loop body, where the collection types in `frozen` must not change.
    fn frozen_block(&mut self, list: &[Stmt], frozen: Option<Ty>) {
        self.frozen.extend(frozen);
        self.block(list);
        if frozen.is_some() {
            self.frozen.pop();
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { ty, mutable, shadow, init } => {
                let name = match shadow.and_then(|k| self.shadowable(k)) {
                    Some(name) => name,
                    None => self.fresh("v"),
                };
                self.tdz.push(name.clone());
                let init_text = self.e(init);
                self.tdz.pop();
                let keyword = if *mutable { "let" } else { "const" };
                self.line(&format!("{keyword} {name}: {} = {init_text};", ty_text(*ty)));
                self.declare(name, *ty, *mutable, false, makes_new(init));
            }
            Stmt::Assign(ty, k, v) | Stmt::AddAssign(ty, k, v) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Write) {
                    let op = if matches!(s, Stmt::Assign(..)) { "=" } else { "+=" };
                    let v = self.e(v);
                    match ty {
                        Ty::Str => self.line(&format!("if ({}) {name} {op} {v};", short(&name))),
                        _ => self.line(&format!("{name} {op} {v};")),
                    }
                }
            }
            Stmt::SubAssign(k, v) => {
                if let Some(name) = self.resolve(Ty::Int, *k, Need::Write) {
                    let v = self.e(v);
                    self.line(&format!("{name} -= {v};"));
                }
            }
            Stmt::MulAssign(k, m) => {
                if let Some(name) = self.resolve(Ty::Int, *k, Need::Write) {
                    self.line(&format!("{name} *= {m};"));
                }
            }
            Stmt::NumAssign(k, op, v) => {
                if let Some(name) = self.resolve(Ty::Num, *k, Need::Write) {
                    let text = match op {
                        0 => format!("{name} -= {};", self.e(v)),
                        1 => format!("{name} *= {};", self.e(v)),
                        2 => format!("{name} /= {};", self.e(v)),
                        _ => format!("{name} **= {};", POWS[*op as usize % POWS.len()]),
                    };
                    self.line(&text);
                }
            }
            Stmt::Compound(op, a, b) => {
                let r = self.fresh("r");
                let (a, b) = (self.e(a), self.e(b));
                // Operands under 256 and shifts by at most 8 keep the bitwise results unsigned on both sides: known.md,
                // "Bitwise results are unsigned".
                let (start, text) = match op {
                    0 | 1 => (a, format!("{r} %= {b};")),
                    _ => {
                        self.abs = true;
                        let (op, m) = match op {
                            2 => ("|=", 256),
                            3 => ("^=", 256),
                            4 => ("&=", 256),
                            5 => ("<<=", 9),
                            _ => (">>=", 9),
                        };
                        (format!("abs({a}) % 256"), format!("{r} {op} abs({b}) % {m};"))
                    }
                };
                self.line(&format!("let {r}: number = {start};"));
                self.line(&text);
                // Two negative operands give JavaScript `-0` for a zero remainder, where Luau gives `0`.
                let zero = if *op == 1 { " + 0" } else { "" };
                self.line(&format!("trace({r}{zero}, \"n\");"));
            }
            Stmt::AddBool(s, v) => {
                let r = self.fresh("r");
                let (s, v) = (self.e(s), self.e(v));
                self.line(&format!("let {r}: string = {s};"));
                self.line(&format!("{r} += {v};"));
                self.line(&format!("trace({r}, \"s\");"));
            }
            Stmt::BitAssign(k, ushr, m) => {
                if let Some(name) = self.resolve(Ty::Int, *k, Need::Write) {
                    // `&` by a mask that is not negative and `>>>` agree for every integer: known.md, "Bitwise results
                    // are unsigned".
                    let text = if *ushr { format!("{name} >>>= {};", m % 9) } else { format!("{name} &= {m};") };
                    self.line(&text);
                }
            }
            Stmt::Logical(op, ty, k, v) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Write) {
                    let op = match op {
                        Logical::Coalesce => "??=",
                        Logical::Or => "||=",
                        Logical::And => "&&=",
                    };
                    // TypeScript reads the target as `never` on the right of a `??=` once it has narrowed it to a value,
                    // so `(x ?? "a") + 1` there is a number.
                    let hidden = op == "??=";
                    if hidden {
                        self.tdz.push(name.clone());
                    }
                    let v = self.e(v);
                    if hidden {
                        self.tdz.pop();
                    }
                    self.line(&format!("{name} {op} {v};"));
                }
            }
            Stmt::Inc(op, k) => {
                if let Some(name) = self.resolve(Ty::Int, *k, Need::Write) {
                    let text = match op {
                        Step::PreInc => format!("++{name};"),
                        Step::PostInc => format!("{name}++;"),
                        Step::PreDec => format!("--{name};"),
                        Step::PostDec => format!("{name}--;"),
                    };
                    self.line(&text);
                }
            }
            Stmt::Trace(ty, e) => {
                let shape = self.shape(*ty);
                let e = self.e(e);
                self.line(&format!("trace({e}, \"{shape}\");"));
            }
            Stmt::Expr(e) => {
                // An assignment or increment whose variable is gone would leave a bare value, which is no statement.
                let target = match e {
                    Expr::Inc(_, k) => Some((Ty::Int, *k)),
                    Expr::Assign(ty, k, _) | Expr::AddAssign(ty, k, _) => Some((*ty, *k)),
                    _ => None,
                };
                if target.is_some_and(|(ty, _)| self.candidates(ty, Need::Write).is_empty()) {
                    return;
                }
                let text = self.e(e);
                self.line(&format!("{text};"));
            }
            Stmt::Mutate(m) => self.mutation(m),
            Stmt::If(c, then, otherwise) => self.if_chain(c, then, otherwise.as_deref(), "if"),
            Stmt::For { count, step, down, capture, body } => {
                let i = self.fresh("i");
                let closure = capture.then(|| self.fresh("c"));
                if let Some(c) = &closure {
                    let n = self.fresh("x");
                    self.line(&format!("let {c}: (n: number) => number = ({n}: number): number => {n};"));
                }
                let head = match (down, step) {
                    (false, 1) => format!("for (let {i} = 0; {i} < {count}; {i}++) {{"),
                    (false, _) => format!("for (let {i} = 0; {i} < {count}; {i} += {step}) {{"),
                    (true, 1) => format!("for (let {i} = {count}; {i} > 0; {i}--) {{"),
                    (true, _) => format!("for (let {i} = {count}; {i} > 0; {i} -= {step}) {{"),
                };
                self.line(&head);
                self.open();
                self.declare(i.clone(), Ty::Int, true, true, false);
                // Each pass has its own `i`, which the closure keeps after the pass ends.
                let first: Vec<String> = closure
                    .iter()
                    .map(|c| {
                        let n = self.fresh("x");
                        format!("{c} = ({n}: number): number => {n} * 10 + {i};")
                    })
                    .collect();
                self.body_after(&first, &[&i], body, None);
                self.close();
                self.line("}");
                if let Some(c) = closure {
                    self.line(&format!("trace({c}(1), \"n\");"));
                    self.declare(c, Ty::FnII, true, false, false);
                }
            }
            Stmt::ForIndex { ty, source, down, peek, cull, body } => {
                // A cull changes a variable it may change, or else a copy that no other statement names.
                let mutable = cull.as_ref().and_then(|_| self.resolve(*ty, *source, Need::Mutate));
                let xs = match mutable {
                    Some(name) => name,
                    None => {
                        let xs = match self.resolve(*ty, *source, Need::Read) {
                            Some(name) => name,
                            None => self.default(*ty),
                        };
                        match cull {
                            Some((_, _, extra)) => {
                                let copy = self.fresh("c");
                                let mut items = vec![format!("...{xs}")];
                                items.extend(extra.iter().map(|x| self.e(x)));
                                self.line(&format!("const {copy}: {} = [{}];", ty_text(*ty), items.join(", ")));
                                copy
                            }
                            None => xs,
                        }
                    }
                };
                let culled = cull.as_ref().map(|(unordered, m, _)| (xs.clone(), (*unordered, *m)));
                let i = self.fresh("i");
                let head = if *down {
                    format!("for (let {i} = {xs}.size() - 1; {i} >= 0; {i}--) {{")
                } else {
                    format!("for (let {i} = 0; {i} < {xs}.size(); {i}++) {{")
                };
                self.line(&head);
                let mut first = Vec::new();
                if let Some(peek) = peek {
                    let at = match peek {
                        0 => i.clone(),
                        p if *p > 0 => format!("{i} + {p}"),
                        p => format!("{i} - {}", -p),
                    };
                    let (optional, shape) = (ty.optional_element(), self.shape(ty.optional_element()));
                    first.push(format!("trace({xs}[{at}] as {}, \"{shape}\");", ty_text(optional)));
                }
                if let Some((_, (unordered, m))) = culled {
                    let method = if unordered { "unorderedRemove" } else { "remove" };
                    let shape = self.shape(ty.optional_element());
                    first.push(format!("if ({i} % {m} === 0) {{"));
                    first.push(format!("\ttrace({xs}.{method}({i}), \"{shape}\");"));
                    first.push("}".to_string());
                }
                self.open();
                self.declare(i.clone(), Ty::Int, true, true, false);
                self.body_after(&first, &[&i, &xs], body, Some(*ty));
                self.close();
                self.line("}");
                if culled.is_some() {
                    let shape = self.shape(*ty);
                    self.line(&format!("trace({xs}, \"{shape}\");"));
                }
            }
            Stmt::ForOf { ty, source, destructure, body } => {
                // The cast keeps TypeScript from inferring the element from a body that assigns what the source reads.
                let source = format!("({} as {})", self.e(source), ty_text(*ty));
                let x = self.fresh("x");
                if *destructure {
                    let y = self.fresh("x");
                    self.line(&format!("for (const {{ a: {x}, s: {y} }} of {source}) {{"));
                    self.open();
                    self.declare(x, Ty::Int, false, true, false);
                    self.declare(y, Ty::Str, false, true, false);
                } else {
                    self.line(&format!("for (const {x} of {source}) {{"));
                    self.open();
                    self.declare(x, ty.element(), false, true, false);
                }
                self.frozen_block(body, Some(*ty));
                self.close();
                self.line("}");
            }
            Stmt::ForOfStr(source, body) => {
                let source = format!("({} as string)", self.e(source));
                let x = self.fresh("x");
                self.line(&format!("for (const {x} of {source}) {{"));
                self.open();
                self.declare(x, Ty::Str, false, true, false);
                self.frozen_block(body, None);
                self.close();
                self.line("}");
            }
            Stmt::Aggregate { ty, source, visit, term } => self.aggregate(*ty, *source, *visit, term),
            Stmt::ForEach { ty, source, body } => {
                let source = self.recv(source);
                let (x, j) = (self.fresh("x"), self.fresh("x"));
                self.line(&format!("{source}.forEach(({x}, {j}) => {{"));
                // The parameters and the body share one scope, so the body may not hide them.
                self.level += 1;
                self.open();
                self.declare(x, ty.element(), false, true, false);
                self.declare(j, Ty::Int, false, true, false);
                self.frozen.push(*ty);
                self.indent += 1;
                self.stmts(body);
                self.indent -= 1;
                self.frozen.pop();
                self.close();
                self.level -= 1;
                self.line("});");
            }
            Stmt::While { count, cond, body } => {
                let g = self.fresh("g");
                self.line(&format!("let {g} = 0;"));
                self.declare(g.clone(), Ty::Int, true, true, false);
                let cond = self.e(cond);
                self.line(&format!("while ({g} < {count} && {cond}) {{"));
                self.line(&format!("\t{g}++;"));
                self.guarded_block(body, &g);
                self.line("}");
            }
            Stmt::DoWhile { count, cond, shadow, body } => {
                let g = self.fresh("g");
                self.line(&format!("let {g} = 0;"));
                self.declare(g.clone(), Ty::Int, true, true, false);
                // The outer `w` is not declared in a frame, so nothing else reads or assigns it.
                let w = shadow.then(|| self.fresh("w"));
                if let Some(w) = &w {
                    self.line(&format!("let {w}: number = 1;"));
                }
                self.line("do {");
                self.line(&format!("\t{g}++;"));
                let ended = self.guarded_block(body, &g);
                if let Some(w) = w.as_ref().filter(|_| !ended) {
                    self.line(&format!("\tconst {w} = 2;"));
                    self.line(&format!("\ttrace({w}, \"n\");"));
                }
                let head = match &w {
                    Some(w) => format!("{g} < {count} && {w} === 1"),
                    None => format!("{g} < {count}"),
                };
                match cond {
                    // Printed after the body, whose variables it does not see.
                    Some(cond) => {
                        let cond = self.e(cond);
                        self.line(&format!("}} while ({head} && {cond});"));
                    }
                    None => self.line(&format!("}} while ({head});")),
                }
            }
            Stmt::Break => self.line("break;"),
            Stmt::Continue => self.line("continue;"),
            Stmt::Switch { disc, cases, default } => {
                let disc = self.e(disc);
                self.line(&format!("switch ({disc} as number) {{"));
                self.indent += 1;
                let mut seen = HashSet::new();
                for (value, label, body, brk) in cases {
                    let label = match label {
                        Some(label) => self.operand(label, 0),
                        None if seen.insert(*value) => value.to_string(),
                        None => continue,
                    };
                    self.line(&format!("case {label}: {{"));
                    let ended = self.block(body);
                    if *brk && !ended {
                        self.line("\tbreak;");
                    }
                    self.line("}");
                }
                if let Some(body) = default {
                    self.line("default: {");
                    self.block(body);
                    self.line("}");
                }
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Try { body, catch, finally } => {
                self.line("try {");
                self.block(body);
                match catch {
                    Some((true, body)) => {
                        let e = self.fresh("e");
                        self.line(&format!("}} catch ({e}) {{"));
                        self.line(&format!("\ttrace({e}, \"x\");"));
                        self.block(body);
                    }
                    Some((false, body)) => {
                        self.line("} catch {");
                        self.block(body);
                    }
                    None if finally.is_none() => self.line("} catch {"),
                    None => {}
                }
                if let Some(body) = finally {
                    self.line("} finally {");
                    self.block(body);
                }
                self.line("}");
            }
            Stmt::Throw(e) => {
                let e = self.e(e);
                self.line(&format!("throw {e};"));
            }
            Stmt::Return(e) => match e {
                Some(e) => {
                    let e = self.e(e);
                    self.line(&format!("return {e};"));
                }
                None => self.line("return;"),
            },
            Stmt::DestructureArray { ty, source, elements, rest } => {
                if elements.is_empty() && !rest {
                    return;
                }
                let source = self.e(source);
                let element = ty.element();
                let mut parts = Vec::new();
                let mut names = Vec::new();
                for e in elements {
                    match e {
                        None => parts.push(String::new()),
                        Some(d) => {
                            let name = self.fresh("v");
                            let default = match element {
                                Ty::Int => number(INTS[*d as usize % INTS.len()]),
                                Ty::Bool => (d % 2 == 0).to_string(),
                                _ => quote(STRS[*d as usize % STRS.len()], 0),
                            };
                            parts.push(format!("{name} = {default}"));
                            names.push((name, element, false));
                        }
                    }
                }
                if *rest {
                    let name = self.fresh("v");
                    parts.push(format!("...{name}"));
                    names.push((name, *ty, true));
                }
                // A trailing hole needs its comma written twice.
                let pattern = if parts.last().is_some_and(String::is_empty) {
                    format!("{},", parts.join(", "))
                } else {
                    parts.join(", ")
                };
                self.line(&format!("const [{pattern}] = {source} as {};", ty_text(*ty)));
                for (name, ty, fresh) in names {
                    self.declare(name, ty, false, false, fresh);
                }
            }
            Stmt::DestructureObject { ty, source, fields, default } => {
                if fields.is_empty() {
                    return;
                }
                let source = self.e(source);
                let mut parts = Vec::new();
                let mut inner = Vec::new();
                let mut names = Vec::new();
                for f in fields {
                    let name = self.fresh("v");
                    let (text, ty) = match f {
                        Field::A => (format!("a: {name}"), Ty::Int),
                        Field::S => (format!("s: {name}"), Ty::Str),
                        Field::End => (format!("end: {name}"), Ty::Int),
                        Field::Then => {
                            let d = quote(STRS[*default as usize % STRS.len()], 0);
                            (format!("then: {name} = {d}"), Ty::Str)
                        }
                        Field::InnerA => {
                            inner.push(format!("a: {name}"));
                            names.push((name, Ty::Int));
                            continue;
                        }
                        Field::InnerS => {
                            inner.push(format!("s: {name}"));
                            names.push((name, Ty::Str));
                            continue;
                        }
                        Field::Inner => continue,
                    };
                    parts.push(text);
                    names.push((name, ty));
                }
                if !inner.is_empty() {
                    parts.push(format!("inner: {{ {} }}", inner.join(", ")));
                }
                // The cast keeps TypeScript from checking an object literal's members against the pattern.
                self.line(&format!("const {{ {} }} = {source} as {};", parts.join(", "), ty_text(*ty)));
                for (name, ty) in names {
                    self.declare(name, ty, false, false, false);
                }
            }
            Stmt::Swap(a, b) => {
                let found = self.candidates(Ty::Int, Need::Write);
                if found.len() < 2 {
                    return;
                }
                let x = found[*a as usize % found.len()].name.clone();
                let y = found[*b as usize % found.len()].name.clone();
                if x != y {
                    self.mark(&x);
                    self.mark(&y);
                    self.line(&format!("[{x}, {y}] = [{y}, {x}];"));
                }
            }
            Stmt::AssignObject(a, s, o) => {
                let ints = self.candidates(Ty::Int, Need::Write);
                let strs = self.candidates(Ty::Str, Need::Write);
                if ints.is_empty() || strs.is_empty() {
                    return;
                }
                let x = ints[*a as usize % ints.len()].name.clone();
                let y = strs[*s as usize % strs.len()].name.clone();
                self.mark(&x);
                self.mark(&y);
                let o = self.e(o);
                self.line(&format!("({{ a: {x}, s: {y} }} = {o} as {});", ty_text(Ty::Obj)));
            }
            Stmt::Block(body) => {
                self.line("{");
                self.block(body);
                self.line("}");
            }
            Stmt::LocalFunction(body, ret) => {
                let name = self.fresh("h");
                let a = self.fresh("a");
                self.line(&format!("function {name}({a}: number): number {{"));
                self.level += 1;
                self.closures += 1;
                let saved_frozen = std::mem::take(&mut self.frozen);
                self.indent += 1;
                self.open();
                self.declare(a, Ty::Int, true, true, false);
                if !self.stmts(body) {
                    let ret = self.e(ret);
                    self.line(&format!("return {ret};"));
                }
                self.close();
                self.indent -= 1;
                self.frozen = saved_frozen;
                self.closures -= 1;
                self.level -= 1;
                self.line("}");
                self.declare(name, Ty::FnII, false, false, false);
            }
        }
    }

    fn if_chain(&mut self, c: &Expr, then: &[Stmt], otherwise: Option<&[Stmt]>, keyword: &str) {
        let text = self.e(c);
        let text = self.opaque(text, c);
        let prefix = if keyword == "if" { "if" } else { "} else if" };
        self.line(&format!("{prefix} ({text}) {{"));
        self.block(then);
        match otherwise {
            Some([Stmt::If(c, then, otherwise)]) => self.if_chain(c, then, otherwise.as_deref(), "else if"),
            Some(otherwise) => {
                self.line("} else {");
                self.block(otherwise);
                self.line("}");
            }
            None => self.line("}"),
        }
    }

    fn mutation(&mut self, m: &Mutation) {
        match m {
            Mutation::Push(ty, k, values) | Mutation::Unshift(ty, k, values) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Mutate) {
                    let method = if matches!(m, Mutation::Push(..)) { "push" } else { "unshift" };
                    let values = self.list(values);
                    self.line(&format!("{name}.{method}({values});"));
                }
            }
            Mutation::Pop(ty, k) | Mutation::Shift(ty, k) | Mutation::Clear(ty, k) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Mutate) {
                    let method = match m {
                        Mutation::Pop(..) => "pop",
                        Mutation::Shift(..) => "shift",
                        _ => "clear",
                    };
                    self.line(&format!("{name}.{method}();"));
                }
            }
            Mutation::SetIndex(ty, k, i, v) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Mutate) {
                    let v = self.e(v);
                    self.line(&format!("if ({name}.size() > {i}) {{"));
                    self.line(&format!("\t{name}[{i}] = {v};"));
                    self.line("}");
                }
            }
            Mutation::Append(ty, k, v) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Mutate) {
                    let v = self.e(v);
                    self.line(&format!("{name}[{name}.size()] = {v};"));
                }
            }
            Mutation::MapSet(ty, k, key, v) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Mutate) {
                    let (key, v) = (self.e(key), self.e(v));
                    self.line(&format!("{name}.set({key}, {v});"));
                }
            }
            Mutation::Delete(ty, k, key) | Mutation::SetAdd(ty, k, key) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Mutate) {
                    let method = if matches!(m, Mutation::Delete(..)) { "delete" } else { "add" };
                    let key = self.e(key);
                    self.line(&format!("{name}.{method}({key});"));
                }
            }
            Mutation::Field(ty, f, k, v, compound) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Read) {
                    let op = if *compound { "+=" } else { "=" };
                    let v = self.e(v);
                    let place = format!("{name}{}", field(*f));
                    match f {
                        Field::S | Field::InnerS => self.line(&format!("if ({}) {place} {op} {v};", short(&place))),
                        Field::Then => {
                            self.line(&format!("if ({}) {place} {op} {v};", short(&format!("({place} ?? \"\")"))))
                        }
                        _ => self.line(&format!("{place} {op} {v};")),
                    }
                }
            }
            Mutation::FieldInc(ty, f, k, op) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Read) {
                    let place = format!("{name}{}", field(*f));
                    let text = match op {
                        Step::PreInc => format!("++{place};"),
                        Step::PostInc => format!("{place}++;"),
                        Step::PreDec => format!("--{place};"),
                        Step::PostDec => format!("{place}--;"),
                    };
                    self.line(&text);
                }
            }
            Mutation::IndexAdd(k, i, v) => {
                if let Some(name) = self.resolve(Ty::ArrInt, *k, Need::Mutate) {
                    let v = self.e(v);
                    self.line(&format!("if ({name}.size() > {i}) {{"));
                    self.line(&format!("\t{name}[{i}] += {v};"));
                    self.line("}");
                }
            }
            Mutation::WriteAt(ty, k, i, write, v) => {
                if let Some(xs) = self.resolve(*ty, *k, Need::Mutate) {
                    let i = self.operand(i, INDEX);
                    // The value may not change an array of this type, which could move the index past the end.
                    self.frozen.push(*ty);
                    let (bound, text) = match write {
                        Write::Set => ("<=", format!("{xs}[{i}] = {};", self.e(v))),
                        Write::Add => ("<", format!("{xs}[{i}] += {};", self.e(v))),
                        Write::Inc(Step::PreInc) => ("<", format!("++{xs}[{i}];")),
                        Write::Inc(Step::PostInc) => ("<", format!("{xs}[{i}]++;")),
                        Write::Inc(Step::PreDec) => ("<", format!("--{xs}[{i}];")),
                        Write::Inc(Step::PostDec) => ("<", format!("{xs}[{i}]--;")),
                        Write::Logical(Logical::Or) => ("<", format!("{xs}[{i}] ||= {};", self.e(v))),
                        Write::Logical(_) => ("<", format!("{xs}[{i}] &&= {};", self.e(v))),
                    };
                    self.frozen.pop();
                    let short = if *ty == Ty::ArrStr && *write == Write::Add {
                        format!(" && {}", short(&format!("{xs}[{i}]")))
                    } else {
                        String::new()
                    };
                    self.line(&format!("if ({i} >= 0 && {i} {bound} {xs}.size(){short}) {{"));
                    self.line(&format!("\t{text}"));
                    self.line("}");
                }
            }
            Mutation::Insert(ty, k, i, v) => {
                if let Some(xs) = self.resolve(*ty, *k, Need::Mutate) {
                    let i = self.operand(i, INDEX);
                    self.frozen.push(*ty);
                    let v = self.e(v);
                    self.frozen.pop();
                    self.line(&format!("if ({i} >= 0 && {i} <= {xs}.size()) {{"));
                    self.line(&format!("\t{xs}.insert({i}, {v});"));
                    self.line("}");
                }
            }
            Mutation::Remove(ty, k, i, unordered) => {
                if let Some(xs) = self.resolve(*ty, *k, Need::Mutate) {
                    let i = self.operand(i, INDEX);
                    let method = if *unordered { "unorderedRemove" } else { "remove" };
                    self.line(&format!("if ({i} >= 0 && {i} < {xs}.size()) {{"));
                    self.line(&format!("\t{xs}.{method}({i});"));
                    self.line("}");
                }
            }
            Mutation::FieldBit(ty, f, k, ushr, m) => {
                if let Some(name) = self.resolve(*ty, *k, Need::Read) {
                    let place = format!("{name}{}", field(*f));
                    let text = if *ushr { format!("{place} >>>= {};", m % 9) } else { format!("{place} &= {m};") };
                    self.line(&text);
                }
            }
            Mutation::ThenCoalesce(k, v) => {
                if let Some(name) = self.resolve(Ty::Rec, *k, Need::Read) {
                    let v = self.e(v);
                    self.line(&format!("{name}.then ??= {v};"));
                }
            }
            Mutation::DeleteThen(k) => {
                if let Some(name) = self.resolve(Ty::Rec, *k, Need::Read) {
                    // Through a cast, so TypeScript does not narrow `name.then` to `undefined` for the rest of the block.
                    self.line(&format!("delete ({name} as {}).then;", ty_text(Ty::Rec)));
                }
            }
        }
    }

    fn aggregate(&mut self, ty: Ty, source: u8, visit: Visit, term: &Expr) {
        let source = match self.resolve(ty, source, Need::Read) {
            Some(name) => name,
            None => self.default(ty),
        };
        let sum = self.fresh("v");
        self.line(&format!("let {sum} = 0;"));
        let (key, value) = match ty {
            Ty::MapSI => (Some(Ty::Str), Ty::Int),
            Ty::MapIS => (Some(Ty::Int), Ty::Str),
            Ty::SetS => (None, Ty::Str),
            _ => (None, Ty::Int),
        };
        let (k, x) = (self.fresh("k"), self.fresh("x"));
        let (head, close, names) = match (visit, key) {
            (Visit::Entries, Some(_)) => (format!("for (const [{k}, {x}] of {source}) {{"), "}", (Some(k), x)),
            (Visit::Values, Some(_)) => (format!("for (const [, {x}] of {source}) {{"), "}", (None, x)),
            (Visit::Whole, Some(_)) => {
                (format!("for (const {x} of {source}) {{"), "}", (Some(format!("{x}[0]")), format!("{x}[1]")))
            }
            (Visit::ForEach, Some(_)) => (format!("{source}.forEach(({x}, {k}) => {{"), "});", (Some(k), x)),
            (Visit::ForEach, None) => (format!("{source}.forEach(({x}) => {{"), "});", (None, x)),
            (_, None) => (format!("for (const {x} of {source}) {{"), "}", (None, x)),
        };
        self.line(&head);
        self.open();
        if let (Some(name), Some(key)) = (names.0, key) {
            self.declare(name, key, false, true, false);
        }
        self.declare(names.1, value, false, true, false);
        self.pure = true;
        let term = self.e(term);
        self.pure = false;
        self.close();
        self.line(&format!("\t{sum} += {term};"));
        self.line(close);
        self.declare(sum, Ty::Int, true, false, false);
    }

    fn function(&mut self, k: usize) {
        let p = self.p;
        let f = &p.functions[k];
        let (params, ret) = SIGNATURES[k];
        let saved_frames = std::mem::replace(&mut self.frames, vec![Frame::default()]);
        self.level = 1;
        let mut texts = Vec::new();
        for (i, &ty) in params.iter().enumerate() {
            let name = self.fresh("a");
            texts.push(match (k, i) {
                (2, 1) => format!("{name}: number = 2"),
                (3, 1) => format!("...{name}: number[]"),
                _ => format!("{name}: {}", ty_text(ty)),
            });
            self.declare(name, ty, true, true, false);
        }
        let first = self.frames[0].vars[0].name.clone();
        self.line(&format!("function f{k}({}): {} {{", texts.join(", "), ty_text(ret)));
        self.indent += 1;
        if k == 4 {
            self.line(&format!("if ({first} <= 0) {{"));
            self.line("\treturn 1;");
            self.line("}");
        }
        if !self.stmts(&f.body) {
            let value = self.e(&f.ret);
            if k == 4 {
                self.line(&format!("return {value} + f4({first} - 1);"));
            } else {
                self.line(&format!("return {value};"));
            }
        }
        self.indent -= 1;
        self.line("}");
        self.level = 0;
        self.frames = saved_frames;
    }

    fn class(&mut self, k: u8) {
        let p = self.p;
        let c = &p.classes[k as usize];
        let saved_frames = std::mem::replace(&mut self.frames, vec![Frame::default()]);
        self.class = Some(k);
        self.level = 1;
        match c.parent {
            None => {
                self.line(&format!("class C{k} {{"));
                self.indent += 1;
                self.line("x: number;");
                self.line(&format!("tag: string = \"C{k}\";"));
                self.line("static count: number = 0;");
                let a = self.fresh("a");
                self.line(&format!("constructor({a}: number) {{"));
                self.line(&format!("\tthis.x = {a} + this.tag.size();"));
                if c.count {
                    self.line(&format!("\tC{k}.count += 1;"));
                }
                self.line("}");
            }
            Some(parent) => {
                self.classes[parent as usize] = true;
                self.line(&format!("class C{k} extends C{parent} {{"));
                self.indent += 1;
                self.line("y: number = 7;");
                if let Some(add) = c.constructor {
                    let a = self.fresh("a");
                    self.line(&format!("constructor({a}: number) {{"));
                    self.line(&format!("\tsuper({a} + {add});"));
                    self.line(&format!("\tthis.y += {a};"));
                    self.line("}");
                }
            }
        }
        if let Some((body, ret)) = &c.method {
            let a = self.fresh("a");
            self.line(&format!("m({a}: number): number {{"));
            self.indent += 1;
            self.declare(a, Ty::Int, true, true, false);
            if !self.stmts(body) {
                let ret = self.e(ret);
                self.line(&format!("return {ret};"));
            }
            self.indent -= 1;
            self.line("}");
        }
        match c.parent {
            None => self.line("describe(): string {\n\t\treturn `${this.tag}:${this.x}`;\n\t}"),
            Some(_) if c.describe => {
                self.line("describe(): string {\n\t\treturn super.describe() + \"/\" + this.y;\n\t}")
            }
            Some(_) => {}
        }
        if k == 0 {
            let a = self.fresh("a");
            self.line(&format!("static make({a}: number): C0 {{"));
            self.line(&format!("\treturn new C0({a});"));
            self.line("}");
        }
        self.indent -= 1;
        self.line("}");
        self.level = 0;
        self.class = None;
        self.frames = saved_frames;
    }
}

impl Expr {
    fn cond(&self) -> &Expr {
        match self {
            Expr::Cond(c, _, _) => c,
            other => other,
        }
    }
}

// TypeScript's precedences of the operators `bare` writes, and the least an operand of a comparison needs.
const COALESCE: u8 = 1;
const OR: u8 = 2;
const AND: u8 = 3;
const INDEX: u8 = 9;
const ADD: u8 = 10;
const MUL: u8 = 11;

fn ts_precedence(x: &Expr) -> Option<u8> {
    Some(match x {
        Expr::Coalesce(..) => COALESCE,
        Expr::Or(Ty::Bool, ..) => OR,
        Expr::And(Ty::Bool, ..) => AND,
        Expr::Add(..) | Expr::Sub(..) | Expr::Concat(..) => ADD,
        Expr::Mul(..) | Expr::Div(..) | Expr::MulK(..) | Expr::Mod(..) => MUL,
        _ => return None,
    })
}

/// Whether TypeScript sees a constant truthiness here, which it rejects in a condition. This follows
/// `getSyntacticTruthySemantics`: literals other than `0`, `1` and the booleans, object and array literals and
/// functions, through casts and through a conditional whose branches both are.
fn constant(e: &Expr) -> bool {
    match e {
        Expr::Int(i) => {
            let text = INTS[*i as usize % INTS.len()];
            !text.starts_with('-') && text != "0" && text != "1"
        }
        Expr::Num(i) => !NUMS[*i as usize % NUMS.len()].starts_with('-'),
        Expr::Str(..) => true,
        Expr::Template(_, parts) => parts.is_empty(),
        Expr::Cast(a, _) => constant(a),
        Expr::Cond(_, a, b) => constant(a) || constant(b),
        Expr::Obj(..)
        | Expr::ObjSpread(..)
        | Expr::ObjSpreadOpt(..)
        | Expr::Rec(..)
        | Expr::RecSpread(..)
        | Expr::Lambda(_) => true,
        Expr::Array(..) => true,
        _ => false,
    }
}

/// Whether the value is a new collection, so a variable it initializes is no alias.
fn makes_new(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Array(..)
            | Expr::Map(..)
            | Expr::Filter(..)
            | Expr::Split(..)
            | Expr::NewMap(..)
            | Expr::NewSet(..)
            | Expr::NewSetFrom(..)
    )
}

/// A truthiness test of a number or string, which CRU032 rejects unless the line before the operand disables it, as
/// code that means it does.
fn truthy(text: String, ty: Ty) -> String {
    if matches!(ty, Ty::Bool | Ty::OptBool) {
        text
    } else {
        format!("\n// oxlint-disable-next-line roblox-ts/lua-truthiness\n{text}")
    }
}

fn is_name(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn field(f: Field) -> &'static str {
    match f {
        Field::A => ".a",
        Field::S => ".s",
        Field::End => ".end",
        Field::InnerA => ".inner.a",
        Field::InnerS => ".inner.s",
        Field::Then => ".then",
        Field::Inner => ".inner",
    }
}

fn number(text: &str) -> String {
    if text.starts_with('-') { format!("({text})") } else { text.to_owned() }
}

fn escape(c: char, out: &mut String) {
    match c {
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
        c => out.push(c),
    }
}

/// A string literal in double quotes, single quotes, or a template without substitutions.
fn quote(value: &str, style: u8) -> String {
    if style % 3 == 2 {
        return format!("`{}`", template_text(value));
    }
    let q = if style.is_multiple_of(3) { '"' } else { '\'' };
    let mut out = String::from(q);
    for c in value.chars() {
        if c == q {
            out.push('\\');
            out.push(c);
        } else {
            escape(c, &mut out);
        }
    }
    out.push(q);
    out
}

fn template_text(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => out.push_str("\\`"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => escape(c, &mut out),
        }
    }
    out
}

/// The condition a string write runs under, so a loop that doubles a string stays under Node's limit on a string's length.
fn short(place: &str) -> String {
    format!("{place}.size() < 256")
}
