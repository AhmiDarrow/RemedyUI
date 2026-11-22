//! RemedyUI scripted widgets, independent of any language: small programs people write and
//! share (a FreeSpeak Home's custom widgets), run as untrusted code.
//!
//! This crate is what every script engine and every host agrees on:
//!
//! - [`ScriptWidget`]: one loaded widget. It says its name and its [settings](SettingSpec),
//!   describes itself for the current settings and host data ([`ScriptWidget::render`]),
//!   and runs the handlers its buttons name ([`ScriptWidget::act`]), keeping its own state
//!   between calls.
//! - [`ScriptEngine`]: a language that loads widgets from a [`Package`]. Engines are crates
//!   of their own (`remedy-ui-script-rhai` for widgets people write by hand in Rhai,
//!   `remedy-ui-script-wasm` for compiled WebAssembly widgets), and [`Engines`] is the set an
//!   app or a board enables; a package names its [`Language`].
//! - The element-description protocol ([`Node`], read from JSON by [`describe`], or by
//!   [`describe_with`] for apps that offer elements of their own): what a
//!   script returns. It is ordinary RemedyUI elements (columns, rows, text, buttons, links,
//!   progress, dividers), never drawing, so scripted widgets are themed, accessible and fast
//!   like built-in ones, and the host builds every element itself.
//! - [`Limits`]: how much one call may use (operations, time, memory, call depth, element
//!   nesting), sized from the device and configurable. Every engine stops a script that
//!   runs past them with a [`ScriptError`] that says which, and the host keeps running.
//! - [`StepRate`]: how many of an engine's steps a millisecond buys on this device,
//!   calibrated the first time the engine is used and kept current by its calls, so a time
//!   budget becomes a step budget that fits the machine.
//!
//! The protocol, as JSON (each engine documents how its language produces it):
//!
//! ```json
//! { "name": "Claps",
//!   "settings": [ { "key": "title", "label": "Title", "kind": "text", "value": "Claps" },
//!                 { "key": "goal", "kind": "number", "min": 1, "max": 100, "value": 10 } ] }
//!
//! { "kind": "column", "gap": 8, "children": [
//!     { "kind": "text", "text": "Claps", "style": "heading", "level": 2 },
//!     { "kind": "text", "text": "3 claps", "style": "body" },
//!     { "kind": "progress", "label": "Claps", "value": 0.3 },
//!     { "kind": "button", "label": "Clap", "on": "clap" },
//!     { "kind": "link", "label": "Rules", "url": "https://example.com/rules" },
//!     { "kind": "divider" } ] }
//! ```
//!
//! A button's `on` is any JSON value the widget chooses; when the button is pressed the host
//! hands it back through [`ScriptWidget::act`].

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Map, Value};

mod rate;
pub use rate::{MIN_SAMPLE, StepRate, calibrate_with, min_sample};

/// The JSON values settings, host data and button handlers are made of, re-exported so they
/// are the version RemedyUI is built with.
pub use serde_json;

/// The language a widget is written in, named by a short id (`rhai`, `wasm`). Engines for
/// other languages name their own.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Language(Cow<'static, str>);

impl Language {
    /// Rhai: widgets people write by hand (`remedy-ui-script-rhai`).
    pub const RHAI: Language = Language(Cow::Borrowed("rhai"));
    /// WebAssembly: widgets compiled from Rust or any language that targets it
    /// (`remedy-ui-script-wasm`).
    pub const WASM: Language = Language(Cow::Borrowed("wasm"));

    /// The language named `id`.
    pub fn new(id: impl Into<Cow<'static, str>>) -> Language {
        Language(id.into())
    }

    /// Its id.
    pub fn id(&self) -> &str {
        &self.0
    }

    /// The language of a file with extension `ext` (`rhai`, `wasm`), if it is one of the two
    /// RemedyUI ships engines for.
    pub fn for_extension(ext: &str) -> Option<Language> {
        match ext.to_ascii_lowercase().as_str() {
            "rhai" => Some(Language::RHAI),
            "wasm" => Some(Language::WASM),
            _ => None,
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.id() {
            "rhai" => f.write_str("Rhai"),
            "wasm" => f.write_str("WebAssembly"),
            other => f.write_str(other),
        }
    }
}

/// A widget as shared: its language and its code (source text for Rhai, a module for
/// WebAssembly).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Package {
    /// What it is written in.
    pub language: Language,
    /// Its code.
    pub code: Rc<[u8]>,
}

impl Package {
    /// A package of `language` with `code`.
    pub fn new(language: Language, code: impl Into<Rc<[u8]>>) -> Package {
        Package { language, code: code.into() }
    }

    /// A Rhai widget from its source.
    pub fn rhai(source: &str) -> Package {
        Package::new(Language::RHAI, source.as_bytes())
    }

    /// A WebAssembly widget from its module.
    pub fn wasm(module: &[u8]) -> Package {
        Package::new(Language::WASM, module)
    }
}

/// How much a widget may use. Every field is a limit per call (a render or a handler)
/// except `memory`, which the widget's machine holds across calls. Made with
/// [`Limits::for_device`] or [`Limits::default`] and adjusted with a setter per field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The most memory one widget's machine may hold.
    pub memory: usize,
    /// The most time one call may work on its thread, timed by
    /// [`remedy_ui_core::time::WorkClock`]: time the system gives other work while the call
    /// runs does not count, so a busy machine does not stop fair work. This is the
    /// backstop for slow devices; `operations` is what stops a script deterministically.
    pub time: Duration,
    /// How many steps one call may take, in the engine's own unit (a Rhai operation, a
    /// WebAssembly fuel unit). `None`: each engine derives it from `time` at the rate it
    /// measured on this device ([`StepRate`]). Set it for a script that must stop at the same
    /// step on every machine.
    pub operations: Option<u64>,
    /// How deeply calls may nest (recursion).
    pub call_depth: usize,
    /// The native stack one call may use, in bytes, for engines whose interpreter recurses on
    /// the caller's stack (Rhai); an engine with a stack of its own (WebAssembly) ignores it.
    pub stack: usize,
    /// How deeply described elements may nest. Past this a tree is not a readable widget and
    /// would only cost the layout.
    pub nesting: usize,
    /// The most handlers (buttons, an element's `on_*`) one description may hold, so a script
    /// cannot make the host keep an unbounded table between renders. Default 4096.
    pub handlers: usize,
    /// The largest picture a description may ask for, in logical pixels a side. Default 4096;
    /// a picture costs its pixels at 4 bytes each, so a raised side costs with its square.
    pub image_side: u32,
    /// The widest gap between children, in logical pixels. Default 64.
    pub gap: u32,
    /// How deeply expressions may nest in the source, at the top level and inside a
    /// function, for engines that parse source (Rhai); others ignore it. Default 64 and 32,
    /// what readable code needs. A safety bound: the parser recurses on the caller's stack
    /// before any step is metered, a few KiB a level, so raising it far needs more `stack`
    /// (and a UI thread with that stack) or hostile source can overflow it.
    pub expr_depth: (usize, usize),
}

impl Limits {
    /// Limits for a device with `total_memory` bytes and `cores` cores: a widget may use
    /// 1/2048 of the memory (between 4 and 64 MiB), and a call a part of one frame that
    /// shrinks on slower devices (between 4 and 16 ms).
    pub fn for_device(total_memory: u64, cores: usize) -> Limits {
        const MIB: u64 = 1024 * 1024;
        let memory = (total_memory / 2048).clamp(4 * MIB, 64 * MIB);
        let ms = (cores as u64 * 2).clamp(4, 16);
        Limits {
            memory: usize::try_from(memory).unwrap_or(usize::MAX),
            time: Duration::from_millis(ms),
            ..Limits::default()
        }
    }

    /// Sets `memory`: the most memory one widget's machine may hold, in bytes.
    pub fn memory(mut self, value: usize) -> Self {
        self.memory = value;
        self
    }

    /// Sets `time`: the longest one call may run on the clock.
    pub fn time(mut self, value: Duration) -> Self {
        self.time = value;
        self
    }

    /// Sets `operations`: the steps one call may take, or `None` to derive them from `time`.
    pub fn operations(mut self, value: Option<u64>) -> Self {
        self.operations = value;
        self
    }

    /// Sets `call_depth`: how deeply calls may nest.
    pub fn call_depth(mut self, value: usize) -> Self {
        self.call_depth = value;
        self
    }

    /// Sets `stack`: the stack one call may use, in bytes.
    pub fn stack(mut self, value: usize) -> Self {
        self.stack = value;
        self
    }

    /// Sets `nesting`: how deeply described elements may nest.
    pub fn nesting(mut self, value: usize) -> Self {
        self.nesting = value;
        self
    }

    /// Sets `handlers`: the most handlers one description may hold.
    pub fn handlers(mut self, value: usize) -> Self {
        self.handlers = value;
        self
    }

    /// Sets `image_side`: the largest picture a description may ask for, a side.
    pub fn image_side(mut self, value: u32) -> Self {
        self.image_side = value;
        self
    }

    /// Sets `gap`: the widest gap between children.
    pub fn gap(mut self, value: u32) -> Self {
        self.gap = value;
        self
    }

    /// Sets `expr_depth`: how deeply expressions may nest, at the top level and in a
    /// function.
    pub fn expr_depth(mut self, top: usize, function: usize) -> Self {
        self.expr_depth = (top, function);
        self
    }

    /// The operations one call may take: `operations`, or `time` at `per_ms` operations a
    /// millisecond (an engine's measured rate).
    /// The steps one call may take on this device: `operations` when set; else the time
    /// limit at `rate` once the rate is measured; else no step limit (only the time limit).
    pub fn step_budget(&self, rate: &StepRate) -> u64 {
        match self.operations {
            Some(n) => n,
            // A budget from a guessed rate would stop fair work on a device faster than the
            // guess; until the device is measured only the time limit applies, and it stops
            // a runaway script just the same.
            None if !rate.is_calibrated() => u64::MAX,
            None => self.operations_at(rate.per_ms()),
        }
    }

    /// The steps one call may take at `per_ms` steps a millisecond: `operations` when set,
    /// else the time limit at that rate.
    pub fn operations_at(&self, per_ms: u64) -> u64 {
        self.operations.unwrap_or_else(|| {
            let ms = u64::try_from(self.time.as_micros()).unwrap_or(u64::MAX) / 1000;
            ms.max(1).saturating_mul(per_ms)
        })
    }
}

impl Default for Limits {
    /// 16 MiB and 8 ms (a mid-range device), operations from the time, 64 nested calls (more
    /// than a widget's own recursion needs), 512 KiB of stack (half the smallest stack a UI
    /// thread gets, 1 MiB on Windows and in browser pages, leaving the rest to the app's own
    /// frames), 64 nested elements, 4096 handlers, pictures up to 4096 pixels a side, gaps
    /// up to 64, and expressions nested 64 deep (32 in a function).
    fn default() -> Self {
        Limits {
            memory: 16 * 1024 * 1024,
            time: Duration::from_millis(8),
            operations: None,
            call_depth: 64,
            stack: 512 * 1024,
            nesting: 64,
            handlers: 4096,
            image_side: 4096,
            gap: 64,
            expr_depth: (64, 32),
        }
    }
}

/// Why a script failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptError {
    /// It does not compile, or is not a widget (what is wrong, for its author).
    Invalid(String),
    /// It asked for something the sandbox does not grant (files, the network, processes,
    /// loading code, a host function that does not exist): what it asked for.
    Denied(String),
    /// It failed while running.
    Runtime(String),
    /// It ran past its time budget.
    TooSlow,
    /// It took more steps than its operation budget.
    OutOfFuel,
    /// Its calls nested past the call depth (runaway recursion).
    TooDeep,
    /// It used more memory than allowed, or made a value too large.
    TooBig,
    /// What it described is not a widget tree (what is wrong).
    Shape(String),
    /// Scripts in this language cannot run here, and why.
    Unsupported(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Invalid(e) => write!(f, "the script is not a widget: {e}"),
            ScriptError::Denied(e) => write!(f, "the script asked for what widgets cannot have: {e}"),
            ScriptError::Runtime(e) => write!(f, "the script failed: {e}"),
            ScriptError::TooSlow => f.write_str("the script took too long"),
            ScriptError::OutOfFuel => f.write_str("the script took too many steps"),
            ScriptError::TooDeep => f.write_str("the script's calls nested too deeply"),
            ScriptError::TooBig => f.write_str("the script used too much memory"),
            ScriptError::Shape(e) => write!(f, "the script described something that is not a widget: {e}"),
            ScriptError::Unsupported(e) => write!(f, "scripted widgets cannot run here: {e}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// The kind of a setting, which decides its editor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SettingKind {
    /// One line of text.
    Text,
    /// Several lines.
    LongText,
    /// A number, optionally bounded.
    Number {
        /// The least.
        min: Option<f64>,
        /// The most.
        max: Option<f64>,
    },
    /// On or off.
    Bool,
    /// One of these.
    Choice(Vec<String>),
    /// A color (`#rrggbb`).
    Color,
    /// A web link.
    Link,
    /// A moment: a date and a time of day, stored as seconds since 1970-01-01T00:00:00Z (a
    /// JSON number; null or missing when not set). Edited with a date picker and a time picker
    /// in the viewer's local time.
    Time,
    /// One of these, each a stored value and the label people see: `(value, label)`. For
    /// choices whose names are not what is saved, such as rooms or roles by id.
    LabeledChoice(Vec<(String, String)>),
    /// A list of values, stored as a JSON array of strings; edited one item a line.
    List,
    /// A picture, stored as a source string (a web address, or a reference the app
    /// resolves, such as an asset id). The board's context shows and chooses pictures.
    Picture,
}

/// A setting a widget declares: the board's property grid edits it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SettingSpec {
    /// Its key in the settings.
    pub key: String,
    /// What people see.
    pub label: String,
    /// Its kind.
    pub kind: SettingKind,
    /// Its value until set.
    pub default: Value,
}

impl SettingSpec {
    /// A setting `key`, shown as `label`, of `kind`, at `default` until set.
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        kind: SettingKind,
        default: Value,
    ) -> SettingSpec {
        SettingSpec { key: key.into(), label: label.into(), kind, default }
    }
}

/// What a widget says about itself when it loads.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Manifest {
    /// Its name, in the add-widget list.
    pub name: String,
    /// The settings it declares.
    pub settings: Vec<SettingSpec>,
}

fn invalid(e: impl Into<String>) -> ScriptError {
    ScriptError::Invalid(e.into())
}

/// Reads a widget's manifest from the protocol's JSON: `name` (a string, "Widget" when
/// missing) and `settings` (a list of `{ key, label, kind, value, min, max, options }`;
/// `kind` is `text`, `longtext`, `number`, `bool`, `choice`, `color`, `link`, `time`, `list`
/// or `picture`, and `value` is the setting's value until someone sets it, its default). A
/// `choice`'s options are strings, or `{ value, label }` maps when what is saved is not what
/// people see; a `time` is seconds since 1970 and a `list` an array of strings.
pub fn manifest(v: &Value) -> Result<Manifest, ScriptError> {
    let Value::Object(m) = v else {
        return Err(invalid("it must describe itself as a map with a name and settings"));
    };
    let name = match m.get("name") {
        None | Some(Value::Null) => "Widget".to_owned(),
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(_) => return Err(invalid("its name must be text")),
    };
    let mut settings = Vec::new();
    match m.get("settings") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for s in list {
                let spec = setting(s)?;
                if settings.iter().any(|o: &SettingSpec| o.key == spec.key) {
                    return Err(invalid(format!("the setting {} is declared twice", spec.key)));
                }
                settings.push(spec);
            }
        }
        Some(_) => return Err(invalid("its settings must be a list")),
    }
    Ok(Manifest { name, settings })
}

fn number(m: &Map<String, Value>, k: &str) -> Option<f64> {
    m.get(k).and_then(Value::as_f64)
}

fn string(m: &Map<String, Value>, k: &str) -> Option<String> {
    m.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn setting(v: &Value) -> Result<SettingSpec, ScriptError> {
    let Value::Object(t) = v else { return Err(invalid("a setting must be a map")) };
    let key = string(t, "key").filter(|k| !k.is_empty()).ok_or_else(|| invalid("a setting has no key"))?;
    let label = string(t, "label").unwrap_or_else(|| key.clone());
    let kind = match string(t, "kind").as_deref().unwrap_or("text") {
        "text" => SettingKind::Text,
        "longtext" => SettingKind::LongText,
        "number" => SettingKind::Number { min: number(t, "min"), max: number(t, "max") },
        "bool" => SettingKind::Bool,
        "choice" => {
            let list = t.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                return Err(invalid(format!("the choice setting {key} has no options")));
            }
            if list.iter().all(Value::is_string) {
                SettingKind::Choice(list.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            } else {
                let mut opts = Vec::new();
                for o in &list {
                    let pair = match o {
                        Value::String(s) => Some((s.clone(), s.clone())),
                        Value::Object(m) => string(m, "value").map(|v| {
                            let label = string(m, "label").unwrap_or_else(|| v.clone());
                            (v, label)
                        }),
                        _ => None,
                    };
                    opts.push(pair.ok_or_else(|| {
                        invalid(format!(
                            "an option of the choice setting {key} is not text or {{ value, label }}"
                        ))
                    })?);
                }
                SettingKind::LabeledChoice(opts)
            }
        }
        "color" => SettingKind::Color,
        "link" => SettingKind::Link,
        "time" => SettingKind::Time,
        "list" => SettingKind::List,
        "picture" => SettingKind::Picture,
        other => return Err(invalid(format!("the setting {key} has an unknown kind {other}"))),
    };
    let default = match t.get("value") {
        Some(d @ (Value::String(_) | Value::Number(_) | Value::Bool(_))) => d.clone(),
        Some(Value::Array(items)) if kind == SettingKind::List => {
            Value::Array(items.iter().filter(|i| i.is_string()).cloned().collect())
        }
        _ => match &kind {
            SettingKind::Number { min, .. } => serde_json::json!(min.unwrap_or(0.0)),
            SettingKind::Bool => Value::Bool(false),
            SettingKind::Choice(o) => Value::String(o.first().cloned().unwrap_or_default()),
            SettingKind::LabeledChoice(o) => {
                Value::String(o.first().map(|(v, _)| v.clone()).unwrap_or_default())
            }
            SettingKind::Time => Value::Null,
            SettingKind::List => Value::Array(Vec::new()),
            _ => Value::String(String::new()),
        },
    };
    Ok(SettingSpec { key, label, kind, default })
}

/// The settings a widget sees: each declared one as `given`, or its default.
pub fn resolve_settings(specs: &[SettingSpec], given: &Map<String, Value>) -> Map<String, Value> {
    specs
        .iter()
        .map(|s| (s.key.clone(), given.get(&s.key).cloned().unwrap_or_else(|| s.default.clone())))
        .collect()
}

/// A button's handler in the last description: an index into [`Described::handlers`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Action(pub usize);

/// How text is shown.
///
/// Exhaustive, like [`Node`]: the protocol's text styles, each of which every host builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// Body text.
    Body,
    /// Secondary.
    Muted,
    /// Emphasized.
    Strong,
    /// A heading of level 1 to 3.
    Heading(u8),
}

/// What a script describes; the host builds elements from it.
///
/// Exhaustive on purpose: the element protocol is a closed set every host must build in full
/// (a host that skipped an element would show a widget other than the one described), so a
/// new element is a new protocol version, not a quiet addition.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// Children top to bottom.
    Column {
        /// Space between them.
        gap: f32,
        /// The children.
        children: Vec<Node>,
    },
    /// Children side by side, wrapping when narrow.
    Row {
        /// Space between them.
        gap: f32,
        /// The children.
        children: Vec<Node>,
    },
    /// Text.
    Text {
        /// The text.
        text: String,
        /// Its style.
        style: TextStyle,
    },
    /// A button.
    Button {
        /// Its label.
        label: String,
        /// What it runs.
        action: Action,
    },
    /// A web link, opened only after the person agrees (the host asks).
    Link {
        /// Its label.
        label: String,
        /// Where it goes (the host opens only `https:` and `mailto:`).
        url: String,
    },
    /// A progress bar, 0 to 1.
    Progress {
        /// What it measures.
        label: String,
        /// How far.
        value: f32,
    },
    /// A line between parts.
    Divider,
    /// A picture (protocol 2). The host resolves `src` (an asset of the script's package, a
    /// picture the app keeps); one it cannot show is replaced by `alt` as text. An empty `alt`
    /// marks the picture as decoration, left out of the accessibility tree.
    Image {
        /// Where the picture comes from, as the host understands it.
        src: String,
        /// What it shows, for people who cannot see it.
        alt: String,
        /// Its width in logical pixels; 0 lets the host choose.
        width: f32,
        /// Its height in logical pixels; 0 lets the host choose.
        height: f32,
    },
    /// One of the host app's own elements (protocol 2): a kind the app offers scripts by name
    /// ([`describe_with`]), such as a scene, a sprite or a part of the app it draws itself.
    /// Hosts that do not offer the kind refuse the description, so a script never shows half a
    /// widget.
    Element {
        /// The element's kind, as the app names it.
        kind: String,
        /// Its properties, as the script gave them (handlers taken out).
        props: Map<String, Value>,
        /// Its handlers: each property named `on` or `on_...` that the script gave, with the
        /// [`Action`] the host hands back through [`ScriptWidget::act`] when it fires.
        handlers: Vec<(String, Action)>,
        /// What the script put inside it.
        children: Vec<Node>,
    },
}

/// The element protocol's version: 1 had columns, rows, text, buttons, links, progress and
/// dividers; 2 adds [`Node::Image`] and the app's own elements ([`Node::Element`]).
pub const PROTOCOL: u32 = 2;

/// A description read by [`describe`]: the tree, and each button's `on` value by
/// [`Action`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Described {
    /// The elements.
    pub node: Node,
    /// What each button hands back when pressed, by [`Action`] index.
    pub handlers: Vec<Value>,
}

impl Described {
    /// The `on` value of `action`, or a [`ScriptError::Shape`] when no such button exists in
    /// this description.
    pub fn handler(&self, action: Action) -> Result<&Value, ScriptError> {
        self.handlers.get(action.0).ok_or_else(|| ScriptError::Shape("that button is gone".into()))
    }
}

/// Reads a description from the protocol's JSON (see the crate docs), nesting at most
/// `limits.nesting` deep.
pub fn describe(v: &Value, limits: &Limits) -> Result<Described, ScriptError> {
    describe_with(v, limits, &|_| false)
}

/// Reads a description like [`describe`], also accepting the app's own elements of the kinds
/// `offered` says yes to (`{ "kind": "scene", ..., "children": [...] }`; see [`Node::Element`]).
/// A kind the app does not offer is refused, as any unknown element is.
pub fn describe_with(
    v: &Value,
    limits: &Limits,
    offered: &dyn Fn(&str) -> bool,
) -> Result<Described, ScriptError> {
    let mut handlers = Vec::new();
    let node = node(v, &mut handlers, 0, limits, offered)?;
    Ok(Described { node, handlers })
}

fn node(
    v: &Value,
    handlers: &mut Vec<Value>,
    depth: usize,
    limits: &Limits,
    offered: &dyn Fn(&str) -> bool,
) -> Result<Node, ScriptError> {
    let shape = |e: &str| ScriptError::Shape(e.to_owned());
    if depth >= limits.nesting {
        return Err(shape("it nests too deeply"));
    }
    let Value::Object(t) = v else { return Err(shape("expected a ui element")) };
    let kind = string(t, "kind").ok_or_else(|| shape("an element has no kind"))?;
    let text = |k: &str| match t.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let children = |handlers: &mut Vec<Value>| -> Result<Vec<Node>, ScriptError> {
        match t.get("children") {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(list)) => {
                list.iter().map(|c| node(c, handlers, depth + 1, limits, offered)).collect()
            }
            Some(_) => Err(shape("children must be a list")),
        }
    };
    let gap = number(t, "gap").unwrap_or(0.0).clamp(0.0, f64::from(limits.gap)) as f32;
    Ok(match kind.as_str() {
        "column" => Node::Column { gap, children: children(handlers)? },
        "row" => Node::Row { gap, children: children(handlers)? },
        "text" => {
            let style = match text("style").as_str() {
                "" | "body" => TextStyle::Body,
                "muted" => TextStyle::Muted,
                "strong" => TextStyle::Strong,
                "heading" => TextStyle::Heading(number(t, "level").unwrap_or(2.0).clamp(1.0, 3.0) as u8),
                other => return Err(ScriptError::Shape(format!("unknown text style {other}"))),
            };
            Node::Text { text: text("text"), style }
        }
        "button" => {
            let on =
                t.get("on").filter(|v| !v.is_null()).ok_or_else(|| shape("a button has nothing to run"))?;
            Node::Button { label: text("label"), action: handler(handlers, on, limits.handlers)? }
        }
        "link" => Node::Link { label: text("label"), url: text("url") },
        "progress" => {
            let v = number(t, "value").filter(|v| v.is_finite()).unwrap_or(0.0);
            Node::Progress { label: text("label"), value: v.clamp(0.0, 1.0) as f32 }
        }
        "divider" => Node::Divider,
        "image" => {
            let side = |k: &str| {
                number(t, k).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, f64::from(limits.image_side))
                    as f32
            };
            Node::Image { src: text("src"), alt: text("alt"), width: side("width"), height: side("height") }
        }
        other if offered(other) => {
            let mut props = Map::new();
            let mut own = Vec::new();
            for (k, v) in t {
                if k == "kind" || k == "children" {
                    continue;
                }
                if (k == "on" || k.starts_with("on_")) && !v.is_null() {
                    own.push((k.clone(), handler(handlers, v, limits.handlers)?));
                } else {
                    props.insert(k.clone(), v.clone());
                }
            }
            // The element's own handlers first, then its children's, in reading order.
            let kids = children(handlers)?;
            Node::Element { kind: other.to_owned(), props, handlers: own, children: kids }
        }
        other => return Err(ScriptError::Shape(format!("unknown element {other}"))),
    })
}

fn handler(handlers: &mut Vec<Value>, on: &Value, max: usize) -> Result<Action, ScriptError> {
    if handlers.len() >= max {
        return Err(ScriptError::Shape("it has too many buttons".into()));
    }
    handlers.push(on.clone());
    Ok(Action(handlers.len() - 1))
}

/// One loaded widget: its machine, its description, its state.
pub trait ScriptWidget {
    /// Its name.
    fn name(&self) -> &str;
    /// The settings it declares.
    fn settings(&self) -> &[SettingSpec];
    /// The limits it runs under.
    fn limits(&self) -> Limits;
    /// Its language.
    fn language(&self) -> Language;
    /// Describes the widget for `settings` (missing ones take their defaults, see
    /// [`resolve_settings`]) and `host` (read-only data the host grants, such as the people
    /// present). The description replaces the last one: its [`Action`]s are the ones
    /// [`ScriptWidget::act`] takes next.
    fn render(&mut self, settings: &Map<String, Value>, host: &Value) -> Result<Node, ScriptError>;
    /// Runs the handler of the last description's button `action`; the host then renders
    /// again.
    fn act(&mut self, action: Action) -> Result<(), ScriptError>;
    /// Runs the handler of `action` with a value from the element that fired it (where a
    /// knob was turned to, which station a dial chose). Engines that take no value run
    /// [`ScriptWidget::act`]; the default does that.
    fn act_with(&mut self, action: Action, value: &Value) -> Result<(), ScriptError> {
        let _ = value;
        self.act(action)
    }
    /// The memory its machine holds now, in bytes.
    fn memory_used(&self) -> usize;
}

impl std::fmt::Debug for dyn ScriptWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptWidget")
            .field("name", &self.name())
            .field("language", &self.language())
            .field("limits", &self.limits())
            .finish()
    }
}

/// A language: loads widgets written in it, each into a sandbox of its own.
pub trait ScriptEngine {
    /// The language it runs.
    fn language(&self) -> Language;
    /// Loads a widget from `code` with `limits`. Loading runs the widget's own setup under
    /// the same limits as a call.
    fn load(&self, code: &[u8], limits: Limits) -> Result<Box<dyn ScriptWidget>, ScriptError>;
}

/// The script engines an app or a board enables, by language.
#[derive(Clone, Default)]
pub struct Engines {
    engines: BTreeMap<Language, Rc<dyn ScriptEngine>>,
}

impl std::fmt::Debug for Engines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.engines.keys()).finish()
    }
}

impl Engines {
    /// No engines: no scripted widget loads.
    pub fn new() -> Engines {
        Engines::default()
    }

    /// Enables `engine` (replacing one for the same language).
    pub fn add(&mut self, engine: Rc<dyn ScriptEngine>) {
        self.engines.insert(engine.language(), engine);
    }

    /// With `engine` enabled.
    pub fn with(mut self, engine: Rc<dyn ScriptEngine>) -> Engines {
        self.add(engine);
        self
    }

    /// The engine for `language`.
    pub fn get(&self, language: &Language) -> Option<Rc<dyn ScriptEngine>> {
        self.engines.get(language).cloned()
    }

    /// Whether widgets in `language` are accepted.
    pub fn accepts(&self, language: &Language) -> bool {
        self.engines.contains_key(language)
    }

    /// The enabled languages.
    pub fn languages(&self) -> Vec<Language> {
        self.engines.keys().cloned().collect()
    }

    /// Loads `package` with the engine for its language, or says it is not accepted here.
    pub fn load(&self, package: &Package, limits: Limits) -> Result<Box<dyn ScriptWidget>, ScriptError> {
        let Some(engine) = self.engines.get(&package.language) else {
            let enabled = self.languages().iter().map(Language::to_string).collect::<Vec<_>>();
            let enabled = if enabled.is_empty() { "none".to_owned() } else { enabled.join(", ") };
            return Err(ScriptError::Unsupported(format!(
                "{} widgets are not accepted here (accepted: {enabled})",
                package.language
            )));
        };
        engine.load(&package.code, limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_manifest_reads_its_settings_and_defaults() {
        let m = manifest(&json!({
            "name": "Claps",
            "settings": [
                { "key": "title", "label": "Title", "kind": "text", "value": "Claps" },
                { "key": "goal", "kind": "number", "min": 1, "max": 100 },
                { "key": "mood", "kind": "choice", "options": ["calm", "loud"] },
            ]
        }));
        let Ok(m) = m else { unreachable!("{m:?}") };
        assert_eq!(m.name, "Claps");
        assert_eq!(m.settings.len(), 3);
        assert_eq!(
            m.settings.get(1).map(|s| &s.kind),
            Some(&SettingKind::Number { min: Some(1.0), max: Some(100.0) })
        );
        assert_eq!(m.settings.get(1).map(|s| &s.default), Some(&json!(1.0)));
        assert_eq!(m.settings.get(2).map(|s| &s.default), Some(&json!("calm")));
        let given = Map::from_iter([("goal".to_owned(), json!(5))]);
        let r = resolve_settings(&m.settings, &given);
        assert_eq!(r.get("goal"), Some(&json!(5)));
        assert_eq!(r.get("title"), Some(&json!("Claps")));
        for bad in [
            json!(5),
            json!({ "settings": 3 }),
            json!({ "settings": [{ "kind": "text" }] }),
            json!({ "settings": [{ "key": "a", "kind": "choice" }] }),
            json!({ "settings": [{ "key": "a", "kind": "rocket" }] }),
            json!({ "settings": [{ "key": "a" }, { "key": "a" }] }),
        ] {
            assert!(matches!(manifest(&bad), Err(ScriptError::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn a_manifest_reads_times_lists_pictures_and_labeled_choices() {
        let m = manifest(&json!({
            "settings": [
                { "key": "when", "kind": "time" },
                { "key": "tags", "kind": "list", "value": ["a", 3, "b"] },
                { "key": "art", "kind": "picture" },
                { "key": "room", "kind": "choice", "options": [{ "value": "7", "label": "Lobby" }, "8"] },
            ]
        }));
        let Ok(m) = m else { unreachable!("{m:?}") };
        let kinds: Vec<_> = m.settings.iter().map(|s| s.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                SettingKind::Time,
                SettingKind::List,
                SettingKind::Picture,
                SettingKind::LabeledChoice(vec![("7".into(), "Lobby".into()), ("8".into(), "8".into())]),
            ]
        );
        let defaults: Vec<_> = m.settings.iter().map(|s| s.default.clone()).collect();
        assert_eq!(defaults, [Value::Null, json!(["a", "b"]), json!(""), json!("7")]);
        let bad = json!({ "settings": [{ "key": "a", "kind": "choice", "options": [{ "label": "x" }] }] });
        assert!(matches!(manifest(&bad), Err(ScriptError::Invalid(_))));
    }

    #[test]
    fn a_description_becomes_elements_with_handlers() {
        let d = describe(
            &json!({ "kind": "column", "gap": 900, "children": [
                { "kind": "text", "text": "Hi", "style": "heading", "level": 7 },
                { "kind": "button", "label": "Go", "on": { "vote": 2 } },
                { "kind": "progress", "label": "P", "value": 3 },
                { "kind": "row", "children": [ { "kind": "divider" }, { "kind": "link", "label": "L", "url": "https://x.y" } ] },
            ] }),
            &Limits::default(),
        );
        let Ok(d) = d else { unreachable!("{d:?}") };
        let Node::Column { gap, children } = &d.node else { unreachable!("{:?}", d.node) };
        assert_eq!(*gap, 64.0);
        assert_eq!(children.first(), Some(&Node::Text { text: "Hi".into(), style: TextStyle::Heading(3) }));
        assert_eq!(children.get(1), Some(&Node::Button { label: "Go".into(), action: Action(0) }));
        assert_eq!(children.get(2), Some(&Node::Progress { label: "P".into(), value: 1.0 }));
        assert_eq!(d.handler(Action(0)), Ok(&json!({ "vote": 2 })));
        assert!(matches!(d.handler(Action(1)), Err(ScriptError::Shape(_))));
    }

    #[test]
    fn what_is_not_a_widget_tree_is_said_plainly() {
        let mut deep = json!({ "kind": "text", "text": "x" });
        for _ in 0..Limits::default().nesting {
            deep = json!({ "kind": "column", "children": [deep] });
        }
        for bad in [
            deep,
            json!({ "kind": "iframe" }),
            json!({ "text": "no kind" }),
            json!({ "kind": "button", "label": "x" }),
            json!({ "kind": "column", "children": 5 }),
            json!({ "kind": "text", "style": "blink" }),
            json!("text"),
        ] {
            assert!(matches!(describe(&bad, &Limits::default()), Err(ScriptError::Shape(_))), "{bad}");
        }
    }

    #[test]
    fn pictures_and_the_apps_own_elements() {
        let tree = json!({ "kind": "column", "children": [
            { "kind": "image", "src": "art/fire.png", "alt": "A fire", "width": 99999, "height": -3 },
            { "kind": "scene", "design": [320, 180], "on_press": { "join": 4 }, "on_drag": null, "children": [
                { "kind": "button", "label": "Sit", "on": "sit" },
            ] },
        ] });
        // Without the app offering it, a scene is an unknown element.
        assert!(matches!(describe(&tree, &Limits::default()), Err(ScriptError::Shape(_))));
        let d = describe_with(&tree, &Limits::default(), &|k| k == "scene");
        let Ok(d) = d else { unreachable!("{d:?}") };
        let Node::Column { children, .. } = &d.node else { unreachable!("{:?}", d.node) };
        assert_eq!(
            children.first(),
            Some(&Node::Image {
                src: "art/fire.png".into(),
                alt: "A fire".into(),
                width: 4096.0,
                height: 0.0
            })
        );
        let Some(Node::Element { kind, props, handlers, children }) = children.get(1) else { unreachable!() };
        assert_eq!(kind, "scene");
        assert_eq!(props.get("design"), Some(&json!([320, 180])));
        assert!(props.get("on_drag").is_some_and(Value::is_null), "a null handler stays a plain property");
        assert_eq!(handlers, &[("on_press".to_owned(), Action(0))]);
        assert_eq!(children.first(), Some(&Node::Button { label: "Sit".into(), action: Action(1) }));
        assert_eq!(d.handler(Action(0)), Ok(&json!({ "join": 4 })));
        assert_eq!(PROTOCOL, 2);
    }

    #[test]
    fn a_description_cannot_hold_endless_handlers() {
        let many: Vec<Value> =
            (0..5000).map(|i| json!({ "kind": "button", "label": "x", "on": i })).collect();
        let tree = json!({ "kind": "column", "children": many });
        let r = describe(&tree, &Limits::default());
        assert!(matches!(r, Err(ScriptError::Shape(_))), "{r:?}");
        // The app may allow more, and wider pictures and gaps.
        let roomy = Limits::default().handlers(8000).image_side(10_000).gap(200);
        assert!(describe(&tree, &roomy).is_ok());
        let pic = json!({ "kind": "row", "gap": 150, "children": [{ "kind": "image", "width": 9000 }] });
        let d = describe(&pic, &roomy).map(|d| d.node);
        assert!(
            matches!(&d, Ok(Node::Row { gap, children }) if *gap == 150.0
                && matches!(children.first(), Some(Node::Image { width, .. }) if *width == 9000.0)),
            "{d:?}"
        );
    }

    #[test]
    fn limits_follow_the_device() {
        let small = Limits::for_device(2 * 1024 * 1024 * 1024, 2);
        let big = Limits::for_device(64 * 1024 * 1024 * 1024, 16);
        assert_eq!(small.memory, 4 * 1024 * 1024);
        assert_eq!(big.memory, 32 * 1024 * 1024);
        assert_eq!((small.time, big.time), (Duration::from_millis(4), Duration::from_millis(16)));
        assert_eq!(small.operations_at(1000), 4000);
        assert_eq!(Limits { operations: Some(7), ..small }.operations_at(1000), 7);
    }

    struct Echo;

    impl ScriptEngine for Echo {
        fn language(&self) -> Language {
            Language::new("echo")
        }
        fn load(&self, _: &[u8], _: Limits) -> Result<Box<dyn ScriptWidget>, ScriptError> {
            Err(ScriptError::Invalid("echo loads nothing".into()))
        }
    }

    #[test]
    fn engines_load_only_the_languages_they_accept() {
        let engines = Engines::new().with(Rc::new(Echo));
        assert!(engines.accepts(&Language::new("echo")) && !engines.accepts(&Language::RHAI));
        let r = engines.load(&Package::rhai("x"), Limits::default());
        let Err(ScriptError::Unsupported(why)) = r else { unreachable!("{r:?}") };
        assert!(why.contains("Rhai") && why.contains("echo"), "{why}");
        let r = engines.load(&Package::new(Language::new("echo"), &b""[..]), Limits::default());
        assert!(matches!(r, Err(ScriptError::Invalid(_))));
        assert_eq!(Language::for_extension("WASM"), Some(Language::WASM));
    }
}
