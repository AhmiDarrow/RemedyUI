//! The runner on a real Windows window with the GPU renderer. One test per process: a
//! process may create only one event loop.
#![cfg(windows)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::layout::Length;
use remedy_ui::reactive::Signal;
use remedy_ui::{App, AppConfig, GeometryStore, Runner, WindowSpec};
use remedy_ui_platform::bridge_windows::WindowsHooks;
use remedy_ui_platform::bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_platform::bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_platform::bridge_winit::{UserEvent, run_on};

#[test]
fn an_app_runs_on_a_real_window_and_quits() {
    let built = Rc::new(Cell::new(false));
    let b2 = built.clone();
    let label = Signal::new(String::from("Starting"));
    let runner = Runner::with_geometry(
        AppConfig::new("RemedyUI window test"),
        GeometryStore::in_memory(),
        move |app: &App| {
            app.open(WindowSpec::new("RemedyUI").size(240.0, 120.0), move |ui, _| {
                ui.root()
                    .child(ui.el("label").text(move || label.get()))
                    .child(ui.el("box").layout(|l| l.height = Length::Px(20.0)));
                b2.set(true);
            });
            app.after(Duration::from_millis(150), move || label.set(String::from("Updated")));
            let app2 = app.clone();
            app.after(Duration::from_millis(400), move || app2.quit());
        },
    );
    let event_loop = match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
        Ok(e) => e,
        Err(e) => unreachable!("an event loop: {e}"),
    };
    let result = run_on(event_loop, runner, WindowsHooks::default());
    assert!(result.is_ok(), "{result:?}");
    assert!(built.get(), "the window was built");
}
