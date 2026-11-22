//! The bounds renderers draw within ([`set_paint_limits`]) change what is drawn. One test, so
//! no other test in this binary sees the bounds changed.

use remedy_ui_core::{Color, ColorMatrix, CornerRadii, Rect};
use remedy_ui_paint::{Brush, DisplayList, Filter, PaintLimits, set_paint_limits};
use remedy_ui_render_cpu::CpuRenderer;

fn probe(list: &DisplayList) -> [u8; 4] {
    match CpuRenderer::new().render(list, 8, 8, 1.0, Color::WHITE) {
        Ok(f) => f.pixel(6, 6).unwrap_or_default(),
        Err(e) => unreachable!("render failed: {e}"),
    }
}

#[test]
fn raised_and_lowered_paint_limits_are_drawn() {
    // 300 clips: the outer ones cover everything, the innermost only the top-left corner.
    let mut deep = DisplayList::new();
    for _ in 0..299 {
        deep.push_clip(Rect::new(0.0, 0.0, 8.0, 8.0), CornerRadii::ZERO);
    }
    deep.push_clip(Rect::new(0.0, 0.0, 4.0, 4.0), CornerRadii::ZERO);
    deep.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), Brush::Solid(Color::BLACK));
    for _ in 0..300 {
        deep.pop_clip();
    }
    // Past the default 256 the innermost clip opens nothing: (6, 6) is black.
    assert_eq!(probe(&deep), [0, 0, 0, 255]);
    set_paint_limits(PaintLimits::default().group_depth(512));
    assert_eq!(probe(&deep), [255, 255, 255, 255], "the innermost clip is drawn");

    // An inverting colour matrix: drawn by default, not with no matrix depth at all.
    let mut inverted = DisplayList::new();
    inverted.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::invert(1.0) });
    inverted.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), Brush::Solid(Color::BLACK));
    inverted.pop_filter();
    assert_eq!(probe(&inverted), [255, 255, 255, 255]);
    set_paint_limits(PaintLimits::default().filter_matrix_depth(0));
    assert_eq!(probe(&inverted), [0, 0, 0, 255], "no matrix group drawn through");

    set_paint_limits(PaintLimits::default());
    assert_eq!(probe(&deep), [0, 0, 0, 255]);
}
