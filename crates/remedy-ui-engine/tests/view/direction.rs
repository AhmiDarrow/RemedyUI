//! A window's layout direction: from its locale, or as a person chose; everything inside
//! (content, overlays, painters, horizontal scrolling) follows it.

use std::time::Duration;

use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::layout::Length;
use remedy_ui_core::style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui_core::{Color, Size};
use remedy_ui_engine::compositor::Compositor;
use remedy_ui_engine::render_cpu::{CpuCompositor, Frame};
use remedy_ui_engine::view::{Ui, WindowDirection};
use remedy_ui_text::FontCollection;

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
const BLUE: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

fn ui() -> Ui {
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    sheet.rule("red", None, States::NONE, StylePatch::default().background(RED));
    sheet.rule("blue", None, States::NONE, StylePatch::default().background(BLUE));
    Ui::new(Theme::dark_forest(), sheet, FontCollection::new())
}

struct Screen {
    scrollers: Vec<(remedy_ui_engine::compositor::LayerId, remedy_ui_core::tree::ElementId)>,
    ui: Ui,
    comp: Compositor,
    cpu: CpuCompositor,
    t: u64,
    last: Option<Frame>,
}

impl Screen {
    fn new(ui: Ui) -> Self {
        Screen {
            scrollers: Vec::new(),
            ui,
            comp: Compositor::new(200, 100, 1.0, ScrollPhysics::CLAMPED),
            cpu: CpuCompositor::new(),
            t: 0,
            last: None,
        }
    }

    /// A frame the way the runner runs one: scroll requests reach the compositor, and the
    /// compositor's offsets come back to the view.
    fn frame(&mut self) {
        let f = self.ui.frame(Size::new(200.0, 100.0), 1.0);
        if let Some(l) = f.layers {
            self.comp.commit(l);
        }
        self.scrollers = f.scrollers;
        for r in self.ui.take_requests() {
            if let remedy_ui_engine::view::Request::ScrollTo { scroller, offset } = r
                && let Some((l, _)) = self.scrollers.iter().find(|(_, e)| *e == scroller)
            {
                self.comp.set_scroll_offset(*l, offset);
            }
        }
        for (l, e) in &self.scrollers {
            if let Some(o) = self.comp.scroll_offset(*l) {
                self.ui.set_scroll_offset(*e, o);
            }
        }
        self.t += 16;
        let plan = self.comp.frame(Duration::from_millis(self.t));
        if let Ok(frame) = self.cpu.present(&plan, 200, 100, Color::BLACK) {
            self.last = Some(frame);
        }
    }

    fn px(&self, x: u32, y: u32) -> [u8; 4] {
        self.last.as_ref().and_then(|f| f.pixel(x, y)).unwrap_or([0; 4])
    }
}

fn block(ui: &Ui, widget: &'static str, w: f32) -> remedy_ui_engine::view::El {
    ui.el(widget).layout(|l| {
        l.width = Length::Px(w);
        l.height = Length::Px(40.0);
        l.shrink = 0.0;
    })
}

#[test]
fn the_locale_sets_the_window_direction() {
    let ui = ui();
    let row = ui.el("row").row().child(block(&ui, "red", 50.0)).child(block(&ui, "blue", 50.0));
    let _ = ui.root().child(row);
    let mut s = Screen::new(ui.clone());
    s.frame();
    assert!(!ui.is_rtl());
    assert_eq!(s.px(10, 10), [255, 0, 0, 255], "left to right: the first block at the left");
    for (tag, rtl) in [
        ("ar", true),
        ("ar-EG", true),
        ("he-IL", true),
        ("fa", true),
        ("ur-PK", true),
        ("hi-IN", false),
        ("bn", false),
    ] {
        ui.set_locale(tag);
        s.frame();
        assert_eq!(ui.is_rtl(), rtl, "{tag}");
    }
    ui.set_locale("ar");
    s.frame();
    assert_eq!(s.px(190, 10), [255, 0, 0, 255], "right to left: the first block at the right");
    assert_eq!(s.px(140, 10), [0, 0, 255, 255]);
    assert_eq!(s.px(10, 10), [255, 255, 255, 255]);
    // Back to a left-to-right language.
    ui.set_locale("en-US");
    s.frame();
    assert_eq!(s.px(10, 10), [255, 0, 0, 255]);
}

#[test]
fn a_chosen_direction_outlasts_locale_changes() {
    let ui = ui();
    let _ = ui.root().child(ui.el("row").row().child(block(&ui, "red", 50.0)));
    let mut s = Screen::new(ui.clone());
    ui.set_direction(WindowDirection::RightToLeft);
    s.frame();
    assert!(ui.is_rtl(), "English, laid out right to left on request");
    assert_eq!(s.px(190, 10), [255, 0, 0, 255]);
    ui.set_locale("de");
    s.frame();
    assert!(ui.is_rtl(), "the choice holds across locale changes");
    ui.set_direction(WindowDirection::LeftToRight);
    ui.set_locale("ar");
    s.frame();
    assert!(!ui.is_rtl(), "an Arabic interface kept left to right");
    assert_eq!(s.px(10, 10), [255, 0, 0, 255]);
    ui.set_direction(WindowDirection::Locale);
    s.frame();
    assert!(ui.is_rtl(), "following the locale again");
    assert_eq!(ui.direction(), WindowDirection::Locale);
}

#[test]
fn overlays_and_painters_follow_the_window_direction() {
    let ui = ui();
    let seen = std::rc::Rc::new(std::cell::Cell::new(None));
    let s2 = seen.clone();
    let painted = ui.el("canvas").width(10.0).height(10.0).paint(move |cx| s2.set(Some(cx.rtl)));
    let overlay_row = ui.el("row").row().child(block(&ui, "red", 30.0));
    let _ = ui.overlay(overlay_row.clone());
    let _ = ui.root().child(painted.clone());
    let mut s = Screen::new(ui.clone());
    s.frame();
    assert_eq!(seen.get(), Some(false));
    ui.set_locale("ar-SA");
    s.frame();
    assert_eq!(seen.get(), Some(true), "painters draw again, mirrored");
    assert!(painted.is_rtl() && overlay_row.is_rtl(), "overlays (menus, dialogs, toasts) inherit");
}

#[test]
fn horizontal_scrolling_starts_at_the_start_side() {
    // Content wider than its scroll container: in a right-to-left window it starts at the
    // right edge, and scrolling moves toward the left, so every part can be reached.
    let ui = ui();
    ui.set_locale("ar");
    let wide = ui
        .el("row")
        .row()
        .layout(|l| l.shrink = 0.0)
        .child(block(&ui, "red", 150.0))
        .child(block(&ui, "blue", 150.0));
    let scroller = ui.el("scroll").scroll().scroll_x().row().layout(|l| {
        l.width = Length::Px(200.0);
        l.height = Length::Px(40.0);
    });
    let scroller = scroller.child(wide.clone());
    let _ = ui.root().child(scroller.clone());
    let mut s = Screen::new(ui.clone());
    s.frame();
    s.frame();
    // The first (red) block is at the start: the right edge.
    assert_eq!(s.px(190, 10), [255, 0, 0, 255], "the start of the content shows first");
    let b = wide.bounds().unwrap_or_default();
    assert!(b.width() >= 299.0, "{b:?}");
    // The start shows at the largest offset; scrolling toward zero moves left, to the end.
    let start = s.ui.scroll_offset(scroller.id());
    assert_eq!(start.x, 100.0, "the start is where the container opened");
    scroller.scroll_to(remedy_ui_core::Vec2::new(0.0, 0.0));
    s.frame();
    s.frame();
    assert_eq!(s.px(10, 10), [0, 0, 255, 255], "the far end (blue) is reachable");
    assert_eq!(s.px(140, 10), [0, 0, 255, 255]);
    assert_eq!(s.px(190, 10), [255, 0, 0, 255]);
    let blue = wide.bounds().unwrap_or_default();
    assert!((blue.min.x - 0.0).abs() < 0.5, "hit testing agrees with what is shown: {blue:?}");
    // Content growing to the left keeps what is shown in place.
    scroller.scroll_to(remedy_ui_core::Vec2::new(100.0, 0.0));
    s.frame();
    let _ = wide.clone().child(block(&s.ui.clone(), "blue", 50.0));
    s.frame();
    s.frame();
    assert_eq!(s.ui.scroll_offset(scroller.id()).x, 150.0);
    assert_eq!(s.px(190, 10), [255, 0, 0, 255], "the start still shows");
}
