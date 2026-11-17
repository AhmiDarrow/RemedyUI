//! An `Auto` window starts on the CPU renderer and moves to the GPU once its frames are
//! slow, on a real Windows window. One test per process (one event loop per process).
//!
//! Optimized builds only (`cargo test --release -p remedy-ui --test windows_auto_gpu`):
//! unoptimized, building the GPU pipelines takes long enough to blow the deadline while a
//! window sits frozen on the desktop.
#![cfg(all(windows, feature = "gpu", not(debug_assertions)))]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::core::{CornerRadii, Rect};
use remedy_ui::reactive::Signal;
use remedy_ui::{App, AppConfig, GeometryStore, RendererKind, Runner, WindowSpec};
use remedy_ui_bridge_windows::WindowsHooks;
use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_bridge_winit::{UserEvent, run_on};

/// Repaints a heavy scene every few milliseconds.
fn tick(app: &App, t: Signal<u32>) {
    let a = app.clone();
    app.after(Duration::from_millis(4), move || {
        t.update(|v| *v += 1);
        tick(&a, t);
    });
}

/// Checks every 100 ms until the window is on the GPU or 8 s have passed, then quits.
fn watch(app: &App, win: remedy_ui::Window, seen: Rc<Cell<Option<RendererKind>>>, since: std::time::Instant) {
    let a = app.clone();
    app.after(Duration::from_millis(100), move || {
        seen.set(win.renderer());
        if seen.get() == Some(RendererKind::Gpu) || since.elapsed() > Duration::from_secs(8) {
            a.quit();
        } else {
            watch(&a, win, seen, since);
        }
    });
}

#[test]
fn a_slow_window_moves_from_the_cpu_to_the_gpu() {
    let first = Rc::new(Cell::new(None));
    let later = Rc::new(Cell::new(None));
    let (f2, l2) = (first.clone(), later.clone());
    let runner = Runner::with_geometry(
        AppConfig::new("RemedyUI auto renderer test"),
        GeometryStore::in_memory(),
        move |app: &App| {
            let t = Signal::new(0u32);
            let win =
                app.open(WindowSpec::new("RemedyUI renderer test").size(1200.0, 800.0), move |ui, _| {
                    let scene = ui.el("box").grow(1.0).fill_width().paint(move |cx| {
                        let phase = t.get() as f32;
                        for i in 0..1500 {
                            let x = ((i * 37) as f32 + phase * 3.0) % cx.size.width;
                            let y = ((i * 91) as f32 + phase) % cx.size.height;
                            let c = remedy_ui::core::Color::new((i % 7) as f32 / 7.0, 0.5, 0.8, 0.5);
                            cx.list.fill_rounded(
                                Rect::new(x, y, x + 40.0, y + 24.0),
                                CornerRadii::all(8.0),
                                remedy_ui_paint::Brush::Solid(c),
                            );
                        }
                    });
                    ui.root().child(scene);
                });
            tick(app, t);
            let w1 = win.clone();
            app.after(Duration::from_millis(30), move || f2.set(w1.renderer()));
            watch(app, win, l2, std::time::Instant::now());
        },
    );
    let event_loop = match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
        Ok(e) => e,
        Err(e) => unreachable!("an event loop: {e}"),
    };
    let result = run_on(event_loop, runner, WindowsHooks::default());
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(first.get(), Some(RendererKind::Cpu), "starts on the CPU");
    assert_eq!(later.get(), Some(RendererKind::Gpu), "moved to the GPU when frames got slow");
}
