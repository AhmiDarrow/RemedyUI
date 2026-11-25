//! Images drawn scaled down: drawn from mipmap levels, so they do not shimmer, alike on both
//! renderers, and only the level drawn is uploaded.

use std::sync::Arc;

use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_engine::paint::{DisplayList, Image, Item, Sampling};
use remedy_ui_engine::render_cpu::{CpuRenderer, Frame};
use remedy_ui_engine::render_gpu::*;

fn gpu() -> GpuRenderer {
    match GpuContext::new(&LadderOptions::default()) {
        Ok(ctx) => GpuRenderer::new(ctx, None),
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    }
}

/// A black and white checkerboard of single pixels: the worst case for a scaled-down draw.
fn checkerboard(n: u32) -> Image {
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            px.extend([v, v, v, 255]);
        }
    }
    Image::new(n, n, Arc::from(px))
}

fn drawn(image: &Image, size: f32, sampling: Sampling) -> DisplayList {
    let mut l = DisplayList::new();
    l.push(Item::Image {
        image: image.clone(),
        rect: Rect::new(0.0, 0.0, size, size),
        radii: CornerRadii::ZERO,
        sampling,
    });
    l
}

/// The farthest any pixel inside `n` × `n` is from mid grey.
fn off_grey(f: &Frame, n: u32) -> u8 {
    let mut worst = 0;
    for y in 1..n - 1 {
        for x in 1..n - 1 {
            let p = f.pixel(x, y).unwrap_or([0; 4]);
            worst = worst.max(p[0].abs_diff(128));
        }
    }
    worst
}

#[test]
fn images_drawn_small_average_their_pixels_on_both_renderers() {
    let image = checkerboard(256);
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    // An eighth of the size, a fifth (not a power of two), and through a scale transform.
    for (size, scale) in [(32.0f32, 1.0f32), (51.0, 1.0), (16.0, 2.0)] {
        let list = drawn(&image, size, Sampling::Linear);
        let n = (size * scale) as u32;
        let (Ok(a), Ok(b)) =
            (c.render(&list, n, n, scale, Color::BLACK), g.render(&list, n, n, scale, Color::BLACK))
        else {
            unreachable!("both render")
        };
        assert!(
            off_grey(&a, n) <= 8,
            "{size}@{scale}: the CPU shows grey, not shimmer ({})",
            off_grey(&a, n)
        );
        assert!(
            off_grey(&b, n) <= 8,
            "{size}@{scale}: the GPU shows grey, not shimmer ({})",
            off_grey(&b, n)
        );
        let worst = a.pixels.iter().zip(&b.pixels).map(|(p, q)| p.abs_diff(*q)).max().unwrap_or(0);
        assert!(worst <= 4, "{size}@{scale}: the renderers agree (worst {worst})");
    }
    assert!(g.context().take_errors().is_empty());
}

#[test]
fn pixel_art_and_images_drawn_large_are_drawn_from_their_own_pixels() {
    let image = checkerboard(64);
    let mut c = CpuRenderer::new();
    // Nearest sampling keeps hard pixels at any size: no level averages them away.
    let Ok(f) = c.render(&drawn(&image, 8.0, Sampling::Nearest), 8, 8, 1.0, Color::BLACK) else {
        unreachable!("renders")
    };
    assert!(f.pixels.chunks(4).all(|p| matches!(p.first(), Some(0 | 255))), "hard pixels");
    // At full size the image itself is drawn, exactly.
    let Ok(f) = c.render(&drawn(&image, 64.0, Sampling::Linear), 64, 64, 1.0, Color::BLACK) else {
        unreachable!("renders")
    };
    assert_eq!(f.pixels, image.pixels.to_vec());
}

#[test]
fn only_the_level_drawn_is_uploaded_and_it_goes_with_its_image() {
    let mut g = gpu();
    let before = g.image_bytes();
    let image = checkerboard(1024);
    // Drawn at a sixteenth: a 64 x 64 level, not the 4 MB image.
    let Ok(_) = g.render(&drawn(&image, 64.0, Sampling::Linear), 64, 64, 1.0, Color::BLACK) else {
        unreachable!("renders")
    };
    let used = g.image_bytes() - before;
    assert!(used < 1024 * 1024, "a small level uploaded, not the full image: {used} bytes");
    // The app drops the image: its levels are freed over the next draws.
    drop(image);
    for _ in 0..2 {
        let _ = g.render(&DisplayList::new(), 8, 8, 1.0, Color::BLACK);
    }
    assert!(g.image_bytes() <= before + 4096, "freed: {} vs {before}", g.image_bytes());
    let mut c = CpuRenderer::new();
    let image = checkerboard(256);
    let _ = c.render(&drawn(&image, 32.0, Sampling::Linear), 32, 32, 1.0, Color::BLACK);
    assert_eq!(c.cached_images(), 1, "the level drawn, not the image too");
    drop(image);
    for _ in 0..2 {
        let _ = c.render(&DisplayList::new(), 8, 8, 1.0, Color::BLACK);
    }
    assert_eq!(c.cached_images(), 0);
}
