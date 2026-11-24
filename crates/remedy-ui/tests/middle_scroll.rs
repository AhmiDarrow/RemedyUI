//! Middle-button drag pans a scroll container, the same as wheel and keyboard scrolling.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::host::HostEvent;
use remedy_ui::input::{Buttons, PointerEvent, PointerPhase};
use remedy_ui::layout::Length;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui_core::{Point, Vec2};

#[test]
fn middle_button_drag_pans_a_scroll_container() {
    let scroller: Rc<RefCell<Option<remedy_ui::El>>> = Rc::new(RefCell::new(None));
    let slot = scroller.clone();
    let mut app = TestApp::new(AppConfig::new("middle-scroll"), move |app: &App| {
        let slot = slot.clone();
        app.open(WindowSpec::new("middle-scroll").size(200.0, 200.0), move |ui, _| {
            let tall = ui.el("tall").layout(|l| {
                l.width = Length::Px(200.0);
                l.height = Length::Px(800.0);
                l.min_height = Length::Px(800.0);
            });
            let bar = w::scroll(ui)
                .layout(|l| {
                    l.width = Length::Px(200.0);
                    l.height = Length::Px(200.0);
                    l.max_height = Length::Px(200.0);
                    l.min_height = Length::Px(200.0);
                })
                .child(tall);
            *slot.borrow_mut() = Some(bar.clone());
            ui.root().child(bar);
        });
    });
    app.frames(3);
    let el = scroller.borrow().clone().expect("scroller");
    let ui = app.ui().expect("ui");
    assert_eq!(ui.scroll_offset(el.id()).y, 0.0, "starts at the top");

    let window = app.host().window_ids().first().copied().expect("window");
    let at = |y: f32, phase: PointerPhase, buttons: Buttons| {
        PointerEvent::new(phase, Point::new(80.0, y)).with_buttons(buttons)
    };
    app.host().send(HostEvent::Pointer(window, at(120.0, PointerPhase::Down, Buttons::MIDDLE)));
    app.host().send(HostEvent::Pointer(window, at(40.0, PointerPhase::Move, Buttons::MIDDLE)));
    app.host().send(HostEvent::Pointer(window, at(40.0, PointerPhase::Up, Buttons::default())));
    app.frames(3);

    let ui = app.ui().expect("ui");
    let offset = ui.scroll_offset(el.id());
    assert!(offset.y >= 70.0, "dragging 80 px up should pan content down, got {offset:?}");
    let _ = Vec2::ZERO;
}
