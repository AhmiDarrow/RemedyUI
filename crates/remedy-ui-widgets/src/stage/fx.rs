//! Shader-free effects for animated scenes: what themes keep needing, cheap by construction.
//!
//! - **Curves** ([`flicker`], [`twinkle`], [`breathe`], [`noise1`]): pure functions of time and
//!   a seed, so a hundred stars twinkle out of step for the cost of a few multiplications, and
//!   a paused scene resumes exactly.
//! - **Noise** ([`value_noise`], [`perlin`], [`fbm`]) and [`noise_image`]: a seeded noise
//!   texture coloured through a ramp, made once per spec and kept (tileable, so it scrolls
//!   seamlessly).
//! - **Light** ([`glow`], [`light_cone`]): radial and linear gradients drawn with additive
//!   blending ([`Blend::Plus`]), so glows stack the way light does.
//! - **Soft art** ([`blur`]): a picture blurred once (three box passes, close to a Gaussian)
//!   and kept: bloom around a lantern, a soft shadow, haze.
//! - **Moving textures** ([`scroll_tiled`], [`shimmer`]): a cached texture drawn tiled at an
//!   offset that moves with time: water, mist, clouds. Moving costs a few image draws, never
//!   a new texture.
//!
//! Draw static art once with `PaintCx::cached`, animate with `PaintCx::ambient` (or a
//! compositor loop, `El::ambient_loop`, for pure transform and opacity), and use these for the
//! pieces in between. Every function here is deterministic: the same inputs draw the same
//! pixels, so goldens and tests hold.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::Arc;

use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_engine::paint::{Blend, Brush, DisplayList, Image, Item, Path, Sampling, Stop};

/// A well-mixed 32-bit hash of `x` (PCG output permutation).
pub fn hash(x: u32) -> u32 {
    let state = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// A random number in `0..1` for item `i` of seed `seed` (where a star sits, how fast it
/// twinkles): the same inputs give the same number.
pub fn rand(seed: u32, i: u32) -> f32 {
    (hash(hash(seed) ^ i.wrapping_mul(0x9E37_79B9)) >> 8) as f32 / (1u32 << 24) as f32
}

fn lattice(x: i32, y: i32, seed: u32) -> u32 {
    hash((x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ hash(seed))
}

fn smooth(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Wraps lattice coordinates for tileable noise (`period` 0: no wrapping).
fn wrap(v: i32, period: i32) -> i32 {
    if period > 0 { v.rem_euclid(period) } else { v }
}

fn value_at(x: f32, y: f32, seed: u32, period: i32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - x0), smooth(y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let v = |dx: i32, dy: i32| {
        (lattice(wrap(ix + dx, period), wrap(iy + dy, period), seed) >> 8) as f32 / (1u32 << 24) as f32
    };
    let top = v(0, 0) + (v(1, 0) - v(0, 0)) * fx;
    let bottom = v(0, 1) + (v(1, 1) - v(0, 1)) * fx;
    top + (bottom - top) * fy
}

fn perlin_at(x: f32, y: f32, seed: u32, period: i32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (ix, iy) = (x0 as i32, y0 as i32);
    let (rx, ry) = (x - x0, y - y0);
    let grad = |dx: i32, dy: i32| {
        let a = (lattice(wrap(ix + dx, period), wrap(iy + dy, period), seed) >> 8) as f32
            / (1u32 << 24) as f32
            * TAU;
        let (s, c) = a.sin_cos();
        c * (rx - dx as f32) + s * (ry - dy as f32)
    };
    let (u, v) = (smooth(rx), smooth(ry));
    let top = grad(0, 0) + (grad(1, 0) - grad(0, 0)) * u;
    let bottom = grad(0, 1) + (grad(1, 1) - grad(0, 1)) * u;
    // Gradient noise stays within about ±0.71: scale it to 0..1.
    ((top + (bottom - top) * v) * 0.707 + 0.5).clamp(0.0, 1.0)
}

/// Smooth value noise at `(x, y)` (one lattice cell per unit), in `0..1`: blobby, cheap.
pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    value_at(x, y, seed, 0)
}

/// Perlin gradient noise at `(x, y)`, in `0..1` (0.5 on lattice points): less blobby than
/// value noise, for water, clouds and fire.
pub fn perlin(x: f32, y: f32, seed: u32) -> f32 {
    perlin_at(x, y, seed, 0)
}

/// Fractal noise: `octaves` layers of Perlin noise, each twice as fine and half as strong, in
/// `0..1`.
pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    fbm_at(x, y, octaves, seed, 0)
}

fn fbm_at(x: f32, y: f32, octaves: u32, seed: u32, period: i32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0);
    for o in 0..octaves.clamp(1, 12) {
        let p = if period > 0 { period * f as i32 } else { 0 };
        sum += amp * perlin_at(x * f, y * f, seed.wrapping_add(o), p);
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// One-dimensional smooth noise of time `t` (seconds), in `0..1`, changing about once a
/// second: the base of the curves below.
pub fn noise1(t: f32, seed: u32) -> f32 {
    value_at(t, 0.5, seed, 0)
}

/// A fire's brightness at time `t` (seconds), between about 0.72 and 1: quick irregular
/// flutter over a slower swell. Give each flame its own `seed`.
pub fn flicker(t: f32, seed: u32) -> f32 {
    let fast = noise1(t * 9.0, seed);
    let slow = noise1(t * 1.7, seed ^ 0x5bd1);
    (0.72 + 0.18 * slow + 0.10 * fast).clamp(0.0, 1.0)
}

/// A star's brightness at time `t` (seconds), in `0.25..1`: mostly steady, now and then a
/// soft dip, each star (`seed`) at its own pace.
pub fn twinkle(t: f32, seed: u32) -> f32 {
    let pace = 0.15 + 0.35 * rand(seed, 1);
    let n = noise1(t * pace + 13.0 * rand(seed, 2), seed);
    // Bias toward bright: dips are short.
    (0.25 + 0.75 * (1.0 - (1.0 - n) * (1.0 - n) * 1.4).clamp(0.0, 1.0)).clamp(0.25, 1.0)
}

/// A slow breath at time `t`: 0 to 1 and back every `period` seconds, eased.
pub fn breathe(t: f32, period: f32) -> f32 {
    if period > 0.0 { 0.5 - 0.5 * (t / period * TAU).cos() } else { 0.0 }
}

/// What a [`noise_image`] looks like.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NoiseSpec {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Noise cells across the width (bigger: finer detail). Whole numbers tile seamlessly.
    pub cells: u32,
    /// Fractal octaves (1: plain Perlin).
    pub octaves: u32,
    /// The seed.
    pub seed: u32,
    /// Colors from noise 0 to noise 1.
    pub ramp: Vec<Stop>,
}

impl NoiseSpec {
    /// A `width` × `height` noise of `cells` cells across, from transparent to `color`.
    pub fn new(width: u32, height: u32, cells: u32, color: Color) -> Self {
        NoiseSpec {
            width,
            height,
            cells,
            octaves: 4,
            seed: 1,
            ramp: vec![Stop { offset: 0.0, color: color.with_alpha(0.0) }, Stop { offset: 1.0, color }],
        }
    }

    /// With `octaves` layers.
    pub fn octaves(mut self, octaves: u32) -> Self {
        self.octaves = octaves;
        self
    }

    /// With `seed`.
    pub fn seed(mut self, seed: u32) -> Self {
        self.seed = seed;
        self
    }

    /// Colored through `ramp` (stops in order of offset).
    pub fn ramp(mut self, ramp: Vec<Stop>) -> Self {
        self.ramp = ramp;
        self
    }

    fn key(&self) -> u64 {
        let mut h = u64::from(hash(
            self.width ^ hash(self.height ^ hash(self.cells ^ hash(self.octaves ^ hash(self.seed)))),
        ));
        for s in &self.ramp {
            let [r, g, b, a] = s.color.to_rgba8();
            h = h.wrapping_mul(0x100_0000_01B3)
                ^ u64::from(hash(s.offset.to_bits() ^ u32::from_le_bytes([r, g, b, a])));
        }
        h
    }
}

fn ramp_at(ramp: &[Stop], v: f32) -> Color {
    let Some(first) = ramp.first() else { return Color::TRANSPARENT };
    if v <= first.offset {
        return first.color;
    }
    for w in ramp.windows(2) {
        if let [a, b] = w
            && v <= b.offset
        {
            let span = (b.offset - a.offset).max(1e-6);
            let t = (v - a.offset) / span;
            let mix = |x: f32, y: f32| x + (y - x) * t;
            return Color::new(
                mix(a.color.r, b.color.r),
                mix(a.color.g, b.color.g),
                mix(a.color.b, b.color.b),
                mix(a.color.a, b.color.a),
            );
        }
    }
    ramp.last().map_or(Color::TRANSPARENT, |s| s.color)
}

thread_local! {
    static NOISE: RefCell<HashMap<u64, Image>> = RefCell::new(HashMap::new());
    static BLURRED: RefCell<HashMap<(u64, u32), Image>> = RefCell::new(HashMap::new());
}

/// Most textures [`noise_image`] and [`blur`] keep each unless the app says otherwise
/// ([`set_fx_limits`]); past it the cache starts again.
pub const FX_KEPT: usize = 64;

/// The largest noise texture [`noise_image`] makes unless the app says otherwise
/// ([`set_fx_limits`]), in pixels a side.
pub const FX_NOISE_SIDE: u32 = 4096;

static KEEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(FX_KEPT);
static NOISE_SIDE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(FX_NOISE_SIDE);

/// Sets how many textures [`noise_image`] and [`blur`] keep each on a UI thread (default
/// [`FX_KEPT`], 64) and the largest noise texture made, a side (default [`FX_NOISE_SIDE`],
/// 4096). Each kept texture costs its pixels at 4 bytes each; a noise side costs with its
/// square once, and must stay within the renderer's texture limit to be drawn on the GPU.
pub fn set_fx_limits(kept: usize, noise_side: u32) {
    KEEP.store(kept, std::sync::atomic::Ordering::Relaxed);
    NOISE_SIDE.store(noise_side, std::sync::atomic::Ordering::Relaxed);
}

/// Keeps `img` under `key` in `cache`, starting the cache again past the limit.
fn keep<K: std::hash::Hash + Eq>(cache: &mut HashMap<K, Image>, key: K, img: &Image) {
    let max = KEEP.load(std::sync::atomic::Ordering::Relaxed);
    if cache.len() >= max {
        cache.clear();
    }
    if max > 0 {
        cache.insert(key, img.clone());
    }
}

/// A seeded noise texture colored through the spec's ramp, made the first time a spec is asked
/// for and then shared (the same spec gives the same [`Image`], so the GPU uploads it once).
/// It tiles seamlessly when `cells` is whole, so [`scroll_tiled`] can move it forever. `None`
/// for an empty or huge size (over [`FX_NOISE_SIDE`], 4096, a side, unless [`set_fx_limits`]
/// says otherwise).
pub fn noise_image(spec: &NoiseSpec) -> Option<Image> {
    let (w, h) = (spec.width, spec.height);
    let max = NOISE_SIDE.load(std::sync::atomic::Ordering::Relaxed);
    if w == 0 || h == 0 || w > max || h > max {
        return None;
    }
    let key = spec.key();
    if let Some(img) = NOISE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(img);
    }
    let cells = spec.cells.max(1);
    let per = cells as f32 / w as f32;
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = fbm_at(x as f32 * per, y as f32 * per, spec.octaves, spec.seed, cells as i32);
            let c = ramp_at(&spec.ramp, v);
            let a = c.a.clamp(0.0, 1.0);
            let q = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
            px.extend_from_slice(&[q(c.r), q(c.g), q(c.b), (a * 255.0 + 0.5) as u8]);
        }
    }
    let img = Image::new(w, h, Arc::from(px));
    NOISE.with(|c| keep(&mut c.borrow_mut(), key, &img));
    Some(img)
}

fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    if len == 0 {
        return;
    }
    let at = |line: usize, i: usize| if horizontal { (line * w + i) * 4 } else { (i * w + line) * 4 };
    let texel =
        |p: usize| src.get(p..p + 4).and_then(|s| <&[u8; 4]>::try_from(s).ok()).copied().unwrap_or([0; 4]);
    let n = (2 * r + 1) as u32;
    for line in 0..lines {
        let mut sum = [0u32; 4];
        // Edges extend with transparency, so a glow fades out instead of smearing.
        for i in 0..=r.min(len - 1) {
            for (s, v) in sum.iter_mut().zip(texel(at(line, i))) {
                *s += u32::from(v);
            }
        }
        for i in 0..len {
            let p = at(line, i);
            if let Some(out) = dst.get_mut(p..p + 4) {
                for (o, s) in out.iter_mut().zip(sum) {
                    *o = (s / n) as u8;
                }
            }
            if i + r + 1 < len {
                for (s, v) in sum.iter_mut().zip(texel(at(line, i + r + 1))) {
                    *s += u32::from(v);
                }
            }
            if i >= r {
                for (s, v) in sum.iter_mut().zip(texel(at(line, i - r))) {
                    *s = s.saturating_sub(u32::from(v));
                }
            }
        }
    }
}

/// `image` blurred by about `radius` pixels (three box passes each way, close to a Gaussian),
/// made once per image and radius and kept: a bloom behind a lantern, a soft shadow, haze.
/// Pass the picture `PaintCx::cached` made, or art of your own.
pub fn blur(image: &Image, radius: u32) -> Image {
    if radius == 0 || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let key = (image.id, radius);
    if let Some(img) = BLURRED.with(|c| c.borrow().get(&key).cloned()) {
        return img;
    }
    let (w, h) = (image.width as usize, image.height as usize);
    // Three boxes of about radius/√3 each approximate a Gaussian of the radius.
    let r = ((radius as f32 / 1.7).round() as usize).max(1);
    let mut a = image.pixels.to_vec();
    let mut b = vec![0u8; a.len()];
    for _ in 0..3 {
        box_pass(&a, &mut b, w, h, r, true);
        box_pass(&b, &mut a, w, h, r, false);
    }
    let img = Image::new(image.width, image.height, Arc::from(a));
    BLURRED.with(|c| keep(&mut c.borrow_mut(), key, &img));
    img
}

/// Drops the noise and blur textures kept (on memory pressure).
pub fn clear_fx_cache() {
    NOISE.with(|c| c.borrow_mut().clear());
    BLURRED.with(|c| c.borrow_mut().clear());
}

/// A soft round glow of `color` at `center`, fading to nothing at `radius`, added to what is
/// below (it brightens, never muddies). `strength` scales it (0 to 1; animate it with
/// [`flicker`] or [`breathe`]).
pub fn glow(list: &mut DisplayList, center: Point, radius: f32, color: Color, strength: f32) {
    if radius <= 0.0 || strength <= 0.0 {
        return;
    }
    let a = color.a * strength.clamp(0.0, 1.0);
    let stops = vec![
        Stop { offset: 0.0, color: color.with_alpha(a) },
        Stop { offset: 0.35, color: color.with_alpha(a * 0.45) },
        Stop { offset: 1.0, color: color.with_alpha(0.0) },
    ];
    let rect = Rect::new(center.x - radius, center.y - radius, center.x + radius, center.y + radius);
    list.push(Item::PushLayer { opacity: 1.0, blend: Blend::Plus });
    list.fill_rect(rect, Brush::Radial { center, radius, stops });
    list.pop_layer();
}

/// A cone of light from `apex` pointing at `angle` radians (0: right, clockwise), `length`
/// long and `spread` radians wide, bright at the apex and fading out, added to what is below:
/// a lighthouse beam, a spotlight, a lamp's throw. Put it in an element with a compositor
/// loop (`AmbientLoop::spin` about `apex`) to sweep it with no repaints.
pub fn light_cone(list: &mut DisplayList, apex: Point, angle: f32, length: f32, spread: f32, color: Color) {
    if length <= 0.0 || spread <= 0.0 {
        return;
    }
    let ray = |a: f32| Point::new(apex.x + length * a.cos(), apex.y + length * a.sin());
    let end = ray(angle);
    let fade = |k: f32| {
        vec![
            Stop { offset: 0.0, color: color.with_alpha(color.a * k) },
            Stop { offset: 1.0, color: color.with_alpha(0.0) },
        ]
    };
    list.push(Item::PushLayer { opacity: 1.0, blend: Blend::Plus });
    // A soft wide cone under a brighter narrow core: soft edges without a blur.
    for (width, k) in [(1.0, 0.45), (0.45, 0.55)] {
        let half = spread * width / 2.0;
        let mut p = Path::new();
        p.move_to(apex).line_to(ray(angle - half)).line_to(ray(angle + half)).close();
        list.push(Item::FillPath {
            path: p,
            brush: Brush::Linear { start: apex, end, stops: fade(k) },
            even_odd: false,
        });
    }
    list.pop_layer();
}

/// Draws `image` tiled over `rect`, shifted by `offset` (wrapped, so any offset works): a
/// texture that moves without being made again. At most a handful of image draws.
pub fn scroll_tiled(list: &mut DisplayList, image: &Image, rect: Rect, offset: Vec2, tile: Vec2) {
    let (tw, th) = (tile.x, tile.y);
    if tw <= 0.0 || th <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let ox = offset.x.rem_euclid(tw);
    let oy = offset.y.rem_euclid(th);
    list.push_clip(rect, CornerRadii::ZERO);
    let mut y = rect.min.y + oy - th;
    while y < rect.max.y {
        let mut x = rect.min.x + ox - tw;
        while x < rect.max.x {
            list.push(Item::Image {
                image: image.clone(),
                rect: Rect::new(x, y, x + tw, y + th),
                radii: CornerRadii::ZERO,
                sampling: Sampling::Linear,
            });
            x += tw;
        }
        y += th;
    }
    list.pop_clip();
}

/// Water shimmer: `image` (a tileable [`noise_image`] of light streaks) scrolled over `rect`
/// in two layers moving at `speed` and at a different angle and pace, added to what is below,
/// at time `t` seconds. Cheap: two scrolled copies of one cached texture.
pub fn shimmer(list: &mut DisplayList, image: &Image, rect: Rect, t: f32, speed: Vec2, strength: f32) {
    let tile = Vec2::new(image.width as f32, image.height as f32);
    list.push(Item::PushLayer { opacity: strength.clamp(0.0, 1.0), blend: Blend::Plus });
    scroll_tiled(list, image, rect, Vec2::new(speed.x * t, speed.y * t), tile);
    list.push(Item::PushLayer { opacity: 0.6, blend: Blend::Normal });
    scroll_tiled(
        list,
        image,
        rect,
        Vec2::new(-speed.y * t * 0.7 + tile.x * 0.5, speed.x * t * 0.6 + tile.y * 0.3),
        tile,
    );
    list.pop_layer();
    list.pop_layer();
}

trait PopLayer {
    fn pop_layer(&mut self);
}

impl PopLayer for DisplayList {
    fn pop_layer(&mut self) {
        self.push(Item::PopLayer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_stays_in_range_and_is_repeatable() {
        for i in 0..500 {
            let (x, y) = (i as f32 * 0.37, i as f32 * 0.11);
            for v in [value_noise(x, y, 3), perlin(x, y, 3), fbm(x, y, 5, 3), flicker(x, 9), twinkle(x, 4)] {
                assert!((0.0..=1.0).contains(&v), "{v}");
            }
            assert!(flicker(x, 9) >= 0.7);
            assert_eq!(fbm(x, y, 4, 7), fbm(x, y, 4, 7));
        }
        assert_ne!(rand(1, 2), rand(1, 3));
        assert!((breathe(1.0, 2.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn curves_move_but_not_in_step() {
        let a: Vec<f32> = (0..60).map(|i| twinkle(i as f32 * 0.25, 1)).collect();
        let b: Vec<f32> = (0..60).map(|i| twinkle(i as f32 * 0.25, 2)).collect();
        assert_ne!(a, b);
        let spread = a.iter().fold(0.0f32, |m, v| m.max(*v)) - a.iter().fold(1.0f32, |m, v| m.min(*v));
        assert!(spread > 0.05, "it twinkles: {spread}");
    }

    #[test]
    fn noise_images_tile_and_are_made_once() {
        let spec = NoiseSpec::new(64, 32, 4, Color::WHITE).seed(5);
        let a = noise_image(&spec);
        let b = noise_image(&spec);
        assert!(a.is_some());
        assert_eq!(a.as_ref().map(|i| i.id), b.as_ref().map(|i| i.id), "cached: the same picture");
        let img = a.unwrap_or_else(|| unreachable!());
        // Tileable: the left column continues the right one.
        let px = |x: usize, y: usize| img.pixels.get((y * 64 + x) * 4 + 3).copied().unwrap_or(0);
        let seam: i32 =
            (0..32).map(|y| (i32::from(px(0, y)) - i32::from(px(63, y))).abs()).max().unwrap_or(0);
        let inner: i32 =
            (0..32).map(|y| (i32::from(px(30, y)) - i32::from(px(31, y))).abs()).max().unwrap_or(0);
        assert!(seam <= inner.max(12) * 2, "seam {seam} vs inner {inner}");
        assert!(noise_image(&NoiseSpec::new(0, 4, 1, Color::WHITE)).is_none());
        let other = noise_image(&spec.clone().seed(6)).map(|i| i.id);
        assert_ne!(other, Some(img.id));
    }

    #[test]
    fn blur_spreads_light_and_keeps_its_total_roughly() {
        let (w, h) = (32u32, 32u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let centre = ((16 * w + 16) * 4) as usize;
        if let Some(p) = px.get_mut(centre..centre + 4) {
            p.fill(255);
        }
        let img = Image::new(w, h, Arc::from(px));
        let b = blur(&img, 4);
        assert_eq!(blur(&img, 4).id, b.id, "made once");
        let at = |x: u32, y: u32| b.pixels.get(((y * w + x) * 4 + 3) as usize).copied().unwrap_or(0);
        assert!(at(16, 16) < 255 && at(18, 16) > 0, "spread");
        assert_eq!(at(0, 0), 0, "far corners stay clear");
        assert_eq!(blur(&img, 0).id, img.id);
    }

    #[test]
    fn helpers_draw_bounded_lists() {
        let mut l = DisplayList::new();
        glow(&mut l, Point::new(10.0, 10.0), 8.0, Color::WHITE, 1.0);
        light_cone(&mut l, Point::ZERO, 0.0, 100.0, 0.4, Color::WHITE);
        let img = Image::new(16, 16, Arc::from(vec![255u8; 16 * 16 * 4]));
        shimmer(&mut l, &img, Rect::new(0.0, 0.0, 40.0, 20.0), 3.3, Vec2::new(10.0, 2.0), 0.5);
        let images = l.items().iter().filter(|i| matches!(i, Item::Image { .. })).count();
        // 40 × 20 over 16 px tiles, shifted: at most 4 × 3 per layer.
        assert!(images <= 24, "{images}");
        let pushes = l.items().iter().filter(|i| matches!(i, Item::PushLayer { .. })).count();
        let pops = l.items().iter().filter(|i| matches!(i, Item::PopLayer)).count();
        assert_eq!(pushes, pops);
        glow(&mut l, Point::ZERO, 0.0, Color::WHITE, 1.0);
        assert_eq!(l.items().iter().filter(|i| matches!(i, Item::PushLayer { .. })).count(), pushes);
    }
}
