//! WebAssembly widgets: the Rust example renders, takes settings and reacts to its button;
//! hostile modules (endless loops, runaway recursion, memory hogs, imports reaching for
//! files, clocks, sockets or processes) are each stopped with a clear error, and a widget
//! starts over after a stop. The checked-in example module is rebuilt from its source when
//! the wasm32 target is installed, to prove it matches.

#![cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]

#[path = "hostile/mod.rs"]
mod hostile;

use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::time::Instant;
use remedy_ui_script::{
    Action, Engines, Limits, Node, Package, ScriptError, ScriptWidget, SettingKind, TextStyle,
};
use remedy_ui_script_wasm::{WasmEngine, WasmWidget};
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

const CLAPS: &[u8] = include_bytes!("../examples/claps.wasm");

fn host() -> Value {
    json!({ "name": "Sam", "house": "the Den" })
}

/// Generous time, so only what a test is about stops a call.
fn roomy() -> Limits {
    Limits::default().time(Duration::from_secs(30))
}

fn load(module: &[u8], limits: Limits) -> Result<WasmWidget, ScriptError> {
    WasmEngine::new().load_module(module, limits)
}

fn render(w: &mut dyn ScriptWidget) -> Result<Node, ScriptError> {
    w.render(&Map::new(), &host())
}

fn shown() -> Node {
    Node::Text { text: "ok".into(), style: TextStyle::Body }
}

#[test]
fn the_example_renders_takes_settings_and_handles_its_button() {
    let engines = Engines::new().with(Rc::new(WasmEngine::new()));
    let mut w = engines.load(&Package::wasm(CLAPS), roomy()).must("claps loads");
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
    w.act(*action).must("clap again");
    let n = w.render(&settings, &host()).must("renders again");
    let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
    assert_eq!(
        children.get(1),
        Some(&Node::Text { text: "2 claps from the Den".into(), style: TextStyle::Body })
    );
    assert_eq!(children.get(2), Some(&Node::Progress { label: "Claps".into(), value: 0.2 }));
    assert_eq!(children.get(4), Some(&Node::Text { text: "Hello, Sam".into(), style: TextStyle::Muted }));
    assert!(w.memory_used() >= 64 * 1024, "{}", w.memory_used());
    assert!(matches!(w.act(Action(99)), Err(ScriptError::Shape(_))));
}

#[test]
fn the_example_says_the_same_as_its_rhai_twin() {
    let mut w = load(CLAPS, roomy()).must("loads");
    let n = render(&mut w).must("renders");
    let Node::Column { gap, children } = n else { unreachable!() };
    assert_eq!((gap, children.len()), (8.0, 5));
}

#[test]
fn a_hostile_module_cannot_reach_files_clocks_the_network_or_processes() {
    for (what, module) in hostile::reaching_out() {
        let r = load(&module, roomy());
        let Err(ScriptError::Denied(why)) = &r else { unreachable!("{what} must be refused, got {r:?}") };
        assert!(why.contains("imports"), "{what}: {why}");
    }
    assert!(matches!(load(&hostile::starts(), roomy()), Err(ScriptError::Invalid(_))));
}

#[test]
fn a_module_that_never_stops_is_stopped() {
    // By its fuel, deterministically.
    let limits = roomy().operations(Some(1_000_000));
    let mut w = load(&hostile::spin(), limits).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    // By its time budget, when the fuel would outlast it.
    let limits = Limits::default().operations(Some(u64::MAX / 2)).time(Duration::from_millis(20));
    let mut w = load(&hostile::spin(), limits).must("loads");
    let start = Instant::now();
    assert_eq!(render(&mut w), Err(ScriptError::TooSlow));
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
    // Spinning while it describes itself, it never loads.
    assert_eq!(load(&hostile::spin_on_load(), limits).err(), Some(ScriptError::TooSlow));
}

#[test]
fn runaway_recursion_is_stopped() {
    let mut w = load(&hostile::recurse(), roomy()).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooDeep));
    // A deeper call depth still stops, at the interpreter's stack.
    let mut w = load(&hostile::recurse(), roomy().call_depth(1_000_000)).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooDeep));
}

#[test]
fn a_module_that_eats_memory_is_stopped_at_its_limit() {
    let limits = roomy().memory(4 * 1024 * 1024);
    let mut w = load(&hostile::hog(), limits).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::TooBig));
    assert!(w.memory_used() <= limits.memory, "{}", w.memory_used());
    assert_eq!(load(&hostile::huge_at_start(), limits).err(), Some(ScriptError::TooBig));
}

#[test]
fn a_widget_says_why_it_failed_and_logs() {
    let mut w = load(&hostile::fails(), roomy()).must("loads");
    assert_eq!(render(&mut w), Err(ScriptError::Runtime(hostile::BOOM.into())));
    let mut w = load(&hostile::logs(), roomy()).must("loads");
    assert_eq!(render(&mut w), Ok(shown()));
}

#[test]
fn a_widget_starts_over_after_a_stop() {
    let limits = roomy().operations(Some(1_000_000));
    let mut w = load(&hostile::spins_after_press(), limits).must("loads");
    let Ok(Node::Button { action, .. }) = render(&mut w) else { unreachable!("a button") };
    w.act(action).must("pressed");
    assert_eq!(render(&mut w), Err(ScriptError::OutOfFuel));
    // A fresh instance: not pressed, so it renders again.
    assert!(matches!(render(&mut w), Ok(Node::Button { .. })));
}

#[test]
fn what_is_not_a_widget_is_said_plainly() {
    for bad in [&b"not wasm"[..], b"\0asm\x01\0\0\0garbage", b""] {
        assert!(matches!(load(bad, roomy()), Err(ScriptError::Invalid(_))), "{bad:?}");
    }
    let no_exports = wat::parse_str("(module (memory (export \"memory\") 1))").must("valid");
    assert!(matches!(load(&no_exports, roomy()), Err(ScriptError::Invalid(_))));
    let mut w = load(&hostile::renders("this is not json"), roomy()).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
    let mut deep = String::from(r#"{"kind":"text","text":"x"}"#);
    for _ in 0..100 {
        deep = format!(r#"{{"kind":"column","children":[{deep}]}}"#);
    }
    let mut w = load(&hostile::renders(&deep), roomy()).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
    let mut w = load(&hostile::renders(r#"{"kind":"iframe"}"#), roomy()).must("loads");
    assert!(matches!(render(&mut w), Err(ScriptError::Shape(_))));
}

#[test]
fn the_same_calls_describe_the_same_widget() {
    let mut a = load(CLAPS, roomy()).must("loads");
    let mut b = load(CLAPS, roomy()).must("loads");
    assert_eq!(render(&mut a), render(&mut b));
}

/// A module's sections other than custom ones (names, producers): what it does.
fn code_sections(module: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 8;
    while let Some(&id) = module.get(at) {
        at += 1;
        let (mut size, mut shift) = (0usize, 0);
        loop {
            let b = *module.get(at).must("a section size");
            at += 1;
            size |= usize::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        let body = module.get(at..at + size).must("a whole section").to_vec();
        at += size;
        if id != 0 {
            out.push((id, body));
        }
    }
    out
}

#[test]
fn the_checked_in_example_is_what_its_source_builds() {
    use std::path::PathBuf;
    use std::process::Command;
    let sysroot = Command::new("rustc").args(["--print", "sysroot"]).output().must("rustc runs");
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    if !sysroot.join("lib/rustlib/wasm32-unknown-unknown").is_dir() {
        eprintln!("the wasm32-unknown-unknown target is not installed: the example is not rebuilt");
        return;
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/claps/Cargo.toml");
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("claps-widget");
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status()
        .must("cargo runs");
    assert!(status.success(), "the example builds");
    let built =
        std::fs::read(target.join("wasm32-unknown-unknown/release/remedy_claps_widget.wasm")).must("built");
    assert!(
        code_sections(&built) == code_sections(CLAPS),
        "examples/claps.wasm is not what examples/claps builds: rebuild it with the command in examples/claps/Cargo.toml"
    );
}

#[test]
fn the_fuel_rate_is_measured_on_this_device() {
    use remedy_ui_script::StepRate;
    assert_eq!(
        wat::parse_str(remedy_ui_script_wasm::CALIBRATION_WAT).must("valid"),
        remedy_ui_script_wasm::CALIBRATION,
        "src/calibration.wasm is CALIBRATION_WAT compiled"
    );
    let Some(per_ms) = remedy_ui_script_wasm::calibrate() else { unreachable!("calibration runs") };
    assert!(remedy_ui_script_wasm::rate().is_calibrated());
    assert!((1_000..1_000_000_000).contains(&per_ms), "{per_ms} fuel a millisecond");
    // A machine half as fast (the same run taking twice as long) gets half the budget.
    let Some((steps, took)) = remedy_ui_script_wasm::measure() else { unreachable!("calibration runs") };
    let (here, slower) = (StepRate::new(1), StepRate::new(1));
    assert!(here.record(steps, took) && slower.record(steps, took * 2));
    let limits = Limits::default();
    let (a, b) = (limits.operations_at(here.per_ms()), limits.operations_at(slower.per_ms()));
    assert!(b < a && (a as f64 / b as f64 - 2.0).abs() < 0.01, "{a} {b}");
    // Budgets made from the measured rate still stop a hostile loop, in about its time.
    let mut w = load(&hostile::spin(), limits).must("loads");
    let start = Instant::now();
    let r = render(&mut w);
    assert!(matches!(r, Err(ScriptError::OutOfFuel | ScriptError::TooSlow)), "{r:?}");
    assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
}
