//! The page view on a real view: rendering only what shows, zooming around the pointer by
//! wheel, pinch, touch and keyboard, stand-ins while a zoom moves, cancellation, memory,
//! selection and copy, search, spreads and rotation, navigation, thumbnails and access.

use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::a11y::Role;
use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Point, Size};
use remedy_ui_doc_pages::*;
use remedy_ui_core::input::{
    Buttons, Key, Modifiers, PinchEvent, PinchPhase, PointerKind, PointerPhase, ScrollPhysics,
};
use remedy_ui_render_cpu::{CpuCompositor, Frame};

mod common;
use common::*;

const WINDOW: Size = Size { width: 800.0, height: 600.0 };
const LONG: Duration = Duration::from_secs(20);

fn doc(pages: usize) -> Arc<Synthetic> {
    Arc::new(Synthetic::new(pages))
}

/// What the window shows, through the compositor and the CPU renderer.
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

fn near(a: [u8; 4], c: Color) -> bool {
    let [r, g, b, _] = c.to_rgba8();
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).abs();
    d(a[0], r) <= 3 && d(a[1], g) <= 3 && d(a[2], b) <= 3
}

/// The effective zoom: page width on screen over its width in points.
fn zoom_of(w: &Win, page: usize) -> f32 {
    w.view.page_rect(page).width() / LETTER.width
}

/// The page point under a point of the view.
fn page_point(w: &Win, page: usize, p: Point) -> Point {
    let r = w.view.page_rect(page);
    let z = zoom_of(w, page);
    Point::new((p.x - r.min.x) / z, (p.y - r.min.y) / z)
}

#[test]
fn the_first_frames_render_only_what_shows_then_are_sharp() {
    let src = doc(300);
    let mut w = Win::new(src.clone(), WINDOW);
    assert!(w.settle(LONG), "the view sharpens: {:?}", w.view.stats());
    let s = w.view.stats();
    // Fit width: one page 768 pixels wide, its top 584 rows on screen: 3 × 3 tiles of 256,
    // a preview, and a ring of prefetch on that page.
    assert!(s.rendered <= 20, "only what shows renders: {s:?}");
    assert!((zoom_of(&w, 0) - 768.0 / 612.0).abs() < 0.003, "fitted to the width");
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(0));
    let f = snapshot(&w);
    let header = f.pixel(300, 100).unwrap_or([0; 4]);
    assert!(near(header, header_color(0)), "page 1's header band shows: {header:?}");
}

#[test]
fn ctrl_wheel_zooms_around_the_pointer() {
    let mut w = Win::new(doc(300), WINDOW);
    assert!(w.settle(LONG));
    let at = Point::new(250.0, 200.0);
    let before = page_point(&w, 0, at);
    assert!(w.wheel(at, -48.0 * 3.0, ctrl()), "Ctrl+wheel is the view's");
    let z = zoom_of(&w, 0);
    assert!((z / (768.0 / 612.0) - 1.1f32.powi(3)).abs() < 0.01, "three notches, 10% each: {z}");
    let after = page_point(&w, 0, at);
    assert!((after.x - before.x).abs() < 0.5 && (after.y - before.y).abs() < 0.5, "{before:?} → {after:?}");
    // Without Ctrl the wheel scrolls.
    let y = w.view.scroll_offset().y;
    assert!(w.wheel(at, 100.0, Modifiers::NONE));
    assert!((w.view.scroll_offset().y - y - 100.0).abs() < 0.01);
    assert!(w.settle(LONG));
}

#[test]
fn keyboard_zooms_by_steps_and_navigates() {
    let mut w = Win::new(doc(300), WINDOW);
    assert!(w.key(Key::Character("=".into()), ctrl()));
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(1.5), "the next step above 125%");
    assert!(w.key(Key::Character("-".into()), ctrl()));
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(1.25));
    assert!(w.key(Key::Character("0".into()), ctrl()));
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(1.0));
    assert_eq!(w.view.fit().get_untracked().ok(), Some(Fit::Free));
    assert!(w.key(Key::End, Modifiers::NONE));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(299));
    assert!(w.key(Key::Home, Modifiers::NONE));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(0));
    let y = w.view.scroll_offset().y;
    assert!(w.key(Key::PageDown, Modifiers::NONE));
    assert!((w.view.scroll_offset().y - y - 540.0).abs() < 0.5, "a screen less a margin");
    assert!(w.key(Key::ArrowDown, Modifiers::NONE));
    w.view.go_to_page(41);
    w.frame();
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(41));
    assert!(w.view.page_rect(41).min.y.abs() <= 16.5, "page 42 at the top");
    // Zoom limits are 1% and 6400%.
    w.view.set_zoom(1000.0, None);
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(MAX_ZOOM));
    w.view.set_zoom(0.0001, None);
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(MIN_ZOOM));
    // The app moves them; a zoom outside the new range moves inside it.
    w.view.set_zoom_range(0.5, 200.0);
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(0.5));
    w.view.set_zoom(1000.0, None);
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(200.0));
}

#[test]
fn pinches_zoom_around_the_fingers() {
    let mut w = Win::new(doc(20), WINDOW);
    assert!(w.settle(LONG));
    let at = Point::new(500.0, 300.0);
    let before = page_point(&w, 0, at);
    let z0 = zoom_of(&w, 0);
    for (phase, scale) in [(PinchPhase::Start, 1.0), (PinchPhase::Change, 1.5), (PinchPhase::End, 1.0)] {
        assert!(w.ui.pinch(&PinchEvent::new(phase, at, scale)));
    }
    w.frame();
    assert!((zoom_of(&w, 0) / z0 - 1.5).abs() < 0.01);
    let after = page_point(&w, 0, at);
    assert!((after.x - before.x).abs() < 0.5 && (after.y - before.y).abs() < 0.5);
    // Two fingers on a touch screen: spreading them to twice the distance doubles the zoom.
    let z1 = zoom_of(&w, 0);
    let (a, b) = (Point::new(300.0, 300.0), Point::new(500.0, 300.0));
    w.pointer(1, PointerKind::Touch, PointerPhase::Down, a, Buttons::PRIMARY);
    w.pointer(2, PointerKind::Touch, PointerPhase::Down, b, Buttons::PRIMARY);
    w.pointer(1, PointerKind::Touch, PointerPhase::Move, Point::new(200.0, 300.0), Buttons::PRIMARY);
    w.pointer(2, PointerKind::Touch, PointerPhase::Move, Point::new(600.0, 300.0), Buttons::PRIMARY);
    w.pointer(1, PointerKind::Touch, PointerPhase::Up, Point::new(200.0, 300.0), Buttons::default());
    w.pointer(2, PointerKind::Touch, PointerPhase::Up, Point::new(600.0, 300.0), Buttons::default());
    assert!((zoom_of(&w, 0) / z1 - 2.0).abs() < 0.02, "{} → {}", z1, zoom_of(&w, 0));
    // One finger pans.
    let y = w.view.scroll_offset().y;
    w.pointer(3, PointerKind::Touch, PointerPhase::Down, Point::new(400.0, 400.0), Buttons::PRIMARY);
    w.pointer(3, PointerKind::Touch, PointerPhase::Move, Point::new(400.0, 300.0), Buttons::PRIMARY);
    w.pointer(3, PointerKind::Touch, PointerPhase::Up, Point::new(400.0, 300.0), Buttons::default());
    assert!((w.view.scroll_offset().y - y - 100.0).abs() < 0.5);
}

#[test]
fn while_a_zoom_moves_the_compositor_scales_the_picture_then_old_tiles_stand_in() {
    let mut slow = Synthetic::new(300);
    slow.delay = Duration::from_millis(60);
    let mut w = Win::new(Arc::new(slow), WINDOW);
    assert!(w.settle(LONG));
    let requested = w.view.stats().requested;
    for _ in 0..3 {
        let _ = w.wheel(Point::new(400.0, 300.0), -48.0, ctrl());
    }
    // The picture stays; its layer is scaled on the compositor (which re-rasters it
    // progressively), and nothing renders while the zoom moves.
    let k = layer_scale(&w);
    assert!((k - 1.1f32.powi(3)).abs() < 0.01, "the view's layer is scaled by the zoom: {k}");
    assert_eq!(w.view.stats().requested, requested, "nothing renders while the zoom moves");
    assert!(!w.view.is_sharp());
    // Once it settles the picture is redrawn at the new zoom: tiles of the old scale stand in
    // for the new ones until they render, so no tile is ever blank.
    let start = std::time::Instant::now();
    while (layer_scale(&w) - 1.0).abs() > 1e-6 {
        assert!(start.elapsed() < LONG);
        std::thread::sleep(Duration::from_millis(5));
        w.frame();
    }
    let s = w.view.stats();
    assert!(s.standing_in > 0 && s.blank == 0, "{s:?}");
    assert!(s.requested > requested, "the exact scale renders now");
    assert!(w.settle(LONG), "and then the view is sharp: {:?}", w.view.stats());
}

/// The scale of the view's own layer on the compositor (1 when not zooming).
fn layer_scale(w: &Win) -> f32 {
    let mut k = 1.0;
    if let Some(t) = &w.last {
        t.for_each(|l| {
            if !l.transform.is_translation() {
                k = l.transform.scale_factor();
            }
        });
    }
    k
}

#[test]
fn work_for_pages_scrolled_away_is_cancelled() {
    let mut slow = Synthetic::new(300);
    slow.delay = Duration::from_millis(40);
    let src = Arc::new(slow);
    let mut w = Win::new(src.clone(), WINDOW);
    for page in [50, 100, 150, 200, 250] {
        w.view.go_to_page(page);
        w.frame();
    }
    assert!(w.settle(LONG));
    let s = w.view.stats();
    assert!(s.cancelled > 0, "{s:?}");
    let rendered = src.rendered.load(std::sync::atomic::Ordering::Relaxed);
    assert!(rendered < s.requested, "cancelled renders stopped early: {rendered} of {}", s.requested);
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(250));
}

#[test]
fn the_cache_is_sized_from_the_screen_and_lets_go_under_pressure() {
    let mut w = Win::new(doc(300), WINDOW);
    assert!(w.settle(LONG));
    for page in 1..12 {
        w.view.go_to_page(page);
        assert!(w.settle(LONG));
    }
    let s = w.view.stats();
    assert!(s.cache_budget >= 3 * 800 * 600 * 4, "at least three screens: {s:?}");
    assert!(s.cached_bytes <= s.cache_budget.max(1));
    let before = s.cached_tiles;
    remedy_ui_view::device::memory_pressure(remedy_ui_view::device::MemoryPressure::Moderate);
    let s = w.view.stats();
    assert!(s.cached_tiles < before, "tiles off screen are freed: {before} → {}", s.cached_tiles);
    w.frame();
    assert!(w.view.is_sharp(), "what shows stays");
}

#[test]
fn dragging_selects_text_and_ctrl_c_copies_it() {
    let mut w = Win::new(doc(5), WINDOW);
    assert!(w.settle(LONG));
    for _ in 0..20 {
        w.frame();
    }
    let r = w.view.page_rect(0);
    let z = zoom_of(&w, 0);
    let to_view = |x: f32, y: f32| Point::new(r.min.x + x * z, r.min.y + y * z);
    let first = line_rect(0, &line_text(0, 0));
    let second = line_rect(1, &line_text(0, 1));
    let start = to_view(first.min.x + 1.0, first.center().y);
    // After "Page 1 line" on the second line (11 characters of 7 points).
    let end = to_view(second.min.x + 11.0 * 7.0 - 1.0, second.center().y);
    w.pointer(0, PointerKind::Mouse, PointerPhase::Down, start, Buttons::PRIMARY);
    w.pointer(0, PointerKind::Mouse, PointerPhase::Move, end, Buttons::PRIMARY);
    w.pointer(0, PointerKind::Mouse, PointerPhase::Up, end, Buttons::default());
    assert_eq!(w.view.selected_text().as_deref(), Some("Page 1 line 1 the quick brown fox\nPage 1 line"));
    assert!(w.key(Key::Character("c".into()), ctrl()));
    assert_eq!(w.copied.last().map(String::as_str), Some("Page 1 line 1 the quick brown fox\nPage 1 line"));
    assert!(w.key(Key::Character("a".into()), ctrl()));
    let all = w.view.selected_text().unwrap_or_default();
    assert_eq!(all.lines().count(), LINES, "Ctrl+A selects the page");
    assert!(w.key(Key::Escape, Modifiers::NONE));
    assert_eq!(w.view.selected_text(), None);
}

#[test]
fn search_finds_every_page_off_the_ui_thread_and_steps_through_hits() {
    let mut w = Win::new(doc(300), WINDOW);
    w.view.find("LINE 7 THE");
    let start = std::time::Instant::now();
    while !w.view.search_status().get_untracked().is_ok_and(|s| s.done) {
        assert!(start.elapsed() < LONG, "{:?}", w.view.search_status().get_untracked());
        w.frame();
        std::thread::sleep(Duration::from_millis(1));
    }
    let s = w.view.search_status().get_untracked().unwrap_or_default();
    assert_eq!(s.matches, 300, "one hit a page, case ignored");
    assert_eq!(s.current, Some(0));
    w.view.find_next();
    w.frame();
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(1), "the next hit is shown");
    assert_eq!(w.view.selected_text().as_deref(), Some("line 7 the"));
    w.view.find_previous();
    w.view.find_previous();
    w.frame();
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(299), "wrapping");
    w.view.find("");
    assert_eq!(w.view.matches().len(), 0);
}

#[test]
fn spreads_and_rotation_lay_pages_out() {
    let mut w = Win::new(doc(9), WINDOW);
    w.view.set_spread(Spread::Two);
    w.frame();
    let (a, b) = (w.view.page_rect(0), w.view.page_rect(1));
    assert_eq!(a.min.y, b.min.y, "facing pages");
    assert!(b.min.x > a.max.x);
    assert!(a.max.x <= WINDOW.width / 2.0 && b.min.x >= WINDOW.width / 2.0, "the pair fits the width");
    w.view.set_spread(Spread::Single);
    w.view.set_rotation(Rotation::Clockwise);
    w.frame();
    let r = w.view.page_rect(0);
    assert!(r.width() > r.height(), "turned on its side: {r:?}");
    assert!(w.settle(LONG));
    // The header band of page 1 is now down the right-hand side.
    let f = snapshot(&w);
    let px =
        f.pixel((r.max.x - 60.0 * zoom_of_rotated(&w)) as u32, (r.min.y + 300.0) as u32).unwrap_or([0; 4]);
    assert!(near(px, header_color(0)), "{px:?}");
}

fn zoom_of_rotated(w: &Win) -> f32 {
    w.view.page_rect(0).width() / LETTER.height
}

#[test]
fn the_view_is_a_document_with_its_page_for_screen_readers() {
    let mut w = Win::new(doc(300), WINDOW);
    assert!(w.settle(LONG));
    for _ in 0..20 {
        w.frame();
    }
    let info = w.el.info();
    let sem = info.and_then(|i| i.semantics);
    assert_eq!(sem.as_ref().map(|s| s.role()), Some(Role::Document));
    let value = sem.as_ref().and_then(|s| s.value_text().map(str::to_string)).unwrap_or_default();
    assert!(value.starts_with("Page 1 of 300"), "{value}");
    let reader = w.el.child_elements().into_iter().next().and_then(|c| c.info()).and_then(|i| i.semantics);
    assert_eq!(reader.as_ref().map(|s| s.role()), Some(Role::Page));
    let text = reader.and_then(|s| s.value_text().map(str::to_string)).unwrap_or_default();
    assert!(text.starts_with("Page 1 line 1 the quick brown fox\nPage 1 line 2"), "{text}");
    let problems = remedy_ui_devtools::audit(&w.ui);
    assert!(problems.is_empty(), "{:?}", problems.iter().map(|p| p.message()).collect::<Vec<_>>());
}

#[test]
fn a_page_that_fails_to_draw_says_why() {
    let mut broken = Synthetic::new(3);
    broken.broken = Some(0);
    let mut w = Win::new(Arc::new(broken), WINDOW);
    let start = std::time::Instant::now();
    while w.view.page_error(0).is_none() {
        assert!(start.elapsed() < LONG);
        w.frame();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(w.view.page_error(0).as_deref(), Some("damaged page"));
    assert_eq!(w.view.page_error(1), None);
}

#[test]
fn thumbnails_show_pages_and_take_the_user_there() {
    let ui = ui();
    let view = PageView::new(doc(300));
    let rail = thumbnails(&ui, &view);
    let shown = page_view(&ui, &view, "Synthetic.pdf");
    let row = ui.el("row").row().height(WINDOW.height).child(rail.clone()).child(shown);
    ui.root().child(row);
    let frame = |n: usize| {
        for _ in 0..n {
            let _ = remedy_ui_core::reactive::run_posted();
            let _ = ui.frame(WINDOW, 1.0);
            // Scroll requests are applied the way the runner applies them.
            for r in ui.take_requests() {
                if let remedy_ui_view::Request::ScrollTo { scroller, offset } = r {
                    ui.set_scroll_offset(scroller, offset);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    frame(30);
    let rows = |rail: &remedy_ui_view::El| -> Vec<remedy_ui_view::El> {
        let list = rail.child_elements().into_iter().next();
        list.and_then(|l| l.child_elements().get(1).map(|c| c.child_elements())).unwrap_or_default()
    };
    let list = rows(&rail);
    assert!(!list.is_empty() && list.len() < 20, "a few rows exist: {}", list.len());
    let third = list.get(2).cloned();
    let sem = third.as_ref().and_then(|r| r.info()).and_then(|i| i.semantics);
    assert_eq!(sem.as_ref().and_then(|s| s.name().map(str::to_string)).as_deref(), Some("Page 3"));
    // Clicking a thumbnail goes to its page, and it becomes the selected one.
    let at = third.as_ref().and_then(|r| r.bounds()).map(|b| b.center()).unwrap_or_default();
    for phase in [PointerPhase::Down, PointerPhase::Up] {
        let buttons = if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() };
        ui.pointer(&remedy_ui_core::input::PointerEvent::new(phase, at).with_buttons(buttons));
    }
    frame(5);
    assert_eq!(view.current_page().get_untracked().ok(), Some(2));
    let selected = third.and_then(|r| r.info()).and_then(|i| i.semantics).and_then(|s| s.selected_state());
    assert_eq!(selected, Some(true));
    // With the rail focused, the keys move through the pages and the rail follows.
    if let Some(list) = rail.child_elements().into_iter().next() {
        list.focus();
    }
    for _ in 0..10 {
        let _ = ui.key(&remedy_ui_core::input::KeyEvent::press(Key::ArrowDown));
        frame(1);
    }
    frame(5);
    assert_eq!(view.current_page().get_untracked().ok(), Some(12));
    let names: Vec<String> = rows(&rail)
        .iter()
        .filter_map(|r| r.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_string)))
        .collect();
    assert!(names.iter().any(|n| n == "Page 13"), "the rail scrolled to page 13: {names:?}");
}

#[test]
fn thumbnails_and_page_view_size_themselves_inside_plain_containers() {
    let ui = ui();
    let view = PageView::new(doc(300));
    // Nothing here sets a width, a grow or a minimum height on the two widgets: a bar, then
    // a row of the rail and the page view, in a column of the window's size.
    let rail = thumbnails(&ui, &view);
    let shown = page_view(&ui, &view, "Synthetic.pdf");
    let bar = ui.el("bar").height(40.0).fill_width();
    let row =
        ui.el("row").row().fill_width().layout(|l| l.grow = 1.0).child(rail.clone()).child(shown.clone());
    let column = ui.el("column").column().width(WINDOW.width).height(WINDOW.height).child(bar).child(row);
    ui.root().child(column);
    for _ in 0..10 {
        let _ = remedy_ui_core::reactive::run_posted();
        let _ = ui.frame(WINDOW, 1.0);
        let _ = ui.take_requests();
        std::thread::sleep(Duration::from_millis(1));
    }
    let Some(rb) = rail.bounds() else { unreachable!("the rail was laid out") };
    let Some(pb) = shown.bounds() else { unreachable!("the page view was laid out") };
    assert!((rb.width() - 144.0).abs() < 0.5, "the rail has its natural strip width: {}", rb.width());
    assert!(
        (pb.width() - (WINDOW.width - rb.width())).abs() < 0.5,
        "the page view takes the rest: {}",
        pb.width()
    );
    let rest = WINDOW.height - 40.0;
    assert!((rb.height() - rest).abs() < 0.5, "the rail fills the height it is given: {}", rb.height());
    assert!((pb.height() - rest).abs() < 0.5, "the page view fills the height it is given: {}", pb.height());
}

#[test]
fn the_vertical_scroll_bar_sits_on_the_end_side() {
    // The page viewer's own scroll bars: the thumb starts at the top of its track.
    let mut w = Win::new(doc(40), WINDOW);
    assert!(w.settle(LONG));
    let ltr = snapshot(&w);
    let thumb = |f: &Frame, x: u32| f.pixel(x, 20).unwrap_or([0; 4]);
    let right_ltr = thumb(&ltr, 795);
    let left_ltr = thumb(&ltr, 5);
    assert_ne!(right_ltr, left_ltr, "left to right, the thumb is at the right edge");
    w.ui.set_locale("ar");
    w.frame();
    w.frame();
    assert!(w.settle(LONG));
    let rtl = snapshot(&w);
    assert_eq!(thumb(&rtl, 5), right_ltr, "right to left, the same thumb at the left edge");
    assert_eq!(thumb(&rtl, 795), left_ltr);
}

#[test]
fn paged_mode_shows_one_page_and_turns_at_its_edges() {
    let mut w = Win::new(doc(5), WINDOW);
    w.view.set_fit(Fit::Page);
    w.view.set_page_mode(PageMode::Paged);
    w.frame();
    let at = Point::new(400.0, 300.0);
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(0));
    // The page fits, so there is nothing to scroll: a notch of the wheel turns it.
    assert!(w.wheel(at, 48.0, Modifiers::NONE));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(1));
    assert!(w.key(Key::PageDown, Modifiers::NONE));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(2));
    assert!(w.key(Key::ArrowUp, Modifiers::NONE));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(1));
    assert!(w.view.next_page() && w.view.next_page() && w.view.next_page());
    assert!(!w.view.next_page(), "the last page");
    assert!(!w.key(Key::PageDown, Modifiers::NONE), "nowhere to go: the key is not the view's");
    // Zoomed in, the page scrolls inside itself before it turns back.
    w.view.set_zoom(w.view.zoom().get_untracked().unwrap_or(1.0) * 3.0, None);
    let y = w.view.scroll_offset().y;
    assert!(w.key(Key::ArrowUp, Modifiers::NONE));
    assert!(w.view.scroll_offset().y < y);
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(4));
    w.view.set_page_mode(PageMode::Continuous);
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(4));
}

#[test]
fn the_app_sets_wheel_zoom_and_shortcuts_and_hears_taps() {
    let mut w = Win::new(doc(3), WINDOW);
    w.view.set_zoom(1.0, None);
    w.view.set_input(PageInput {
        wheel_zoom: Some(ZoomStep::Add(0.01)),
        wheel_zoom_shift: Some(ZoomStep::Add(0.15)),
        reset_zoom: 1.25,
        ..PageInput::default()
    });
    let at = Point::new(400.0, 300.0);
    assert!(w.wheel(at, -48.0, ctrl()));
    assert!((w.view.zoom().get_untracked().unwrap_or(0.0) - 1.01).abs() < 1e-4);
    assert!(w.wheel(at, -48.0, ctrl().with_shift()));
    assert!((w.view.zoom().get_untracked().unwrap_or(0.0) - 1.16).abs() < 1e-4);
    assert!(w.key(Key::Character("0".into()), ctrl()));
    assert_eq!(w.view.zoom().get_untracked().ok(), Some(1.25));
    // A view told to leave the zoom wheel alone does.
    w.view.set_input(PageInput { wheel_zoom: None, ..PageInput::default() });
    assert!(!w.wheel(at, -48.0, ctrl()));
    // Shortcuts move, and a command runs from code.
    w.view.rebind(
        PageCommand::NextPage,
        vec![remedy_ui_core::input::Chord::new(Modifiers::NONE, Key::Character("n".into()))],
    );
    w.view.rebind(PageCommand::LastPage, Vec::new());
    assert!(w.key(Key::Character("n".into()), Modifiers::NONE));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(1));
    assert!(!w.key(Key::End, Modifiers::NONE), "unbound");
    assert!(w.view.run(PageCommand::FirstPage));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(0));
    // A click that does not drag is a tap, with the page under it.
    let taps = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let t = taps.clone();
    w.view.on_tap(move |hit| t.borrow_mut().push(hit.page.map(|(page, _)| page)));
    w.frame();
    w.pointer(0, PointerKind::Mouse, PointerPhase::Down, at, Buttons::PRIMARY);
    w.pointer(0, PointerKind::Mouse, PointerPhase::Up, at, Buttons::default());
    assert_eq!(*taps.borrow(), vec![Some(0)]);
}

#[test]
fn changed_pages_render_again_and_a_new_document_keeps_the_place() {
    let mut w = Win::new(doc(300), WINDOW);
    w.view.go_to_page(7);
    assert!(w.settle(LONG));
    let (zoom, scroll) = (w.view.zoom().get_untracked().ok(), w.view.scroll_offset());
    let before = w.view.stats();
    w.view.pages_changed(&[7]);
    assert!(w.view.stats().cached_tiles < before.cached_tiles, "page 8's tiles are gone");
    assert!(w.settle(LONG));
    assert!(w.view.stats().rendered > before.rendered, "and drawn again");
    assert_eq!((w.view.zoom().get_untracked().ok(), w.view.scroll_offset()), (zoom, scroll));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(7));
    // Another document with fewer pages: the view stays as near as it can.
    w.view.set_source(doc(3));
    assert_eq!(w.view.page_count(), 3);
    assert_eq!(w.view.page_count_signal().get_untracked().ok(), Some(3));
    assert!(w.settle(LONG));
    assert_eq!(w.view.current_page().get_untracked().ok(), Some(2));
    assert!(w.view.is_sharp());
}

#[test]
fn a_page_filter_recolours_the_pages_and_nothing_around_them() {
    let mut w = Win::new(doc(300), WINDOW);
    assert!(w.settle(LONG));
    let plain = snapshot(&w);
    // The panel shows in the margin left of the page; the header band is page pixels.
    let (panel, header) = (plain.pixel(4, 300), plain.pixel(300, 100).unwrap_or([0; 4]));
    w.view.set_page_filter(Some(remedy_ui_core::ColorMatrix::invert(1.0)));
    w.frame();
    assert!(w.settle(LONG));
    let inverted = snapshot(&w);
    assert_eq!(inverted.pixel(4, 300), panel, "the panel keeps its colour");
    let now = inverted.pixel(300, 100).unwrap_or([0; 4]);
    let d = |a: u8, b: u8| (i32::from(a) - (255 - i32::from(b))).abs();
    assert!(
        d(now[0], header[0]) <= 3 && d(now[1], header[1]) <= 3 && d(now[2], header[2]) <= 3,
        "{header:?} → {now:?}"
    );
    // A matrix that changes nothing is no filter.
    w.view.set_page_filter(Some(remedy_ui_core::ColorMatrix::IDENTITY));
    assert_eq!(w.view.page_filter(), None);
}
