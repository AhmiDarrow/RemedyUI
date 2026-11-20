//! `.rfx` source to a syntax tree: the lexer and a recursive-descent parser with depth and
//! size limits, so hostile input fails with a located error instead of exhausting memory or
//! the stack.

use std::fmt;

/// A position in the source, 1-based.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Line.
    pub line: u32,
    /// Column (in characters).
    pub col: u32,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// One problem in a source, where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where.
    pub at: Span,
    /// What, in plain words.
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn new(at: Span, message: impl Into<String>) -> Self {
        Diagnostic { at, message: message.into() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    Ident(String),
    Num(f32),
    Hex([f32; 4]),
    Sym(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    pub(crate) at: Span,
}

const SYMS: &[&str] = &[
    "..", "->", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "(", ")", "{", "}", ",", ";",
    ":", "=", "+", "-", "*", "/", "%", "<", ">", "!", "?", ".",
];

/// Splits `src` into tokens. At most `max_tokens`.
pub(crate) fn lex(src: &str, max_tokens: usize) -> Result<Vec<Token>, Diagnostic> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    let at = |line, col| Span { line, col };
    while let Some(&c) = chars.get(i) {
        if out.len() >= max_tokens {
            return Err(Diagnostic::new(at(line, col), format!("too long: more than {max_tokens} tokens")));
        }
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while chars.get(i).is_some_and(|&c| c != '\n') {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = at(line, col);
            i += 2;
            col += 2;
            loop {
                match chars.get(i) {
                    None => return Err(Diagnostic::new(start, "a comment that never ends")),
                    Some('*') if chars.get(i + 1) == Some(&'/') => {
                        i += 2;
                        col += 2;
                        break;
                    }
                    Some('\n') => {
                        i += 1;
                        line += 1;
                        col = 1;
                    }
                    Some(_) => {
                        i += 1;
                        col += 1;
                    }
                }
            }
            continue;
        }
        let here = at(line, col);
        let start = i;
        if c.is_ascii_alphabetic() || c == '_' {
            while chars.get(i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') {
                i += 1;
            }
            let word: String = chars.get(start..i).unwrap_or(&[]).iter().collect();
            if word.len() > 64 {
                return Err(Diagnostic::new(here, "a name longer than 64 characters"));
            }
            col += (i - start) as u32;
            out.push(Token { tok: Tok::Ident(word), at: here });
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
            // A '.' followed by '.' is a range, not a decimal point.
            if chars.get(i) == Some(&'.') && chars.get(i + 1) != Some(&'.') {
                i += 1;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            if matches!(chars.get(i), Some('e' | 'E'))
                && (chars.get(i + 1).is_some_and(char::is_ascii_digit)
                    || (matches!(chars.get(i + 1), Some('-' | '+'))
                        && chars.get(i + 2).is_some_and(char::is_ascii_digit)))
            {
                i += 2;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            let text: String = chars.get(start..i).unwrap_or(&[]).iter().collect();
            col += (i - start) as u32;
            match text.parse::<f32>() {
                Ok(v) if v.is_finite() => out.push(Token { tok: Tok::Num(v), at: here }),
                _ => return Err(Diagnostic::new(here, format!("`{text}` is not a usable number"))),
            }
            continue;
        }
        if c == '#' {
            i += 1;
            while chars.get(i).is_some_and(char::is_ascii_hexdigit) {
                i += 1;
            }
            let hex: String = chars.get(start + 1..i).unwrap_or(&[]).iter().collect();
            col += (i - start) as u32;
            let byte = |k: usize| {
                u8::from_str_radix(hex.get(k..k + 2).unwrap_or("ff"), 16).map(|b| f32::from(b) / 255.0)
            };
            let nib = |k: usize| {
                u8::from_str_radix(hex.get(k..k + 1).unwrap_or("f"), 16).map(|b| f32::from(b * 17) / 255.0)
            };
            let rgba = match hex.len() {
                3 | 4 => (nib(0), nib(1), nib(2), if hex.len() == 4 { nib(3) } else { Ok(1.0) }),
                6 | 8 => (byte(0), byte(2), byte(4), if hex.len() == 8 { byte(6) } else { Ok(1.0) }),
                _ => {
                    return Err(Diagnostic::new(
                        here,
                        format!("`#{hex}` is not a colour (#rgb, #rgba, #rrggbb or #rrggbbaa)"),
                    ));
                }
            };
            match rgba {
                (Ok(r), Ok(g), Ok(b), Ok(a)) => out.push(Token { tok: Tok::Hex([r, g, b, a]), at: here }),
                _ => return Err(Diagnostic::new(here, format!("`#{hex}` is not a colour"))),
            }
            continue;
        }
        let rest: String = chars.get(i..(i + 2).min(chars.len())).unwrap_or(&[]).iter().collect();
        match SYMS.iter().find(|s| rest.starts_with(**s)) {
            Some(s) => {
                i += s.len();
                col += s.len() as u32;
                out.push(Token { tok: Tok::Sym(s), at: here });
            }
            None => return Err(Diagnostic::new(here, format!("unexpected `{c}`"))),
        }
    }
    out.push(Token { tok: Tok::Eof, at: at(line, col) });
    Ok(out)
}

/// Binary operators, by precedence groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Num(f32, Span),
    Color([f32; 4], Span),
    Name(String, Span),
    Unary(char, Box<Expr>, Span),
    Binary(BinOp, Box<Expr>, Box<Expr>, Span),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>, Span),
    /// `f(args)` with optional `name: value` arguments.
    Call(String, Vec<(Option<String>, Expr)>, Span),
    Field(Box<Expr>, String, Span),
}

impl Expr {
    pub(crate) fn span(&self) -> Span {
        match self {
            Expr::Num(_, s)
            | Expr::Color(_, s)
            | Expr::Name(_, s)
            | Expr::Unary(_, _, s)
            | Expr::Binary(_, _, _, s)
            | Expr::Ternary(_, _, _, s)
            | Expr::Call(_, _, s)
            | Expr::Field(_, _, s) => *s,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Stmt {
    /// `let x = e;` or `var x = e;` (both bind; `var` may be assigned again).
    Let { name: String, value: Expr, mutable: bool, at: Span },
    /// `x = e;`, `x += e;` …
    Assign { name: String, op: Option<BinOp>, value: Expr, at: Span },
    /// `for i in a..b { … }` with literal bounds.
    For { var: String, from: i64, to: i64, body: Vec<Stmt>, at: Span },
    /// `if c { … } else { … }`.
    If { cond: Expr, then: Vec<Stmt>, otherwise: Vec<Stmt>, at: Span },
    /// `return e;` (functions only, last).
    Return(Expr, Span),
}

#[derive(Clone, Debug)]
pub(crate) struct ParamDecl {
    pub(crate) ty: String,
    pub(crate) name: String,
    pub(crate) default: Option<Expr>,
    pub(crate) range: Option<(f32, f32)>,
    pub(crate) at: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct FnDecl {
    pub(crate) name: String,
    pub(crate) args: Vec<(String, String)>,
    pub(crate) ret: String,
    pub(crate) body: Vec<Stmt>,
    pub(crate) at: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum Item {
    Setting(String, Expr, Span),
    Const(String, Expr, Span),
    Fn(FnDecl),
    /// `pass name [scale k] { … }`
    Pass {
        name: String,
        scale: Option<f32>,
        body: Vec<Stmt>,
        at: Span,
    },
    /// `pass name [scale k] = op(args);` (blur, bloom)
    Filter {
        name: String,
        scale: Option<f32>,
        call: Expr,
        at: Span,
    },
    /// `particles name [over src] (opts) { … }`
    Particles {
        name: String,
        over: Option<String>,
        opts: Vec<(String, Expr)>,
        body: Vec<Stmt>,
        at: Span,
    },
    Output(String, Span),
}

#[derive(Clone, Debug)]
pub(crate) struct File {
    pub(crate) name: String,
    pub(crate) params: Vec<ParamDecl>,
    pub(crate) items: Vec<Item>,
}

pub(crate) struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: u32,
    max_depth: u32,
}

type P<T> = Result<T, Diagnostic>;

impl Parser {
    pub(crate) fn new(toks: Vec<Token>, max_depth: u32) -> Self {
        Parser { toks, pos: 0, depth: 0, max_depth }
    }

    fn peek(&self) -> &Tok {
        self.toks.get(self.pos).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn peek2(&self) -> &Tok {
        self.toks.get(self.pos + 1).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn at(&self) -> Span {
        self.toks.get(self.pos).or(self.toks.last()).map_or(Span::default(), |t| t.at)
    }

    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn is(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(x) if x == w)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> P<()> {
        if self.eat(s) { Ok(()) } else { Err(self.unexpected(&format!("`{s}`"))) }
    }

    fn unexpected(&self, wanted: &str) -> Diagnostic {
        let found = match self.peek() {
            Tok::Ident(w) => format!("`{w}`"),
            Tok::Num(n) => format!("`{n}`"),
            Tok::Hex(_) => "a colour".to_owned(),
            Tok::Sym(s) => format!("`{s}`"),
            Tok::Eof => "the end of the file".to_owned(),
        };
        Diagnostic::new(self.at(), format!("expected {wanted}, found {found}"))
    }

    fn ident(&mut self) -> P<String> {
        match self.peek().clone() {
            Tok::Ident(w) => {
                self.pos += 1;
                Ok(w)
            }
            _ => Err(self.unexpected("a name")),
        }
    }

    fn number(&mut self) -> P<f32> {
        let neg = self.eat("-");
        match self.bump() {
            Tok::Num(n) => Ok(if neg { -n } else { n }),
            _ => {
                self.pos -= 1;
                Err(self.unexpected("a number"))
            }
        }
    }

    fn enter(&mut self) -> P<()> {
        self.depth += 1;
        if self.depth > self.max_depth {
            Err(Diagnostic::new(self.at(), format!("nested more than {} deep", self.max_depth)))
        } else {
            Ok(())
        }
    }

    pub(crate) fn file(&mut self) -> P<File> {
        if !self.is_word("effect") {
            return Err(self.unexpected("`effect name(…) { … }`"));
        }
        self.pos += 1;
        let name = self.ident()?;
        let mut params = Vec::new();
        if self.eat("(") {
            while !self.is(")") {
                let at = self.at();
                let ty = self.ident()?;
                let pname = self.ident()?;
                let default = if self.eat("=") { Some(self.expr()?) } else { None };
                let range = if self.is_word("in") {
                    self.pos += 1;
                    let lo = self.number()?;
                    self.expect("..")?;
                    let hi = self.number()?;
                    Some((lo, hi))
                } else {
                    None
                };
                params.push(ParamDecl { ty, name: pname, default, range, at });
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        self.expect("{")?;
        let mut items = Vec::new();
        while !self.is("}") {
            if matches!(self.peek(), Tok::Eof) {
                return Err(self.unexpected("`}`"));
            }
            items.push(self.item()?);
        }
        self.expect("}")?;
        if !matches!(self.peek(), Tok::Eof) {
            return Err(self.unexpected("the end of the file (one effect per file)"));
        }
        Ok(File { name, params, items })
    }

    fn item(&mut self) -> P<Item> {
        let at = self.at();
        let word = self.ident()?;
        match word.as_str() {
            "fps" | "scale" | "blend" | "still" => {
                let v = self.expr()?;
                self.expect(";")?;
                Ok(Item::Setting(word, v, at))
            }
            "const" => {
                let name = self.ident()?;
                self.expect("=")?;
                let v = self.expr()?;
                self.expect(";")?;
                Ok(Item::Const(name, v, at))
            }
            "fn" => {
                let name = self.ident()?;
                self.expect("(")?;
                let mut args = Vec::new();
                while !self.is(")") {
                    let a = self.ident()?;
                    self.expect(":")?;
                    let t = self.ident()?;
                    args.push((a, t));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
                self.expect("->")?;
                let ret = self.ident()?;
                let body = self.block()?;
                Ok(Item::Fn(FnDecl { name, args, ret, body, at }))
            }
            "pass" => {
                let name = self.ident()?;
                let scale = if self.is_word("scale") {
                    self.pos += 1;
                    Some(self.number()?)
                } else {
                    None
                };
                if self.eat("=") {
                    let call = self.expr()?;
                    self.expect(";")?;
                    Ok(Item::Filter { name, scale, call, at })
                } else {
                    let body = self.block()?;
                    Ok(Item::Pass { name, scale, body, at })
                }
            }
            "particles" => {
                let name = self.ident()?;
                let over = if self.is_word("over") {
                    self.pos += 1;
                    Some(self.ident()?)
                } else {
                    None
                };
                let mut opts = Vec::new();
                if self.eat("(") {
                    while !self.is(")") {
                        let k = self.ident()?;
                        self.expect(":")?;
                        let v = self.expr()?;
                        opts.push((k, v));
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                }
                let body = self.block()?;
                Ok(Item::Particles { name, over, opts, body, at })
            }
            "output" => {
                let name = self.ident()?;
                self.expect(";")?;
                Ok(Item::Output(name, at))
            }
            other => Err(Diagnostic::new(
                at,
                format!(
                    "`{other}` is not an effect item (fps, scale, blend, still, const, fn, pass, particles, output)"
                ),
            )),
        }
    }

    fn block(&mut self) -> P<Vec<Stmt>> {
        self.enter()?;
        self.expect("{")?;
        let mut out = Vec::new();
        while !self.is("}") {
            if matches!(self.peek(), Tok::Eof) {
                return Err(self.unexpected("`}`"));
            }
            out.push(self.stmt()?);
        }
        self.expect("}")?;
        self.depth -= 1;
        Ok(out)
    }

    fn stmt(&mut self) -> P<Stmt> {
        let at = self.at();
        if self.is_word("let") || self.is_word("var") {
            let mutable = self.is_word("var");
            self.pos += 1;
            let name = self.ident()?;
            self.expect("=")?;
            let value = self.expr()?;
            self.expect(";")?;
            return Ok(Stmt::Let { name, value, mutable, at });
        }
        if self.is_word("for") {
            self.pos += 1;
            let var = self.ident()?;
            if !self.is_word("in") {
                return Err(self.unexpected("`in`"));
            }
            self.pos += 1;
            let from = self.number()?;
            self.expect("..")?;
            let to = self.number()?;
            if from.fract() != 0.0 || to.fract() != 0.0 {
                return Err(Diagnostic::new(at, "loop bounds are whole numbers"));
            }
            let body = self.block()?;
            return Ok(Stmt::For { var, from: from as i64, to: to as i64, body, at });
        }
        if self.is_word("if") {
            self.pos += 1;
            let cond = self.expr()?;
            let then = self.block()?;
            let otherwise = if self.is_word("else") {
                self.pos += 1;
                if self.is_word("if") { vec![self.stmt()?] } else { self.block()? }
            } else {
                Vec::new()
            };
            return Ok(Stmt::If { cond, then, otherwise, at });
        }
        if self.is_word("return") {
            self.pos += 1;
            let e = self.expr()?;
            self.expect(";")?;
            return Ok(Stmt::Return(e, at));
        }
        if let (Tok::Ident(name), Tok::Sym(s)) = (self.peek().clone(), self.peek2().clone()) {
            let op = match s {
                "=" => Some(None),
                "+=" => Some(Some(BinOp::Add)),
                "-=" => Some(Some(BinOp::Sub)),
                "*=" => Some(Some(BinOp::Mul)),
                "/=" => Some(Some(BinOp::Div)),
                _ => None,
            };
            if let Some(op) = op {
                self.pos += 2;
                let value = self.expr()?;
                self.expect(";")?;
                return Ok(Stmt::Assign { name, op, value, at });
            }
        }
        Err(self.unexpected("a statement (let, var, an assignment, for, if, return)"))
    }

    pub(crate) fn expr(&mut self) -> P<Expr> {
        self.enter()?;
        let e = self.ternary();
        self.depth -= 1;
        e
    }

    fn ternary(&mut self) -> P<Expr> {
        let c = self.binary(0)?;
        if self.is("?") {
            let at = self.at();
            self.pos += 1;
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Expr::Ternary(Box::new(c), Box::new(a), Box::new(b), at));
        }
        Ok(c)
    }

    fn binop(&self, level: u8) -> Option<BinOp> {
        let Tok::Sym(s) = self.peek() else { return None };
        let (op, l) = match *s {
            "||" => (BinOp::Or, 0),
            "&&" => (BinOp::And, 1),
            "==" => (BinOp::Eq, 2),
            "!=" => (BinOp::Ne, 2),
            "<" => (BinOp::Lt, 3),
            "<=" => (BinOp::Le, 3),
            ">" => (BinOp::Gt, 3),
            ">=" => (BinOp::Ge, 3),
            "+" => (BinOp::Add, 4),
            "-" => (BinOp::Sub, 4),
            "*" => (BinOp::Mul, 5),
            "/" => (BinOp::Div, 5),
            "%" => (BinOp::Mod, 5),
            _ => return None,
        };
        (l == level).then_some(op)
    }

    fn binary(&mut self, level: u8) -> P<Expr> {
        if level > 5 {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        while let Some(op) = self.binop(level) {
            let at = self.at();
            self.pos += 1;
            self.enter()?;
            let rhs = self.binary(level + 1);
            self.depth -= 1;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs?), at);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> P<Expr> {
        let at = self.at();
        for (sym, c) in [("-", '-'), ("!", '!'), ("+", '+')] {
            if self.eat(sym) {
                self.enter()?;
                let e = self.unary();
                self.depth -= 1;
                return Ok(if c == '+' { e? } else { Expr::Unary(c, Box::new(e?), at) });
            }
        }
        self.postfix()
    }

    fn postfix(&mut self) -> P<Expr> {
        let mut e = self.primary()?;
        while self.is(".") {
            let at = self.at();
            self.pos += 1;
            let f = self.ident()?;
            e = Expr::Field(Box::new(e), f, at);
        }
        Ok(e)
    }

    fn primary(&mut self) -> P<Expr> {
        let at = self.at();
        match self.bump() {
            Tok::Num(n) => Ok(Expr::Num(n, at)),
            Tok::Hex(c) => Ok(Expr::Color(c, at)),
            Tok::Sym("(") => {
                let e = self.expr()?;
                // `(a, b)`, `(a, b, c)`, `(a, b, c, d)`: a vector.
                if self.is(",") {
                    let mut items = vec![(None, e)];
                    while self.eat(",") {
                        if self.is(")") {
                            break;
                        }
                        items.push((None, self.expr()?));
                    }
                    self.expect(")")?;
                    let name = match items.len() {
                        2 => "vec2",
                        3 => "vec3",
                        4 => "vec4",
                        n => {
                            return Err(Diagnostic::new(
                                at,
                                format!("a tuple of {n} values; vectors have 2 to 4"),
                            ));
                        }
                    };
                    return Ok(Expr::Call(name.into(), items, at));
                }
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                if self.eat("(") {
                    let mut args = Vec::new();
                    while !self.is(")") {
                        let key = match (self.peek().clone(), self.peek2().clone()) {
                            (Tok::Ident(k), Tok::Sym(":")) => {
                                self.pos += 2;
                                Some(k)
                            }
                            _ => None,
                        };
                        args.push((key, self.expr()?));
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    Ok(Expr::Call(name, args, at))
                } else {
                    Ok(Expr::Name(name, at))
                }
            }
            _ => {
                self.pos -= 1;
                Err(self.unexpected("a value"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> P<File> {
        Parser::new(lex(src, 100_000)?, 64).file()
    }

    #[test]
    fn lexes_numbers_ranges_colours_and_comments() {
        let t = lex("0..1 1.5e2 .5 #f80 #ff8000cc // x\n/* y\n z */ a", 100).unwrap_or_default();
        let toks: Vec<Tok> = t.into_iter().map(|t| t.tok).collect();
        assert_eq!(toks.first(), Some(&Tok::Num(0.0)));
        assert_eq!(toks.get(1), Some(&Tok::Sym("..")));
        assert_eq!(toks.get(3), Some(&Tok::Num(150.0)));
        assert_eq!(toks.get(4), Some(&Tok::Num(0.5)));
        assert!(matches!(toks.get(5), Some(Tok::Hex([r, _, b, a])) if *r == 1.0 && *b == 0.0 && *a == 1.0));
        assert!(matches!(toks.get(6), Some(Tok::Hex([_, _, _, a])) if (*a - 0.8).abs() < 1e-6));
        assert_eq!(toks.get(7), Some(&Tok::Ident("a".into())));
    }

    #[test]
    fn errors_say_where() {
        let e = parse("effect x {\n  pass p {\n    out = ;\n  }\n}").err();
        assert_eq!(e.map(|d| (d.at.line, d.at.col)), Some((3, 11)));
        assert!(parse("effect x { } trailing").is_err());
        assert!(lex("1e99", 10).is_err(), "infinite literals are refused");
    }

    #[test]
    fn deep_nesting_is_refused_not_overflowed() {
        let deep = format!("effect x {{ pass p {{ out = {}1{}; }} }}", "(".repeat(5000), ")".repeat(5000));
        let e = parse(&deep).err();
        assert!(e.is_some_and(|d| d.message.contains("nested")));
        let neg = format!("effect x {{ pass p {{ out = {}1; }} }}", "-".repeat(5000));
        assert!(parse(&neg).is_err());
    }
}
