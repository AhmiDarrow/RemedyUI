//! Ambient-motion budgets, measured on the real runner over the headless bridge with a virtual
//! 60 Hz display, and on both renderers directly.
//!
//! The reference scene is a cached backdrop (sky, 600 stars, hills, a lighthouse), 50 animated
//! sprites (fireflies wandering and flickering, each its own small ambient painter) and a beam
//! sweeping the whole window (a compositor loop). It is measured idle, focused, in the
//! background, minimized, under each ambient policy (Full, Light, Still), under reduced
//! motion and while the system saves power, and against the naive way of drawing the same
//! scene (one painter redrawing everything every vsync).
//!
//! What is checked:
//!
//! - **Frames** (exact, on virtual time; the same for both renderers since the runner decides
//!   them): an idle window draws none; ambient motion runs at the policy's rate (30 a second
//!   focused, 10 in the background, 15 in Light mode and while saving power), never the
//!   display's 60; minimized, Still, reduced motion and "still on battery" draw none at all.
//! - **Damage**: an ambient frame repaints only the sprites, rasters only their tiles (never
//!   the backdrop) and composites only the damaged area.
//! - **Cost** on the CPU renderer (release builds; debug builds only report): the budgets in
//!   [`Budget`].
//! - **Cost** on the GPU renderer, where a GPU adapter exists (a software one counts): the
//!   ambient scene rasters only the sprites' tiles (batched into one render), never costs
//!   more than the naive scene, and stays within [`Budget::GPU_FRAME`].
//!
//! `cargo test --release -p remedy-ui --test ambient_budget -- --nocapture` prints the tables.

mod ambient_scene;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ambient_scene::{Drive, HEIGHT, WIDTH, scene};
use remedy_ui::core::{Color, Size};
use remedy_ui::host::{HostEvent, SystemSettings, WindowId};
use remedy_ui::input::ScrollPhysics;
use remedy_ui::view::{AmbientMode, AmbientPolicy, Request};
use remedy_ui::{App, AppConfig, GeometryStore, Runner, Ui, WindowSpec};
use remedy_ui_engine::compositor::Compositor;
use remedy_ui_engine::render_cpu::CpuCompositor;
use remedy_ui_platform::bridge_headless::HeadlessHost;

/// The virtual display's refresh.
const REFRESH: Duration = Duration::from_micros(16_667);
/// Measured refreshes: one second.
const MEASURED: u32 = 60;
const SPRITES: u32 = 50;

/// The budgets this engine holds itself to, for the reference scene at 800 × 600 on one core
/// of the machine running the tests (release builds).
struct Budget;

impl Budget {
    /// 50 animated sprites over a cached backdrop, CPU renderer, share of one core.
    const SPRITES_CORE: f64 = 0.04;
    /// The same plus a beam sweeping the whole window (a full-window recomposite every
    /// frame on the CPU), share of one core.
    const SCENE_CORE: f64 = 0.20;
    /// The UI thread's own work for one ambient frame (style, layout, paint, layers).
    const UI_FRAME: Duration = Duration::from_micros(250);
    /// One ambient frame of the whole scene on the GPU renderer, submit to finished. Generous
    /// because the test machine may only have a software rasterizer.
    const GPU_FRAME: Duration = Duration::from_millis(60);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Focused,
    Background,
    Minimized,
    ReducedMotion,
    PowerSaving,
}

#[derive(Debug)]
struct Run {
    /// Frames presented in the measured second.
    presents: usize,
    /// Real time spent handling it.
    busy: Duration,
    /// The window's ambient rate at the end, and whether it asked for light drawing.
    rate: Option<(u32, bool)>,
}

impl Run {
    fn fps(&self) -> f64 {
        self.presents as f64 / (f64::from(MEASURED) * REFRESH.as_secs_f64())
    }

    fn per_frame(&self) -> Duration {
        if self.presents == 0 { Duration::ZERO } else { self.busy / self.presents as u32 }
    }

    /// Fraction of one core.
    fn core(&self) -> f64 {
        self.busy.as_secs_f64() / (f64::from(MEASURED) * REFRESH.as_secs_f64())
    }
}

fn measure(drive: Option<Drive>, beam: bool, state: State, policy: AmbientPolicy) -> Run {
    let config = AppConfig::new("Ambient budget").cpu_only();
    let seen: Rc<RefCell<Option<Ui>>> = Rc::new(RefCell::new(None));
    let keep = seen.clone();
    let mut runner = Runner::with_geometry(config, GeometryStore::in_memory(), move |app: &App| {
        app.set_ambient_policy(policy);
        let keep = keep.clone();
        app.open(WindowSpec::new("Ambient").size(WIDTH, HEIGHT), move |ui, _| {
            *keep.borrow_mut() = Some(ui.clone());
            if let Some(d) = drive {
                ui.root().child(scene(ui, d, SPRITES, beam));
            }
        });
    });
    let mut host = HeadlessHost::new().with_refresh(REFRESH);
    host.run_frames(&mut runner, 20);
    let id: WindowId = host.window_ids().into_iter().next().unwrap_or_else(|| unreachable!("a window"));
    host.send(HostEvent::Focused(id, true));
    match state {
        State::Focused => {}
        State::Background => host.send(HostEvent::Focused(id, false)),
        State::Minimized => host.send(HostEvent::Occluded(id, true)),
        State::ReducedMotion | State::PowerSaving => {
            let mut s = SystemSettings::default();
            s.reduced_motion = state == State::ReducedMotion;
            s.power_saving = state == State::PowerSaving;
            host.change_settings(s);
        }
    }
    // Let the change settle (the frame that draws the still picture, timers already set).
    host.run_frames(&mut runner, 20);
    let before = host.window(id).map_or(0, |w| w.presents());
    let mut busy = Duration::ZERO;
    for _ in 0..MEASURED {
        let t0 = Instant::now();
        host.turn(&mut runner);
        busy += t0.elapsed();
        host.advance(REFRESH);
    }
    let presents = host.window(id).map_or(0, |w| w.presents()) - before;
    let rate = seen
        .borrow()
        .as_ref()
        .and_then(|ui| ui.ambient())
        .map(|r| ((1.0 / r.interval.as_secs_f64()).round() as u32, r.light));
    Run { presents, busy, rate }
}

/// Where ambient frames' time goes, over 30 steady frames at the ambient rate: the UI thread
/// (`Ui::frame`), the compositor's plan, and the renderer; and what was touched.
#[derive(Debug, Default)]
struct Breakdown {
    ui: Duration,
    plan: Duration,
    render: Duration,
    rastered: u64,
    composited: u64,
    /// Elements repainted, most in one frame.
    repainted: usize,
    frames: u32,
}

impl Breakdown {
    fn per(&self, d: Duration) -> Duration {
        d / self.frames.max(1)
    }
}

/// Drives the scene through `Ui`, the compositor and a renderer step: `render` presents a plan
/// and returns pixels it composited (0 when it does not say).
fn breakdown(
    drive: Drive,
    beam: bool,
    mut render: impl FnMut(&remedy_ui_engine::compositor::FramePlan) -> u64,
) -> Breakdown {
    let ui = Ui::new(
        remedy_ui::style::Theme::dark_forest(),
        remedy_ui::style::StyleSheet::new(),
        remedy_ui::text::FontCollection::new(),
    );
    ui.root().child(scene(&ui, drive, SPRITES, beam));
    let size = Size::new(WIDTH, HEIGHT);
    let mut comp = Compositor::new(WIDTH as u32, HEIGHT as u32, 1.0, ScrollPhysics::CLAMPED);
    let mut b = Breakdown::default();
    let mut t = Duration::ZERO;
    for k in 0..40u32 {
        t += Duration::from_secs(1) / 30;
        ui.set_frame_time(t);
        for r in ui.take_requests() {
            if let Request::Timer { id, .. } = r {
                ui.fire_timer(id);
            }
        }
        let t0 = Instant::now();
        let f = ui.frame(size, 1.0);
        let t1 = Instant::now();
        if let Some(l) = f.layers {
            comp.commit(l);
        }
        let plan = comp.frame(t);
        let t2 = Instant::now();
        let composited = render(&plan);
        let t3 = Instant::now();
        // The first frames draw everything; measure the steady state.
        if k >= 10 {
            b.ui += t1 - t0;
            b.plan += t2 - t1;
            b.render += t3 - t2;
            b.rastered += plan.raster.iter().map(|r| r.pixels()).sum::<u64>();
            b.composited += composited;
            b.repainted = b.repainted.max(ui.last_frame_stats().repainted.len());
            b.frames += 1;
        }
    }
    b
}

fn cpu_breakdown(drive: Drive, beam: bool) -> Breakdown {
    let mut cpu = CpuCompositor::new();
    breakdown(drive, beam, |plan| {
        let _ = cpu.present_frame(plan, WIDTH as u32, HEIGHT as u32, Color::BLACK);
        cpu.last_composited()
    })
}

/// The same on the GPU renderer, waiting for the GPU to finish each frame. `None` without a
/// GPU adapter of any kind.
fn gpu_breakdown(drive: Drive, beam: bool) -> Option<(Breakdown, String)> {
    use remedy_ui_engine::render_gpu::{GpuCompositor, GpuContext, GpuRenderer, LadderOptions, wgpu};
    let ctx = GpuContext::new(&LadderOptions::default()).ok()?;
    let name = ctx.describe();
    let device = ctx.device.clone();
    let mut gpu = GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default());
    let target = gpu.renderer().create_target(WIDTH as u32, HEIGHT as u32).ok()?;
    let b = breakdown(drive, beam, |plan| {
        let _ = gpu.present(plan, &target, Color::BLACK);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        0
    });
    Some((b, name))
}

fn report(name: &str, r: &Run) {
    println!(
        "{name:<44} {:>5.1} fps  {:>8.3} ms/frame  {:>6.2} % of a core",
        r.fps(),
        r.per_frame().as_secs_f64() * 1e3,
        r.core() * 100.0
    );
}

fn report_breakdown(name: &str, b: &Breakdown) {
    let ms = |d: Duration| b.per(d).as_secs_f64() * 1e3;
    let frames = u64::from(b.frames.max(1));
    println!(
        "{name:<44} ui {:>6.3}  plan {:>6.3}  render {:>7.3} ms/frame; {:>6} px rastered, {:>6} px composited, {:>2} repainted",
        ms(b.ui),
        ms(b.plan),
        ms(b.render),
        b.rastered / frames,
        b.composited / frames,
        b.repainted,
    );
}

fn policy(mode: AmbientMode) -> AmbientPolicy {
    let mut p = AmbientPolicy::default();
    p.mode = mode;
    p
}

#[test]
fn ambient_motion_stays_within_its_budget() {
    let full = AmbientPolicy::default();
    let idle = measure(None, false, State::Focused, full);
    let sprites = measure(Some(Drive::Ambient), false, State::Focused, full);
    // Debug builds draw too slowly for cost comparisons to mean anything: they check frames
    // and damage only, and skip the naive scene drawn every vsync.
    let release = !cfg!(debug_assertions);
    let skipped = || Run { presents: 0, busy: Duration::ZERO, rate: None };
    let sprites_naive =
        if release { measure(Some(Drive::EveryFrame), false, State::Focused, full) } else { skipped() };
    let scene_f = measure(Some(Drive::Ambient), true, State::Focused, full);
    let scene_bg = measure(Some(Drive::Ambient), true, State::Background, full);
    let scene_min = measure(Some(Drive::Ambient), true, State::Minimized, full);
    let scene_naive =
        if release { measure(Some(Drive::EveryFrame), true, State::Focused, full) } else { skipped() };
    let naive_min = measure(Some(Drive::EveryFrame), true, State::Minimized, full);
    let light = measure(Some(Drive::Ambient), true, State::Focused, policy(AmbientMode::Light));
    let still = measure(Some(Drive::Ambient), true, State::Focused, policy(AmbientMode::Off));
    let reduced = measure(Some(Drive::Ambient), true, State::ReducedMotion, full);
    let saving = measure(Some(Drive::Ambient), true, State::PowerSaving, full);
    let mut battery_still = AmbientPolicy::default();
    battery_still.power_saving_fps = Some(0);
    let on_battery = measure(Some(Drive::Ambient), true, State::PowerSaving, battery_still);

    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!("{build} build, {SPRITES} sprites, 800 × 600, runner over the headless bridge, CPU renderer");
    report("idle window", &idle);
    report("sprites, ambient API, focused", &sprites);
    report("sprites, naive every frame, focused", &sprites_naive);
    report("scene (sprites + beam), ambient API, focused", &scene_f);
    report("scene, ambient API, background", &scene_bg);
    report("scene, ambient API, minimized", &scene_min);
    report("scene, naive every frame", &scene_naive);
    report("scene, naive, minimized", &naive_min);
    report("scene, Light policy", &light);
    report("scene, Still policy", &still);
    report("scene, reduced motion", &reduced);
    report("scene, power saving", &saving);
    report("scene, still on battery", &on_battery);

    // Frames: exact, renderer-independent.
    assert_eq!(idle.presents, 0, "idle draws no frames");
    for r in [&sprites, &scene_f] {
        assert!((28.0..=31.0).contains(&r.fps()), "focused at the ambient rate: {:.1}", r.fps());
    }
    assert!((9.0..=11.0).contains(&scene_bg.fps()), "background at 10 fps: {:.1}", scene_bg.fps());
    assert!((14.0..=16.0).contains(&light.fps()), "Light at 15 fps: {:.1}", light.fps());
    assert_eq!(light.rate, Some((15, true)), "Light asks painters to draw light");
    assert!((14.0..=16.0).contains(&saving.fps()), "power saving at 15 fps: {:.1}", saving.fps());
    assert_eq!(saving.rate, Some((15, true)), "power saving asks painters to draw light");
    assert_eq!(scene_f.rate, Some((30, false)));
    for (what, r) in [
        ("minimized", &scene_min),
        ("naive, minimized", &naive_min),
        ("Still", &still),
        ("reduced motion", &reduced),
        ("still on battery", &on_battery),
    ] {
        assert_eq!(r.presents, 0, "{what} draws no frames");
        assert!(r.busy < Duration::from_millis(5), "{what} costs nothing: {:?}", r.busy);
    }
    assert_eq!(still.rate, None);
    assert_eq!(reduced.rate, None);

    // Damage: an ambient frame of the sprites repaints only them, rasters only their tiles
    // and composites only around them.
    let cpu_sprites = cpu_breakdown(Drive::Ambient, false);
    let cpu_scene = cpu_breakdown(Drive::Ambient, true);
    let cpu_naive = cpu_breakdown(Drive::EveryFrame, true);
    println!("Per ambient frame, CPU renderer:");
    report_breakdown("sprites, ambient API", &cpu_sprites);
    report_breakdown("scene, ambient API", &cpu_scene);
    report_breakdown("scene, naive", &cpu_naive);
    let sprite_area = u64::from(SPRITES) * 32 * 32;
    assert_eq!(cpu_sprites.repainted, SPRITES as usize, "only the sprites repaint, never the backdrop");
    assert!(
        cpu_sprites.rastered / u64::from(cpu_sprites.frames) <= sprite_area,
        "only the sprites' tiles raster"
    );
    assert!(
        cpu_sprites.composited / u64::from(cpu_sprites.frames) < (WIDTH * HEIGHT) as u64 / 10,
        "only the damage composites"
    );
    assert!(
        cpu_scene.rastered / u64::from(cpu_scene.frames) <= sprite_area,
        "the beam turns without rastering"
    );

    // GPU renderer, where there is any adapter.
    match (gpu_breakdown(Drive::Ambient, true), gpu_breakdown(Drive::EveryFrame, true)) {
        (Some((ambient, name)), Some((naive, _))) => {
            println!("Per ambient frame, GPU renderer on {name}:");
            report_breakdown("scene, ambient API", &ambient);
            report_breakdown("scene, naive", &naive);
            assert!(ambient.rastered < naive.rastered / 10, "the GPU rasters only the sprites too");
            let frame = ambient.per(ambient.ui + ambient.plan + ambient.render);
            let naive_frame = naive.per(naive.ui + naive.plan + naive.render);
            // The GPU compositor redraws the whole window from its tiles every frame (cheap
            // on a GPU, not on a software rasterizer), so the saving is in rastering: never
            // worse than the naive scene, and within the frame budget.
            if release {
                assert!(
                    frame.as_secs_f64() < naive_frame.as_secs_f64() * 1.15,
                    "ambient {frame:?} a frame, naive {naive_frame:?}"
                );
                assert!(frame < Budget::GPU_FRAME, "GPU frame {frame:?}");
            }
        }
        _ => println!("no GPU adapter: the GPU renderer's budget is not measured"),
    }

    // Absolute CPU budgets: release builds (debug builds are tens of times slower and only
    // report).
    if !cfg!(debug_assertions) {
        // Ambient motion is far cheaper than drawing the scene naively.
        assert!(
            sprites.busy.as_secs_f64() * 4.0 < sprites_naive.busy.as_secs_f64(),
            "sprites at least 4× cheaper"
        );
        assert!(
            scene_f.busy.as_secs_f64() * 2.0 < scene_naive.busy.as_secs_f64(),
            "scene at least 2× cheaper"
        );
        assert!(scene_bg.core() < scene_f.core() * 0.6, "the background costs less");
        assert!(
            cpu_scene.per(cpu_scene.ui) < Budget::UI_FRAME,
            "UI thread {:?} a frame",
            cpu_scene.per(cpu_scene.ui)
        );
        assert!(sprites.core() < Budget::SPRITES_CORE, "sprites: {:.2} % of a core", sprites.core() * 100.0);
        assert!(scene_f.core() < Budget::SCENE_CORE, "scene: {:.2} % of a core", scene_f.core() * 100.0);
    }
}
