//! What each element takes: one table the checker, the interpreter and the compiler share,
//! so a file means the same thing interpreted and compiled.

use crate::rui::syntax::{Markup, Node, RuiError, Value};

/// What an argument takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// Fixed text: `"Send"` or `@t("Send")`.
    Label,
    /// Text that may change: fixed text or a text signal (`$status`).
    Text,
    /// A flag signal (`$noise_suppression`), read and written.
    Flag,
    /// A number signal (`$gain`), read and written.
    Number,
    /// A chosen index (`$device`), read and written.
    Choice,
    /// A list of choices (`$devices`); the element follows it as it changes.
    Items,
    /// A level from 0 to 1, or none while unknown (`$progress`).
    Level,
    /// A command (`!save`).
    Command,
    /// A range of numbers: `0..200`.
    Range,
    /// A number: `1`, `0.5`.
    Count,
    /// A space: a spacing token (`s3`, three grid units) or pixels (`12`).
    Space,
    /// `true` or `false`.
    Switch,
    /// A word: a keyword (`center`) or a string.
    Word,
}

impl Kind {
    /// The kinds bound to signals the app exposes, and what they hold.
    pub fn signal_type(self) -> Option<&'static str> {
        Some(match self {
            Kind::Text => "String",
            Kind::Flag => "bool",
            Kind::Number => "f64",
            Kind::Choice => "usize",
            Kind::Items => "Vec<String>",
            Kind::Level => "Option<f32>",
            _ => return None,
        })
    }

    /// How it is written, for messages.
    pub fn describe(self) -> &'static str {
        match self {
            Kind::Label => "text in quotes or @t(\"...\")",
            Kind::Text => "text in quotes, @t(\"...\") or a text signal",
            Kind::Flag => "a flag signal ($name)",
            Kind::Number => "a number signal ($name)",
            Kind::Choice => "a choice signal ($name)",
            Kind::Items => "a list signal ($name)",
            Kind::Level => "a level signal ($name)",
            Kind::Command => "a command (!name)",
            Kind::Range => "a range (0..100)",
            Kind::Count => "a number",
            Kind::Space => "a spacing token (s0_5, s1, s2...) or pixels",
            Kind::Switch => "true or false",
            Kind::Word => "a word",
        }
    }

    /// What it is, in a few words, for messages about how a signal is used.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Text => "a text signal",
            Kind::Flag => "a flag signal",
            Kind::Number => "a number signal",
            Kind::Choice => "a choice signal",
            Kind::Items => "a list signal",
            Kind::Level => "a level signal",
            other => other.describe(),
        }
    }

    /// Whether `v` is written the way this kind takes (signals and commands are checked
    /// against what the app exposes separately).
    pub fn accepts(self, v: &Value) -> bool {
        match (self, v) {
            (Kind::Label, Value::Str(_) | Value::Translate(_)) => true,
            (Kind::Text, Value::Str(_) | Value::Translate(_) | Value::Signal(_)) => true,
            (Kind::Flag | Kind::Number | Kind::Choice | Kind::Items | Kind::Level, Value::Signal(_)) => true,
            (Kind::Command, Value::Command(_)) => true,
            (Kind::Range, Value::Range(..)) => true,
            (Kind::Count, Value::Number(_)) => true,
            (Kind::Space, Value::Number(_)) => true,
            (Kind::Space, Value::Token(t)) => space_token(t).is_some(),
            (Kind::Switch, Value::Bool(_)) => true,
            (Kind::Word, Value::Token(_) | Value::Str(_)) => true,
            _ => false,
        }
    }
}

/// A spacing word (`s3`, `s0_5`, `s12`) as the theme's spacing token: the scale every Remedy
/// app shares (design 13), so markup and Rust use the same steps.
pub fn space_token(token: &str) -> Option<remedy_ui_core::style::Space> {
    remedy_ui_core::style::Space::from_name(token)
}

/// The spacing words markup takes, in order, for messages.
pub fn space_names() -> String {
    remedy_ui_core::style::Space::ALL.iter().map(|s| s.name()).collect::<Vec<_>>().join(", ")
}

/// A slider's step when the file gives none: a hundredth of the range.
pub fn default_step(min: f64, max: f64) -> f64 {
    let span = max - min;
    if span > 0.0 { span / 100.0 } else { 1.0 }
}

/// One argument an element takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Param {
    /// Its name.
    pub name: &'static str,
    /// What it takes.
    pub kind: Kind,
    /// It must be given.
    pub required: bool,
}

const fn p(name: &'static str, kind: Kind) -> Param {
    Param { name, kind, required: true }
}

const fn opt(name: &'static str, kind: Kind) -> Param {
    Param { name, kind, required: false }
}

/// One element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Its name in markup.
    pub name: String,
    /// What it takes, besides [`COMMON`].
    pub params: Vec<Param>,
    /// It holds children.
    pub children: bool,
}

/// What every element takes: layout and style.
pub const COMMON: &[Param] = &[
    opt("gap", Kind::Space),
    opt("padding", Kind::Space),
    opt("grow", Kind::Count),
    opt("fill_width", Kind::Switch),
    opt("variant", Kind::Word),
    opt("align", Kind::Word),
    opt("justify", Kind::Word),
    opt("visible", Kind::Flag),
];

/// The words `align` takes.
pub const ALIGNS: &[&str] = &["start", "center", "end", "stretch", "baseline"];
/// The words `justify` takes.
pub const JUSTIFIES: &[&str] = &["start", "center", "end", "space_between", "space_around"];

/// The elements every file can use.
pub fn built_in() -> Vec<Spec> {
    use Kind::*;
    let s = |name: &str, params: &[Param], children: bool| Spec {
        name: name.to_owned(),
        params: params.to_vec(),
        children,
    };
    vec![
        s("column", &[], true),
        s("row", &[], true),
        s("wrap", &[], true),
        s("card", &[], true),
        s("scroll", &[], true),
        s("section", &[p("title", Label)], true),
        s("spacer", &[], false),
        s("divider", &[opt("vertical", Switch)], false),
        s("text", &[p("text", Text)], false),
        s("heading", &[p("text", Text), opt("level", Count)], false),
        s("badge", &[p("text", Text)], false),
        s("button", &[p("label", Label), p("on_press", Command)], false),
        s("link", &[p("label", Label), p("on_press", Command)], false),
        s("switch", &[p("label", Label), p("checked", Flag)], false),
        s("check", &[p("label", Label), p("checked", Flag)], false),
        s("toggle_button", &[p("label", Label), p("on", Flag)], false),
        s("chip", &[p("label", Label), p("selected", Flag)], false),
        s("lamp", &[p("label", Label), p("on", Flag)], false),
        s("select", &[p("label", Label), p("items", Items), p("selected", Choice)], false),
        s("radio_group", &[p("label", Label), p("items", Items), p("selected", Choice)], false),
        s("segmented", &[p("label", Label), p("items", Items), p("selected", Choice)], false),
        s("slider", &[p("label", Label), p("value", Number), p("range", Range), opt("step", Count)], false),
        s("stepper", &[p("label", Label), p("value", Number), p("range", Range), opt("step", Count)], false),
        s("progress", &[p("label", Label), p("value", Level)], false),
        s("spinner", &[p("label", Label)], false),
        s("empty_state", &[p("title", Label), p("hint", Label)], false),
        s("text_input", &[p("label", Label), p("value", Text)], false),
        s("text_area", &[p("label", Label), p("value", Text)], false),
    ]
}

/// What a file's `$name` and `!name` references are, found by where they are used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uses {
    /// Each signal, the kind it is used as, in first-use order.
    pub signals: Vec<(String, Kind)>,
    /// Each command, in first-use order.
    pub commands: Vec<String>,
}

/// Checks `markup` against `specs`: every element known, every argument known and written
/// the way it takes, every required one given, children only where they belong, a signal
/// used as one kind throughout, and fixed choices (`align`, `justify`) among the known words.
/// Returns what the file references, for the app's side to provide.
pub fn check(markup: &Markup, specs: &[Spec]) -> Result<Uses, RuiError> {
    let mut uses = Uses::default();
    check_node(&markup.file, &markup.root, specs, &mut uses)?;
    Ok(uses)
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> String {
    let mut v: Vec<&str> = it.collect();
    v.sort_unstable();
    v.join(", ")
}

fn check_node(file: &str, n: &Node, specs: &[Spec], uses: &mut Uses) -> Result<(), RuiError> {
    let Some(spec) = specs.iter().find(|s| s.name == n.name) else {
        return Err(RuiError::new(file, n.at, format!("no element is called `{}`", n.name))
            .help(format!("known: {}", names(specs.iter().map(|s| s.name.as_str())))));
    };
    let all = || spec.params.iter().chain(COMMON);
    for a in &n.args {
        let Some(param) = all().find(|p| p.name == a.name) else {
            return Err(RuiError::new(file, a.at, format!("`{}` takes no `{}`", n.name, a.name))
                .help(format!("it takes: {}", names(all().map(|p| p.name)))));
        };
        if !param.kind.accepts(&a.value) {
            let err = RuiError::new(
                file,
                a.value_at,
                format!("`{}` takes {}, not {}", a.name, param.kind.describe(), a.value.describe()),
            );
            return Err(if param.kind == Kind::Space {
                err.help(format!("spacing: {}", space_names()))
            } else {
                err
            });
        }
        let word = |list: &[&str]| match &a.value {
            Value::Token(w) | Value::Str(w) if !list.contains(&w.as_str()) => {
                Err(RuiError::new(file, a.value_at, format!("`{}` takes no `{w}`", a.name))
                    .help(list.join(", ")))
            }
            _ => Ok(()),
        };
        match a.name.as_str() {
            "align" => word(ALIGNS)?,
            "justify" => word(JUSTIFIES)?,
            _ => {}
        }
        match &a.value {
            Value::Signal(s) => {
                // Text arguments that take fixed text or a signal make it a text signal.
                let kind = param.kind;
                match uses.signals.iter().find(|(name, _)| name == s) {
                    Some((_, k)) if *k != kind => {
                        return Err(RuiError::new(
                            file,
                            a.value_at,
                            format!("`${s}` is used as {} here but as {} before", kind.noun(), k.noun()),
                        ));
                    }
                    Some(_) => {}
                    None => uses.signals.push((s.clone(), kind)),
                }
            }
            Value::Command(c) if !uses.commands.contains(c) => uses.commands.push(c.clone()),
            _ => {}
        }
    }
    for p in &spec.params {
        if p.required && n.arg(p.name).is_none() {
            return Err(RuiError::new(file, n.at, format!("`{}` needs `{}`", n.name, p.name)).help(format!(
                "`{}` takes {}",
                p.name,
                p.kind.describe()
            )));
        }
    }
    if let Some(first) = n.children.first().filter(|_| !spec.children) {
        return Err(RuiError::new(file, first.at, format!("`{}` holds no children", n.name)));
    }
    for c in &n.children {
        check_node(file, c, specs, uses)?;
    }
    Ok(())
}
