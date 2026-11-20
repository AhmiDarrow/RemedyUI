//! Prism effects on the GPU: one fixed shader runs every `.rfx` program.
//!
//! Effects never reach the GPU as shader code. A compiled program is data: instructions in
//! a small integer texture and its constants in a float texture. The kernel below is written
//! and tested by us, interprets those instructions for every pixel (the same specification
//! as the CPU interpreter in `remedy_ui_effects::vm`), and stops after a fixed number of
//! instructions whatever the data says. Next to it: a separable Gaussian blur and instanced
//! particle splats. All pipelines are made once per device, so loading a theme compiles
//! nothing.
//!
//! Every texture stage reads only what the effect owns: its own textures, its earlier passes
//! and its last frame.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_effects::ir::{Effect, Op, Source, Splat, StageKind};
use remedy_ui_effects::{FrameInput, Instance, buffer_size};
use remedy_ui_paint::Blend;

use crate::renderer::FORMAT;
use crate::{GpuContext, GpuError};

/// Instructions per row of the code texture (two per texel).
const CODE_ROW: u32 = 2048;
/// Constants per row of the constant texture.
const CONST_ROW: u32 = 256;
/// The most instructions the kernel runs for one pixel, whatever the program says.
const MAX_CODE: u32 = 16_384;

const KERNEL: &str = r"
struct Header {
    info: vec4<u32>,   // instructions, output register, -, -
    view: vec4<f32>,   // buffer width, height; element width, height (logical pixels)
    misc: vec4<f32>,   // time, aspect, texel, light
};

@group(0) @binding(0) var<uniform> H: Header;
@group(0) @binding(1) var code: texture_2d<u32>;
@group(0) @binding(2) var consts: texture_2d<f32>;
@group(0) @binding(3) var repeat_s: sampler;
@group(0) @binding(4) var clamp_s: sampler;
@group(0) @binding(5) var t0: texture_2d<f32>;
@group(0) @binding(6) var t1: texture_2d<f32>;
@group(0) @binding(7) var t2: texture_2d<f32>;
@group(0) @binding(8) var t3: texture_2d<f32>;
@group(0) @binding(9) var b0: texture_2d<f32>;
@group(0) @binding(10) var b1: texture_2d<f32>;
@group(0) @binding(11) var b2: texture_2d<f32>;
@group(0) @binding(12) var b3: texture_2d<f32>;
@group(0) @binding(13) var b4: texture_2d<f32>;
@group(0) @binding(14) var b5: texture_2d<f32>;
@group(0) @binding(15) var b6: texture_2d<f32>;
@group(0) @binding(16) var b7: texture_2d<f32>;
@group(0) @binding(17) var last_frame: texture_2d<f32>;

var<private> regs: array<vec4<f32>, 64>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn bad(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) == 0x7f800000u;
}
fn fin(x: f32) -> f32 { return select(x, 0.0, bad(x)); }
fn fin4(v: vec4<f32>) -> vec4<f32> { return vec4<f32>(fin(v.x), fin(v.y), fin(v.z), fin(v.w)); }

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
fn unit(h: u32) -> f32 { return f32(h >> 8u) / 16777216.0; }
fn cell(x: f32) -> u32 {
    if (bad(x)) { return 0u; }
    return bitcast<u32>(i32(clamp(floor(x), -2.0e9, 2.0e9)));
}
fn h1(a: u32) -> u32 { return pcg(0x9E3779B9u ^ a); }
fn h2(a: u32, b: u32) -> u32 { return pcg(h1(a) ^ b); }
fn h3(a: u32, b: u32, c: u32) -> u32 { return pcg(h2(a, b) ^ c); }
fn frac(x: f32) -> f32 { return select(x - floor(x), 0.0, bad(x)); }
fn quintic(t: f32) -> f32 { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }

fn value2(p: vec2<f32>) -> f32 {
    let ix = cell(p.x); let iy = cell(p.y);
    let u = quintic(frac(p.x)); let w = quintic(frac(p.y));
    let a = unit(h2(ix, iy)); let b = unit(h2(ix + 1u, iy));
    let c = unit(h2(ix, iy + 1u)); let d = unit(h2(ix + 1u, iy + 1u));
    let top = a + (b - a) * u;
    let bottom = c + (d - c) * u;
    return top + (bottom - top) * w;
}
fn value3(p: vec3<f32>) -> f32 {
    let ix = cell(p.x); let iy = cell(p.y); let iz = cell(p.z);
    let u = quintic(frac(p.x)); let v = quintic(frac(p.y)); let w = quintic(frac(p.z));
    let z0 = mix(mix(unit(h3(ix, iy, iz)), unit(h3(ix + 1u, iy, iz)), u), mix(unit(h3(ix, iy + 1u, iz)), unit(h3(ix + 1u, iy + 1u, iz)), u), v);
    let z1 = mix(mix(unit(h3(ix, iy, iz + 1u)), unit(h3(ix + 1u, iy, iz + 1u)), u), mix(unit(h3(ix, iy + 1u, iz + 1u)), unit(h3(ix + 1u, iy + 1u, iz + 1u)), u), v);
    return z0 + (z1 - z0) * w;
}
fn grad2(h: u32, x: f32, y: f32) -> f32 {
    let d = 0.70710677;
    switch (h & 7u) {
        case 0u: { return x; }
        case 1u: { return -x; }
        case 2u: { return y; }
        case 3u: { return -y; }
        case 4u: { return d * x + d * y; }
        case 5u: { return -d * x + d * y; }
        case 6u: { return d * x - d * y; }
        default: { return -d * x - d * y; }
    }
}
fn grad3(h: u32, x: f32, y: f32, z: f32) -> f32 {
    switch (h % 12u) {
        case 0u: { return x + y; }
        case 1u: { return -x + y; }
        case 2u: { return x - y; }
        case 3u: { return -x - y; }
        case 4u: { return x + z; }
        case 5u: { return -x + z; }
        case 6u: { return x - z; }
        case 7u: { return -x - z; }
        case 8u: { return y + z; }
        case 9u: { return -y + z; }
        case 10u: { return y - z; }
        default: { return -y - z; }
    }
}
fn perlin2(p: vec2<f32>) -> f32 {
    let ix = cell(p.x); let iy = cell(p.y);
    let fx = frac(p.x); let fy = frac(p.y);
    let g00 = grad2(h2(ix, iy), fx, fy);
    let g10 = grad2(h2(ix + 1u, iy), fx - 1.0, fy);
    let g01 = grad2(h2(ix, iy + 1u), fx, fy - 1.0);
    let g11 = grad2(h2(ix + 1u, iy + 1u), fx - 1.0, fy - 1.0);
    let u = quintic(fx); let v = quintic(fy);
    let top = g00 + (g10 - g00) * u;
    let bottom = g01 + (g11 - g01) * u;
    return clamp((top + (bottom - top) * v) * 0.70710677 + 0.5, 0.0, 1.0);
}
fn perlin3(p: vec3<f32>) -> f32 {
    let ix = cell(p.x); let iy = cell(p.y); let iz = cell(p.z);
    let fx = frac(p.x); let fy = frac(p.y); let fz = frac(p.z);
    let u = quintic(fx); let v = quintic(fy); let w = quintic(fz);
    let z0 = mix(
        mix(grad3(h3(ix, iy, iz), fx, fy, fz), grad3(h3(ix + 1u, iy, iz), fx - 1.0, fy, fz), u),
        mix(grad3(h3(ix, iy + 1u, iz), fx, fy - 1.0, fz), grad3(h3(ix + 1u, iy + 1u, iz), fx - 1.0, fy - 1.0, fz), u), v);
    let z1 = mix(
        mix(grad3(h3(ix, iy, iz + 1u), fx, fy, fz - 1.0), grad3(h3(ix + 1u, iy, iz + 1u), fx - 1.0, fy, fz - 1.0), u),
        mix(grad3(h3(ix, iy + 1u, iz + 1u), fx, fy - 1.0, fz - 1.0), grad3(h3(ix + 1u, iy + 1u, iz + 1u), fx - 1.0, fy - 1.0, fz - 1.0), u), v);
    return clamp((z0 + (z1 - z0) * w) * 0.5 + 0.5, 0.0, 1.0);
}
fn corner(i: u32, j: u32, px: f32, py: f32) -> f32 {
    let t = 0.5 - px * px - py * py;
    if (t <= 0.0) { return 0.0; }
    let t2 = t * t;
    return t2 * t2 * grad2(h2(i, j), px, py);
}
fn simplex2(p: vec2<f32>) -> f32 {
    let F2 = 0.36602542;
    let G2 = 0.21132487;
    let s = (p.x + p.y) * F2;
    let i = cell(p.x + s); let j = cell(p.y + s);
    let t = (f32(bitcast<i32>(i)) + f32(bitcast<i32>(j))) * G2;
    let x0 = p.x - (f32(bitcast<i32>(i)) - t);
    let y0 = p.y - (f32(bitcast<i32>(j)) - t);
    var i1 = 0u; var j1 = 1u;
    if (x0 > y0) { i1 = 1u; j1 = 0u; }
    let x1 = x0 - f32(i1) + G2; let y1 = y0 - f32(j1) + G2;
    let x2 = x0 - 1.0 + 2.0 * G2; let y2 = y0 - 1.0 + 2.0 * G2;
    let n = corner(i, j, x0, y0) + corner(i + i1, j + j1, x1, y1) + corner(i + 1u, j + 1u, x2, y2);
    return clamp(n * 35.0 + 0.5, 0.0, 1.0);
}
fn voronoi2(p: vec2<f32>) -> vec4<f32> {
    let ix = cell(p.x); let iy = cell(p.y);
    let fx = frac(p.x); let fy = frac(p.y);
    var f1 = 8.0; var f2 = 8.0; var id = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let a = h2(ix + bitcast<u32>(dx), iy + bitcast<u32>(dy));
            let b = pcg(a);
            let c = pcg(b);
            let qx = f32(dx) + unit(a) - fx;
            let qy = f32(dy) + unit(b) - fy;
            let d = sqrt(qx * qx + qy * qy);
            if (d < f1) { f2 = f1; f1 = d; id = unit(c); } else if (d < f2) { f2 = d; }
        }
    }
    return vec4<f32>(f1, f2, id, 0.0);
}
fn hash4(p: vec4<f32>, dims: u32) -> vec4<f32> {
    var h = h1(cell(p.x));
    if (dims >= 2u) { h = pcg(h ^ cell(p.y)); }
    if (dims >= 3u) { h = pcg(h ^ cell(p.z)); }
    if (dims >= 4u) { h = pcg(h ^ cell(p.w)); }
    let a = pcg(h); let b = pcg(a); let c = pcg(b);
    return vec4<f32>(unit(h), unit(a), unit(b), unit(c));
}

fn unpremul(c: vec4<f32>) -> vec4<f32> {
    if (c.a <= 1.0 / 512.0) { return vec4<f32>(0.0); }
    return vec4<f32>(c.rgb / c.a, c.a);
}
fn sample_src(slot: u32, uv: vec2<f32>) -> vec4<f32> {
    if (bad(uv.x) || bad(uv.y)) { return vec4<f32>(0.0); }
    let q = clamp(uv, vec2<f32>(-1.0e4), vec2<f32>(1.0e4));
    var c = vec4<f32>(0.0);
    switch (slot) {
        case 0u: { c = textureSampleLevel(t0, repeat_s, q, 0.0); }
        case 1u: { c = textureSampleLevel(t1, repeat_s, q, 0.0); }
        case 2u: { c = textureSampleLevel(t2, repeat_s, q, 0.0); }
        case 3u: { c = textureSampleLevel(t3, repeat_s, q, 0.0); }
        case 16u: { c = textureSampleLevel(b0, clamp_s, q, 0.0); }
        case 17u: { c = textureSampleLevel(b1, clamp_s, q, 0.0); }
        case 18u: { c = textureSampleLevel(b2, clamp_s, q, 0.0); }
        case 19u: { c = textureSampleLevel(b3, clamp_s, q, 0.0); }
        case 20u: { c = textureSampleLevel(b4, clamp_s, q, 0.0); }
        case 21u: { c = textureSampleLevel(b5, clamp_s, q, 0.0); }
        case 22u: { c = textureSampleLevel(b6, clamp_s, q, 0.0); }
        case 23u: { c = textureSampleLevel(b7, clamp_s, q, 0.0); }
        case 255u: { c = textureSampleLevel(last_frame, clamp_s, q, 0.0); }
        default: { }
    }
    return unpremul(c);
}

fn input(i: u32, uv: vec2<f32>) -> vec4<f32> {
    switch (i) {
        case 0u: { return vec4<f32>(uv, 0.0, 0.0); }
        case 1u: { return vec4<f32>(uv * H.view.zw, 0.0, 0.0); }
        case 2u: { return vec4<f32>(H.view.zw, 0.0, 0.0); }
        case 3u: { return vec4<f32>(H.misc.x); }
        case 4u: { return vec4<f32>(H.misc.y); }
        case 11u: { return vec4<f32>(H.misc.z); }
        case 12u: { return vec4<f32>(H.misc.w); }
        default: { return vec4<f32>(0.0); }
    }
}

fn b2f(b: vec4<bool>) -> vec4<f32> { return select(vec4<f32>(0.0), vec4<f32>(1.0), b); }
fn truth(v: vec4<f32>) -> vec4<bool> { return v > vec4<f32>(0.5); }
fn mask(n: u32) -> vec4<f32> { return vec4<f32>(1.0, f32(n >= 2u), f32(n >= 3u), f32(n >= 4u)); }

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / H.view.xy;
    let n = min(H.info.x, 16384u);
    for (var pc = 0u; pc < n; pc++) {
        let half = pc / 2u;
        let w = textureLoad(code, vec2<i32>(i32(half % 1024u), i32(half / 1024u)), 0);
        let odd = (pc & 1u) == 1u;
        let w0 = select(w.x, w.z, odd);
        let w1 = select(w.y, w.w, odd);
        let op = w0 & 0xffu;
        let dst = (w0 >> 8u) & 63u;
        let A = regs[(w0 >> 16u) & 63u];
        let B = regs[(w0 >> 24u) & 63u];
        let C = regs[w1 & 63u];
        let D = regs[(w1 >> 8u) & 63u];
        let imm = w1 >> 16u;
        var r = vec4<f32>(0.0);
        switch (op) {
            case 0u: { r = textureLoad(consts, vec2<i32>(i32(imm % 256u), i32(imm / 256u)), 0); }
            case 1u: { r = input(imm, uv); }
            case 2u: { r = A; }
            case 3u: { r = A + B; }
            case 4u: { r = A - B; }
            case 5u: { r = A * B; }
            case 6u: { r = select(fin4(A / B), vec4<f32>(0.0), B == vec4<f32>(0.0)); }
            case 7u: { r = select(fin4(A - B * floor(A / B)), vec4<f32>(0.0), B == vec4<f32>(0.0)); }
            case 8u: { r = min(A, B); }
            case 9u: { r = max(A, B); }
            case 10u: {
                let e = clamp(B, vec4<f32>(-64.0), vec4<f32>(64.0));
                let m = fin4(pow(abs(A), e));
                let odd = (fract(e) == vec4<f32>(0.0)) & (fract(e * 0.5) != vec4<f32>(0.0));
                r = select(m, -m, (A < vec4<f32>(0.0)) & odd);
            }
            case 11u: { r = atan2(A, B); }
            case 12u: { r = b2f(B >= A); }
            case 13u: { r = -A; }
            case 14u: { r = abs(A); }
            case 15u: { r = sign(A); }
            case 16u: { r = floor(A); }
            case 17u: { r = ceil(A); }
            case 18u: { r = fin4(A - floor(A)); }
            case 19u: { r = floor(A + vec4<f32>(0.5)); }
            case 20u: { r = sqrt(max(A, vec4<f32>(0.0))); }
            case 21u: { r = exp(min(A, vec4<f32>(80.0))); }
            case 22u: { r = log(max(A, vec4<f32>(1.0e-30))); }
            case 23u: { r = sin(A); }
            case 24u: { r = cos(A); }
            case 25u: { r = clamp(fin4(tan(A)), vec4<f32>(-1.0e6), vec4<f32>(1.0e6)); }
            case 26u: { r = asin(clamp(A, vec4<f32>(-1.0), vec4<f32>(1.0))); }
            case 27u: { r = acos(clamp(A, vec4<f32>(-1.0), vec4<f32>(1.0))); }
            case 28u: { r = atan(A); }
            case 29u: { r = min(max(A, B), C); }
            case 30u: { r = A + (B - A) * C; }
            case 31u: {
                let wd = B - A;
                let t = select(clamp((C - A) / wd, vec4<f32>(0.0), vec4<f32>(1.0)), b2f(C >= A), wd == vec4<f32>(0.0));
                r = t * t * (vec4<f32>(3.0) - 2.0 * t);
            }
            case 32u: { r = select(A, B, truth(C)); }
            case 33u: { r = b2f(A < B); }
            case 34u: { r = b2f(A <= B); }
            case 35u: { r = b2f(A > B); }
            case 36u: { r = b2f(A >= B); }
            case 37u: { r = b2f(A == B); }
            case 38u: { r = b2f(A != B); }
            case 39u: { r = b2f(truth(A) & truth(B)); }
            case 40u: { r = b2f(truth(A) | truth(B)); }
            case 41u: { r = b2f(!truth(A)); }
            case 42u: { r = vec4<f32>(dot(A * mask(clamp(imm, 1u, 4u)), B)); }
            case 43u: { let m = A * mask(clamp(imm, 1u, 4u)); r = vec4<f32>(fin(sqrt(dot(m, m)))); }
            case 44u: {
                let m = A * mask(clamp(imm, 1u, 4u));
                let l = sqrt(dot(m, m));
                r = select(vec4<f32>(0.0), m / l, l > 1.0e-20 && !bad(l));
            }
            case 45u: { r = vec4<f32>(A[imm & 3u], A[(imm >> 2u) & 3u], A[(imm >> 4u) & 3u], A[(imm >> 6u) & 3u]); }
            case 46u: { r = vec4<f32>(A.x, B.x, C.x, D.x); }
            case 47u: { r = hash4(A, clamp(imm, 1u, 4u)); }
            case 48u: { r = vec4<f32>(value2(A.xy)); }
            case 49u: { r = vec4<f32>(perlin2(A.xy)); }
            case 50u: { r = vec4<f32>(simplex2(A.xy)); }
            case 51u: { r = voronoi2(A.xy); }
            case 52u: { r = sample_src(imm, A.xy); }
            case 53u: { r = A * B + C; }
            case 54u: { r = exp2(min(A, vec4<f32>(120.0))); }
            case 55u: { r = log2(max(A, vec4<f32>(1.0e-30))); }
            case 56u: { r = trunc(A); }
            case 57u: { r = vec4<f32>(value3(A.xyz)); }
            case 58u: { r = vec4<f32>(perlin3(A.xyz)); }
            default: { }
        }
        regs[dst] = r;
    }
    let c = regs[H.info.y & 63u];
    let a = clamp(fin(c.a), 0.0, 1.0);
    let rgb = clamp(fin4(c).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb * a, a);
}
";

const BLUR: &str = r"
struct Blur {
    dir: vec4<f32>,                 // step u, step v, taps either side, -
    w: array<vec4<f32>, 9>,          // weights, centre first
};
@group(0) @binding(0) var<uniform> P: Blur;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
@group(0) @binding(3) var<uniform> size: vec4<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn weight(i: u32) -> f32 { return P.w[i / 4u][i % 4u]; }

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / size.xy;
    let d = P.dir.xy;
    let n = min(u32(P.dir.z), 32u);
    var acc = textureSampleLevel(src, s, uv, 0.0) * weight(0u);
    for (var i = 1u; i <= n; i++) {
        let o = d * f32(i);
        acc += (textureSampleLevel(src, s, uv - o, 0.0) + textureSampleLevel(src, s, uv + o, 0.0)) * weight(i);
    }
    return acc;
}
";

const PARTICLES: &str = r"
struct View {
    view: vec4<f32>,    // buffer width, height, buffer pixels per logical pixel, splat
};
@group(0) @binding(0) var<uniform> V: View;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) r: f32,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, @location(0) ab: vec4<f32>, @location(1) rad: vec4<f32>, @location(2) color: vec4<f32>) -> Out {
    let k = V.view.z;
    let a = ab.xy * k;
    let b = (ab.xy + ab.zw) * k;
    let r = max(rad.x * k, 0.5);
    let lo = min(a, b) - vec2<f32>(r + 1.0);
    let hi = max(a, b) + vec2<f32>(r + 1.0);
    let corner = vec2<f32>(f32(v & 1u), f32((v >> 1u) & 1u));
    let p = mix(lo, hi, corner);
    var o: Out;
    o.pos = vec4<f32>(p.x / V.view.x * 2.0 - 1.0, 1.0 - p.y / V.view.y * 2.0, 0.0, 1.0);
    o.a = a;
    o.b = b;
    o.r = r;
    o.color = color;
    return o;
}

@fragment
fn fs(o: Out) -> @location(0) vec4<f32> {
    let p = o.pos.xy;
    let pa = p - o.a;
    let ba = o.b - o.a;
    let len = dot(ba, ba);
    let h = select(0.0, clamp(dot(pa, ba) / len, 0.0, 1.0), len > 1.0e-12);
    let d = length(pa - ba * h);
    var cov = 0.0;
    let splat = u32(V.view.w);
    if (splat == 0u) {
        let k = clamp(1.0 - (d / o.r) * (d / o.r), 0.0, 1.0);
        cov = k * k;
    } else if (splat == 1u) {
        cov = clamp(o.r - d + 0.5, 0.0, 1.0);
    } else {
        let w = max(o.r * 0.25, 1.0);
        cov = clamp(1.0 - abs(d - (o.r - w)) / w, 0.0, 1.0);
    }
    let a = o.color.a * cov;
    return vec4<f32>(o.color.rgb * a, a);
}
";

/// The pipelines effects run on, made once per device.
struct Kernels {
    shade: wgpu::RenderPipeline,
    shade_layout: wgpu::BindGroupLayout,
    blur: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    /// Particles, by blend: plus, normal, screen.
    particles: [wgpu::RenderPipeline; 3],
    particle_layout: wgpu::BindGroupLayout,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    /// A transparent 1×1 texture bound where nothing is.
    empty: wgpu::TextureView,
}

fn fullscreen(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
        ty,
        count: None,
    }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn texture(kind: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: kind,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

impl Kernels {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Kernels {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let mut entries = vec![
            entry(0, uniform()),
            entry(1, texture(wgpu::TextureSampleType::Uint)),
            entry(2, texture(wgpu::TextureSampleType::Float { filterable: false })),
            entry(3, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            entry(4, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        ];
        for b in 5..=17 {
            entries.push(entry(b, texture(float)));
        }
        let shade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RemedyUI effect kernel"),
            entries: &entries,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RemedyUI effect kernel"),
            source: wgpu::ShaderSource::Wgsl(KERNEL.into()),
        });
        let shade = fullscreen(device, "RemedyUI effect kernel", &module, &shade_layout, None);

        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RemedyUI effect blur"),
            entries: &[
                entry(0, uniform()),
                entry(1, texture(float)),
                entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(3, uniform()),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RemedyUI effect blur"),
            source: wgpu::ShaderSource::Wgsl(BLUR.into()),
        });
        let blur = fullscreen(device, "RemedyUI effect blur", &module, &blur_layout, None);

        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RemedyUI effect particles"),
            entries: &[entry(0, uniform())],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RemedyUI effect particles"),
            source: wgpu::ShaderSource::Wgsl(PARTICLES.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RemedyUI effect particles"),
            bind_group_layouts: &[Some(&particle_layout)],
            immediate_size: 0,
        });
        let comp = |src, dst| wgpu::BlendComponent {
            src_factor: src,
            dst_factor: dst,
            operation: wgpu::BlendOperation::Add,
        };
        use wgpu::BlendFactor::{One, OneMinusSrc, OneMinusSrcAlpha};
        let blends = [
            wgpu::BlendState { color: comp(One, One), alpha: comp(One, One) },
            wgpu::BlendState { color: comp(One, OneMinusSrcAlpha), alpha: comp(One, OneMinusSrcAlpha) },
            wgpu::BlendState { color: comp(One, OneMinusSrc), alpha: comp(One, OneMinusSrc) },
        ];
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 2 },
        ];
        let particles = blends.map(|blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("RemedyUI effect particles"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 48,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &attrs,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        });
        let sampler = |mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("RemedyUI effect"),
                address_mode_u: mode,
                address_mode_v: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        };
        let empty = target(device, 1, 1, "RemedyUI effect empty");
        queue.write_texture(
            empty.as_image_copy(),
            &[0, 0, 0, 0],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: None },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        Kernels {
            shade,
            shade_layout,
            blur,
            blur_layout,
            particles,
            particle_layout,
            repeat: sampler(wgpu::AddressMode::Repeat),
            clamp: sampler(wgpu::AddressMode::ClampToEdge),
            empty: empty.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// The kernels of `ctx`'s device, made on first use.
    fn of(ctx: &GpuContext) -> Arc<Kernels> {
        static ALL: Mutex<Vec<(u64, Arc<Kernels>)>> = Mutex::new(Vec::new());
        let mut all = ALL.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, k)) = all.iter().find(|(s, _)| *s == ctx.serial) {
            return k.clone();
        }
        let k = Arc::new(Kernels::new(&ctx.device, &ctx.queue));
        // Only the current device's are kept.
        all.clear();
        all.push((ctx.serial, k.clone()));
        k
    }
}

fn target(device: &wgpu::Device, w: u32, h: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn data_texture(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("RemedyUI effect"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    })
}

fn bytes_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn bytes_u32(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A shade stage's program on the GPU.
struct Code {
    code: wgpu::Texture,
    consts: wgpu::Texture,
    const_rows: u32,
    /// Which earlier pass each of the kernel's eight pass bindings shows.
    sampled: Vec<u8>,
}

/// One stage's textures: its buffer, a second for a blur's first pass.
struct StageTargets {
    buffer: wgpu::Texture,
    mid: Option<wgpu::Texture>,
}

/// Renders effects on the GPU. Keep one per element: it holds the effect's buffers, its
/// programs as textures, and its last frame.
pub struct GpuEffect {
    kernels: Arc<Kernels>,
    serial: u64,
    effect: Option<Arc<Effect>>,
    codes: Vec<Option<Code>>,
    targets: Vec<StageTargets>,
    last: Option<wgpu::Texture>,
    /// Theme textures uploaded, by image id.
    images: HashMap<u64, wgpu::Texture>,
    /// Instructions run in the last frame, for tests and the overlay: (pixels, instructions).
    work: (u64, u64),
}

impl std::fmt::Debug for GpuEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuEffect({:?})", self.effect.as_ref().map(|e| &e.name))
    }
}

impl GpuEffect {
    /// An effect renderer on `ctx`'s device (its kernels are made there once).
    pub fn new(ctx: &GpuContext) -> Self {
        GpuEffect {
            kernels: Kernels::of(ctx),
            serial: ctx.serial,
            effect: None,
            codes: Vec::new(),
            targets: Vec::new(),
            last: None,
            images: HashMap::new(),
            work: (0, 0),
        }
    }

    /// The output's size in pixels for `input`.
    pub fn output_size(effect: &Effect, input: &FrameInput) -> (u32, u32) {
        effect.stages.last().map_or((1, 1), |s| buffer_size(effect, s, input))
    }

    /// Pixels shaded and instructions those pixels ran in the latest frame.
    pub fn last_work(&self) -> (u64, u64) {
        self.work
    }

    fn prepare(&mut self, ctx: &GpuContext, effect: &Arc<Effect>) {
        if self.serial != ctx.serial {
            // A new device: everything made on the old one is gone.
            *self = GpuEffect::new(ctx);
        }
        if self.effect.as_ref().is_some_and(|e| Arc::ptr_eq(e, effect)) {
            return;
        }
        self.effect = Some(effect.clone());
        self.targets.clear();
        self.last = None;
        self.codes = effect
            .stages
            .iter()
            .map(|s| match &s.kind {
                StageKind::Shade(p) => {
                    // Earlier passes sampled, mapped onto the kernel's eight bindings.
                    let mut sampled: Vec<u8> = Vec::new();
                    let mut words: Vec<u32> = Vec::with_capacity(p.code.len() * 2);
                    for ins in &p.code {
                        let mut ins = *ins;
                        if ins.op == Op::Sample
                            && let Some(Source::Buffer(b)) = Source::decode(ins.imm)
                        {
                            let slot = match sampled.iter().position(|x| *x == b) {
                                Some(k) => k,
                                None => {
                                    sampled.push(b);
                                    sampled.len() - 1
                                }
                            };
                            ins.imm = 16 + slot as u16;
                        }
                        words.extend(ins.encode());
                    }
                    let n = (p.code.len() as u32).max(1);
                    let rows = n.div_ceil(CODE_ROW);
                    words.resize((rows * CODE_ROW * 2) as usize, 0);
                    let code = data_texture(
                        &ctx.device,
                        CODE_ROW / 2,
                        rows,
                        wgpu::TextureFormat::Rgba32Uint,
                        "RemedyUI effect code",
                    );
                    ctx.queue.write_texture(
                        code.as_image_copy(),
                        &bytes_u32(&words),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(CODE_ROW / 2 * 16),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d { width: CODE_ROW / 2, height: rows, depth_or_array_layers: 1 },
                    );
                    let const_rows = (p.consts.len() as u32).max(1).div_ceil(CONST_ROW);
                    let consts = data_texture(
                        &ctx.device,
                        CONST_ROW,
                        const_rows,
                        wgpu::TextureFormat::Rgba32Float,
                        "RemedyUI effect constants",
                    );
                    Some(Code { code, consts, const_rows, sampled })
                }
                _ => None,
            })
            .collect();
    }

    fn image(&mut self, ctx: &GpuContext, image: &remedy_ui_paint::Image) -> Option<wgpu::TextureView> {
        if image.width == 0
            || image.height == 0
            || image.pixels.len() != image.width as usize * image.height as usize * 4
        {
            return None;
        }
        let max = ctx.device.limits().max_texture_dimension_2d;
        if image.width > max || image.height > max {
            return None;
        }
        let t = self.images.entry(image.id).or_insert_with(|| {
            let t = data_texture(&ctx.device, image.width, image.height, FORMAT, "RemedyUI effect texture");
            ctx.queue.write_texture(
                t.as_image_copy(),
                &image.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(image.width * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
            );
            t
        });
        Some(t.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// Renders `instance` for `input` and copies the picture into `out`, which must be
    /// [`GpuEffect::output_size`] and of [`FORMAT`] with `COPY_DST` usage. One submission.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        instance: &Instance,
        input: &FrameInput,
        out: &wgpu::Texture,
    ) -> Result<(), GpuError> {
        if ctx.is_lost() {
            return Err(GpuError::Lost);
        }
        let effect = instance.effect().clone();
        self.prepare(ctx, &effect);
        let mut input = *input;
        input.max_side = input.max_side.min(ctx.device.limits().max_texture_dimension_2d);
        let uniforms = remedy_ui_effects::uniforms(&effect, instance, &input);
        let k = self.kernels.clone();
        let device = &ctx.device;

        // Buffers at this frame's sizes.
        let sizes: Vec<(u32, u32)> = effect.stages.iter().map(|s| buffer_size(&effect, s, &input)).collect();
        if self.targets.len() != sizes.len()
            || self.targets.iter().zip(&sizes).any(|(t, s)| (t.buffer.width(), t.buffer.height()) != *s)
        {
            self.targets = effect
                .stages
                .iter()
                .zip(&sizes)
                .map(|(s, &(w, h))| StageTargets {
                    buffer: target(device, w, h, "RemedyUI effect buffer"),
                    mid: matches!(s.kind, StageKind::Blur { .. })
                        .then(|| target(device, w, h, "RemedyUI effect blur")),
                })
                .collect();
            self.last = None;
        }
        let textures: Vec<Option<wgpu::TextureView>> =
            instance.textures().iter().map(|t| t.as_ref().and_then(|i| self.image(ctx, i))).collect();
        let views: Vec<wgpu::TextureView> = self
            .targets
            .iter()
            .map(|t| t.buffer.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let last_view = self.last.as_ref().map(|l| l.create_view(&wgpu::TextureViewDescriptor::default()));

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI effect") });
        let mut work = (0u64, 0u64);
        let uniform_buffer = |bytes: &[u8]| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RemedyUI effect"),
                size: bytes.len().max(16) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            ctx.queue.write_buffer(&b, 0, bytes);
            b
        };
        // A blur (or copy, with no taps) of `src` into `dst`.
        let blur_pass = |encoder: &mut wgpu::CommandEncoder,
                         src: &wgpu::TextureView,
                         dst: &wgpu::TextureView,
                         (w, h): (u32, u32),
                         dir: [f32; 2],
                         weights: &[f32]| {
            let mut data = vec![dir[0], dir[1], (weights.len().max(1) - 1) as f32, 0.0];
            let mut wts = weights.to_vec();
            wts.resize(36, 0.0);
            data.extend(wts);
            let params = uniform_buffer(&bytes_f32(&data));
            let size = uniform_buffer(&bytes_f32(&[w as f32, h as f32, 0.0, 0.0]));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RemedyUI effect blur"),
                layout: &k.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&k.clamp) },
                    wgpu::BindGroupEntry { binding: 3, resource: size.as_entire_binding() },
                ],
            });
            let mut rp = pass(encoder, dst, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
            rp.set_pipeline(&k.blur);
            rp.set_bind_group(0, &bind, &[]);
            rp.draw(0..3, 0..1);
        };

        for (si, stage) in effect.stages.iter().enumerate() {
            let (w, h) = sizes.get(si).copied().unwrap_or((1, 1));
            let Some(view) = views.get(si) else { continue };
            match &stage.kind {
                StageKind::Shade(program) => {
                    let Some(Some(code)) = self.codes.get(si) else { continue };
                    let consts = remedy_ui_effects::constants(program, instance.values(), &uniforms);
                    let mut flat: Vec<f32> = consts.iter().flatten().copied().collect();
                    flat.resize((code.const_rows * CONST_ROW * 4) as usize, 0.0);
                    ctx.queue.write_texture(
                        code.consts.as_image_copy(),
                        &bytes_f32(&flat),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(CONST_ROW * 16),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: CONST_ROW,
                            height: code.const_rows,
                            depth_or_array_layers: 1,
                        },
                    );
                    let texel = input.width.max(1.0) / w as f32;
                    let aspect = if input.height > 0.0 { input.width / input.height } else { 1.0 };
                    let time = if input.time.is_finite() { input.time.rem_euclid(3600.0) } else { 0.0 };
                    let mut header = bytes_u32(&[
                        (program.code.len() as u32).min(MAX_CODE),
                        u32::from(program.outputs.first().copied().unwrap_or(0)),
                        0,
                        0,
                    ]);
                    header.extend(bytes_f32(&[
                        w as f32,
                        h as f32,
                        input.width,
                        input.height,
                        time,
                        aspect,
                        texel,
                        if input.light { 1.0 } else { 0.0 },
                    ]));
                    let header = uniform_buffer(&header);
                    let code_view = code.code.create_view(&wgpu::TextureViewDescriptor::default());
                    let const_view = code.consts.create_view(&wgpu::TextureViewDescriptor::default());
                    let tex =
                        |i: usize| textures.get(i).cloned().flatten().unwrap_or_else(|| k.empty.clone());
                    let buf = |i: usize| {
                        code.sampled
                            .get(i)
                            .and_then(|b| views.get(usize::from(*b)))
                            .cloned()
                            .unwrap_or_else(|| k.empty.clone())
                    };
                    let owned: Vec<wgpu::TextureView> = (0..4)
                        .map(tex)
                        .chain((0..8).map(buf))
                        .chain(std::iter::once(last_view.clone().unwrap_or_else(|| k.empty.clone())))
                        .collect();
                    let mut entries = vec![
                        wgpu::BindGroupEntry { binding: 0, resource: header.as_entire_binding() },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&code_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&const_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&k.repeat),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&k.clamp),
                        },
                    ];
                    for (i, v) in owned.iter().enumerate() {
                        entries.push(wgpu::BindGroupEntry {
                            binding: 5 + i as u32,
                            resource: wgpu::BindingResource::TextureView(v),
                        });
                    }
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("RemedyUI effect kernel"),
                        layout: &k.shade_layout,
                        entries: &entries,
                    });
                    let mut rp = pass(&mut encoder, view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
                    rp.set_pipeline(&k.shade);
                    rp.set_bind_group(0, &bind, &[]);
                    rp.draw(0..3, 0..1);
                    let px = u64::from(w) * u64::from(h);
                    work.0 += px;
                    work.1 += px * program.code.len() as u64;
                }
                StageKind::Blur { src, radius } => {
                    let r = remedy_ui_effects::blur_radius(&effect, stage, *radius, &uniforms, &input);
                    let weights = remedy_ui_effects::blur_weights(r);
                    let (Some(src_view), Some(mid)) =
                        (views.get(usize::from(*src)), self.targets.get(si).and_then(|t| t.mid.as_ref()))
                    else {
                        continue;
                    };
                    let mid_view = mid.create_view(&wgpu::TextureViewDescriptor::default());
                    blur_pass(&mut encoder, src_view, &mid_view, (w, h), [1.0 / w as f32, 0.0], &weights);
                    blur_pass(&mut encoder, &mid_view, view, (w, h), [0.0, 1.0 / h as f32], &weights);
                }
                StageKind::Particles { src, emitter } => {
                    match src.and_then(|s| views.get(usize::from(s))) {
                        Some(s) => blur_pass(&mut encoder, s, view, (w, h), [0.0, 0.0], &[1.0]),
                        None => drop(pass(&mut encoder, view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT))),
                    }
                    let sprites = remedy_ui_effects::particles(&effect, stage, instance, &uniforms, &input);
                    if sprites.is_empty() {
                        continue;
                    }
                    let data: Vec<f32> = sprites
                        .iter()
                        .flat_map(|s| {
                            [
                                s.pos[0], s.pos[1], s.trail[0], s.trail[1], s.radius, 0.0, 0.0, 0.0,
                                s.color[0], s.color[1], s.color[2], s.color[3],
                            ]
                        })
                        .collect();
                    let instances = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("RemedyUI effect particles"),
                        size: (data.len() * 4) as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    ctx.queue.write_buffer(&instances, 0, &bytes_f32(&data));
                    let splat = match emitter.splat {
                        Splat::Soft => 0.0,
                        Splat::Disc => 1.0,
                        _ => 2.0,
                    };
                    let per_logical = w as f32 / input.width.max(1.0);
                    let viewu = uniform_buffer(&bytes_f32(&[w as f32, h as f32, per_logical, splat]));
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("RemedyUI effect particles"),
                        layout: &k.particle_layout,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: viewu.as_entire_binding() }],
                    });
                    let pipeline = match emitter.blend {
                        Blend::Plus => &k.particles[0],
                        Blend::Screen => &k.particles[2],
                        _ => &k.particles[1],
                    };
                    let mut rp = pass(&mut encoder, view, wgpu::LoadOp::Load);
                    rp.set_pipeline(pipeline);
                    rp.set_bind_group(0, &bind, &[]);
                    rp.set_vertex_buffer(0, instances.slice(..));
                    rp.draw(0..4, 0..sprites.len() as u32);
                }
            }
        }
        let (ow, oh) = sizes.last().copied().unwrap_or((1, 1));
        if let Some(last) = self.targets.last() {
            if (out.width(), out.height()) == (ow, oh) {
                encoder.copy_texture_to_texture(
                    last.buffer.as_image_copy(),
                    out.as_image_copy(),
                    wgpu::Extent3d { width: ow, height: oh, depth_or_array_layers: 1 },
                );
            }
            if effect.feedback {
                let keep = match self.last.take() {
                    Some(t) if (t.width(), t.height()) == (ow, oh) => t,
                    _ => target(device, ow, oh, "RemedyUI effect last frame"),
                };
                encoder.copy_texture_to_texture(
                    last.buffer.as_image_copy(),
                    keep.as_image_copy(),
                    wgpu::Extent3d { width: ow, height: oh, depth_or_array_layers: 1 },
                );
                self.last = Some(keep);
            }
        }
        ctx.queue.submit([encoder.finish()]);
        self.work = work;
        Ok(())
    }
}
