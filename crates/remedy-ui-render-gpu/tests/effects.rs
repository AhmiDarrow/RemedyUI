//! Prism effects on the GPU kernel match the CPU reference: every built-in effect, at
//! several moments, plus programs written to hit every instruction. Runs on any adapter,
//! the software rung included.

use std::sync::Arc;

use remedy_ui_effects::{CpuEffect, Effect, FrameInput, Instance, LIBRARY};
use remedy_ui_render_gpu::effects::GpuEffect;
use remedy_ui_render_gpu::{FORMAT, GpuContext, LadderOptions, wgpu};

fn ctx() -> GpuContext {
    match GpuContext::new(&LadderOptions::default()) {
        Ok(c) => c,
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    }
}

fn out_texture(ctx: &GpuContext, w: u32, h: u32) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// (pixels differing by more than `tolerance` in any channel, mean absolute difference).
fn compare(a: &[u8], b: &[u8], tolerance: u8) -> (usize, f64) {
    let mut over = 0;
    let mut sum = 0u64;
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        let worst = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        if worst > tolerance {
            over += 1;
        }
        sum += p.iter().zip(q).map(|(x, y)| u64::from(x.abs_diff(*y))).sum::<u64>();
    }
    (over, sum as f64 / a.len().max(1) as f64)
}

fn both(ctx: &GpuContext, effect: Arc<Effect>, frames: &[f32], w: f32, h: f32) -> Vec<(Vec<u8>, Vec<u8>)> {
    let instance = Instance::new(effect.clone());
    let mut cpu = CpuEffect::new();
    let mut gpu = GpuEffect::new(ctx);
    frames
        .iter()
        .map(|&t| {
            let input = FrameInput::new(w, h, 1.0, t);
            let c = cpu.render(&instance, &input);
            let (ow, oh) = GpuEffect::output_size(&effect, &input);
            assert_eq!((ow, oh), (c.width, c.height));
            let out = out_texture(ctx, ow, oh);
            let r = gpu.render(ctx, &instance, &input, &out);
            assert!(r.is_ok(), "{r:?}");
            let g = match ctx.read_back(&out) {
                Ok(f) => f.pixels,
                Err(e) => unreachable!("read back: {e}"),
            };
            (c.pixels, g)
        })
        .collect()
}

#[test]
fn every_library_effect_matches_the_cpu_reference() {
    let ctx = ctx();
    for (name, _) in LIBRARY {
        let Some(effect) = remedy_ui_effects::library(name) else { unreachable!("{name} compiles") };
        let frames: Vec<f32> =
            if effect.feedback { (0..8).map(|k| k as f32 / 30.0).collect() } else { vec![0.0, 1.7, 3.3] };
        for (k, (c, g)) in both(&ctx, effect, &frames, 160.0, 120.0).into_iter().enumerate() {
            let (over, mean) = compare(&c, &g, 24);
            let pixels = c.len() / 4;
            println!("{name} frame {k}: mean difference {mean:.3}, {over} of {pixels} pixels beyond 24");
            assert!(mean < 1.5, "{name} frame {k}: mean difference {mean:.3}");
            assert!(over * 100 <= pixels, "{name} frame {k}: {over} of {pixels} pixels differ visibly");
        }
    }
    assert!(ctx.take_errors().is_empty(), "{:?}", ctx.take_errors());
}

/// One pass per instruction family, each covering every pixel with values that exercise it.
#[test]
fn every_instruction_matches_the_cpu_reference() {
    let ctx = ctx();
    let programs = [
        (
            "arithmetic",
            "vec4(fract(px.x * 0.37) + px.y * 0.01, mod(px.x, 7.0) / 7.0, abs(sin(px.x * 0.3)) * step(0.5, uv.y), 1.0)",
        ),
        (
            "math",
            "vec4(sqrt(uv.x), exp(-uv.y * 3.0), log(1.0 + uv.x) , clamp(pow(uv.y, 2.2) + atan2(uv.y - 0.5, uv.x - 0.5) * 0.1, 0.0, 1.0))",
        ),
        (
            "trig",
            "vec4(acos(uv.x * 2.0 - 1.0) / pi, asin(uv.y * 2.0 - 1.0) / pi + 0.5, tan(uv.x) * 0.5, 1.0)",
        ),
        (
            "logic",
            "vec4(select(0.2, 0.8, uv.x > 0.5 && uv.y < 0.5 || uv.x < 0.1), smoothstep(0.2, 0.8, uv.x), round(uv.y * 4.0) / 4.0, 1.0)",
        ),
        ("vectors", "vec4(normalize(vec3(uv, 1.0)) * 0.5 + vec3(0.5), length(uv - vec2(0.5)))"),
        ("swizzles", "vec4(vec4(uv, uv.yx).wzyx.xyz, 1.0)"),
        ("hash", "vec4(hash3(floor(px / 4.0)), 1.0)"),
        ("noise", "vec4(noise(uv * 8.0), perlin(uv * 6.0), simplex(uv * 5.0), 1.0)"),
        ("noise3", "vec4(noise(vec3(uv * 5.0, 1.5)), perlin(vec3(uv * 4.0, 0.5)), 0.5, 1.0)"),
        ("voronoi", "vec4(voronoi(uv * 6.0), 1.0)"),
        ("fbm", "vec4(fbm(uv * 3.0, 6), ridged(uv * 3.0, 4), turbulence(uv * 3.0, 4), 1.0)"),
        ("camera", "vec4(camera_ray(uv, vec3(0.0, 1.0, -3.0), vec3(0.0), 1.2) * 0.5 + vec3(0.5), 1.0)"),
    ];
    for (name, expr) in programs {
        let src = format!("effect {name} {{ scale 1.0; pass p {{ out = {expr}; }} }}");
        let effect = match Effect::compile(&src) {
            Ok(e) => Arc::new(e),
            Err(e) => unreachable!("{name}: {e}"),
        };
        for (c, g) in both(&ctx, effect, &[0.0], 96.0, 64.0) {
            let (over, mean) = compare(&c, &g, 6);
            println!("{name}: mean difference {mean:.3}, {over} pixels beyond 6");
            assert!(mean < 0.6, "{name}: mean difference {mean:.3}");
            assert!(over * 200 <= c.len() / 4, "{name}: {over} pixels differ");
        }
    }
    assert!(ctx.take_errors().is_empty(), "{:?}", ctx.take_errors());
}

/// A storm of huge particles is capped before it reaches the GPU (a fill-rate attack could
/// otherwise stall it), and finishes quickly.
#[test]
fn huge_particles_cannot_flood_the_gpu() {
    let ctx = ctx();
    let effect = match Effect::compile(
        "effect storm { scale 1.0; particles p (count: 8192, life: 1.0) {
            pos = vec2(rand(1.0), rand(2.0)) * size; radius = 1.0e6; trail = vec2(1.0e6); color = vec4(1.0);
        } }",
    ) {
        Ok(e) => Arc::new(e),
        Err(e) => unreachable!("{e}"),
    };
    let started = std::time::Instant::now();
    let _ = both(&ctx, effect, &[0.5], 400.0, 300.0);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(ctx.take_errors().is_empty());
}

/// A lost device is reported, never a panic; a renderer made on the next device works.
#[test]
fn a_lost_device_is_an_error_not_a_crash() {
    let ctx = ctx();
    let Some(effect) = remedy_ui_effects::library("fire") else { unreachable!() };
    let instance = Instance::new(effect.clone());
    let input = FrameInput::new(64.0, 48.0, 1.0, 0.5);
    let (w, h) = GpuEffect::output_size(&effect, &input);
    let out = out_texture(&ctx, w, h);
    let mut gpu = GpuEffect::new(&ctx);
    assert!(gpu.render(&ctx, &instance, &input, &out).is_ok());
    ctx.force_loss();
    assert_eq!(gpu.render(&ctx, &instance, &input, &out), Err(remedy_ui_render_gpu::GpuError::Lost));
    let fresh = self::ctx();
    let out = out_texture(&fresh, w, h);
    let mut gpu = GpuEffect::new(&fresh);
    assert!(gpu.render(&fresh, &instance, &input, &out).is_ok());
}
