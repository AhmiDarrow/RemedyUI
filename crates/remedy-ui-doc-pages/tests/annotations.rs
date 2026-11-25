//! Annotations on a real view: marking text, drawing ink and shapes, notes and text boxes,
//! selecting, moving, nudging and deleting them, undo and redo through the window's history,
//! exact placement under zoom and rotation, and what screen readers hear.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Point, Rect, Size, Vec2};
use remedy_ui_doc_pages::*;
use remedy_ui_input::{Buttons, Key, Modifiers, PointerKind, PointerPhase, ScrollPhysics};
use remedy_ui_render_cpu::{CpuCompositor, Frame};

mod common;
use common::*;

const WINDOW: Size = Size { width: 800.0, height: 600.0 };
const LONG: Duration = Duration::from_secs(20);

fn win() -> Win {
    let mut w = Win::new(Arc::new(Synthetic::new(5)), WINDOW);
    let _ = w.settle(LONG);
    for _ in 0..20 {
        w.frame();
    }
    w
}

fn zoom(w: &Win) -> f32 {
    w.view.page_rect(0).width() / LETTER.width
}

/// A page-0 point (points) in the view.
fn at(w: &Win, x: f32, y: f32) -> Point {
    let r = w.view.page_rect(0);
    let z = zoom(w);
    Point::new(r.min.x + x * z, r.min.y + y * z)
}

fn drag(w: &mut Win, from: Point, to: Point) {
    w.pointer(0, PointerKind::Mouse, PointerPhase::Down, from, Buttons::PRIMARY);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        let p = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        w.pointer(0, PointerKind::Mouse, PointerPhase::Move, p, Buttons::PRIMARY);
    }
    w.pointer(0, PointerKind::Mouse, PointerPhase::Up, to, Buttons::default());
}

fn click(w: &mut Win, p: Point) {
    w.pointer(0, PointerKind::Mouse, PointerPhase::Down, p, Buttons::PRIMARY);
    w.pointer(0, PointerKind::Mouse, PointerPhase::Up, p, Buttons::default());
}

fn snapshot(w: &Win) -> Frame {
    let mut c = Compositor::new(WINDOW.width as u32, WINDOW.height as u32, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    if let Some(t) = w.last.clone() {
        c.commit(t);
    }
    let plan = c.frame(Duration::ZERO);
    cpu.present(&plan, WINDOW.width as u32, WINDOW.height as u32, Color::BLACK).unwrap_or(Frame {
        width: 0,
        height: 0,
        pixels: Vec::new(),
    })
}

fn pixel(w: &Win, p: Point) -> [u8; 4] {
    snapshot(w).pixel(p.x.round() as u32, p.y.round() as u32).unwrap_or([0; 4])
}

fn first(w: &Win) -> Annotation {
    w.view.annotations().get().into_iter().next().unwrap_or_else(|| unreachable!("an annotation"))
}

#[test]
fn text_tools_mark_the_selection_with_its_words() {
    let mut w = win();
    let line = line_rect(0, &line_text(0, 0));
    w.view.set_tool(Tool::Highlight);
    {
        let (p0, p1) = (
            at(&w, line.min.x + 1.0, line.center().y),
            at(&w, line.min.x + 6.0 * 7.0 - 1.0, line.center().y),
        );
        drag(&mut w, p0, p1);
    }
    let list = w.view.annotations().get();
    assert_eq!(list.len(), 1, "{list:?}");
    let Some(a) = list.first() else { unreachable!("a mark") };
    assert!(matches!(&a.kind, AnnotationKind::Highlight { quads } if quads.len() == 1));
    assert_eq!(a.marked_text, "Page 1");
    assert_eq!(w.view.selected_text(), None, "the selection became the mark");
    assert_eq!(w.ui.next_undo().map(|h| h.undo_label()).as_deref(), Some("Undo Add highlight"));

    // A selection made first is marked when a text tool is chosen; one mark per page.
    w.view.set_tool(Tool::Select);
    let second = line_rect(1, &line_text(0, 1));
    {
        let (p0, p1) = (
            at(&w, second.min.x + 1.0, second.center().y),
            at(&w, second.min.x + 6.0 * 7.0 - 1.0, second.center().y),
        );
        drag(&mut w, p0, p1);
    }
    assert!(w.view.selected_text().is_some());
    w.view.set_tool(Tool::Underline);
    assert_eq!(w.view.annotations().get().len(), 2);
    assert!(w.view.annotations().get().iter().any(|a| matches!(a.kind, AnnotationKind::Underline { .. })));

    // The highlight shows in its color, blended over the page.
    w.frame();
    let mid = at(&w, line.min.x + 3.0, line.center().y);
    let px = pixel(&w, mid);
    assert!(px[2] < px[0] && px[2] < px[1], "yellow over white: {px:?}");
}

#[test]
fn ink_shapes_notes_and_text_boxes_are_drawn_on_pages() {
    let mut w = win();
    let opened: Rc<RefCell<Vec<Annotation>>> = Rc::default();
    let o = opened.clone();
    w.view.on_open_annotation(move |a| o.borrow_mut().push(a));
    w.view.set_annotation_style(AnnotationStyle {
        color: Color::rgba8(200, 0, 0, 255),
        width: 4.0,
        text_size: 14.0,
    });

    w.view.set_tool(Tool::Ink);
    {
        let (p0, p1) = (at(&w, 300.0, 400.0), at(&w, 400.0, 450.0));
        drag(&mut w, p0, p1);
    }
    let ink = first(&w);
    let AnnotationKind::Ink { strokes, width } = &ink.kind else { unreachable!("ink: {ink:?}") };
    assert_eq!(*width, 4.0);
    // A straight drag keeps only its ends.
    assert_eq!(strokes.first().map(Vec::len), Some(2), "{strokes:?}");
    let s = strokes.first().cloned().unwrap_or_default();
    let (Some(s0), Some(s1)) = (s.first(), s.get(1)) else { unreachable!("two points") };
    assert!((s0.x - 300.0).abs() < 0.5 && (s1.y - 450.0).abs() < 0.5, "in page points: {s:?}");
    w.frame();
    let px = pixel(&w, at(&w, 350.0, 425.0));
    assert!(px[0] > 150 && px[1] < 80, "the stroke shows in red: {px:?}");

    w.view.set_tool(Tool::Shape(Shape::Rect));
    {
        let (p0, p1) = (at(&w, 100.0, 120.0), at(&w, 200.0, 180.0));
        drag(&mut w, p0, p1);
    }
    w.view.set_tool(Tool::Shape(Shape::Arrow));
    {
        let (p0, p1) = (at(&w, 100.0, 220.0), at(&w, 250.0, 220.0));
        drag(&mut w, p0, p1);
    }
    // A click without a drag makes no shape.
    {
        let (p0,) = (at(&w, 450.0, 220.0),);
        click(&mut w, p0);
    }
    let shapes: Vec<Shape> = w
        .view
        .annotations()
        .get()
        .iter()
        .filter_map(|a| match a.kind {
            AnnotationKind::Shape { shape, .. } => Some(shape),
            _ => None,
        })
        .collect();
    assert_eq!(shapes, vec![Shape::Rect, Shape::Arrow]);

    w.view.set_tool(Tool::Note);
    {
        let (p0,) = (at(&w, 500.0, 300.0),);
        click(&mut w, p0);
    }
    let note = w.view.annotations().get().last().cloned().unwrap_or_else(|| unreachable!("a note"));
    assert!(matches!(note.kind, AnnotationKind::Note { .. }));
    assert_eq!(opened.borrow().last().map(|a| a.id), Some(note.id), "placing a note opens it");
    assert_eq!(w.view.selected_annotation().get(), Some(note.id));

    w.view.set_tool(Tool::FreeText);
    {
        let (p0, p1) = (at(&w, 100.0, 250.0), at(&w, 400.0, 300.0));
        drag(&mut w, p0, p1);
    }
    let tb = w.view.annotations().get().last().cloned().unwrap_or_else(|| unreachable!("a text box"));
    assert_eq!(opened.borrow().last().map(|a| a.id), Some(tb.id), "a new text box opens for writing");
    // The app writes what the person typed.
    assert!(w.view.update_annotation(tb.id, |a| {
        if let AnnotationKind::FreeText { text, .. } = &mut a.kind {
            *text = "Check this figure".into();
        }
    }));
    w.frame();
    // Its words are drawn: some dark pixels inside the box.
    let f = snapshot(&w);
    let (a, b) = (at(&w, 102.0, 252.0), at(&w, 300.0, 268.0));
    let mut ink_px = 0;
    for y in (a.y as u32)..(b.y as u32) {
        for x in (a.x as u32)..(b.x as u32) {
            if f.pixel(x, y).is_some_and(|p| p[0] > 120 && p[1] < 100) {
                ink_px += 1;
            }
        }
    }
    assert!(ink_px > 20, "the text box's words are drawn ({ink_px} red pixels)");
}

#[test]
fn annotations_are_selected_moved_nudged_deleted_and_undone() {
    let mut w = win();
    let id = w.view.add_annotation(Annotation::new(
        0,
        AnnotationKind::Shape {
            shape: Shape::Rect,
            from: Point::new(100.0, 200.0),
            to: Point::new(200.0, 280.0),
            width: 3.0,
        },
        Color::rgba8(0, 0, 200, 255),
    ));
    w.view.set_tool(Tool::Select);
    // Pressing the shape's edge selects it; dragging moves it by the drag in page points.
    let z = zoom(&w);
    {
        let (p0, p1) = (at(&w, 100.0, 240.0), at(&w, 150.0, 260.0));
        drag(&mut w, p0, p1);
    }
    assert_eq!(w.view.selected_annotation().get(), Some(id));
    let moved = w.view.annotation(id).unwrap_or_else(|| unreachable!("it"));
    let AnnotationKind::Shape { from, .. } = moved.kind else { unreachable!("a shape") };
    assert!((from.x - 150.0).abs() < 1.0 / z + 0.01 && (from.y - 220.0).abs() < 1.0 / z + 0.01, "{from:?}");
    assert_eq!(w.ui.next_undo().map(|h| h.label).as_deref(), Some("Move rectangle"));

    // Arrows nudge the selected annotation a point (Shift: ten).
    assert!(w.key(Key::ArrowRight, Modifiers::NONE));
    assert!(w.key(Key::ArrowDown, Modifiers { shift: true, ..Modifiers::NONE }));
    let b = w.view.annotation(id).map(|a| a.bounds()).unwrap_or(Rect::ZERO);
    let before = moved.bounds();
    assert!((b.min.x - before.min.x - 1.0).abs() < 0.01 && (b.min.y - before.min.y - 10.0).abs() < 0.01);

    // Delete removes it; undo brings it back where it was, redo removes it again.
    assert!(w.key(Key::Delete, Modifiers::NONE));
    assert!(w.view.annotations().get().is_empty());
    assert_eq!(w.view.selected_annotation().get(), None);
    assert!(w.ui.undo_change());
    assert_eq!(w.view.annotation(id).map(|a| a.bounds()), Some(b));
    assert!(w.ui.redo_change());
    assert!(w.view.annotation(id).is_none());
    // Back through the delete, both nudges and the move, to where it was added.
    for _ in 0..4 {
        assert!(w.ui.undo_change());
    }
    assert_eq!(w.view.annotations().get().len(), 1);
    let AnnotationKind::Shape { from, .. } = first(&w).kind else { unreachable!("a shape") };
    assert_eq!(from, Point::new(100.0, 200.0));

    // ] and [ step through annotations in reading order; Escape lets go.
    let second = w.view.add_annotation(Annotation::new(
        0,
        AnnotationKind::Note { at: Point::new(50.0, 50.0) },
        Color::WHITE,
    ));
    assert!(w.key(Key::Character("]".into()), Modifiers::NONE));
    assert_eq!(w.view.selected_annotation().get(), Some(second), "the note is higher on the page");
    assert!(w.key(Key::Character("]".into()), Modifiers::NONE));
    assert_eq!(w.view.selected_annotation().get(), Some(id));
    assert!(w.key(Key::Escape, Modifiers::NONE));
    assert_eq!(w.view.selected_annotation().get(), None);
    // Without a selection, arrows scroll as before.
    let s = w.view.scroll_offset();
    assert!(w.key(Key::ArrowDown, Modifiers::NONE));
    assert!(w.view.scroll_offset().y > s.y);
}

#[test]
fn annotations_stay_exact_under_zoom_and_rotation() {
    let mut w = win();
    let blue = Color::rgba8(0, 0, 220, 255);
    let id = w.view.add_annotation(Annotation::new(
        0,
        AnnotationKind::Shape {
            shape: Shape::Line,
            from: Point::new(100.0, 300.0),
            to: Point::new(400.0, 300.0),
            width: 6.0,
        },
        blue,
    ));
    for z in [0.5_f32, 2.0, 4.0] {
        w.view.set_zoom(z, None);
        let _ = w.settle(LONG);
        w.view.select_annotation(Some(id));
        w.view.select_annotation(None);
        let _ = w.settle(LONG);
        let p = at(&w, 250.0, 300.0);
        if p.x > 0.0 && p.y > 0.0 && p.x < WINDOW.width && p.y < WINDOW.height {
            let px = pixel(&w, p);
            assert!(px[2] > 150 && px[0] < 80, "on the line at zoom {z}: {px:?}");
            let off = pixel(&w, Point::new(p.x, p.y + 10.0 * z.max(1.0)));
            assert!(!(off[2] > 150 && off[0] < 80), "the line stays its width at zoom {z}: {off:?}");
        }
    }
    w.view.set_zoom(1.0, None);
    w.view.set_rotation(Rotation::Clockwise);
    let _ = w.settle(LONG);
    w.view.select_annotation(Some(id));
    w.view.select_annotation(None);
    let _ = w.settle(LONG);
    // Turned clockwise: page point (x, y) shows at (left + (H - y) z, top + x z).
    let r = w.view.page_rect(0);
    let z = r.width() / LETTER.height;
    let p = Point::new(r.min.x + (LETTER.height - 300.0) * z, r.min.y + 250.0 * z);
    if p.y < WINDOW.height {
        let px = pixel(&w, p);
        assert!(px[2] > 150 && px[0] < 80, "the line turned with the page: {px:?}");
    }
    // Pressing it on the turned page still finds it.
    w.view.set_tool(Tool::Select);
    click(&mut w, p);
    assert_eq!(w.view.selected_annotation().get(), Some(id));
}

#[test]
fn the_app_loads_and_reads_annotations_and_screen_readers_hear_them() {
    let mut w = win();
    let mut hl = Annotation::new(
        0,
        AnnotationKind::Highlight { quads: vec![Rect::new(72.0, 90.0, 200.0, 104.0)] },
        Color::WHITE,
    );
    hl.marked_text = "the quick brown fox".into();
    hl.comment = "Cite this".into();
    hl.author = Some("Sam".into());
    hl.key = Some("12 0 R".into());
    let note = Annotation::new(1, AnnotationKind::Note { at: Point::new(10.0, 10.0) }, Color::WHITE);
    w.view.set_annotations(vec![hl, note]);
    let list = w.view.annotations().get();
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|a| a.id != 0), "loaded annotations get ids");
    let Some(loaded) = list.first() else { unreachable!("the highlight") };
    assert_eq!(loaded.key.as_deref(), Some("12 0 R"), "the app's own key is kept");
    assert!(w.ui.next_undo().is_none(), "loading is not an edit");
    w.frame();
    let reader = w.el.child_elements().into_iter().next().and_then(|c| c.info()).and_then(|i| i.semantics);
    let text = reader.and_then(|s| s.value_text().map(str::to_string)).unwrap_or_default();
    assert!(
        text.contains(
            "1 annotation:\nHighlight: \u{201c}the quick brown fox\u{201d}, comment: Cite this, by Sam"
        ),
        "{text}"
    );
    // Selecting it says so.
    let id = loaded.id;
    w.view.select_annotation(Some(id));
    w.frame();
    let reader = w.el.child_elements().into_iter().next().and_then(|c| c.info()).and_then(|i| i.semantics);
    let text = reader.and_then(|s| s.value_text().map(str::to_string)).unwrap_or_default();
    assert!(text.contains("Selected: Highlight"), "{text}");
    // Enter opens it for the app.
    let opened: Rc<RefCell<Option<u64>>> = Rc::default();
    let o = opened.clone();
    w.view.on_open_annotation(move |a| *o.borrow_mut() = Some(a.id));
    assert!(w.key(Key::Enter, Modifiers::NONE));
    assert_eq!(*opened.borrow(), Some(id));
    // The app edits its comment; undoable.
    assert!(w.view.update_annotation(id, |a| a.comment = "Cite on page 3".into()));
    assert_eq!(w.view.annotation(id).map(|a| a.comment), Some("Cite on page 3".into()));
    assert!(w.ui.undo_change());
    assert_eq!(w.view.annotation(id).map(|a| a.comment), Some("Cite this".into()));
    let problems = remedy_ui_devtools::audit(&w.ui);
    assert!(problems.is_empty(), "{:?}", problems.iter().map(|p| p.message()).collect::<Vec<_>>());
    let _ = Vec2::ZERO;
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&remedy_ui_doc_pages::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn screen_readers_and_undo_hear_the_windows_language() {
    let mut w = win();
    let mut hl = Annotation::new(
        0,
        AnnotationKind::Highlight { quads: vec![Rect::new(72.0, 90.0, 200.0, 104.0)] },
        Color::WHITE,
    );
    hl.marked_text = "the quick brown fox".into();
    hl.comment = "Cite this".into();
    w.view.set_annotations(vec![hl]);
    w.ui.set_locale("de");
    w.frame();
    let sem = w.el.info().and_then(|i| i.semantics);
    let value = sem.and_then(|s| s.value_text().map(str::to_string)).unwrap_or_default();
    assert!(value.starts_with("Seite 1 von "), "{value}");
    let reader = w.el.child_elements().into_iter().next().and_then(|c| c.info()).and_then(|i| i.semantics);
    assert_eq!(reader.as_ref().and_then(|s| s.name().map(str::to_string)).as_deref(), Some("Seite 1"));
    let text = reader.and_then(|s| s.value_text().map(str::to_string)).unwrap_or_default();
    assert!(
        text.contains(
            "1 Anmerkung:\nHervorhebung: \u{201e}the quick brown fox\u{201c}, Kommentar: Cite this"
        ),
        "{text}"
    );
    let id = w.view.annotations().get().first().map_or(0, |a| a.id);
    assert!(w.view.remove_annotation(id));
    assert_eq!(w.ui.next_undo().map(|h| h.label).as_deref(), Some("Hervorhebung löschen"));
}

#[test]
fn an_image_is_placed_moved_resized_by_a_corner_and_undone() {
    let mut w = win();
    let pixels: std::sync::Arc<[u8]> = vec![255u8; 4 * 2 * 2].into();
    let image = remedy_ui_paint::Image::new(2, 2, pixels);
    let rect = Rect::new(100.0, 200.0, 200.0, 250.0);
    let id = w.view.add_annotation(Annotation::new(0, AnnotationKind::Image { rect, image }, Color::WHITE));
    assert_eq!(first(&w).kind_name(), "Image");
    w.view.set_tool(Tool::Select);
    // A click inside selects it; then its bottom-right corner pulls it larger, in shape.
    let inside = at(&w, 150.0, 225.0);
    click(&mut w, inside);
    assert_eq!(w.view.selected_annotation().get(), Some(id));
    let (corner, to) = (at(&w, 200.0, 250.0), at(&w, 300.0, 260.0));
    drag(&mut w, corner, to);
    let r = w.view.annotation(id).and_then(|a| a.rect()).unwrap_or(Rect::ZERO);
    assert_eq!(r.min, rect.min, "the opposite corner stays");
    assert!((r.width() - 200.0).abs() < 2.0 && (r.height() - 100.0).abs() < 1.0, "twice the size, same shape: {r:?}");
    assert_eq!(w.ui.next_undo().map(|h| h.label).as_deref(), Some("Change image"));
    assert!(w.ui.undo_change());
    assert_eq!(w.view.annotation(id).and_then(|a| a.rect()), Some(rect));
    // Dragging its middle moves it.
    let (from, to) = (at(&w, 150.0, 225.0), at(&w, 160.0, 245.0));
    drag(&mut w, from, to);
    let moved = w.view.annotation(id).and_then(|a| a.rect()).unwrap_or(Rect::ZERO);
    assert!((moved.min.x - 110.0).abs() < 1.0 && (moved.min.y - 220.0).abs() < 1.0, "{moved:?}");
    assert_eq!(w.ui.next_undo().map(|h| h.label).as_deref(), Some("Move image"));
}
