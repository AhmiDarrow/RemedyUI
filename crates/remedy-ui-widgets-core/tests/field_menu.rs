//! The text field Cut / Copy / Paste / Select All context menu.

use std::time::Duration;

use remedy_ui_core::a11y::Role;
use remedy_ui_core::{Point, Size};
use remedy_ui_core::input::{Buttons, PointerEvent, PointerPhase};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{Request, Ui};
use remedy_ui_widgets_core as w;

struct T {
    ui: Ui,
    size: Size,
}

impl T {
    fn new() -> T {
        let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts);
        T { ui, size: Size::new(640.0, 420.0) }
    }

    fn frame(&mut self) {
        for _ in 0..2 {
            let _ = self.ui.frame(self.size, 1.0);
            let _ = self.ui.take_requests();
        }
    }

    fn right_click(&mut self, at: Point) {
        let down = PointerEvent::new(PointerPhase::Down, at)
            .with_buttons(Buttons::SECONDARY)
            .with_time(Duration::from_millis(20));
        let up = PointerEvent::new(PointerPhase::Up, at).with_time(Duration::from_millis(40));
        let _ = self.ui.pointer(&down);
        self.frame();
        let _ = self.ui.pointer(&up);
        self.frame();
    }
}

#[test]
fn a_text_input_offers_cut_copy_paste_and_select_all_from_the_context_menu() {
    let mut t = T::new();
    let field = w::text_input(&t.ui, "Address");
    t.ui.root().child(field.el());
    t.frame();
    field.set_value("hello world");
    t.frame();
    let b = field.el().bounds().unwrap_or_default();
    assert!(b.width() > 0.0 && b.height() > 0.0, "the field should be laid out");
    let at = Point::new(b.min.x + 16.0, b.min.y + b.height() / 2.0);
    t.right_click(at);
    let names: Vec<String> =
        t.ui.reader_view().into_iter().filter(|n| n.role == Role::MenuItem).filter_map(|n| n.name).collect();
    assert!(names.iter().any(|n| n == "Cut"), "got {names:?}");
    assert!(names.iter().any(|n| n == "Copy"), "got {names:?}");
    assert!(names.iter().any(|n| n == "Paste"), "got {names:?}");
    assert!(names.iter().any(|n| n == "Select All"), "got {names:?}");

    // Select All should be enabled (the field has text). Activate it.
    let all =
        t.ui.reader_view()
            .into_iter()
            .find(|n| n.role == Role::MenuItem && n.name.as_deref() == Some("Select All"))
            .and_then(|n| t.ui.element_by_node_id(n.id));
    assert!(all.is_some(), "Select All item");
    let Some(all) = all else { return };
    let ab = all.bounds().unwrap_or_default();
    assert!(ab.height() > 0.0, "Select All should be laid out");
    let click = Point::new(ab.min.x + 8.0, ab.min.y + ab.height() / 2.0);
    let _ = t.ui.pointer(
        &PointerEvent::new(PointerPhase::Down, click)
            .with_buttons(Buttons::PRIMARY)
            .with_time(Duration::from_millis(400)),
    );
    let _ = t.ui.pointer(&PointerEvent::new(PointerPhase::Up, click).with_time(Duration::from_millis(420)));
    t.frame();
    assert_eq!(field.selection(), 0..field.value().len(), "Select All from the menu");
}

#[test]
fn a_secret_fields_context_menu_disables_cut_and_copy() {
    let mut t = T::new();
    let field = w::secret_input(&t.ui, "Password");
    t.ui.root().child(field.el());
    t.frame();
    // Secret fields don't take set_value; type into them.
    field.el().focus();
    t.frame();
    for c in ['s', 'e', 'c', 'r', 'e', 't'] {
        let _ = t.ui.key(&remedy_ui_core::input::KeyEvent::press(remedy_ui_core::input::Key::char(c)));
    }
    t.frame();
    field.select_all();
    t.frame();
    let b = field.el().bounds().unwrap_or_default();
    assert!(b.width() > 0.0 && b.height() > 0.0, "the field should be laid out");
    t.right_click(Point::new(b.min.x + 16.0, b.min.y + b.height() / 2.0));
    let items: Vec<_> = t.ui.reader_view().into_iter().filter(|n| n.role == Role::MenuItem).collect();
    let cut = items.iter().find(|n| n.name.as_deref() == Some("Cut"));
    let copy = items.iter().find(|n| n.name.as_deref() == Some("Copy"));
    assert!(cut.is_some_and(|n| n.states.contains(&"disabled")), "secrets are never cut: {items:?}");
    assert!(copy.is_some_and(|n| n.states.contains(&"disabled")), "secrets are never copied: {items:?}");
    let copy_id = copy.map(|n| n.id);
    // Choosing Copy must not request a clipboard write.
    let _ = t.ui.take_requests();
    if let Some(el) = copy_id.and_then(|id| t.ui.element_by_node_id(id))
        && let Some(cb) = el.bounds()
    {
        let click = Point::new(cb.min.x + 8.0, cb.min.y + cb.height() / 2.0);
        let _ = t.ui.pointer(
            &PointerEvent::new(PointerPhase::Down, click)
                .with_buttons(Buttons::PRIMARY)
                .with_time(Duration::from_millis(400)),
        );
        let _ =
            t.ui.pointer(&PointerEvent::new(PointerPhase::Up, click).with_time(Duration::from_millis(420)));
        t.frame();
    }
    assert!(
        !t.ui.take_requests().iter().any(|r| matches!(r, Request::Copy { .. })),
        "a secret field must not copy out"
    );
}
