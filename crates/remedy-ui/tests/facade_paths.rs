//! Apps that depend only on `remedy-ui` reach everything through the facade: derived stores
//! and compiled markup name the facade's paths, and the semantics and drawing crates are
//! re-exported.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui::a11y::{Role, Semantics};
use remedy_ui::paint::{Affine, Brush, Path};
use remedy_ui::reactive::Signal;

/// `.rui` compiled with the default root, `remedy_ui::markup`; `the_compiled_markup_is_current`
/// keeps it in step with the compiler.
mod facade_screen {
    include!("markup/facade.rs");
}

#[derive(remedy_ui::Store, Clone, PartialEq)]
struct Draft {
    title: String,
    pinned: bool,
}

#[test]
fn a_store_derived_through_the_facade_works() -> Result<(), remedy_ui::reactive::StoreError> {
    let draft = remedy_ui::reactive::Store::new(Draft { title: "Notes".into(), pinned: false });
    draft.pinned().set(true)?;
    assert_eq!(draft.title().get()?, "Notes");
    assert!(draft.pinned().get()?);
    Ok(())
}

#[test]
fn the_compiled_markup_is_current() {
    let src = include_str!("markup/facade.rui");
    let Ok(rust) = remedy_ui::markup::codegen::compile("facade.rui", src, &[]) else {
        unreachable!("the screen compiles")
    };
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/markup/facade.rs");
    if std::env::var_os("RUI_BLESS").is_some() {
        let _ = std::fs::write(&path, &rust);
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(rust, golden, "rerun with RUI_BLESS=1 and review the diff");
    assert!(rust.contains("use ::remedy_ui::markup as remedy_ui_markup;"));
}

#[test]
fn compiled_markup_builds_through_the_facade() {
    let theme = remedy_ui::themes::ThemeChoice::default().resolve(None, false);
    let ui = remedy_ui::Ui::new(
        theme.clone(),
        remedy_ui::themes::sheet(&theme),
        remedy_ui::text::FontCollection::new(),
    );
    let saved = Rc::new(Cell::new(0));
    let s2 = saved.clone();
    let bindings = facade_screen::Bindings {
        noise_suppression: Signal::new(true),
        save: Rc::new(move || s2.set(s2.get() + 1)),
    };
    let el = facade_screen::build(&ui, &bindings, &|s| s.to_owned());
    ui.root().child(el);
    // A person's own version of the file loads through the same names.
    let _scope = bindings.scope(|s| s.to_owned());
    assert_eq!(facade_screen::FILE, "facade.rui");
    assert!(facade_screen::SOURCE.contains("switch"));
    let _ = ui.frame(remedy_ui::core::Size::new(320.0, 200.0), 1.0);
    // The semantics types are the facade's own re-exports.
    let s = Semantics::new(Role::Button).label("Save");
    assert_eq!(s.role(), Role::Button);
    // So are the drawing types painters use.
    let mut path = Path::new();
    path.move_to(remedy_ui::core::Point::new(0.0, 0.0));
    path.line_to(remedy_ui::core::Point::new(4.0, 4.0));
    let _ = (Brush::Solid(remedy_ui::core::Color::rgb8(1, 2, 3)), Affine::IDENTITY, path);
}
