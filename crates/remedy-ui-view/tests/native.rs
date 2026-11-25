//! What content drawn by the platform or the GPU needs from the view: where its element is
//! (scrolled, clipped, under overlays, hidden), painters that draw every frame, and which
//! renderer and window the view has.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Insets, Size, Vec2};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::layout::Length;
use remedy_ui_render_cpu::CpuCompositor;
use remedy_ui_core::style::{StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{Placement, Request, Ui};

fn ui() -> Ui {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(Theme::dark_forest(), StyleSheet::new(), fonts)
}

type Heard = Rc<RefCell<Vec<Option<Placement>>>>;

fn last(h: &Heard) -> Option<Placement> {
    h.borrow().last().cloned().flatten()
}

#[test]
fn placed_elements_follow_scrolling_clipping_overlays_and_hiding() {
    let ui = ui();
    let heard: Heard = Rc::default();
    let h2 = heard.clone();
    let target = ui
        .el("target")
        .layout(|l| {
            l.height = Length::Px(50.0);
            l.width = Length::Px(80.0);
            l.shrink = 0.0;
        })
        .on_placed(move |p| h2.borrow_mut().push(p.cloned()));
    let list = ui
        .el("list")
        .scroll()
        .layout(|l| {
            l.height = Length::Px(100.0);
            l.display = remedy_ui_core::layout::Display::Flex;
            l.axis = remedy_ui_core::layout::Axis::Column;
        })
        .child(ui.el("spacer").layout(|l| l.height = Length::Px(20.0)))
        .child(target.clone())
        .child(ui.el("rest").layout(|l| l.height = Length::Px(400.0)));
    ui.root().child(list.clone());
    let size = Size::new(200.0, 120.0);
    let _ = ui.frame(size, 2.0);
    let p = last(&heard).unwrap_or_else(|| unreachable!("placed"));
    assert_eq!((p.bounds.min.y, p.bounds.max.y), (20.0, 70.0));
    assert_eq!(p.visible, p.bounds, "all of it shows");
    assert!(p.covered.is_empty());
    assert_eq!(p.scale, 2.0);
    let _ = ui.frame(size, 2.0);
    assert_eq!(heard.borrow().len(), 1, "told only when it changes");
    // Scrolled partly out of the scroll container: the box moves, what shows is cut.
    ui.set_scroll_offset(list.id(), Vec2::new(0.0, 40.0));
    let _ = ui.frame(size, 2.0);
    let p = last(&heard).unwrap_or_else(|| unreachable!("placed"));
    assert_eq!((p.bounds.min.y, p.bounds.max.y), (-20.0, 30.0));
    assert_eq!((p.visible.min.y, p.visible.max.y), (0.0, 30.0));
    // An overlay over it is reported, cut to what shows.
    let menu = ui.overlay(ui.el("menu").layout(|l| {
        l.position = remedy_ui_core::layout::Position::Absolute;
        l.inset = Insets { top: 10.0, start: 30.0, bottom: 0.0, end: 0.0 };
        l.width = Length::Px(40.0);
        l.height = Length::Px(60.0);
    }));
    let _ = ui.frame(size, 2.0);
    let p = last(&heard).unwrap_or_else(|| unreachable!("placed"));
    assert_eq!(p.covered.len(), 1, "{p:?}");
    let c = p.covered.first().copied().unwrap_or_else(|| unreachable!("covered"));
    assert_eq!((c.min.x, c.min.y, c.max.x, c.max.y), (30.0, 10.0, 70.0, 30.0));
    menu.remove();
    let _ = ui.frame(size, 2.0);
    assert!(last(&heard).is_some_and(|p| p.covered.is_empty()), "the menu closed");
    // Scrolled out of sight, then hidden: nothing to place.
    ui.set_scroll_offset(list.id(), Vec2::new(0.0, 200.0));
    let _ = ui.frame(size, 2.0);
    assert_eq!(heard.borrow().last(), Some(&None));
    ui.set_scroll_offset(list.id(), Vec2::ZERO);
    let _ = ui.frame(size, 2.0);
    assert!(last(&heard).is_some());
    let _ = target.hidden(true);
    let _ = ui.frame(size, 2.0);
    assert_eq!(heard.borrow().last(), Some(&None));
}

#[test]
fn a_painter_can_ask_to_draw_again_next_frame() {
    let ui = ui();
    let painted = Rc::new(Cell::new(0u32));
    let keep_going = Rc::new(Cell::new(true));
    let (p2, k2) = (painted.clone(), keep_going.clone());
    ui.root().child(ui.el("spinner").layout(|l| l.height = Length::Px(20.0)).paint(move |cx| {
        p2.set(p2.get() + 1);
        if k2.get() {
            cx.request_next_frame();
        }
    }));
    let mut comp = Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    let mut t = 0;
    let mut frame = || {
        let f = ui.frame(Size::new(200.0, 120.0), 1.0);
        if let Some(l) = f.layers {
            comp.commit(l);
        }
        t += 16;
        let plan = comp.frame(Duration::from_millis(t));
        let _ = cpu.present(&plan, 200, 120, Color::BLACK);
    };
    frame();
    assert_eq!(painted.get(), 1);
    assert!(ui.take_requests().contains(&Request::Frame), "asks for the next frame");
    frame();
    frame();
    assert_eq!(painted.get(), 3, "drawn every frame while it asks");
    keep_going.set(false);
    frame();
    let _ = ui.take_requests();
    frame();
    frame();
    assert_eq!(painted.get(), 4, "stops when it stops asking");
    assert!(!ui.take_requests().contains(&Request::Frame));
}

#[test]
fn the_view_knows_its_renderer_and_window() {
    let ui = ui();
    assert!(ui.window_handles().is_none(), "no native window until the runner says");
    let gpu = ui.gpu_rendering();
    assert!(!gpu.get());
    ui.set_gpu_rendering(true);
    assert!(gpu.get());
    let _ = ui.take_requests();
    ui.prefer_gpu();
    assert!(ui.take_requests().contains(&Request::PreferGpu));
}
