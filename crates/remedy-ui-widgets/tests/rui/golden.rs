//! The compiler's output for the settings screen is the checked-in golden file, which
//! `tests/rui.rs` compiles and runs. After a deliberate change, rerun with `RUI_BLESS=1` and
//! review the diff.

use std::path::PathBuf;

#[test]
fn the_compiler_writes_the_golden_file() {
    let src = include_str!("ui/settings_voice.rui");
    let Ok(rust) = remedy_ui_widgets::rui::codegen::compile_in(
        remedy_ui_widgets::rui::codegen::DIRECT_ROOT,
        "settings_voice.rui",
        src,
        &[],
    ) else {
        unreachable!("the settings screen compiles")
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/rui/golden/settings_voice.rs");
    if std::env::var_os("RUI_BLESS").is_some() {
        let _ = std::fs::write(&path, &rust);
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(rust, golden, "the compiler's output changed; rerun with RUI_BLESS=1 and review the diff");
    // Each signal's type follows from where it is used.
    assert!(rust.contains("pub input_gain: remedy_ui_markup::rt::reactive::Signal<f64>,"));
    assert!(rust.contains("pub input_devices: remedy_ui_markup::rt::reactive::Signal<Vec<String>>,"));
    assert!(rust.contains("pub nickname: remedy_ui_markup::rt::reactive::Signal<String>,"));
    assert!(rust.contains("pub save: ::std::rc::Rc<dyn Fn()>,"));
}
