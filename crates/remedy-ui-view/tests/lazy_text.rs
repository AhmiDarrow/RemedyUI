//! A huge paragraph in a text element or a multi-line field draws only what shows: the first
//! frame lays out and draws the lines in the window, scrolling down draws the lines it
//! reaches, and typing in the middle of the text keeps the caret where the text is.

use remedy_ui_compositor::{Layer, LayerTree};
use remedy_ui_core::{Size, Vec2};
use remedy_ui_core::input::{Key, KeyEvent};
use remedy_ui_core::layout::Length;
use remedy_ui_paint::Item;
use remedy_ui_core::style::{StyleSheet, Theme};
use remedy_ui_text::{FontCollection, LAZY_MIN_BYTES};
use remedy_ui_view::{FieldKind, Ui};

const SIZE: Size = Size { width: 420.0, height: 300.0 };

fn ui() -> Ui {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    Ui::new(Theme::dark_forest(), StyleSheet::new(), f)
}

fn prose(bytes: usize) -> String {
    let words =
        ["the", "quick", "brown", "fox", "jumps", "over", "a", "lazy", "dog", "while", "nobody", "reads"];
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes {
        i = i.wrapping_mul(31).wrapping_add(7);
        s.push_str(words.get(i % words.len()).copied().unwrap_or("a"));
        s.push(' ');
    }
    s
}

/// The y of every glyph drawn, in the coordinates of the content it is drawn in (a scroll
/// container's content, unscrolled).
fn glyph_ys(layer: &Layer, at: f32, out: &mut Vec<f32>) {
    for item in layer.content.items() {
        if let Item::Glyphs(run) = item {
            out.extend(run.glyphs.iter().map(|(_, p)| p.y + at));
        }
    }
    for child in &layer.children {
        glyph_ys(child, at, out);
    }
}

/// Runs a frame and returns the glyphs the layers shown now draw.
fn frame(ui: &Ui, last: &mut Option<LayerTree>) -> Vec<f32> {
    if let Some(l) = ui.frame(SIZE, 1.0).layers {
        *last = Some(l);
    }
    let mut ys = Vec::new();
    for root in last.as_ref().map(|l| l.roots.clone()).unwrap_or_default() {
        glyph_ys(&root, 0.0, &mut ys);
    }
    ys
}

#[test]
fn a_huge_text_draws_what_shows_and_what_it_scrolls_to() {
    let ui = ui();
    let mut last = None;
    let text = prose(LAZY_MIN_BYTES * 10);
    let body = ui.el("body").text(text.as_str()).fill_width().layout(|l| l.shrink = 0.0);
    let scroller = ui
        .el("doc")
        .scroll()
        .column()
        .layout(|l| {
            l.height = Length::Px(SIZE.height);
            l.width = Length::Px(SIZE.width);
        })
        .child(body.clone());
    ui.root().child(scroller.clone());
    let _ = frame(&ui, &mut last);
    let ys = frame(&ui, &mut last);
    let height = body.bounds().map_or(0.0, |b| b.height());
    assert!(height > 20.0 * SIZE.height, "the whole paragraph is measured: {height}");
    let deepest = ys.iter().copied().fold(0.0, f32::max);
    assert!(!ys.is_empty(), "the top lines draw");
    assert!(deepest < SIZE.height * 4.0, "only the lines near the window draw: down to {deepest}");
    // Scrolled far down, the lines there draw.
    let to = Vec2::new(0.0, height * 0.4);
    ui.set_scroll_offset(scroller.id(), to);
    let _ = frame(&ui, &mut last);
    let ys = frame(&ui, &mut last);
    let on_screen = ys.iter().filter(|&&y| (to.y..to.y + SIZE.height).contains(&y)).count();
    assert!(on_screen > 100, "the lines scrolled to draw: {on_screen} glyphs in the window");
    assert!(ys.iter().all(|&y| (y - to.y).abs() < SIZE.height * 4.0), "and none far from it");
}

#[test]
fn typing_in_a_huge_field_follows_the_caret() {
    let ui = ui();
    let mut last = None;
    let text = prose(LAZY_MIN_BYTES * 8);
    let f = ui.text_field("text_input", FieldKind::Multi);
    let _ = ui.root().child(f.el().layout(|l| {
        l.width = Length::Px(SIZE.width);
        l.height = Length::Px(SIZE.height);
    }));
    f.set_value(&text);
    let _ = frame(&ui, &mut last);
    f.el().focus();
    let _ = frame(&ui, &mut last);
    // To the middle, then type.
    let mid = text.len() / 2;
    let mid = (mid..).find(|&i| text.as_bytes().get(i) == Some(&b' ')).unwrap_or(mid) + 1;
    assert!(f.replace_range(mid..mid, "z"));
    let _ = frame(&ui, &mut last);
    for c in ["y", "x"] {
        ui.key(&KeyEvent::press(Key::Character(c.into())).with_text(Some(c.into())));
        let _ = frame(&ui, &mut last);
    }
    assert_eq!(f.value().get(mid..mid + 3), Some("zyx"));
    assert_eq!(f.caret(), mid + 3);
    let caret = f.caret_rect().unwrap_or_default();
    let field = f.el().bounds().unwrap_or_default();
    assert!(field.contains(caret.center()), "the field scrolled to the caret: {caret:?} in {field:?}");
    let ys = frame(&ui, &mut last);
    assert!(ys.len() < 20_000, "only the lines around the caret draw: {} glyphs", ys.len());
}
