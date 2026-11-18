//! A rotating triangle rendered by the app with its own wgpu pipeline on RemedyUI's device,
//! checked in the frame the compositor composed: on the GPU renderer (straight from the
//! texture) and on the CPU renderer (read back). Plus resizing, animation, input, and the
//! element going away.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use common::*;
use remedy_ui_core::Point;
use remedy_ui_embed_gpu::*;
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerKind, PointerPhase};

/// Checks the triangle in the composed frame at angle 0, then turned half a circle.
fn rotates(gpu: bool) {
    let (mut s, el, view, angle, seen) = triangle(gpu);
    assert_eq!(seen.prepared.get(), 1, "prepared once");
    // Apex up: inside near the top, the middle inside, a corner outside.
    assert!(near(s.px(100, 30), RED), "apex: {:?}", s.px(100, 30));
    assert!(near(s.px(100, 100), RED), "middle: {:?}", s.px(100, 100));
    assert!(near(s.px(100, 170), BLUE), "below the apex-up base: {:?}", s.px(100, 170));
    assert!(near(s.px(4, 4), BLUE), "corner: {:?}", s.px(4, 4));
    // Turned half a circle: the apex is at the bottom now.
    angle.set(std::f32::consts::PI);
    view.request_redraw();
    s.frame();
    assert!(near(s.px(100, 30), BLUE), "above the turned base: {:?}", s.px(100, 30));
    assert!(near(s.px(100, 170), RED), "turned apex: {:?}", s.px(100, 170));
    assert!(near(s.px(100, 100), RED), "middle: {:?}", s.px(100, 100));
    // A quarter turn: the apex points left.
    angle.set(std::f32::consts::FRAC_PI_2);
    view.request_redraw();
    s.frame();
    assert!(near(s.px(30, 100), RED), "apex left: {:?}", s.px(30, 100));
    assert!(near(s.px(170, 100), BLUE), "right of the base: {:?}", s.px(170, 100));
    let stats = view.stats();
    assert_eq!(stats.rendered, 3);
    assert_eq!(stats.allocations, 1, "textures made once");
    assert_eq!(*seen.sizes.borrow(), vec![(200, 200); 3], "sized from the element");
    // A repaint for no reason of the content's shows the same picture without rendering.
    el.repaint();
    s.frame();
    assert_eq!(view.stats().rendered, 3);
    assert!(near(s.px(30, 100), RED));
    if gpu {
        assert_eq!(view.status(), EmbedStatus::Gpu);
        assert_eq!(view.stats().read_back, 0, "the GPU renderer samples the texture itself");
    } else {
        assert_eq!(view.status(), EmbedStatus::ReadBack);
        assert_eq!(view.stats().read_back, 3, "read back once per picture");
    }
}

#[test]
fn a_rotating_triangle_shows_in_the_gpu_composed_frame() {
    rotates(true);
}

#[test]
fn on_the_cpu_renderer_the_picture_is_read_back() {
    rotates(false);
}

#[test]
fn the_element_asks_for_the_gpu() {
    let (s, _el, _view, _angle, _seen) = triangle(false);
    // The first frame has gone; the request stays until the runner takes it.
    let requests = s.ui.take_requests();
    assert!(requests.contains(&remedy_ui_view::Request::PreferGpu), "{requests:?}");
}

#[test]
fn resizing_makes_new_textures_at_the_new_size() {
    let (mut s, _el, view, _angle, seen) = triangle(true);
    s.resize(120.0, 80.0);
    s.frame();
    assert_eq!(seen.sizes.borrow().last().copied(), Some((120, 80)));
    assert_eq!(view.stats().allocations, 2);
    // Still the whole picture, scaled to the new box: the middle is red, a corner blue.
    assert!(near(s.px(60, 40), RED), "{:?}", s.px(60, 40));
    assert!(near(s.px(2, 2), BLUE), "{:?}", s.px(2, 2));
}

#[test]
fn animating_renders_a_new_picture_every_frame_until_stopped() {
    let (mut s, _el, view, _angle, _seen) = triangle(true);
    view.set_animating(true);
    let _ = s.ui.take_requests();
    for _ in 0..3 {
        s.frame();
    }
    assert_eq!(view.stats().rendered, 4, "one per frame");
    assert!(s.ui.take_requests().contains(&remedy_ui_view::Request::Frame), "asks for the next frame");
    view.set_animating(false);
    s.frame();
    let before = view.stats().rendered;
    s.frame();
    s.frame();
    assert_eq!(view.stats().rendered, before, "stopped");
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> PointerEvent {
    PointerEvent {
        id: 0,
        kind: PointerKind::Mouse,
        phase,
        position: Point::new(x, y),
        buttons: if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() },
        pressure: 0.5,
        time: Duration::ZERO,
        coalesced: Vec::new(),
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn pointer_keys_and_focus_reach_the_content() {
    let ui = ui();
    let angle = Rc::new(Cell::new(0.0f32));
    let seen = Rc::new(Seen::default());
    let content = Triangle { angle, seen: seen.clone(), pipeline: None };
    let (el, _view) = gpu_view(&ui, "Spinning triangle", content);
    // Placed 20 pixels in, so element coordinates differ from the window's.
    ui.root().child(
        ui.el("pad").padding(remedy_ui_core::Insets::all(20.0)).child(el.clone().width(100.0).height(100.0)),
    );
    let mut s = Screen::new(ui.clone(), true);
    s.frame();
    assert!(ui.pointer(&pointer(PointerPhase::Down, 50.0, 60.0)));
    assert!(ui.pointer(&pointer(PointerPhase::Up, 50.0, 60.0)));
    let key = KeyEvent {
        key: Key::Character("w".into()),
        physical: String::new(),
        modifiers: Modifiers::NONE,
        pressed: true,
        repeat: false,
        composing: false,
        text: Some("w".into()),
    };
    assert!(ui.key(&key));
    let inputs = seen.inputs.borrow().clone();
    let firsts: Vec<&EmbedInput> = inputs.iter().filter(|i| !matches!(i, EmbedInput::Focus(_))).collect();
    match firsts.first() {
        Some(EmbedInput::Pointer { local, pixel, event }) => {
            assert_eq!(*local, Point::new(30.0, 40.0), "in the element's own space");
            assert_eq!(*pixel, Point::new(30.0, 40.0));
            assert_eq!(event.phase, PointerPhase::Down);
        }
        other => unreachable!("a pointer press first: {other:?}"),
    }
    assert!(inputs.contains(&EmbedInput::Focus(true)), "pressing focuses: {inputs:?}");
    assert!(inputs.contains(&EmbedInput::Key(key)), "keys while focused");
    // Screen readers get an image with the label.
    let sem = el.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!(sem.role(), remedy_ui_a11y::Role::Image);
    assert_eq!(sem.name(), Some("Spinning triangle"));
}

#[test]
fn a_removed_element_lets_its_textures_go() {
    let (mut s, el, view, _angle, _seen) = triangle(true);
    assert!(view.is_open());
    el.remove();
    s.frame();
    assert!(!view.is_open());
    view.request_redraw();
    s.frame();
    assert_eq!(view.stats().rendered, 1, "nothing renders for a removed element");
}
