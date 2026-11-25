//! Rendering effects on the CPU: the reference for the GPU kernels, and what shows effects in
//! windows the CPU renderer draws.
//!
//! Also the frame logic both renderers share, so they cannot drift: the per-frame values
//! ([`uniforms`]), each program's constant table ([`constants`]), buffer sizes, blur radii,
//! and the particle simulation ([`particles`], which runs here for both).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::paint::Blend;

use crate::effects::Instance;
use crate::effects::ir::{Effect, Input, Program, Slot, Source, Splat, Stage, StageKind};
use crate::effects::vm::{Env, LANES, Reg, View, Vm, hash_cell, lane_of, pcg};

/// What a frame is drawn for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameInput {
    /// The element's size in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// Effect time in seconds (wrapped to the hour before programs see it).
    pub time: f32,
    /// Draw lighter: ambient light mode or power saving (half the particles; `light` reads 1).
    pub light: bool,
    /// The most a buffer may be on a side, in pixels (the renderer's texture limit). Default
    /// [`FrameInput::DEFAULT_MAX_SIDE`]; the GPU renderer also holds it to its device's limit.
    pub max_side: u32,
}

impl FrameInput {
    /// The side a buffer is held to unless the frame says otherwise (4096 pixels).
    pub const DEFAULT_MAX_SIDE: u32 = 4096;

    /// A frame of `width` × `height` logical pixels at `scale`, at `time`.
    pub fn new(width: f32, height: f32, scale: f32, time: f32) -> Self {
        FrameInput { width, height, scale, time, light: false, max_side: Self::DEFAULT_MAX_SIDE }
    }

    fn time(&self) -> f32 {
        if self.time.is_finite() { self.time.rem_euclid(3600.0) } else { 0.0 }
    }

    fn aspect(&self) -> f32 {
        if self.height > 0.0 { self.width / self.height } else { 1.0 }
    }
}

/// An effect's picture: premultiplied RGBA8, to be drawn stretched over the element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// The default of [`set_max_effect_pixels`]: 4 Mi pixels (16 MiB of RGBA8).
pub const MAX_EFFECT_PIXELS: u64 = 4 << 20;

static EFFECT_PIXELS: AtomicU64 = AtomicU64::new(MAX_EFFECT_PIXELS);
static OVERDRAW: AtomicU64 = AtomicU64::new(MAX_OVERDRAW);

/// Pixels all of one effect's buffers may hold together, on both renderers (default
/// [`MAX_EFFECT_PIXELS`]). A huge element draws its effect at a lower resolution rather than
/// allocating without bound. Raising it lets one effect take that many pixels of memory (four
/// bytes each, per buffer) and that much work a frame; applies from the next frame.
pub fn set_max_effect_pixels(pixels: u64) {
    EFFECT_PIXELS.store(pixels, Ordering::Relaxed);
}

/// The pixel budget of one effect now ([`set_max_effect_pixels`]).
pub fn max_effect_pixels() -> u64 {
    EFFECT_PIXELS.load(Ordering::Relaxed)
}

/// How many times their buffer's pixels particles may cover a frame, on both renderers
/// (default [`MAX_OVERDRAW`]); the rest are not drawn. Raising it lets a hostile emitter cost
/// that many more pixel writes a frame; applies from the next frame.
pub fn set_max_overdraw(times: u64) {
    OVERDRAW.store(times, Ordering::Relaxed);
}

/// The particle overdraw budget now ([`set_max_overdraw`]).
pub fn max_overdraw() -> u64 {
    OVERDRAW.load(Ordering::Relaxed)
}

/// The lowest device scale an effect is drawn at (for absurdly large elements).
const MIN_SCALE: f32 = 1.0e-4;

/// Particles may cover at most this many times their buffer's pixels a frame (counting each
/// particle's box); the rest are not drawn. Keeps a hostile emitter (thousands of huge
/// particles) from costing billions of pixel writes on either renderer. The default of
/// [`set_max_overdraw`].
pub const MAX_OVERDRAW: u64 = 8;

/// `input` made safe to render `effect` with: finite sizes and time, and a scale lowered
/// until every buffer fits within the texture limit and all of them within
/// [`max_effect_pixels`]. Both renderers render what this gives.
pub fn fit(effect: &Effect, input: &FrameInput) -> FrameInput {
    let mut f = *input;
    let clean = |v: f32, hi: f32| if v.is_finite() { v.clamp(0.0, hi) } else { 0.0 };
    f.width = clean(f.width, 100_000.0);
    f.height = clean(f.height, 100_000.0);
    f.scale = if f.scale.is_finite() && f.scale > 0.0 { f.scale.clamp(MIN_SCALE, 8.0) } else { 1.0 };
    f.max_side = f.max_side.max(1);
    let budget = max_effect_pixels().max(1);
    for _ in 0..8 {
        let total: u64 = effect
            .stages
            .iter()
            .map(|s| {
                let (w, h) = buffer_size(effect, s, &f);
                u64::from(w) * u64::from(h)
            })
            .sum();
        if total <= budget || f.scale <= MIN_SCALE {
            break;
        }
        f.scale = (f.scale * (budget as f32 / total as f32).sqrt() * 0.97).max(MIN_SCALE);
    }
    f
}

/// Drops the particles past the overdraw budget for a buffer of `buffer_px` pixels, `k`
/// buffer pixels per logical pixel ([`max_overdraw`]).
pub fn cap_overdraw(sprites: &mut Vec<Sprite>, k: f32, buffer_px: u64) {
    let budget = buffer_px.saturating_mul(max_overdraw()).max(65_536);
    let mut spent = 0u64;
    let keep = sprites
        .iter()
        .take_while(|s| {
            let r = (s.radius * k).max(0.5) + 1.0;
            let w = (s.trail[0] * k).abs() + 2.0 * r;
            let h = (s.trail[1] * k).abs() + 2.0 * r;
            spent += (w * h) as u64;
            spent <= budget
        })
        .count();
    sprites.truncate(keep);
}

/// The pixel size of `stage`'s buffer for `input`.
pub fn buffer_size(effect: &Effect, stage: &Stage, input: &FrameInput) -> (u32, u32) {
    let k = effect.scale * stage.scale * input.scale.clamp(MIN_SCALE, 8.0);
    let side = |v: f32| -> u32 {
        let px = (v.max(0.0) * k).round();
        if px.is_finite() { (px as u32).clamp(1, input.max_side.max(1)) } else { 1 }
    };
    (side(input.width), side(input.height))
}

/// The per-frame values (the effect's uniform program run once).
pub fn uniforms(effect: &Effect, instance: &Instance, input: &FrameInput) -> Vec<[f32; 4]> {
    let consts = constants(&effect.uniforms, instance.values(), &[]);
    let env = FrameEnv { input: *input };
    let mut vm = Vm::new();
    vm.run(&effect.uniforms, &consts, &env, 1);
    effect
        .uniforms
        .outputs
        .iter()
        .map(|r| {
            let reg = vm.reg(*r);
            [reg[0][0], reg[1][0], reg[2][0], reg[3][0]].map(|x| if x.is_finite() { x } else { 0.0 })
        })
        .collect()
}

/// `program`'s constant table for this frame.
pub fn constants(program: &Program, values: &[[f32; 4]], uniforms: &[[f32; 4]]) -> Vec<[f32; 4]> {
    program
        .consts
        .iter()
        .map(|s| match s {
            Slot::Value(v) => *v,
            Slot::Param(i) => values.get(*i).copied().unwrap_or([0.0; 4]),
            Slot::Uniform(i) => uniforms.get(*i).copied().unwrap_or([0.0; 4]),
        })
        .collect()
}

/// A per-frame value's first component, finite.
fn scalar(uniforms: &[[f32; 4]], i: usize) -> f32 {
    uniforms.get(i).map(|v| v[0]).filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// The blur radius of a blur stage in its buffer's pixels (at most 32).
pub fn blur_radius(
    effect: &Effect,
    stage: &Stage,
    radius_uniform: usize,
    uniforms: &[[f32; 4]],
    input: &FrameInput,
) -> f32 {
    let (w, _) = buffer_size(effect, stage, input);
    let per_logical = w as f32 / input.width.max(1.0);
    (scalar(uniforms, radius_uniform).max(0.0) * per_logical).min(32.0)
}

/// The Gaussian weights of a blur of `radius` buffer pixels, centre first.
pub fn blur_weights(radius: f32) -> Vec<f32> {
    if radius < 0.5 {
        return vec![1.0];
    }
    let n = radius.ceil().min(32.0) as usize;
    let sigma = (radius / 2.0).max(0.5);
    let raw: Vec<f32> = (0..=n).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = raw.first().copied().unwrap_or(1.0) + 2.0 * raw.iter().skip(1).sum::<f32>();
    raw.into_iter().map(|w| w / total).collect()
}

/// One particle, ready to draw: position and trail in logical pixels, radius in logical
/// pixels, straight-alpha colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    /// Centre.
    pub pos: [f32; 2],
    /// Radius.
    pub radius: f32,
    /// Colour, straight alpha.
    pub color: [f32; 4],
    /// The trail: the particle is drawn as a capsule from `pos` to `pos + trail`.
    pub trail: [f32; 2],
}

/// The particles of emitter stage `stage` this frame: every particle's program run, on the
/// CPU, for both renderers. Light frames draw half of them.
pub fn particles(
    _effect: &Effect,
    stage: &Stage,
    instance: &Instance,
    uniforms: &[[f32; 4]],
    input: &FrameInput,
) -> Vec<Sprite> {
    let StageKind::Particles { emitter, .. } = &stage.kind else { return Vec::new() };
    let count = if input.light { emitter.count.div_ceil(2) } else { emitter.count };
    let life = scalar(uniforms, emitter.life).clamp(0.05, 3600.0);
    let consts = constants(&emitter.program, instance.values(), uniforms);
    let mut vm = Vm::new();
    let mut out = Vec::with_capacity(count as usize);
    let time = input.time();
    let mut first = 0u32;
    while first < count {
        let n = ((count - first) as usize).min(LANES);
        let env = ParticleEnv { input: *input, first, count, life, time };
        vm.run(&emitter.program, &consts, &env, n);
        let reg = |k: usize| vm.reg(emitter.program.outputs.get(k).copied().unwrap_or(0));
        let (pos, radius, color, trail) = (reg(0), reg(1), reg(2), reg(3));
        for l in 0..n {
            let fin = |x: f32| if x.is_finite() { x.clamp(-1.0e5, 1.0e5) } else { 0.0 };
            let (p, r, c, t) = (lane_of(pos, l), lane_of(radius, l), lane_of(color, l), lane_of(trail, l));
            out.push(Sprite {
                pos: [fin(p[0]), fin(p[1])],
                radius: fin(r[0]).clamp(0.0, 512.0),
                color: c.map(|x| fin(x).clamp(0.0, 1.0)),
                trail: [fin(t[0]).clamp(-1024.0, 1024.0), fin(t[1]).clamp(-1024.0, 1024.0)],
            });
        }
        first += n as u32;
    }
    out
}

/// A particle's phase in its life: (seconds since it spawned, a random number for this
/// spawn).
pub fn particle_age(id: u32, life: f32, time: f32) -> (f32, f32) {
    let phase = (hash_cell(&[id as i32, 0x5eed]) >> 8) as f32 / 16_777_216.0 * life;
    let t = time + phase;
    let cycle = (t / life).floor();
    let age = t - cycle * life;
    let seed = (pcg(hash_cell(&[id as i32, cycle as i32])) >> 8) as f32 / 16_777_216.0;
    (age.clamp(0.0, life), seed)
}

/// How much of a particle covers a point `d` buffer pixels from its spine, for radius `r`.
pub fn splat_coverage(splat: Splat, d: f32, r: f32) -> f32 {
    let r = r.max(0.5);
    match splat {
        Splat::Soft => {
            let k = (1.0 - (d / r) * (d / r)).clamp(0.0, 1.0);
            k * k
        }
        Splat::Disc => (r - d + 0.5).clamp(0.0, 1.0),
        Splat::Ring => {
            let w = (r * 0.25).max(1.0);
            (1.0 - (d - (r - w)).abs() / w).clamp(0.0, 1.0)
        }
    }
}

/// Distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (pax, pay) = (p[0] - a[0], p[1] - a[1]);
    let (bax, bay) = (b[0] - a[0], b[1] - a[1]);
    let len = bax * bax + bay * bay;
    let h = if len > 1.0e-12 { ((pax * bax + pay * bay) / len).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt()
}

/// Blends premultiplied `src` onto premultiplied `dst`.
pub fn blend(mode: Blend, src: [f32; 4], dst: [f32; 4]) -> [f32; 4] {
    let (sa, da) = (src[3], dst[3]);
    let mut out = [0.0; 4];
    for ((o, s), d) in out.iter_mut().zip(src).zip(dst) {
        *o = match mode {
            Blend::Plus => s + d,
            Blend::Screen => s + d - s * d,
            Blend::Multiply => s * d + s * (1.0 - da) + d * (1.0 - sa),
            Blend::Normal => s + d * (1.0 - sa),
        }
        .clamp(0.0, 1.0);
    }
    out
}

struct FrameEnv {
    input: FrameInput,
}

impl Env for FrameEnv {
    fn input(&self, input: Input, out: &mut Reg, n: usize) {
        let i = &self.input;
        let v = match input {
            Input::Time => [i.time(); 4],
            Input::Size => [i.width, i.height, 0.0, 0.0],
            Input::Aspect => [i.aspect(); 4],
            Input::Light => [if i.light { 1.0 } else { 0.0 }; 4],
            _ => [0.0; 4],
        };
        fill(out, v, n);
    }

    fn sample(&self, _: Source, _: f32, _: f32) -> [f32; 4] {
        [0.0; 4]
    }
}

fn fill(out: &mut Reg, v: [f32; 4], n: usize) {
    for (comp, x) in out.iter_mut().zip(v) {
        comp.iter_mut().take(n).for_each(|o| *o = x);
    }
}

struct ParticleEnv {
    input: FrameInput,
    first: u32,
    count: u32,
    life: f32,
    time: f32,
}

impl Env for ParticleEnv {
    fn input(&self, input: Input, out: &mut Reg, n: usize) {
        match input {
            Input::Id | Input::Seed | Input::Age | Input::Progress => {
                for l in 0..n {
                    let id = self.first + l as u32;
                    let (age, seed) = particle_age(id, self.life, self.time);
                    let v = match input {
                        Input::Id => id as f32,
                        Input::Seed => seed,
                        Input::Age => age,
                        _ => age / self.life,
                    };
                    for comp in out.iter_mut() {
                        if let Some(o) = comp.get_mut(l) {
                            *o = v;
                        }
                    }
                }
            }
            Input::Life => fill(out, [self.life; 4], n),
            Input::Count => fill(out, [self.count as f32; 4], n),
            other => FrameEnv { input: self.input }.input(other, out, n),
        }
    }

    fn sample(&self, _: Source, _: f32, _: f32) -> [f32; 4] {
        [0.0; 4]
    }
}

/// A buffer: premultiplied RGBA8.
#[derive(Clone, Debug, Default)]
struct Buffer {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Buffer {
    fn view(&self) -> View<'_> {
        View { width: self.width, height: self.height, pixels: &self.pixels, repeat: false }
    }
}

struct PixelEnv<'a> {
    input: FrameInput,
    width: u32,
    height: u32,
    y: u32,
    x0: u32,
    textures: &'a [Option<View<'a>>],
    buffers: &'a [Buffer],
    feedback: Option<View<'a>>,
}

impl Env for PixelEnv<'_> {
    fn input(&self, input: Input, out: &mut Reg, n: usize) {
        let (w, h) = (self.width as f32, self.height as f32);
        let v = (self.y as f32 + 0.5) / h;
        match input {
            Input::Uv | Input::Px => {
                let (sx, sy) =
                    if input == Input::Px { (self.input.width, self.input.height) } else { (1.0, 1.0) };
                let [xs, ys, zs, ws] = out;
                for (l, ((x, y), (z, ww))) in
                    xs.iter_mut().zip(ys.iter_mut()).zip(zs.iter_mut().zip(ws.iter_mut())).enumerate().take(n)
                {
                    *x = (self.x0 as f32 + l as f32 + 0.5) / w * sx;
                    *y = v * sy;
                    *z = 0.0;
                    *ww = 0.0;
                }
            }
            Input::Texel => fill(out, [self.input.width.max(1.0) / w; 4], n),
            other => FrameEnv { input: self.input }.input(other, out, n),
        }
    }

    fn sample(&self, source: Source, u: f32, v: f32) -> [f32; 4] {
        match source {
            Source::Texture(i) => {
                self.textures.get(usize::from(i)).copied().flatten().map_or([0.0; 4], |t| t.sample(u, v))
            }
            Source::Buffer(i) => self.buffers.get(usize::from(i)).map_or([0.0; 4], |b| b.view().sample(u, v)),
            Source::Feedback => self.feedback.map_or([0.0; 4], |f| f.sample(u, v)),
        }
    }
}

fn to_u8(x: f32) -> u8 {
    if x.is_finite() { (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 }
}

/// Writes rows `rows` of a shade stage into `out` (those rows' bytes).
#[allow(clippy::too_many_arguments)]
fn shade_rows(
    program: &Program,
    consts: &[[f32; 4]],
    input: FrameInput,
    (width, height): (u32, u32),
    rows: std::ops::Range<u32>,
    textures: &[Option<View<'_>>],
    buffers: &[Buffer],
    feedback: Option<View<'_>>,
    out: &mut [u8],
) {
    let mut vm = Vm::new();
    let reg = program.outputs.first().copied().unwrap_or(0);
    let row_bytes = width as usize * 4;
    for (k, y) in rows.enumerate() {
        let Some(row) = out.get_mut(k * row_bytes..(k + 1) * row_bytes) else { continue };
        let mut x0 = 0u32;
        while x0 < width {
            let n = ((width - x0) as usize).min(LANES);
            let env = PixelEnv { input, width, height, y, x0, textures, buffers, feedback };
            vm.run(program, consts, &env, n);
            let c = vm.reg(reg);
            for l in 0..n {
                let [r, g, b, a] = lane_of(c, l);
                let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
                let i = (x0 as usize + l) * 4;
                if let Some(px) = row.get_mut(i..i + 4) {
                    for (p, v) in px.iter_mut().zip([r * a, g * a, b * a, a]) {
                        *p = to_u8(if v.is_finite() { v.clamp(0.0, a.max(0.0)) } else { 0.0 });
                    }
                }
            }
            x0 += n as u32;
        }
    }
}

/// Renders effects on the CPU. Keep one per element: it holds the buffers between frames
/// (and the last frame, for effects that read it).
#[derive(Debug, Default)]
pub struct CpuEffect {
    buffers: Vec<Buffer>,
    last: Option<Buffer>,
    threads: usize,
}

impl CpuEffect {
    /// A renderer with no buffers yet, using up to four threads for large buffers.
    pub fn new() -> Self {
        CpuEffect { buffers: Vec::new(), last: None, threads: 4 }
    }

    /// Uses at most `n` threads (1: render on the calling thread only).
    pub fn with_threads(mut self, n: usize) -> Self {
        self.threads = n.max(1);
        self
    }

    /// Forgets the last frame (an effect that reads it starts again).
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Renders `instance` for `input`.
    pub fn render(&mut self, instance: &Instance, input: &FrameInput) -> Picture {
        let effect: &Arc<Effect> = instance.effect();
        let fitted = fit(effect, input);
        let input = &fitted;
        let uniforms = uniforms(effect, instance, input);
        let textures: Vec<Option<View<'_>>> = instance
            .textures()
            .iter()
            .map(|t| {
                t.as_ref()
                    .filter(|i| {
                        i.pixels.len() == i.width as usize * i.height as usize * 4
                            && i.width > 0
                            && i.height > 0
                    })
                    .map(|i| View { width: i.width, height: i.height, pixels: &i.pixels, repeat: true })
            })
            .collect();
        let mut buffers: Vec<Buffer> = std::mem::take(&mut self.buffers);
        buffers.resize_with(effect.stages.len(), Buffer::default);
        for (si, stage) in effect.stages.iter().enumerate() {
            let (w, h) = buffer_size(effect, stage, input);
            let mut target = std::mem::take(buffers.get_mut(si).unwrap_or(&mut Buffer::default()));
            target.width = w;
            target.height = h;
            target.pixels.clear();
            target.pixels.resize(w as usize * h as usize * 4, 0);
            let earlier = buffers.get(..si).unwrap_or(&[]);
            match &stage.kind {
                StageKind::Shade(program) => {
                    let consts = constants(program, instance.values(), &uniforms);
                    let feedback = self.last.as_ref().map(Buffer::view);
                    let threads = if (w as usize * h as usize) < 16_384 { 1 } else { self.threads };
                    render_rows(
                        program,
                        &consts,
                        *input,
                        (w, h),
                        &textures,
                        earlier,
                        feedback,
                        &mut target.pixels,
                        threads,
                    );
                }
                StageKind::Blur { src, radius } => {
                    let r = blur_radius(effect, stage, *radius, &uniforms, input);
                    if let Some(s) = earlier.get(usize::from(*src)) {
                        blur(s, &mut target, r);
                    }
                }
                StageKind::Particles { src, emitter } => {
                    if let Some(s) = src.and_then(|s| earlier.get(usize::from(s))) {
                        resample(s, &mut target);
                    }
                    let mut sprites = particles(effect, stage, instance, &uniforms, input);
                    let k = w as f32 / input.width.max(1.0);
                    cap_overdraw(&mut sprites, k, u64::from(w) * u64::from(h));
                    for s in &sprites {
                        splat(&mut target, s, k, emitter.splat, emitter.blend);
                    }
                }
            }
            if let Some(slot) = buffers.get_mut(si) {
                *slot = target;
            }
        }
        let out = buffers.last().cloned().unwrap_or_default();
        self.buffers = buffers;
        if effect.feedback {
            self.last = Some(out.clone());
        }
        Picture { width: out.width, height: out.height, pixels: out.pixels }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_rows(
    program: &Program,
    consts: &[[f32; 4]],
    input: FrameInput,
    (w, h): (u32, u32),
    textures: &[Option<View<'_>>],
    buffers: &[Buffer],
    feedback: Option<View<'_>>,
    out: &mut [u8],
    threads: usize,
) {
    #[cfg(not(target_arch = "wasm32"))]
    if threads > 1 {
        let per = h.div_ceil(threads as u32).max(1);
        let row_bytes = w as usize * 4;
        std::thread::scope(|scope| {
            for (k, chunk) in out.chunks_mut(per as usize * row_bytes).enumerate() {
                let y0 = k as u32 * per;
                let rows = y0..(y0 + per).min(h);
                scope.spawn(move || {
                    shade_rows(program, consts, input, (w, h), rows, textures, buffers, feedback, chunk)
                });
            }
        });
        return;
    }
    let _ = threads;
    shade_rows(program, consts, input, (w, h), 0..h, textures, buffers, feedback, out);
}

/// Premultiplied bilinear sample, edge-clamped.
fn sample_premul(b: &Buffer, u: f32, v: f32) -> [f32; 4] {
    if b.width == 0 || b.height == 0 {
        return [0.0; 4];
    }
    let x = u * b.width as f32 - 0.5;
    let y = v * b.height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xx: i64, yy: i64| -> [f32; 4] {
        let xx = xx.clamp(0, i64::from(b.width) - 1);
        let yy = yy.clamp(0, i64::from(b.height) - 1);
        let i = ((yy * i64::from(b.width) + xx) * 4) as usize;
        match b.pixels.get(i..i + 4).and_then(|p| <&[u8; 4]>::try_from(p).ok()) {
            Some(p) => p.map(|c| f32::from(c) / 255.0),
            None => [0.0; 4],
        }
    };
    let (ix, iy) = (x0 as i64, y0 as i64);
    let (a, bb, c, d) = (at(ix, iy), at(ix + 1, iy), at(ix, iy + 1), at(ix + 1, iy + 1));
    crate::effects::vm::bilerp(a, bb, c, d, fx, fy)
}

/// `src` drawn into `dst` (its size), bilinear.
fn resample(src: &Buffer, dst: &mut Buffer) {
    let (w, h) = (dst.width, dst.height);
    for y in 0..h {
        for x in 0..w {
            let c = sample_premul(src, (x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let i = (y as usize * w as usize + x as usize) * 4;
            if let Some(px) = dst.pixels.get_mut(i..i + 4) {
                for (p, v) in px.iter_mut().zip(c) {
                    *p = to_u8(v);
                }
            }
        }
    }
}

/// A separable Gaussian blur of `src` into `dst` (its own size): a horizontal pass sampling
/// `src` at `dst`'s pixel spacing, quantized, then a vertical pass. The GPU does the same.
fn blur(src: &Buffer, dst: &mut Buffer, radius: f32) {
    let (w, h) = (dst.width, dst.height);
    let weights = blur_weights(radius);
    let mut mid = Buffer { width: w, height: h, pixels: vec![0; dst.pixels.len()] };
    let (dx, dy) = (1.0 / w as f32, 1.0 / h as f32);
    for y in 0..h {
        let v = (y as f32 + 0.5) * dy;
        for x in 0..w {
            let u = (x as f32 + 0.5) * dx;
            let mut acc = [0.0f32; 4];
            for (i, wt) in weights.iter().enumerate() {
                let taps: &[f32] = if i == 0 { &[0.0] } else { &[-(i as f32), i as f32] };
                for &t in taps {
                    let c = sample_premul(src, u + t * dx, v);
                    for (a, c) in acc.iter_mut().zip(c) {
                        *a += c * wt;
                    }
                }
            }
            let i = (y as usize * w as usize + x as usize) * 4;
            if let Some(px) = mid.pixels.get_mut(i..i + 4) {
                for (p, a) in px.iter_mut().zip(acc) {
                    *p = to_u8(a);
                }
            }
        }
    }
    for y in 0..h {
        let v = (y as f32 + 0.5) * dy;
        for x in 0..w {
            let u = (x as f32 + 0.5) * dx;
            let mut acc = [0.0f32; 4];
            for (i, wt) in weights.iter().enumerate() {
                let taps: &[f32] = if i == 0 { &[0.0] } else { &[-(i as f32), i as f32] };
                for &t in taps {
                    let c = sample_premul(&mid, u, v + t * dy);
                    for (a, c) in acc.iter_mut().zip(c) {
                        *a += c * wt;
                    }
                }
            }
            let i = (y as usize * w as usize + x as usize) * 4;
            if let Some(px) = dst.pixels.get_mut(i..i + 4) {
                for (p, a) in px.iter_mut().zip(acc) {
                    *p = to_u8(a);
                }
            }
        }
    }
}

/// Draws one particle into `buf`; `k` is buffer pixels per logical pixel.
fn splat(buf: &mut Buffer, s: &Sprite, k: f32, shape: Splat, mode: Blend) {
    let a = [s.pos[0] * k, s.pos[1] * k];
    let b = [(s.pos[0] + s.trail[0]) * k, (s.pos[1] + s.trail[1]) * k];
    let r = (s.radius * k).max(0.5);
    let pad = r + 1.0;
    let x0 = (a[0].min(b[0]) - pad).floor().max(0.0) as u32;
    let y0 = (a[1].min(b[1]) - pad).floor().max(0.0) as u32;
    let x1 = ((a[0].max(b[0]) + pad).ceil().max(0.0) as u32).min(buf.width);
    let y1 = ((a[1].max(b[1]) + pad).ceil().max(0.0) as u32).min(buf.height);
    let alpha = s.color[3];
    if alpha <= 0.0 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let d = segment_distance([x as f32 + 0.5, y as f32 + 0.5], a, b);
            let cov = splat_coverage(shape, d, r);
            if cov <= 0.0 {
                continue;
            }
            let sa = alpha * cov;
            let src = [s.color[0] * sa, s.color[1] * sa, s.color[2] * sa, sa];
            let i = (y as usize * buf.width as usize + x as usize) * 4;
            if let Some(px) = buf.pixels.get_mut(i..i + 4).and_then(|p| <&mut [u8; 4]>::try_from(p).ok()) {
                let dst = px.map(|c| f32::from(c) / 255.0);
                let o = blend(mode, src, dst);
                for (p, v) in px.iter_mut().zip(o) {
                    *p = to_u8(v);
                }
            }
        }
    }
}
