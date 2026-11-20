//! The CPU interpreter for [`Program`]s: the reference the GPU kernel is tested against.
//!
//! It runs one instruction over a batch of [`LANES`] pixels (or particles) at a time, so the
//! cost of decoding an instruction is shared by the batch and the inner loops are plain
//! array arithmetic the compiler vectorizes. Every instruction is total: division by zero,
//! roots and logarithms of negatives, and overflow give finite numbers, never NaN, so a
//! hostile program cannot poison a picture.

use crate::ir::{Input, Op, Program, Source};

/// Pixels or particles evaluated together.
pub const LANES: usize = 64;

/// One register over a batch: four components, each `LANES` wide.
pub type Reg = [[f32; LANES]; 4];

const ZERO: Reg = [[0.0; LANES]; 4];

/// What a program reads that is not a register: inputs per lane and sampled pictures.
pub trait Env {
    /// Fills `out` with `input` for the first `n` lanes of the batch.
    fn input(&self, input: Input, out: &mut Reg, n: usize);
    /// Samples `source` at `uv` (straight alpha). `None` reads transparent black.
    fn sample(&self, source: Source, u: f32, v: f32) -> [f32; 4];
}

/// A picture a program can sample: premultiplied RGBA8.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, premultiplied.
    pub pixels: &'a [u8],
    /// Repeat outside 0..1 (textures) instead of clamping to the edge (buffers).
    pub repeat: bool,
}

impl View<'_> {
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (i64::from(self.width), i64::from(self.height));
        let (x, y) = if self.repeat {
            (x.rem_euclid(w), y.rem_euclid(h))
        } else {
            (x.clamp(0, w - 1), y.clamp(0, h - 1))
        };
        let i = ((y * w + x) * 4) as usize;
        match self.pixels.get(i..i + 4).and_then(|p| <&[u8; 4]>::try_from(p).ok()) {
            Some(p) => p.map(|c| f32::from(c) / 255.0),
            None => [0.0; 4],
        }
    }

    /// Bilinear sample at `uv` with pixel centres at half-pixels, unpremultiplied.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 || !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let x = u.clamp(-1.0e4, 1.0e4) * self.width as f32 - 0.5;
        let y = v.clamp(-1.0e4, 1.0e4) * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let a = self.texel(ix, iy);
        let b = self.texel(ix + 1, iy);
        let c = self.texel(ix, iy + 1);
        let d = self.texel(ix + 1, iy + 1);
        unpremultiply(bilerp(a, b, c, d, fx, fy))
    }
}

/// Bilinear blend of four texels: `a` `b` on top, `c` `d` below.
pub(crate) fn bilerp(a: [f32; 4], b: [f32; 4], c: [f32; 4], d: [f32; 4], fx: f32, fy: f32) -> [f32; 4] {
    let mut out = [0.0; 4];
    for ((((o, a), b), c), d) in out.iter_mut().zip(a).zip(b).zip(c).zip(d) {
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        *o = top + (bottom - top) * fy;
    }
    out
}

/// Lane `l` of a register, as one value.
#[inline]
pub(crate) fn lane_of(r: &Reg, l: usize) -> [f32; 4] {
    let at = |k: usize| r.get(k).and_then(|c| c.get(l)).copied().unwrap_or(0.0);
    [at(0), at(1), at(2), at(3)]
}

/// Straight alpha from premultiplied.
pub fn unpremultiply(c: [f32; 4]) -> [f32; 4] {
    if c[3] <= 1.0 / 512.0 { [0.0; 4] } else { [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]] }
}

/// The interpreter: registers for one batch, reused between runs.
pub struct Vm {
    /// Boxed, so a result computed into `out` is put in place by swapping two pointers
    /// (a register is 1 KiB; swapping it unboxed would copy it).
    #[allow(clippy::vec_box)]
    regs: Vec<Box<Reg>>,
    /// Where each instruction writes, while its operands are read where they are.
    out: Box<Reg>,
}

impl Default for Vm {
    fn default() -> Self {
        Vm::new()
    }
}

impl std::fmt::Debug for Vm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Vm({} registers)", self.regs.len())
    }
}

/// A well-mixed 32-bit hash (PCG output permutation): the one both renderers use.
#[inline]
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28).wrapping_add(4))) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// The hash of up to four whole-number cell coordinates.
#[inline]
pub fn hash_cell(c: &[i32]) -> u32 {
    let mut h = 0x9E37_79B9u32;
    for &x in c {
        h = pcg(h ^ (x as u32));
    }
    h
}

/// The hashes of a lattice cell's corners along x then y: `hash_cell(&[x + dx, y + dy])`
/// for `dx`, `dy` in 0 and 1, as `[[h00, h10], [h01, h11]]`, sharing the x step (six
/// hashes instead of eight; the same values).
#[inline]
fn corners2(ix: i32, iy: i32) -> [[u32; 2]; 2] {
    let x0 = pcg(0x9E37_79B9 ^ (ix as u32));
    let x1 = pcg(0x9E37_79B9 ^ (ix.wrapping_add(1) as u32));
    let (y0, y1) = (iy as u32, iy.wrapping_add(1) as u32);
    [[pcg(x0 ^ y0), pcg(x1 ^ y0)], [pcg(x0 ^ y1), pcg(x1 ^ y1)]]
}

#[inline]
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

/// Four random numbers 0 to 1 for the cell holding `p` (its first `dims` components).
pub fn hash4(p: [f32; 4], dims: usize) -> [f32; 4] {
    let mut cell = [0i32; 4];
    let dims = dims.clamp(1, 4);
    for (c, x) in cell.iter_mut().zip(p).take(dims) {
        *c = floor_i(x);
    }
    let h0 = hash_cell(cell.get(..dims).unwrap_or(&[]));
    let h1 = pcg(h0);
    let h2 = pcg(h1);
    let h3 = pcg(h2);
    [unit(h0), unit(h1), unit(h2), unit(h3)]
}

#[inline]
fn floor_i(x: f32) -> i32 {
    if x.is_finite() { x.floor().clamp(-2.0e9, 2.0e9) as i32 } else { 0 }
}

#[inline]
fn quintic(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 2D value noise, 0 to 1.
pub fn value2(x: f32, y: f32) -> f32 {
    let (ix, iy) = (floor_i(x), floor_i(y));
    let (fx, fy) = (fract(x), fract(y));
    let [[h00, h10], [h01, h11]] = corners2(ix, iy);
    let (v00, v10, v01, v11) = (unit(h00), unit(h10), unit(h01), unit(h11));
    let (u, w) = (quintic(fx), quintic(fy));
    let top = v00 + (v10 - v00) * u;
    let bottom = v01 + (v11 - v01) * u;
    top + (bottom - top) * w
}

/// 3D value noise, 0 to 1.
pub fn value3(x: f32, y: f32, z: f32) -> f32 {
    let (ix, iy, iz) = (floor_i(x), floor_i(y), floor_i(z));
    let (u, v, w) = (quintic(fract(x)), quintic(fract(y)), quintic(fract(z)));
    let xy = corners2(ix, iy);
    let (z0, z1) = (iz as u32, iz.wrapping_add(1) as u32);
    let h = |dx: usize, dy: usize, dz: usize| {
        let c = xy.get(dy).and_then(|r| r.get(dx)).copied().unwrap_or(0);
        unit(pcg(c ^ if dz == 0 { z0 } else { z1 }))
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let z0 = lerp(lerp(h(0, 0, 0), h(1, 0, 0), u), lerp(h(0, 1, 0), h(1, 1, 0), u), v);
    let z1 = lerp(lerp(h(0, 0, 1), h(1, 0, 1), u), lerp(h(0, 1, 1), h(1, 1, 1), u), v);
    lerp(z0, z1, w)
}

const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// The eight unit gradients 2D gradient noise picks from (no trigonometry, so both
/// renderers pick bit-identical directions).
const GRAD2: [(f32, f32); 8] =
    [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (D, D), (-D, D), (D, -D), (-D, -D)];

#[inline]
fn grad2(h: u32, x: f32, y: f32) -> f32 {
    let (gx, gy) = GRAD2.get((h & 7) as usize).copied().unwrap_or((1.0, 0.0));
    gx * x + gy * y
}

/// The twelve edge directions of a cube, the gradients 3D gradient noise picks from.
const GRAD3: [[f32; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

/// A table rather than a `match`: the hash is random, so a branch per direction would be
/// mispredicted most of the time. (Multiplying by 1, -1 or 0 is exact; the noise's sum ends
/// in `+ 0.5`, which drops the sign of a zero, so it is the same noise bit for bit.)
#[inline]
fn grad3(h: u32, x: f32, y: f32, z: f32) -> f32 {
    let [gx, gy, gz] = GRAD3.get((h % 12) as usize).copied().unwrap_or([1.0, 1.0, 0.0]);
    gx * x + gy * y + gz * z
}

/// 2D gradient (Perlin) noise, 0 to 1.
pub fn perlin2(x: f32, y: f32) -> f32 {
    let (ix, iy) = (floor_i(x), floor_i(y));
    let (fx, fy) = (fract(x), fract(y));
    let [[h00, h10], [h01, h11]] = corners2(ix, iy);
    let (g00, g10) = (grad2(h00, fx, fy), grad2(h10, fx - 1.0, fy));
    let (g01, g11) = (grad2(h01, fx, fy - 1.0), grad2(h11, fx - 1.0, fy - 1.0));
    let (u, v) = (quintic(fx), quintic(fy));
    let top = g00 + (g10 - g00) * u;
    let bottom = g01 + (g11 - g01) * u;
    ((top + (bottom - top) * v) * D + 0.5).clamp(0.0, 1.0)
}

/// 3D gradient (Perlin) noise, 0 to 1.
pub fn perlin3(x: f32, y: f32, z: f32) -> f32 {
    let (ix, iy, iz) = (floor_i(x), floor_i(y), floor_i(z));
    let (fx, fy, fz) = (fract(x), fract(y), fract(z));
    let xy = corners2(ix, iy);
    let (z0, z1) = (iz as u32, iz.wrapping_add(1) as u32);
    let g = |dx: usize, dy: usize, dz: usize| {
        let c = xy.get(dy).and_then(|r| r.get(dx)).copied().unwrap_or(0);
        let h = pcg(c ^ if dz == 0 { z0 } else { z1 });
        grad3(h, fx - dx as f32, fy - dy as f32, fz - dz as f32)
    };
    let (u, v, w) = (quintic(fx), quintic(fy), quintic(fz));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let z0 = lerp(lerp(g(0, 0, 0), g(1, 0, 0), u), lerp(g(0, 1, 0), g(1, 1, 0), u), v);
    let z1 = lerp(lerp(g(0, 0, 1), g(1, 0, 1), u), lerp(g(0, 1, 1), g(1, 1, 1), u), v);
    (lerp(z0, z1, w) * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// 2D simplex noise, 0 to 1.
pub fn simplex2(x: f32, y: f32) -> f32 {
    const F2: f32 = 0.366_025_42;
    const G2: f32 = 0.211_324_87;
    let s = (x + y) * F2;
    let (i, j) = (floor_i(x + s), floor_i(y + s));
    let t = (i as f32 + j as f32) * G2;
    let (x0, y0) = (x - (i as f32 - t), y - (j as f32 - t));
    let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
    let (x1, y1) = (x0 - i1 as f32 + G2, y0 - j1 as f32 + G2);
    let (x2, y2) = (x0 - 1.0 + 2.0 * G2, y0 - 1.0 + 2.0 * G2);
    let corner = |dx: i32, dy: i32, px: f32, py: f32| {
        let t = 0.5 - px * px - py * py;
        if t <= 0.0 {
            0.0
        } else {
            let t2 = t * t;
            t2 * t2 * grad2(hash_cell(&[i.wrapping_add(dx), j.wrapping_add(dy)]), px, py)
        }
    };
    let n = corner(0, 0, x0, y0) + corner(i1, j1, x1, y1) + corner(1, 1, x2, y2);
    (n * 35.0 + 0.5).clamp(0.0, 1.0)
}

/// 2D cellular noise: distance to the nearest and second-nearest feature point, and a random
/// number 0 to 1 for the nearest point's cell.
pub fn voronoi2(x: f32, y: f32) -> [f32; 4] {
    let (ix, iy) = (floor_i(x), floor_i(y));
    let (fx, fy) = (fract(x), fract(y));
    let (mut f1, mut f2, mut id) = (8.0f32, 8.0f32, 0.0f32);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let h0 = hash_cell(&[ix.wrapping_add(dx), iy.wrapping_add(dy)]);
            let h1 = pcg(h0);
            let h2 = pcg(h1);
            let px = dx as f32 + unit(h0) - fx;
            let py = dy as f32 + unit(h1) - fy;
            let d = (px * px + py * py).sqrt();
            if d < f1 {
                f2 = f1;
                f1 = d;
                id = unit(h2);
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    [f1, f2, id, 0.0]
}

#[inline]
fn fract(x: f32) -> f32 {
    if x.is_finite() { x - x.floor() } else { 0.0 }
}

#[inline]
fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

#[inline]
fn truth(x: f32) -> bool {
    x > 0.5
}

#[inline]
fn b2f(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// What every component-wise opcode computes for one component (the specification both
/// renderers follow).
#[inline]
pub fn scalar(op: Op, a: f32, b: f32, c: f32) -> f32 {
    match op {
        Op::Mov => a,
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * b,
        Op::Div => {
            if b == 0.0 {
                0.0
            } else {
                finite(a / b)
            }
        }
        Op::Mod => {
            if b == 0.0 {
                0.0
            } else {
                finite(a - b * (a / b).floor())
            }
        }
        Op::Min => a.min(b),
        Op::Max => a.max(b),
        Op::Pow => {
            // |a|^b, negative when a is negative and b an odd whole number (as arithmetic
            // has it); never NaN.
            let b = b.clamp(-64.0, 64.0);
            let m = finite(a.abs().powf(b));
            let odd = b.fract() == 0.0 && (b * 0.5).fract() != 0.0;
            if a < 0.0 && odd { -m } else { m }
        }
        Op::Atan2 => a.atan2(b),
        Op::Step => b2f(b >= a),
        Op::Neg => -a,
        Op::Abs => a.abs(),
        Op::Sign => {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Op::Floor => a.floor(),
        Op::Ceil => a.ceil(),
        Op::Fract => fract(a),
        Op::Round => (a + 0.5).floor(),
        Op::Trunc => a.trunc(),
        Op::Sqrt => a.max(0.0).sqrt(),
        Op::Exp => a.min(80.0).exp(),
        Op::Exp2 => a.min(120.0).exp2(),
        Op::Log => a.max(1.0e-30).ln(),
        Op::Log2 => a.max(1.0e-30).log2(),
        Op::Sin => a.sin(),
        Op::Cos => a.cos(),
        Op::Tan => finite(a.tan()).clamp(-1.0e6, 1.0e6),
        Op::Asin => a.clamp(-1.0, 1.0).asin(),
        Op::Acos => a.clamp(-1.0, 1.0).acos(),
        Op::Atan => a.atan(),
        Op::Clamp => a.max(b).min(c),
        Op::Mix => a + (b - a) * c,
        Op::Fma => a * b + c,
        Op::Smoothstep => {
            let w = b - a;
            let t = if w == 0.0 { b2f(c >= a) } else { ((c - a) / w).clamp(0.0, 1.0) };
            t * t * (3.0 - 2.0 * t)
        }
        Op::Select => {
            if truth(c) {
                b
            } else {
                a
            }
        }
        Op::Lt => b2f(a < b),
        Op::Le => b2f(a <= b),
        Op::Gt => b2f(a > b),
        Op::Ge => b2f(a >= b),
        Op::Eq => b2f(a == b),
        Op::Ne => b2f(a != b),
        Op::And => b2f(truth(a) && truth(b)),
        Op::Or => b2f(truth(a) || truth(b)),
        Op::Not => b2f(!truth(a)),
        _ => 0.0,
    }
}

/// The instruction's result for one lane, for opcodes that mix components.
fn lane(op: Op, imm: u16, a: [f32; 4], b: [f32; 4], c: [f32; 4], d: [f32; 4], env: &dyn Env) -> [f32; 4] {
    let n = usize::from(imm).clamp(1, 4);
    match op {
        Op::Dot => {
            let s: f32 = a.iter().zip(b).take(n).map(|(x, y)| x * y).sum();
            [s; 4]
        }
        Op::Length => {
            let s: f32 = a.iter().take(n).map(|x| x * x).sum();
            [finite(s.sqrt()); 4]
        }
        Op::Normalize => {
            let l: f32 = a.iter().take(n).map(|x| x * x).sum::<f32>().sqrt();
            let mut out = [0.0; 4];
            if l > 1.0e-20 && l.is_finite() {
                for (o, x) in out.iter_mut().zip(a).take(n) {
                    *o = x / l;
                }
            }
            out
        }
        Op::Swizzle => {
            let pick = |k: usize| a.get(usize::from((imm >> (2 * k)) & 3)).copied().unwrap_or(0.0);
            [pick(0), pick(1), pick(2), pick(3)]
        }
        Op::Combine => [a[0], b[0], c[0], d[0]],
        Op::Hash => hash4(a, n),
        Op::Noise => [value2(a[0], a[1]); 4],
        Op::Noise3 => [value3(a[0], a[1], a[2]); 4],
        Op::Perlin => [perlin2(a[0], a[1]); 4],
        Op::Perlin3 => [perlin3(a[0], a[1], a[2]); 4],
        Op::Simplex => [simplex2(a[0], a[1]); 4],
        Op::Voronoi => voronoi2(a[0], a[1]),
        Op::Sample => match Source::decode(imm) {
            Some(s) => env.sample(s, a[0], a[1]),
            None => [0.0; 4],
        },
        _ => [0.0; 4],
    }
}

/// Applies `f` to every component of the first `n` lanes: one tight loop per opcode, which
/// the compiler vectorizes (the opcode is matched once per instruction, not per lane).
#[inline(always)]
fn each(dst: &mut Reg, a: &Reg, b: &Reg, c: &Reg, n: usize, f: impl Fn(f32, f32, f32) -> f32) {
    for (((o, x), y), z) in dst.iter_mut().zip(a).zip(b).zip(c) {
        for (((o, &x), &y), &z) in o.iter_mut().zip(x).zip(y).zip(z).take(n) {
            *o = f(x, y, z);
        }
    }
}

fn wide(op: Op, d: &mut Reg, a: &Reg, b: &Reg, c: &Reg, n: usize) {
    match op {
        Op::Mov => each(d, a, b, c, n, |x, _, _| x),
        Op::Add => each(d, a, b, c, n, |x, y, _| x + y),
        Op::Sub => each(d, a, b, c, n, |x, y, _| x - y),
        Op::Mul => each(d, a, b, c, n, |x, y, _| x * y),
        Op::Fma => each(d, a, b, c, n, |x, y, z| x * y + z),
        Op::Min => each(d, a, b, c, n, |x, y, _| x.min(y)),
        Op::Max => each(d, a, b, c, n, |x, y, _| x.max(y)),
        Op::Mix => each(d, a, b, c, n, |x, y, t| x + (y - x) * t),
        Op::Clamp => each(d, a, b, c, n, |x, lo, hi| x.max(lo).min(hi)),
        Op::Floor => each(d, a, b, c, n, |x, _, _| x.floor()),
        Op::Abs => each(d, a, b, c, n, |x, _, _| x.abs()),
        Op::Neg => each(d, a, b, c, n, |x, _, _| -x),
        Op::Select => each(d, a, b, c, n, |f, t, k| if k > 0.5 { t } else { f }),
        Op::Div => each(d, a, b, c, n, |x, y, _| if y == 0.0 { 0.0 } else { finite(x / y) }),
        Op::Sin => each(d, a, b, c, n, |x, _, _| x.sin()),
        Op::Cos => each(d, a, b, c, n, |x, _, _| x.cos()),
        Op::Smoothstep => each(d, a, b, c, n, |e0, e1, x| scalar(Op::Smoothstep, e0, e1, x)),
        // The rest: each with its own loop (`scalar` with a known op folds to that op's
        // arithmetic), so no per-value dispatch.
        op => {
            macro_rules! loops {
                ($($o:ident),*) => {
                    match op {
                        $(Op::$o => each(d, a, b, c, n, |x, y, z| scalar(Op::$o, x, y, z)),)*
                        op => each(d, a, b, c, n, |x, y, z| scalar(op, x, y, z)),
                    }
                };
            }
            loops!(
                Mod, Pow, Atan2, Step, Sign, Ceil, Fract, Round, Trunc, Sqrt, Exp, Exp2, Log, Log2, Tan,
                Asin, Acos, Atan, Lt, Le, Gt, Ge, Eq, Ne, And, Or, Not
            )
        }
    }
}

fn component_wise(op: Op) -> bool {
    !matches!(
        op,
        Op::Const
            | Op::Input
            | Op::Dot
            | Op::Length
            | Op::Normalize
            | Op::Swizzle
            | Op::Combine
            | Op::Hash
            | Op::Noise
            | Op::Noise3
            | Op::Perlin
            | Op::Perlin3
            | Op::Simplex
            | Op::Voronoi
            | Op::Sample
    )
}

impl Vm {
    /// An interpreter.
    pub fn new() -> Self {
        Vm { regs: Vec::new(), out: Box::new(ZERO) }
    }

    /// Runs `program` for the first `n` lanes of a batch. `consts` is the program's constant
    /// table resolved for this frame. Read results with [`Vm::reg`].
    pub fn run(&mut self, program: &Program, consts: &[[f32; 4]], env: &dyn Env, n: usize) {
        let n = n.min(LANES);
        let want = usize::from(program.regs).max(1);
        while self.regs.len() < want {
            self.regs.push(Box::new(ZERO));
        }
        let Vm { regs, out } = self;
        for ins in &program.code {
            if usize::from(ins.dst) >= regs.len() {
                continue;
            }
            // Operands are read in place (no copies); the result goes to `out`, then takes
            // the destination's place, so an operand may also be the destination.
            let arity = usize::from(ins.op.arity());
            let operand = |k: usize| -> &Reg {
                match ins.src.get(k) {
                    Some(r) if k < arity => regs.get(usize::from(*r)).map_or(&ZERO, |b| &**b),
                    _ => &ZERO,
                }
            };
            let (a, b, c, d) = (operand(0), operand(1), operand(2), operand(3));
            let dst: &mut Reg = out;
            match ins.op {
                Op::Const => {
                    let v = consts.get(usize::from(ins.imm)).copied().unwrap_or([0.0; 4]);
                    for (comp, x) in dst.iter_mut().zip(v) {
                        comp.iter_mut().take(n).for_each(|o| *o = x);
                    }
                }
                Op::Input => match Input::from_u16(ins.imm) {
                    Some(i) => env.input(i, dst, n),
                    None => *dst = ZERO,
                },
                op if component_wise(op) => wide(op, dst, a, b, c, n),
                op => {
                    for l in 0..n {
                        let out = lane(
                            op,
                            ins.imm,
                            lane_of(a, l),
                            lane_of(b, l),
                            lane_of(c, l),
                            lane_of(d, l),
                            env,
                        );
                        for (comp, v) in dst.iter_mut().zip(out) {
                            if let Some(slot) = comp.get_mut(l) {
                                *slot = v;
                            }
                        }
                    }
                }
            }
            if let Some(slot) = regs.get_mut(usize::from(ins.dst)) {
                std::mem::swap(slot, out);
            }
        }
    }

    /// Register `r` after a run.
    pub fn reg(&self, r: u8) -> &Reg {
        self.regs.get(usize::from(r)).map_or(&ZERO, |b| &**b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_stays_in_range_and_is_deterministic() {
        for i in 0..2000 {
            let (x, y, z) = (i as f32 * 0.137 - 50.0, i as f32 * 0.071 + 3.3, i as f32 * 0.013);
            for v in [value2(x, y), perlin2(x, y), simplex2(x, y), value3(x, y, z), perlin3(x, y, z)] {
                assert!((0.0..=1.0).contains(&v), "{v} at {x} {y}");
            }
            let c = voronoi2(x, y);
            assert!(c[0] <= c[1] && c[0] >= 0.0 && c[1] < 3.0 && (0.0..1.0).contains(&c[2]));
        }
        assert_eq!(perlin2(1.3, 2.7), perlin2(1.3, 2.7));
        assert_ne!(hash4([1.0, 2.0, 0.0, 0.0], 2), hash4([2.0, 1.0, 0.0, 0.0], 2));
    }

    /// The gradient as it was first written, a branch per direction.
    fn grad3_branches(h: u32, x: f32, y: f32, z: f32) -> f32 {
        match h % 12 {
            0 => x + y,
            1 => -x + y,
            2 => x - y,
            3 => -x - y,
            4 => x + z,
            5 => -x + z,
            6 => x - z,
            7 => -x - z,
            8 => y + z,
            9 => -y + z,
            10 => y - z,
            _ => -y - z,
        }
    }

    /// The noises from the plain per-corner hash, as they were first written.
    fn reference(kind: u8, x: f32, y: f32, z: f32) -> f32 {
        let (ix, iy, iz) = (floor_i(x), floor_i(y), floor_i(z));
        let (fx, fy, fz) = (fract(x), fract(y), fract(z));
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let h2 = |dx: i32, dy: i32| hash_cell(&[ix.wrapping_add(dx), iy.wrapping_add(dy)]);
        let h3 = |dx: i32, dy: i32, dz: i32| {
            hash_cell(&[ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz)])
        };
        let (u, v, w) = (quintic(fx), quintic(fy), quintic(fz));
        match kind {
            0 => {
                let c = |dx, dy| unit(h2(dx, dy));
                let top = c(0, 0) + (c(1, 0) - c(0, 0)) * u;
                let bottom = c(0, 1) + (c(1, 1) - c(0, 1)) * u;
                top + (bottom - top) * v
            }
            1 => {
                let c = |dx, dy, dz| unit(h3(dx, dy, dz));
                let z0 = lerp(lerp(c(0, 0, 0), c(1, 0, 0), u), lerp(c(0, 1, 0), c(1, 1, 0), u), v);
                let z1 = lerp(lerp(c(0, 0, 1), c(1, 0, 1), u), lerp(c(0, 1, 1), c(1, 1, 1), u), v);
                lerp(z0, z1, w)
            }
            2 => {
                let g = |dx: i32, dy: i32| grad2(h2(dx, dy), fx - dx as f32, fy - dy as f32);
                let top = g(0, 0) + (g(1, 0) - g(0, 0)) * u;
                let bottom = g(0, 1) + (g(1, 1) - g(0, 1)) * u;
                ((top + (bottom - top) * v) * D + 0.5).clamp(0.0, 1.0)
            }
            _ => {
                let g = |dx: i32, dy: i32, dz: i32| {
                    grad3_branches(h3(dx, dy, dz), fx - dx as f32, fy - dy as f32, fz - dz as f32)
                };
                let z0 = lerp(lerp(g(0, 0, 0), g(1, 0, 0), u), lerp(g(0, 1, 0), g(1, 1, 0), u), v);
                let z1 = lerp(lerp(g(0, 0, 1), g(1, 0, 1), u), lerp(g(0, 1, 1), g(1, 1, 1), u), v);
                (lerp(z0, z1, w) * 0.5 + 0.5).clamp(0.0, 1.0)
            }
        }
    }

    #[test]
    fn shared_corner_hashes_give_the_same_noise() {
        let edges = [0.0, -0.0, 1.0e-7, -1.0e-7, 0.999_999_9, -1.5, 1.0e9, -1.0e9, 2.147e9, -2.2e9];
        let mut points: Vec<(f32, f32, f32)> = (0..4000)
            .map(|i| (i as f32 * 0.173 - 300.0, i as f32 * -0.0917 + 41.3, i as f32 * 0.0311 - 7.7))
            .collect();
        for &a in &edges {
            for &b in &edges {
                points.push((a, b, a - b));
            }
        }
        for (x, y, z) in points {
            assert_eq!(value2(x, y).to_bits(), reference(0, x, y, z).to_bits(), "value2({x}, {y})");
            assert_eq!(value3(x, y, z).to_bits(), reference(1, x, y, z).to_bits(), "value3({x}, {y}, {z})");
            assert_eq!(perlin2(x, y).to_bits(), reference(2, x, y, z).to_bits(), "perlin2({x}, {y})");
            assert_eq!(perlin3(x, y, z).to_bits(), reference(3, x, y, z).to_bits(), "perlin3({x}, {y}, {z})");
        }
    }

    #[test]
    fn every_scalar_op_is_total() {
        let hostile = [0.0, -0.0, 1.0, -1.0, 1.0e38, -1.0e38, f32::MIN_POSITIVE, 3.0e-39];
        for (op, ..) in crate::ir::OPS {
            for &a in &hostile {
                for &b in &hostile {
                    let v = scalar(*op, a, b, 0.5);
                    assert!(!v.is_nan(), "{op:?}({a}, {b}) is NaN");
                }
            }
        }
    }

    #[test]
    fn views_sample_bilinear_and_wrap_or_clamp() {
        // 2×1: opaque red, then transparent.
        let px = [255, 0, 0, 255, 0, 0, 0, 0];
        let clamp = View { width: 2, height: 1, pixels: &px, repeat: false };
        assert_eq!(clamp.sample(0.25, 0.5), [1.0, 0.0, 0.0, 1.0]);
        let mid = clamp.sample(0.5, 0.5);
        assert!((mid[3] - 0.5).abs() < 1e-6 && (mid[0] - 1.0).abs() < 1e-6, "unpremultiplied: {mid:?}");
        let wrap = View { repeat: true, ..clamp };
        assert!(wrap.sample(1.0, 0.5)[3] > 0.4, "the right edge blends with the left when repeating");
        assert_eq!(clamp.sample(f32::NAN, 0.0), [0.0; 4]);
    }
}
