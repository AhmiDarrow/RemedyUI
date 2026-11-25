//! A window presents only what changed: swapchain images keep what they showed, each present
//! copies in what changed since that image was last shown and tells the system only that. Every
//! image presented must still equal the window's whole frame, through a resize and frames where
//! nothing changed. One test per process (one event loop per process).
//!
//! Windows, and Linux on X11 (Xvfb is enough: `xvfb-run cargo test --test present_damage`);
//! without a display the Linux run says so and passes.
#![cfg(any(windows, target_os = "linux"))]

mod common;

use std::time::Duration;

use remedy_ui_platform::bridge_winit::run_on;
use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Rect, Size};
use remedy_ui_core::host::{App, Host, HostEvent, WindowOptions};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_engine::paint::{Brush, DisplayList};
use remedy_ui_engine::render_gpu::{
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

/// Presents whose swapchain image is read back and compared; more follow unchecked.
const CHECKED: usize = 40;

/// Frames the window's second resize takes to reach the surface.
const LAG: usize = 8;

#[derive(Default)]
struct Probe {
    /// How far the dot has moved.
    x: f32,
    /// Frames presented since the window's second resize while the surface was not told.
    lagged: usize,
    withheld: Option<(u32, u32)>,
    /// Presents in a row that asked to be drawn again, and the most seen.
    retries: usize,
    most_retries: usize,
    gpu: Option<GpuCompositor>,
    compositor: Option<Compositor>,
    surface: Option<GpuSurface>,
    shown: usize,
    resized: usize,
    checks: PresentChecks,
    own: bool,
    reports: bool,
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
                    // The second resize reaches the surface only some frames later, as when
                    // events lag behind the window: the swapchain must keep presenting.
                    Some(_) if self.resized == 2 && self.lagged < LAG => {
                        self.withheld = Some((width, height));
                        return host.request_frame(window);
                    }
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
                // The dot moves on most frames and holds still on every fourth; after the
                // checked presents it also holds still long enough that every image is
                // presented untouched.
                let still = self.shown % 4 == 3 || (CHECKED + 4..CHECKED + 12).contains(&self.shown);
                if !still {
                    self.x = (self.x + 7.0) % 250.0;
                }
                c.commit(tree(10.0 + self.x));
                let plan = c.frame(at);
                let presented = gpu.present_to_surface(&plan, s, Color::WHITE);
                self.retries = if matches!(presented, Ok(Presented::Retry)) { self.retries + 1 } else { 0 };
                self.most_retries = self.most_retries.max(self.retries);
                match presented {
                    Ok(Presented::Shown) => self.shown += 1,
                    Ok(_) => {}
                    Err(e) => self.errors.push(e.to_string()),
                }
                if self.resized == 2 && self.lagged < LAG {
                    self.lagged += 1;
                    if self.lagged == LAG
                        && let Some((w, h)) = self.withheld.take()
                    {
                        gpu.resize_surface(s, w, h);
                    }
                }
                self.errors.extend(gpu.renderer().context().take_errors());
                if self.shown == 15 && self.resized == 0 {
                    self.resized = 1;
                    let _ = host.set_size(window, Size::new(340.0, 220.0));
                }
                if self.shown == CHECKED && self.resized == 1 {
                    // The rest present as an app's frames do, unchecked (a read back changes
                    // how a present returns the image), through another resize: the
                    // validation layer watches those.
                    self.checks = s.stop_checking_presents();
                    self.resized = 2;
                    let _ = host.set_size(window, Size::new(320.0, 240.0));
                }
                if self.shown >= CHECKED + 25 {
                    self.own = s.own_swapchain();
                    self.reports = s.reports_damage();
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
    let Some(event_loop) = common::event_loop() else { return };
    let mut probe = Probe::default();
    let result = run_on(event_loop, &mut probe, common::Hooks::default());
    assert!(result.is_ok(), "{result:?}");
    let validation = common::validation();
    assert!(validation.is_empty(), "validation: {validation:#?}");
    assert!(!probe.deadline_hit, "finished in time; errors: {:?}", probe.errors);
    assert!(probe.errors.is_empty(), "{:?}", probe.errors);
    // A swapchain the window outgrew is built again at once (a retry or two), never left to
    // ask for frames it cannot show until the surface hears of the new size.
    assert!(probe.most_retries <= 2, "presents in a row that showed nothing: {}", probe.most_retries);
    let c = probe.checks;
    eprintln!("{}: {c:?}, own swapchain: {}, reports damage: {}", probe.backend, probe.own, probe.reports);
    // D3D12 always can; Vulkan where the device offers VK_KHR_incremental_present (Mesa and
    // Android do; NVIDIA's Windows driver does not). A device that offers it but failed to
    // open with it is a recorded GPU error, which fails the test above.
    if probe.backend.contains("Dx12") {
        assert!(probe.reports, "{}", probe.backend);
    }
    // On D3D12 and Vulkan the window presents through RemedyUI's own swapchain.
    let own_expected = probe.backend.contains("Dx12") || probe.backend.contains("Vulkan");
    assert_eq!(probe.own, own_expected, "{}", probe.backend);
    assert!(c.compared >= 35, "the swapchain could be read back: {c:?}");
    assert_eq!(c.mismatched, 0, "every presented image equals the whole frame: {c:?}");
    // An image that holds nothing known (new to the swapchain, or after the resize) must be
    // drawn whole; every other present copies in only what changed.
    assert_eq!(c.damaged + c.unknown, c.compared, "a whole frame only where the image held nothing: {c:?}");
    assert!(c.unknown <= 8, "only each image's first use (twice: at start and after the resize): {c:?}");
}
