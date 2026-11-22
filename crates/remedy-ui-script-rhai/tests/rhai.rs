//! Rhai widgets: the example renders, takes settings and reacts to its buttons; hostile
//! scripts (endless loops, runaway recursion, huge allocations, reaching for files, the
//! network, processes or other code) are each stopped with a clear error, and the widget
//! keeps working after a stop.

use std::time::Duration;

use remedy_ui_core::time::Instant;
use remedy_ui_script::{
    Action, Engines, Limits, Node, Package, ScriptError, ScriptWidget, SettingKind, TextStyle,
};
use remedy_ui_script_rhai::{RhaiEngine, RhaiWidget};
use serde_json::{Map, Value, json};

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

const CLAPS: &str = include_str!("../examples/claps.rhai");
const POLL: &str = include_str!("../examples/poll.rhai");

fn host() -> Value {
    json!({ "name": "Sam", "house": "the Den" })
}

fn load(src: &str, limits: Limits) -> Result<RhaiWidget, ScriptError> {
    RhaiEngine::new().load_source(src, limits)
}

fn render(w: &mut dyn ScriptWidget) -> Result<Node, ScriptError> {
    w.render(&Map::new(), &host())
}

/// A widget whose render runs `body` and then shows "ok".
fn widget_running(body: &str) -> String {
    format!("fn render(settings, host) {{ {body}; ui::text(\"ok\") }} #{{ name: \"probe\" }}")
}

#[test]
fn the_example_renders_takes_settings_and_handles_its_buttons() {
    let engines = Engines::new().with(std::rc::Rc::new(RhaiEngine::new()));
    let mut w = engines.load(&Package::rhai(CLAPS), Limits::default()).must("claps loads");
    assert_eq!(w.name(), "Claps");
    assert_eq!(w.settings().len(), 2);
    assert_eq!(
        w.settings().get(1).map(|s| &s.kind),
        Some(&SettingKind::Number { min: Some(1.0), max: Some(100.0) })
    );
    let mut settings = Map::new();
    settings.insert("title".into(), "Party".into());
    let n = w.render(&settings, &host()).must("renders");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    assert_eq!(children.first(), Some(&Node::Text { text: "Party".into(), style: TextStyle::Heading(2) }));
    assert_eq!(
        children.get(1),
        Some(&Node::Text { text: "0 claps from the Den".into(), style: TextStyle::Body })
    );
    let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!("{children:?}") };
    w.act(*action).must("clap");
    let n = w.render(&settings, &host()).must("renders again");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!("{children:?}") };
    w.act(*action).must("clap again");
    let n = w.render(&settings, &host()).must("renders a third time");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    assert_eq!(
        children.get(1),
        Some(&Node::Text { text: "2 claps from the Den".into(), style: TextStyle::Body })
    );
    assert_eq!(children.get(2), Some(&Node::Progress { label: "Claps".into(), value: 0.2 }));
    assert_eq!(children.get(4), Some(&Node::Text { text: "Hello, Sam".into(), style: TextStyle::Muted }));
    assert!(w.memory_used() > 0);
    // A button from a description that is gone runs nothing.
    assert!(matches!(w.act(Action(99)), Err(ScriptError::Shape(_))));
}

#[test]
fn handlers_can_be_function_pointers_with_arguments_and_closures() {
    let mut w = load(POLL, Limits::default()).must("poll loads");
    let n = render(&mut w).must("renders");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    let Some(Node::Row { children: buttons, .. }) = children.get(1) else { unreachable!("{children:?}") };
    let Some(Node::Button { action: tea, .. }) = buttons.get(1) else { unreachable!("{buttons:?}") };
    w.act(*tea).must("vote tea");
    let n = render(&mut w).must("renders");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    assert!(children.contains(&Node::Text { text: "Tea: 1".into(), style: TextStyle::Body }), "{children:?}");
    let Some(Node::Button { action: reset, .. }) = children.last() else { unreachable!("{children:?}") };
    w.act(*reset).must("reset");
    let n = render(&mut w).must("renders");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    assert!(children.contains(&Node::Text { text: "Tea: 0".into(), style: TextStyle::Body }), "{children:?}");
}

#[test]
fn a_hostile_script_cannot_reach_files_the_network_processes_or_other_code() {
    for probe in [
        "open_file(\"x\")",
        "read_file(\"/etc/passwd\")",
        "http_get(\"https://example.com\")",
        "system(\"rm -rf /\")",
        "exec(\"x\")",
        "env(\"HOME\")",
        "eval(\"1 + 1\")",
        "import \"socket\" as s",
        "timestamp()",
        "rand()",
    ] {
        let r = load(&widget_running(probe), Limits::default()).and_then(|mut w| render(&mut w));
        assert!(
            matches!(r, Err(ScriptError::Denied(_) | ScriptError::Invalid(_))),
            "{probe} must be refused, got {r:?}"
        );
    }
    // Replacing the ui functions is not possible either.
    let r = load(&widget_running("ui::text = 5"), Limits::default()).and_then(|mut w| render(&mut w));
    assert!(r.is_err(), "{r:?}");
}

#[test]
fn a_script_that_never_stops_is_stopped() {
    // By its operation budget, deterministically.
    let limits = Limits::default().operations(Some(100_000)).time(Duration::from_secs(60));
    let mut w = load(&widget_running("loop {}"), limits).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    // By its time budget, when the operations would outlast it.
    let limits = Limits::default().operations(Some(u64::MAX / 2)).time(Duration::from_millis(20));
    let mut w = load(&widget_running("loop {}"), limits).must("loads");
    let start = Instant::now();
    assert_eq!(render(&mut w), Err(ScriptError::TooSlow));
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
    // Loading code that spins at the top level is stopped too.
    let r = load("fn render(s, h) { ui::text(1) } loop {}", limits);
    assert_eq!(r.err(), Some(ScriptError::TooSlow));
}

#[test]
fn runaway_recursion_is_stopped() {
    let src = "fn down(n) { down(n + 1) } fn render(s, h) { down(0) } #{ name: \"deep\" }";
    let mut w = load(src, Limits::default()).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooDeep));
    // With calls allowed far deeper than the stack holds, the stack meter stops it first.
    let mut w = load(src, Limits::default().call_depth(1_000_000).stack(64 * 1024)).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooDeep));
    // Recursion within the depth is fine.
    let src = "fn sum(n) { if n == 0 { 0 } else { n + sum(n - 1) } } fn render(s, h) { ui::text(sum(20)) } #{ name: \"sum\" }";
    let mut w = load(src, Limits::default()).must("loads");
    assert_eq!(render(&mut w), Ok(Node::Text { text: "210".into(), style: TextStyle::Body }));
}

#[test]
fn a_script_that_eats_memory_is_stopped_at_its_limit() {
    let limits =
        Limits::default().memory(4 * 1024 * 1024).time(Duration::from_secs(10)).operations(Some(50_000_000));
    for hog in [
        // A string that doubles.
        "let s = \"x\"; loop { s += s; }",
        // A list of growing strings.
        "let t = []; let s = \"xxxxxxxxxxxxxxxx\"; loop { s += s; t.push(s); }",
        // One huge pad.
        "let s = \"\"; s.pad(1_000_000_000, 'x');",
    ] {
        let mut w = load(&widget_running(hog), limits).must("loads");
        assert_eq!(render(&mut w), Err(ScriptError::TooBig), "{hog}");
    }
    // A map with ever more keys.
    let small = limits.memory(64 * 1024);
    let hog = "let m = #{}; let i = 0; loop { m[`k${i}`] = i; i += 1; }";
    let mut w = load(&widget_running(hog), small).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooBig), "{hog}");
    // Lists nested ever deeper are stopped before they are too deep to take apart.
    let hog = "let a = []; loop { a = [a]; }";
    let mut w = load(&widget_running(hog), limits).must("loads");
    assert!(render(&mut w).is_err(), "{hog}");
    // State kept between calls counts too: a widget that keeps too much starts over.
    let src = "fn render(s, h) { ui::button(\"Grow\", \"grow\") } \
               fn grow() { let s = \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"; while s.len < 1_000_000 { s += s; } \
                 if this.keep == () { this.keep = []; } this.keep.push(s); } #{ name: \"keeper\" }";
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
    // ...and the widget still works.
    assert!(render(&mut w).is_ok());
}

#[test]
fn a_widget_keeps_working_after_a_stop() {
    let src = "fn render(s, h) { if this.spin ?? false { loop {} } ui::button(\"Spin\", \"spin\") } \
               fn spin() { this.spin = true; } #{ name: \"once\" }";
    let limits = Limits::default().operations(Some(200_000)).time(Duration::from_secs(60));
    let mut w = load(src, limits).must("loads");
    let Ok(Node::Button { action, .. }) = render(&mut w) else { unreachable!("a button") };
    w.act(action).must("spin is set");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel), "stopped again, not stuck");
}

#[test]
fn what_is_not_a_widget_is_said_plainly() {
    for bad in
        ["5", "fn render(s, h) {", "#{ name: \"x\" }", "fn render(s, h) { 1 } 5", "fn render(s) { 1 } #{}"]
    {
        assert!(matches!(load(bad, Limits::default()), Err(ScriptError::Invalid(_))), "{bad}");
    }
    let r = RhaiEngine::new().load_source("fn render(s, h) { 1 } #{ name: 5 }", Limits::default());
    assert!(matches!(r, Err(ScriptError::Invalid(_))), "{r:?}");
    let deep = "fn render(s, h) { let n = ui::text(\"x\"); for i in 0..200 { n = ui::column([n]); } n } #{ name: \"d\" }";
    let mut w = load(deep, Limits::default().time(Duration::from_secs(60))).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
    let odd = "fn render(s, h) { #{ kind: \"iframe\" } } #{ name: \"o\" }";
    let mut w = load(odd, Limits::default()).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
    // Not text at all.
    let engines = Engines::new().with(std::rc::Rc::new(RhaiEngine::new()));
    let r =
        engines.load(&Package::new(remedy_ui_script::Language::RHAI, &[0xff, 0xfe][..]), Limits::default());
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
    let Some(per_ms) = remedy_ui_script_rhai::calibrate() else { unreachable!("calibration runs") };
    assert!(remedy_ui_script_rhai::rate().is_calibrated());
    assert!((100..100_000_000).contains(&per_ms), "{per_ms} operations a millisecond");
    // A machine half as fast (the same run taking twice as long) gets half the budget.
    let Some((steps, took)) = remedy_ui_script_rhai::measure() else { unreachable!("calibration runs") };
    let (here, slower) = (StepRate::new(1), StepRate::new(1));
    assert!(here.record(steps, took) && slower.record(steps, took * 2));
    let limits = Limits::default();
    let (a, b) = (limits.operations_at(here.per_ms()), limits.operations_at(slower.per_ms()));
    assert!(b < a && (a as f64 / b as f64 - 2.0).abs() < 0.01, "{a} {b}");
    // Budgets made from the measured rate still stop a hostile loop, in about its time.
    let mut w = load(&widget_running("loop {}"), limits).must("loads");
    let start = Instant::now();
    let r = render(&mut w);
    assert!(matches!(r, Err(ScriptError::OutOfFuel | ScriptError::TooSlow)), "{r:?}");
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
}

#[test]
fn a_busy_machine_does_not_stop_fair_work() {
    // Keep every core busy, twice over, while fair calls run: their budget is processor time
    // on their own thread, so time the system gives the spinners does not count against them.
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
fn the_app_sets_how_deeply_expressions_nest() {
    let (open, close) = ("(".repeat(80), ")".repeat(80));
    let deep = format!("fn render(s, h) {{ ui::text(\"x\") }} let x = {open}1{close}; #{{ name: \"deep\" }}");
    let e = load(&deep, Limits::default()).err();
    assert!(e.is_some(), "80 deep is past the default 64");
    let w = load(&deep, Limits::default().expr_depth(512, 256));
    assert!(w.is_ok(), "{:?} (default: {e:?})", w.err());
}
