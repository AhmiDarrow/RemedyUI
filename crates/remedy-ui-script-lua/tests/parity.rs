//! The same cases the handwritten engine has always covered: the example widget, settings
//! and buttons, closures in place of curried function pointers, a hostile script that cannot
//! reach files, the network, processes or other code, runaway loops and recursion, the memory
//! limit, a widget that keeps working after a stop, invalid widgets said plainly, determinism,
//! the measured step rate, a busy machine, and expression depth (RemedyLua's parser does not
//! recurse, so [`Limits::expr_depth`] is not a stop).

#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests fail by panicking and index what they just checked"
)]

use std::time::{Duration, Instant};

use remedy_ui_script::{
    Action, Engines, Language, Limits, Node, Package, ScriptError, ScriptWidget, SettingKind, TextStyle,
};
use remedy_ui_script_lua::{LuaEngine, LuaWidget};
use serde_json::{Map, Value};

trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

const CLAPS: &str = include_str!("../examples/claps.lua");
const POLL: &str = include_str!("../examples/poll.lua");

fn host() -> Value {
    serde_json::json!({ "name": "Sam", "house": "the Den" })
}

fn load(src: &str, limits: Limits) -> Result<LuaWidget, ScriptError> {
    LuaEngine::new().load_source(src, limits)
}

fn render(w: &mut dyn ScriptWidget) -> Result<Node, ScriptError> {
    w.render(&Map::new(), &host())
}

/// A widget whose render runs `body` and then shows "ok".
fn widget_running(body: &str) -> String {
    format!("return {{ name = 'probe', render = function() {body}\nreturn ui.text('ok') end }}")
}

fn column(n: &Node) -> &[Node] {
    match n {
        Node::Column { children, .. } => children,
        other => unreachable!("{other:?}"),
    }
}

#[test]
fn the_example_renders_takes_settings_and_handles_its_buttons() {
    let engines = Engines::new().with(std::rc::Rc::new(LuaEngine::new()));
    let mut w = engines.load(&Package::lua(CLAPS), Limits::default()).must("claps loads");
    assert_eq!(w.name(), "Claps");
    assert_eq!(w.settings().len(), 2);
    assert_eq!(
        w.settings().get(1).map(|s| &s.kind),
        Some(&SettingKind::Number { min: Some(1.0), max: Some(100.0) })
    );
    let mut settings = Map::new();
    settings.insert("title".into(), "Party".into());
    let n = w.render(&settings, &host()).must("renders");
    let children = column(&n);
    assert_eq!(children.first(), Some(&Node::Text { text: "Party".into(), style: TextStyle::Heading(2) }));
    assert_eq!(
        children.get(1),
        Some(&Node::Text { text: "0 claps from the Den".into(), style: TextStyle::Body })
    );
    let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!("{children:?}") };
    w.act(*action).must("clap");
    let n = w.render(&settings, &host()).must("renders again");
    let children = column(&n);
    let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!("{children:?}") };
    w.act(*action).must("clap again");
    let n = w.render(&settings, &host()).must("renders a third time");
    let children = column(&n);
    assert_eq!(
        children.get(1),
        Some(&Node::Text { text: "2 claps from the Den".into(), style: TextStyle::Body })
    );
    assert_eq!(children.get(2), Some(&Node::Progress { label: "Claps".into(), value: 0.2 }));
    assert_eq!(children.get(4), Some(&Node::Text { text: "Hello, Sam".into(), style: TextStyle::Muted }));
    assert!(w.memory_used() > 0);
    assert!(matches!(w.act(Action(99)), Err(ScriptError::Shape(_))));
}

#[test]
fn handlers_can_be_closures_in_place_of_curried_functions() {
    let mut w = load(POLL, Limits::default()).must("poll loads");
    let n = render(&mut w).must("renders");
    let children = column(&n);
    let Some(Node::Row { children: buttons, .. }) = children.get(1) else { unreachable!("{children:?}") };
    let Some(Node::Button { action: tea, .. }) = buttons.get(1) else { unreachable!("{buttons:?}") };
    w.act(*tea).must("vote tea");
    let n = render(&mut w).must("renders");
    let children = column(&n);
    assert!(children.contains(&Node::Text { text: "Tea: 1".into(), style: TextStyle::Body }), "{children:?}");
    let Some(Node::Button { action: reset, .. }) = children.last() else { unreachable!("{children:?}") };
    w.act(*reset).must("reset");
    let n = render(&mut w).must("renders");
    let children = column(&n);
    assert!(children.contains(&Node::Text { text: "Tea: 0".into(), style: TextStyle::Body }), "{children:?}");
}

#[test]
fn a_hostile_script_cannot_reach_files_the_network_processes_or_other_code() {
    for probe in [
        "open_file('x')",
        "read_file('/etc/passwd')",
        "http_get('https://example.com')",
        "system('rm -rf /')",
        "exec('x')",
        "env('HOME')",
        "eval('1 + 1')",
        "import 'socket'",
        "timestamp()",
        "rand()",
        "io.open('/etc/passwd')",
        "os.execute('x')",
        "require('socket')",
        "load('return 1')",
        "debug.getinfo(1)",
    ] {
        let r = load(&widget_running(probe), Limits::default()).and_then(|mut w| render(&mut w));
        assert!(
            matches!(r, Err(ScriptError::Denied(_) | ScriptError::Invalid(_))),
            "{probe} must be refused, got {r:?}"
        );
    }
    let r = load(&widget_running("ui.text = 5"), Limits::default()).and_then(|mut w| render(&mut w));
    assert!(matches!(r, Err(ScriptError::Denied(_))), "{r:?}");
    let r = load(&widget_running("math.random()"), Limits::default()).and_then(|mut w| render(&mut w));
    assert!(matches!(r, Err(ScriptError::Denied(_))), "{r:?}");
}

#[test]
fn a_script_that_never_stops_is_stopped() {
    let limits = Limits::default().operations(Some(100_000)).time(Duration::from_secs(60));
    let mut w = load(&widget_running("while true do end"), limits).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    let limits = Limits::default().operations(Some(u64::MAX / 2)).time(Duration::from_millis(20));
    let mut w = load(&widget_running("while true do end"), limits).must("loads");
    let start = Instant::now();
    assert_eq!(render(&mut w), Err(ScriptError::TooSlow));
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
    let r = load(
        "function render() return ui.text(1) end\nwhile true do end\nreturn { name = 'x', render = render }",
        limits,
    );
    assert_eq!(r.err(), Some(ScriptError::TooSlow));
}

#[test]
fn runaway_recursion_is_stopped() {
    // `+ 0` keeps this from being a tail call, so the frame count grows (Lua reuses the
    // frame of `return down(n + 1)`).
    let src = "local function down(n) return down(n + 1) + 0 end\n\
               return { name = 'deep', render = function() down(0) end }";
    let mut w = load(src, Limits::default()).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooDeep));
    // Lua-to-Lua calls use no native stack, so a depth far past a widget's needs is stopped
    // by fuel, and the process is not overflowed.
    let mut w = load(
        src,
        Limits::default().call_depth(1_000_000).operations(Some(200_000)).time(Duration::from_secs(60)),
    )
    .must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    let src = "local function sum(n) if n == 0 then return 0 else return n + sum(n - 1) end end\n\
               return { name = 'sum', render = function() return ui.text(tostring(sum(20))) end }";
    let mut w = load(src, Limits::default()).must("loads");
    assert_eq!(render(&mut w), Ok(Node::Text { text: "210".into(), style: TextStyle::Body }));
}

#[test]
fn a_script_that_eats_memory_is_stopped_at_its_limit() {
    let limits =
        Limits::default().memory(4 * 1024 * 1024).time(Duration::from_secs(10)).operations(Some(50_000_000));
    for hog in [
        "local s = 'x' while true do s = s .. s end",
        "local t = {} local s = 'xxxxxxxxxxxxxxxx' while true do s = s .. s t[#t + 1] = s end",
        "string.rep('x', 1000000000)",
    ] {
        let mut w = load(&widget_running(hog), limits).must("loads");
        assert_eq!(render(&mut w), Err(ScriptError::TooBig), "{hog}");
    }
    // RemedyLua's state (libraries, the `ui` table, the dispatcher) is larger than 64 KiB,
    // which was enough for the Rhai engine. The cap sits just above an idle widget.
    let idle = load("return { name = 'idle', render = function() return ui.text('ok') end }", limits)
        .must("an idle widget loads");
    let small = limits.memory(idle.memory_used().saturating_add(128 * 1024));
    drop(idle);
    let hog = "local m = {} local i = 0 while true do m['k' .. i] = i i = i + 1 end";
    let mut w = load(&widget_running(hog), small).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooBig), "{hog}");
    let hog = "local a = {} while true do a = { a } end";
    let mut w = load(&widget_running(hog), limits).must("loads");
    assert!(render(&mut w).is_err(), "{hog}");
    let src = "function grow()\n\
                 local s = 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'\n\
                 while #s < 1000000 do s = s .. s end\n\
                 this.keep = this.keep or {}\n\
                 this.keep[#this.keep + 1] = s\n\
               end\n\
               return { name = 'keeper', render = function() return ui.button('Grow', 'grow') end }";
    let mut w = load(src, limits).must("loads");
    let mut stopped = false;
    for _ in 0..16 {
        let Ok(Node::Button { action, .. }) = render(&mut w) else { unreachable!("a button") };
        if w.act(action) == Err(ScriptError::TooBig) {
            stopped = true;
            break;
        }
        assert!(w.memory_used() <= limits.memory);
    }
    assert!(stopped, "the state outgrew its limit and was stopped");
    assert!(w.memory_used() <= limits.memory);
    assert!(render(&mut w).is_ok());
}

#[test]
fn a_widget_keeps_working_after_a_stop() {
    let src = "function spin() this.spin = true end\n\
               return { name = 'once', render = function()\n\
                 if this.spin then while true do end end\n\
                 return ui.button('Spin', 'spin')\n\
               end }";
    let limits = Limits::default().operations(Some(200_000)).time(Duration::from_secs(60));
    let mut w = load(src, limits).must("loads");
    let Ok(Node::Button { action, .. }) = render(&mut w) else { unreachable!("a button") };
    w.act(action).must("spin is set");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel), "stopped again, not stuck");
}

#[test]
fn what_is_not_a_widget_is_said_plainly() {
    for bad in [
        "return 5",
        "function render(",
        "return { name = 'x' }",
        "return { render = function() return 1 end } return 5",
    ] {
        let r = load(bad, Limits::default());
        assert!(matches!(r, Err(ScriptError::Invalid(_))), "{bad}: {r:?}");
    }
    let missing = load("return { name = 'x' }", Limits::default());
    assert!(
        matches!(missing, Err(ScriptError::Invalid(ref m)) if m.contains("shows nothing")),
        "{missing:?}"
    );
    let r = LuaEngine::new()
        .load_source("return { name = 5, render = function() return ui.text('x') end }", Limits::default());
    assert!(matches!(r, Err(ScriptError::Invalid(ref m)) if m.contains("name")), "{r:?}");
    let deep = "return { name = 'd', render = function()\n\
                  local n = ui.text('x')\n\
                  for _ = 1, 200 do n = ui.column(n) end\n\
                  return n\n\
                end }";
    let mut w = load(deep, Limits::default().time(Duration::from_secs(60))).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
    let odd = "return { name = 'o', render = function() return ui.node('iframe') end }";
    let mut w = load(odd, Limits::default()).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
    let engines = Engines::new().with(std::rc::Rc::new(LuaEngine::new()));
    let r = engines.load(&Package::new(Language::LUA, &[0xff, 0xfe][..]), Limits::default());
    assert!(matches!(r, Err(ScriptError::Invalid(_))));
}

#[test]
fn the_same_calls_describe_the_same_widget() {
    let mut a = load(CLAPS, Limits::default()).must("loads");
    let mut b = load(CLAPS, Limits::default()).must("loads");
    assert_eq!(render(&mut a), render(&mut b));
}

#[test]
fn the_step_rate_is_measured_on_this_device() {
    use remedy_ui_script::StepRate;
    let Some(per_ms) = remedy_ui_script_lua::calibrate() else { unreachable!("calibration runs") };
    assert!(remedy_ui_script_lua::rate().is_calibrated());
    assert!((100..100_000_000).contains(&per_ms), "{per_ms} steps a millisecond");
    let Some((steps, took)) = remedy_ui_script_lua::measure() else { unreachable!("calibration runs") };
    let (here, slower) = (StepRate::new(1), StepRate::new(1));
    assert!(here.record(steps, took) && slower.record(steps, took * 2));
    let limits = Limits::default();
    let (a, b) = (limits.operations_at(here.per_ms()), limits.operations_at(slower.per_ms()));
    assert!(b < a && (a as f64 / b as f64 - 2.0).abs() < 0.01, "{a} {b}");
    let mut w = load(&widget_running("while true do end"), limits).must("loads");
    let start = Instant::now();
    let r = render(&mut w);
    assert!(matches!(r, Err(ScriptError::OutOfFuel | ScriptError::TooSlow)), "{r:?}");
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
}

#[test]
fn a_busy_machine_does_not_stop_fair_work() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let spinners: Vec<_> = (0..cores * 2)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 1u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                }
                std::hint::black_box(x);
            })
        })
        .collect();
    let mut failures = Vec::new();
    for _ in 0..200 {
        let mut w = match load(POLL, Limits::default()) {
            Ok(w) => w,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        if let Err(e) = render(&mut w) {
            failures.push(e);
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for s in spinners {
        let _ = s.join();
    }
    assert!(failures.is_empty(), "fair calls were stopped on a busy machine: {failures:?}");
}

#[test]
fn deep_expressions_are_bounded_by_memory_not_a_parser_stack() {
    let (open, close) = ("(".repeat(80), ")".repeat(80));
    let deep = format!(
        "local x = {open}1{close}\nreturn {{ name = 'deep', render = function() return ui.text(tostring(x)) end }}"
    );
    assert!(load(&deep, Limits::default()).is_ok(), "80 nested parens still parse");
    let again = load(&deep, Limits::default().expr_depth(1, 1));
    assert!(again.is_ok(), "expr_depth is ignored: {:?}", again.err());
}

#[test]
fn a_typed_dialect_marker_selects_remedylua() {
    let plain = "local x: number = 1\nreturn { name = 't', render = function() return ui.text('ok') end }";
    assert!(matches!(load(plain, Limits::default()), Err(ScriptError::Invalid(_))));
    let typed = format!("-- rlua\n{plain}");
    let mut w = load(&typed, Limits::default()).must("rlua loads");
    assert!(render(&mut w).is_ok());
    let bad = "-- rlua\nlocal x: number = 'no'\nreturn { name = 't', render = function() return ui.text('ok') end }";
    assert!(matches!(load(bad, Limits::default()), Err(ScriptError::Invalid(_))));
}
