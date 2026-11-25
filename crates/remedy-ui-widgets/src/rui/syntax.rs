//! The `.rui` language: elements with named arguments and children.
//!
//! ```text
//! // settings_voice.rui
//! column(gap: s3) {
//!     section(title: @t("Input")) {
//!         select(label: @t("Device"), items: $input_devices, selected: $input_device)
//!         slider(label: @t("Gain"), value: $input_gain, range: 0..200, step: 1)
//!     }
//!     switch(label: @t("Noise suppression"), checked: $noise_suppression)
//! }
//! ```
//!
//! A file holds one element. An element is a name, optional arguments in parentheses and
//! optional children in braces. Argument values are literals (numbers, strings, `true`,
//! `false`, ranges `a..b`), tokens (`s3`, `center`), the signals (`$name`) and commands
//! (`!name`) the app exposes for the file, and translations (`@t("text")`). There are no
//! other expressions, so a file is data: loading a stranger's cannot run anything.

use std::fmt;

/// A place in a file: line and column, both from 1 (columns count characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pos {
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in characters.
    pub col: u32,
}

/// What went wrong in a `.rui` file, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuiError {
    /// The file's name as the app gave it.
    pub file: String,
    /// Where.
    pub at: Pos,
    /// What.
    pub message: String,
    /// What would be right (the names the app exposes, the arguments an element takes).
    pub help: Option<String>,
}

impl RuiError {
    pub(crate) fn new(file: &str, at: Pos, message: impl Into<String>) -> Self {
        RuiError { file: file.to_owned(), at, message: message.into(), help: None }
    }

    pub(crate) fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl fmt::Display for RuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: {}", self.file, self.at.line, self.at.col, self.message)?;
        if let Some(h) = &self.help {
            write!(f, " ({h})")?;
        }
        Ok(())
    }
}

impl std::error::Error for RuiError {}

/// An argument's value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// `12`, `-0.5`.
    Number(f64),
    /// `"text"`.
    Str(String),
    /// `true`, `false`.
    Bool(bool),
    /// `0..200`.
    Range(f64, f64),
    /// A bare word: a design token (`s3`) or a keyword (`center`).
    Token(String),
    /// `$name`: a signal the app exposes.
    Signal(String),
    /// `!name`: a command the app exposes.
    Command(String),
    /// `@t("text")`: text in the person's language.
    Translate(String),
}

impl Value {
    /// How the value is written, for messages.
    pub fn describe(&self) -> &'static str {
        match self {
            Value::Number(_) => "a number",
            Value::Str(_) => "a string",
            Value::Bool(_) => "true or false",
            Value::Range(..) => "a range",
            Value::Token(_) => "a word",
            Value::Signal(_) => "a signal",
            Value::Command(_) => "a command",
            Value::Translate(_) => "a translation",
        }
    }
}

/// `name: value`.
#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    /// The argument's name.
    pub name: String,
    /// Its value.
    pub value: Value,
    /// Where the name is.
    pub at: Pos,
    /// Where the value is.
    pub value_at: Pos,
}

/// An element.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What it is (`column`, `switch`).
    pub name: String,
    /// Where its name is.
    pub at: Pos,
    /// Its arguments, in order.
    pub args: Vec<Arg>,
    /// What is inside it.
    pub children: Vec<Node>,
}

impl Node {
    /// The argument named `name`.
    pub fn arg(&self, name: &str) -> Option<&Arg> {
        self.args.iter().find(|a| a.name == name)
    }
}

/// A parsed `.rui` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Markup {
    /// The file's name, for messages.
    pub file: String,
    /// Its element.
    pub root: Node,
}

/// How deeply elements may nest. Parsing, checking and building walk the tree recursively
/// on the UI thread, whose stack is fixed when the thread starts (1 MiB on Windows), so a
/// file from a stranger nested a million deep must be refused rather than crash the app.
/// Real screens nest a few dozen deep; this leaves room for any of them within the smallest
/// UI-thread stack the bridges run on. The default of [`set_max_depth`].
pub const MAX_DEPTH: usize = 128;

static DEPTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(MAX_DEPTH);

/// Sets how deeply elements may nest in every file parsed from now on (default
/// [`MAX_DEPTH`], 128). A safety bound: parsing, checking and building recurse once a level
/// on the UI thread's stack, so a much deeper bound lets a file from a stranger overflow it.
/// Raise it only for markup you trust, on a UI thread with the stack for it.
pub fn set_max_depth(depth: usize) {
    DEPTH.store(depth, std::sync::atomic::Ordering::Relaxed);
}

/// How deeply elements may nest now ([`set_max_depth`]).
pub fn max_depth() -> usize {
    DEPTH.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Number(f64),
    Str(String),
    Punct(&'static str),
}

struct Lexer<'a> {
    file: &'a str,
    chars: Vec<char>,
    i: usize,
    pos: Pos,
}

impl<'a> Lexer<'a> {
    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.i + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.i).copied()?;
        self.i += 1;
        if c == '\n' {
            self.pos.line += 1;
            self.pos.col = 1;
        } else {
            self.pos.col += 1;
        }
        Some(c)
    }

    fn err(&self, at: Pos, m: impl Into<String>) -> RuiError {
        RuiError::new(self.file, at, m)
    }

    fn tokens(mut self) -> Result<Vec<(Tok, Pos)>, RuiError> {
        let mut out = Vec::new();
        loop {
            // Space and comments.
            match (self.peek(0), self.peek(1)) {
                (Some(c), _) if c.is_whitespace() => {
                    self.bump();
                    continue;
                }
                (Some('/'), Some('/')) => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                    continue;
                }
                (Some('/'), Some('*')) => {
                    let at = self.pos;
                    self.bump();
                    self.bump();
                    loop {
                        match (self.peek(0), self.peek(1)) {
                            (Some('*'), Some('/')) => {
                                self.bump();
                                self.bump();
                                break;
                            }
                            (Some(_), _) => {
                                self.bump();
                            }
                            (None, _) => return Err(self.err(at, "this comment is never closed")),
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let at = self.pos;
            let Some(c) = self.peek(0) else { break };
            if c.is_alphabetic() || c == '_' {
                let mut s = String::new();
                while let Some(c) = self.peek(0).filter(|c| c.is_alphanumeric() || *c == '_') {
                    s.push(c);
                    self.bump();
                }
                out.push((Tok::Ident(s), at));
            } else if c.is_ascii_digit() || (c == '-' && self.peek(1).is_some_and(|d| d.is_ascii_digit())) {
                let mut s = String::new();
                if c == '-' {
                    s.push('-');
                    self.bump();
                }
                while let Some(d) = self.peek(0).filter(char::is_ascii_digit) {
                    s.push(d);
                    self.bump();
                }
                // A fraction only when a digit follows the point: `0..200` is a range.
                if self.peek(0) == Some('.') && self.peek(1).is_some_and(|d| d.is_ascii_digit()) {
                    s.push('.');
                    self.bump();
                    while let Some(d) = self.peek(0).filter(char::is_ascii_digit) {
                        s.push(d);
                        self.bump();
                    }
                }
                let n: f64 = s.parse().map_err(|_| self.err(at, format!("`{s}` is not a number")))?;
                out.push((Tok::Number(n), at));
            } else if c == '"' {
                self.bump();
                let mut s = String::new();
                loop {
                    match self.bump() {
                        Some('"') => break,
                        Some('\\') => {
                            let esc_at = self.pos;
                            match self.bump() {
                                Some('n') => s.push('\n'),
                                Some('t') => s.push('\t'),
                                Some('"') => s.push('"'),
                                Some('\\') => s.push('\\'),
                                Some('u') if self.peek(0) == Some('{') => {
                                    self.bump();
                                    let mut hex = String::new();
                                    while let Some(h) = self.peek(0).filter(char::is_ascii_hexdigit) {
                                        hex.push(h);
                                        self.bump();
                                    }
                                    if self.bump() != Some('}') {
                                        return Err(
                                            self.err(esc_at, "a `\\u{...}` escape needs its closing brace")
                                        );
                                    }
                                    let ch = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                                    s.push(ch.ok_or_else(|| {
                                        self.err(esc_at, format!("`{hex}` is not a character"))
                                    })?);
                                }
                                _ => {
                                    return Err(self
                                        .err(esc_at, "unknown escape")
                                        .help("use \\n, \\t, \\\", \\\\ or \\u{hex}"));
                                }
                            }
                        }
                        Some(ch) => s.push(ch),
                        None => return Err(self.err(at, "this string is never closed")),
                    }
                }
                out.push((Tok::Str(s), at));
            } else {
                let p: &'static str = match (c, self.peek(1)) {
                    ('.', Some('.')) => "..",
                    ('(', _) => "(",
                    (')', _) => ")",
                    ('{', _) => "{",
                    ('}', _) => "}",
                    (':', _) => ":",
                    (',', _) => ",",
                    ('$', _) => "$",
                    ('!', _) => "!",
                    ('@', _) => "@",
                    _ => return Err(self.err(at, format!("unexpected `{c}`"))),
                };
                for _ in 0..p.len() {
                    self.bump();
                }
                out.push((Tok::Punct(p), at));
            }
        }
        Ok(out)
    }
}

struct Parser<'a> {
    file: &'a str,
    toks: Vec<(Tok, Pos)>,
    i: usize,
    end: Pos,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|(t, _)| t)
    }

    fn at(&self) -> Pos {
        self.toks.get(self.i).map_or(self.end, |(_, p)| *p)
    }

    fn err(&self, m: impl Into<String>) -> RuiError {
        RuiError::new(self.file, self.at(), m)
    }

    fn punct(&mut self, p: &str) -> bool {
        if self.peek()
            == Some(&Tok::Punct(match p {
                "(" => "(",
                ")" => ")",
                "{" => "{",
                "}" => "}",
                ":" => ":",
                "," => ",",
                ".." => "..",
                "$" => "$",
                "!" => "!",
                _ => "@",
            }))
        {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self, what: &str) -> Result<(String, Pos), RuiError> {
        let at = self.at();
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                self.i += 1;
                Ok((s, at))
            }
            _ => Err(self.err(format!("expected {what}"))),
        }
    }

    fn node(&mut self, depth: usize) -> Result<Node, RuiError> {
        let (name, at) = self.ident("an element name")?;
        let max = max_depth();
        if depth >= max {
            return Err(RuiError::new(self.file, at, format!("elements nest more than {max} deep"))
                .help("flatten the layout"));
        }
        let mut args: Vec<Arg> = Vec::new();
        if self.punct("(") {
            while !self.punct(")") {
                let (an, aat) = self.ident("an argument name or `)`")?;
                if !self.punct(":") {
                    return Err(self.err(format!("expected `:` after `{an}`")));
                }
                let value_at = self.at();
                let value = self.value()?;
                if args.iter().any(|a| a.name == an) {
                    return Err(RuiError::new(self.file, aat, format!("`{an}` is given twice")));
                }
                args.push(Arg { name: an, value, at: aat, value_at });
                if !self.punct(",") {
                    if self.punct(")") {
                        break;
                    }
                    return Err(self.err("expected `,` or `)`"));
                }
            }
        }
        let mut children = Vec::new();
        if self.punct("{") {
            while !self.punct("}") {
                if self.peek().is_none() {
                    return Err(RuiError::new(self.file, at, format!("`{name}`'s `{{` is never closed")));
                }
                children.push(self.node(depth + 1)?);
            }
        }
        Ok(Node { name, at, args, children })
    }

    fn value(&mut self) -> Result<Value, RuiError> {
        let at = self.at();
        let Some(t) = self.peek().cloned() else { return Err(self.err("expected a value")) };
        self.i += 1;
        Ok(match t {
            Tok::Number(a) => {
                if self.punct("..") {
                    match self.peek().cloned() {
                        Some(Tok::Number(b)) => {
                            self.i += 1;
                            if b < a {
                                return Err(RuiError::new(
                                    self.file,
                                    at,
                                    "a range must not end before it starts",
                                ));
                            }
                            Value::Range(a, b)
                        }
                        _ => return Err(self.err("expected the number that ends the range")),
                    }
                } else {
                    Value::Number(a)
                }
            }
            Tok::Str(s) => Value::Str(s),
            Tok::Ident(s) if s == "true" => Value::Bool(true),
            Tok::Ident(s) if s == "false" => Value::Bool(false),
            Tok::Ident(s) => Value::Token(s),
            Tok::Punct("$") => Value::Signal(self.ident("a signal name after `$`")?.0),
            Tok::Punct("!") => Value::Command(self.ident("a command name after `!`")?.0),
            Tok::Punct("@") => {
                let (f, fat) = self.ident("`t` after `@`")?;
                if f != "t" {
                    return Err(RuiError::new(self.file, fat, format!("`@{f}` is not known"))
                        .help("use @t(\"text\")"));
                }
                if !self.punct("(") {
                    return Err(self.err("expected `(` after `@t`"));
                }
                let s = match self.peek().cloned() {
                    Some(Tok::Str(s)) => {
                        self.i += 1;
                        s
                    }
                    _ => return Err(self.err("expected the text to translate, in quotes")),
                };
                if !self.punct(")") {
                    return Err(self.err("expected `)`"));
                }
                Value::Translate(s)
            }
            Tok::Punct(p) => {
                return Err(RuiError::new(self.file, at, format!("expected a value, found `{p}`")));
            }
        })
    }
}

impl Markup {
    /// Parses `source`, naming it `file` in messages.
    pub fn parse(file: &str, source: &str) -> Result<Markup, RuiError> {
        let lexer = Lexer { file, chars: source.chars().collect(), i: 0, pos: Pos { line: 1, col: 1 } };
        let end = {
            let mut p = Pos { line: 1, col: 1 };
            for c in source.chars() {
                if c == '\n' {
                    p.line += 1;
                    p.col = 1;
                } else {
                    p.col += 1;
                }
            }
            p
        };
        let toks = lexer.tokens()?;
        let mut parser = Parser { file, toks, i: 0, end };
        if parser.peek().is_none() {
            return Err(RuiError::new(file, end, "the file holds no element"));
        }
        let root = parser.node(0)?;
        if parser.peek().is_some() {
            return Err(parser.err("a file holds one element; put several inside a `column` or `row`"));
        }
        Ok(Markup { file: file.to_owned(), root })
    }
}
