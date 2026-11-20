//! Red team: hostile effects, inputs and sources against the compiler and the CPU renderer.
//! Nothing may panic, hang, allocate without bound or take long; everything bounded by the
//! limits and by `fit`.
#![allow(clippy::indexing_slicing, reason = "test fixtures index sizes they just made")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_effects::{CpuEffect, Effect, FrameInput, Instance, LIBRARY, Limits, MAX_EFFECT_PIXELS};

fn effect(src: &str) -> Arc<Effect> {
    match Effect::compile(src) {
        Ok(e) => Arc::new(e),
        Err(e) => unreachable!("{e}\n{src}"),
    }
}

fn timed<T>(what: &str, limit: Duration, f: impl FnOnce() -> T) -> T {
    let t0 = Instant::now();
    let out = f();
    assert!(t0.elapsed() < limit, "{what} took {:?}", t0.elapsed());
    out
}

#[test]
fn a_huge_element_cannot_allocate_without_bound() {
    // Ten full-resolution passes over a 100,000 × 100,000 element at scale 8.
    let mut src = String::from("effect big { scale 1.0; pass p0 { out = 1.0; }");
    for i in 1..10 {
        src.push_str(&format!(" pass p{i} {{ out = sample(p{}, uv); }}", i - 1));
    }
    src.push_str(" }");
    let e = effect(&src);
    let fitted = remedy_ui_effects::fit(&e, &FrameInput::new(100_000.0, 100_000.0, 8.0, 0.0));
    let total: u64 = e
        .stages
        .iter()
        .map(|s| {
            let (w, h) = remedy_ui_effects::buffer_size(&e, s, &fitted);
            u64::from(w) * u64::from(h)
        })
        .sum();
    assert!(total <= MAX_EFFECT_PIXELS, "{total} pixels");
    // And a merely large one renders, bounded.
    let pic = timed("large render", Duration::from_secs(20), || {
        CpuEffect::new().render(&Instance::new(e.clone()), &FrameInput::new(20_000.0, 20_000.0, 2.0, 0.0))
    });
    assert!(u64::from(pic.width) * u64::from(pic.height) <= MAX_EFFECT_PIXELS);
}

#[test]
fn hostile_frame_inputs_render_something_small() {
    let e = effect("effect x { pass p { out = vec4(uv, time, 1.0); } }");
    for input in [
        FrameInput::new(f32::NAN, 100.0, 1.0, 0.0),
        FrameInput::new(100.0, f32::INFINITY, 1.0, 0.0),
        FrameInput::new(100.0, 100.0, 0.0, 0.0),
        FrameInput::new(100.0, 100.0, f32::NAN, f32::NAN),
        FrameInput::new(-5.0, -5.0, -1.0, f32::NEG_INFINITY),
        FrameInput::new(100.0, 100.0, 1.0, 1.0e30),
    ] {
        let pic = CpuEffect::new().render(&Instance::new(e.clone()), &input);
        assert!(
            pic.width >= 1 && pic.height >= 1 && pic.width <= 4096,
            "{input:?}: {}x{}",
            pic.width,
            pic.height
        );
        assert_eq!(pic.pixels.len(), (pic.width * pic.height * 4) as usize);
    }
}

#[test]
fn huge_particles_are_capped_by_overdraw() {
    let e = effect(
        "effect storm { scale 1.0; particles p (count: 8192, life: 1.0, splat: soft) {
            pos = vec2(rand(1.0), rand(2.0)) * size; radius = 1.0e6; trail = vec2(1.0e6, 1.0e6); color = vec4(1.0);
        } }",
    );
    timed("8192 huge particles", Duration::from_secs(5), || {
        CpuEffect::new()
            .with_threads(1)
            .render(&Instance::new(e.clone()), &FrameInput::new(400.0, 300.0, 1.0, 0.5))
    });
}

#[test]
fn malformed_textures_are_ignored() {
    let e = effect("effect t(texture a) { scale 1.0; pass p { out = sample(a, uv * 3.0 - vec2(1.0)); } }");
    let mut i = Instance::new(e);
    let bad = remedy_ui_paint::Image::new(64, 64, vec![255u8; 10].into());
    assert!(i.set_texture("a", Some(bad)).is_ok());
    let empty = remedy_ui_paint::Image::new(0, 0, Vec::new().into());
    let pic = CpuEffect::new().render(&i, &FrameInput::new(32.0, 32.0, 1.0, 0.0));
    assert!(pic.pixels.iter().all(|b| *b == 0), "a picture whose size lies reads as nothing");
    assert!(i.set_texture("a", Some(empty)).is_ok());
    let _ = CpuEffect::new().render(&i, &FrameInput::new(32.0, 32.0, 1.0, 0.0));
}

/// Many values alive at once: registers are reused only when a value is dead. The result is
/// checked against the same sum computed directly.
#[test]
fn register_reuse_under_pressure_keeps_every_value() {
    let n = 40;
    let mut src = String::from("effect pressure { scale 1.0; pass p {\n");
    for i in 0..n {
        src.push_str(&format!("let a{i} = hash(vec2({i}.0, floor(px.x)));\n"));
    }
    // Use them all at the end, in reverse, so they are all alive together.
    let sum: Vec<String> = (0..n).rev().map(|i| format!("a{i} * {}.0", i + 1)).collect();
    src.push_str(&format!("out = vec4(vec3(({}) / 820.0), 1.0);\n}} }}", sum.join(" + ")));
    let e = effect(&src);
    let pic = CpuEffect::new().render(&Instance::new(e), &FrameInput::new(16.0, 1.0, 1.0, 0.0));
    for x in 0..16u32 {
        let expect: f32 = (0..n)
            .map(|i| remedy_ui_effects::vm::hash4([i as f32, x as f32, 0.0, 0.0], 2)[0] * (i + 1) as f32)
            .sum::<f32>()
            / 820.0;
        let got = f32::from(pic.pixels[(x * 4) as usize]) / 255.0;
        assert!((got - expect.clamp(0.0, 1.0)).abs() <= 1.0 / 255.0 + 1e-4, "pixel {x}: {got} vs {expect}");
    }
}

/// A small, deterministic random source for the fuzzer.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Mutates the library's sources (deleting, duplicating and swapping pieces, inserting
/// tokens) and compiles each with untrusted limits: never a panic, never slow; whatever
/// compiles renders a small frame without a panic either.
#[test]
fn mutated_sources_never_panic_or_hang() {
    let tokens = [
        "(",
        ")",
        "{",
        "}",
        ";",
        ",",
        "=",
        "+",
        "*",
        "/",
        "-",
        ".",
        "..",
        "for",
        "if",
        "else",
        "let",
        "var",
        "fn",
        "pass",
        "out",
        "uv",
        "time",
        "1e9",
        "0",
        "-1",
        "vec4(",
        "sample(",
        "last",
        "#fff",
        "fbm(uv,8)",
        "for i in 0..128 {",
        "particles q (count: 2048) { pos = vec2(0.0); }",
        "\u{0}",
        "é",
        "//",
        "/*",
    ];
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let started = Instant::now();
    let mut compiled = 0;
    for round in 0..1500 {
        let (_, base) = LIBRARY[rng.below(LIBRARY.len())];
        let mut chars: Vec<char> = base.chars().collect();
        for _ in 0..(1 + rng.below(4)) {
            let len = chars.len().max(1);
            match rng.below(4) {
                0 => {
                    let a = rng.below(len);
                    let b = (a + rng.below(40)).min(chars.len());
                    chars.drain(a..b);
                }
                1 => {
                    let a = rng.below(len);
                    let b = (a + rng.below(60)).min(chars.len());
                    let piece: Vec<char> = chars[a..b].to_vec();
                    let at = rng.below(chars.len().max(1));
                    chars.splice(at..at, piece);
                }
                2 => {
                    let t = tokens[rng.below(tokens.len())];
                    let at = rng.below(chars.len().max(1));
                    chars.splice(at..at, t.chars());
                }
                _ => {
                    let a = rng.below(len);
                    let b = rng.below(len);
                    if a < chars.len() && b < chars.len() {
                        chars.swap(a, b);
                    }
                }
            }
        }
        let src: String = chars.into_iter().collect();
        let t0 = Instant::now();
        let r = Effect::compile_with(&src, &Limits::untrusted());
        assert!(t0.elapsed() < Duration::from_secs(2), "round {round} compiled in {:?}", t0.elapsed());
        if let Ok(e) = r {
            compiled += 1;
            let t0 = Instant::now();
            let _ = CpuEffect::new()
                .with_threads(1)
                .render(&Instance::new(Arc::new(e)), &FrameInput::new(24.0, 16.0, 1.0, 0.7));
            assert!(t0.elapsed() < Duration::from_secs(2), "round {round} rendered in {:?}", t0.elapsed());
        }
    }
    println!("{compiled} of 1500 mutants compiled; {:?}", started.elapsed());
    assert!(compiled > 0, "some mutants still compile");
}
