//! RemedyUI scripted widgets: small programs people write and share (a FreeSpeak Home's
//! custom widgets), run as untrusted code.
//!
//! A script is Luau, which was made for running untrusted code. Each widget gets a virtual
//! machine of its own, sandboxed: no files, network, processes or loading other code; only
//! the standard library's pure parts, a `ui` table that describes elements, its own `state`,
//! its settings, and the read-only data the host grants (`host`). Every call into it runs
//! against a time budget (an interrupt stops a script that runs past it, even in a tight loop)
//! and the machine against a memory limit. Both are sized from the device ([`Limits`]).
//!
//! A script never touches elements: it returns a [`Node`] tree, which the host builds into
//! ordinary RemedyUI elements, so scripted widgets are themed, accessible and fast like
//! built-in ones. Buttons name handlers ([`Action`]); the host runs one with
//! [`ScriptWidget::act`] and renders again.
//!
//! ```lua
//! return {
//!   name = "Claps",
//!   settings = { { key = "title", label = "Title", kind = "text", default = "Claps" } },
//!   render = function(settings, state, ui, host)
//!     state.claps = state.claps or 0
//!     return ui.column({ gap = 8 }, {
//!       ui.heading(settings.title),
//!       ui.text(state.claps .. " claps"),
//!       ui.button("Clap", function() state.claps = state.claps + 1 end),
//!     })
//!   end,
//! }
//! ```

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, LuaOptions, RegistryKey, StdLib, Table, Value, VmState};

/// How much a script may use, sized from the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most memory one widget's machine may hold.
    pub memory: usize,
    /// The longest one call (a render or a handler) may run.
    pub time: Duration,
}

impl Limits {
    /// Limits for a device with `total_memory` bytes and `cores` cores: a widget may use
    /// 1/2048 of the memory (between 4 and 64 MiB), and a call a part of one frame that
    /// shrinks on slower devices (between 4 and 16 ms).
    pub fn for_device(total_memory: u64, cores: usize) -> Limits {
        const MIB: u64 = 1024 * 1024;
        let memory = (total_memory / 2048).clamp(4 * MIB, 64 * MIB);
        let ms = (cores as u64 * 2).clamp(4, 16);
        Limits { memory: usize::try_from(memory).unwrap_or(usize::MAX), time: Duration::from_millis(ms) }
    }
}

impl Default for Limits {
    /// 16 MiB and 8 ms: a mid-range device.
    fn default() -> Self {
        Limits { memory: 16 * 1024 * 1024, time: Duration::from_millis(8) }
    }
}

/// Why a script failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptError {
    /// It does not compile, or is not a widget (what is wrong, for its author).
    Invalid(String),
    /// It failed while running.
    Runtime(String),
    /// It ran past its time budget.
    TooSlow,
    /// It used more memory than allowed.
    TooBig,
    /// What it described is not a widget tree (what is wrong).
    Shape(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Invalid(e) => write!(f, "the script is not a widget: {e}"),
            ScriptError::Runtime(e) => write!(f, "the script failed: {e}"),
            ScriptError::TooSlow => f.write_str("the script took too long"),
            ScriptError::TooBig => f.write_str("the script used too much memory"),
            ScriptError::Shape(e) => write!(f, "the script described something that is not a widget: {e}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// The kind of a setting, which decides its editor.
#[derive(Clone, Debug, PartialEq)]
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
}

/// A setting a widget declares: the board's property grid edits it.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    /// Its key in the settings.
    pub key: String,
    /// What people see.
    pub label: String,
    /// Its kind.
    pub kind: SettingKind,
    /// Its value until set.
    pub default: serde_json::Value,
}

/// A handler a button names; the host runs it with [`ScriptWidget::act`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Action(pub usize);

/// How text is shown.
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
        /// Where it goes (`https:` or `mailto:` only).
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
}

/// Turns a stop from the time budget or the memory limit into a [`ScriptError`].
fn classify(e: &mlua::Error) -> ScriptError {
    let text = e.to_string();
    match e {
        mlua::Error::MemoryError(_) => ScriptError::TooBig,
        _ if text.contains(TIME_UP) => ScriptError::TooSlow,
        _ if text.contains("not enough memory") => ScriptError::TooBig,
        _ => ScriptError::Runtime(text),
    }
}

const TIME_UP: &str = "time budget used up";

/// One scripted widget: its machine, its description, its state.
pub struct ScriptWidget {
    lua: Lua,
    name: String,
    settings: Vec<SettingSpec>,
    render: RegistryKey,
    state: RegistryKey,
    handlers: Vec<RegistryKey>,
    deadline: Rc<Cell<Option<Instant>>>,
    limits: Limits,
}

impl std::fmt::Debug for ScriptWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptWidget").field("name", &self.name).field("limits", &self.limits).finish()
    }
}

fn string_field(t: &Table, k: &str) -> Option<String> {
    t.get::<Option<String>>(k).ok().flatten()
}

fn setting(t: &Table) -> Result<SettingSpec, ScriptError> {
    let bad = |e: &str| ScriptError::Invalid(e.to_owned());
    let key = string_field(t, "key").ok_or_else(|| bad("a setting has no key"))?;
    let label = string_field(t, "label").unwrap_or_else(|| key.clone());
    let kind = match string_field(t, "kind").as_deref().unwrap_or("text") {
        "text" => SettingKind::Text,
        "longtext" => SettingKind::LongText,
        "number" => SettingKind::Number {
            min: t.get::<Option<f64>>("min").ok().flatten(),
            max: t.get::<Option<f64>>("max").ok().flatten(),
        },
        "bool" => SettingKind::Bool,
        "choice" => {
            let opts: Vec<String> = t
                .get::<Option<Table>>("options")
                .ok()
                .flatten()
                .map(|o| o.sequence_values::<String>().filter_map(Result::ok).collect())
                .unwrap_or_default();
            if opts.is_empty() {
                return Err(bad(&format!("the choice setting {key} has no options")));
            }
            SettingKind::Choice(opts)
        }
        "color" => SettingKind::Color,
        "link" => SettingKind::Link,
        other => return Err(bad(&format!("the setting {key} has an unknown kind {other}"))),
    };
    let default = match t.get::<Value>("default").unwrap_or(Value::Nil) {
        Value::String(s) => serde_json::Value::String(s.to_string_lossy()),
        Value::Number(n) => serde_json::json!(n),
        Value::Integer(n) => serde_json::json!(n),
        Value::Boolean(b) => serde_json::Value::Bool(b),
        _ => match &kind {
            SettingKind::Number { min, .. } => serde_json::json!(min.unwrap_or(0.0)),
            SettingKind::Bool => serde_json::Value::Bool(false),
            SettingKind::Choice(o) => serde_json::Value::String(o.first().cloned().unwrap_or_default()),
            _ => serde_json::Value::String(String::new()),
        },
    };
    Ok(SettingSpec { key, label, kind, default })
}

/// The `ui` table: functions that return plain tables describing nodes.
const UI_PRELUDE: &str = r#"
local ui = {}
local function kids(t) if type(t) ~= "table" then return {} end return t end
function ui.column(opts, children) return { kind = "column", gap = (opts or {}).gap or 0, children = kids(children) } end
function ui.row(opts, children) return { kind = "row", gap = (opts or {}).gap or 0, children = kids(children) } end
function ui.text(s) return { kind = "text", text = tostring(s), style = "body" } end
function ui.muted(s) return { kind = "text", text = tostring(s), style = "muted" } end
function ui.strong(s) return { kind = "text", text = tostring(s), style = "strong" } end
function ui.heading(s, level) return { kind = "text", text = tostring(s), style = "heading", level = level or 2 } end
function ui.button(label, on_click) return { kind = "button", label = tostring(label), on_click = on_click } end
function ui.link(label, url) return { kind = "link", label = tostring(label), url = tostring(url) } end
function ui.progress(label, value) return { kind = "progress", label = tostring(label), value = tonumber(value) or 0 } end
function ui.divider() return { kind = "divider" } end
return table.freeze(ui)
"#;

/// Nesting deeper than this is not a readable widget (and would only exhaust the stack).
const MAX_DEPTH: usize = 64;

impl ScriptWidget {
    /// Compiles `source` into a widget with `limits`.
    pub fn load(source: &str, limits: Limits) -> Result<ScriptWidget, ScriptError> {
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())
            .map_err(|e| ScriptError::Invalid(e.to_string()))?;
        lua.set_memory_limit(limits.memory).map_err(|e| ScriptError::Invalid(e.to_string()))?;
        let deadline: Rc<Cell<Option<Instant>>> = Rc::default();
        let d2 = deadline.clone();
        lua.set_interrupt(move |_| match d2.get() {
            Some(at) if Instant::now() > at => Err(mlua::Error::RuntimeError(TIME_UP.into())),
            _ => Ok(VmState::Continue),
        });
        let ui: Table =
            lua.load(UI_PRELUDE).set_name("ui").eval().map_err(|e| ScriptError::Invalid(e.to_string()))?;
        lua.globals().set("ui", ui).map_err(|e| ScriptError::Invalid(e.to_string()))?;
        // Everything the script can reach is now in place: freeze it.
        lua.sandbox(true).map_err(|e| ScriptError::Invalid(e.to_string()))?;
        deadline.set(Some(Instant::now() + limits.time));
        let module: Value = lua.load(source).set_name("widget").eval().map_err(|e| match classify(&e) {
            ScriptError::Runtime(t) => ScriptError::Invalid(t),
            other => other,
        })?;
        deadline.set(None);
        let Value::Table(module) = module else {
            return Err(ScriptError::Invalid("it must return a table with name and render".into()));
        };
        let name = string_field(&module, "name").unwrap_or_else(|| "Widget".into());
        let render: Function = module
            .get::<Option<Function>>("render")
            .ok()
            .flatten()
            .ok_or_else(|| ScriptError::Invalid("it has no render function".into()))?;
        let mut settings = Vec::new();
        if let Ok(Some(list)) = module.get::<Option<Table>>("settings") {
            for t in list.sequence_values::<Table>() {
                settings.push(setting(&t.map_err(|e| ScriptError::Invalid(e.to_string()))?)?);
            }
        }
        let render = lua.create_registry_value(render).map_err(|e| ScriptError::Invalid(e.to_string()))?;
        let state = lua.create_table().map_err(|e| ScriptError::Invalid(e.to_string()))?;
        let state = lua.create_registry_value(state).map_err(|e| ScriptError::Invalid(e.to_string()))?;
        Ok(ScriptWidget { lua, name, settings, render, state, handlers: Vec::new(), deadline, limits })
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The settings it declares.
    pub fn settings(&self) -> &[SettingSpec] {
        &self.settings
    }

    /// The limits it runs under.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    fn timed<R>(&self, f: impl FnOnce() -> mlua::Result<R>) -> Result<R, ScriptError> {
        self.deadline.set(Some(Instant::now() + self.limits.time));
        let r = f();
        self.deadline.set(None);
        r.map_err(|e| classify(&e))
    }

    fn to_lua(&self, v: &serde_json::Value) -> mlua::Result<Value> {
        Ok(match v {
            serde_json::Value::Null => Value::Nil,
            serde_json::Value::Bool(b) => Value::Boolean(*b),
            serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(s) => Value::String(self.lua.create_string(s)?),
            serde_json::Value::Array(a) => {
                let t = self.lua.create_table()?;
                for (i, x) in a.iter().enumerate() {
                    t.raw_set(i + 1, self.to_lua(x)?)?;
                }
                t.set_readonly(true);
                Value::Table(t)
            }
            serde_json::Value::Object(m) => {
                let t = self.lua.create_table()?;
                for (k, x) in m {
                    t.raw_set(k.as_str(), self.to_lua(x)?)?;
                }
                t.set_readonly(true);
                Value::Table(t)
            }
        })
    }

    /// Describes the widget for `settings` (missing ones take their defaults) and `host`
    /// (read-only data the host grants, such as the people present).
    pub fn render(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        host: &serde_json::Value,
    ) -> Result<Node, ScriptError> {
        let mut values = serde_json::Map::new();
        for s in &self.settings {
            values.insert(s.key.clone(), settings.get(&s.key).cloned().unwrap_or_else(|| s.default.clone()));
        }
        let shown = self.timed(|| {
            let render: Function = self.lua.registry_value(&self.render)?;
            let state: Table = self.lua.registry_value(&self.state)?;
            let ui: Value = self.lua.globals().get("ui")?;
            let s = self.to_lua(&serde_json::Value::Object(values))?;
            let h = self.to_lua(host)?;
            render.call::<Value>((s, state, ui, h))
        })?;
        // The handlers of the last render are replaced by this one's.
        for k in self.handlers.drain(..) {
            let _ = self.lua.remove_registry_value(k);
        }
        let mut handlers = Vec::new();
        let node = node(&self.lua, &shown, &mut handlers, 0)?;
        self.handlers = handlers;
        Ok(node)
    }

    /// Runs the handler a button names; the host then renders again.
    pub fn act(&mut self, action: Action) -> Result<(), ScriptError> {
        let Some(key) = self.handlers.get(action.0) else {
            return Err(ScriptError::Shape("that button is gone".into()));
        };
        self.timed(|| {
            let f: Function = self.lua.registry_value(key)?;
            f.call::<()>(())
        })
    }

    /// The memory its machine holds now.
    pub fn memory_used(&self) -> usize {
        self.lua.used_memory()
    }
}

fn node(lua: &Lua, v: &Value, handlers: &mut Vec<RegistryKey>, depth: usize) -> Result<Node, ScriptError> {
    let shape = |e: &str| ScriptError::Shape(e.to_owned());
    if depth > MAX_DEPTH {
        return Err(shape("it nests too deeply"));
    }
    let Value::Table(t) = v else { return Err(shape("expected a ui element")) };
    let kind = string_field(t, "kind").ok_or_else(|| shape("an element has no kind"))?;
    let text = |k: &str| string_field(t, k).unwrap_or_default();
    let children = |handlers: &mut Vec<RegistryKey>| -> Result<Vec<Node>, ScriptError> {
        let Ok(Some(list)) = t.get::<Option<Table>>("children") else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for c in list.sequence_values::<Value>() {
            let c = c.map_err(|e| ScriptError::Shape(e.to_string()))?;
            out.push(node(lua, &c, handlers, depth + 1)?);
        }
        Ok(out)
    };
    let gap = t.get::<Option<f64>>("gap").ok().flatten().unwrap_or(0.0).clamp(0.0, 64.0) as f32;
    Ok(match kind.as_str() {
        "column" => Node::Column { gap, children: children(handlers)? },
        "row" => Node::Row { gap, children: children(handlers)? },
        "text" => {
            let style = match text("style").as_str() {
                "muted" => TextStyle::Muted,
                "strong" => TextStyle::Strong,
                "heading" => {
                    let l = t.get::<Option<f64>>("level").ok().flatten().unwrap_or(2.0).clamp(1.0, 3.0);
                    TextStyle::Heading(l as u8)
                }
                _ => TextStyle::Body,
            };
            Node::Text { text: text("text"), style }
        }
        "button" => {
            let f = t
                .get::<Option<Function>>("on_click")
                .ok()
                .flatten()
                .ok_or_else(|| shape("a button has no function to run"))?;
            let key = lua.create_registry_value(f).map_err(|e| ScriptError::Shape(e.to_string()))?;
            handlers.push(key);
            Node::Button { label: text("label"), action: Action(handlers.len() - 1) }
        }
        "link" => Node::Link { label: text("label"), url: text("url") },
        "progress" => {
            let v = t.get::<Option<f64>>("value").ok().flatten().unwrap_or(0.0);
            Node::Progress { label: text("label"), value: v.clamp(0.0, 1.0) as f32 }
        }
        "divider" => Node::Divider,
        other => return Err(ScriptError::Shape(format!("unknown element {other}"))),
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    const CLAPS: &str = r#"
return {
  name = "Claps",
  settings = { { key = "title", label = "Title", kind = "text", default = "Claps" },
               { key = "goal", kind = "number", min = 1, max = 100, default = 10 } },
  render = function(settings, state, ui, host)
    state.claps = state.claps or 0
    return ui.column({ gap = 8 }, {
      ui.heading(settings.title),
      ui.text(state.claps .. " of " .. settings.goal),
      ui.progress("Claps", state.claps / settings.goal),
      ui.button("Clap", function() state.claps = state.claps + 1 end),
      ui.muted("Hello, " .. (host.name or "guest")),
    })
  end,
}
"#;

    fn host() -> serde_json::Value {
        serde_json::json!({ "name": "Sam" })
    }

    #[test]
    fn a_widget_renders_takes_settings_and_handles_its_buttons() {
        let mut w = ScriptWidget::load(CLAPS, Limits::default()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(w.name(), "Claps");
        assert_eq!(w.settings().len(), 2);
        assert_eq!(
            w.settings().get(1).map(|s| &s.kind),
            Some(&SettingKind::Number { min: Some(1.0), max: Some(100.0) })
        );
        let mut settings = serde_json::Map::new();
        settings.insert("title".into(), "Party".into());
        let n = w.render(&settings, &host()).unwrap_or_else(|e| unreachable!("{e}"));
        let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
        assert_eq!(
            children.first(),
            Some(&Node::Text { text: "Party".into(), style: TextStyle::Heading(2) })
        );
        assert_eq!(children.get(1), Some(&Node::Text { text: "0 of 10".into(), style: TextStyle::Body }));
        let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!() };
        w.act(*action).unwrap_or_else(|e| unreachable!("{e}"));
        w.act(*action).unwrap_or_else(|e| unreachable!("{e}"));
        let n = w.render(&settings, &host()).unwrap_or_else(|e| unreachable!("{e}"));
        let Node::Column { children, .. } = &n else { unreachable!() };
        assert_eq!(children.get(1), Some(&Node::Text { text: "2 of 10".into(), style: TextStyle::Body }));
        assert_eq!(children.get(2), Some(&Node::Progress { label: "Claps".into(), value: 0.2 }));
        assert_eq!(children.get(4), Some(&Node::Text { text: "Hello, Sam".into(), style: TextStyle::Muted }));
    }

    #[test]
    fn a_hostile_script_cannot_reach_files_the_network_processes_or_loaders() {
        for probe in [
            "io.open('x')",
            "os.execute('x')",
            "os.remove('x')",
            "require('socket')",
            "dofile('x')",
            "loadfile('x')",
            "package.loadlib('x')",
            "debug.getregistry()",
            "ui.text = nil",
            "string.rep = nil",
        ] {
            let src = format!(
                "return {{ name = 'x', render = function(s, st, ui) {probe} return ui.text('ran') end }}"
            );
            let r = ScriptWidget::load(&src, Limits::default())
                .and_then(|mut w| w.render(&serde_json::Map::new(), &host()));
            assert!(r.is_err(), "{probe} must fail, got {r:?}");
        }
    }

    #[test]
    fn a_script_that_never_stops_is_stopped_in_time() {
        let src = "return { name = 'spin', render = function(s, st, ui) while true do end end }";
        let mut w = ScriptWidget::load(src, Limits { time: Duration::from_millis(20), ..Limits::default() })
            .unwrap_or_else(|e| unreachable!("{e}"));
        let start = Instant::now();
        assert_eq!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::TooSlow));
        assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
        // Loading code that spins at the top level is stopped too.
        let r = ScriptWidget::load(
            "while true do end",
            Limits { time: Duration::from_millis(20), ..Limits::default() },
        );
        assert_eq!(r.err(), Some(ScriptError::TooSlow));
    }

    #[test]
    fn a_script_that_eats_memory_is_stopped_at_its_limit() {
        let src = "return { name = 'hog', render = function(s, st, ui) local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1000) .. i end end }";
        let limits = Limits { memory: 4 * 1024 * 1024, time: Duration::from_secs(10) };
        let mut w = ScriptWidget::load(src, limits).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::TooBig));
        assert!(w.memory_used() <= limits.memory + 1024 * 1024);
    }

    #[test]
    fn what_is_not_a_widget_is_said_plainly() {
        assert!(matches!(ScriptWidget::load("return 5", Limits::default()), Err(ScriptError::Invalid(_))));
        assert!(matches!(ScriptWidget::load("return {", Limits::default()), Err(ScriptError::Invalid(_))));
        assert!(matches!(
            ScriptWidget::load("return { name = 'x' }", Limits::default()),
            Err(ScriptError::Invalid(_))
        ));
        let deep = "return { name='d', render = function(s, st, ui) local n = ui.text('x') for i = 1, 200 do n = ui.column({}, { n }) end return n end }";
        let mut w = ScriptWidget::load(deep, Limits::default()).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(matches!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::Shape(_))));
        let odd = "return { name='o', render = function(s, st, ui) return { kind = 'iframe' } end }";
        let mut w = ScriptWidget::load(odd, Limits::default()).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(matches!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::Shape(_))));
    }

    #[test]
    fn limits_follow_the_device() {
        let small = Limits::for_device(2 * 1024 * 1024 * 1024, 2);
        let big = Limits::for_device(64 * 1024 * 1024 * 1024, 16);
        assert_eq!(small.memory, 4 * 1024 * 1024);
        assert_eq!(big.memory, 32 * 1024 * 1024);
        assert_eq!((small.time, big.time), (Duration::from_millis(4), Duration::from_millis(16)));
    }
}
