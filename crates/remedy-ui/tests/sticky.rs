//! Sticky elements (CSS `position: sticky`) driven through the headless host: they stick
//! while their scroll container scrolls, leave with their parent, follow the nearest scroll
//! container, take clicks where they are drawn, and tell screen readers where they are.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::host::{AccessibilityEvent, HostEvent, WindowId};
use remedy_ui::input::{Buttons, Modifiers, PointerEvent, PointerPhase};
use remedy_ui::layout::Length;
use remedy_ui::reactive::Signal;
use remedy_ui::style::{States, StickyInsets, StylePatch, StyleSheet};
use remedy_ui::view::{El, RichText};
use remedy_ui::{App, AppConfig, GeometryStore, Runner, WindowSpec};
use remedy_ui_core::{Color, Point, Rect, Vec2};
use remedy_ui_platform::bridge_headless::HeadlessHost;

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
const GREEN: Color = Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
const BLUE: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

/// A sheet where `header` sticks to the top (set from the style sheet, not in code), rows
/// are green and the second section's rows blue.
fn config() -> AppConfig {
    AppConfig::new("Sticky test").cpu_only().theme(remedy_ui::style::Theme::dark_forest(), |_| {
        let mut s = StyleSheet::new();
        s.rule(
            "header",
            None,
            States::NONE,
            StylePatch::new().background(RED).sticky(StickyInsets::top(0.0)),
        );
        // Stuck, it turns white (a stand-in for the shadow an app shows).
        s.rule("header", None, States::STUCK, StylePatch::new().background(Color::WHITE));
        s.rule("row", None, States::NONE, StylePatch::new().background(GREEN));
        s.rule("other", None, States::NONE, StylePatch::new().background(BLUE));
        s.rule("glass", None, States::NONE, {
            StylePatch::new()
                .background(Color { r: 1.0, g: 1.0, b: 1.0, a: 0.2 })
                .backdrop_blur(8.0)
                .sticky(StickyInsets::top(0.0))
        });
        s.rule("stripe", None, States::NONE, StylePatch::new().background(Color::BLACK));
        s
    })
}

fn press(x: f32, y: f32, phase: PointerPhase) -> PointerEvent {
    PointerEvent::new(phase, Point::new(x, y))
        .with_buttons(if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() })
        .with_modifiers(Modifiers::NONE)
}

fn px(host: &HeadlessHost, id: WindowId, x: usize, y: usize) -> [u8; 4] {
    let Some(w) = host.window(id) else { return [0; 4] };
    let i = (y * w.size().0 as usize + x) * 4;
    let s = w.surface();
    [0, 1, 2, 3].map(|k| s.get(i + k).copied().unwrap_or(0))
}

fn first(host: &HeadlessHost) -> WindowId {
    host.window_ids().first().copied().unwrap_or(WindowId(0))
}

type Slot = Rc<RefCell<Option<El>>>;

fn take(slot: &Slot) -> El {
    slot.borrow().clone().unwrap_or_else(|| unreachable!("the element was built"))
}

/// A 200 × 200 window scrolling two sections: the first a 30-high sticky header and ten
/// 40-high rows (430 in all), the second twenty rows.
fn page(clicks: Signal<u32>, header: Slot, list: Slot) -> Runner {
    Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (header, list) = (header.clone(), list.clone());
        app.open(WindowSpec::new("Sticky").size(200.0, 200.0), move |ui, _| {
            let h = ui
                .el("header")
                .layout(|l| l.height = Length::Px(30.0))
                .text(RichText::plain("Section header"))
                .on_click(move || clicks.update(|c| *c += 1));
            let mut a = ui.el("section").column().child(h.clone());
            for _ in 0..10 {
                a = a.child(ui.el("row").layout(|l| l.height = Length::Px(40.0)));
            }
            let mut b = ui.el("section").column();
            for _ in 0..20 {
                b = b.child(ui.el("other").layout(|l| l.height = Length::Px(40.0)));
            }
            let scroller = ui.el("scroll").scroll().column().layout(|l| l.grow = 1.0).child(a).child(b);
            *header.borrow_mut() = Some(h);
            *list.borrow_mut() = Some(scroller.clone());
            ui.root().child(scroller);
        });
    })
}

/// Scrolls `list` to `y` with the wheel, as a user does.
fn scroll_to(host: &mut HeadlessHost, runner: &mut Runner, list: &El, y: f32) {
    let id = first(host);
    let ui = list.ui();
    let now = ui.scroll_offset(list.id()).y;
    host.wheel(id, Point::new(150.0, 150.0), Vec2::new(0.0, y - now));
    host.run_frames(runner, 3);
    assert_eq!(ui.scroll_offset(list.id()).y, y, "scrolled to {y}");
}

/// The window bounds the screen reader has for the node labelled `label`.
fn reader_bounds(host: &mut HeadlessHost, id: WindowId, label: &str) -> Option<(f64, f64)> {
    host.take_accessibility(id)
        .iter()
        .flat_map(|u| u.nodes.iter())
        .rev()
        .find(|(_, n)| n.label().is_some_and(|l| l.contains(label)))
        .and_then(|(_, n)| n.bounds())
        .map(|b| (b.y0, b.y1))
}

#[test]
fn a_sticky_header_sticks_takes_clicks_and_leaves_with_its_section() {
    let clicks = Signal::new(0u32);
    let (header, list): (Slot, Slot) = Default::default();
    let mut runner = page(clicks, header.clone(), list.clone());
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let id = first(&host);
    host.send(HostEvent::Accessibility(id, AccessibilityEvent::Requested));
    host.run_frames(&mut runner, 2);
    let (header, list) = (take(&header), take(&list));
    let at = |e: &El| e.bounds().unwrap_or(Rect::ZERO).min.y;
    assert_eq!(at(&header), 0.0, "at the top before scrolling");
    assert!(!header.states().contains(States::STUCK), "in its place: not stuck");
    assert_eq!(reader_bounds(&mut host, id, "Section header"), Some((0.0, 30.0)));

    // Scrolled into the section: the header stays at the top while the rows move under it.
    scroll_to(&mut host, &mut runner, &list, 200.0);
    assert_eq!(at(&header), 0.0, "stuck to the top");
    assert!(header.states().contains(States::STUCK));
    assert_eq!(px(&host, id, 10, 10), [255, 255, 255, 255], "drawn at the top, styled stuck");
    assert_eq!(px(&host, id, 10, 100), [0, 255, 0, 255], "rows scroll under it");
    // The screen reader's bounds and hit testing follow the stuck header: it has not moved,
    // so nothing new is sent for it.
    assert_eq!(reader_bounds(&mut host, id, "Section header"), None);
    host.send(HostEvent::Pointer(id, press(10.0, 10.0, PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, press(10.0, 10.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert_eq!(clicks.get(), 1, "a click on the stuck header reaches it");

    // Near the section's end it is pushed out with it: its bottom stays at the section's
    // (430 - 420 = 10 down the viewport).
    scroll_to(&mut host, &mut runner, &list, 420.0);
    assert_eq!(at(&header), -20.0, "leaves with its parent");
    assert_eq!(px(&host, id, 10, 5), [255, 255, 255, 255], "its last 10 pixels show");
    assert_eq!(px(&host, id, 10, 15), [0, 0, 255, 255], "the next section follows");
    assert_eq!(reader_bounds(&mut host, id, "Section header"), Some((-20.0, 10.0)));
    host.send(HostEvent::Pointer(id, press(10.0, 15.0, PointerPhase::Down)));
    host.send(HostEvent::Pointer(id, press(10.0, 15.0, PointerPhase::Up)));
    host.run_frames(&mut runner, 1);
    assert_eq!(clicks.get(), 1, "below where it went, the click is not the header's");

    // Back at the top it is where layout put it again.
    scroll_to(&mut host, &mut runner, &list, 0.0);
    assert_eq!(at(&header), 0.0);
    assert!(!header.states().contains(States::STUCK), "back in place");
    assert_eq!(px(&host, id, 10, 10), [255, 0, 0, 255]);
}

#[test]
fn a_sticky_element_follows_its_nearest_scroll_container() {
    let (header, inner, outer): (Slot, Slot, Slot) = Default::default();
    let (h2, i2, o2) = (header.clone(), inner.clone(), outer.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (h3, i3, o3) = (h2.clone(), i2.clone(), o2.clone());
        app.open(WindowSpec::new("Nested").size(200.0, 200.0), move |ui, _| {
            // Code rather than the sheet: El::sticky with the top side set.
            let h = ui.el("note").sticky(StickyInsets::top(5.0)).layout(|l| l.height = Length::Px(20.0));
            let mut box_ = ui.el("section").column().child(h.clone());
            for _ in 0..10 {
                box_ = box_.child(ui.el("row").layout(|l| l.height = Length::Px(40.0)));
            }
            let inner = ui.el("scroll").scroll().column().layout(|l| {
                l.height = Length::Px(100.0);
                l.shrink = 0.0;
            });
            let inner = inner.child(box_);
            let mut outer = ui.el("scroll").scroll().column().layout(|l| l.grow = 1.0);
            outer = outer.child(ui.el("other").layout(|l| l.height = Length::Px(50.0))).child(inner.clone());
            for _ in 0..10 {
                outer = outer.child(ui.el("other").layout(|l| l.height = Length::Px(40.0)));
            }
            *h3.borrow_mut() = Some(h);
            *i3.borrow_mut() = Some(inner);
            *o3.borrow_mut() = Some(outer.clone());
            ui.root().child(outer);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let (header, inner, outer) = (take(&header), take(&inner), take(&outer));
    let at = |e: &El| e.bounds().unwrap_or(Rect::ZERO).min.y;
    assert_eq!(at(&inner), 50.0);
    // The inner container scrolls (the wheel over it): the note keeps 5 below its top.
    let id = first(&host);
    host.wheel(id, Point::new(100.0, 100.0), Vec2::new(0.0, 60.0));
    host.run_frames(&mut runner, 3);
    let ui = inner.ui();
    assert_eq!(ui.scroll_offset(inner.id()).y, 60.0, "the inner one took the wheel");
    assert_eq!(at(&header), 55.0, "5 below the inner viewport's top");
    // The outer one scrolling moves the inner viewport and the note with it: the note sticks
    // to the inner container, not to the outer one.
    host.wheel(id, Point::new(100.0, 20.0), Vec2::new(0.0, 30.0));
    host.run_frames(&mut runner, 3);
    assert_eq!(ui.scroll_offset(outer.id()).y, 30.0, "the outer one took the wheel");
    assert_eq!(at(&inner), 20.0);
    assert_eq!(at(&header), 25.0, "moved with the inner viewport");
}

#[test]
fn frosted_glass_on_a_sticky_header_blurs_what_scrolls_under_it() {
    let (header, list): (Slot, Slot) = Default::default();
    let (h2, l2) = (header.clone(), list.clone());
    let mut runner = Runner::with_geometry(config(), GeometryStore::in_memory(), move |app: &App| {
        let (h3, l3) = (h2.clone(), l2.clone());
        app.open(WindowSpec::new("Glass").size(200.0, 200.0), move |ui, _| {
            let h = ui.el("glass").layout(|l| l.height = Length::Px(40.0));
            let mut list = ui.el("scroll").scroll().column().layout(|l| l.grow = 1.0).child(h.clone());
            // Alternating 4-high black and green stripes: blurred, they mix.
            for n in 0..200 {
                let kind = if n % 2 == 0 { "stripe" } else { "row" };
                list = list.child(ui.el(kind).layout(|l| l.height = Length::Px(4.0)));
            }
            *h3.borrow_mut() = Some(h);
            *l3.borrow_mut() = Some(list.clone());
            ui.root().child(list);
        });
    });
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    let (header, list) = (take(&header), take(&list));
    scroll_to(&mut host, &mut runner, &list, 300.0);
    let id = first(&host);
    assert_eq!(header.bounds().unwrap_or(Rect::ZERO).min.y, 0.0, "stuck");
    // Below the header the stripes are sharp; under it they are blurred into one another
    // and lightened by the glass's white tint.
    let sharp: Vec<u8> = (60..68).map(|y| px(&host, id, 100, y)[1]).collect();
    assert!(sharp.contains(&0) && sharp.contains(&255), "sharp stripes below: {sharp:?}");
    let under: Vec<u8> = (16..24).map(|y| px(&host, id, 100, y)[1]).collect();
    assert!(under.iter().all(|g| (60..=230).contains(g)), "blurred under the glass: {under:?}");
    let spread = under.iter().max().copied().unwrap_or(0) - under.iter().min().copied().unwrap_or(0);
    assert!(spread < 40, "the blur evens the stripes out: {under:?}");
}
