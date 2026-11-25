//! Progressive zoom on the CPU: while the new scale rasters, the old tiles drawn scaled fill
//! the screen, so a zoom never shows holes, and the finished frame matches direct rendering.

use std::time::Duration;

use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, Rect};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_engine::paint::{Affine, Brush, DisplayList};
use remedy_ui_engine::render_cpu::{CpuCompositor, Frame};

fn checker() -> DisplayList {
    let mut l = DisplayList::new();
    for row in 0..10 {
        for col in 0..10 {
            let c = if (row + col) % 2 == 0 { Color::rgb8(200, 40, 40) } else { Color::rgb8(40, 40, 200) };
            let (x, y) = (col as f32 * 100.0, row as f32 * 100.0);
            l.fill_rect(Rect::new(x, y, x + 100.0, y + 100.0), Brush::Solid(c));
        }
    }
    l
}

fn tree(scale: f32) -> LayerTree {
    let mut l = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 1000.0, 1000.0), checker());
    l.transform = Affine::scale(scale, scale);
    LayerTree::new(vec![l])
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * f.width + x) * 4) as usize;
    f.pixels.get(i..i + 4).and_then(|p| <[u8; 4]>::try_from(p).ok()).unwrap_or([0; 4])
}

#[test]
fn stand_ins_fill_the_screen_until_the_new_scale_is_rastered() {
    let (w, h) = (1200, 900);
    let bg = Color::rgb8(0, 255, 0);
    let mut c = Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    c.commit(tree(1.0));
    let p = c.frame(Duration::ZERO);
    assert!(cpu.present(&p, w, h, bg).is_ok());
    let (pixels, took) = cpu.last_raster();
    assert!(pixels > 0 && took > Duration::ZERO, "the raster cost is measured");
    c.record_raster(pixels, took);
    c.set_zoom_raster_time(Duration::ZERO);
    c.commit(tree(3.0));
    let p = c.frame(Duration::from_millis(16));
    assert!(p.pending > 0, "most of the zoomed screen waits for its tiles");
    let f = cpu.present(&p, w, h, bg);
    assert!(f.is_ok(), "{:?}", f.as_ref().err());
    let Ok(f) = f else { return };
    // No pixel shows the background: the scaled 1x tiles cover what is not rastered yet.
    let green = (0..h).step_by(7).flat_map(|y| (0..w).step_by(7).map(move |x| (x, y)));
    let holes = green.filter(|&(x, y)| px(&f, x, y) == [0, 255, 0, 255]).count();
    assert_eq!(holes, 0, "no holes during the zoom");
    // Square (1, 0) of the checker spans x 300..600 at 3x: blue, scaled or not.
    assert_eq!(px(&f, 350, 50), [40, 40, 200, 255]);
    let mut t = 32;
    let mut last = f;
    loop {
        let p = c.frame(Duration::from_millis(t));
        if let Ok(f) = cpu.present(&p, w, h, bg) {
            last = f;
        }
        if p.pending == 0 && !p.animating {
            break;
        }
        t += 16;
        assert!(t < 5000, "the zoom finishes");
    }
    assert_eq!(px(&last, 350, 50), [40, 40, 200, 255]);
    assert_eq!(px(&last, 50, 50), [200, 40, 40, 255]);
    assert_eq!(px(&last, 299, 299), [200, 40, 40, 255], "crisp edges once rastered at 3x");
    assert_eq!(px(&last, 301, 299), [40, 40, 200, 255]);
}
