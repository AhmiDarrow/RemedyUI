//! Prism effects as elements: an `.rfx` effect (fire, water, a starfield, a raymarched orb)
//! drawn in an element, by the GPU kernel in windows the GPU renderer draws and by the CPU
//! renderer elsewhere, with nothing for the app to decide.
//!
//! ```no_run
//! use remedy_ui_effects::{Instance, library};
//! use remedy_ui_media_effects::effect_view;
//! # fn build(ui: &remedy_ui_view::Ui) -> Result<(), Box<dyn std::error::Error>> {
//! let fire = library("fire").ok_or("no fire")?;
//! let (el, view) = effect_view(ui, Instance::new(fire));
//! ui.root().child(el);
//! view.set("intensity", 1.4)?; // from the app, a theme, or a script, at event rate
//! # Ok(()) }
//! ```
//!
//! - **Motion under the window's ambient policy.** The element draws a new frame at the
//!   effect's own rate (`fps` in the effect), never faster than the window's ambient rate,
//!   only while it is seen; it gets a compositor layer of its own, so a frame costs only the
//!   effect. In Light mode and while saving power, effects draw light (`light` reads 1, half
//!   the particles). Paused, Still, reduced motion, hidden: the effect's still frame (`still`
//!   in the effect), drawn once and kept.
//! - **Both renderers.** On the GPU the effect runs on Prism's fixed kernel into textures the
//!   element owns; on the CPU, on the interpreter, at the effect's resolution scale.
//! - **A quality ladder on the CPU.** If frames take longer than the budget
//!   ([`EffectView::set_cpu_budget`], 4 ms by default), the element steps down: half
//!   resolution, a quarter at no more than 15 frames a second, then the still frame. It steps
//!   back up when frames are cheap again. Heavy effects also ask for the GPU renderer.
//! - **Parameters** are set by name ([`EffectView::set`]), typed and clamped; each change
//!   draws a new frame. Pictures bind to texture parameters ([`EffectView::set_texture`]).
//!
//! Layer 5: built from public engine APIs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::time::Instant;
use remedy_ui_core::{CornerRadii, Rect};
use remedy_ui_effects::{CompileError, CpuEffect, Effect, FrameInput, Instance, Limits, ParamError, Value};
use remedy_ui_paint::{Blend, Image, Item, Sampling};
use remedy_ui_view::{El, ElementId, PaintCx, Ui, WeakUi};

/// Where the element's frames are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Backend {
    /// Nothing drawn yet.
    #[default]
    None,
    /// The GPU kernel.
    Gpu,
    /// The CPU interpreter.
    Cpu,
}

/// How the element is doing ([`EffectView::stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EffectStats {
    /// Frames rendered (still frames included).
    pub frames: u64,
    /// Still frames rendered.
    pub stills: u64,
    /// Where the latest frame was drawn.
    pub backend: Backend,
    /// The CPU quality step: 0 full, 1 half resolution, 2 a quarter at 15 fps, 3 still.
    pub level: u8,
    /// The latest frame's render time on the CPU (zero on the GPU, whose work is queued).
    pub last: Duration,
    /// Size of the latest picture, in pixels.
    pub pixels: (u32, u32),
}

/// The CPU quality ladder: resolution multiplier and frame-rate cap for each step.
const LADDER: [(f32, u32); 3] = [(1.0, u32::MAX), (0.5, u32::MAX), (0.25, 15)];
/// Steps that are cheap for this long climb back up.
const CLIMB_AFTER: u32 = 90;

#[cfg(feature = "gpu")]
struct GpuSide {
    ctx: remedy_ui_render_gpu::GpuContext,
    fx: remedy_ui_render_gpu::effects::GpuEffect,
    /// Two registered textures: drawn into in turn.
    buffers: Vec<(remedy_ui_render_gpu::wgpu::Texture, u64)>,
    front: usize,
    frame: u64,
}

#[cfg(feature = "gpu")]
impl GpuSide {
    fn release(&mut self) {
        for (_, id) in self.buffers.drain(..) {
            self.ctx.unregister_texture(id);
        }
    }
}

struct State {
    instance: Instance,
    cpu: CpuEffect,
    #[cfg(feature = "gpu")]
    gpu: Option<GpuSide>,
    /// The still frame drawn, with what it was drawn for.
    still: Option<((u32, u32, u64, bool), Item)>,
    stats: EffectStats,
    /// Recent CPU frame cost (moving average, seconds) and frames below the budget.
    cost: f64,
    cheap: u32,
    budget: Duration,
}

struct Shared {
    state: RefCell<State>,
    ui: WeakUi,
    id: Cell<Option<ElementId>>,
    closed: Cell<bool>,
}

/// The app's handle to an effect element. Cheap to clone; UI thread only.
#[derive(Clone)]
pub struct EffectView(Rc<Shared>);

impl std::fmt::Debug for EffectView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.0.state.borrow();
        write!(f, "EffectView({}, {:?})", s.instance.effect().name, s.stats)
    }
}

impl EffectView {
    fn changed(&self) {
        {
            let mut s = self.0.state.borrow_mut();
            s.still = None;
        }
        if let (Some(ui), Some(id)) = (self.0.ui.upgrade(), self.0.id.get())
            && !self.0.closed.get()
        {
            ui.element(id).repaint();
        }
    }

    /// Sets parameter `name` (clamped to its range) and draws a new frame.
    pub fn set(&self, name: &str, value: impl Into<Value>) -> Result<(), ParamError> {
        let changed = {
            let mut s = self.0.state.borrow_mut();
            let before = s.instance.version();
            s.instance.set(name, value)?;
            s.instance.version() != before
        };
        if changed {
            self.changed();
        }
        Ok(())
    }

    /// The value of parameter `name`.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.0.state.borrow().instance.get(name)
    }

    /// Binds a picture to texture parameter `name` and draws a new frame.
    pub fn set_texture(&self, name: &str, image: Option<Image>) -> Result<(), ParamError> {
        self.0.state.borrow_mut().instance.set_texture(name, image)?;
        self.changed();
        Ok(())
    }

    /// The effect.
    pub fn effect(&self) -> std::sync::Arc<Effect> {
        self.0.state.borrow().instance.effect().clone()
    }

    /// How the element is doing.
    pub fn stats(&self) -> EffectStats {
        self.0.state.borrow().stats
    }

    /// The most a CPU frame may take before the element lowers its quality (default 4 ms).
    pub fn set_cpu_budget(&self, budget: Duration) {
        self.0.state.borrow_mut().budget = budget;
    }

    /// False once the element is gone.
    pub fn is_open(&self) -> bool {
        !self.0.closed.get()
    }
}

/// An element drawing `instance`, and the handle to change it. The element grows to fill
/// its parent's free space by default; size it like any element.
#[track_caller]
pub fn effect_view(ui: &Ui, instance: Instance) -> (El, EffectView) {
    let heavy = instance.effect().cost > 400;
    let shared = Rc::new(Shared {
        state: RefCell::new(State {
            instance,
            cpu: CpuEffect::new(),
            #[cfg(feature = "gpu")]
            gpu: None,
            still: None,
            stats: EffectStats::default(),
            cost: 0.0,
            cheap: 0,
            budget: Duration::from_millis(4),
        }),
        ui: ui.downgrade(),
        id: Cell::new(None),
        closed: Cell::new(false),
    });
    // Heavy effects ask for the GPU renderer, where the app lets the runner choose.
    if heavy {
        ui.prefer_gpu();
    }
    let gpu_window = ui.gpu_rendering();
    let painting = shared.clone();
    let el = ui.el("effect").grow(1.0).paint(move |cx| {
        let on_gpu = gpu_window.get();
        paint(&painting, cx, on_gpu);
    });
    shared.id.set(Some(el.id()));
    let closing = shared.clone();
    let el = el.on_removed(move || {
        closing.closed.set(true);
        #[cfg(feature = "gpu")]
        if let Ok(mut s) = closing.state.try_borrow_mut()
            && let Some(mut g) = s.gpu.take()
        {
            g.release();
        }
    });
    (el, EffectView(shared))
}

/// Compiles `source` with `limits` ([`Limits::untrusted`] for themes and scripts) and makes
/// an element of it.
#[track_caller]
pub fn effect_from_source(ui: &Ui, source: &str, limits: &Limits) -> Result<(El, EffectView), CompileError> {
    let effect = Effect::compile_with(source, limits)?;
    Ok(effect_view(ui, Instance::new(effect.shared())))
}

fn paint(shared: &Shared, cx: &mut PaintCx<'_>, on_gpu: bool) {
    if shared.closed.get() {
        return;
    }
    let Ok(mut s) = shared.state.try_borrow_mut() else { return };
    let effect = s.instance.effect().clone();
    let level = usize::from(s.stats.level);
    let gpu_path = on_gpu && cfg!(feature = "gpu");
    // On the CPU the ladder may hold the effect still; on the GPU it runs at its own rate.
    let (res, fps_cap) =
        if gpu_path { (1.0, u32::MAX) } else { LADDER.get(level).copied().unwrap_or((1.0, 0)) };
    let live =
        if !gpu_path && level >= LADDER.len() { None } else { cx.ambient_at(effect.fps.min(fps_cap).max(1)) };
    let rect = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
    if rect.is_empty() {
        return;
    }
    let (time, light) = match live {
        Some(tick) => (tick.t.as_secs_f32(), tick.light),
        None => (effect.still, false),
    };
    let mut input = FrameInput::new(cx.size.width, cx.size.height, cx.scale * res, time);
    input.light = light;
    let blend = effect.blend;
    let push = |cx: &mut PaintCx<'_>, item: Item| {
        if blend != Blend::Normal {
            cx.list.push(Item::PushLayer { opacity: 1.0, blend });
            cx.list.push(item);
            cx.list.push(Item::PopLayer);
        } else {
            cx.list.push(item);
        }
    };
    // A still frame is drawn once for its size, parameters and renderer, then kept.
    let key = {
        let w = (cx.size.width * cx.scale * res).round() as u32;
        let h = (cx.size.height * cx.scale * res).round() as u32;
        (w, h, s.instance.version(), gpu_path)
    };
    if live.is_none()
        && let Some((k, item)) = &s.still
        && *k == key
    {
        let item = item.clone();
        push(cx, item);
        return;
    }
    let item = match draw(&mut s, &input, rect, gpu_path) {
        Some(i) => i,
        None => return,
    };
    s.stats.frames += 1;
    if live.is_none() {
        s.stats.stills += 1;
        s.still = Some((key, item.clone()));
    }
    push(cx, item);
}

/// Renders one frame on the chosen side, falling back to the CPU when the GPU cannot.
fn draw(s: &mut State, input: &FrameInput, rect: Rect, gpu_path: bool) -> Option<Item> {
    #[cfg(feature = "gpu")]
    if gpu_path && let Some(item) = draw_gpu(s, input, rect) {
        s.stats.backend = Backend::Gpu;
        s.stats.last = Duration::ZERO;
        return Some(item);
    }
    let _ = gpu_path;
    let started = Instant::now();
    let pic = s.cpu.render(&s.instance, input);
    let took = started.elapsed();
    s.stats.backend = Backend::Cpu;
    s.stats.last = took;
    s.stats.pixels = (pic.width, pic.height);
    // The ladder: a moving average against the budget.
    let secs = took.as_secs_f64();
    s.cost = if s.cost == 0.0 { secs } else { s.cost * 0.8 + secs * 0.2 };
    let budget = s.budget.as_secs_f64();
    if s.cost > budget && usize::from(s.stats.level) < LADDER.len() {
        s.stats.level += 1;
        s.cost = 0.0;
        s.cheap = 0;
    } else if s.cost < budget / 4.0 && s.stats.level > 0 {
        s.cheap += 1;
        if s.cheap > CLIMB_AFTER {
            s.stats.level -= 1;
            s.cheap = 0;
            s.cost = 0.0;
        }
    } else {
        s.cheap = 0;
    }
    let image = Image::new(pic.width, pic.height, pic.pixels.into());
    Some(Item::Image { image, rect, radii: CornerRadii::ZERO, sampling: Sampling::Linear })
}

#[cfg(feature = "gpu")]
fn draw_gpu(s: &mut State, input: &FrameInput, rect: Rect) -> Option<Item> {
    use remedy_ui_render_gpu::effects::GpuEffect;
    use remedy_ui_render_gpu::{FORMAT, GpuContext, LadderOptions, wgpu};
    let ctx = GpuContext::shared(&LadderOptions::default()).ok()?;
    if s.gpu.as_ref().is_none_or(|g| g.ctx.is_lost() || g.ctx.device_serial() != ctx.device_serial()) {
        if let Some(mut old) = s.gpu.take() {
            old.release();
        }
        s.gpu = Some(GpuSide {
            fx: GpuEffect::new(&ctx),
            ctx: ctx.clone(),
            buffers: Vec::new(),
            front: 0,
            frame: 0,
        });
    }
    let g = s.gpu.as_mut()?;
    let (w, h) = GpuEffect::output_size(s.instance.effect(), input);
    if g.buffers.first().is_none_or(|(t, _)| (t.width(), t.height()) != (w, h)) {
        g.release();
        for _ in 0..2 {
            let t = g.ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("RemedyUI effect"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let id = g.ctx.register_texture(&t).ok()?;
            g.buffers.push((t, id));
        }
    }
    let back = 1 - g.front;
    let (texture, id) = g.buffers.get(back)?;
    let ((), error) =
        g.ctx.catch_errors(|| g.fx.render(&g.ctx, &s.instance, input, texture).ok().unwrap_or(()));
    if error.is_some() {
        return None;
    }
    g.front = back;
    g.frame += 1;
    s.stats.pixels = (w, h);
    let texture = remedy_ui_paint::ExternalTexture { id: *id, frame: g.frame, width: w, height: h };
    Some(Item::Texture { texture, rect, radii: CornerRadii::ZERO })
}
