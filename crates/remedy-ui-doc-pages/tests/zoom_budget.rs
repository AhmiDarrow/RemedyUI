//! The 300-page PDF reference scene (docs/design/09-performance-and-resources.md): zooming
//! from 1% to 6400% and back within the frame budget, rendering only what shows.
//!
//! Measured in release builds (`cargo test --release`); debug builds are not what ships.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Point, Size};
use remedy_ui_doc_pages::*;
use remedy_ui_input::ScrollPhysics;
use remedy_ui_render_cpu::CpuCompositor;
use remedy_ui_render_gpu::{GpuCompositor, GpuContext, GpuRenderer, LadderOptions};

mod common;
use common::*;

/// The UI thread's share of a frame (input, layout and paint of the view, the compositor's
/// plan): the budget the explorer scene holds too.
const UI_BUDGET: Duration = Duration::from_millis(8);
/// A 60 Hz display: frames come this often while the wheel turns, and a whole frame (the UI
/// thread's work and the renderer's) must fit in one.
const FRAME: Duration = Duration::from_micros(16_667);
const WINDOW: Size = Size { width: 1280.0, height: 800.0 };

#[derive(Default)]
struct Times {
    ui: Vec<Duration>,
    raster: Vec<Duration>,
}

impl Times {
    fn summary(v: &[Duration]) -> (Duration, Duration, Duration) {
        let mut s = v.to_vec();
        s.sort();
        let at =
            |q: usize| s.get((s.len() * q / 100).min(s.len().saturating_sub(1))).copied().unwrap_or_default();
        (at(50), at(95), s.last().copied().unwrap_or_default())
    }
}

enum Renderer {
    Cpu(Box<CpuCompositor>),
    Gpu(Box<GpuCompositor>, remedy_ui_render_gpu::wgpu::Texture),
}

struct Scene {
    w: Win,
    comp: Compositor,
    renderer: Renderer,
    times: Times,
}

impl Scene {
    /// One display frame: optional input, then everything the UI thread does, timed; then
    /// the CPU renderer runs the plan (timed apart: on the GPU this is the GPU's work).
    fn frame(&mut self, input: impl FnOnce(&Win)) {
        let started = Instant::now();
        input(&self.w);
        if let Some(t) = self.w.frame_layers() {
            self.comp.commit(t);
        }
        let plan = self.comp.frame(started.elapsed());
        let ui = started.elapsed();
        let r = Instant::now();
        let (w, h) = (WINDOW.width as u32, WINDOW.height as u32);
        match &mut self.renderer {
            Renderer::Cpu(cpu) => {
                if cpu.present(&plan, w, h, Color::BLACK).is_ok() {
                    let (px, took) = cpu.last_raster();
                    self.comp.record_raster(px, took);
                }
            }
            Renderer::Gpu(gpu, target) => {
                // Read back so the time includes the GPU finishing the frame.
                let shown = gpu.present(&plan, target, Color::BLACK);
                assert!(shown.is_ok(), "the GPU frame: {:?}", shown.err());
                let back = gpu.renderer().read_back(target);
                assert!(back.is_ok(), "the GPU frame: {:?}", back.err());
                let px: u64 = plan.raster.iter().map(|r| r.pixels()).sum();
                self.comp.record_raster(px, r.elapsed());
            }
        }
        let raster = r.elapsed();
        self.times.ui.push(ui);
        self.times.raster.push(raster);
        if let Some(rest) = FRAME.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    fn settle(&mut self) -> usize {
        let start = Instant::now();
        let mut frames = 0;
        while !self.w.view.is_sharp() {
            assert!(start.elapsed() < Duration::from_secs(30), "settles: {:?}", self.w.view.stats());
            self.frame(|_| {});
            frames += 1;
        }
        frames
    }
}

/// What one run of the scene measured.
struct Run {
    zoom_in: Times,
    zoom_out: Times,
    sharpen: Times,
}

/// Zooms the document in from 1% to 6400% a wheel notch a frame, lets it sharpen, zooms out
/// as fast and lets it sharpen again, recording every frame.
fn run(name: &str, renderer: Renderer) -> Run {
    let src = Arc::new(Synthetic::new(300));
    let w = Win::new(src.clone(), WINDOW);
    let comp = Compositor::new(WINDOW.width as u32, WINDOW.height as u32, 1.0, ScrollPhysics::CLAMPED);
    let mut s = Scene { w, comp, renderer, times: Times::default() };
    let mid = Point::new(WINDOW.width / 2.0, WINDOW.height / 2.0);
    s.w.view.set_zoom(MIN_ZOOM, None);
    s.w.view.go_to_page(120);
    let _ = s.settle();
    let at_start = s.w.view.stats();
    s.times = Times::default();

    // In: one wheel notch (10%) a frame, 1% to 6400% (93 notches).
    let mut notches = 0;
    while s.w.view.zoom().get_untracked().unwrap_or(MAX_ZOOM) < MAX_ZOOM {
        s.frame(|w| {
            let _ = w.ui.wheel(&remedy_ui_input::WheelEvent {
                position: mid,
                delta: remedy_ui_core::Vec2::new(0.0, -48.0),
                precise: false,
                time: Duration::ZERO,
                modifiers: ctrl(),
            });
        });
        notches += 1;
        let st = s.w.view.stats();
        assert_eq!(st.blank, 0, "something shows for every tile while zooming (notch {notches}): {st:?}");
    }
    let zoom_in = std::mem::take(&mut s.times);
    let settle_in = s.settle();
    let sharp_in = std::mem::take(&mut s.times);
    let at_top = s.w.view.stats();

    // Out again, as fast: 6400% to 1%.
    while s.w.view.zoom().get_untracked().unwrap_or(MIN_ZOOM) > MIN_ZOOM {
        s.frame(|w| {
            let _ = w.ui.wheel(&remedy_ui_input::WheelEvent {
                position: mid,
                delta: remedy_ui_core::Vec2::new(0.0, 48.0),
                precise: false,
                time: Duration::ZERO,
                modifiers: ctrl(),
            });
        });
    }
    let zoom_out = std::mem::take(&mut s.times);
    let settle_out = s.settle();
    let sharp_out = std::mem::take(&mut s.times);
    let end = s.w.view.stats();

    let report = |what: &str, t: &Times| {
        let (p50, p95, worst) = Times::summary(&t.ui);
        let (r50, r95, rworst) = Times::summary(&t.raster);
        eprintln!(
            "300-page zoom on {name}, {what}: {} frames; UI thread p50 {p50:?} p95 {p95:?} worst {worst:?}; \
             renderer p50 {r50:?} p95 {r95:?} worst {rworst:?}",
            t.ui.len()
        );
    };
    report("1% to 6400%", &zoom_in);
    report(&format!("sharpening at 6400% ({settle_in} frames)"), &sharp_in);
    report("6400% to 1%", &zoom_out);
    report(&format!("sharpening at 1% ({settle_out} frames)"), &sharp_out);
    eprintln!(
        "{name} tiles: {} at 1% before, {} rendered by the end of the zoom in, {} by the end ({} cancelled); \
         most wanted in one frame {}; source pixels {}; cache {} of {} bytes",
        at_start.rendered,
        at_top.rendered,
        end.rendered,
        end.cancelled,
        end.most_wanted,
        src.pixels.load(Ordering::Relaxed),
        end.cached_bytes,
        end.cache_budget
    );
    assert!(notches >= 90, "1% to 6400% in 10% steps: {notches}");
    // Never more than what shows: at 1% the window holds about 50 rows of pages, one tile
    // each, with their previews; at any other zoom, a screen of 256-pixel tiles and a ring.
    let screen_tiles = (WINDOW.width / 256.0).ceil() as usize + 2;
    let screen_rows = (WINDOW.height / 256.0).ceil() as usize + 2;
    let pages_shown = (WINDOW.height / (792.0 * MIN_ZOOM + 8.0)).ceil() as usize + 1;
    // Pages a screen above and below are drawn too (their previews only), for scrolls and
    // zooms the compositor moves before the view redraws.
    let bound = 3 * pages_shown + screen_tiles * screen_rows;
    assert!(
        end.most_wanted <= bound,
        "one frame wanted {} tiles; what shows is at most {bound}",
        end.most_wanted
    );
    // Zooming in, the tiles already there stand in until the zoom stops: the whole zoom in
    // renders the previews of the pages it passes and one sharp screen at the end.
    let zoom_in_renders = at_top.rendered - at_start.rendered;
    assert!(zoom_in_renders <= (bound * 2) as u64, "zooming in rendered {zoom_in_renders} tiles");
    let mut sharpen = sharp_in;
    sharpen.ui.extend(sharp_out.ui);
    sharpen.raster.extend(sharp_out.raster);
    Run { zoom_in, zoom_out, sharpen }
}

fn total(t: &Times) -> Vec<Duration> {
    t.ui.iter().zip(&t.raster).map(|(a, b)| *a + *b).collect()
}

#[test]
#[cfg_attr(debug_assertions, ignore = "frame budgets are measured in release builds")]
fn a_300_page_document_zooms_from_1_to_6400_percent_within_budget() {
    // The UI thread's work holds its budget on any renderer; the CPU renderer's raster is
    // recorded (a window whose CPU frames run long moves to the GPU).
    let cpu = run("the CPU renderer", Renderer::Cpu(Box::new(CpuCompositor::new())));
    for (what, t) in [("zoom in", &cpu.zoom_in), ("zoom out", &cpu.zoom_out), ("sharpening", &cpu.sharpen)] {
        let (_, p95, worst) = Times::summary(&t.ui);
        assert!(p95 <= UI_BUDGET, "{what}: p95 UI frame {p95:?} over {UI_BUDGET:?}");
        assert!(worst <= UI_BUDGET * 2, "{what}: worst UI frame {worst:?}");
    }
    // On a hardware GPU, whole frames (UI thread and GPU) fit the display's frame.
    let ctx = GpuContext::new(&LadderOptions::default());
    let Ok(ctx) = ctx else { unreachable!("every machine has at least the software rung") };
    let hardware = format!("{:?}", ctx.adapter.get_info().device_type) != "Cpu";
    let name = ctx.describe();
    let gpu = GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default());
    let target = gpu.renderer().create_target(WINDOW.width as u32, WINDOW.height as u32);
    assert!(target.is_ok(), "a frame target: {:?}", target.as_ref().err());
    let Ok(target) = target else { return };
    let gpu = run(&name, Renderer::Gpu(Box::new(gpu), target));
    for (what, t) in [("zoom in", &gpu.zoom_in), ("zoom out", &gpu.zoom_out), ("sharpening", &gpu.sharpen)] {
        let (_, p95, _) = Times::summary(&t.ui);
        assert!(p95 <= UI_BUDGET, "{what} on the GPU: p95 UI frame {p95:?} over {UI_BUDGET:?}");
        let (p50, whole95, worst) = Times::summary(&total(t));
        eprintln!("{name}, {what}: whole frames p50 {p50:?} p95 {whole95:?} worst {worst:?}");
        if hardware {
            assert!(whole95 <= FRAME, "{what} on {name}: p95 whole frame {whole95:?} over {FRAME:?}");
        }
    }
}
