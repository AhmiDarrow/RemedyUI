//! RemedyUI scripted widgets and themes in Lua 5.4, run by
//! [RemedyLua](https://github.com/AhmiDarrow/RemedyLua): Lua in safe Rust with fuel, memory
//! and depth limits in every step and no C.
//!
//! [`LuaEngine`] is a [`ScriptEngine`]: enable it in an app's or a board's
//! [`Engines`](crate::script::Engines), and packages whose language is `lua` load through
//! it. It goes further than the other engines: a widget keeps a tree of elements it creates,
//! updates and removes from Lua, built on reactive values (signals, computed values,
//! effects), with animated values, timers and frame hooks driven by the host's frame loop
//! ([`ScriptWidget::advance`]), lifecycle hooks and the app's theme tokens.
//!
//! ```lua
//! local claps = ui.signal(0)
//! local label = ui.computed(function() return claps:get() .. " claps" end)
//! ui.mount(ui.column{ gap = 8,
//!   ui.heading("Claps"),
//!   ui.text(label),
//!   ui.button("Clap", function() claps:update(function(n) return n + 1 end) end),
//! })
//! return { name = "Claps" }
//! ```
//!
//! # The sandbox
//!
//! Each widget gets a Lua state of its own with only what the host grants ([`Grants`]):
//! Lua's pure libraries (`string`, `table`, `math`, `utf8`, `coroutine` and the safe base
//! functions), the `ui` library, a `this` table the widget keeps between calls, and the app's
//! own functions and elements when granted. There is no `io`, `os`, `require`, `load`,
//! `debug`, `eval` or `import`, and no clock or randomness (`math.random` is refused): the
//! same calls describe the same widget. Reaching for what was not granted is a
//! [`ScriptError::Denied`]. `print` writes to the engine's trace when [`Grants::print`] allows
//! it. Every call the host makes (loading, a render, a handler, a frame) runs as one entry
//! into the state against the [`Limits`]:
//!
//! - fuel: every instruction and every library function's work is counted, and the whole
//!   call shares one budget, the effects and timers it sets off included
//!   ([`ScriptError::OutOfFuel`]); the budget is [`Limits::operations`] when set, else the
//!   time limit at this device's measured rate ([`rate`]), and until that rate is measured
//!   only the time limit applies ([`ScriptError::TooSlow`]);
//! - memory: the state's heap is capped at [`Limits::memory`], counted before anything is
//!   made ([`ScriptError::TooBig`]), and the number of elements, values, effects and timers a
//!   widget may hold is bounded too;
//! - depth: Lua calls nest at most [`Limits::call_depth`] deep, and library callbacks within
//!   [`Limits::stack`] bytes of native stack ([`ScriptError::TooDeep`]). [`Limits::expr_depth`]
//!   is not applied: RemedyLua's parser does not recurse, and source size is bounded by the
//!   memory limit instead.
//!
//! A widget's last expression is its manifest (`name`, `settings`, and `render` when it
//! describes itself afresh each call). `render(settings, host)` returns an element made with
//! `ui`. A button's `on` is a function or the name of one. There is no hot reload: a changed
//! script is a new widget.
//!
//! A stopped call leaves the widget usable: its world is settled and the next call starts
//! afresh. Handles to removed elements and dropped values are recognized as gone.

mod api;
mod world;

use std::sync::Arc;
use std::time::Duration;

use remedy_lua::{Data, DataLimits, Dialect, Error, Libs, LimitKind, Lua, Root, Value};
use remedy_ui_core::time::WorkClock;
use crate::script::{
    Action, Described, Language, Limits, Manifest, Node, ScriptEngine, ScriptError, ScriptWidget,
    SettingSpec, StepRate,
};
use serde_json::{Map, Value as Json};

use crate::script_lua::api::{Ctx, FRAME, Kind, err, flush, get_ref, signal_get, sweep_refs, with};
use crate::script_lua::world::{Id, Ref, World};

/// RemedyLua steps a millisecond assumed only until this device's rate is measured
/// ([`rate`]): low on purpose (a fraction of what a current desktop core runs), so a budget
/// made then errs short.
pub const FALLBACK_STEPS_PER_MS: u64 = 20_000;

/// RemedyLua steps a millisecond on this device, for the whole process.
static RATE: StepRate = StepRate::new(FALLBACK_STEPS_PER_MS);

/// How many RemedyLua steps a millisecond buys on this device: what a time budget becomes
/// when [`Limits::operations`] is not given. Calibrated the first time a widget loads (a
/// fixed script run away from the UI thread), then kept current by every call long enough to
/// time well.
pub fn rate() -> &'static StepRate {
    &RATE
}

/// The fixed script calibration runs: calls, arithmetic, table and string work.
const CALIBRATION: &str = "local function step(n) return n + 1 end \
    local t, s = {}, 0 \
    while true do s = step(s) t[#t + 1] = s if #t > 32 then t = {} end end";

/// Runs the calibration script on this thread: the steps it took and how long, or `None`.
pub fn measure() -> Option<(u64, Duration)> {
    crate::script::calibrate_with(FALLBACK_STEPS_PER_MS, |steps| {
        let mut lua = Lua::new(remedy_lua::Limits::default().fuel(steps), Libs::BASE);
        let start = WorkClock::start();
        let r = lua.exec(CALIBRATION, "=calibration");
        matches!(r, Err(Error::Limit(LimitKind::Fuel))).then(|| start.work())
    })
}

/// Measures this device's rate now, on this thread, and records it.
pub fn calibrate() -> Option<u64> {
    let (steps, took) = measure()?;
    RATE.record(steps, took).then(|| RATE.per_ms())
}

/// A function of the app's that widgets may call: JSON arguments in, a JSON result or an
/// error message out. Granted by name ([`Grants::function`]); scripts call it as
/// `app.name(...)`.
pub type AppFunction = Arc<dyn Fn(&[Json]) -> Result<Json, String> + Send + Sync>;

/// What a widget may reach beyond Lua's pure libraries and the `ui` library. Nothing else is
/// there: a widget can only use what is granted here.
#[derive(Clone, Default)]
pub struct Grants {
    /// The app's own element kinds scripts may describe ([`Node::Element`]).
    pub elements: Vec<String>,
    /// The app's functions, by name.
    pub functions: Vec<(String, AppFunction)>,
    /// Whether `print` works (to the trace at debug level).
    pub print: bool,
    /// The theme tokens widgets read with `ui.token` (a JSON object, such as `{ "color": {
    /// "accent": "#5865f2" } }`).
    pub theme: Json,
    /// The person asked for reduced motion: animated values jump to their targets.
    pub reduced_motion: bool,
}

impl std::fmt::Debug for Grants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grants")
            .field("elements", &self.elements)
            .field("functions", &self.functions.iter().map(|(n, _)| n).collect::<Vec<_>>())
            .field("print", &self.print)
            .finish_non_exhaustive()
    }
}

impl Grants {
    /// Nothing beyond Lua's pure libraries and `ui`.
    pub fn none() -> Grants {
        Grants::default()
    }

    /// Offers the app's element `kind`.
    pub fn element(mut self, kind: impl Into<String>) -> Grants {
        self.elements.push(kind.into());
        self
    }

    /// Grants the app's function `name`.
    pub fn function(
        mut self,
        name: impl Into<String>,
        f: impl Fn(&[Json]) -> Result<Json, String> + Send + Sync + 'static,
    ) -> Grants {
        self.functions.push((name.into(), Arc::new(f)));
        self
    }

    /// Lets `print` write to the trace.
    pub fn print(mut self, on: bool) -> Grants {
        self.print = on;
        self
    }

    /// The theme tokens.
    pub fn theme(mut self, tokens: Json) -> Grants {
        self.theme = tokens;
        self
    }

    /// Follows the person's reduced-motion setting: animated values jump to their targets.
    pub fn reduced_motion(mut self, on: bool) -> Grants {
        self.reduced_motion = on;
        self
    }
}

/// The engine for Lua widgets and themes, with what it grants them.
#[derive(Clone, Debug, Default)]
pub struct LuaEngine {
    grants: Grants,
}

impl LuaEngine {
    /// An engine granting nothing beyond Lua's pure libraries and `ui`.
    pub fn new() -> LuaEngine {
        LuaEngine::default()
    }

    /// An engine granting `grants`.
    pub fn with_grants(grants: Grants) -> LuaEngine {
        LuaEngine { grants }
    }

    /// Loads the widget in `source` with `limits`.
    pub fn load_source(&self, source: &str, limits: Limits) -> Result<LuaWidget, ScriptError> {
        LuaWidget::load(source, limits, &self.grants)
    }
}

impl ScriptEngine for LuaEngine {
    fn language(&self) -> Language {
        Language::LUA
    }

    fn load(&self, code: &[u8], limits: Limits) -> Result<Box<dyn ScriptWidget>, ScriptError> {
        let source = std::str::from_utf8(code)
            .map_err(|e| ScriptError::Invalid(format!("a Lua widget is text, and this is not: {e}")))?;
        Ok(Box::new(self.load_source(source, limits)?))
    }
}

/// What a call into the widget does: set up by the widget, carried out by the dispatcher
/// (one host function, so a whole call is one entry into the state, on one budget).
enum Op {
    Load,
    Render { settings: Json, host: Json },
    Act { handler: Ref, value: Option<Json> },
    Advance(Duration),
    Theme(Json),
    Unmount,
}

/// The dispatcher's mailbox, kept in the state beside the world.
#[derive(Default)]
struct Mailbox {
    op: Option<Op>,
}

/// One Lua widget: its state, its world, its description.
pub struct LuaWidget {
    lua: Lua,
    ctx: Ctx,
    dispatch: Root,
    _refs: Root,
    manifest: Manifest,
    limits: Limits,
    offered: Vec<String>,
    last: Option<Described>,
    broken: bool,
    /// The unmount hooks ran (by `unmount`, or as the widget was dropped).
    unmounted: bool,
}

impl std::fmt::Debug for LuaWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaWidget").field("name", &self.manifest.name).field("limits", &self.limits).finish()
    }
}

/// Globals a widget does not get, and what they would reach.
const DENIED: &[(&str, &str)] = &[
    ("io", "files (io)"),
    ("os", "the operating system (os)"),
    ("require", "loading other code (require)"),
    ("load", "running code built from text (load)"),
    ("loadstring", "running code built from text (loadstring)"),
    ("dofile", "running files (dofile)"),
    ("loadfile", "loading files (loadfile)"),
    ("debug", "the debug library (debug)"),
    ("package", "loading other code (package)"),
    ("collectgarbage", "the collector (collectgarbage)"),
    ("print", "printing (print is not granted)"),
    ("eval", "running code built from text (eval)"),
    ("import", "loading other code (import)"),
    ("open_file", "files (open_file)"),
    ("read_file", "files (read_file)"),
    ("http_get", "the network (http_get)"),
    ("system", "processes (system)"),
    ("exec", "processes (exec)"),
    ("env", "the environment (env)"),
    ("timestamp", "the clock (timestamp)"),
    ("rand", "randomness (rand)"),
];

/// What a RemedyLua error means for a widget.
fn classify(e: &Error) -> ScriptError {
    match e {
        Error::Limit(LimitKind::Fuel) => ScriptError::OutOfFuel,
        Error::Limit(LimitKind::Memory) => ScriptError::TooBig,
        Error::Limit(LimitKind::Time | LimitKind::Interrupted) => ScriptError::TooSlow,
        Error::Syntax(d) => ScriptError::Invalid(d.to_string()),
        Error::Runtime { message, .. } => {
            if message.contains("stack overflow") {
                return ScriptError::TooDeep;
            }
            // An over-long string is a catchable error in RemedyLua 0.1.0 (`string_len`),
            // not `Error::Limit(Memory)`. To the host it is still the memory cap.
            if message.contains("resulting string too large") {
                return ScriptError::TooBig;
            }
            if message.contains("widgets have no randomness") || message.contains("the ui library is fixed") {
                return ScriptError::Denied(message.clone());
            }
            if message.contains("elements nest too deeply") {
                return ScriptError::Shape("it nests too deeply".into());
            }
            for (name, what) in DENIED {
                if message.contains(&format!("(global '{name}')")) {
                    return ScriptError::Denied(what.to_string());
                }
            }
            ScriptError::Runtime(message.clone())
        }
        other => ScriptError::Runtime(other.message()),
    }
}

fn stopped_for_good() -> ScriptError {
    ScriptError::Runtime("the widget was stopped for good after a host error".into())
}

fn data_to_json(d: &Data) -> Json {
    match d {
        Data::Nil => Json::Null,
        Data::Bool(b) => Json::Bool(*b),
        Data::Int(i) => Json::from(*i),
        Data::Num(f) => serde_json::Number::from_f64(*f).map_or(Json::Null, Json::Number),
        Data::Str(s) => Json::String(s.clone()),
        Data::Table(pairs) => {
            let is_list = !pairs.is_empty()
                && pairs
                    .iter()
                    .enumerate()
                    .all(|(i, (k, _))| matches!(k, Data::Int(n) if *n == i as i64 + 1));
            if is_list {
                Json::Array(pairs.iter().map(|(_, v)| data_to_json(v)).collect())
            } else {
                let mut m = Map::new();
                for (k, v) in pairs {
                    let key = match k {
                        Data::Str(s) => s.clone(),
                        Data::Int(i) => i.to_string(),
                        Data::Num(f) => f.to_string(),
                        Data::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    m.insert(key, data_to_json(v));
                }
                Json::Object(m)
            }
        }
        // `Data` is non-exhaustive: a future variant stays representable.
        _ => Json::Null,
    }
}

fn json_to_data(v: &Json) -> Data {
    match v {
        Json::Null => Data::Nil,
        Json::Bool(b) => Data::Bool(*b),
        Json::Number(n) => n.as_i64().map_or_else(|| Data::Num(n.as_f64().unwrap_or(0.0)), Data::Int),
        Json::String(s) => Data::Str(s.clone()),
        Json::Array(a) => Data::Table(
            a.iter().enumerate().map(|(i, x)| (Data::Int(i as i64 + 1), json_to_data(x))).collect(),
        ),
        Json::Object(m) => {
            Data::Table(m.iter().map(|(k, x)| (Data::Str(k.clone()), json_to_data(x))).collect())
        }
    }
}

/// Bounds for copying a widget's values out: as deep as elements may nest, plus a little.
fn data_limits(limits: &Limits) -> DataLimits {
    DataLimits {
        depth: limits.nesting.saturating_mul(2).saturating_add(4),
        nodes: 100_000,
        bytes: limits.memory,
    }
}

/// A plain Lua value (no functions or handles) as JSON.
fn plain_json(lua: &Lua, v: Value, limits: &Limits) -> Result<Json, ScriptError> {
    lua.to_data(v, data_limits(limits))
        .map(|d| data_to_json(&d))
        .map_err(|e| ScriptError::Shape(format!("a value cannot be shown: {}", e.message())))
}

/// The description of element `id` as the protocol's JSON, its handlers collected.
fn describe_element(
    lua: &mut Lua,
    ctx: Ctx,
    id: Id,
    limits: &Limits,
    handlers: &mut Vec<Value>,
    depth: usize,
) -> Result<Json, ScriptError> {
    if depth >= limits.nesting {
        return Err(ScriptError::Shape("it nests too deeply".into()));
    }
    let found = with(lua, ctx, |w| w.elements.get(id).map(|e| (e.kind.clone(), e.props, e.children.clone())));
    let (kind, props, children) = found
        .map_err(|e| classify(&e))?
        .ok_or_else(|| ScriptError::Shape("that element was removed".into()))?;
    let mut m = Map::new();
    m.insert("kind".into(), Json::String(kind));
    if let Value::Table(t) = get_ref(lua, ctx, props) {
        for (k, v) in lua.table_pairs(t) {
            let Some(key) = lua.str_bytes(k).map(|b| String::from_utf8_lossy(b).into_owned()) else {
                continue;
            };
            if key == "kind" || key == "children" {
                continue;
            }
            let handler = key == "on" || key.starts_with("on_");
            let json = match v {
                Value::Func(_) if handler => {
                    handlers.push(v);
                    serde_json::json!({ "$fn": handlers.len() - 1 })
                }
                Value::Str(_) if handler => {
                    let name =
                        lua.str_bytes(v).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                    if name.is_empty() {
                        return Err(ScriptError::Shape(
                            "a button's handler must be a function or its name".into(),
                        ));
                    }
                    let g = lua.get_global(&name).map_err(|e| classify(&e))?;
                    if !matches!(g, Value::Func(_)) {
                        return Err(ScriptError::Shape(format!("there is no function {name}")));
                    }
                    handlers.push(g);
                    serde_json::json!({ "$fn": handlers.len() - 1 })
                }
                _ if handler => {
                    return Err(ScriptError::Shape(
                        "a button's handler must be a function or its name".into(),
                    ));
                }
                Value::Func(_) => {
                    return Err(ScriptError::Shape(format!("the property {key} is a function, not a value")));
                }
                Value::UserData(_) => match lua.userdata::<api::Handle>(v).copied() {
                    Some(h) if h.kind == Kind::Signal => {
                        let x = signal_get(lua, ctx, h.id, false).map_err(|e| classify(&e))?;
                        plain_json(lua, x, limits)?
                    }
                    _ => return Err(ScriptError::Shape(format!("the property {key} cannot be shown"))),
                },
                other => plain_json(lua, other, limits)?,
            };
            m.insert(key, json);
        }
    }
    let mut kids = Vec::with_capacity(children.len());
    for c in children {
        kids.push(describe_element(lua, ctx, c, limits, handlers, depth + 1)?);
    }
    if !kids.is_empty() {
        m.insert("children".into(), Json::Array(kids));
    }
    Ok(Json::Object(m))
}

/// Reads the dispatcher's mailbox (the state's app data).
fn mailbox(lua: &mut Lua) -> Result<&mut Mailbox, Error> {
    lua.app_data_mut::<Mailbox>().ok_or_else(|| err("the widget's mailbox is gone"))
}

/// Runs the timers due and the frame hooks for a move of the clock by `elapsed`.
fn advance_clock(lua: &mut Lua, ctx: Ctx, elapsed: Duration) -> Result<(), Error> {
    let moving = with(lua, ctx, |w| {
        let before = w.moving();
        w.now = w.now.saturating_add(elapsed);
        before
    })?;
    // What moved is read again.
    with(lua, ctx, |w| {
        for id in moving {
            w.notify(id);
        }
    })?;
    // Timers due, earliest first; each runs at most once a frame.
    let due: Vec<(Id, Ref, Option<Duration>)> = with(lua, ctx, |w| {
        let mut list: Vec<(Duration, Id, Ref, Option<Duration>)> = w
            .timers
            .ids()
            .into_iter()
            .filter_map(|id| {
                w.timers.get(id).filter(|t| t.due <= w.now).map(|t| (t.due, id, t.func, t.every))
            })
            .collect();
        list.sort_by_key(|(due, ..)| *due);
        list.into_iter().map(|(_, id, f, every)| (id, f, every)).collect()
    })?;
    for (id, func, every) in due {
        let alive = with(lua, ctx, |w| {
            let now = w.now;
            match (w.timers.get_mut(id), every) {
                (Some(t), Some(e)) => {
                    t.due = t.due.saturating_add(e).max(now);
                    true
                }
                (Some(_), None) => {
                    if let Some(t) = w.timers.remove(id) {
                        w.drop_ref(t.func);
                    }
                    true
                }
                (None, _) => false,
            }
        })?;
        if alive {
            let f = get_ref(lua, ctx, func);
            lua.call(f, &[])?;
            flush(lua, ctx)?;
        }
    }
    let frames: Vec<Ref> =
        with(lua, ctx, |w| w.frames.ids().into_iter().filter_map(|id| w.frames.get(id).copied()).collect())?;
    let ms = Value::Num(elapsed.as_secs_f64() * 1000.0);
    for f in frames {
        let f = get_ref(lua, ctx, f);
        if !f.is_nil() {
            lua.call(f, &[ms])?;
            flush(lua, ctx)?;
        }
    }
    if with(lua, ctx, |w| !w.frames.ids().is_empty())? {
        with(lua, ctx, |w| w.changed = true)?;
    }
    Ok(())
}

/// Runs the hooks in `list` (taken from the world).
fn run_hooks(lua: &mut Lua, ctx: Ctx, hooks: Vec<Ref>) -> Result<(), Error> {
    for h in hooks {
        let f = get_ref(lua, ctx, h);
        lua.call(f, &[])?;
        flush(lua, ctx)?;
    }
    Ok(())
}

/// The dispatcher's work for one call.
fn dispatch(lua: &mut Lua, ctx: Ctx, render_fn: Option<Ref>, args: &[Value]) -> Result<Vec<Value>, Error> {
    let op = mailbox(lua)?.op.take().ok_or_else(|| err("nothing to do"))?;
    with(lua, ctx, World::settle_after_stop)?;
    let out = match op {
        Op::Load => {
            let chunk = args.first().copied().unwrap_or(Value::Nil);
            let r = lua.call(chunk, &[])?;
            flush(lua, ctx)?;
            r
        }
        Op::Render { settings, host } => {
            // The settings and host data as signals, then the description.
            let (s_sig, h_sig) = with(lua, ctx, |w| (w.settings, w.host))?;
            let s = lua.from_data(&json_to_data(&settings))?;
            let h = lua.from_data(&json_to_data(&host))?;
            if let Some(id) = s_sig {
                api::signal_set(lua, ctx, id, s)?;
            }
            if let Some(id) = h_sig {
                api::signal_set(lua, ctx, id, h)?;
            }
            flush(lua, ctx)?;
            let root = match render_fn {
                Some(f) => {
                    // Elements the last render made are released now.
                    with(lua, ctx, |w| {
                        w.release_render();
                        w.rendering = true;
                    })?;
                    sweep_refs(lua, ctx)?;
                    let f = get_ref(lua, ctx, f);
                    let r = lua.call(f, &[s, h]);
                    with(lua, ctx, |w| w.rendering = false)?;
                    let r = r?;
                    flush(lua, ctx)?;
                    match r.first().copied() {
                        Some(v @ Value::UserData(_)) => match lua.userdata::<api::Handle>(v).copied() {
                            Some(hd) if hd.kind == Kind::Element => Some(hd.id),
                            _ => return Err(err("render must return an element made with ui.*")),
                        },
                        _ => return Err(err("render must return an element made with ui.*")),
                    }
                }
                None => with(lua, ctx, |w| w.root)?,
            };
            let first = with(lua, ctx, |w| !w.mounted)?;
            if first && root.is_some() {
                let hooks = with(lua, ctx, |w| {
                    w.mounted = true;
                    w.on_mount.clone()
                })?;
                run_hooks(lua, ctx, hooks)?;
            }
            let Some(root) = root else {
                return Err(err("the widget shows nothing: call ui.mount(element) or define render"));
            };
            // The root's id, for the widget to describe after the call (with the world settled).
            with(lua, ctx, |w| w.root_shown = Some(root))?;
            Vec::new()
        }
        Op::Act { handler, value } => {
            let f = get_ref(lua, ctx, handler);
            let args = match value {
                Some(v) => vec![lua.from_data(&json_to_data(&v))?],
                None => Vec::new(),
            };
            lua.call(f, &args)?;
            flush(lua, ctx)?;
            Vec::new()
        }
        Op::Advance(dt) => {
            advance_clock(lua, ctx, dt)?;
            Vec::new()
        }
        Op::Theme(tokens) => {
            let t = lua.from_data(&json_to_data(&tokens))?;
            if let Some(id) = with(lua, ctx, |w| w.theme)? {
                api::signal_set(lua, ctx, id, t)?;
            }
            flush(lua, ctx)?;
            Vec::new()
        }
        Op::Unmount => {
            let hooks = with(lua, ctx, |w| std::mem::take(&mut w.on_unmount))?;
            run_hooks(lua, ctx, hooks)?;
            Vec::new()
        }
    };
    sweep_refs(lua, ctx)?;
    Ok(out)
}

/// Builds a widget's state: the `refs` table, the world, the mailbox, the `ui` library, the
/// host's values as signals and the granted app functions.
fn setup(lua: &mut Lua, limits: &Limits, grants: &Grants) -> Result<(Ctx, Root), Error> {
    let refs = lua.create_table()?;
    let refs_root = lua.root(Value::Table(refs));
    let ctx = api::make_ctx(lua, World::new(api::caps(limits.memory, limits.nesting)), refs)?;
    lua.set_app_data(Mailbox::default());
    let ui = api::install(lua, ctx, &grants.elements)?;
    // The host's data, as values scripts read (and computed values follow).
    let (settings, s_id) = api::host_signal(lua, ctx, Value::Nil)?;
    let (host, h_id) = api::host_signal(lua, ctx, Value::Nil)?;
    let theme_v = lua.from_data(&json_to_data(&grants.theme))?;
    let (theme, t_id) = api::host_signal(lua, ctx, theme_v)?;
    with(lua, ctx, |w| {
        w.reduced_motion = grants.reduced_motion;
        w.settings = Some(s_id);
        w.host = Some(h_id);
        w.theme = Some(t_id);
    })?;
    for (name, v) in [("settings", settings), ("host", host), ("theme", theme)] {
        let k = lua.create_string(name)?;
        lua.table_set(ui, k, v)?;
    }
    lua.set_global("ui", Value::Table(ui))?;
    if !grants.functions.is_empty() {
        let app = lua.create_table()?;
        for (name, f) in &grants.functions {
            let (f, dl) = (f.clone(), data_limits(limits));
            let func = lua.create_function(name, move |lua, args| {
                let mut json = Vec::with_capacity(args.len());
                for a in args {
                    json.push(data_to_json(&lua.to_data(*a, dl)?));
                }
                let out = f(&json).map_err(Error::runtime)?;
                Ok(vec![lua.from_data(&json_to_data(&out))?])
            })?;
            let k = lua.create_string(name)?;
            lua.table_set(app, k, func)?;
        }
        lua.set_global("app", Value::Table(app))?;
    }
    // `this` is the widget's own state, kept across calls.
    let this = lua.create_table()?;
    lua.set_global("this", Value::Table(this))?;
    // The `ui` table is fixed, and there is no clock or randomness, so the same calls
    // describe the same widget and a script cannot replace the constructors.
    lua.exec(
        "do local raw = ui \
         ui = setmetatable({}, { \
           __index = raw, \
           __newindex = function() error('the ui library is fixed', 2) end, \
           __metatable = 'locked' }) \
         end \
         math.random = function() error('widgets have no randomness (math.random)', 2) end \
         math.randomseed = function() error('widgets have no randomness (math.randomseed)', 2) end",
        "=sandbox",
    )?;
    Ok((ctx, refs_root))
}

/// The dispatcher: one host function every call enters the state through, so a whole call
/// (its handler, the effects and timers it sets off) runs on one budget.
fn dispatcher(lua: &mut Lua, ctx: Ctx, render_fn: Option<Ref>) -> Result<Root, Error> {
    let f = lua.create_function("widget", move |lua, args| dispatch(lua, ctx, render_fn, args))?;
    Ok(lua.root(f))
}

impl LuaWidget {
    /// Compiles and runs `source` (its setup) with `limits` and `grants`, and reads its
    /// manifest: the table the chunk returns (`name`, `settings`, and optionally `render`).
    pub fn load(source: &str, limits: Limits, grants: &Grants) -> Result<LuaWidget, ScriptError> {
        RATE.calibrate_once("remedy-ui-script-lua calibration", measure);
        let libs = Libs::BASE | Libs::STRING | Libs::TABLE | Libs::MATH | Libs::UTF8 | Libs::COROUTINE;
        let libs = if grants.print { libs | Libs::PRINT } else { libs };
        // A seed from the source: hashing (and so `pairs` order and fuel) is the same every
        // time this widget runs. A script that floods its own tables only spends its own
        // fuel (probing is charged), so a fixed seed costs no safety here.
        let seed = source
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
        let mut lua = Lua::new(lua_limits(&limits).hash_seed(seed), libs);
        if grants.print {
            lua.set_print(|s| {
                remedy_ui_core::trace::tracing::debug!(target: "remedy_ui::script", "{s}");
            });
        }
        let (ctx, refs) = setup(&mut lua, &limits, grants)
            .map_err(|e| ScriptError::Invalid(format!("the widget could not be set up: {e}")))?;
        let dispatch = dispatcher(&mut lua, ctx, None).map_err(|e| classify(&e))?;
        let (chunk_name, dialect) = source_dialect(source);
        if dialect == Dialect::RemedyLua {
            let errors: Vec<String> = remedy_lua::check(source.as_bytes(), chunk_name)
                .into_iter()
                .filter(|d| d.severity == remedy_lua::Severity::Error)
                .map(|d| d.lua_message())
                .collect();
            if !errors.is_empty() {
                return Err(ScriptError::Invalid(errors.join("\n")));
            }
        }
        let chunk = lua.load_with(source, chunk_name, dialect).map_err(|e| classify(&e))?;
        let manifest = crate::script::manifest(&serde_json::json!({ "name": "Widget" }))?;
        let mut widget = LuaWidget {
            lua,
            ctx,
            dispatch,
            _refs: refs,
            manifest,
            limits,
            offered: grants.elements.clone(),
            last: None,
            broken: false,
            unmounted: false,
        };
        let returned = widget.call(Op::Load, &[chunk.value()]).map_err(|e| match e {
            ScriptError::Runtime(t) => ScriptError::Invalid(t),
            other => other,
        })?;
        let Some(Value::Table(m)) = returned.first().copied() else {
            return Err(ScriptError::Invalid(
                "it must return its manifest, a table with its name and settings (and render)".into(),
            ));
        };
        // `render`, if given, is kept; the rest is the manifest.
        let mut render = None;
        let plain = widget.lua.create_table().map_err(|e| classify(&e))?;
        for (k, v) in widget.lua.table_pairs(m) {
            if widget.lua.str_bytes(k) == Some(b"render".as_slice()) {
                if !matches!(v, Value::Func(_)) {
                    return Err(ScriptError::Invalid("render must be a function".into()));
                }
                render = Some(v);
            } else if !matches!(v, Value::Func(_) | Value::UserData(_)) {
                widget.lua.table_set(plain, k, v).map_err(|e| classify(&e))?;
            }
        }
        widget.manifest =
            crate::script::manifest(&plain_json(&widget.lua, Value::Table(plain), &limits)?)?;
        if let Some(r) = render {
            let key = api::keep(&mut widget.lua, widget.ctx, r).map_err(|e| classify(&e))?;
            widget.dispatch = dispatcher(&mut widget.lua, ctx, Some(key)).map_err(|e| classify(&e))?;
        } else {
            let mounted =
                with(&mut widget.lua, widget.ctx, |w| w.root.is_some()).map_err(|e| classify(&e))?;
            if !mounted {
                return Err(ScriptError::Invalid(
                    "the widget shows nothing: it has no render(settings, host) and did not call ui.mount"
                        .into(),
                ));
            }
        }
        Ok(widget)
    }

    /// Runs `op` as one entry into the state, against the limits; afterwards the world is
    /// settled whatever happened.
    fn call(&mut self, op: Op, args: &[Value]) -> Result<Vec<Value>, ScriptError> {
        if self.broken {
            return Err(stopped_for_good());
        }
        // The state keeps its own hash seed across set_limits.
        self.lua.set_limits(lua_limits(&self.limits));
        // The time limit, by this thread's work clock, as the backstop to the fuel.
        let start = WorkClock::start();
        let clock: remedy_lua::Clock = Arc::new(move || start.work());
        self.lua.set_time_limit(Some((clock, self.limits.time)));
        match mailbox(&mut self.lua) {
            Ok(m) => m.op = Some(op),
            Err(_) => {
                self.broken = true;
                return Err(ScriptError::Runtime("the widget's state is gone".into()));
            }
        }
        let began = WorkClock::start();
        let fuel = self.lua.limits().fuel;
        let r = self.lua.call(self.dispatch.value(), args);
        match &r {
            // The watchdog's stop reached this call: it is spent, and the next call runs. A stop
            // set before a call starts stops that call (it is never cleared unseen).
            Err(Error::Limit(LimitKind::Interrupted)) => self.lua.interrupt_handle().reset(),
            // The state refuses calls for good (a panic in an earlier call poisoned it): nothing
            // more is asked of it.
            Err(Error::Host(_)) => {
                self.broken = true;
                return Err(stopped_for_good());
            }
            _ => {}
        }
        let used = fuel.saturating_sub(self.lua.fuel_left());
        let _ = RATE.record(used, began.work());
        let _ = with(&mut self.lua, self.ctx, World::settle_after_stop);
        self.release_dropped();
        let _ = sweep_refs(&mut self.lua, self.ctx);
        r.map_err(|e| classify(&e))
    }

    /// Releases what scripts let go of: an element no longer shown and not inside one that is,
    /// or a value nothing reads, whose last handle the collector has freed. Elements inside a
    /// released one that scripts still hold handles to are kept, detached.
    fn release_dropped(&mut self) {
        let ctx = self.ctx;
        for key in self.lua.take_collected() {
            let Ok(key) = i64::try_from(key) else { continue };
            let (kind, idx, low) = api::from_handle_key(key);
            let found = with(&mut self.lua, ctx, |w| match kind {
                Kind::Signal => w.signals.at(idx, low),
                _ => w.elements.at(idx, low),
            });
            let Ok(Some(id)) = found else { continue };
            // A new handle may have been made since this one went.
            if api::live_handle(&self.lua, ctx, kind, id).is_some() {
                continue;
            }
            if kind == Kind::Signal {
                let _ = with(&mut self.lua, ctx, |w| w.free_signal(id));
                continue;
            }
            // An element: only a detached one that is not shown goes, with what is inside it
            // and not held by a script.
            let gone = with(&mut self.lua, ctx, |w| {
                let detached = w.elements.get(id).is_some_and(|e| e.parent.is_none()) && w.root != Some(id);
                if !detached {
                    return Vec::new();
                }
                let mut all = vec![id];
                let mut i = 0;
                while let Some(n) = all.get(i).copied() {
                    all.extend(w.elements.get(n).map(|e| e.children.clone()).unwrap_or_default());
                    i += 1;
                }
                all
            });
            let Ok(subtree) = gone else { continue };
            let held: Vec<Id> = subtree
                .iter()
                .skip(1)
                .copied()
                .filter(|c| api::live_handle(&self.lua, ctx, Kind::Element, *c).is_some())
                .collect();
            let _ = with(&mut self.lua, ctx, |w| {
                for h in &held {
                    // Kept: taken out first, so freeing the rest leaves it whole.
                    if let Some(p) = w.elements.get(*h).and_then(|e| e.parent)
                        && let Some(parent) = w.elements.get_mut(p)
                    {
                        parent.children.retain(|c| c != h);
                    }
                    if let Some(e) = w.elements.get_mut(*h) {
                        e.parent = None;
                    }
                }
                if subtree.first().is_some_and(|r| w.elements.contains(*r)) {
                    w.free_element(id);
                }
            });
        }
    }

    /// A handle that stops this widget's running call from any thread (a watchdog): the call
    /// ends with [`ScriptError::TooSlow`], and the next call runs normally. Set between calls,
    /// it stops the next call.
    pub fn interrupt_handle(&self) -> remedy_lua::Interrupt {
        self.lua.interrupt_handle()
    }

    /// Replaces the theme tokens: what reads them runs again, and the widget shows the change
    /// at its next render.
    pub fn set_theme(&mut self, tokens: &Json) -> Result<(), ScriptError> {
        self.call(Op::Theme(tokens.clone()), &[]).map(|_| ())
    }

    /// Describes what the widget shows now (after a render's call).
    fn describe_shown(&mut self) -> Result<Node, ScriptError> {
        let root = with(&mut self.lua, self.ctx, |w| w.root_shown.take()).map_err(|e| classify(&e))?;
        let root = root.ok_or_else(|| ScriptError::Shape("the widget shows nothing".into()))?;
        let mut handlers = Vec::new();
        let json = describe_element(&mut self.lua, self.ctx, root, &self.limits, &mut handlers, 0)?;
        let offered = self.offered.clone();
        let described =
            crate::script::describe_with(&json, &self.limits, &|k| offered.iter().any(|o| o == k))?;
        // The handlers stay reachable until the next description.
        let mut keys = Vec::with_capacity(handlers.len());
        for h in handlers {
            keys.push(api::keep(&mut self.lua, self.ctx, h).map_err(|e| classify(&e))?);
        }
        let _ = with(&mut self.lua, self.ctx, |w| {
            for r in std::mem::replace(&mut w.handlers, keys) {
                w.drop_ref(r);
            }
        });
        let _ = sweep_refs(&mut self.lua, self.ctx);
        let _ = with(&mut self.lua, self.ctx, |w| w.changed = false);
        let node = described.node.clone();
        self.last = Some(described);
        Ok(node)
    }

    /// The handler of `action` in the last description.
    fn handler(&mut self, action: Action) -> Result<Ref, ScriptError> {
        let gone = || ScriptError::Shape("that button is gone".into());
        let on = self.last.as_ref().ok_or_else(gone)?.handler(action)?;
        let i =
            on.get("$fn").and_then(Json::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(gone)?;
        let keys = with(&mut self.lua, self.ctx, |w| w.handlers.get(i).copied()).map_err(|e| classify(&e))?;
        keys.ok_or_else(gone)
    }

    /// The number of elements the widget holds.
    pub fn elements(&mut self) -> usize {
        with(&mut self.lua, self.ctx, |w| w.elements.len()).unwrap_or(0)
    }
}

/// RemedyLua limits for a widget's `limits` (fuel set per call).
fn lua_limits(limits: &Limits) -> remedy_lua::Limits {
    let mut l = remedy_lua::Limits::default()
        .memory(limits.memory)
        // A host-set depth is honored. One frame is the least a call can be.
        .call_depth(limits.call_depth.max(1))
        // Library callbacks need a little native depth; RemedyLua caps it near 200.
        .native_depth(limits.call_depth.clamp(8, 200))
        .string_len(limits.memory / 4)
        .output(64 * 1024)
        .fuel(fuel_budget(limits));
    l.stack_bytes = limits.stack;
    l
}

/// The fuel one call may use: [`Limits::step_budget`]. Until the rate is measured that is
/// no step cap, and [`Lua::set_time_limit`] is what stops a runaway call.
fn fuel_budget(limits: &Limits) -> u64 {
    limits.step_budget(&RATE)
}

/// RemedyLua when the first non-empty line is `-- rlua` (the package stores bytes, not a
/// file name); otherwise plain Lua 5.4. The chunk name selects the same dialect.
fn source_dialect(source: &str) -> (&'static str, Dialect) {
    let marked = source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .is_some_and(|l| l == "-- rlua" || l.starts_with("-- rlua "));
    if marked { ("@widget.rlua", Dialect::RemedyLua) } else { ("@widget.lua", Dialect::Lua54) }
}

impl ScriptWidget for LuaWidget {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn settings(&self) -> &[SettingSpec] {
        &self.manifest.settings
    }

    fn limits(&self) -> Limits {
        self.limits
    }

    fn language(&self) -> Language {
        Language::LUA
    }

    fn render(&mut self, settings: &Map<String, Json>, host: &Json) -> Result<Node, ScriptError> {
        let values = Json::Object(crate::script::resolve_settings(&self.manifest.settings, settings));
        self.call(Op::Render { settings: values, host: host.clone() }, &[])?;
        self.describe_shown()
    }

    fn act(&mut self, action: Action) -> Result<(), ScriptError> {
        let handler = self.handler(action)?;
        self.call(Op::Act { handler, value: None }, &[]).map(|_| ())
    }

    fn act_with(&mut self, action: Action, value: &Json) -> Result<(), ScriptError> {
        let handler = self.handler(action)?;
        self.call(Op::Act { handler, value: Some(value.clone()) }, &[]).map(|_| ())
    }

    fn memory_used(&self) -> usize {
        self.lua.memory_used()
    }

    fn advance(&mut self, elapsed: Duration) -> Result<bool, ScriptError> {
        self.call(Op::Advance(elapsed), &[])?;
        with(&mut self.lua, self.ctx, |w| w.changed).map_err(|e| classify(&e))
    }

    fn unmount(&mut self) {
        if self.unmounted {
            return;
        }
        self.unmounted = true;
        let has_hooks = self.lua.app_data::<World>().is_some_and(|w| !w.on_unmount.is_empty());
        if has_hooks && !self.broken {
            let _ = self.call(Op::Unmount, &[]);
        }
    }

    fn next_frame(&self) -> Option<Duration> {
        let w = self.lua.app_data::<World>()?;
        if w.animating() || !w.frames.ids().is_empty() {
            return Some(FRAME);
        }
        w.timers
            .ids()
            .into_iter()
            .filter_map(|id| w.timers.get(id))
            .map(|t| t.due.saturating_sub(w.now))
            .min()
    }
}

impl Drop for LuaWidget {
    /// The unmount hooks run as the widget goes (their errors have nowhere to go).
    fn drop(&mut self) {
        if !self.unmounted {
            self.unmount();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_lua_data_round_trip() {
        let v = serde_json::json!({ "a": [1, 2.5, "x", true, null], "b": { "c": "d" } });
        let back = data_to_json(&json_to_data(&v));
        assert_eq!(back.get("b"), v.get("b"));
        assert_eq!(back.get("a").and_then(|a| a.get(1)), Some(&serde_json::json!(2.5)));
    }
}
