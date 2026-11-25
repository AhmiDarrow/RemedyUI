//! GPU frames presented to a real window, through a resize and a forced device loss. One
//! test per process, because a process may create only one event loop.
//!
//! Windows, and Linux on X11 (Xvfb is enough: `xvfb-run cargo test --test window`); without a
//! display the Linux run says so and passes.
#![cfg(any(windows, target_os = "linux"))]

mod common;

use std::time::Duration;

use remedy_ui_bridge_winit::run_on;
use remedy_ui_compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Rect, Size};
use remedy_ui_core::host::{App, Host, HostEvent, WindowId, WindowOptions};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_paint::{Brush, DisplayList};
use remedy_ui_render_gpu::{GpuCompositor, GpuContext, GpuRenderer, GpuSurface, LadderOptions, Presented};

fn tree() -> LayerTree {
    let mut l = DisplayList::new();
    l.fill_rounded(
        Rect::new(10.0, 10.0, 190.0, 140.0),
        CornerRadii::all(12.0),
        Brush::Solid(Color::rgb8(40, 120, 70)),
    );
    LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, 200.0, 150.0), l)])
}

#[derive(Default)]
struct Probe {
    window: Option<WindowId>,
    gpu: Option<GpuCompositor>,
    compositor: Option<Compositor>,
    surface: Option<GpuSurface>,
    shown: usize,
    errors: Vec<String>,
    resized: bool,
    lost: bool,
    recovered: bool,
    swapchain_lost: bool,
    /// The window's center after a lost swapchain was rebuilt, with nothing changed since.
    after_swapchain: Option<[u8; 4]>,
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
                self.gpu = Some(GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default()));
                let opts = {
                    let mut v_ = WindowOptions::default();
                    v_.title = "RemedyUI GPU test".into();
                    v_.size = Size::new(200.0, 150.0);
                    v_
                };
                self.window = host.create_window(&opts).ok();
            }
            HostEvent::Resized { window, width, height, scale } => {
                let Some(gpu) = self.gpu.as_ref() else { return };
                match self.surface.as_mut() {
                    Some(s) => gpu.resize_surface(s, width, height),
                    None => {
                        match host.window_handles(window).map_err(|e| e.to_string()).and_then(|h| {
                            gpu.create_surface(h, width, height, false).map_err(|e| e.to_string())
                        }) {
                            Ok(s) => self.surface = Some(s),
                            Err(e) => return self.errors.push(e),
                        }
                    }
                }
                let c = self.compositor.get_or_insert_with(|| {
                    let mut c = Compositor::new(width, height, scale, ScrollPhysics::CLAMPED);
                    c.commit(tree());
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
                if gpu.is_lost() {
                    match gpu.recover(c) {
                        Ok(()) => self.recovered = true,
                        Err(e) => return self.errors.push(e.to_string()),
                    }
                }
                let plan = c.frame(at);
                self.errors.extend(gpu.renderer().context().take_errors());
                match gpu.present_to_surface(&plan, s, Color::WHITE) {
                    Ok(Presented::Shown) => self.shown += 1,
                    Ok(_) => {}
                    Err(e) => self.errors.push(e.to_string()),
                }
                match self.shown {
                    2 if !self.resized => {
                        self.resized = true;
                        let _ = host.set_size(window, Size::new(260.0, 180.0));
                    }
                    3 if !self.lost => {
                        self.lost = true;
                        gpu.renderer().context().force_loss();
                    }
                    4 if !self.swapchain_lost => {
                        self.swapchain_lost = true;
                        s.lose_swapchain();
                    }
                    n if n >= 5 => {
                        if let Some(Ok(f)) = gpu.read_surface(s) {
                            let at = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
                            self.after_swapchain = f.pixels.get(at..at + 4).and_then(|p| p.try_into().ok());
                        }
                        host.close_window(window)
                    }
                    _ => {}
                }
                host.request_frame(window);
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
fn gpu_frames_reach_a_real_window_through_resize_and_device_loss() {
    let Some(event_loop) = common::event_loop() else { return };
    let mut probe = Probe::default();
    let result = run_on(event_loop, &mut probe, common::Hooks::default());
    assert!(result.is_ok(), "{result:?}");
    let validation = common::validation();
    assert!(validation.is_empty(), "validation: {validation:#?}");
    assert!(!probe.deadline_hit, "finished in time; errors: {:?}", probe.errors);
    assert!(probe.errors.is_empty(), "{:?}", probe.errors);
    assert!(probe.resized && probe.lost && probe.recovered);
    assert!(probe.shown >= 5, "frames shown: {}", probe.shown);
    // The content survives a lost swapchain: its tiles are on the device, which lived.
    assert_eq!(probe.after_swapchain, Some([40, 120, 70, 255]), "the card, not the white background");
}
