//! RemedyUI scripted widgets in [Rhai](https://rhai.rs): the language for widgets people
//! write by hand. Rhai is pure Rust (no C, no C++), so it runs on every platform RemedyUI
//! does, browser pages included.
//!
//! [`RhaiEngine`] is a [`ScriptEngine`]: enable it in an app's or a board's
//! [`Engines`](remedy_ui_script::Engines) and widgets whose [`Package`](remedy_ui_script::Package)
//! says `rhai` load through it.
//!
//! A widget is a script whose last expression is its manifest (its name and settings),
//! with a `render(settings, host)` function that returns elements built with the `ui`
//! functions, and the handlers its buttons name. `this` in `render` and in handlers is the
//! widget's own state, a map kept between calls.
//!
//! ```rhai
//! fn render(settings, host) {
//!     let claps = this.claps ?? 0;
//!     ui::column(#{ gap: 8 }, [
//!         ui::heading(settings.title),
//!         ui::text(`${claps} claps from ${host.house ?? "nobody"}`),
//!         ui::button("Clap", "clap"),
//!     ])
//! }
//!
//! fn clap() { this.claps = (this.claps ?? 0) + 1; }
//!
//! #{ name: "Claps", settings: [ #{ key: "title", label: "Title", kind: "text", value: "Claps" } ] }
//! ```
//!
//! The `ui` functions return the maps of the element protocol (`remedy_ui_script`'s crate
//! docs): `ui::column([..])` or `ui::column(#{ gap }, [..])`, `ui::row` the same,
//! `ui::text(x)`, `ui::muted(x)`, `ui::strong(x)`, `ui::heading(x)` or `ui::heading(x,
//! level)`, `ui::button(label, handler)` (a handler is the name of a function, or a function
//! pointer such as `Fn("vote").curry(2)` or a closure), `ui::link(label, url)`,
//! `ui::progress(label, value)` and `ui::divider()`. A script may also build the maps itself.
//!
//! The sandbox. A widget gets an engine of its own with only Rhai's pure standard library
//! and the `ui` functions: there is no file, network or process access in Rhai to begin
//! with, `import` and `eval` are switched off, there is no clock and no randomness (the same
//! calls describe the same widget), and `print` and `debug` go to the engine's trace. Each
//! call (loading, a render, a handler) runs against the [`Limits`]:
//!
//! - an operation budget (Rhai counts every step), stopping runaway loops deterministically
//!   ([`ScriptError::OutOfFuel`]), and the time budget on the page-safe clock as the backstop
//!   ([`ScriptError::TooSlow`]): the engine checks it while the script runs, so even a tight
//!   loop is interrupted;
//! - the call depth ([`ScriptError::TooDeep`]);
//! - memory ([`ScriptError::TooBig`]): Rhai allocates through Rust's allocator and cannot
//!   meter it, so the engine bounds what memory a script can reach instead: every string,
//!   array and map (with everything inside it) is kept within `memory`, so is the state the
//!   widget keeps between calls (checked after each call), and the operation budget bounds
//!   how many values one call can make.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_core::time::WorkClock;
use remedy_ui_script::StepRate;
use remedy_ui_script::{
    Action, Described, Language, Limits, Manifest, Node, ScriptEngine, ScriptError, ScriptWidget, SettingSpec,
};
use rhai::{AST, Array, CallFnOptions, Dynamic, Engine, EvalAltResult, FnPtr, INT, Map, Module, Scope};
use serde_json::Value;

/// Rhai operations a millisecond assumed only in the moment before this device's rate is
/// measured ([`rate`]): low on purpose (an eighth of what a current desktop core measures,
/// metering included), so a step budget made then errs short rather than long.
pub const FALLBACK_OPERATIONS_PER_MS: u64 = 1_000;

/// Rhai operations a millisecond on this device, for the whole process.
static RATE: StepRate = StepRate::new(FALLBACK_OPERATIONS_PER_MS);

/// How many Rhai operations a millisecond buys on this device: what a time budget becomes
/// when [`Limits::operations`] is not given. Calibrated the first time a widget loads (a
/// fixed script run for a few milliseconds away from the UI thread), then kept current by
/// every call long enough to time well, so it follows the device when it slows down or
/// speeds up. [`FALLBACK_OPERATIONS_PER_MS`] until the first measurement.
pub fn rate() -> &'static StepRate {
    &RATE
}

/// The fixed script calibration runs: function calls, arithmetic, list and map updates and a
/// branch, the mix widgets are made of, metered like any widget.
const CALIBRATION: &str = "fn step(n) { n + 1 } \
    fn render(s, h) { let a = []; let m = #{}; let n = 0; \
      loop { n = step(n); a.push(n); m.last = n; if a.len() > 32 { a = []; } } } \
    #{ name: \"calibration\" }";

/// Runs the calibration script for a few milliseconds on this thread: the operations it took
/// and how long they took, or `None` when it could not run.
pub fn measure() -> Option<(u64, std::time::Duration)> {
    remedy_ui_script::calibrate_with(FALLBACK_OPERATIONS_PER_MS, |steps| {
        let limits = Limits::default().operations(Some(steps)).time(std::time::Duration::from_secs(10));
        let mut w = RhaiWidget::load(CALIBRATION, limits).ok()?;
        let start = WorkClock::start();
        let r = w.render(&serde_json::Map::new(), &Value::Null);
        (r == Err(ScriptError::OutOfFuel)).then(|| start.work())
    })
}

/// Measures this device's rate now, on this thread, and records it: the operations a
/// millisecond it measured, or `None` when the calibration could not run.
pub fn calibrate() -> Option<u64> {
    let (steps, took) = measure()?;
    RATE.record(steps, took).then(|| RATE.per_ms())
}

/// How deeply expressions may nest at the top level and in a function: what readable code
/// needs (Rhai's own optimized defaults). The parser recurses on these before any step is
/// metered, and at a few KiB a level both fit well within the stack [`Limits`] gives a call.
const EXPR_DEPTH: usize = 64;
const FN_EXPR_DEPTH: usize = 32;

/// How many operations pass between looks at the clock: often enough that a call overruns
/// its time budget by a sliver at most, rarely enough that reading the clock costs nothing
/// that shows.
const CLOCK_EVERY: u64 = 64;

/// How many steps pass between measurements of what a call holds (its variables and the
/// widget's state). One step makes at most one value, and no value may exceed
/// `memory / MEASURE_EVERY`, so a call never holds more than twice its memory limit before
/// it is stopped.
const MEASURE_EVERY: u64 = 64;

/// The engine for Rhai widgets.
#[derive(Clone, Copy, Debug, Default)]
pub struct RhaiEngine;

impl RhaiEngine {
    /// The engine.
    pub fn new() -> RhaiEngine {
        RhaiEngine
    }

    /// Loads the widget in `source` with `limits`.
    pub fn load_source(&self, source: &str, limits: Limits) -> Result<RhaiWidget, ScriptError> {
        RhaiWidget::load(source, limits)
    }
}

impl ScriptEngine for RhaiEngine {
    fn language(&self) -> Language {
        Language::RHAI
    }

    fn load(&self, code: &[u8], limits: Limits) -> Result<Box<dyn ScriptWidget>, ScriptError> {
        let source = std::str::from_utf8(code)
            .map_err(|e| ScriptError::Invalid(format!("a Rhai widget is text, and this is not: {e}")))?;
        Ok(Box::new(RhaiWidget::load(source, limits)?))
    }
}

/// One Rhai widget: its engine, its compiled script, its state.
pub struct RhaiWidget {
    engine: Engine,
    ast: AST,
    manifest: Manifest,
    state: Dynamic,
    last: Option<Described>,
    fns: Vec<FnPtr>,
    meter: Rc<Meter>,
    limits: Limits,
}

/// What a running call is measured against: the processor time it may use and where its stack
/// began. Rhai's evaluator recurses on the caller's stack, so the engine measures how far
/// the stack has grown at every step and stops the script before it can overflow it.
///
/// Rhai allocates through Rust's allocator, which no library can meter for one script, and
/// does not check a map that grows one key at a time. So every [`MEASURE_EVERY`] steps the
/// engine measures what the call holds, its variables and the widget's state, and stops it
/// past `memory`, or when values nest deeper than a widget's elements may (values nested
/// without end would take the stack to take apart).
#[derive(Default)]
struct Meter {
    /// The processor time the running call may use (none between calls).
    allowed: Cell<Option<std::time::Duration>>,
    stack_base: Cell<usize>,
    stack: Cell<usize>,
    steps: Cell<u64>,
    memory: Cell<usize>,
    depth: Cell<usize>,
    /// The thread's clock when the call began, and the operations it has taken: a measurement
    /// of the rate.
    began: Cell<Option<WorkClock>>,
    ops: Cell<u64>,
}

impl Meter {
    fn start(&self, limits: &Limits) {
        self.began.set(Some(WorkClock::start()));
        self.ops.set(0);
        self.allowed.set(Some(limits.time));
        self.stack_base.set(stack_position());
        self.stack.set(limits.stack);
        self.steps.set(0);
        self.memory.set(limits.memory);
        self.depth.set(value_depth(limits));
    }

    /// One step of the script: now and then, measures what it holds.
    fn step(&self, ctx: &rhai::EvalContext<'_, '_, '_, '_, '_, '_>) -> Result<(), Box<EvalAltResult>> {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if !n.is_multiple_of(MEASURE_EVERY) {
            return Ok(());
        }
        let mut budget = Budget { bytes: self.memory.get(), depth: self.depth.get(), too_deep: false };
        let held = ctx.scope().iter_raw().map(|(_, _, v)| v).chain(ctx.this_ptr());
        for v in held {
            if !budget.take(v, 0) {
                return Err(if budget.too_deep {
                    EvalAltResult::ErrorTerminated(NEST_UP.into(), rhai::Position::NONE).into()
                } else {
                    EvalAltResult::ErrorDataTooLarge("what the widget holds".into(), rhai::Position::NONE)
                        .into()
                });
            }
        }
        Ok(())
    }

    /// Ends the call, and records its operations over its time as a measurement of the rate.
    fn stop(&self) {
        self.allowed.set(None);
        if let Some(began) = self.began.take() {
            let _ = RATE.record(self.ops.get(), began.work());
        }
    }

    /// Why the call must stop now, if it must.
    fn check(&self, ops: u64) -> Option<Dynamic> {
        self.ops.set(ops);
        let used = self.stack_base.get().abs_diff(stack_position());
        if used > self.stack.get() {
            return Some(STACK_UP.into());
        }
        if !ops.is_multiple_of(CLOCK_EVERY) {
            return None;
        }
        match (self.allowed.get(), self.began.get()) {
            (Some(allowed), Some(began)) if began.over(allowed) && RATE.ran_for(ops, allowed) => {
                Some(TIME_UP.into())
            }
            _ => None,
        }
    }
}

/// Where the stack is now: the address of a local. Stacks grow down on every target
/// RemedyUI runs on (and so does a browser page's shadow stack), so the distance from a
/// call's first position is the stack it has used.
#[inline(never)]
fn stack_position() -> usize {
    let here = 0u8;
    std::ptr::from_ref(std::hint::black_box(&here)).addr()
}

impl std::fmt::Debug for RhaiWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RhaiWidget").field("name", &self.manifest.name).field("limits", &self.limits).finish()
    }
}

/// The message a stop for time carries.
const TIME_UP: &str = "time budget used up";
/// The message a stop for stack carries.
const STACK_UP: &str = "stack budget used up";
/// The message a stop for values nested too deeply carries.
const NEST_UP: &str = "values nest too deeply";

/// The `ui` functions.
fn ui_module() -> Module {
    fn el(kind: &str) -> Map {
        let mut m = Map::new();
        m.insert("kind".into(), kind.into());
        m
    }
    fn text(s: &Dynamic, style: &str) -> Dynamic {
        let mut m = el("text");
        m.insert("text".into(), s.to_string().into());
        m.insert("style".into(), style.into());
        Dynamic::from_map(m)
    }
    fn group(kind: &str, opts: &Map, children: Array) -> Dynamic {
        let mut m = el(kind);
        m.insert("gap".into(), opts.get("gap").cloned().unwrap_or(Dynamic::from_int(0)));
        m.insert("children".into(), Dynamic::from_array(children));
        Dynamic::from_map(m)
    }
    type Out = Result<Dynamic, Box<EvalAltResult>>;
    let mut m = Module::new();
    m.set_native_fn("column", |c: Array| -> Out { Ok(group("column", &Map::new(), c)) });
    m.set_native_fn("column", |o: Map, c: Array| -> Out { Ok(group("column", &o, c)) });
    m.set_native_fn("row", |c: Array| -> Out { Ok(group("row", &Map::new(), c)) });
    m.set_native_fn("row", |o: Map, c: Array| -> Out { Ok(group("row", &o, c)) });
    m.set_native_fn("text", |s: Dynamic| -> Out { Ok(text(&s, "body")) });
    m.set_native_fn("muted", |s: Dynamic| -> Out { Ok(text(&s, "muted")) });
    m.set_native_fn("strong", |s: Dynamic| -> Out { Ok(text(&s, "strong")) });
    m.set_native_fn("heading", |s: Dynamic| -> Out { Ok(text(&s, "heading")) });
    m.set_native_fn("heading", |s: Dynamic, level: INT| -> Out {
        let d = text(&s, "heading");
        let Some(mut m) = d.try_cast::<Map>() else { return Ok(Dynamic::UNIT) };
        m.insert("level".into(), level.into());
        Ok(Dynamic::from_map(m))
    });
    m.set_native_fn("button", |label: Dynamic, on: Dynamic| -> Out {
        let mut m = el("button");
        m.insert("label".into(), label.to_string().into());
        m.insert("on".into(), on);
        Ok(Dynamic::from_map(m))
    });
    m.set_native_fn("link", |label: Dynamic, url: Dynamic| -> Out {
        let mut m = el("link");
        m.insert("label".into(), label.to_string().into());
        m.insert("url".into(), url.to_string().into());
        Ok(Dynamic::from_map(m))
    });
    m.set_native_fn("progress", |label: Dynamic, value: Dynamic| -> Out {
        let mut m = el("progress");
        m.insert("label".into(), label.to_string().into());
        let v = value.as_float().or_else(|_| value.as_int().map(|i| i as f64)).unwrap_or(0.0);
        m.insert("value".into(), v.into());
        Ok(Dynamic::from_map(m))
    });
    m.set_native_fn("divider", || -> Out { Ok(Dynamic::from_map(el("divider"))) });
    m
}

/// The innermost error of a chain of calls.
fn innermost(e: &EvalAltResult) -> &EvalAltResult {
    match e {
        EvalAltResult::ErrorInFunctionCall(_, _, inner, _) | EvalAltResult::ErrorInModule(_, inner, _) => {
            innermost(inner)
        }
        other => other,
    }
}

/// What a Rhai error means for a widget.
fn classify(e: &EvalAltResult) -> ScriptError {
    match innermost(e) {
        EvalAltResult::ErrorTooManyOperations(_) => ScriptError::OutOfFuel,
        EvalAltResult::ErrorTerminated(why, _) if why.to_string() == STACK_UP => ScriptError::TooDeep,
        EvalAltResult::ErrorTerminated(why, _) if why.to_string() == NEST_UP => {
            ScriptError::Shape("its values nest deeper than a widget's elements may".into())
        }
        EvalAltResult::ErrorTerminated(..) => ScriptError::TooSlow,
        EvalAltResult::ErrorStackOverflow(_) => ScriptError::TooDeep,
        EvalAltResult::ErrorDataTooLarge(..) | EvalAltResult::ErrorTooManyVariables(_) => ScriptError::TooBig,
        EvalAltResult::ErrorFunctionNotFound(f, _) => ScriptError::Denied(format!(
            "there is no function {f} for widgets (they reach only the ui functions and Rhai's own)"
        )),
        EvalAltResult::ErrorModuleNotFound(m, _) => ScriptError::Denied(format!("loading other code ({m})")),
        EvalAltResult::ErrorParsing(..) => ScriptError::Invalid(e.to_string()),
        _ => ScriptError::Runtime(e.to_string()),
    }
}

/// Symbols switched off in widgets, and what they would reach.
const DENIED: &[(&str, &str)] =
    &[("eval", "running code built from text (eval)"), ("import", "loading other code (import)")];

/// A JSON value as Rhai data.
fn to_dynamic(v: &Value) -> Dynamic {
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Dynamic::from_int(i),
            None => Dynamic::from_float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => s.clone().into(),
        Value::Array(a) => Dynamic::from_array(a.iter().map(to_dynamic).collect()),
        Value::Object(m) => {
            Dynamic::from_map(m.iter().map(|(k, v)| (k.as_str().into(), to_dynamic(v))).collect())
        }
    }
}

/// Rhai data as JSON, function pointers set aside in `fns` and named `{ "$fn": index }`.
fn to_json(d: &Dynamic, fns: &mut Vec<FnPtr>, depth: usize, limits: &Limits) -> Result<Value, ScriptError> {
    // Elements nest as a map holding a list of maps, so a tree `nesting` deep is twice that
    // in values, plus the button handlers inside.
    if depth > limits.nesting.saturating_mul(2).saturating_add(2) {
        return Err(ScriptError::Shape("it nests too deeply".into()));
    }
    let d = d.flatten_clone();
    if d.is_unit() {
        return Ok(Value::Null);
    }
    if let Ok(b) = d.as_bool() {
        return Ok(Value::Bool(b));
    }
    if let Ok(i) = d.as_int() {
        return Ok(Value::from(i));
    }
    if let Ok(f) = d.as_float() {
        return Ok(serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number));
    }
    if let Ok(c) = d.as_char() {
        return Ok(Value::String(c.to_string()));
    }
    if d.is_string() {
        return Ok(Value::String(d.into_immutable_string().map(|s| s.to_string()).unwrap_or_default()));
    }
    if d.is_fnptr() {
        let Some(f) = d.try_cast::<FnPtr>() else { return Ok(Value::Null) };
        fns.push(f);
        return Ok(serde_json::json!({ "$fn": fns.len() - 1 }));
    }
    if let Some(a) = d.read_lock::<Array>() {
        return a
            .iter()
            .map(|x| to_json(x, fns, depth + 1, limits))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if let Some(m) = d.read_lock::<Map>() {
        let mut out = serde_json::Map::new();
        for (k, v) in m.iter() {
            out.insert(k.to_string(), to_json(v, fns, depth + 1, limits)?);
        }
        return Ok(Value::Object(out));
    }
    Err(ScriptError::Shape(format!("a {} is not part of a widget", d.type_name())))
}

/// How deeply values may nest: as deep as a description of elements nested `nesting` deep
/// (a map holding a list of maps), plus the handlers inside.
fn value_depth(limits: &Limits) -> usize {
    limits.nesting.saturating_mul(2).saturating_add(2)
}

/// What is left to hold: bytes, and how deep values may nest.
struct Budget {
    bytes: usize,
    depth: usize,
    /// Whether it was spent by nesting.
    too_deep: bool,
}

impl Budget {
    /// Takes what `d` holds (its slot and what it owns: string bytes, list and map entries,
    /// keys) from the budget; false once the budget is spent or `d` nests too deeply. Stops
    /// descending as soon as it is.
    fn take(&mut self, d: &Dynamic, depth: usize) -> bool {
        let slot = std::mem::size_of::<Dynamic>();
        if depth > self.depth {
            self.too_deep = true;
            return false;
        }
        if self.bytes < slot {
            return false;
        }
        self.bytes -= slot;
        if let Some(s) = d.read_lock::<rhai::ImmutableString>() {
            return self.spend(s.len());
        }
        if let Some(a) = d.read_lock::<Array>() {
            return a.iter().all(|x| self.take(x, depth + 1));
        }
        if let Some(m) = d.read_lock::<Map>() {
            return m.iter().all(|(k, v)| self.spend(k.len()) && self.take(v, depth + 1));
        }
        true
    }

    fn spend(&mut self, bytes: usize) -> bool {
        match self.bytes.checked_sub(bytes) {
            Some(left) => {
                self.bytes = left;
                true
            }
            None => false,
        }
    }
}

/// The bytes `d` holds, if within `limits`.
fn retained(d: &Dynamic, limits: &Limits) -> Option<usize> {
    let mut budget = Budget { bytes: limits.memory, depth: value_depth(limits), too_deep: false };
    budget.take(d, 0).then(|| limits.memory - budget.bytes)
}

impl RhaiWidget {
    /// Compiles `source` into a widget with `limits` and reads its manifest.
    pub fn load(source: &str, limits: Limits) -> Result<RhaiWidget, ScriptError> {
        let meter: Rc<Meter> = Rc::default();
        // No one value may be larger than what may be made between two measurements.
        let value_bytes = (limits.memory / MEASURE_EVERY as usize).max(1);
        RATE.calibrate_once("remedy-ui-script-rhai calibration", measure);
        let mut engine = Engine::new();
        engine
            .set_max_operations(limits.step_budget(&RATE))
            .set_max_call_levels(limits.call_depth)
            .set_max_expr_depths(EXPR_DEPTH, FN_EXPR_DEPTH)
            .set_max_string_size(value_bytes)
            .set_max_array_size(value_bytes / std::mem::size_of::<Dynamic>())
            .set_max_map_size(value_bytes / std::mem::size_of::<Dynamic>())
            .set_strict_variables(true)
            .set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
        for (symbol, _) in DENIED {
            engine.disable_symbol(*symbol);
        }
        let m2 = meter.clone();
        engine.on_progress(move |ops| m2.check(ops));
        let m3 = meter.clone();
        // The debugger interface is the one hook Rhai gives that sees a call's variables at
        // every step; it is marked volatile (it may change between Rhai versions), which the
        // pinned version makes safe.
        #[allow(deprecated)]
        engine.register_debugger(
            |_, debugger| debugger,
            move |ctx, _, _, _, _| m3.step(&ctx).map(|()| rhai::debugger::DebuggerCommand::StepInto),
        );
        engine.on_print(|s| remedy_ui_trace::tracing::debug!(target: "remedy_ui::script", "{s}"));
        engine.on_debug(
            |s, _, pos| remedy_ui_trace::tracing::debug!(target: "remedy_ui::script", "{pos:?} {s}"),
        );
        engine.register_static_module("ui", ui_module().into());
        let ast = engine.compile(source).map_err(|e| ScriptError::Invalid(e.to_string()))?;
        if !ast.iter_functions().any(|f| f.name == "render" && f.params.len() == 2) {
            return Err(ScriptError::Invalid("it has no render(settings, host) function".into()));
        }
        meter.start(&limits);
        let shown = engine.eval_ast_with_scope::<Dynamic>(&mut Scope::new(), &ast);
        meter.stop();
        let shown = shown.map_err(|e| match classify(&e) {
            ScriptError::Runtime(t) => ScriptError::Invalid(t),
            other => other,
        })?;
        if !shown.is_map() {
            return Err(ScriptError::Invalid(
                "its last expression must be its manifest, a map with its name and settings".into(),
            ));
        }
        let manifest = remedy_ui_script::manifest(&to_json(&shown, &mut Vec::new(), 0, &limits)?)?;
        Ok(RhaiWidget {
            engine,
            ast,
            manifest,
            state: Dynamic::from_map(Map::new()),
            last: None,
            fns: Vec::new(),
            meter,
            limits,
        })
    }

    /// Runs `f` against the time budget, then checks what the widget keeps.
    fn timed<R>(
        &mut self,
        f: impl FnOnce(&Engine, &AST, &mut Dynamic) -> Result<R, Box<EvalAltResult>>,
    ) -> Result<R, ScriptError> {
        // The budget follows the rate as it is measured.
        self.engine.set_max_operations(self.limits.step_budget(&RATE));
        self.meter.start(&self.limits);
        let r = f(&self.engine, &self.ast, &mut self.state);
        self.meter.stop();
        let r = r.map_err(|e| classify(&e))?;
        if retained(&self.state, &self.limits).is_none() {
            // What it kept is over the limit: it starts over, rather than keep it.
            self.state = Dynamic::from_map(Map::new());
            return Err(ScriptError::TooBig);
        }
        Ok(r)
    }
}

impl ScriptWidget for RhaiWidget {
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
        Language::RHAI
    }

    fn render(
        &mut self,
        settings: &serde_json::Map<String, Value>,
        host: &Value,
    ) -> Result<Node, ScriptError> {
        let values = Value::Object(remedy_ui_script::resolve_settings(&self.manifest.settings, settings));
        let (s, h) = (to_dynamic(&values), to_dynamic(host));
        let shown = self.timed(|engine, ast, state| {
            let options = CallFnOptions::new().eval_ast(false).bind_this_ptr(state);
            engine.call_fn_with_options::<Dynamic>(options, &mut Scope::new(), ast, "render", (s, h))
        })?;
        let mut fns = Vec::new();
        let json = to_json(&shown, &mut fns, 0, &self.limits)?;
        let described = remedy_ui_script::describe(&json, &self.limits)?;
        let node = described.node.clone();
        self.last = Some(described);
        self.fns = fns;
        Ok(node)
    }

    fn act(&mut self, action: Action) -> Result<(), ScriptError> {
        let gone = || ScriptError::Shape("that button is gone".into());
        let on = self.last.as_ref().ok_or_else(gone)?.handler(action)?;
        let f = match on {
            Value::String(name) => {
                FnPtr::new(name.as_str()).map_err(|e| ScriptError::Shape(e.to_string()))?
            }
            Value::Object(m) => {
                let i = m.get("$fn").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
                i.and_then(|i| self.fns.get(i)).cloned().ok_or_else(gone)?
            }
            _ => return Err(ScriptError::Shape("a button's handler must be a function or its name".into())),
        };
        self.timed(|engine, ast, state| f.call_as_method::<Dynamic>(engine, ast, state, ()).map(|_| ()))
    }

    fn memory_used(&self) -> usize {
        retained(&self.state, &self.limits).unwrap_or(self.limits.memory)
    }
}
