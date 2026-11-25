//! [`set_max_depth`] moves how deeply elements may nest. A binary of its own, so no other
//! test sees the bound changed.

use remedy_ui_widgets::rui::{MAX_DEPTH, Markup, max_depth, set_max_depth};

#[test]
fn the_app_sets_how_deeply_elements_nest() {
    let deep = format!("{}{}", "column {".repeat(150), "}".repeat(150));
    assert_eq!(max_depth(), MAX_DEPTH);
    assert!(Markup::parse("deep.rui", &deep).is_err(), "150 deep is past the default 128");
    set_max_depth(200);
    assert!(Markup::parse("deep.rui", &deep).is_ok());
    set_max_depth(4);
    let e = Markup::parse("deep.rui", "column { column { column { column { column {} } } } }");
    assert!(e.is_err_and(|e| e.message.contains("more than 4")));
}
