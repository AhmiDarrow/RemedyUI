//! The `.rfx` compiler: what it accepts, what it refuses (with where), what it optimizes,
//! and hostile sources (the red team): every one must fail fast with a located error or
//! compile into something bounded, never hang, overflow the stack or panic.

use std::time::{Duration, Instant};

use remedy_ui_effects::ir::{Op, StageKind};
use remedy_ui_effects::{CpuEffect, Effect, FrameInput, Instance, Limits, Value};

fn compile(src: &str) -> Result<Effect, String> {
    Effect::compile(src).map_err(|e| e.to_string())
}

fn untrusted(src: &str) -> Result<Effect, String> {
    Effect::compile_with(src, &Limits::untrusted()).map_err(|e| e.to_string())
}

fn err(src: &str) -> String {
    match compile(src) {
        Ok(_) => String::from("<compiled>"),
        Err(e) => e,
    }
}

fn shade(e: &Effect, i: usize) -> &remedy_ui_effects::ir::Program {
    match e.stages.get(i).map(|s| &s.kind) {
        Some(StageKind::Shade(p)) => p,
        other => unreachable!("stage {i} is {other:?}"),
    }
}

fn ops(p: &remedy_ui_effects::ir::Program, op: Op) -> usize {
    p.code.iter().filter(|i| i.op == op).count()
}

/// Renders a small frame and returns its pixels.
fn render(e: Effect, t: f32) -> Vec<u8> {
    let inst = Instance::new(e.shared());
    CpuEffect::new().with_threads(1).render(&inst, &FrameInput::new(32.0, 24.0, 1.0, t)).pixels
}

#[test]
fn a_minimal_effect_compiles_and_renders() {
    let e = compile("effect red { scale 1.0; pass p { out = #ff0000; } }");
    let px = render(e.unwrap_or_else(|e| unreachable!("{e}")), 0.0);
    assert_eq!(px.get(..4), Some(&[255u8, 0, 0, 255][..]));
}

#[test]
fn errors_are_located_and_say_what() {
    let cases = [
        ("effect x {\n pass p { out = nope; }\n}", "2:17", "`nope` is not defined"),
        ("effect x { pass p { out = vec2(1.0) + vec3(1.0); } }", "1:37", "cannot be combined"),
        ("effect x { pass p { let a = 1.0; } }", "1:12", "must set `out`"),
        ("effect x { pass p { let a = 1.0; a = 2.0; out = a; } }", "1:34", "is a `let`"),
        ("effect x { blend sideways; pass p { out = 1.0; } }", "1:12", "blend is"),
        ("effect x { fps 500; pass p { out = 1.0; } }", "1:12", "fps must be"),
        ("effect x { pass p { out = sample(nothing, uv); } }", "", "`nothing` is not defined"),
        ("effect x { particles s (count: 4) { pos = uv; } }", "", "cannot be used in particles"),
        (
            "effect x { pass a { out = 1.0; } particles s (count: 4) { pos = vec2(sample(a, vec2(0.0)).x); } }",
            "",
            "only passes sample",
        ),
        ("effect x { pass p { out = fbm(uv, 12); } }", "", "octaves"),
        ("effect x { pass p { out = uv.q; } }", "", "not a swizzle"),
        ("effect x { pass p { out = uv.z; } }", "", "has no `.z`"),
        ("effect x(float a = 1 in 2..1) { pass p { out = a; } }", "", "low end"),
        ("effect x { pass p = glow(q, 1.0); }", "", "not a filter"),
    ];
    for (src, at, says) in cases {
        let e = err(src);
        assert!(e.contains(says), "{src}\n  gave: {e}\n  wanted: {says}");
        if !at.is_empty() {
            assert!(e.starts_with(at), "{src}\n  gave: {e}\n  wanted at {at}");
        }
    }
}

#[test]
fn branches_become_selects_and_variables_follow_them() {
    let e = compile(
        "effect x { scale 1.0; pass p {
            var c = vec3(0.0);
            if uv.x > 0.5 { c = vec3(1.0, 0.0, 0.0); } else { c = vec3(0.0, 0.0, 1.0); }
            out = c;
        } }",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(ops(shade(&e, 0), Op::Select) >= 1, "the branch is a select");
    let px = render(e, 0.0);
    // Left half blue, right half red.
    assert_eq!(px.get(..4), Some(&[0u8, 0, 255, 255][..]));
    let right = (31 * 4) as usize;
    assert_eq!(px.get(right..right + 4), Some(&[255u8, 0, 0, 255][..]));
    assert!(err("effect x { pass p { if uv.x > 0.5 { out = 1.0; } } }").contains("one branch only"));
}

#[test]
fn loops_unroll_and_functions_inline() {
    let e = compile(
        "effect x { scale 1.0;
            fn twice(v: float) -> float { return v * 2.0; }
            pass p { var s = 0.0; for i in 0..4 { s += twice(uv.x) * (i + 1.0); } out = vec3(s * 0.05); } }",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    // Σ (i+1) * 2x = 20x: the loop collapses to arithmetic on uv.x, no calls remain.
    let p = shade(&e, 0);
    assert!(p.code.len() < 30, "{} instructions", p.code.len());
}

#[test]
fn constants_fold_and_frame_values_are_hoisted() {
    let e = compile(
        "effect x(float a = 0.5, float b = 0.25) { scale 1.0;
            pass p { let k = sin(a * b + time) * 3.0 + 2.0 * 4.0; out = vec3(uv.x * k); } }",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    let p = shade(&e, 0);
    // Only the per-pixel multiply remains here; sin(a*b + time) runs once a frame.
    assert_eq!(ops(p, Op::Sin), 0, "hoisted: {:?}", p.code);
    assert_eq!(ops(&e.uniforms, Op::Sin), 1);
    assert!(p.cost() <= 6, "pixel cost {}", p.cost());
}

#[test]
fn whole_powers_become_products_with_the_same_pixels() {
    let src = |k: &str| {
        format!(
            "effect x {{ scale 1.0; pass p {{ let v = uv.x * 2.0 - 1.0;
                out = vec3({k}, {k}, {k}) * 0.5 + 0.5; }} }}"
        )
    };
    for (n, by_hand) in [
        ("2.0", "v * v"),
        ("3.0", "v * v * v"),
        ("4.0", "(v * v) * (v * v)"),
        ("5.0", "(v * v) * (v * v) * v"),
        ("8.0", "((v * v) * (v * v)) * ((v * v) * (v * v))"),
    ] {
        let lowered = compile(&src(&format!("pow(v, {n})"))).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(ops(shade(&lowered, 0), Op::Pow), 0, "pow(v, {n}) is still a pow");
        let hand = compile(&src(by_hand)).unwrap_or_else(|e| unreachable!("{e}"));
        // Odd powers keep the sign (v runs from -1 to 1 across the frame).
        assert_eq!(render(lowered, 0.0), render(hand, 0.0), "pow(v, {n})");
    }
    // Other exponents stay a pow.
    for other in ["pow(abs(v), 2.5)", "pow(v, 9.0)", "pow(v, -2.0)", "pow(abs(v), uv.y)"] {
        let e = compile(&src(other)).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(ops(shade(&e, 0), Op::Pow), 1, "{other}");
    }
}

#[test]
fn a_product_used_once_by_a_sum_fuses_into_one_instruction() {
    let fused = compile("effect x { scale 1.0; pass p { out = vec3(uv.x * uv.y + uv.x); } }")
        .unwrap_or_else(|e| unreachable!("{e}"));
    let p = shade(&fused, 0);
    assert_eq!((ops(p, Op::Fma), ops(p, Op::Mul), ops(p, Op::Add)), (1, 0, 0), "{:?}", p.code);
    // A product used twice is computed once and kept.
    let shared =
        compile("effect x { scale 1.0; pass p { let m = uv.x * uv.y; out = vec3(m + uv.x, m, 0.0); } }")
            .unwrap_or_else(|e| unreachable!("{e}"));
    let p = shade(&shared, 0);
    assert_eq!((ops(p, Op::Fma), ops(p, Op::Mul)), (0, 1), "{:?}", p.code);
    // The same pixels as the two instructions.
    let by_hand = compile("effect x { scale 1.0; pass p { out = vec3(fma(uv.x, uv.y, uv.x)); } }")
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(render(fused, 0.0), render(by_hand, 0.0));
}

#[test]
fn common_work_is_shared() {
    let e = compile("effect x { scale 1.0; pass p { out = vec3(noise(uv * 4.0) + noise(uv * 4.0)); } }")
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(ops(shade(&e, 0), Op::Noise), 1);
}

#[test]
fn parameters_are_typed_ranged_and_clamped() {
    let e = compile(
        "effect x(float k = 2.0 in 0..1, color c = #ff000080, vec2 v = (1.0)) { pass p { out = c * k; } }",
    );
    // `(1.0)` is a float, widened to a vec2 default.
    let e = e.unwrap_or_else(|e| unreachable!("{e}")).shared();
    assert_eq!(e.param("k").map(|(_, p)| p.default[0]), Some(1.0), "defaults are clamped too");
    let mut i = Instance::new(e);
    assert!(i.set("k", Value::Float(f32::NAN)).is_ok());
    assert_eq!(i.get("k"), Some(Value::Float(1.0)), "NaN keeps the default");
    assert!(i.set("k", Value::Color([1.0; 4])).is_err(), "wrong type");
    assert!(i.set("nope", 1.0).is_err());
    assert!(i.set("c", Value::Color([2.0, -1.0, 0.5, 1.0])).is_ok());
    assert_eq!(i.get("c"), Some(Value::Color([1.0, 0.0, 0.5, 1.0])));
}

#[test]
fn matrices_and_3d_helpers_work() {
    let e = compile(
        "effect x { scale 1.0; pass p {
            let m = mat3(vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0));
            let v = m * vec3(1.0, 0.0, 0.0);
            let n = cross(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0));
            out = vec4(v.y, n.z, transpose(m)[0], 1.0);
        } }",
    );
    // Indexing a matrix is not part of the language: a clear error.
    assert!(e.is_err());
    let e = compile(
        "effect x { scale 1.0; pass p {
            let m = mat3(vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0));
            let v = m * vec3(1.0, 0.0, 0.0);
            let n = cross(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0));
            let r = camera_ray(uv, vec3(0.0, 0.0, -3.0), vec3(0.0), 1.0);
            out = vec4(v.y, n.z, step(0.0, r.z), 1.0);
        } }",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    let px = render(e, 0.0);
    assert_eq!(px.get(..4), Some(&[255u8, 255, 255, 255][..]), "M·x = y, x × y = z, rays look forward");
}

#[test]
fn every_library_effect_compiles_for_apps_and_strangers() {
    for (name, src) in remedy_ui_effects::LIBRARY {
        let e = compile(src).unwrap_or_else(|e| unreachable!("{name}: {e}"));
        assert!(e.cost > 0, "{name}");
        // Built-ins hold themselves to the limits people's themes get: anything they do, a
        // theme can do.
        untrusted(src).unwrap_or_else(|e| unreachable!("{name} is beyond what a theme may do: {e}"));
        assert!(remedy_ui_effects::library(name).is_some());
    }
}

// ---- red team ----

fn quick<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t0 = Instant::now();
    let out = f();
    assert!(t0.elapsed() < Duration::from_secs(5), "{what} took {:?}", t0.elapsed());
    out
}

#[test]
fn hostile_sources_fail_fast_with_a_reason() {
    let deep = format!("effect x {{ pass p {{ out = {}1.0{}; }} }}", "(".repeat(5_000), ")".repeat(5_000));
    let nested_loops = "effect x { pass p { var s = 0.0;
        for a in 0..100 { for b in 0..100 { for c in 0..100 { s += noise(uv * a + b + c); } } }
        out = s; } }";
    let huge_loop = "effect x { pass p { var s = 0.0; for i in 0..1000000000 { s += i; } out = s; } }";
    let recursion =
        "effect x { fn a(v: float) -> float { return b(v); } fn b(v: float) -> float { return a(v); }
        pass p { out = a(1.0); } }";
    let self_call = "effect x { fn a(v: float) -> float { return a(v + 1.0); } pass p { out = a(1.0); } }";
    let doubling = {
        // f0(x) = x; f(k)(x) = f(k-1)(x) + f(k-1)(x*1.01): inlining doubles each level.
        let mut s = String::from("effect x { fn f0(v: float) -> float { return noise(vec2(v)); }\n");
        for k in 1..40 {
            s.push_str(&format!(
                "fn f{k}(v: float) -> float {{ return f{}(v) + f{}(v * 1.01); }}\n",
                k - 1,
                k - 1
            ));
        }
        s.push_str("pass p { out = f39(uv.x); } }");
        s
    };
    let wide =
        format!("effect x {{ pass p {{ out = {}; }} }}", vec!["voronoi(uv * 3.0).x"; 3000].join(" + "));
    let params = format!(
        "effect x({}) {{ pass p {{ out = 1.0; }} }}",
        (0..100).map(|i| format!("float p{i}")).collect::<Vec<_>>().join(", ")
    );
    let particles = "effect x { particles s (count: 100000000) { pos = vec2(0.0); } }";
    let passes = format!(
        "effect x {{ {} }}",
        (0..100).map(|i| format!("pass p{i} {{ out = 1.0; }}")).collect::<String>()
    );
    let big = format!("effect x {{ pass p {{ out = 1.0; }} }} {}", " ".repeat(10 << 20));
    let garbage = "\u{0}\u{1}effect ☃ { pass p { out = #zz; } }";
    let unterminated = "effect x { pass p { out = 1.0; } /* never ends";
    let cases: Vec<(&str, String, &str)> = vec![
        ("deep nesting", deep, "nested"),
        ("nested loops", nested_loops.into(), "loop iterations"),
        ("huge loop", huge_loop.into(), "a loop runs"),
        ("mutual recursion", recursion.into(), "deep"),
        ("self recursion", self_call.into(), "deep"),
        ("exponential inlining", doubling, ""),
        ("wide expression", wide, ""),
        ("too many parameters", params, "parameters"),
        ("too many particles", particles.into(), "count"),
        ("too many passes", passes, "passes"),
        ("huge source", big, "larger than"),
        ("garbage", garbage.into(), "unexpected"),
        ("unterminated comment", unterminated.into(), "never ends"),
    ];
    for (what, src, says) in cases {
        let r = quick(what, || Effect::compile_with(&src, &Limits::untrusted()));
        match r {
            Ok(_) => unreachable!("{what} compiled"),
            Err(e) => {
                let e = e.to_string();
                assert!(says.is_empty() || e.contains(says), "{what}: {e}");
            }
        }
    }
}

#[test]
fn a_raymarched_scene_is_too_costly_for_strangers_at_full_resolution() {
    let scene = |scale: &str| {
        format!(
            "effect orb {{ scale {scale};
            fn map(p: vec3) -> float {{ return sd_sphere(p, 1.0) + fbm(p * 2.0, 4) * 0.2; }}
            pass p {{
                let rd = camera_ray(uv, vec3(0.0, 0.0, -3.0), vec3(0.0), 0.9);
                var t = 0.0;
                for i in 0..64 {{ t += map(vec3(0.0, 0.0, -3.0) + rd * t); }}
                out = vec3(exp(-t * 0.2));
            }} }}"
        )
    };
    let full = untrusted(&scene("1.0"));
    assert!(full.as_ref().is_err_and(|e| e.contains("costs")), "{full:?}");
    // Apps may; and nothing about it is unbounded either way.
    assert!(compile(&scene("1.0")).is_ok_and(|e| e.cost > 2500));
}

#[test]
fn hostile_arithmetic_renders_finite_pixels() {
    let e = compile(
        "effect x { scale 1.0; pass p {
            let a = 1.0 / (uv.x - uv.x);
            let b = log(-1.0 - uv.y) + sqrt(-uv.x) + pow(-2.0, 0.5) + tan(1.5707964);
            let c = normalize(vec2(0.0)) + asin(vec2(9.0));
            let big = exp(1000.0 * uv.x) * 1e30 * 1e30;
            out = vec4(a + b, c, big - big);
        } }",
    )
    .unwrap_or_else(|e| unreachable!("{e}"));
    let px = render(e, 0.0);
    assert_eq!(px.len(), 32 * 24 * 4, "every pixel written");
}

#[test]
fn the_same_source_compiles_the_same() {
    for (_, src) in remedy_ui_effects::LIBRARY {
        assert_eq!(compile(src), compile(src));
    }
    // A branch that changes several variables (the merge once followed a hash map's order).
    let src = "effect x { scale 1.0; pass p {
        var a = 0.0; var b = 0.0; var c = 0.0; var d = 0.0; var e = 0.0; var f = 0.0;
        if uv.x < 0.5 { a = 1.0; b = uv.y; c = 2.0; d = uv.x; e = 3.0; f = 4.0; }
        out = vec3(a + b * c, d + e, f); } }";
    let first = compile(src);
    for _ in 0..16 {
        assert_eq!(compile(src), first);
    }
}

#[test]
fn slips_of_the_keyboard_get_a_suggestion() {
    let cases = [
        ("effect x { pass p { out = fmb(uv, 3); } }", "`fmb` is not a function; did you mean `fbm`?"),
        ("effect x { pass p { out = smoothstpe(0.0, 1.0, uv.x); } }", "did you mean `smoothstep`?"),
        ("effect x { pass p { out = sd_circel(uv, 0.3); } }", "did you mean `sd_circle`?"),
        (
            "effect x(float glow = 1.0) { pass p { out = glwo; } }",
            "`glwo` is not defined; did you mean `glow`?",
        ),
        ("effect x { pass p { let shade = 1.0; out = shdae; } }", "did you mean `shade`?"),
        ("effect x { pass p { out = vec4(tiem); } }", "did you mean `time`?"),
        ("effect x { pass base { out = 1.0; } pass b = blur(bsae, 2.0); }", "did you mean `base`?"),
        ("effect x { pass base { out = 1.0; } output bsae; }", "did you mean `base`?"),
        (
            "effect x { fn f(v: flaot) -> float { return v; } pass p { out = f(1.0); } }",
            "did you mean `float`?",
        ),
        (
            "effect x { pass ripple { out = 1.0; } pass p { out = sample(rippel, uv); } }",
            "did you mean `ripple`?",
        ),
    ];
    for (src, says) in cases {
        let e = err(src);
        assert!(e.contains(says), "{src}\n  gave: {e}\n  wanted: {says}");
    }
    // Nothing near enough: no guess.
    let e = err("effect x { pass p { out = wibblewobble(uv); } }");
    assert!(e.contains("is not a function") && !e.contains("did you mean"), "{e}");
}

#[test]
fn reports_show_the_line_and_where_on_it() {
    let src = "effect x {\n    pass p {\n        out = vec4(fmb(uv, 3));\n    }\n}";
    let Err(e) = Effect::compile(src) else { unreachable!("does not compile") };
    let report = e.report(src);
    let want = "error: `fmb` is not a function; did you mean `fbm`?\n --> 3:20\n  |\n3 |         out = vec4(fmb(uv, 3));\n  |                    ^\n";
    assert_eq!(report, want);
}

#[test]
fn over_budget_effects_name_the_passes_that_cost_most() {
    // A cheap pass, then a heavy one: the error points at the heavy pass and names it.
    let src = "effect x {\n scale 1.0;\n pass cheap { out = uv.x; }\n pass heavy { var a = 0.0; for i in 0..40 { a += fbm(uv * float(i + 1), 8); } out = a; }\n}";
    let e = Effect::compile_with(src, &Limits::untrusted()).map_err(|e| e.to_string());
    let Err(e) = e else { unreachable!("over the untrusted budget") };
    assert!(e.starts_with("4:2"), "located at the heavy pass: {e}");
    assert!(e.contains("most of it: `heavy`"), "{e}");
}

#[test]
fn effects_know_whether_they_move() {
    let still = compile("effect s(float k = 0.5) { pass p { out = vec3(noise(uv * 8.0) * k); } }");
    assert!(still.is_ok_and(|e| !e.animated), "no time, no particles, no feedback: a still picture");
    for src in [
        "effect a { pass p { out = vec3(sin(time)); } }",
        "effect b { const k = time * 2.0; pass p { out = vec3(k); } }",
        "effect c { pass p { out = sample(last, uv); } }",
        "effect d { particles s (count: 4) { pos = vec2(1.0); } }",
    ] {
        assert!(compile(src).is_ok_and(|e| e.animated), "{src} moves");
    }
    let still_ones: Vec<&str> = remedy_ui_effects::LIBRARY
        .iter()
        .filter(|(n, _)| remedy_ui_effects::library(n).is_some_and(|e| !e.animated))
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(still_ones, vec!["frost", "paper"], "the built-in effects that do not move");
}
