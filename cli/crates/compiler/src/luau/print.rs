use std::fmt::Write;

use super::{BinOp, Expr, Field, Function, Part, Stmt, UnOp, is_name};

/// Prints a chunk with tabs, one statement per line.
pub fn print(chunk: &[Stmt]) -> String {
    let mut p = Printer { out: String::with_capacity(4096), depth: 0 };
    p.block(chunk);
    p.out
}

struct Printer {
    out: String,
    depth: usize,
}

const OR: u8 = 1;
const AND: u8 = 2;
const COMPARE: u8 = 3;
const CONCAT: u8 = 4;
const ADD: u8 = 5;
const MUL: u8 = 6;
const UNARY: u8 = 7;
const POW: u8 = 8;
const ATOM: u8 = 9;

fn binary_precedence(op: BinOp) -> (u8, bool) {
    match op {
        BinOp::Or => (OR, false),
        BinOp::And => (AND, false),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => (COMPARE, false),
        BinOp::Concat => (CONCAT, true),
        BinOp::Add | BinOp::Sub => (ADD, false),
        BinOp::Mul | BinOp::Div | BinOp::FloorDiv | BinOp::Mod => (MUL, false),
        BinOp::Pow => (POW, true),
    }
}

fn binary_token(op: BinOp) -> &'static str {
    match op {
        BinOp::Or => "or",
        BinOp::And => "and",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Eq => "==",
        BinOp::Ne => "~=",
        BinOp::Concat => "..",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::FloorDiv => "//",
        BinOp::Mod => "%",
        BinOp::Pow => "^",
    }
}

fn precedence(e: &Expr) -> u8 {
    match e {
        Expr::Binary(op, ..) => binary_precedence(*op).0,
        Expr::Unary(..) => UNARY,
        Expr::Number(n) if n.is_sign_negative() || n.is_nan() => UNARY,
        Expr::If(..) => 0,
        _ => ATOM,
    }
}

/// Whether the expression can be called or indexed without parentheses.
fn is_prefix(e: &Expr) -> bool {
    matches!(e, Expr::Name(_) | Expr::Index(..) | Expr::Call(..) | Expr::Method(..) | Expr::Paren(_))
}

impl Printer {
    fn line(&mut self) {
        self.out.push('\n');
        for _ in 0..self.depth {
            self.out.push('\t');
        }
    }

    fn block(&mut self, block: &[Stmt]) {
        for (i, s) in block.iter().enumerate() {
            if i > 0 || !self.out.is_empty() {
                self.line();
            }
            let last = i + 1 == block.len();
            let terminal = matches!(s, Stmt::Return(_) | Stmt::Break | Stmt::Continue);
            if terminal && !last {
                self.out.push_str("do ");
                self.stmt(s);
                self.out.push_str(" end");
            } else {
                self.stmt(s);
            }
        }
    }

    fn body(&mut self, block: &[Stmt]) {
        self.depth += 1;
        let start = self.out.len();
        self.block(block);
        if self.out.len() == start {
            self.depth -= 1;
            self.out.push(' ');
            return;
        }
        self.depth -= 1;
        self.line();
    }

    fn names(&mut self, names: &[super::Name]) {
        for (i, n) in names.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.out.push_str(n);
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) {
        for (i, e) in exprs.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.expr(e, 0);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Local(names, values) => {
                self.out.push_str("local ");
                self.names(names);
                if !values.is_empty() {
                    self.out.push_str(" = ");
                    self.exprs(values);
                }
            }
            Stmt::LocalFunction(name, f) => {
                self.out.push_str("local function ");
                self.out.push_str(name);
                self.function(f);
            }
            Stmt::Function(target, method, f) => {
                if !self.is_path(target) || method.as_ref().is_some_and(|m| !is_name(m)) {
                    let mut f = f.clone();
                    let target = match method {
                        Some(m) => {
                            f.params.insert(0, "self".into());
                            target.clone().field(m.clone())
                        }
                        None => target.clone(),
                    };
                    let value = Expr::Function(Box::new(f));
                    return self.stmt(&Stmt::Assign(vec![target], vec![value]));
                }
                self.out.push_str("function ");
                self.path(target);
                if let Some(m) = method {
                    self.out.push(':');
                    self.out.push_str(m);
                }
                self.function(f);
            }
            Stmt::Assign(targets, values) => {
                if let Some(first) = targets.first() {
                    self.statement_start(first);
                }
                self.exprs(targets);
                self.out.push_str(" = ");
                self.exprs(values);
            }
            Stmt::Compound(op, target, value) => {
                self.statement_start(target);
                self.expr(target, 0);
                self.out.push(' ');
                self.out.push_str(binary_token(*op));
                self.out.push_str("= ");
                self.expr(value, 0);
            }
            Stmt::Call(e) => {
                self.statement_start(e);
                self.expr(e, 0);
            }
            Stmt::Do(block) => {
                self.out.push_str("do");
                self.body(block);
                self.out.push_str("end");
            }
            Stmt::If(condition, then, otherwise) => {
                self.out.push_str("if ");
                self.expr(condition, 0);
                self.out.push_str(" then");
                self.body(then);
                let mut otherwise = otherwise.as_ref();
                while let Some(block) = otherwise {
                    if let [Stmt::If(c, t, o)] = block.as_slice() {
                        self.out.push_str("elseif ");
                        self.expr(c, 0);
                        self.out.push_str(" then");
                        self.body(t);
                        otherwise = o.as_ref();
                    } else {
                        self.out.push_str("else");
                        self.body(block);
                        break;
                    }
                }
                self.out.push_str("end");
            }
            Stmt::While(condition, block) => {
                self.out.push_str("while ");
                self.expr(condition, 0);
                self.out.push_str(" do");
                self.body(block);
                self.out.push_str("end");
            }
            Stmt::Repeat(block, until) => {
                self.out.push_str("repeat");
                self.body(block);
                self.out.push_str("until ");
                self.expr(until, 0);
            }
            Stmt::NumericFor(name, start, end, step, block) => {
                self.out.push_str("for ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.expr(start, 0);
                self.out.push_str(", ");
                self.expr(end, 0);
                if let Some(step) = step {
                    self.out.push_str(", ");
                    self.expr(step, 0);
                }
                self.out.push_str(" do");
                self.body(block);
                self.out.push_str("end");
            }
            Stmt::GenericFor(names, iter, block) => {
                self.out.push_str("for ");
                self.names(names);
                self.out.push_str(" in ");
                self.exprs(iter);
                self.out.push_str(" do");
                self.body(block);
                self.out.push_str("end");
            }
            Stmt::Return(values) => {
                self.out.push_str("return");
                if !values.is_empty() {
                    self.out.push(' ');
                    self.exprs(values);
                }
            }
            Stmt::Break => self.out.push_str("break"),
            Stmt::Continue => self.out.push_str("continue"),
        }
    }

    /// A statement that starts with `(` would continue the previous line's call, so it starts with `;` instead.
    fn statement_start(&mut self, e: &Expr) {
        let mut head = e;
        while let Expr::Index(base, _) | Expr::Call(base, _) | Expr::Method(base, ..) = head {
            head = base;
        }
        if !is_prefix(head) || matches!(head, Expr::Paren(_)) {
            self.out.push(';');
        }
    }

    fn is_path(&self, e: &Expr) -> bool {
        match e {
            Expr::Name(_) => true,
            Expr::Index(base, key) => matches!(&**key, Expr::String(s) if is_name(s)) && self.is_path(base),
            _ => false,
        }
    }

    fn path(&mut self, e: &Expr) {
        match e {
            Expr::Name(n) => self.out.push_str(n),
            Expr::Index(base, key) => {
                self.path(base);
                if let Expr::String(s) = &**key {
                    self.out.push('.');
                    self.out.push_str(s);
                }
            }
            _ => unreachable!("checked by is_path"),
        }
    }

    fn function(&mut self, f: &Function) {
        self.out.push('(');
        self.names(&f.params);
        if f.varargs {
            if !f.params.is_empty() {
                self.out.push_str(", ");
            }
            self.out.push_str("...");
        }
        self.out.push(')');
        if f.body.is_empty() {
            self.out.push_str(" end");
            return;
        }
        self.body(&f.body);
        self.out.push_str("end");
    }

    fn prefix(&mut self, e: &Expr) {
        if is_prefix(e) {
            self.expr(e, 0);
        } else {
            self.out.push('(');
            self.expr(e, 0);
            self.out.push(')');
        }
    }

    fn expr(&mut self, e: &Expr, min: u8) {
        if precedence(e) < min {
            self.out.push('(');
            self.expr(e, 0);
            self.out.push(')');
            return;
        }
        match e {
            Expr::Nil => self.out.push_str("nil"),
            Expr::Bool(b) => self.out.push_str(if *b { "true" } else { "false" }),
            Expr::Number(n) => self.number(*n),
            Expr::String(s) => self.string(s),
            Expr::VarArgs => self.out.push_str("..."),
            Expr::Name(n) => self.out.push_str(n),
            Expr::Index(base, key) => {
                self.prefix(base);
                match &**key {
                    Expr::String(s) if is_name(s) => {
                        self.out.push('.');
                        self.out.push_str(s);
                    }
                    key => {
                        self.out.push('[');
                        self.expr(key, 0);
                        self.out.push(']');
                    }
                }
            }
            Expr::Call(callee, args) => {
                self.prefix(callee);
                self.out.push('(');
                self.exprs(args);
                self.out.push(')');
            }
            Expr::Method(base, name, args) => {
                self.prefix(base);
                self.out.push(':');
                self.out.push_str(name);
                self.out.push('(');
                self.exprs(args);
                self.out.push(')');
            }
            Expr::Function(f) => {
                self.out.push_str("function");
                self.function(f);
            }
            Expr::Table(fields) => self.table(fields),
            Expr::Binary(op, left, right) => {
                let (p, right_assoc) = binary_precedence(*op);
                let (l, r) = if right_assoc { (p + 1, p) } else { (p, p + 1) };
                self.expr(left, l);
                self.out.push(' ');
                self.out.push_str(binary_token(*op));
                self.out.push(' ');
                self.expr(right, r);
            }
            Expr::Unary(op, operand) => {
                self.out.push_str(match op {
                    UnOp::Not => "not ",
                    UnOp::Neg => "-",
                    UnOp::Len => "#",
                });
                self.expr(operand, if *op == UnOp::Neg { UNARY + 1 } else { UNARY });
            }
            Expr::If(condition, then, otherwise) => {
                self.out.push_str("if ");
                self.expr(condition, 0);
                self.out.push_str(" then ");
                self.expr(then, 0);
                self.out.push_str(" else ");
                self.expr(otherwise, 0);
            }
            Expr::Interpolated(parts) => self.interpolated(parts),
            Expr::Paren(inner) => {
                self.out.push('(');
                self.expr(inner, 0);
                self.out.push(')');
            }
        }
    }

    fn number(&mut self, n: f64) {
        if n.is_nan() {
            self.out.push_str("0 / 0");
        } else if n.is_infinite() {
            self.out.push_str(if n > 0.0 { "math.huge" } else { "-math.huge" });
        } else if n == n.trunc() && n.abs() < 9.007_199_254_740_992e15 {
            if n == 0.0 && n.is_sign_negative() {
                self.out.push_str("-0");
            } else {
                let _ = write!(self.out, "{}", n as i64);
            }
        } else if n.abs() >= 1e21 || n.abs() < 1e-7 {
            let _ = write!(self.out, "{n:e}");
        } else {
            let _ = write!(self.out, "{n}");
        }
    }

    fn string(&mut self, s: &str) {
        self.out.push('"');
        for c in s.chars() {
            match c {
                '"' => self.out.push_str("\\\""),
                _ => self.escape(c),
            }
        }
        self.out.push('"');
    }

    fn escape(&mut self, c: char) {
        match c {
            '\\' => self.out.push_str("\\\\"),
            '\n' => self.out.push_str("\\n"),
            '\r' => self.out.push_str("\\r"),
            '\t' => self.out.push_str("\\t"),
            '\0'..'\u{20}' | '\u{7f}' => {
                let _ = write!(self.out, "\\{:03}", c as u32);
            }
            _ => self.out.push(c),
        }
    }

    fn interpolated(&mut self, parts: &[Part]) {
        self.out.push('`');
        for part in parts {
            match part {
                Part::Text(text) => {
                    for c in text.chars() {
                        match c {
                            '`' => self.out.push_str("\\`"),
                            '{' => self.out.push_str("\\{"),
                            _ => self.escape(c),
                        }
                    }
                }
                Part::Expr(e) => {
                    self.out.push('{');
                    if matches!(e, Expr::Table(_)) {
                        self.out.push('(');
                        self.expr(e, 0);
                        self.out.push(')');
                    } else {
                        self.expr(e, 0);
                    }
                    self.out.push('}');
                }
            }
        }
        self.out.push('`');
    }

    fn table(&mut self, fields: &[Field]) {
        if fields.is_empty() {
            self.out.push_str("{}");
            return;
        }
        if fields.iter().all(|f| matches!(f, Field::Item(_))) {
            self.out.push_str("{ ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    self.out.push_str(", ");
                }
                if let Field::Item(e) = f {
                    self.expr(e, 0);
                }
            }
            self.out.push_str(" }");
            return;
        }
        self.out.push('{');
        self.depth += 1;
        for f in fields {
            self.line();
            match f {
                Field::Item(e) => self.expr(e, 0),
                Field::Keyed(Expr::String(key), value) if is_name(key) => {
                    self.out.push_str(key);
                    self.out.push_str(" = ");
                    self.expr(value, 0);
                }
                Field::Keyed(key, value) => {
                    self.out.push('[');
                    self.expr(key, 0);
                    self.out.push_str("] = ");
                    self.expr(value, 0);
                }
            }
            self.out.push(',');
        }
        self.depth -= 1;
        self.line();
        self.out.push('}');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::luau::Function;

    fn expr(e: Expr) -> String {
        print(&[Stmt::Return(vec![e])]).trim_start_matches("return ").to_owned()
    }

    fn n(v: f64) -> Expr {
        Expr::Number(v)
    }

    #[test]
    fn parentheses_follow_precedence_and_associativity() {
        let sum = n(1.0).binary(BinOp::Add, n(2.0));
        assert_eq!(expr(sum.clone().binary(BinOp::Mul, n(3.0))), "(1 + 2) * 3");
        assert_eq!(expr(n(3.0).binary(BinOp::Sub, sum.clone())), "3 - (1 + 2)");
        assert_eq!(expr(sum.clone().binary(BinOp::Sub, n(3.0))), "1 + 2 - 3");
        let concat = Expr::string("a").binary(BinOp::Concat, Expr::string("b"));
        assert_eq!(expr(concat.clone().binary(BinOp::Concat, Expr::string("c"))), "(\"a\" .. \"b\") .. \"c\"");
        assert_eq!(expr(Expr::string("c").binary(BinOp::Concat, concat)), "\"c\" .. \"a\" .. \"b\"");
        assert_eq!(expr(Expr::unary(UnOp::Neg, n(2.0)).binary(BinOp::Pow, n(2.0))), "(-2)^2".replace('^', " ^ "));
        assert_eq!(expr(Expr::unary(UnOp::Neg, n(-1.0))), "-(-1)");
        assert_eq!(expr(Expr::unary(UnOp::Not, Expr::name("a").binary(BinOp::And, Expr::name("b")))), "not (a and b)");
        let choice = Expr::if_else(Expr::name("c"), n(1.0), n(2.0));
        assert_eq!(expr(choice.clone().binary(BinOp::Add, n(1.0))), "(if c then 1 else 2) + 1");
        assert_eq!(expr(choice), "if c then 1 else 2");
    }

    #[test]
    fn prefix_expressions_wrap_non_prefix_bases() {
        assert_eq!(expr(Expr::string("x").method("rep", vec![n(2.0)])), "(\"x\"):rep(2)");
        assert_eq!(expr(Expr::Table(Vec::new()).field("a")), "({}).a");
        assert_eq!(expr(Expr::name("t").field("end").field("ok")), "t[\"end\"].ok");
    }

    #[test]
    fn literals_print_as_luau() {
        assert_eq!(expr(n(f64::NAN)), "0 / 0");
        assert_eq!(expr(n(f64::INFINITY)), "math.huge");
        assert_eq!(expr(n(0.5)), "0.5");
        assert_eq!(expr(n(1e300)), "1e300");
        assert_eq!(expr(Expr::string("a\"b\\c\n\u{1}é")), "\"a\\\"b\\\\c\\n\\001é\"");
        let parts =
            vec![Part::Text("{a} `b`".into()), Part::Expr(Expr::Table(Vec::new())), Part::Expr(Expr::name("x"))];
        assert_eq!(expr(Expr::Interpolated(parts)), "`\\{a} \\`b\\`{({})}{x}`");
        let table = Expr::Table(vec![
            Field::Keyed(Expr::string("end"), n(1.0)),
            Field::Keyed(Expr::string("ok"), n(2.0)),
            Field::Keyed(n(3.0), n(4.0)),
        ]);
        assert_eq!(expr(table), "{\n\t[\"end\"] = 1,\n\tok = 2,\n\t[3] = 4,\n}");
    }

    #[test]
    fn statements_keep_luau_block_rules() {
        let block = vec![
            Stmt::Return(Vec::new()),
            Stmt::Call(Expr::name("f").call(Vec::new())),
            Stmt::Call(Expr::Paren(Box::new(Expr::name("g"))).call(Vec::new())),
            Stmt::Do(Vec::new()),
            Stmt::If(
                Expr::name("a"),
                vec![Stmt::Break],
                Some(vec![Stmt::If(Expr::name("b"), Vec::new(), Some(vec![Stmt::Continue]))]),
            ),
        ];
        assert_eq!(
            print(&block),
            "do return end\nf()\n;(g)()\ndo end\nif a then\n\tbreak\nelseif b then else\n\tcontinue\nend"
        );
        let method = Function { params: vec!["x".into()], varargs: true, body: Vec::new() };
        assert_eq!(
            print(&[Stmt::Function(Expr::name("C"), Some("end".into()), method.clone())]),
            "C[\"end\"] = function(self, x, ...) end"
        );
        assert_eq!(print(&[Stmt::Function(Expr::name("C"), Some("run".into()), method)]), "function C:run(x, ...) end");
    }
}
