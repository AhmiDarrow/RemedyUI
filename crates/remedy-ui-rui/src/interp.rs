//! Building elements from a parsed file at run time: for live editing and for people's
//! own layouts. Everything is checked before the first element is made, so a broken file
//! builds nothing.

use std::collections::HashMap;
use std::rc::Rc;

use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

use crate::rt::{self, Common, Space, w};
use crate::spec::{self, Kind, Param, Spec};
use crate::syntax::{Markup, Node, RuiError, Value};

/// A signal or command the app exposes to a file.
#[derive(Clone)]
pub enum Binding {
    /// Text.
    Text(Signal<String>),
    /// A flag.
    Flag(Signal<bool>),
    /// A number.
    Number(Signal<f64>),
    /// A chosen index.
    Choice(Signal<usize>),
    /// A list of choices.
    Items(Signal<Vec<String>>),
    /// A level from 0 to 1, or none while unknown.
    Level(Signal<Option<f32>>),
    /// A command.
    Command(Rc<dyn Fn()>),
}

impl Binding {
    fn kind(&self) -> Kind {
        match self {
            Binding::Text(_) => Kind::Text,
            Binding::Flag(_) => Kind::Flag,
            Binding::Number(_) => Kind::Number,
            Binding::Choice(_) => Kind::Choice,
            Binding::Items(_) => Kind::Items,
            Binding::Level(_) => Kind::Level,
            Binding::Command(_) => Kind::Command,
        }
    }
}

impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.kind())
    }
}

/// What the app exposes to a file: its signals, its commands and how it translates.
#[derive(Clone)]
pub struct Scope {
    bindings: HashMap<String, Binding>,
    translate: Rc<dyn Fn(&str) -> String>,
}

impl Default for Scope {
    fn default() -> Self {
        Scope { bindings: HashMap::new(), translate: Rc::new(str::to_owned) }
    }
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(&self.bindings).finish()
    }
}

impl Scope {
    /// Nothing exposed; `@t` keeps text as written.
    pub fn new() -> Self {
        Scope::default()
    }

    /// Exposes `binding` as `name`.
    pub fn bind(mut self, name: &str, binding: Binding) -> Self {
        self.bindings.insert(name.to_owned(), binding);
        self
    }

    /// Exposes a text signal.
    pub fn text(self, name: &str, s: Signal<String>) -> Self {
        self.bind(name, Binding::Text(s))
    }

    /// Exposes a flag signal.
    pub fn flag(self, name: &str, s: Signal<bool>) -> Self {
        self.bind(name, Binding::Flag(s))
    }

    /// Exposes a number signal.
    pub fn number(self, name: &str, s: Signal<f64>) -> Self {
        self.bind(name, Binding::Number(s))
    }

    /// Exposes a choice signal.
    pub fn choice(self, name: &str, s: Signal<usize>) -> Self {
        self.bind(name, Binding::Choice(s))
    }

    /// Exposes a list of choices.
    pub fn items(self, name: &str, s: Signal<Vec<String>>) -> Self {
        self.bind(name, Binding::Items(s))
    }

    /// Exposes a level.
    pub fn level(self, name: &str, s: Signal<Option<f32>>) -> Self {
        self.bind(name, Binding::Level(s))
    }

    /// Exposes a command.
    pub fn command(self, name: &str, f: impl Fn() + 'static) -> Self {
        self.bind(name, Binding::Command(Rc::new(f)))
    }

    /// Translates `@t("...")` with `f` (the app's message catalog).
    pub fn translator(mut self, f: impl Fn(&str) -> String + 'static) -> Self {
        self.translate = Rc::new(f);
        self
    }

    /// `text` in the person's language.
    pub fn tr(&self, text: &str) -> String {
        (self.translate)(text)
    }

    /// The exposed names of `kind`, sorted, for messages.
    pub fn names_of(&self, kind: Kind) -> Vec<String> {
        let mut v: Vec<String> =
            self.bindings.iter().filter(|(_, b)| b.kind() == kind).map(|(n, _)| n.clone()).collect();
        v.sort_unstable();
        v
    }

    /// The binding called `name`.
    pub fn get(&self, name: &str) -> Option<&Binding> {
        self.bindings.get(name)
    }
}

/// An argument's value, resolved against the scope, for the app's own elements.
#[derive(Clone, Debug)]
pub enum Bound {
    /// Fixed text (already translated).
    Label(String),
    /// A binding.
    Binding(Binding),
    /// A range.
    Range(f64, f64),
    /// A number.
    Count(f64),
    /// A space.
    Space(Space),
    /// `true` or `false`.
    Switch(bool),
    /// A word.
    Word(String),
}

/// One use of an app's own element: its arguments and children.
#[derive(Debug, Default)]
pub struct Call {
    args: HashMap<String, Bound>,
    /// What the file put inside it, built.
    pub children: Vec<El>,
}

impl Call {
    /// A call with nothing given (for the compiler's code).
    pub fn new() -> Self {
        Call::default()
    }

    /// Gives `name` (for the compiler's code).
    pub fn arg(mut self, name: &str, v: Bound) -> Self {
        self.args.insert(name.to_owned(), v);
        self
    }

    /// Adds a child (for the compiler's code).
    pub fn child(mut self, el: El) -> Self {
        self.children.push(el);
        self
    }

    /// The argument `name`, if given.
    pub fn get(&self, name: &str) -> Option<&Bound> {
        self.args.get(name)
    }

    /// Fixed text given as `name`.
    pub fn label(&self, name: &str) -> Option<&str> {
        match self.args.get(name)? {
            Bound::Label(s) | Bound::Word(s) => Some(s),
            _ => None,
        }
    }

    /// A flag signal given as `name`.
    pub fn flag(&self, name: &str) -> Option<Signal<bool>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Flag(s)) => Some(*s),
            _ => None,
        }
    }

    /// A number signal given as `name`.
    pub fn number(&self, name: &str) -> Option<Signal<f64>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Number(s)) => Some(*s),
            _ => None,
        }
    }

    /// A text signal given as `name`.
    pub fn text(&self, name: &str) -> Option<Signal<String>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Text(s)) => Some(*s),
            _ => None,
        }
    }

    /// A choice signal given as `name`.
    pub fn choice(&self, name: &str) -> Option<Signal<usize>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Choice(s)) => Some(*s),
            _ => None,
        }
    }

    /// A list signal given as `name`.
    pub fn items(&self, name: &str) -> Option<Signal<Vec<String>>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Items(s)) => Some(*s),
            _ => None,
        }
    }

    /// A level signal given as `name`.
    pub fn level(&self, name: &str) -> Option<Signal<Option<f32>>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Level(s)) => Some(*s),
            _ => None,
        }
    }

    /// A command given as `name`.
    pub fn command(&self, name: &str) -> Option<Rc<dyn Fn()>> {
        match self.args.get(name)? {
            Bound::Binding(Binding::Command(f)) => Some(f.clone()),
            _ => None,
        }
    }

    /// A number given as `name`.
    pub fn count(&self, name: &str) -> Option<f64> {
        match self.args.get(name)? {
            Bound::Count(n) => Some(*n),
            _ => None,
        }
    }

    /// A range given as `name`.
    pub fn range(&self, name: &str) -> Option<(f64, f64)> {
        match self.args.get(name)? {
            Bound::Range(a, b) => Some((*a, *b)),
            _ => None,
        }
    }

    /// `true` or `false` given as `name`.
    pub fn switch(&self, name: &str) -> Option<bool> {
        match self.args.get(name)? {
            Bound::Switch(b) => Some(*b),
            _ => None,
        }
    }
}

/// Builds one of the app's own elements.
pub type Builder = Rc<dyn Fn(&Ui, Call) -> El>;

/// The elements a file may use: the built-in ones and the app's own.
#[derive(Clone)]
pub struct Widgets {
    specs: Vec<Spec>,
    custom: HashMap<String, Builder>,
}

impl Default for Widgets {
    fn default() -> Self {
        Widgets { specs: spec::built_in(), custom: HashMap::new() }
    }
}

impl std::fmt::Debug for Widgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.specs.iter().map(|s| &s.name)).finish()
    }
}

impl Widgets {
    /// The built-in elements.
    pub fn new() -> Self {
        Widgets::default()
    }

    /// Adds the app's own element `name`, taking `params` (and [`spec::COMMON`], which the
    /// engine applies) and children if `children`. Replaces a built-in of the same name.
    pub fn register(
        &mut self,
        name: &str,
        params: &[Param],
        children: bool,
        build: impl Fn(&Ui, Call) -> El + 'static,
    ) {
        self.specs.retain(|s| s.name != name);
        self.specs.push(Spec { name: name.to_owned(), params: params.to_vec(), children });
        self.custom.insert(name.to_owned(), Rc::new(build));
    }

    /// What each element takes.
    pub fn specs(&self) -> &[Spec] {
        &self.specs
    }
}

/// Checks `markup` against what `widgets` and `scope` offer.
pub fn check(markup: &Markup, scope: &Scope, widgets: &Widgets) -> Result<(), RuiError> {
    let uses = spec::check(markup, widgets.specs())?;
    let find = |name: &str, n: &Node| -> Option<(crate::syntax::Pos, Kind)> {
        let mut stack = vec![n];
        while let Some(n) = stack.pop() {
            for a in &n.args {
                match &a.value {
                    Value::Signal(s) | Value::Command(s) if s == name => {
                        let spec = widgets.specs().iter().find(|s| s.name == n.name)?;
                        let kind = spec.params.iter().chain(spec::COMMON).find(|p| p.name == a.name)?.kind;
                        return Some((a.value_at, kind));
                    }
                    _ => {}
                }
            }
            stack.extend(n.children.iter());
        }
        None
    };
    for (name, kind) in &uses.signals {
        let at = find(name, &markup.root).map(|(p, _)| p).unwrap_or_default();
        let help = |kind: Kind| {
            let names = scope.names_of(kind);
            if names.is_empty() {
                format!("this file is given no {}s", kind.noun().trim_start_matches("a "))
            } else {
                format!("exposed: {}", names.iter().map(|n| format!("${n}")).collect::<Vec<_>>().join(", "))
            }
        };
        match scope.get(name) {
            None => {
                return Err(RuiError::new(
                    &markup.file,
                    at,
                    format!("`${name}` is not exposed to this file"),
                )
                .help(help(*kind)));
            }
            Some(b) if b.kind() != *kind => {
                return Err(RuiError::new(
                    &markup.file,
                    at,
                    format!("`${name}` is {} but this takes {}", describe_binding(b), kind.noun()),
                )
                .help(help(*kind)));
            }
            Some(_) => {}
        }
    }
    for name in &uses.commands {
        if !matches!(scope.get(name), Some(Binding::Command(_))) {
            let at = find(name, &markup.root).map(|(p, _)| p).unwrap_or_default();
            let names = scope.names_of(Kind::Command);
            return Err(RuiError::new(&markup.file, at, format!("`!{name}` is not exposed to this file"))
                .help(if names.is_empty() {
                    "this file is given no commands".to_owned()
                } else {
                    format!(
                        "exposed: {}",
                        names.iter().map(|n| format!("!{n}")).collect::<Vec<_>>().join(", ")
                    )
                }));
        }
    }
    Ok(())
}

fn describe_binding(b: &Binding) -> &'static str {
    match b {
        Binding::Text(_) => "a text signal",
        Binding::Flag(_) => "a flag signal",
        Binding::Number(_) => "a number signal",
        Binding::Choice(_) => "a choice signal",
        Binding::Items(_) => "a list signal",
        Binding::Level(_) => "a level signal",
        Binding::Command(_) => "a command",
    }
}

/// Builds `markup`'s elements with what `scope` exposes. Checks everything first: a file
/// that does not fit builds nothing and says why.
pub fn build(ui: &Ui, markup: &Markup, scope: &Scope, widgets: &Widgets) -> Result<El, RuiError> {
    check(markup, scope, widgets)?;
    Ok(node(ui, &markup.root, scope, widgets))
}

fn resolve(v: &Value, scope: &Scope) -> Bound {
    match v {
        Value::Str(s) => Bound::Label(s.clone()),
        Value::Translate(s) => Bound::Label(scope.tr(s)),
        Value::Signal(s) | Value::Command(s) => match scope.get(s) {
            Some(b) => Bound::Binding(b.clone()),
            // Checked before building; nothing unexposed reaches here.
            None => Bound::Label(String::new()),
        },
        Value::Range(a, b) => Bound::Range(*a, *b),
        Value::Number(n) => Bound::Count(*n),
        Value::Bool(b) => Bound::Switch(*b),
        Value::Token(t) => match spec::space_units(t) {
            Some(n) => Bound::Space(Space::Units(n)),
            None => Bound::Word(t.clone()),
        },
    }
}

fn common(call: &Call) -> Common {
    let space = |name: &str| match call.get(name) {
        Some(Bound::Space(s)) => Some(*s),
        #[allow(clippy::cast_possible_truncation)]
        Some(Bound::Count(n)) => Some(Space::Px(*n as f32)),
        _ => None,
    };
    let word = |name: &str| call.label(name).map(str::to_owned);
    Common {
        gap: space("gap"),
        padding: space("padding"),
        #[allow(clippy::cast_possible_truncation)]
        grow: call.count("grow").map(|g| g as f32),
        fill_width: call.switch("fill_width").unwrap_or(false),
        variant: word("variant").map(|v| rt::intern(&v)),
        align: word("align").and_then(|a| rt::align(&a)),
        justify: word("justify").and_then(|j| rt::justify(&j)),
        visible: call.flag("visible"),
    }
}

fn node(ui: &Ui, n: &Node, scope: &Scope, widgets: &Widgets) -> El {
    let mut call = Call::new();
    for a in &n.args {
        call = call.arg(&a.name, resolve(&a.value, scope));
    }
    let children: Vec<El> = n.children.iter().map(|c| node(ui, c, scope, widgets)).collect();
    let c = common(&call);
    if let Some(custom) = widgets.custom.get(&n.name) {
        call.children = children;
        let el = custom(ui, call);
        return rt::finish(ui, el, &c, Vec::new());
    }
    let label = |name: &str| call.label(name).unwrap_or_default().to_owned();
    // Text arguments that take a signal or fixed text.
    let text_el = |el: El| match call.text("text") {
        Some(s) => rt::live_text(el, s),
        None => el,
    };
    let fixed = || call.label("text").unwrap_or_default().to_owned();
    let flag = |name: &str| call.flag(name).unwrap_or_else(|| Signal::new(false));
    let range = call.range("range").unwrap_or((0.0, 1.0));
    let step = call.count("step").unwrap_or_else(|| rt::default_step(range.0, range.1));
    let el = match n.name.as_str() {
        "column" => w::column(ui),
        "row" => w::row(ui),
        "wrap" => w::wrap(ui),
        "card" => w::card(ui),
        "scroll" => w::scroll(ui),
        "section" => w::section(ui, &label("title")),
        "spacer" => w::spacer(ui),
        "divider" => w::divider(ui, call.switch("vertical").unwrap_or(false)),
        "text" => text_el(w::text(ui, fixed())),
        "heading" => text_el(w::heading(ui, rt::heading_level(call.count("level").unwrap_or(1.0)), fixed())),
        "badge" => text_el(w::badge(ui, fixed())),
        "button" | "link" => {
            let f = call.command("on_press");
            let press = move || {
                if let Some(f) = &f {
                    f();
                }
            };
            if n.name == "button" {
                w::button(ui, label("label"), press)
            } else {
                w::link(ui, label("label"), press)
            }
        }
        "switch" => w::switch(ui, label("label"), flag("checked")),
        "check" => w::check(ui, label("label"), flag("checked")),
        "toggle_button" => w::toggle_button(ui, label("label"), flag("on")),
        "chip" => w::chip(ui, label("label"), flag("selected")),
        "lamp" => w::lamp(ui, label("label"), flag("on")),
        "select" | "radio_group" | "segmented" => {
            let (Some(items), Some(selected)) = (call.items("items"), call.choice("selected")) else {
                return w::column(ui);
            };
            let (ui2, l, which) = (ui.clone(), label("label"), n.name.clone());
            return rt::with_items(ui, &c, items, move |list| match which.as_str() {
                "select" => w::select(&ui2, l.clone(), list.to_vec(), selected),
                "radio_group" => w::radio_group(&ui2, l.clone(), &rt::strs(list), selected),
                _ => w::segmented(&ui2, l.clone(), &rt::strs(list), selected),
            });
        }
        "slider" | "stepper" => {
            let value = call.number("value").unwrap_or_else(|| Signal::new(0.0));
            if n.name == "slider" {
                w::slider(ui, label("label"), value, range.0, range.1, step)
            } else {
                w::stepper(ui, label("label"), value, range.0, range.1, step)
            }
        }
        "progress" => {
            w::progress(ui, label("label"), call.level("value").unwrap_or_else(|| Signal::new(None)))
        }
        "spinner" => w::spinner(ui, label("label")),
        "empty_state" => w::empty_state(ui, label("title"), label("hint")),
        "text_input" | "text_area" => {
            let field = if n.name == "text_input" {
                w::text_input(ui, label("label"))
            } else {
                w::text_area(ui, label("label"))
            };
            match call.text("value") {
                Some(s) => rt::bound_field(field, s),
                None => {
                    field.set_value(call.label("value").unwrap_or_default());
                    field.el()
                }
            }
        }
        // The checker knows every element; nothing else reaches here.
        _ => w::column(ui),
    };
    rt::finish(ui, el, &c, children)
}
