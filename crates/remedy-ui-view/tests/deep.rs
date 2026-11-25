//! Element trees of any depth: built, laid out, painted, composited, read by assistive
//! technology, hit, tabbed through and removed on a test thread's stack (2 MB), in time
//! that grows with the tree's size, not its square.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Point, Size};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase, ScrollPhysics};
use remedy_ui_core::layout::Length;
use remedy_ui_render_cpu::CpuCompositor;
use remedy_ui_core::style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };

fn ui() -> Ui {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    sheet.rule("level", None, States::NONE, StylePatch::default().background(RED));
    sheet.rule("faded", None, States::NONE, StylePatch::default().opacity(0.999));
    Ui::new(Theme::dark_forest(), sheet, fonts)
}

/// `n` levels over a clickable, focusable leaf: clips, faded groups, focusable levels and
/// now and then a scroll container (a compositor layer of its own).
fn deep(ui: &Ui, n: usize, clicks: Rc<Cell<u32>>) -> El {
    let mut el =
        ui.el("level").text("deepest").focusable(true).on_click(move || clicks.set(clicks.get() + 1)).layout(
            |l| {
                l.width = Length::Px(60.0);
                l.height = Length::Px(20.0);
            },
        );
    for k in 0..n {
        let level = match k % 4 {
            0 => ui.el("level").clip(),
            1 => ui.el("faded"),
            2 => ui.el("level").focusable(true),
            _ if k % 1000 == 3 => ui.el("level").scroll(),
            _ => ui.el("level"),
        };
        el = level.child(el);
    }
    el
}

fn present(ui: &Ui, comp: &mut Compositor, cpu: &mut CpuCompositor, t: u64) -> Option<[u8; 4]> {
    let f = ui.frame(Size::new(200.0, 120.0), 1.0);
    if let Some(layers) = f.layers {
        comp.commit(layers);
    }
    let plan = comp.frame(Duration::from_millis(t));
    // A corner of the leaf its text does not reach.
    cpu.present(&plan, 200, 120, Color::BLACK).ok().and_then(|f| f.pixel(57, 18))
}

#[test]
fn element_trees_of_any_depth_work_end_to_end() {
    const N: usize = 20_000;
    let started = Instant::now();
    let ui = ui();
    ui.connect_accessibility();
    let clicks = Rc::new(Cell::new(0));
    let top = deep(&ui, N, clicks.clone());
    ui.root().child(top.clone());
    let (mut comp, mut cpu) = (Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED), CpuCompositor::new());
    let px = present(&ui, &mut comp, &mut cpu, 0);
    assert!(px.is_some_and(|[r, g, b, _]| r > 200 && g < 60 && b < 60), "the levels drawn: {px:?}");
    let nodes = ui.take_accessibility_update().map(|u| u.nodes.len());
    assert!(nodes.is_some_and(|n| n > N), "every level reaches assistive technology: {nodes:?}");

    // The pointer reaches the deepest element through every level.
    let at = Point::new(5.0, 5.0);
    ui.pointer(&PointerEvent::new(PointerPhase::Move, at));
    ui.pointer(&PointerEvent::new(PointerPhase::Down, at).with_buttons(Buttons::PRIMARY));
    ui.pointer(&PointerEvent::new(PointerPhase::Up, at));
    assert_eq!(clicks.get(), 1, "a click on the deepest element");
    // Focus moves through them, both ways.
    for _ in 0..3 {
        ui.key(&KeyEvent::press(Key::Tab));
    }
    ui.key(&KeyEvent::press(Key::Tab).with_modifiers(Modifiers { shift: true, ..Modifiers::NONE }));
    let _ = present(&ui, &mut comp, &mut cpu, 16);
    let _ = ui.take_accessibility_update();

    // Removed, the window is empty again.
    top.remove();
    let px = present(&ui, &mut comp, &mut cpu, 32);
    assert_eq!(px, Some([255, 255, 255, 255]), "only the window's background is left");
    let _ = ui.take_accessibility_update();
    drop(ui);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(60), "linear work ({took:?})");
}
