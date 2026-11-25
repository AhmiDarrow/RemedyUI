//! What presenting a small change costs on a large window: a dot moves over a full-window
//! background, presented without vsync so the GPU's work shows in the time each present
//! takes. Prints the median, p95 and total; run by hand to compare present paths:
//! `cargo test --release -p remedy-ui-render-gpu --test present_cost -- --ignored --nocapture`.
#![cfg(windows)]

use std::time::{Duration, Instant};

use remedy_ui_platform::bridge_windows::WindowsHooks;
use remedy_ui_platform::bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_platform::bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_platform::bridge_winit::{UserEvent, run_on};
use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Rect, Size};
use remedy_ui_core::host::{App, Host, HostEvent, WindowOptions};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_engine::paint::{Brush, DisplayList};
use remedy_ui_engine::render_gpu::{GpuCompositor, GpuContext, GpuRenderer, GpuSurface, LadderOptions, Presented};

const FRAMES: usize = 600;
const WARMUP: usize = 60;

fn tree(w: f32, h: f32, x: f32) -> LayerTree {
    let mut back = DisplayList::new();
    for i in 0..64 {
        let c = Color::rgb8((i * 4) as u8, 90, 255 - (i * 4) as u8);
        let x0 = i as f32 * w / 64.0;
        back.fill_rect(Rect::new(x0, 0.0, x0 + w / 64.0, h), Brush::Solid(c));
    }
    let mut dot = DisplayList::new();
    dot.fill_rounded(Rect::new(0.0, 0.0, 24.0, 24.0), CornerRadii::all(12.0), Brush::Solid(Color::WHITE));
    LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, w, h), back).with_child(Layer::new(
        LayerId(2),
        Rect::new(x, 200.0, x + 24.0, 224.0),
        dot,
    ))])
}

#[derive(Default)]
struct Probe {
    gpu: Option<GpuCompositor>,
    compositor: Option<Compositor>,
    surface: Option<GpuSurface>,
    size: (f32, f32),
    times: Vec<Duration>,
    started: Option<Instant>,
    total: Duration,
    backend: String,
    errors: Vec<String>,
}

impl App for Probe {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        match event {
            HostEvent::Resumed => {
                host.set_timer(host.now() + Duration::from_secs(60));
                let ctx = match GpuContext::new(&LadderOptions::default()) {
                    Ok(c) => c,
                    Err(e) => return self.errors.push(e.to_string()),
                };
                self.backend = ctx.describe();
                self.gpu = Some(GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default()));
                let mut opts = WindowOptions::default();
                opts.title = "RemedyUI present cost".into();
                opts.size = Size::new(1800.0, 1000.0);
                if let Err(e) = host.create_window(&opts) {
                    self.errors.push(e.to_string());
                }
            }
            HostEvent::Resized { window, width, height, scale } => {
                let Some(gpu) = self.gpu.as_ref() else { return };
                if self.surface.is_none() {
                    let made = host.window_handles(window).map_err(|e| e.to_string()).and_then(|h| {
                        gpu.create_surface_paced(h, width, height, false, false).map_err(|e| e.to_string())
                    });
                    match made {
                        Ok(s) => self.surface = Some(s),
                        Err(e) => return self.errors.push(e),
                    }
                } else if let Some(s) = self.surface.as_mut() {
                    gpu.resize_surface(s, width, height);
                }
                self.size = (width as f32 / scale, height as f32 / scale);
                let (w, h) = self.size;
                let c = self.compositor.get_or_insert_with(|| {
                    let mut c = Compositor::new(width, height, scale, ScrollPhysics::CLAMPED);
                    c.commit(tree(w, h, 10.0));
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
                let n = self.times.len();
                let (w, h) = self.size;
                c.commit(tree(w, h, 10.0 + (n as f32 * 5.0) % (w - 40.0)));
                let plan = c.frame(at);
                let t = Instant::now();
                match gpu.present_to_surface(&plan, s, Color::WHITE) {
                    Ok(Presented::Shown) => {
                        if n == WARMUP {
                            self.started = Some(Instant::now());
                        }
                        self.times.push(t.elapsed());
                    }
                    Ok(_) => {}
                    Err(e) => self.errors.push(e.to_string()),
                }
                self.errors.extend(gpu.renderer().context().take_errors());
                if self.times.len() >= FRAMES + WARMUP {
                    // Everything submitted has finished: the GPU's share is in the total.
                    let _ = gpu.renderer().context().device.poll(wgpu::PollType::wait_indefinitely());
                    self.total = self.started.map(|s| s.elapsed()).unwrap_or_default();
                    host.close_window(window);
                } else {
                    host.request_frame(window);
                }
            }
            HostEvent::Timer(_) => host.exit(),
            HostEvent::Destroyed(_) => {
                self.surface = None;
                host.exit();
            }
            _ => {}
        }
    }
}

#[test]
#[ignore = "a measurement, run by hand"]
fn a_small_change_on_a_large_window() {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
        Ok(e) => e,
        Err(e) => unreachable!("an event loop: {e}"),
    };
    let mut probe = Probe::default();
    let result = run_on(event_loop, &mut probe, WindowsHooks::default());
    assert!(result.is_ok(), "{result:?}");
    assert!(probe.errors.is_empty(), "{:?}", probe.errors);
    let mut t: Vec<Duration> = probe.times.split_off(WARMUP.min(probe.times.len()));
    assert!(t.len() >= FRAMES, "{} presents", t.len());
    t.sort();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    eprintln!(
        "{}: {} presents, median {:.0} us, p95 {:.0} us, {:.1} ms in all ({:.0} us a frame)",
        probe.backend,
        t.len(),
        us(t.get(t.len() / 2).copied().unwrap_or_default()),
        us(t.get(t.len() * 95 / 100).copied().unwrap_or_default()),
        probe.total.as_secs_f64() * 1e3,
        us(probe.total) / t.len() as f64,
    );
}
