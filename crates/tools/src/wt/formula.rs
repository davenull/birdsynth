//! The wavetable editor's formula language, compiled to a small stack
//! machine (no `eval` anywhere). A formula is evaluated once per sample of
//! each frame it's applied to, with these names:
//!
//! | name   | value                                                        |
//! |--------|--------------------------------------------------------------|
//! | `x`    | position in the cycle, -1 to 1                               |
//! | `w`    | position in the cycle, 0 to 1                                |
//! | `y`    | position in the table, 0 (first frame) to 1 (last)           |
//! | `z`    | position in the table, -1 to 1                               |
//! | `q`    | the frame's number, 0, 1, 2, …                               |
//! | `in`   | the sample that's there now (to process a table)             |
//! | `sel`  | 1 in a selected frame, else 0                                |
//! | `rand` | a new random number from -1 to 1 each time it's read         |
//! | `pi`, `tau`, `e` | constants                                          |
//!
//! Operators, loosest first: `c ? a : b`, `||`, `&&`, `== !=`, `< > <= >=`,
//! `+ -`, `* / %`, unary `- + !`, `^` (power, right to left; `-x^2` is
//! `-(x^2)`). Comparisons and logic give 1 or 0.
//!
//! Functions: `sin cos tan asin acos atan sinh cosh tanh exp ln log (=ln)
//! log2 log10 sqrt abs sign floor ceil round frac` (one argument), `atan2
//! min max pow mod step` (two), `clamp lerp smoothstep` (three), and wave
//! shapes of a phase in cycles: `saw(p) square(p) tri(p)`, `pulse(p, width)`.
//! A result that isn't a number (like `sqrt(-1)`) becomes 0, and samples are
//! kept within ±16.

use std::fmt;

use wt_dsp::mip::FRAME_LEN;
use wt_dsp::rng::Rng;

#[derive(Clone, Debug, PartialEq)]
pub struct FormulaError {
    /// Byte offset (the formula is ASCII, so also the column) where it went wrong.
    pub pos: usize,
    pub msg: String,
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at {})", self.msg, self.pos + 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Var {
    X,
    W,
    Y,
    Z,
    Q,
    In,
    Sel,
    Rand,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Ln,
    Log2,
    Log10,
    Sqrt,
    Abs,
    Sign,
    Floor,
    Ceil,
    Round,
    Frac,
    Atan2,
    Min,
    Max,
    Pow,
    Mod,
    Step,
    Clamp,
    Lerp,
    Smoothstep,
    Saw,
    Square,
    Tri,
    Pulse,
    Rand,
}

const FUNCS: &[(&str, Func, usize)] = &[
    ("sin", Func::Sin, 1),
    ("cos", Func::Cos, 1),
    ("tan", Func::Tan, 1),
    ("asin", Func::Asin, 1),
    ("acos", Func::Acos, 1),
    ("atan", Func::Atan, 1),
    ("sinh", Func::Sinh, 1),
    ("cosh", Func::Cosh, 1),
    ("tanh", Func::Tanh, 1),
    ("exp", Func::Exp, 1),
    ("ln", Func::Ln, 1),
    ("log", Func::Ln, 1),
    ("log2", Func::Log2, 1),
    ("log10", Func::Log10, 1),
    ("sqrt", Func::Sqrt, 1),
    ("abs", Func::Abs, 1),
    ("sign", Func::Sign, 1),
    ("floor", Func::Floor, 1),
    ("ceil", Func::Ceil, 1),
    ("round", Func::Round, 1),
    ("frac", Func::Frac, 1),
    ("atan2", Func::Atan2, 2),
    ("min", Func::Min, 2),
    ("max", Func::Max, 2),
    ("pow", Func::Pow, 2),
    ("mod", Func::Mod, 2),
    ("step", Func::Step, 2),
    ("clamp", Func::Clamp, 3),
    ("lerp", Func::Lerp, 3),
    ("smoothstep", Func::Smoothstep, 3),
    ("saw", Func::Saw, 1),
    ("square", Func::Square, 1),
    ("tri", Func::Tri, 1),
    ("pulse", Func::Pulse, 2),
    ("rand", Func::Rand, 0),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Const(f32),
    Var(Var),
    Neg,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    /// cond, a, b → cond ≠ 0 ? a : b
    Select,
    Call(Func),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok<'a> {
    Num(f32),
    Name(&'a str),
    Sym(&'static str),
    End,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    /// The current token and where it starts.
    tok: Tok<'a>,
    at: usize,
}

const SYMS: &[&str] = &["<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "%", "^", "<", ">", "!", "?", ":", "(", ")", ","];

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Result<Lexer<'a>, FormulaError> {
        let mut l = Lexer { src, pos: 0, tok: Tok::End, at: 0 };
        l.advance()?;
        Ok(l)
    }

    fn advance(&mut self) -> Result<(), FormulaError> {
        let b = self.src.as_bytes();
        while self.pos < b.len() && b[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        self.at = self.pos;
        if self.pos >= b.len() {
            self.tok = Tok::End;
            return Ok(());
        }
        let c = b[self.pos];
        if c.is_ascii_digit() || (c == b'.' && b.get(self.pos + 1).is_some_and(u8::is_ascii_digit)) {
            let start = self.pos;
            while self.pos < b.len() && b[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos < b.len() && b[self.pos] == b'.' {
                self.pos += 1;
                while self.pos < b.len() && b[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
            }
            // an exponent, but only if digits follow (so "2e" isn't a number)
            if self.pos < b.len() && (b[self.pos] == b'e' || b[self.pos] == b'E') {
                let mut p = self.pos + 1;
                if p < b.len() && (b[p] == b'+' || b[p] == b'-') {
                    p += 1;
                }
                if p < b.len() && b[p].is_ascii_digit() {
                    while p < b.len() && b[p].is_ascii_digit() {
                        p += 1;
                    }
                    self.pos = p;
                }
            }
            let text = &self.src[start..self.pos];
            let v: f64 = text.parse().map_err(|_| FormulaError { pos: start, msg: format!("\"{text}\" isn't a number") })?;
            self.tok = Tok::Num(v as f32);
            return Ok(());
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = self.pos;
            while self.pos < b.len() && (b[self.pos].is_ascii_alphanumeric() || b[self.pos] == b'_') {
                self.pos += 1;
            }
            self.tok = Tok::Name(&self.src[start..self.pos]);
            return Ok(());
        }
        for s in SYMS {
            if self.src[self.pos..].starts_with(s) {
                self.pos += s.len();
                self.tok = Tok::Sym(s);
                return Ok(());
            }
        }
        let ch = self.src[self.pos..].chars().next().unwrap_or('?');
        Err(FormulaError { pos: self.pos, msg: format!("unexpected character '{ch}'") })
    }
}

fn describe(t: Tok) -> String {
    match t {
        Tok::Num(v) => format!("the number {v}"),
        Tok::Name(n) => format!("'{n}'"),
        Tok::Sym(s) => format!("'{s}'"),
        Tok::End => "the end".into(),
    }
}

/// Binding powers of the infix operators: (left, right, op).
fn infix(s: &str) -> Option<(u8, u8, Op)> {
    Some(match s {
        "||" => (3, 4, Op::Or),
        "&&" => (5, 6, Op::And),
        "==" => (7, 8, Op::Eq),
        "!=" => (7, 8, Op::Ne),
        "<" => (9, 10, Op::Lt),
        ">" => (9, 10, Op::Gt),
        "<=" => (9, 10, Op::Le),
        ">=" => (9, 10, Op::Ge),
        "+" => (11, 12, Op::Add),
        "-" => (11, 12, Op::Sub),
        "*" => (13, 14, Op::Mul),
        "/" => (13, 14, Op::Div),
        "%" => (13, 14, Op::Rem),
        "^" => (18, 17, Op::Pow),
        _ => return None,
    })
}
const PREFIX_BP: u8 = 15;
const TERNARY_BP: u8 = 2;

struct Parser<'a> {
    lex: Lexer<'a>,
    code: Vec<Op>,
    depth: usize,
    max_depth: usize,
}

impl<'a> Parser<'a> {
    fn emit(&mut self, op: Op, effect: isize) {
        self.code.push(op);
        self.depth = (self.depth as isize + effect) as usize;
        self.max_depth = self.max_depth.max(self.depth);
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, FormulaError> {
        Err(FormulaError { pos: self.lex.at, msg: msg.into() })
    }

    fn expect(&mut self, sym: &'static str, what: &str) -> Result<(), FormulaError> {
        if self.lex.tok == Tok::Sym(sym) {
            self.lex.advance()
        } else {
            self.err(format!("expected {what}, found {}", describe(self.lex.tok)))
        }
    }

    fn expr(&mut self, min_bp: u8) -> Result<(), FormulaError> {
        self.prefix()?;
        loop {
            let s = match self.lex.tok {
                Tok::Sym(s) => s,
                Tok::End => break,
                t => return self.err(format!("expected an operator, found {}", describe(t))),
            };
            if s == "?" {
                if TERNARY_BP < min_bp {
                    break;
                }
                self.lex.advance()?;
                self.expr(0)?;
                self.expect(":", "':' to go with '?'")?;
                self.expr(TERNARY_BP - 1)?;
                self.emit(Op::Select, -2);
                continue;
            }
            let Some((l, r, op)) = infix(s) else { break };
            if l < min_bp {
                break;
            }
            self.lex.advance()?;
            self.expr(r)?;
            self.emit(op, -1);
        }
        Ok(())
    }

    fn prefix(&mut self) -> Result<(), FormulaError> {
        match self.lex.tok {
            Tok::Num(v) => {
                self.lex.advance()?;
                self.emit(Op::Const(v), 1);
            }
            Tok::Sym("(") => {
                self.lex.advance()?;
                self.expr(0)?;
                self.expect(")", "')'")?;
            }
            Tok::Sym(s @ ("-" | "+" | "!")) => {
                self.lex.advance()?;
                self.expr(PREFIX_BP)?;
                match s {
                    "-" => self.emit(Op::Neg, 0),
                    "!" => self.emit(Op::Not, 0),
                    _ => {}
                }
            }
            Tok::Name(name) => {
                let at = self.lex.at;
                self.lex.advance()?;
                if self.lex.tok == Tok::Sym("(") {
                    let Some(&(_, f, arity)) = FUNCS.iter().find(|(n, _, _)| *n == name) else {
                        return Err(FormulaError { pos: at, msg: format!("unknown function '{name}'") });
                    };
                    self.lex.advance()?;
                    let mut args = 0;
                    if self.lex.tok != Tok::Sym(")") {
                        loop {
                            self.expr(0)?;
                            args += 1;
                            if self.lex.tok == Tok::Sym(",") {
                                self.lex.advance()?;
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect(")", "')' or ','")?;
                    if args != arity {
                        let s = if arity == 1 { "" } else { "s" };
                        return Err(FormulaError { pos: at, msg: format!("{name} takes {arity} argument{s}, not {args}") });
                    }
                    self.emit(Op::Call(f), 1 - arity as isize);
                } else {
                    let op = match name {
                        "x" => Op::Var(Var::X),
                        "w" => Op::Var(Var::W),
                        "y" => Op::Var(Var::Y),
                        "z" => Op::Var(Var::Z),
                        "q" => Op::Var(Var::Q),
                        "in" => Op::Var(Var::In),
                        "sel" => Op::Var(Var::Sel),
                        "rand" => Op::Var(Var::Rand),
                        "pi" => Op::Const(std::f32::consts::PI),
                        "tau" => Op::Const(std::f32::consts::TAU),
                        "e" => Op::Const(std::f32::consts::E),
                        _ if FUNCS.iter().any(|(n, _, _)| *n == name) => {
                            return Err(FormulaError { pos: at, msg: format!("{name} is a function: write {name}(…)") });
                        }
                        _ => return Err(FormulaError { pos: at, msg: format!("unknown name '{name}'") }),
                    };
                    self.emit(op, 1);
                }
            }
            Tok::End => return self.err(if self.lex.src.trim().is_empty() { "the formula is empty" } else { "the formula ends too soon" }),
            t => return self.err(format!("expected a value, found {}", describe(t))),
        }
        Ok(())
    }
}

/// The values a formula can read at one sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vars {
    pub x: f32,
    pub w: f32,
    pub y: f32,
    pub z: f32,
    pub q: f32,
    pub input: f32,
    pub sel: f32,
}

#[derive(Clone, Debug)]
pub struct Formula {
    code: Vec<Op>,
    depth: usize,
}

fn frac(v: f32) -> f32 {
    v - v.floor()
}

fn b(c: bool) -> f32 {
    if c { 1.0 } else { 0.0 }
}

impl Formula {
    pub fn compile(src: &str) -> Result<Formula, FormulaError> {
        if let Some((i, ch)) = src.char_indices().find(|(_, c)| !c.is_ascii()) {
            return Err(FormulaError { pos: i, msg: format!("unexpected character '{ch}'") });
        }
        let mut p = Parser { lex: Lexer::new(src)?, code: Vec::new(), depth: 0, max_depth: 0 };
        p.expr(0)?;
        if p.lex.tok != Tok::End {
            return p.err(format!("expected an operator, found {}", describe(p.lex.tok)));
        }
        Ok(Formula { code: p.code, depth: p.max_depth })
    }

    /// Evaluate at one sample. `stack` is reused between calls.
    pub fn eval(&self, v: &Vars, rng: &mut Rng, stack: &mut Vec<f32>) -> f32 {
        stack.clear();
        stack.reserve(self.depth);
        for &op in &self.code {
            match op {
                Op::Const(c) => stack.push(c),
                Op::Var(var) => stack.push(match var {
                    Var::X => v.x,
                    Var::W => v.w,
                    Var::Y => v.y,
                    Var::Z => v.z,
                    Var::Q => v.q,
                    Var::In => v.input,
                    Var::Sel => v.sel,
                    Var::Rand => rng.next_f32() * 2.0 - 1.0,
                }),
                Op::Neg => {
                    let a = stack.last_mut().unwrap();
                    *a = -*a;
                }
                Op::Not => {
                    let a = stack.last_mut().unwrap();
                    *a = b(*a == 0.0);
                }
                Op::Select => {
                    let e = stack.pop().unwrap();
                    let t = stack.pop().unwrap();
                    let c = stack.last_mut().unwrap();
                    *c = if *c != 0.0 { t } else { e };
                }
                Op::Call(f) => {
                    let r = match f {
                        Func::Rand => rng.next_f32() * 2.0 - 1.0,
                        Func::Atan2 | Func::Min | Func::Max | Func::Pow | Func::Mod | Func::Step | Func::Pulse => {
                            let y = stack.pop().unwrap();
                            let x = stack.pop().unwrap();
                            match f {
                                Func::Atan2 => x.atan2(y),
                                Func::Min => x.min(y),
                                Func::Max => x.max(y),
                                Func::Pow => x.powf(y),
                                Func::Mod => x.rem_euclid(y),
                                Func::Step => b(y >= x),
                                _ => {
                                    if frac(x) < y {
                                        1.0
                                    } else {
                                        -1.0
                                    }
                                }
                            }
                        }
                        Func::Clamp | Func::Lerp | Func::Smoothstep => {
                            let c = stack.pop().unwrap();
                            let bb = stack.pop().unwrap();
                            let a = stack.pop().unwrap();
                            match f {
                                Func::Clamp => a.max(bb).min(c),
                                Func::Lerp => a + (bb - a) * c,
                                _ => {
                                    let t = ((c - a) / (bb - a)).clamp(0.0, 1.0);
                                    t * t * (3.0 - 2.0 * t)
                                }
                            }
                        }
                        _ => {
                            let a = stack.pop().unwrap();
                            match f {
                                Func::Sin => a.sin(),
                                Func::Cos => a.cos(),
                                Func::Tan => a.tan(),
                                Func::Asin => a.asin(),
                                Func::Acos => a.acos(),
                                Func::Atan => a.atan(),
                                Func::Sinh => a.sinh(),
                                Func::Cosh => a.cosh(),
                                Func::Tanh => a.tanh(),
                                Func::Exp => a.exp(),
                                Func::Ln => a.ln(),
                                Func::Log2 => a.log2(),
                                Func::Log10 => a.log10(),
                                Func::Sqrt => a.sqrt(),
                                Func::Abs => a.abs(),
                                Func::Sign => {
                                    if a > 0.0 {
                                        1.0
                                    } else if a < 0.0 {
                                        -1.0
                                    } else {
                                        0.0
                                    }
                                }
                                Func::Floor => a.floor(),
                                Func::Ceil => a.ceil(),
                                Func::Round => a.round(),
                                Func::Frac => frac(a),
                                Func::Saw => 2.0 * frac(a) - 1.0,
                                Func::Square => {
                                    if frac(a) < 0.5 {
                                        1.0
                                    } else {
                                        -1.0
                                    }
                                }
                                Func::Tri => 1.0 - 4.0 * (frac(a + 0.25) - 0.5).abs(),
                                _ => unreachable!(),
                            }
                        }
                    };
                    stack.push(r);
                }
                _ => {
                    let y = stack.pop().unwrap();
                    let x = stack.last_mut().unwrap();
                    *x = match op {
                        Op::Add => *x + y,
                        Op::Sub => *x - y,
                        Op::Mul => *x * y,
                        Op::Div => *x / y,
                        Op::Rem => x.rem_euclid(y),
                        Op::Pow => x.powf(y),
                        Op::Lt => b(*x < y),
                        Op::Gt => b(*x > y),
                        Op::Le => b(*x <= y),
                        Op::Ge => b(*x >= y),
                        Op::Eq => b(*x == y),
                        Op::Ne => b(*x != y),
                        Op::And => b(*x != 0.0 && y != 0.0),
                        Op::Or => b(*x != 0.0 || y != 0.0),
                        _ => unreachable!(),
                    };
                }
            }
        }
        let r = stack.pop().unwrap_or(0.0);
        if r.is_finite() { r } else { 0.0 }
    }

    /// Run over a table (count × 2048, rewritten in place): every frame whose
    /// `apply` flag is set, with `sel` telling the formula which are selected.
    pub fn run(&self, frames: &mut [f32], apply: &[bool], selected: &[bool], seed: u32) {
        let count = frames.len() / FRAME_LEN;
        let mut rng = Rng::new(seed);
        let mut stack = Vec::with_capacity(self.depth);
        for f in 0..count {
            if !apply.get(f).copied().unwrap_or(true) {
                continue;
            }
            let y = if count > 1 { f as f32 / (count - 1) as f32 } else { 0.0 };
            let mut v = Vars { y, z: 2.0 * y - 1.0, q: f as f32, sel: b(selected.get(f).copied().unwrap_or(false)), ..Vars::default() };
            for i in 0..FRAME_LEN {
                let s = &mut frames[f * FRAME_LEN + i];
                v.w = i as f32 / FRAME_LEN as f32;
                v.x = 2.0 * v.w - 1.0;
                v.input = *s;
                // keep a runaway formula from writing absurd samples
                *s = self.eval(&v, &mut rng, &mut stack).clamp(-16.0, 16.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn at(src: &str, v: Vars) -> f32 {
        let f = Formula::compile(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        f.eval(&v, &mut Rng::new(1), &mut Vec::new())
    }
    fn err(src: &str) -> FormulaError {
        Formula::compile(src).expect_err(src)
    }

    #[test]
    fn precedence_and_associativity() {
        let v = Vars { x: 0.5, w: 0.75, ..Vars::default() };
        assert_eq!(at("1 + 2 * 3", v), 7.0);
        assert_eq!(at("(1 + 2) * 3", v), 9.0);
        assert_eq!(at("2 ^ 3 ^ 2", v), 512.0);
        assert_eq!(at("-2 ^ 2", v), -4.0);
        assert_eq!(at("10 - 4 - 3", v), 3.0);
        assert_eq!(at("7 % 3", v), 1.0);
        assert_eq!(at("-7 % 3", v), 2.0);
        assert_eq!(at("x < 0 ? -1 : x > 0.4 ? 2 : 3", v), 2.0);
        assert_eq!(at("1 || 0 && 0", v), 1.0);
        assert_eq!(at("!x + !0", v), 1.0);
        assert!((at("2e3 + .5 + 1.5e-1", v) - 2000.65).abs() < 1e-3);
        assert_eq!(at("x == 0.5 && w != 1", v), 1.0);
        assert!((at("sin(pi / 2) + cos(0) + tau / (2 * pi) - e", v) - (3.0 - std::f32::consts::E)).abs() < 1e-6);
        assert_eq!(at("clamp(5, -1, 1) + min(1, 2) + max(1, 2) + lerp(0, 10, 0.25)", v), 6.5);
        assert_eq!(at("sqrt(-1)", v), 0.0, "not a number becomes 0");
    }

    /// (frame, sample, expected value)
    type Check = (usize, usize, f32);

    /// The golden set: each formula over a 3-frame table, checked at fixed points.
    #[test]
    fn golden_formulas() {
        let cases: &[(&str, &[Check])] = &[
            ("sin(w * tau)", &[(0, 0, 0.0), (0, 512, 1.0), (1, 1536, -1.0)]),
            ("x", &[(0, 0, -1.0), (2, 1024, 0.0), (1, 1536, 0.5)]),
            ("saw(w)", &[(0, 0, -1.0), (0, 1024, 0.0)]),
            ("square(w)", &[(0, 100, 1.0), (0, 1500, -1.0)]),
            ("tri(w)", &[(0, 0, 0.0), (0, 512, 1.0), (0, 1536, -1.0)]),
            ("pulse(w, 0.25)", &[(0, 400, 1.0), (0, 600, -1.0)]),
            ("lerp(saw(w), sin(w * tau), y)", &[(0, 512, -0.5), (2, 512, 1.0), (1, 512, 0.25)]),
            ("z", &[(0, 9, -1.0), (1, 9, 0.0), (2, 9, 1.0)]),
            ("q", &[(2, 9, 2.0)]),
            ("sign(x) * abs(x) ^ 0.5", &[(0, 1536, 0.5f32.sqrt()), (0, 512, -0.5f32.sqrt())]),
            ("x ^ 3", &[(0, 1536, 0.125)]),
            ("tanh(4 * x) / tanh(4)", &[(0, 0, -1.0), (0, 1024, 0.0)]),
            ("sel", &[(0, 5, 0.0), (1, 5, 1.0)]),
            ("in * 0.5", &[(0, 1536, 0.25)]),
            ("step(0, x)", &[(0, 1023, 0.0), (0, 1024, 1.0)]),
            ("smoothstep(-1, 1, x)", &[(0, 1024, 0.5)]),
        ];
        for (src, checks) in cases {
            let f = Formula::compile(src).unwrap();
            // "in" starts as x, for the processing formulas
            let mut t: Vec<f32> = (0..3 * FRAME_LEN).map(|i| 2.0 * (i % FRAME_LEN) as f32 / FRAME_LEN as f32 - 1.0).collect();
            f.run(&mut t, &[true; 3], &[false, true, false], 9);
            for &(fr, i, want) in *checks {
                let got = t[fr * FRAME_LEN + i];
                assert!((got - want).abs() < 1e-4, "{src} frame {fr} sample {i}: {got} vs {want}");
            }
        }
        // rand: seeded, so the same every time, spread over -1..1
        let f = Formula::compile("rand").unwrap();
        let mut a = vec![0.0; FRAME_LEN];
        let mut b2 = vec![0.0; FRAME_LEN];
        f.run(&mut a, &[true], &[false], 5);
        f.run(&mut b2, &[true], &[false], 5);
        assert_eq!(a, b2);
        assert!(a.iter().all(|v| (-1.0..=1.0).contains(v)) && a.iter().any(|&v| v > 0.9) && a.iter().any(|&v| v < -0.9));
        // only the frames asked for change
        let f = Formula::compile("1").unwrap();
        let mut t = vec![0.0; 2 * FRAME_LEN];
        f.run(&mut t, &[false, true], &[false, true], 1);
        assert!(t[..FRAME_LEN].iter().all(|&v| v == 0.0) && t[FRAME_LEN..].iter().all(|&v| v == 1.0));
        let _ = PI;
    }

    #[test]
    fn errors_say_where() {
        assert_eq!(err("sin(x"), FormulaError { pos: 5, msg: "expected ')' or ',', found the end".into() });
        assert_eq!(err("2 +* 3").pos, 3);
        assert_eq!(err("foo(x)"), FormulaError { pos: 0, msg: "unknown function 'foo'".into() });
        assert_eq!(err("x y"), FormulaError { pos: 2, msg: "expected an operator, found 'y'".into() });
        assert_eq!(err("sin(1, 2)"), FormulaError { pos: 0, msg: "sin takes 1 argument, not 2".into() });
        assert_eq!(err("2 * blah").msg, "unknown name 'blah'");
        assert_eq!(err("2 * blah").pos, 4);
        assert_eq!(err("").msg, "the formula is empty");
        assert_eq!(err("1 +").msg, "the formula ends too soon");
        assert_eq!(err("sin(").msg, "the formula ends too soon");
        assert_eq!(err("   ").msg, "the formula is empty");
        assert_eq!(err("x ? 1").msg, "expected ':' to go with '?', found the end");
        assert_eq!(err("x # 2"), FormulaError { pos: 2, msg: "unexpected character '#'".into() });
        assert_eq!(err("sin + 1").msg, "sin is a function: write sin(…)");
        assert_eq!(err("(1 + 2").pos, 6);
        assert_eq!(err("1.2.3").pos, 3);
        assert_eq!(err("x × 2").pos, 2);
    }
}
