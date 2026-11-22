//! RemedyUI scripted widgets in WebAssembly: widgets compiled from Rust or any language that
//! targets WebAssembly, run by [wasmi](https://github.com/wasmi-labs/wasmi), a pure-Rust
//! interpreter (no JIT, no C), so they run on every platform RemedyUI does, browser pages
//! included.
//!
//! [`WasmEngine`] is a [`ScriptEngine`]: enable it in an app's or a board's `Engines` and
//! widgets whose package says `wasm` load through it.
//!
//! # The widget ABI
//!
//! A widget is a module that exports:
//!
//! | export | signature | what it does |
//! |---|---|---|
//! | `memory` | memory | its linear memory |
//! | `remedy_alloc` | `(len: i32) -> i32` | room for `len` bytes the host writes in (0 when it has none) |
//! | `remedy_describe` | `() -> i64` | its manifest as JSON (`remedy_ui_script::manifest`) |
//! | `remedy_render` | `(ptr: i32, len: i32) -> i64` | reads `{"settings": {..}, "host": ..}` at `ptr`, returns its elements as JSON (`remedy_ui_script::describe`) |
//! | `remedy_act` | `(ptr: i32, len: i32)` | reads the pressed button's `on` value as JSON at `ptr` and runs it |
//!
//! An `i64` result is a place in its memory: the address in the high 32 bits, the length in
//! the low 32. Text is UTF-8 JSON. The host writes inputs into memory the widget allocated
//! with `remedy_alloc`; the widget may free it (and its own outputs) on its next call.
//!
//! It may import, from module `remedy`, and nothing else:
//!
//! | import | signature | what it does |
//! |---|---|---|
//! | `log` | `(ptr: i32, len: i32)` | writes UTF-8 text to the engine's trace |
//! | `fail` | `(ptr: i32, len: i32)` | stops the call with that UTF-8 text as the reason |
//!
//! `crates/remedy-ui-script-wasm/examples/claps` is a widget written in Rust, with the
//! command that builds it.
//!
//! # The sandbox
//!
//! A module is validated before it runs. It gets no WASI and no host access beyond the two
//! imports above: a module that imports anything else (files, clocks, sockets, processes)
//! does not load ([`ScriptError::Denied`]), and neither does one with a start function
//! (widgets start in `remedy_describe`, under the limits). Each call runs against the
//! [`Limits`]:
//!
//! - fuel, wasmi's count of executed instructions ([`ScriptError::OutOfFuel`]), handed out
//!   in slices so the engine looks at the page-safe clock between them and stops a call past
//!   its time budget ([`ScriptError::TooSlow`]) even in a tight loop;
//! - the call depth and the interpreter's own stack ([`ScriptError::TooDeep`]);
//! - memory: linear memory and tables may not grow past `memory` ([`ScriptError::TooBig`]
//!   when that is why it stopped).
//!
//! Floating-point results are deterministic across machines (wasmi's deterministic mode),
//! there is no clock and no randomness, so the same calls describe the same widget. A
//! module stopped in the middle of a call may hold half-done work (an allocator mid-update,
//! a lock held), so after any stop the widget starts over from a fresh instance of its
//! module.

use remedy_ui_core::time::WorkClock;
use remedy_ui_script::{
    Action, Described, Language, Limits, Manifest, Node, ScriptEngine, ScriptError, ScriptWidget,
    SettingSpec, StepRate,
};
use serde_json::Value;
use wasmi::errors::LimiterError;
use wasmi::{
    Caller, Config, Engine, ExternType, Instance, Linker, Memory, Module, ResourceLimiter, Store, TrapCode,
    TypedFunc, TypedResumableCall, WasmParams, WasmResults,
};

/// wasmi fuel a millisecond assumed only in the moment before this device's rate is
/// measured ([`rate`]): low on purpose (an eighth of what a current desktop core measures),
/// so a fuel budget made then errs short rather than long.
pub const FALLBACK_FUEL_PER_MS: u64 = 100_000;

/// wasmi fuel a millisecond on this device, for the whole process.
static RATE: StepRate = StepRate::new(FALLBACK_FUEL_PER_MS);

/// How much wasmi fuel a millisecond buys on this device: what a time budget becomes when
/// [`Limits::operations`] is not given. Calibrated the first time a widget loads (a fixed
/// module run for a few milliseconds away from the UI thread), then kept current by every
/// call long enough to time well, so it follows the device when it slows down or speeds up.
/// [`FALLBACK_FUEL_PER_MS`] until the first measurement.
pub fn rate() -> &'static StepRate {
    &RATE
}

/// The module calibration runs, as text: calls, arithmetic, memory stores and a branch in a
/// loop that runs until its fuel is spent. [`CALIBRATION`] is it compiled (a test checks
/// they match).
pub const CALIBRATION_WAT: &str = r#"(module
  (memory (export "memory") 1)
  (data (i32.const 0) "{\"name\":\"calibration\"}")
  (func (export "remedy_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "remedy_describe") (result i64) (i64.const 22))
  (func $step (param i32) (result i32) (i32.add (local.get 0) (i32.const 1)))
  (func (export "remedy_render") (param i32 i32) (result i64) (local i32)
    (loop $l
      (local.set 2 (call $step (local.get 2)))
      (i32.store (i32.add (i32.const 2048) (i32.and (local.get 2) (i32.const 1020))) (local.get 2))
      (br_if $l (i32.ne (local.get 2) (i32.const -1))))
    (i64.const 0))
  (func (export "remedy_act") (param i32 i32)))"#;

/// The calibration module ([`CALIBRATION_WAT`], compiled).
pub const CALIBRATION: &[u8] = include_bytes!("calibration.wasm");

/// Runs the calibration module for a few milliseconds on this thread: the fuel it took and
/// how long it took, or `None` when it could not run.
pub fn measure() -> Option<(u64, std::time::Duration)> {
    remedy_ui_script::calibrate_with(FALLBACK_FUEL_PER_MS, |steps| {
        let limits = Limits::default().operations(Some(steps)).time(std::time::Duration::from_secs(10));
        let mut w = WasmWidget::load(CALIBRATION, limits).ok()?;
        let start = WorkClock::start();
        let r = w.render(&serde_json::Map::new(), &Value::Null);
        (r == Err(ScriptError::OutOfFuel)).then(|| start.work())
    })
}

/// Measures this device's rate now, on this thread, and records it: the fuel a millisecond
/// it measured, or `None` when the calibration could not run.
pub fn calibrate() -> Option<u64> {
    let (steps, took) = measure()?;
    RATE.record(steps, took).then(|| RATE.per_ms())
}

/// How many slices the fuel of a call's time budget is handed out in: the engine looks at the
/// clock between them, so a call overruns its time budget by about a slice.
const SLICES: u64 = 64;

/// The module widgets import from.
const HOST_MODULE: &str = "remedy";

/// The engine for WebAssembly widgets.
#[derive(Clone, Copy, Debug, Default)]
pub struct WasmEngine;

impl WasmEngine {
    /// The engine.
    pub fn new() -> WasmEngine {
        WasmEngine
    }

    /// Loads the widget in `module` with `limits`.
    pub fn load_module(&self, module: &[u8], limits: Limits) -> Result<WasmWidget, ScriptError> {
        WasmWidget::load(module, limits)
    }
}

impl ScriptEngine for WasmEngine {
    fn language(&self) -> Language {
        Language::WASM
    }

    fn load(&self, code: &[u8], limits: Limits) -> Result<Box<dyn ScriptWidget>, ScriptError> {
        Ok(Box::new(WasmWidget::load(code, limits)?))
    }
}

/// What the store holds for the host: the limits it enforces and why a call stopped.
struct Host {
    limiter: Limiter,
    /// The reason a widget gave through `fail`.
    failed: Option<String>,
    /// The instance's memory, where host functions read what they are handed. (Kept here
    /// rather than looked up through the caller's exports, which wasmi 2.0.1 got wrong in
    /// unoptimized builds.)
    memory: Option<Memory>,
}

/// Keeps linear memory and tables within the widget's memory limit, and remembers when it
/// refused, so a call that then fails is reported as too big.
struct Limiter {
    memory: usize,
    refused: bool,
}

/// The bytes one table element holds in the interpreter.
const TABLE_ELEMENT: usize = 16;

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        let allow = desired <= self.memory && maximum.is_none_or(|m| desired <= m);
        self.refused |= !allow;
        Ok(allow)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        let allow =
            desired.saturating_mul(TABLE_ELEMENT) <= self.memory && maximum.is_none_or(|m| desired <= m);
        self.refused |= !allow;
        Ok(allow)
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        // A module may define tables, each bounded by the memory limit as it grows.
        usize::MAX
    }

    fn memories(&self) -> usize {
        1
    }
}

/// The exports of one instance.
struct Exports {
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    render: TypedFunc<(i32, i32), i64>,
    act: TypedFunc<(i32, i32), ()>,
}

/// One WebAssembly widget: its module, its instance, its last description.
pub struct WasmWidget {
    module: Module,
    linker: Linker<Host>,
    store: Store<Host>,
    exports: Exports,
    manifest: Manifest,
    last: Option<Described>,
    /// Set after a call was stopped: the next call starts from a fresh instance.
    stale: bool,
    limits: Limits,
}

impl std::fmt::Debug for WasmWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmWidget").field("name", &self.manifest.name).field("limits", &self.limits).finish()
    }
}

fn invalid(e: impl std::fmt::Display) -> ScriptError {
    ScriptError::Invalid(e.to_string())
}

/// What a module importing `module::name` was reaching for.
fn denied_import(module: &str, name: &str) -> ScriptError {
    let what = if module.starts_with("wasi") {
        "WASI (files, clocks, randomness, processes)"
    } else if module == HOST_MODULE {
        "a host function RemedyUI does not give widgets"
    } else {
        "something outside the widget's sandbox"
    };
    ScriptError::Denied(format!(
        "it imports {module}::{name}, {what}; widgets may import only {HOST_MODULE}::log and {HOST_MODULE}::fail"
    ))
}

/// Reads `len` bytes at `ptr` of `memory`, as UTF-8.
fn read_text(memory: &[u8], ptr: i32, len: i32) -> Option<String> {
    let start = usize::try_from(ptr as u32).ok()?;
    let end = start.checked_add(usize::try_from(len as u32).ok()?)?;
    memory.get(start..end).map(|b| String::from_utf8_lossy(b).into_owned())
}

fn engine(limits: &Limits) -> Engine {
    let mut config = Config::default();
    config
        .consume_fuel(true)
        // Compile the whole module when it loads: translated lazily, a function's first use
        // would charge its translation to that call's fuel as one indivisible step, and a
        // call could fail on compiling rather than on what the widget does.
        .compilation_mode(wasmi::CompilationMode::Eager)
        .allow_start_fn(false)
        .set_max_recursion_depth(limits.call_depth.max(1))
        .set_max_cached_stacks(1)
        .enforced_limits(wasmi::EnforcedLimits::strict());
    // The value stack: within the call's stack budget, and at least what wasmi starts with.
    config.set_max_stack_height(limits.stack.max(1000));
    config.set_min_stack_height(1000);
    Engine::new(&config)
}

fn linker(engine: &Engine) -> Result<Linker<Host>, ScriptError> {
    let mut linker = Linker::<Host>::new(engine);
    linker
        .func_wrap(HOST_MODULE, "log", |caller: Caller<'_, Host>, ptr: i32, len: i32| {
            let Some(memory) = caller.data().memory else { return };
            if let Some(text) = read_text(memory.data(&caller), ptr, len) {
                remedy_ui_core::trace::tracing::debug!(target: "remedy_ui::script", "{text}");
            }
        })
        .map_err(invalid)?;
    linker
        .func_wrap(
            HOST_MODULE,
            "fail",
            |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
                let text = caller
                    .data()
                    .memory
                    .and_then(|m| read_text(m.data(&caller), ptr, len))
                    .unwrap_or_else(|| "it failed, at a place outside its memory".to_owned());
                caller.data_mut().failed = Some(text.clone());
                Err(wasmi::Error::new(text))
            },
        )
        .map_err(invalid)?;
    Ok(linker)
}

fn store(engine: &Engine, limits: &Limits) -> Store<Host> {
    let mut store = Store::new(
        engine,
        Host { limiter: Limiter { memory: limits.memory, refused: false }, failed: None, memory: None },
    );
    store.limiter(|host| &mut host.limiter);
    store
}

fn instantiate(
    linker: &Linker<Host>,
    store: &mut Store<Host>,
    module: &Module,
) -> Result<(Instance, Exports), ScriptError> {
    let instance = linker
        .instantiate_and_start(&mut *store, module)
        .map_err(|e| if store.data().limiter.refused { ScriptError::TooBig } else { invalid(e) })?;
    let missing = |what: &str| ScriptError::Invalid(format!("it does not export {what}"));
    let memory = instance.get_memory(&*store, "memory").ok_or_else(|| missing("its memory"))?;
    store.data_mut().memory = Some(memory);
    let alloc = instance
        .get_typed_func(&*store, "remedy_alloc")
        .map_err(|_| missing("remedy_alloc(len: i32) -> i32"))?;
    let render = instance
        .get_typed_func(&*store, "remedy_render")
        .map_err(|_| missing("remedy_render(ptr: i32, len: i32) -> i64"))?;
    let act = instance
        .get_typed_func(&*store, "remedy_act")
        .map_err(|_| missing("remedy_act(ptr: i32, len: i32)"))?;
    Ok((instance, Exports { memory, alloc, render, act }))
}

/// Runs `f` with `params` in fuel slices against `limits`, and records the fuel it took over
/// its time as a measurement of the rate.
fn run<P: WasmParams, R: WasmResults>(
    store: &mut Store<Host>,
    limits: &Limits,
    f: &TypedFunc<P, R>,
    params: P,
) -> Result<R, ScriptError> {
    // Processor time on this thread: what the system gives other work does not count.
    let began = WorkClock::start();
    let rate = RATE.per_ms();
    let total = limits.step_budget(&RATE);
    // Slices are a part of the time budget at the measured rate, whatever the fuel budget.
    let slice = (*limits).operations(None).operations_at(rate) / SLICES;
    let slice = slice.max(1);
    let mut left = total;
    store.data_mut().failed = None;
    store.data_mut().limiter.refused = false;
    let mut give = slice.min(left);
    left -= give;
    let mut given = give;
    store.set_fuel(give).map_err(invalid)?;
    let mut call = f.call_resumable(&mut *store, params);
    let r = loop {
        match call {
            Ok(TypedResumableCall::Finished(r)) => break Ok(r),
            Ok(TypedResumableCall::HostTrap(t)) => {
                let why = store.data_mut().failed.take().unwrap_or_else(|| t.host_error().to_string());
                break Err(ScriptError::Runtime(why));
            }
            Ok(TypedResumableCall::OutOfFuel(t)) => {
                if left == 0 {
                    break Err(ScriptError::OutOfFuel);
                }
                // Fuel used so far: what the call really ran (see `StepRate::ran_for`).
                let used = given.saturating_sub(store.get_fuel().unwrap_or(0));
                if began.over(limits.time) && RATE.ran_for(used, limits.time) {
                    break Err(ScriptError::TooSlow);
                }
                give = slice.min(left);
                left -= give;
                given = given.saturating_add(give);
                let have = store.get_fuel().map_err(invalid)?;
                store.set_fuel(have.saturating_add(give)).map_err(invalid)?;
                call = t.resume(&mut *store);
            }
            Err(e) => break Err(classify(store, &e)),
        }
    };
    let unused = store.get_fuel().unwrap_or(0);
    let _ = RATE.record(given.saturating_sub(unused), began.work());
    r
}

/// What a trap means for a widget.
fn classify(store: &mut Store<Host>, e: &wasmi::Error) -> ScriptError {
    if let Some(why) = store.data_mut().failed.take() {
        return ScriptError::Runtime(why);
    }
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => ScriptError::OutOfFuel,
        Some(TrapCode::StackOverflow) => ScriptError::TooDeep,
        // An allocator that was refused memory gives up: that is the reason.
        _ if store.data().limiter.refused => ScriptError::TooBig,
        Some(TrapCode::GrowthOperationLimited) => ScriptError::TooBig,
        _ => ScriptError::Runtime(e.to_string()),
    }
}

/// Splits a place returned as `i64` into its address and length.
fn place(v: i64) -> (i32, i32) {
    let v = v as u64;
    ((v >> 32) as u32 as i32, v as u32 as i32)
}

impl WasmWidget {
    /// Validates and instantiates `module` with `limits`, and reads its manifest.
    pub fn load(module: &[u8], limits: Limits) -> Result<WasmWidget, ScriptError> {
        if !module.starts_with(b"\0asm") {
            return Err(invalid("a WebAssembly widget is a module (it starts with \\0asm), and this is not"));
        }
        RATE.calibrate_once("remedy-ui-script-wasm calibration", measure);
        let engine = engine(&limits);
        let module = Module::new(&engine, module).map_err(invalid)?;
        for import in module.imports() {
            let allowed = import.module() == HOST_MODULE
                && matches!(import.name(), "log" | "fail")
                && matches!(import.ty(), ExternType::Func(_));
            if !allowed {
                return Err(denied_import(import.module(), import.name()));
            }
        }
        let linker = linker(&engine)?;
        let mut store = store(&engine, &limits);
        let (instance, exports) = instantiate(&linker, &mut store, &module)?;
        let describe: TypedFunc<(), i64> = instance
            .get_typed_func(&store, "remedy_describe")
            .map_err(|_| invalid("it does not export remedy_describe() -> i64"))?;
        let at = run(&mut store, &limits, &describe, ()).map_err(|e| match e {
            ScriptError::Runtime(t) => ScriptError::Invalid(t),
            other => other,
        })?;
        let json = Self::output(&store, &exports.memory, at).map_err(|e| match e {
            ScriptError::Shape(t) => ScriptError::Invalid(t),
            other => other,
        })?;
        let manifest = remedy_ui_script::manifest(&json)?;
        Ok(WasmWidget { module, linker, store, exports, manifest, last: None, stale: false, limits })
    }

    /// Reads the JSON at the place `at` in memory.
    fn output(store: &Store<Host>, memory: &Memory, at: i64) -> Result<Value, ScriptError> {
        let (ptr, len) = place(at);
        let data = memory.data(store);
        let start = usize::try_from(ptr as u32).unwrap_or(usize::MAX);
        let end = start.saturating_add(usize::try_from(len as u32).unwrap_or(usize::MAX));
        let bytes = data
            .get(start..end)
            .ok_or_else(|| ScriptError::Shape("it returned a place outside its memory".into()))?;
        serde_json::from_slice(bytes)
            .map_err(|e| ScriptError::Shape(format!("it returned something that is not JSON: {e}")))
    }

    /// Writes `input` into memory the widget allocates for it.
    fn input(&mut self, input: &[u8]) -> Result<(i32, i32), ScriptError> {
        let len = i32::try_from(input.len()).map_err(|_| ScriptError::TooBig)?;
        let ptr = run(&mut self.store, &self.limits, &self.exports.alloc, len)?;
        if ptr == 0 && len > 0 {
            return Err(ScriptError::TooBig);
        }
        self.exports
            .memory
            .write(&mut self.store, ptr as u32 as usize, input)
            .map_err(|_| ScriptError::Shape("remedy_alloc gave a place outside its memory".into()))?;
        Ok((ptr, len))
    }

    /// Runs `f`; after a stop, the widget starts over on its next call.
    fn guarded<R>(
        &mut self,
        f: impl FnOnce(&mut WasmWidget) -> Result<R, ScriptError>,
    ) -> Result<R, ScriptError> {
        if self.stale {
            let mut store = store(self.store.engine(), &self.limits);
            let (_, exports) = instantiate(&self.linker, &mut store, &self.module)?;
            (self.store, self.exports, self.last, self.stale) = (store, exports, None, false);
        }
        let r = f(self);
        self.stale = r.is_err();
        r
    }
}

impl ScriptWidget for WasmWidget {
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
        Language::WASM
    }

    fn render(
        &mut self,
        settings: &serde_json::Map<String, Value>,
        host: &Value,
    ) -> Result<Node, ScriptError> {
        let settings = Value::Object(remedy_ui_script::resolve_settings(&self.manifest.settings, settings));
        let input = serde_json::json!({ "settings": settings, "host": host }).to_string();
        self.guarded(|w| {
            let (ptr, len) = w.input(input.as_bytes())?;
            let at = run(&mut w.store, &w.limits, &w.exports.render, (ptr, len))?;
            let json = Self::output(&w.store, &w.exports.memory, at)?;
            let described = remedy_ui_script::describe(&json, &w.limits)?;
            let node = described.node.clone();
            w.last = Some(described);
            Ok(node)
        })
    }

    fn act(&mut self, action: Action) -> Result<(), ScriptError> {
        let gone = || ScriptError::Shape("that button is gone".into());
        let on = self.last.as_ref().ok_or_else(gone)?.handler(action)?.to_string();
        self.guarded(|w| {
            let (ptr, len) = w.input(on.as_bytes())?;
            run(&mut w.store, &w.limits, &w.exports.act, (ptr, len))
        })
    }

    fn memory_used(&self) -> usize {
        self.exports.memory.data_size(&self.store)
    }
}
