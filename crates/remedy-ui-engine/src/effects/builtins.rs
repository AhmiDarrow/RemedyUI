//! The built-in function library of `.rfx`: each call lowers to kernel instructions (or to a
//! few of them: `fbm` is octaves of gradient noise, `cross` is two swizzled products).
//! Functions written in `.rfx` itself (shapes, colour, 3D helpers) are in the prelude.

use crate::effects::compile::{Compiler, Lower, Ty, Val, swz, unify};
use crate::effects::ir::{Input, Op};
use crate::effects::syntax::{Diagnostic, Span};

type R<T> = Result<T, Diagnostic>;

fn arity(name: &str, vals: &[(Val, Span)], n: usize, at: Span) -> R<()> {
    if vals.len() == n {
        Ok(())
    } else {
        Err(Diagnostic::new(
            at,
            format!("`{name}` takes {n} argument{}, given {}", if n == 1 { "" } else { "s" }, vals.len()),
        ))
    }
}

fn v(c: &Compiler<'_>, vals: &[(Val, Span)], k: usize, at: Span) -> R<(u32, Ty)> {
    match vals.get(k) {
        Some((val, s)) => c.vector(val, *s),
        None => Err(Diagnostic::new(at, "an argument is missing")),
    }
}

fn want(t: Ty, ok: &[Ty], name: &str, at: Span) -> R<()> {
    if ok.contains(&t) {
        Ok(())
    } else {
        let list: Vec<&str> = ok.iter().map(|t| t.name()).collect();
        Err(Diagnostic::new(at, format!("`{name}` takes a {}, given a {}", list.join(" or "), t.name())))
    }
}

/// Lowers built-in `name` applied to `vals`.
pub(crate) fn call(
    c: &mut Compiler<'_>,
    name: &str,
    vals: &[(Val, Span)],
    at: Span,
    particle: bool,
) -> R<Val> {
    let unary = |op: Op| -> Option<Op> { Some(op) };
    let component = match name {
        "abs" => unary(Op::Abs),
        "sign" => unary(Op::Sign),
        "floor" => unary(Op::Floor),
        "ceil" => unary(Op::Ceil),
        "fract" => unary(Op::Fract),
        "round" => unary(Op::Round),
        "trunc" => unary(Op::Trunc),
        "sqrt" => unary(Op::Sqrt),
        "exp" => unary(Op::Exp),
        "exp2" => unary(Op::Exp2),
        "log" => unary(Op::Log),
        "log2" => unary(Op::Log2),
        "sin" => unary(Op::Sin),
        "cos" => unary(Op::Cos),
        "tan" => unary(Op::Tan),
        "asin" => unary(Op::Asin),
        "acos" => unary(Op::Acos),
        _ => None,
    };
    if let Some(op) = component {
        arity(name, vals, 1, at)?;
        let (x, t) = v(c, vals, 0, at)?;
        return Ok(Val::V(c.op(op, 0, &[x], at)?, t));
    }
    let binary = match name {
        "min" => Some(Op::Min),
        "max" => Some(Op::Max),
        "pow" => Some(Op::Pow),
        "mod" => Some(Op::Mod),
        "step" => Some(Op::Step),
        "atan2" => Some(Op::Atan2),
        "atan" if vals.len() == 2 => Some(Op::Atan2),
        _ => None,
    };
    if let Some(op) = binary {
        arity(name, vals, 2, at)?;
        let (a, ta) = v(c, vals, 0, at)?;
        let (b, tb) = v(c, vals, 1, at)?;
        let t = unify(ta, tb)
            .ok_or_else(|| Diagnostic::new(at, format!("`{name}` of a {} and a {}", ta.name(), tb.name())))?;
        return Ok(Val::V(c.op(op, 0, &[a, b], at)?, t));
    }
    let ternary = match name {
        "clamp" => Some(Op::Clamp),
        "mix" | "lerp" => Some(Op::Mix),
        "smoothstep" => Some(Op::Smoothstep),
        "fma" => Some(Op::Fma),
        _ => None,
    };
    if let Some(op) = ternary {
        arity(name, vals, 3, at)?;
        let (a, ta) = v(c, vals, 0, at)?;
        let (b, tb) = v(c, vals, 1, at)?;
        let (d, td) = v(c, vals, 2, at)?;
        let t = unify(ta, tb).and_then(|t| unify(t, td)).ok_or_else(|| {
            Diagnostic::new(at, format!("`{name}` of a {}, a {} and a {}", ta.name(), tb.name(), td.name()))
        })?;
        return Ok(Val::V(c.op(op, 0, &[a, b, d], at)?, t));
    }
    match name {
        "atan" => {
            arity(name, vals, 1, at)?;
            let (x, t) = v(c, vals, 0, at)?;
            Ok(Val::V(c.op(Op::Atan, 0, &[x], at)?, t))
        }
        "select" => {
            // select(if_false, if_true, condition)
            arity(name, vals, 3, at)?;
            let (a, ta) = v(c, vals, 0, at)?;
            let (b, tb) = v(c, vals, 1, at)?;
            let (k, tk) = v(c, vals, 2, at)?;
            want(tk, &[Ty::F], "select's condition", at)?;
            let t =
                unify(ta, tb).ok_or_else(|| Diagnostic::new(at, "select's two values differ in shape"))?;
            Ok(Val::V(c.op(Op::Select, 0, &[a, b, k], at)?, t))
        }
        "saturate" => {
            arity(name, vals, 1, at)?;
            let (x, t) = v(c, vals, 0, at)?;
            let (z, o) = (c.konst([0.0; 4], at)?, c.konst([1.0; 4], at)?);
            Ok(Val::V(c.op(Op::Clamp, 0, &[x, z, o], at)?, t))
        }
        "radians" | "degrees" => {
            arity(name, vals, 1, at)?;
            let (x, t) = v(c, vals, 0, at)?;
            let k =
                if name == "radians" { std::f32::consts::PI / 180.0 } else { 180.0 / std::f32::consts::PI };
            let k = c.konst([k; 4], at)?;
            Ok(Val::V(c.op(Op::Mul, 0, &[x, k], at)?, t))
        }
        "dot" => {
            arity(name, vals, 2, at)?;
            let (a, ta) = v(c, vals, 0, at)?;
            let (b, tb) = v(c, vals, 1, at)?;
            if ta != tb {
                return Err(Diagnostic::new(at, format!("dot of a {} and a {}", ta.name(), tb.name())));
            }
            Ok(Val::V(c.op(Op::Dot, ta.n() as u16, &[a, b], at)?, Ty::F))
        }
        "length" => {
            arity(name, vals, 1, at)?;
            let (a, t) = v(c, vals, 0, at)?;
            if t == Ty::F {
                return Ok(Val::V(c.op(Op::Abs, 0, &[a], at)?, Ty::F));
            }
            Ok(Val::V(c.op(Op::Length, t.n() as u16, &[a], at)?, Ty::F))
        }
        "distance" => {
            arity(name, vals, 2, at)?;
            let (a, ta) = v(c, vals, 0, at)?;
            let (b, tb) = v(c, vals, 1, at)?;
            let t = unify(ta, tb).ok_or_else(|| Diagnostic::new(at, "distance between different shapes"))?;
            let d = c.op(Op::Sub, 0, &[a, b], at)?;
            if t == Ty::F {
                return Ok(Val::V(c.op(Op::Abs, 0, &[d], at)?, Ty::F));
            }
            Ok(Val::V(c.op(Op::Length, t.n() as u16, &[d], at)?, Ty::F))
        }
        "normalize" => {
            arity(name, vals, 1, at)?;
            let (a, t) = v(c, vals, 0, at)?;
            want(t, &[Ty::V2, Ty::V3, Ty::V4], name, at)?;
            Ok(Val::V(c.op(Op::Normalize, t.n() as u16, &[a], at)?, t))
        }
        "cross" => {
            arity(name, vals, 2, at)?;
            let (a, ta) = v(c, vals, 0, at)?;
            let (b, tb) = v(c, vals, 1, at)?;
            want(ta, &[Ty::V3], name, at)?;
            want(tb, &[Ty::V3], name, at)?;
            let yzx = swz(&[1, 2, 0, 3]);
            let zxy = swz(&[2, 0, 1, 3]);
            let a1 = c.op(Op::Swizzle, yzx, &[a], at)?;
            let b1 = c.op(Op::Swizzle, zxy, &[b], at)?;
            let a2 = c.op(Op::Swizzle, zxy, &[a], at)?;
            let b2 = c.op(Op::Swizzle, yzx, &[b], at)?;
            let p = c.op(Op::Mul, 0, &[a1, b1], at)?;
            let q = c.op(Op::Mul, 0, &[a2, b2], at)?;
            Ok(Val::V(c.op(Op::Sub, 0, &[p, q], at)?, Ty::V3))
        }
        "reflect" => {
            // i - 2 dot(n, i) n
            arity(name, vals, 2, at)?;
            let (i, ti) = v(c, vals, 0, at)?;
            let (n, tn) = v(c, vals, 1, at)?;
            if ti != tn || ti == Ty::F {
                return Err(Diagnostic::new(at, "reflect(incident, normal) takes two vectors of one size"));
            }
            let d = c.op(Op::Dot, ti.n() as u16, &[n, i], at)?;
            let two = c.konst([-2.0; 4], at)?;
            let k = c.op(Op::Mul, 0, &[d, two], at)?;
            Ok(Val::V(c.op(Op::Fma, 0, &[n, k, i], at)?, ti))
        }
        "refract" => {
            arity(name, vals, 3, at)?;
            let (i, ti) = v(c, vals, 0, at)?;
            let (n, tn) = v(c, vals, 1, at)?;
            let (eta, te) = v(c, vals, 2, at)?;
            if ti != tn || ti == Ty::F || te != Ty::F {
                return Err(Diagnostic::new(at, "refract(incident, normal, eta)"));
            }
            let d = c.op(Op::Dot, ti.n() as u16, &[n, i], at)?;
            let one = c.konst([1.0; 4], at)?;
            let d2 = c.op(Op::Mul, 0, &[d, d], at)?;
            let e2 = c.op(Op::Mul, 0, &[eta, eta], at)?;
            let r = c.op(Op::Sub, 0, &[one, d2], at)?;
            let r = c.op(Op::Mul, 0, &[e2, r], at)?;
            let k = c.op(Op::Sub, 0, &[one, r], at)?;
            let sk = c.op(Op::Sqrt, 0, &[k], at)?;
            let ed = c.op(Op::Mul, 0, &[eta, d], at)?;
            let f = c.op(Op::Add, 0, &[ed, sk], at)?;
            let ei = c.op(Op::Mul, 0, &[eta, i], at)?;
            let fnn = c.op(Op::Mul, 0, &[f, n], at)?;
            let out = c.op(Op::Sub, 0, &[ei, fnn], at)?;
            let zero = c.konst([0.0; 4], at)?;
            let ok = c.op(Op::Ge, 0, &[k, zero], at)?;
            Ok(Val::V(c.op(Op::Select, 0, &[zero, out, ok], at)?, ti))
        }
        "hash" | "hash2" | "hash3" | "hash4" => {
            arity(name, vals, 1, at)?;
            let (p, t) = v(c, vals, 0, at)?;
            let h = c.op(Op::Hash, t.n() as u16, &[p], at)?;
            let n = match name {
                "hash" => 1,
                "hash2" => 2,
                "hash3" => 3,
                _ => 4,
            };
            if n == 1 {
                Ok(Val::V(c.op(Op::Swizzle, swz(&[0, 0, 0, 0]), &[h], at)?, Ty::F))
            } else {
                Ok(Val::V(h, Ty::vec(n)))
            }
        }
        "noise" | "perlin" | "simplex" | "voronoi" => {
            arity(name, vals, 1, at)?;
            let (p, t) = v(c, vals, 0, at)?;
            let p = if t == Ty::F {
                let z = c.konst([0.0; 4], at)?;
                c.combine(&[p, z], at)?
            } else {
                p
            };
            let t = if t == Ty::F { Ty::V2 } else { t };
            let op = match (name, t) {
                ("noise", Ty::V2) => Op::Noise,
                ("noise", Ty::V3) => Op::Noise3,
                ("perlin", Ty::V2) => Op::Perlin,
                ("perlin", Ty::V3) => Op::Perlin3,
                ("simplex", Ty::V2) => Op::Simplex,
                ("voronoi", Ty::V2) => Op::Voronoi,
                _ => {
                    return Err(Diagnostic::new(
                        at,
                        format!(
                            "`{name}` takes a {}, given a {}",
                            if matches!(name, "noise" | "perlin") { "vec2 or vec3" } else { "vec2" },
                            t.name()
                        ),
                    ));
                }
            };
            let out = c.op(op, 0, &[p], at)?;
            Ok(Val::V(out, if op == Op::Voronoi { Ty::V3 } else { Ty::F }))
        }
        "fbm" | "ridged" | "turbulence" => {
            // fbm(p, octaves[, gain[, lacunarity]]): octaves of gradient noise, 0 to 1.
            if !(2..=4).contains(&vals.len()) {
                return Err(Diagnostic::new(at, format!("{name}(p, octaves[, gain[, lacunarity]])")));
            }
            let (p, t) = v(c, vals, 0, at)?;
            want(t, &[Ty::V2, Ty::V3], name, at)?;
            let (o, _) = v(c, vals, 1, at)?;
            let octaves = c
                .is_const(o)
                .map(|k| k[0])
                .filter(|k| k.fract() == 0.0 && (1.0..=8.0).contains(k))
                .ok_or_else(|| Diagnostic::new(at, "octaves is a whole number from 1 to 8"))?
                as u32;
            let gain = match vals.get(2) {
                Some(_) => v(c, vals, 2, at)?.0,
                None => c.konst([0.5; 4], at)?,
            };
            let lac = match vals.get(3) {
                Some(_) => v(c, vals, 3, at)?.0,
                None => c.konst([2.0; 4], at)?,
            };
            let noise = if t == Ty::V2 { Op::Perlin } else { Op::Perlin3 };
            let mut sum: Option<u32> = None;
            let mut norm: Option<u32> = None;
            let mut amp = c.konst([1.0; 4], at)?;
            let mut q = p;
            let one = c.konst([1.0; 4], at)?;
            let two = c.konst([2.0; 4], at)?;
            for k in 0..octaves {
                // Each octave is moved so the octaves' lattices do not line up.
                let shift = c.konst([17.13 * k as f32, 9.71 * k as f32, 5.37 * k as f32, 0.0], at)?;
                let qs = c.op(Op::Add, 0, &[q, shift], at)?;
                let n = c.op(noise, 0, &[qs], at)?;
                let n = match name {
                    "fbm" => n,
                    // |2n - 1|: creases at the noise's zeros.
                    _ => {
                        let minus = c.konst([-1.0; 4], at)?;
                        let s = c.op(Op::Fma, 0, &[n, two, minus], at)?;
                        let a = c.op(Op::Abs, 0, &[s], at)?;
                        if name == "ridged" {
                            let r = c.op(Op::Sub, 0, &[one, a], at)?;
                            c.op(Op::Mul, 0, &[r, r], at)?
                        } else {
                            a
                        }
                    }
                };
                sum = Some(match sum {
                    None => c.op(Op::Mul, 0, &[n, amp], at)?,
                    Some(s) => c.op(Op::Fma, 0, &[n, amp, s], at)?,
                });
                norm = Some(match norm {
                    None => amp,
                    Some(s) => c.op(Op::Add, 0, &[s, amp], at)?,
                });
                amp = c.op(Op::Mul, 0, &[amp, gain], at)?;
                q = c.op(Op::Mul, 0, &[q, lac], at)?;
            }
            let (Some(s), Some(n)) = (sum, norm) else { return Err(Diagnostic::new(at, "no octaves")) };
            Ok(Val::V(c.op(Op::Div, 0, &[s, n], at)?, Ty::F))
        }
        "sample" => {
            if particle {
                return Err(Diagnostic::new(
                    at,
                    "particles cannot sample (they run once per particle, not per pixel)",
                ));
            }
            arity(name, vals, 2, at)?;
            let src = match vals.first() {
                Some((Val::Src(s), _)) => *s,
                _ => return Err(Diagnostic::new(at, "sample(texture or pass, uv)")),
            };
            let (uv, t) = v(c, vals, 1, at)?;
            want(t, &[Ty::V2], "sample's uv", at)?;
            Ok(Val::V(c.op(Op::Sample, src.encode(), &[uv], at)?, Ty::V4))
        }
        "height_normal" => {
            // height_normal(src, uv, strength): a surface normal from the source's brightness
            // (a height map), by central differences one buffer pixel apart.
            if particle {
                return Err(Diagnostic::new(at, "particles cannot sample"));
            }
            arity(name, vals, 3, at)?;
            let src = match vals.first() {
                Some((Val::Src(s), _)) => *s,
                _ => return Err(Diagnostic::new(at, "height_normal(texture or pass, uv, strength)")),
            };
            let (uv, _) = v(c, vals, 1, at)?;
            let (k, _) = v(c, vals, 2, at)?;
            let texel = c.input(Input::Texel, at)?;
            let size = c.input(Input::Size, at)?;
            let step = c.op(Op::Div, 0, &[texel, size], at)?;
            let sx = c.op(Op::Swizzle, swz(&[0, 0, 0, 0]), &[step], at)?;
            let sy = c.op(Op::Swizzle, swz(&[1, 1, 1, 1]), &[step], at)?;
            let zero = c.konst([0.0; 4], at)?;
            let dx = c.combine(&[sx, zero], at)?;
            let dy = c.combine(&[zero, sy], at)?;
            let w = c.konst([0.2126, 0.7152, 0.0722, 0.0], at)?;
            let h = |c: &mut Compiler<'_>, off: u32, neg: bool| -> R<u32> {
                let p = c.op(if neg { Op::Sub } else { Op::Add }, 0, &[uv, off], at)?;
                let s = c.op(Op::Sample, src.encode(), &[p], at)?;
                let l = c.op(Op::Dot, 3, &[s, w], at)?;
                let a = c.op(Op::Swizzle, swz(&[3, 3, 3, 3]), &[s], at)?;
                c.op(Op::Mul, 0, &[l, a], at)
            };
            let (l, r, u, d) = (h(c, dx, true)?, h(c, dx, false)?, h(c, dy, true)?, h(c, dy, false)?);
            let gx = c.op(Op::Sub, 0, &[l, r], at)?;
            let gy = c.op(Op::Sub, 0, &[u, d], at)?;
            let gx = c.op(Op::Mul, 0, &[gx, k], at)?;
            let gy = c.op(Op::Mul, 0, &[gy, k], at)?;
            let one = c.konst([1.0; 4], at)?;
            let n = c.combine(&[gx, gy, one], at)?;
            Ok(Val::V(c.op(Op::Normalize, 3, &[n], at)?, Ty::V3))
        }
        "rand" => {
            if !particle {
                return Err(Diagnostic::new(at, "rand(k) is for particles; passes use hash(p)"));
            }
            arity(name, vals, 1, at)?;
            let (k, _) = v(c, vals, 0, at)?;
            let id = c.input(Input::Id, at)?;
            let seed = c.input(Input::Seed, at)?;
            let big = c.konst([65_536.0; 4], at)?;
            let s = c.op(Op::Mul, 0, &[seed, big], at)?;
            let p = c.combine(&[id, k, s], at)?;
            let h = c.op(Op::Hash, 3, &[p], at)?;
            Ok(Val::V(c.op(Op::Swizzle, swz(&[0, 0, 0, 0]), &[h], at)?, Ty::F))
        }
        "luma" => {
            arity(name, vals, 1, at)?;
            let (x, t) = v(c, vals, 0, at)?;
            want(t, &[Ty::V3, Ty::V4], name, at)?;
            let w = c.konst([0.2126, 0.7152, 0.0722, 0.0], at)?;
            Ok(Val::V(c.op(Op::Dot, 3, &[x, w], at)?, Ty::F))
        }
        "inverse_lerp" => {
            arity(name, vals, 3, at)?;
            let (a, _) = v(c, vals, 0, at)?;
            let (b, _) = v(c, vals, 1, at)?;
            let (x, t) = v(c, vals, 2, at)?;
            let num = c.op(Op::Sub, 0, &[x, a], at)?;
            let den = c.op(Op::Sub, 0, &[b, a], at)?;
            Ok(Val::V(c.op(Op::Div, 0, &[num, den], at)?, t))
        }
        "remap" => {
            // remap(x, from_lo, from_hi, to_lo, to_hi)
            arity(name, vals, 5, at)?;
            let (x, t) = v(c, vals, 0, at)?;
            let (a, _) = v(c, vals, 1, at)?;
            let (b, _) = v(c, vals, 2, at)?;
            let (lo, _) = v(c, vals, 3, at)?;
            let (hi, _) = v(c, vals, 4, at)?;
            let num = c.op(Op::Sub, 0, &[x, a], at)?;
            let den = c.op(Op::Sub, 0, &[b, a], at)?;
            let k = c.op(Op::Div, 0, &[num, den], at)?;
            Ok(Val::V(c.op(Op::Mix, 0, &[lo, hi, k], at)?, t))
        }
        "transpose" => {
            arity(name, vals, 1, at)?;
            let Some((Val::M(cols, t), _)) = vals.first() else {
                return Err(Diagnostic::new(at, "transpose takes a matrix"));
            };
            let n = cols.len();
            let mut out = Vec::new();
            for row in 0..n {
                let mut comps = Vec::new();
                for col in cols {
                    comps.push(c.op(Op::Swizzle, swz(&[row as u8; 4]), &[*col], at)?);
                }
                out.push(c.combine(&comps, at)?);
            }
            Ok(Val::M(out, *t))
        }
        _ => Err(Diagnostic::new(at, format!("`{name}` is not a function"))),
    }
}

/// Functions written in `.rfx` itself, available to every effect.
pub(crate) const PRELUDE: &str = r#"
effect prelude {
// ---- 2D shapes: signed distances (negative inside) ----
fn sd_circle(p: vec2, r: float) -> float { return length(p) - r; }
fn sd_box(p: vec2, b: vec2) -> float {
    let d = abs(p) - b;
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0);
}
fn sd_round_box(p: vec2, b: vec2, r: float) -> float { return sd_box(p, b - vec2(r)) - r; }
fn sd_segment(p: vec2, a: vec2, b: vec2) -> float {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.000001), 0.0, 1.0);
    return length(pa - ba * h);
}
fn sd_ring(p: vec2, r: float, w: float) -> float { return abs(length(p) - r) - w * 0.5; }
fn sd_triangle(p: vec2, r: float) -> float {
    let k = 1.7320508;
    var q = vec2(abs(p.x) - r, p.y + r / k);
    if q.x + k * q.y > 0.0 { q = vec2(q.x - k * q.y, -k * q.x - q.y) / 2.0; }
    let qx = q.x - clamp(q.x, -2.0 * r, 0.0);
    return -length(vec2(qx, q.y)) * sign(q.y);
}
fn sd_star(p: vec2, r: float, points: float, inner: float) -> float {
    let a = atan2(p.y, p.x);
    let seg = tau / points;
    let m = abs(mod(a, seg) - seg * 0.5) / (seg * 0.5);
    return length(p) - r * mix(inner, 1.0, pow(m, 2.0));
}
// Smooth minimum and maximum: blends shapes with a fillet of size k.
fn smin(a: float, b: float, k: float) -> float {
    let h = clamp(0.5 + 0.5 * (b - a) / max(k, 0.000001), 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}
fn smax(a: float, b: float, k: float) -> float { return -smin(-a, -b, k); }
// Coverage of a distance at this pixel, antialiased over one buffer pixel.
fn fill(d: float) -> float { return clamp(0.5 - d / texel, 0.0, 1.0); }
fn stroke(d: float, w: float) -> float { return fill(abs(d) - w * 0.5); }
fn glow_of(d: float, radius: float) -> float { return exp(-max(d, 0.0) / max(radius, 0.0001)); }

// ---- 2D space ----
fn rotate(p: vec2, a: float) -> vec2 {
    let c = cos(a);
    let s = sin(a);
    return vec2(c * p.x - s * p.y, s * p.x + c * p.y);
}
fn polar(p: vec2) -> vec2 { return vec2(length(p), atan2(p.y, p.x)); }
fn from_polar(r: float, a: float) -> vec2 { return vec2(cos(a), sin(a)) * r; }
fn centered(uv: vec2) -> vec2 { return (uv - vec2(0.5)) * vec2(aspect, 1.0); }
fn tile(p: vec2, cell: vec2) -> vec2 { return mod(p, cell) - cell * 0.5; }
fn barrel(uv: vec2, k: float) -> vec2 {
    let c = uv * 2.0 - vec2(1.0);
    return (c * (1.0 + k * dot(c, c))) * 0.5 + vec2(0.5);
}
fn warp(p: vec2, amount: float, octaves: float) -> vec2 {
    return p + vec2(fbm(p + vec2(1.7, 9.2), 3), fbm(p + vec2(8.3, 2.8), 3)) * amount;
}

// ---- colour ----
fn hsv(h: float, s: float, v: float) -> vec3 {
    let k = vec3(1.0, 0.6666667, 0.3333333);
    let p = abs(fract(vec3(h) + k) * 6.0 - vec3(3.0));
    return v * mix(vec3(1.0), clamp(p - vec3(1.0), 0.0, 1.0), s);
}
fn palette(t: float, a: vec3, b: vec3, c: vec3, d: vec3) -> vec3 { return a + b * cos(tau * (c * t + d)); }
fn ramp3(t: float, a: vec4, b: vec4, c: vec4) -> vec4 {
    return mix(mix(a, b, clamp(t * 2.0, 0.0, 1.0)), c, clamp(t * 2.0 - 1.0, 0.0, 1.0));
}
fn ramp4(t: float, a: vec4, b: vec4, c: vec4, d: vec4) -> vec4 {
    let x = clamp(t, 0.0, 1.0) * 3.0;
    return mix(mix(mix(a, b, clamp(x, 0.0, 1.0)), c, clamp(x - 1.0, 0.0, 1.0)), d, clamp(x - 2.0, 0.0, 1.0));
}
fn over(top: vec4, bottom: vec4) -> vec4 {
    let a = top.a + bottom.a * (1.0 - top.a);
    let rgb = (top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / max(a, 0.000001);
    return vec4(rgb, a);
}
fn screen(a: vec3, b: vec3) -> vec3 { return vec3(1.0) - (vec3(1.0) - a) * (vec3(1.0) - b); }
fn tonemap(c: vec3) -> vec3 { return c / (c + vec3(1.0)); }
fn aces(c: vec3) -> vec3 {
    return clamp((c * (2.51 * c + vec3(0.03))) / (c * (2.43 * c + vec3(0.59)) + vec3(0.14)), 0.0, 1.0);
}
fn vignette(uv: vec2, amount: float) -> float {
    let d = uv - vec2(0.5);
    return clamp(1.0 - dot(d, d) * amount * 4.0, 0.0, 1.0);
}

// ---- 3D: shapes (signed distances) ----
fn sd_sphere(p: vec3, r: float) -> float { return length(p) - r; }
fn sd_box3(p: vec3, b: vec3) -> float {
    let q = abs(p) - b;
    return length(max(q, vec3(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}
fn sd_round_box3(p: vec3, b: vec3, r: float) -> float { return sd_box3(p, b - vec3(r)) - r; }
fn sd_torus(p: vec3, major: float, minor: float) -> float {
    let q = vec2(length(p.xz) - major, p.y);
    return length(q) - minor;
}
fn sd_capsule(p: vec3, a: vec3, b: vec3, r: float) -> float {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.000001), 0.0, 1.0);
    return length(pa - ba * h) - r;
}
fn sd_cylinder(p: vec3, h: float, r: float) -> float {
    let d = abs(vec2(length(p.xz), p.y)) - vec2(r, h);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2(0.0)));
}
fn sd_plane(p: vec3, n: vec3, h: float) -> float { return dot(p, n) + h; }
fn sd_octahedron(p: vec3, s: float) -> float {
    let q = abs(p);
    return (q.x + q.y + q.z - s) * 0.57735027;
}

// ---- 3D: space ----
fn rotate_x(p: vec3, a: float) -> vec3 { let c = cos(a); let s = sin(a); return vec3(p.x, c * p.y - s * p.z, s * p.y + c * p.z); }
fn rotate_y(p: vec3, a: float) -> vec3 { let c = cos(a); let s = sin(a); return vec3(c * p.x + s * p.z, p.y, -s * p.x + c * p.z); }
fn rotate_z(p: vec3, a: float) -> vec3 { let c = cos(a); let s = sin(a); return vec3(c * p.x - s * p.y, s * p.x + c * p.y, p.z); }
fn repeat3(p: vec3, cell: vec3) -> vec3 { return mod(p + cell * 0.5, cell) - cell * 0.5; }
fn twist(p: vec3, k: float) -> vec3 { let r = rotate(p.xz, p.y * k); return vec3(r.x, p.y, r.y); }
// A camera basis looking from `eye` at `target`.
fn look_at(eye: vec3, target: vec3, up: vec3) -> mat3 {
    let f = normalize(target - eye);
    let r = normalize(cross(f, up));
    let u = cross(r, f);
    return mat3(r, u, f);
}
// The view ray through this pixel for a pinhole camera with a vertical field of view (radians).
fn camera_ray(uv: vec2, eye: vec3, target: vec3, fov: float) -> vec3 {
    let p = (uv * 2.0 - vec2(1.0)) * vec2(aspect, -1.0);
    let z = 1.0 / tan(fov * 0.5);
    return normalize(look_at(eye, target, vec3(0.0, 1.0, 0.0)) * vec3(p, z));
}
// Where a 3D point lands on screen (uv), for a camera like camera_ray's; z is its depth.
fn project(p: vec3, eye: vec3, target: vec3, fov: float) -> vec3 {
    let m = transpose(look_at(eye, target, vec3(0.0, 1.0, 0.0)));
    let v = m * (p - eye);
    let z = 1.0 / tan(fov * 0.5);
    let s = v.xy * z / max(v.z, 0.0001);
    return vec3(s.x / aspect * 0.5 + 0.5, 0.5 - s.y * 0.5, v.z);
}

// ---- lighting ----
fn lambert(n: vec3, l: vec3) -> float { return max(dot(n, l), 0.0); }
fn blinn(n: vec3, l: vec3, v: vec3, shininess: float) -> float {
    return pow(max(dot(n, normalize(l + v)), 0.0), shininess);
}
fn fresnel(cos_theta: float, f0: float) -> float { return f0 + (1.0 - f0) * pow(1.0 - clamp(cos_theta, 0.0, 1.0), 5.0); }
// Cook-Torrance with GGX: the light a rough surface reflects towards the viewer.
fn ggx(n: vec3, l: vec3, v: vec3, roughness: float, f0: float) -> float {
    let h = normalize(l + v);
    let a = max(roughness * roughness, 0.002);
    let nh = max(dot(n, h), 0.0);
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 0.0001);
    let d = a * a / (pi * pow(nh * nh * (a * a - 1.0) + 1.0, 2.0));
    let k = a * 0.5;
    let g = nl / (nl * (1.0 - k) + k) * nv / (nv * (1.0 - k) + k);
    return d * g * fresnel(dot(h, v), f0) / (4.0 * nv + 0.0001) ;
}
fn rim(n: vec3, v: vec3, power: float) -> float { return pow(1.0 - max(dot(n, v), 0.0), power); }
fn fog(c: vec3, fog_color: vec3, distance: float, density: float) -> vec3 {
    return mix(fog_color, c, exp(-distance * density));
}
fn sky(dir: vec3, horizon: vec3, zenith: vec3) -> vec3 { return mix(horizon, zenith, pow(max(dir.y, 0.0), 0.6)); }
}
"#;
