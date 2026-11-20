//! A window presents only what changed: swapchain images keep what they showed, each present
//! copies in what changed since that image was last shown and tells the system only that. Every
//! image presented must still equal the window's whole frame, through a resize and frames where
//! nothing changed. One test per process (one event loop per process).
#![cfg(windows)]

use std::time::Duration;

use remedy_ui_bridge_windows::WindowsHooks;
use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_bridge_winit::{UserEvent, run_on};
use remedy_ui_compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Rect, Size};
use remedy_ui_host::{App, Host, HostEvent, WindowOptions};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_paint::{Brush, DisplayList};
use remedy_ui_render_gpu::{
    GpuCompositor, GpuContext, GpuRenderer, GpuSurface, LadderOptions, PresentChecks, Presented,
};

/// Stripes, and a dot at `x` (frames where `x` is unchanged change nothing).
fn tree(x: f32) -> LayerTree {
    let mut back = DisplayList::new();
    for i in 0..40 {
        let c = Color::rgb8((i * 6) as u8, 90, 200 - (i * 4) as u8);
        back.fill_rect(Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 400.0), Brush::Solid(c));
    }
    let mut dot = DisplayList::new();
    dot.fill_rounded(Rect::new(0.0, 0.0, 20.0, 20.0), CornerRadii::all(10.0), Brush::Solid(Color::WHITE));
    LayerTree::new(vec![
        Layer::new(LayerId(1), Rect::new(0.0, 0.0, 400.0, 400.0), back).with_child(Layer::new(
            LayerId(2),
            Rect::new(x, 60.0, x + 20.0, 80.0),
            dot,
        )),
    ])
}

#[derive(Default)]
struct Probe {
    gpu: Option<GpuCompositor>,
    compositor: Option<Compositor>,
    surface: Option<GpuSurface>,
    shown: usize,
    resized: bool,
    checks: PresentChecks,
    backend: String,
    errors: Vec<String>,
    deadline_hit: bool,
}

impl App for Probe {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        match event {
            HostEvent::Resumed => {
                host.set_timer(host.now() + Duration::from_secs(30));
                let ctx = match GpuContext::new(&LadderOptions::default()) {
                    Ok(c) => c,
                    Err(e) => return self.errors.push(e.to_string()),
                };
                self.backend = ctx.describe();
                self.gpu = Some(GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default()));
                let mut opts = WindowOptions::default();
                opts.title = "RemedyUI present damage test".into();
                opts.size = Size::new(300.0, 200.0);
                if let Err(e) = host.create_window(&opts) {
                    self.errors.push(e.to_string());
                }
            }
            HostEvent::Resized { window, width, height, scale } => {
                let Some(gpu) = self.gpu.as_ref() else { return };
                match self.surface.as_mut() {
                    Some(s) => gpu.resize_surface(s, width, height),
                    None => {
                        let made = host.window_handles(window).map_err(|e| e.to_string()).and_then(|h| {
                            gpu.create_surface(h, width, height, false).map_err(|e| e.to_string())
                        });
                        match made {
                            Ok(mut s) => {
                                s.check_presents();
                                // Built again so the swapchain can be read back.
                                gpu.resize_surface(&mut s, width, height);
                                self.surface = Some(s);
                            }
                            Err(e) => return self.errors.push(e),
                        }
                    }
                }
                let c = self.compositor.get_or_insert_with(|| {
                    let mut c = Compositor::new(width, height, scale, ScrollPhysics::CLAMPED);
                    c.commit(tree(10.0));
                    c
                });
                c.resize(width, height, scale);
                host.request_frame(window);
            }
            HostEvent::Frame { window, at } => {
                let (Some(gpu), Some(c), Some(s)) =
                    (self.gpu.as_mut(), self.compositor.as_mut(), self.surface.as_mut())
                else {
                    return;
                };
                // The dot moves on most frames and holds still on every fourth.
                let step = if self.shown % 4 == 3 { 0.0 } else { 7.0 };
                c.commit(tree(10.0 + (self.shown as f32 * step) % 250.0));
                let plan = c.frame(at);
                match gpu.present_to_surface(&plan, s, Color::WHITE) {
                    Ok(Presented::Shown) => self.shown += 1,
                    Ok(_) => {}
                    Err(e) => self.errors.push(e.to_string()),
                }
                self.errors.extend(gpu.renderer().context().take_errors());
                if self.shown == 15 && !self.resized {
                    self.resized = true;
                    let _ = host.set_size(window, Size::new(340.0, 220.0));
                }
                if self.shown >= 40 {
                    self.checks = s.present_checks();
                    host.close_window(window);
                } else {
                    host.request_frame(window);
                }
            }
            HostEvent::Timer(_) => {
                self.deadline_hit = true;
                host.exit();
            }
            HostEvent::Destroyed(_) => {
                self.surface = None;
                host.exit();
            }
            _ => {}
        }
    }
}

#[test]
fn presenting_only_what_changed_shows_the_whole_frame() {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
        Ok(e) => e,
        Err(e) => unreachable!("an event loop: {e}"),
    };
    let mut probe = Probe::default();
    let result = run_on(event_loop, &mut probe, WindowsHooks::default());
    assert!(result.is_ok(), "{result:?}");
    assert!(!probe.deadline_hit, "finished in time; errors: {:?}", probe.errors);
    assert!(probe.errors.is_empty(), "{:?}", probe.errors);
    let c = probe.checks;
    eprintln!("{}: {c:?}", probe.backend);
    assert!(c.compared >= 35, "the swapchain could be read back: {c:?}");
    assert_eq!(c.mismatched, 0, "every presented image equals the whole frame: {c:?}");
    assert!(c.damaged >= 20, "most presents copied in only what changed: {c:?}");
}
