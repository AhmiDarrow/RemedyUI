//! Golden images: every built-in effect rendered on the CPU (the reference renderer) at a
//! fixed moment, compared with `tests/golden/<name>.png`. After a deliberate change to an
//! effect or the kernels, rerun with `PRISM_BLESS=1`, look at the new images and commit them.
//!
//! The comparison allows a level or two of difference: the CPU's sine and exponential come
//! from each platform's maths library, which may differ in the last bits.

use std::path::PathBuf;

use remedy_ui_effects::{CpuEffect, FrameInput, Instance, LIBRARY, library};

const W: f32 = 128.0;
const H: f32 = 80.0;
const AT: f32 = 1.25;

/// A test picture for texture parameters: stripes, a gradient and a disc.
fn picture() -> remedy_ui_paint::Image {
    let (w, h) = (128u32, 80u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let stripe = if (x / 8 + y / 8) % 2 == 0 { 220u8 } else { 60 };
            let (dx, dy) = (x as f32 - 64.0, y as f32 - 40.0);
            let disc = (dx * dx + dy * dy).sqrt() < 24.0;
            px.extend(if disc { [255, 190, 60, 255] } else { [stripe, (x * 2) as u8, (y * 3) as u8, 255] });
        }
    }
    remedy_ui_paint::Image::new(w, h, px.into())
}

fn render(name: &str, threads: usize) -> (u32, u32, Vec<u8>) {
    let Some(effect) = library(name) else { unreachable!("{name} compiles") };
    let mut instance = Instance::new(effect.clone());
    for t in effect.textures.clone() {
        let _ = instance.set_texture(&t, Some(picture()));
    }
    let mut cpu = CpuEffect::new().with_threads(threads);
    // Effects that read their last frame build up over a few frames first.
    let frames = if effect.feedback { 12 } else { 1 };
    let mut draw = |at: f32| {
        let mut pic = cpu.render(&instance, &FrameInput::new(W, H, 1.0, at));
        for k in 1..frames {
            pic = cpu.render(&instance, &FrameInput::new(W, H, 1.0, at + k as f32 / 30.0));
        }
        pic
    };
    let mut pic = draw(AT);
    // An effect that shows nothing at AT (lightning between strikes) is drawn at its still
    // moment, which shows it.
    if blank(&pic.pixels) {
        pic = draw(effect.still);
    }
    assert!(!blank(&pic.pixels), "{name} draws nothing at {AT} s or at its still moment");
    (pic.width, pic.height, pic.pixels)
}

/// Whether every pixel is the same.
fn blank(px: &[u8]) -> bool {
    px.chunks_exact(4).all(|p| px.get(..4) == Some(p))
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.png"))
}

fn write_png(path: &PathBuf, w: u32, h: u32, px: &[u8]) {
    let file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => unreachable!("{}: {e}", path.display()),
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    let written = enc.write_header().and_then(|mut wr| wr.write_image_data(px));
    assert!(written.is_ok(), "{written:?}");
}

fn read_png(path: &PathBuf) -> Option<(u32, u32, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

#[test]
fn every_effect_matches_its_golden_image() {
    let bless = std::env::var_os("PRISM_BLESS").is_some();
    if bless {
        let _ = std::fs::create_dir_all(golden_path("x").parent().unwrap_or(std::path::Path::new(".")));
    }
    let mut missing = Vec::new();
    for (name, _) in LIBRARY {
        let (w, h, px) = render(name, 1);
        let path = golden_path(name);
        if bless {
            // Goldens are stored premultiplied, exactly as rendered.
            write_png(&path, w, h, &px);
            continue;
        }
        let Some((gw, gh, gold)) = read_png(&path) else {
            missing.push(*name);
            continue;
        };
        assert_eq!((gw, gh), (w, h), "{name}: size");
        let mut off = 0usize;
        let mut worst = 0u8;
        for (a, b) in px.chunks(4).zip(gold.chunks(4)) {
            let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            worst = worst.max(d);
            if d > 2 {
                off += 1;
            }
        }
        assert!(
            off * 200 <= px.len() / 4,
            "{name}: {off} pixels differ from the golden image (worst {worst}); rerun with PRISM_BLESS=1 if this was deliberate"
        );
    }
    assert!(missing.is_empty(), "no golden image for {missing:?}: run with PRISM_BLESS=1");
}

#[test]
fn threads_change_nothing() {
    for name in ["starfield", "water", "nebula", "fire"] {
        assert_eq!(render(name, 1), render(name, 4), "{name}");
    }
}
