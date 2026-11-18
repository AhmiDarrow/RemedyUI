//! Scripted widgets in a real browser page (run with the web bridge's `tools/web-test.sh`):
//! both engines load the example widgets, render and react to a press; hostile widgets
//! (endless loops, runaway recursion, memory hogs, reaching for what the sandbox does not
//! give) are stopped with a clear error on the page's own clock and stack; and a board shows
//! scripted widgets of both languages.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

#[path = "../../remedy-ui-script-wasm/tests/hostile/mod.rs"]
mod hostile;

use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::Size;
use remedy_ui_core::time::Instant;
use remedy_ui_script::{Node, ScriptWidget, TextStyle};
use remedy_ui_script_rhai::RhaiEngine;
use remedy_ui_script_wasm::WasmEngine;
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;
use remedy_ui_widgets_board::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

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

const CLAPS_RHAI: &str = include_str!("../../remedy-ui-script-rhai/examples/claps.rhai");
const CLAPS_WASM: &[u8] = include_bytes!("../../remedy-ui-script-wasm/examples/claps.wasm");

fn engines() -> Engines {
    Engines::new().with(Rc::new(RhaiEngine::new())).with(Rc::new(WasmEngine::new()))
}

fn roomy() -> Limits {
    Limits::default().time(Duration::from_secs(10))
}

fn host() -> serde_json::Value {
    serde_json::json!({ "name": "Sam", "house": "the Den" })
}

fn render(w: &mut dyn ScriptWidget) -> Result<Node, ScriptError> {
    w.render(&serde_json::Map::new(), &host())
}

fn claps_line(n: &Node) -> Option<&Node> {
    let Node::Column { children, .. } = n else { return None };
    children.get(1)
}

#[wasm_bindgen_test]
fn both_examples_render_and_react_in_a_page() {
    for package in [Package::rhai(CLAPS_RHAI), Package::wasm(CLAPS_WASM)] {
        let what = package.language.to_string();
        let mut w = engines().load(&package, roomy()).must(&what);
        assert_eq!(w.name(), "Claps", "{what}");
        let n = render(w.as_mut()).must(&what);
        let Node::Column { children, .. } = &n else { unreachable!("{what}: {n:?}") };
        let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!("{what}: {children:?}") };
        w.act(*action).must(&what);
        let n = render(w.as_mut()).must(&what);
        assert_eq!(
            claps_line(&n),
            Some(&Node::Text { text: "1 claps from the Den".into(), style: TextStyle::Body }),
            "{what}"
        );
    }
}

/// Loads `package` with `limits` and renders it once.
fn run(package: Package, limits: Limits) -> Result<Node, ScriptError> {
    let mut w = engines().load(&package, limits)?;
    render(w.as_mut())
}

fn rhai_running(body: &str) -> Package {
    Package::rhai(&format!("fn render(s, h) {{ {body}; ui::text(\"ok\") }} #{{ name: \"probe\" }}"))
}

#[wasm_bindgen_test]
fn endless_loops_stop_on_fuel_and_on_the_page_clock() {
    let fuel = roomy().operations(Some(200_000));
    assert_eq!(run(rhai_running("loop {}"), fuel), Err(ScriptError::OutOfFuel));
    assert_eq!(run(Package::wasm(&hostile::spin()), fuel), Err(ScriptError::OutOfFuel));
    let clock = Limits::default().operations(Some(u64::MAX / 2)).time(Duration::from_millis(30));
    for package in [rhai_running("loop {}"), Package::wasm(&hostile::spin())] {
        let start = Instant::now();
        assert_eq!(run(package, clock), Err(ScriptError::TooSlow));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
    }
}

#[wasm_bindgen_test]
fn runaway_recursion_stops() {
    let rhai = Package::rhai("fn down(n) { down(n + 1) } fn render(s, h) { down(0) } #{ name: \"deep\" }");
    assert_eq!(run(rhai.clone(), roomy()), Err(ScriptError::TooDeep));
    assert_eq!(run(rhai, roomy().call_depth(1_000_000)), Err(ScriptError::TooDeep));
    assert_eq!(run(Package::wasm(&hostile::recurse()), roomy()), Err(ScriptError::TooDeep));
}

#[wasm_bindgen_test]
fn memory_hogs_stop_at_their_limit() {
    let limits = roomy().memory(4 * 1024 * 1024).operations(Some(50_000_000));
    assert_eq!(run(rhai_running("let s = \"x\"; loop { s += s; }"), limits), Err(ScriptError::TooBig));
    let keys = "let m = #{}; let i = 0; loop { m[`k${i}`] = i; i += 1; }";
    assert_eq!(run(rhai_running(keys), limits.memory(64 * 1024)), Err(ScriptError::TooBig));
    assert!(run(rhai_running("let a = []; loop { a = [a]; }"), limits).is_err());
    assert_eq!(run(Package::wasm(&hostile::hog()), limits), Err(ScriptError::TooBig));
}

#[wasm_bindgen_test]
fn what_the_sandbox_does_not_give_is_refused() {
    for probe in ["open_file(\"x\")", "http_get(\"https://example.com\")", "eval(\"1\")", "import \"x\" as y"]
    {
        let r = run(rhai_running(probe), roomy());
        assert!(matches!(r, Err(ScriptError::Denied(_) | ScriptError::Invalid(_))), "{probe}: {r:?}");
    }
    for (what, module) in hostile::reaching_out() {
        let r = run(Package::wasm(&module), roomy());
        assert!(matches!(r, Err(ScriptError::Denied(_))), "{what}: {r:?}");
    }
}

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

#[wasm_bindgen_test]
fn a_board_in_a_page_shows_scripted_widgets_of_both_languages() {
    let mut r = Registry::with_builtins();
    for (id, package) in
        [("claps-rhai", Package::rhai(CLAPS_RHAI)), ("claps-wasm", Package::wasm(CLAPS_WASM))]
    {
        r.add(Rc::new(ScriptKind::new(id, package, &engines(), roomy()).must(id)));
    }
    let (b, area) = Board::default().add_area("Home", 12, 1).must("an area");
    let (b, _) = b.add_widget(&area, "claps-rhai", serde_json::Map::new()).must("rhai");
    let (b, _) = b.add_widget(&area, "claps-wasm", serde_json::Map::new()).must("wasm");
    let state = BoardState::new(b);
    let mut cx = Context::local(|_| {});
    cx.host.set(host());
    cx.engines = engines();
    cx.limits = roomy();
    let ui = ui();
    ui.root().child(board(&ui, &state, Rc::new(r), cx));
    let _ = ui.frame(Size::new(1200.0, 800.0), 1.0);
    let mut texts = Vec::new();
    let mut stack: Vec<_> = ui.layers().to_vec();
    while let Some(e) = stack.pop() {
        stack.extend(e.child_elements());
        texts.extend(e.info().and_then(|i| i.text));
    }
    let shown = texts.iter().filter(|t| t.as_str() == "0 claps from the Den").count();
    assert_eq!(shown, 2, "{texts:?}");
}

#[wasm_bindgen_test]
fn the_engines_measure_their_rates_in_the_page() {
    // Loading queues each engine's calibration as cooperative work; the page runs it in a
    // slice between frames, as the bridge does.
    let _ = engines().load(&Package::rhai(CLAPS_RHAI), roomy());
    let _ = engines().load(&Package::wasm(CLAPS_WASM), roomy());
    let mut turns = 0;
    while remedy_ui_core::exec::run_pending(Duration::from_millis(50)) && turns < 100 {
        turns += 1;
    }
    assert!(remedy_ui_script_rhai::rate().is_calibrated(), "Rhai calibrated in a slice");
    assert!(remedy_ui_script_wasm::rate().is_calibrated(), "WebAssembly calibrated in a slice");
    let rhai = remedy_ui_script_rhai::calibrate().must("Rhai calibrates");
    let wasm = remedy_ui_script_wasm::calibrate().must("WebAssembly calibrates");
    assert!((100..100_000_000).contains(&rhai), "{rhai}");
    assert!((1_000..1_000_000_000).contains(&wasm), "{wasm}");
    // Budgets from the page's own rates still stop an endless loop in about its time.
    for package in [rhai_running("loop {}"), Package::wasm(&hostile::spin())] {
        let start = Instant::now();
        let r = run(package, Limits::default());
        assert!(matches!(r, Err(ScriptError::OutOfFuel | ScriptError::TooSlow)), "{r:?}");
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    }
}
