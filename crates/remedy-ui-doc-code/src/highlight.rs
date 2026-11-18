//! Syntax colors: Sublime Text grammars read by syntect, each piece of code given a kind
//! (keyword, string, comment...) and every kind given a color from the theme, so a theme (and
//! a user's edits to it) colors code like everything else.

use std::ops::Range;
use std::sync::{OnceLock, RwLock};

use remedy_ui_core::Color;
use remedy_ui_style::Theme;
use syntect::parsing::{
    ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxDefinition, SyntaxReference, SyntaxSet,
};

/// What a piece of code is, for its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Anything else.
    Plain,
    /// `fn`, `if`, `return`, `pub`.
    Keyword,
    /// Text in quotes.
    String,
    /// Comments.
    Comment,
    /// Numbers and other literals (`true`, `None`).
    Constant,
    /// Function names where they are defined or called.
    Function,
    /// Type names.
    Type,
    /// Markup tags (`<div>`).
    Tag,
    /// Markup attributes, object keys.
    Attribute,
    /// Operators and punctuation that mean something (`=>`, `&&`).
    Operator,
}

impl Kind {
    /// Every kind, for themes listing their code colors.
    pub const ALL: [Kind; 10] = [
        Kind::Plain,
        Kind::Keyword,
        Kind::String,
        Kind::Comment,
        Kind::Constant,
        Kind::Function,
        Kind::Type,
        Kind::Tag,
        Kind::Attribute,
        Kind::Operator,
    ];

    /// The theme token a theme may set to color this kind (`code-keyword`...).
    pub fn token(self) -> &'static str {
        match self {
            Kind::Plain => "code-plain",
            Kind::Keyword => "code-keyword",
            Kind::String => "code-string",
            Kind::Comment => "code-comment",
            Kind::Constant => "code-constant",
            Kind::Function => "code-function",
            Kind::Type => "code-type",
            Kind::Tag => "code-tag",
            Kind::Attribute => "code-attribute",
            Kind::Operator => "code-operator",
        }
    }

    /// The color of this kind under `theme`: the theme's own `code-*` token when it has one,
    /// else one of its roles.
    pub fn color(self, theme: &Theme) -> Color {
        if let Some(c) = theme.colors.custom.get(self.token()) {
            return *c;
        }
        let d = theme.derive();
        let c = &theme.colors;
        match self {
            Kind::Plain | Kind::Operator => d.text,
            Kind::Keyword => remedy_ui_style::ensure_contrast(c.accent, c.card, 4.5),
            Kind::String => remedy_ui_style::ensure_contrast(c.ok, c.card, 4.5),
            Kind::Comment => d.muted,
            Kind::Constant => remedy_ui_style::ensure_contrast(c.warn, c.card, 4.5),
            Kind::Function => remedy_ui_style::ensure_contrast(c.accent.mix_oklab(d.text, 0.45), c.card, 4.5),
            Kind::Type => remedy_ui_style::ensure_contrast(c.ok.mix_oklab(c.accent, 0.5), c.card, 4.5),
            Kind::Tag => remedy_ui_style::ensure_contrast(c.danger, c.card, 4.5),
            Kind::Attribute => remedy_ui_style::ensure_contrast(c.warn.mix_oklab(c.accent, 0.5), c.card, 4.5),
        }
    }
}

/// The kind a scope name means, by the conventions every Sublime grammar follows.
fn kind_of(scope: &str) -> Option<Kind> {
    let starts = |p: &str| scope == p || scope.starts_with(&format!("{p}."));
    Some(if starts("comment") {
        Kind::Comment
    } else if starts("string") {
        Kind::String
    } else if starts("constant") {
        Kind::Constant
    } else if starts("entity.name.function") || starts("support.function") || starts("meta.function-call") {
        Kind::Function
    } else if starts("entity.name.type")
        || starts("entity.name.class")
        || starts("entity.name.struct")
        || starts("entity.name.enum")
        || starts("support.type")
        || starts("support.class")
    {
        Kind::Type
    } else if starts("keyword.operator") {
        Kind::Operator
    } else if starts("keyword") || starts("storage") {
        // Grammars mark `fn`, `let` and `struct` as `storage.type`: keywords to read.
        Kind::Keyword
    } else if starts("entity.name.tag") {
        Kind::Tag
    } else if starts("entity.other.attribute-name") || starts("meta.mapping.key") {
        Kind::Attribute
    } else {
        return None;
    })
}

/// The kind a stack of scopes gives (the innermost one that means something).
fn kind_of_stack(stack: &ScopeStack) -> Kind {
    stack.as_slice().iter().rev().find_map(|s: &Scope| kind_of(&s.build_string())).unwrap_or(Kind::Plain)
}

fn set() -> &'static RwLock<SyntaxSet> {
    static SET: OnceLock<RwLock<SyntaxSet>> = OnceLock::new();
    SET.get_or_init(|| {
        #[cfg(feature = "syntaxes")]
        let s = SyntaxSet::load_defaults_newlines();
        #[cfg(not(feature = "syntaxes"))]
        let s = SyntaxSet::new();
        RwLock::new(s)
    })
}

/// Why a grammar could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrammarError(pub String);

impl std::fmt::Display for GrammarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the grammar could not be read: {}", self.0)
    }
}

impl std::error::Error for GrammarError {}

/// Adds a grammar (a `.sublime-syntax` file's text) for every code view from now on: an
/// app's own language, or one a user brought. A grammar for a name already known replaces it.
pub fn add_grammar(sublime_syntax: &str) -> Result<String, GrammarError> {
    let def = SyntaxDefinition::load_from_str(sublime_syntax, true, None)
        .map_err(|e| GrammarError(e.to_string()))?;
    let name = def.name.clone();
    let mut slot = set().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut builder = slot.clone().into_builder();
    builder.add(def);
    *slot = builder.build();
    Ok(name)
}

/// The names of the languages code can be colored in.
pub fn languages() -> Vec<String> {
    let s = set().read().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut names: Vec<String> = s.syntaxes().iter().filter(|x| !x.hidden).map(|x| x.name.clone()).collect();
    names.sort();
    names
}

fn find<'a>(s: &'a SyntaxSet, language: &str) -> Option<&'a SyntaxReference> {
    let l = language.trim().trim_start_matches('.');
    if l.is_empty() {
        return None;
    }
    s.find_syntax_by_token(l)
        .or_else(|| s.find_syntax_by_name(l))
        .or_else(|| s.syntaxes().iter().find(|x| x.name.eq_ignore_ascii_case(l)))
}

/// True when code in `language` (a name such as "Rust", a file extension such as "rs", or a
/// Markdown fence word) can be colored.
pub fn knows(language: &str) -> bool {
    let s = set().read().unwrap_or_else(std::sync::PoisonError::into_inner);
    find(&s, language).is_some()
}

/// Each line of `text` as runs of kinds: byte ranges within the line (without its `\n`).
/// Unknown languages (and lines a grammar fails on) come back plain.
pub fn highlight(text: &str, language: &str) -> Vec<Vec<(Range<usize>, Kind)>> {
    let s = set().read().unwrap_or_else(std::sync::PoisonError::into_inner);
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(syntax) = find(&s, language) else {
        return lines
            .iter()
            .map(|l| if l.is_empty() { Vec::new() } else { vec![(0..l.len(), Kind::Plain)] })
            .collect();
    };
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::with_capacity(lines.len());
    let mut line_buf = String::new();
    for line in lines {
        line_buf.clear();
        line_buf.push_str(line);
        // Grammars expect the line break.
        line_buf.push('\n');
        let ops: Vec<(usize, ScopeStackOp)> = state.parse_line(&line_buf, &s).unwrap_or_default();
        let mut runs: Vec<(Range<usize>, Kind)> = Vec::new();
        let mut at = 0;
        let push = |runs: &mut Vec<(Range<usize>, Kind)>, r: Range<usize>, k: Kind| {
            if r.start >= r.end {
                return;
            }
            match runs.last_mut() {
                Some((last, lk)) if *lk == k && last.end == r.start => last.end = r.end,
                _ => runs.push((r, k)),
            }
        };
        for (pos, op) in ops {
            let pos = pos.min(line.len());
            push(&mut runs, at..pos, kind_of_stack(&stack));
            at = pos.max(at);
            let _ = stack.apply(&op);
        }
        push(&mut runs, at..line.len(), kind_of_stack(&stack));
        out.push(runs);
    }
    out
}
