//! A raised or lowered [`PaintLimits`] draws the same on the GPU as on the CPU. One test, so
//! no other test in this binary sees the bounds changed.

use remedy_ui_core::{Color, ColorMatrix, CornerRadii, Rect};
use remedy_ui_engine::paint::{Brush, DisplayList, Filter, PaintLimits, set_paint_limits};
use remedy_ui_engine::render_cpu::{CpuRenderer, Frame};
use remedy_ui_engine::render_gpu::{GpuContext, GpuRenderer, LadderOptions};

fn both(gpu: &mut GpuRenderer, list: &DisplayList) -> (Frame, Frame) {
    match (
        CpuRenderer::new().render(list, 16, 16, 1.0, Color::WHITE),
        gpu.render(list, 16, 16, 1.0, Color::WHITE),
    ) {
        (Ok(c), Ok(g)) => (c, g),
        _ => unreachable!("both render"),
    }
}

fn at(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    f.pixel(x, y).unwrap_or_default()
}

#[test]
fn raised_paint_limits_draw_alike_on_both_renderers() {
    let mut gpu = match GpuContext::new(&LadderOptions::default()) {
        Ok(ctx) => GpuRenderer::new(ctx, None),
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    };
    // 300 clips: the outer ones cover everything, the innermost only the top-left quarter.
    let mut deep = DisplayList::new();
    for _ in 0..299 {
        deep.push_clip(Rect::new(0.0, 0.0, 16.0, 16.0), CornerRadii::ZERO);
    }
    deep.push_clip(Rect::new(0.0, 0.0, 8.0, 8.0), CornerRadii::ZERO);
    deep.fill_rect(Rect::new(0.0, 0.0, 16.0, 16.0), Brush::Solid(Color::BLACK));
    for _ in 0..300 {
        deep.pop_clip();
    }
    let mut inverted = DisplayList::new();
    inverted.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::invert(1.0) });
    inverted.fill_rect(Rect::new(0.0, 0.0, 16.0, 16.0), Brush::Solid(Color::BLACK));
    inverted.pop_filter();

    let black = [0, 0, 0, 255];
    let white = [255, 255, 255, 255];
    let (c, g) = both(&mut gpu, &deep);
    assert_eq!((at(&c, 12, 12), at(&g, 12, 12)), (black, black), "past 256 the innermost clip opens nothing");
    set_paint_limits(PaintLimits::default().group_depth(512).filter_matrix_depth(0));
    let (c, g) = both(&mut gpu, &deep);
    assert_eq!((at(&c, 12, 12), at(&g, 12, 12)), (white, white), "the innermost clip is drawn");
    assert_eq!((at(&c, 2, 2), at(&g, 2, 2)), (black, black));
    let (c, g) = both(&mut gpu, &inverted);
    assert_eq!((at(&c, 4, 4), at(&g, 4, 4)), (black, black), "no matrix group drawn through");
    set_paint_limits(PaintLimits::default());
    let (c, g) = both(&mut gpu, &inverted);
    assert_eq!((at(&c, 4, 4), at(&g, 4, 4)), (white, white));
}
