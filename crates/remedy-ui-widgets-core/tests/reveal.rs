//! `reveal_on_hover`: a slider folded beside its button until the pointer comes over them or
//! keyboard focus goes in, and reachable by Tab and screen readers while folded.

use remedy_ui_core::{Point, Size};
use remedy_ui_input::{Key, KeyEvent, PointerEvent, PointerPhase};
use remedy_ui_layout::Align;
use remedy_ui_reactive::Signal;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts);
    ui.set_reduced_motion(true);
    ui
}

fn frame(ui: &Ui) {
    for _ in 0..2 {
        let _ = ui.frame(Size::new(400.0, 200.0), 1.0);
        let _ = ui.take_requests();
    }
}

fn width(el: &El) -> f32 {
    el.bounds().map_or(-1.0, |b| b.width())
}

#[test]
fn a_revealed_slider_shows_on_hover_and_on_focus_and_stays_reachable_folded() {
    let ui = ui();
    let button = w::button(&ui, "Mute", || {});
    let slider = w::slider(&ui, "Volume", Signal::new(0.5), 0.0, 1.0, 0.05);
    let holder = ui.el("box").child(slider.clone());
    let group = ui.el("box").row().align(Align::Center).child(button.clone()).child(holder.clone());
    let group = w::reveal_on_hover(group, &holder, 84.0);
    let _ = ui.root().child(ui.el("box").column().child(group.clone()));
    frame(&ui);
    assert_eq!(width(&holder), 0.0, "folded at first");
    let named = ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Volume"));
    assert!(named, "a screen reader finds it folded");

    // The pointer over the button unfolds it; leaving folds it again.
    let over = group.bounds().map_or(Point::new(0.0, 0.0), |b| b.center());
    let _ = ui.pointer(&PointerEvent::new(PointerPhase::Move, over));
    frame(&ui);
    assert_eq!(width(&holder), 84.0, "shown under the pointer");
    let _ = ui.pointer(&PointerEvent::new(PointerPhase::Move, Point::new(390.0, 190.0)));
    frame(&ui);
    assert_eq!(width(&holder), 0.0, "folded once the pointer leaves");

    // Tab reaches it through the button, and focus inside unfolds it.
    let _ = ui.key(&KeyEvent::press(Key::Tab).with_text(None));
    frame(&ui);
    let _ = ui.key(&KeyEvent::press(Key::Tab).with_text(None));
    frame(&ui);
    assert!(slider.is_focused(), "Tab reaches the folded slider");
    assert_eq!(width(&holder), 84.0, "shown while focus is in it");
}
