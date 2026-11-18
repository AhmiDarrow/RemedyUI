//! The Luau machine behind [`ScriptWidget`](crate::ScriptWidget).

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Function, Lua, LuaOptions, RegistryKey, StdLib, Table, Value, VmState};
use std::time::Instant;

use crate::{Action, Limits, Node, ScriptError, SettingKind, SettingSpec, TextStyle};

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
