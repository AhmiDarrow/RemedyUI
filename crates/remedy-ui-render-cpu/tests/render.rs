//! Pixel tests for the CPU renderer: exact colors where geometry is pixel-aligned, and
//! coverage properties where it is not.

use std::sync::Arc;

use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_paint::{
    Affine, Blend, Brush, DisplayList, GlyphRun, Image, Item, Path, Sampling, Stop, Stroke,
};
use remedy_ui_render_cpu::{CpuRenderer, Frame, RenderError};
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

const RED: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn render(list: &DisplayList, w: u32, h: u32, scale: f32) -> Frame {
    match CpuRenderer::new().render(list, w, h, scale, Color::WHITE) {
        Ok(f) => f,
        Err(e) => unreachable!("render failed: {e}"),
    }
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    f.pixel(x, y).unwrap_or([0, 0, 0, 0])
}

fn close(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| x.abs_diff(*y) <= tol)
}

#[test]
fn aligned_rect_paints_exact_pixels() {
    let mut list = DisplayList::new();
    list.fill_rect(Rect::new(2.0, 2.0, 6.0, 6.0), Brush::Solid(Color::rgb8(255, 0, 0)));
    let f = render(&list, 8, 8, 1.0);
    assert_eq!(f.pixels.len(), 8 * 8 * 4);
    assert_eq!(px(&f, 2, 2), RED);
    assert_eq!(px(&f, 5, 5), RED);
    assert_eq!(px(&f, 1, 1), WHITE);
    assert_eq!(px(&f, 6, 6), WHITE);
    assert_eq!(f.pixel(8, 0), None);
}

#[test]
fn scale_maps_logical_to_device_pixels() {
    let mut list = DisplayList::new();
    list.fill_rect(Rect::new(1.0, 1.0, 2.0, 2.0), Brush::Solid(Color::rgb8(255, 0, 0)));
    let f = render(&list, 8, 8, 2.0);
    for (x, y) in [(2, 2), (3, 3), (2, 3), (3, 2)] {
        assert_eq!(px(&f, x, y), RED, "({x},{y}) is inside the scaled rect");
    }
    assert_eq!(px(&f, 4, 4), WHITE);
    assert_eq!(px(&f, 1, 1), WHITE);
}

#[test]
fn clips_limit_drawing_and_pop() {
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(0.0, 0.0, 4.0, 8.0), CornerRadii::ZERO);
    list.fill_rect(Rect::new(0.0, 0.0, 8.0, 4.0), Brush::Solid(Color::rgb8(255, 0, 0)));
    list.pop_clip();
    list.fill_rect(Rect::new(0.0, 4.0, 8.0, 8.0), Brush::Solid(Color::rgb8(255, 0, 0)));
    let f = render(&list, 8, 8, 1.0);
    assert_eq!(px(&f, 1, 1), RED);
    assert_eq!(px(&f, 6, 1), WHITE, "clipped away");
    assert_eq!(px(&f, 6, 6), RED, "after the clip pops, drawing is unclipped");
}

#[test]
fn rounded_corners_leave_the_corner_pixel_empty() {
    let mut list = DisplayList::new();
    list.fill_rounded(Rect::new(0.0, 0.0, 20.0, 20.0), CornerRadii::all(8.0), Brush::Solid(Color::BLACK));
    let f = render(&list, 20, 20, 1.0);
    assert_eq!(px(&f, 0, 0), WHITE, "outside the rounded corner");
    assert_eq!(px(&f, 10, 10), [0, 0, 0, 255]);
    assert_eq!(px(&f, 10, 0), [0, 0, 0, 255], "the straight edge is filled");
}

#[test]
fn borders_are_rings() {
    let mut list = DisplayList::new();
    list.border(Rect::new(0.0, 0.0, 10.0, 10.0), CornerRadii::ZERO, 2.0, Color::BLACK);
    let f = render(&list, 10, 10, 1.0);
    assert_eq!(px(&f, 0, 5), [0, 0, 0, 255]);
    assert_eq!(px(&f, 1, 5), [0, 0, 0, 255]);
    assert_eq!(px(&f, 5, 5), WHITE, "the inside stays empty");
    assert_eq!(px(&f, 9, 9), [0, 0, 0, 255]);
}

#[test]
fn linear_gradient_runs_between_its_stops() {
    let mut list = DisplayList::new();
    let stops = vec![Stop { offset: 0.0, color: Color::BLACK }, Stop { offset: 1.0, color: Color::WHITE }];
    list.fill_rect(
        Rect::new(0.0, 0.0, 100.0, 4.0),
        Brush::Linear { start: Point::new(0.0, 0.0), end: Point::new(100.0, 0.0), stops },
    );
    let f = render(&list, 100, 4, 1.0);
    let (l, m, r) = (px(&f, 1, 2)[0], px(&f, 50, 2)[0], px(&f, 98, 2)[0]);
    assert!(l < 10 && r > 245, "ends match the stops: {l} {r}");
    assert!(l < m && m < r && (100..160).contains(&m), "the middle is between: {m}");
}

#[test]
fn layers_apply_opacity_to_the_group() {
    let mut list = DisplayList::new();
    list.push(Item::PushLayer { opacity: 0.5, blend: Blend::Normal });
    // Two overlapping opaque rects: with group opacity the overlap is not darker.
    list.fill_rect(Rect::new(0.0, 0.0, 6.0, 8.0), Brush::Solid(Color::BLACK));
    list.fill_rect(Rect::new(2.0, 0.0, 8.0, 8.0), Brush::Solid(Color::BLACK));
    list.push(Item::PopLayer);
    let f = render(&list, 8, 8, 1.0);
    let (a, b) = (px(&f, 1, 4), px(&f, 4, 4));
    assert!(close(a, [128, 128, 128, 255], 2), "half black over white: {a:?}");
    assert_eq!(a, b, "the overlap is the same shade: the group fades as one");
}

#[test]
fn transforms_nest_and_pop() {
    let mut list = DisplayList::new();
    list.push(Item::PushTransform(Affine::translate(Vec2::new(4.0, 0.0))));
    list.push(Item::PushTransform(Affine::translate(Vec2::new(0.0, 4.0))));
    list.fill_rect(Rect::new(0.0, 0.0, 2.0, 2.0), Brush::Solid(Color::rgb8(255, 0, 0)));
    list.push(Item::PopTransform);
    list.push(Item::PopTransform);
    list.fill_rect(Rect::new(0.0, 0.0, 2.0, 2.0), Brush::Solid(Color::rgb8(255, 0, 0)));
    let f = render(&list, 8, 8, 1.0);
    assert_eq!(px(&f, 4, 4), RED, "both translations apply");
    assert_eq!(px(&f, 0, 0), RED, "and both pop");
    assert_eq!(px(&f, 4, 0), WHITE);
}

#[test]
fn paths_fill_and_stroke() {
    let mut list = DisplayList::new();
    list.push(Item::FillPath {
        path: Path::circle(Point::new(10.0, 10.0), 6.0),
        brush: Brush::Solid(Color::BLACK),
        even_odd: false,
    });
    let mut line = Path::new();
    line.move_to(Point::new(0.0, 25.0)).line_to(Point::new(20.0, 25.0));
    list.push(Item::StrokePath { path: line, stroke: Stroke::new(2.0), brush: Brush::Solid(Color::BLACK) });
    let f = render(&list, 20, 30, 1.0);
    assert_eq!(px(&f, 10, 10), [0, 0, 0, 255], "the circle's center");
    assert_eq!(px(&f, 1, 1), WHITE, "outside the circle");
    assert_eq!(px(&f, 10, 24), [0, 0, 0, 255], "the stroke covers its width");
    assert_eq!(px(&f, 10, 27), WHITE);
}

#[test]
fn shadows_blur_past_their_shape() {
    let mut list = DisplayList::new();
    list.push(Item::Shadow {
        rect: Rect::new(10.0, 10.0, 30.0, 30.0),
        radii: CornerRadii::ZERO,
        offset: Vec2::new(0.0, 0.0),
        blur: 8.0,
        spread: 0.0,
        color: Color::BLACK,
    });
    let f = render(&list, 40, 40, 1.0);
    let center = px(&f, 20, 20)[0];
    let edge = px(&f, 8, 20)[0];
    let far = px(&f, 0, 20)[0];
    assert!(center < 20, "the middle is dark: {center}");
    assert!(edge > center && edge < 250, "it fades past the edge: {edge}");
    assert!(far > edge, "and keeps fading: {far}");
}

#[test]
fn images_scale_into_their_rect() {
    // A 2x1 image: red then blue, drawn 8x4 with nearest sampling.
    let pixels: Arc<[u8]> = Arc::from(vec![255, 0, 0, 255, 0, 0, 255, 255]);
    let image = Image { id: 7, width: 2, height: 1, pixels };
    let mut list = DisplayList::new();
    list.push(Item::Image {
        image,
        rect: Rect::new(0.0, 0.0, 8.0, 4.0),
        radii: CornerRadii::ZERO,
        sampling: Sampling::Nearest,
    });
    let f = render(&list, 8, 4, 1.0);
    assert_eq!(px(&f, 1, 2), RED);
    assert_eq!(px(&f, 6, 2), [0, 0, 255, 255]);
}

fn text_list(text: &str, color: Color) -> Option<DisplayList> {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        fonts.add_static(face.data).ok()?;
    }
    let mut engine = TextEngine::new(fonts);
    let p = engine.layout(&RichText::plain(text), &TextStyle::default(), ParagraphOptions::default());
    let mut list = DisplayList::new();
    for run in p.glyph_runs() {
        list.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x, g.y))).collect(),
            hinting: Hinting::Full,
            antialias: Antialias::Grayscale,
        }));
    }
    Some(list)
}

fn ink(f: &Frame) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..f.height {
        for x in 0..f.width {
            if px(f, x, y)[0] < 128 {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn text_draws_in_its_color_and_scales() {
    let Some(list) = text_list("Hi", Color::BLACK) else { unreachable!("fonts load") };
    let f1 = render(&list, 60, 40, 1.0);
    let ink1 = ink(&f1);
    assert!(!ink1.is_empty(), "text puts ink down");
    assert!(ink1.iter().all(|&(_, y)| y < 30), "glyphs sit near their line");
    let f2 = render(&list, 120, 80, 2.0);
    let ink2 = ink(&f2);
    assert!(ink2.len() > ink1.len() * 3, "2x scale rasterizes at 2x: {} vs {}", ink2.len(), ink1.len());

    let Some(red) = text_list("Hi", Color::rgb8(255, 0, 0)) else { unreachable!("fonts load") };
    let f = render(&red, 60, 40, 1.0);
    let darkest = f.pixels.as_chunks::<4>().0.iter().min_by_key(|p| p[1]);
    assert_eq!(darkest.map(|&[r, g, b, _]| (r, g, b)), Some((255, 0, 0)), "red text stays red");
}

#[test]
fn text_respects_clips() {
    let Some(text) = text_list("HHHH", Color::BLACK) else { unreachable!("fonts load") };
    let mut list = DisplayList::new();
    list.push_clip(Rect::new(0.0, 0.0, 10.0, 40.0), CornerRadii::ZERO);
    list.extend(&text);
    list.pop_clip();
    let f = render(&list, 80, 40, 1.0);
    let ink = ink(&f);
    assert!(!ink.is_empty());
    assert!(ink.iter().all(|&(x, _)| x < 10), "no ink past the clip");
}

#[test]
fn caches_survive_frames_and_render_identically() {
    let Some(list) = text_list("Remedy", Color::BLACK) else { unreachable!("fonts load") };
    let mut r = CpuRenderer::new();
    let a = r.render(&list, 100, 40, 1.0, Color::WHITE);
    let b = r.render(&list, 100, 40, 1.0, Color::WHITE);
    assert!(a.is_ok());
    assert_eq!(a, b, "cached glyphs draw the same as fresh ones");
    r.clear_caches();
    assert_eq!(r.render(&list, 100, 40, 1.0, Color::WHITE), b);
}

#[test]
fn bad_sizes_are_errors() {
    let list = DisplayList::new();
    let mut r = CpuRenderer::new();
    assert_eq!(r.render(&list, 0, 10, 1.0, Color::WHITE), Err(RenderError::BadSize));
    assert_eq!(r.render(&list, 70_000, 10, 1.0, Color::WHITE), Err(RenderError::BadSize));
}

#[test]
fn images_the_app_drops_leave_the_cache() {
    let mut r = CpuRenderer::new();
    let image = |id: u64| Image { id, width: 2, height: 2, pixels: Arc::from(vec![255u8; 16]) };
    let (a, b) = (image(1), image(2));
    let mut list = DisplayList::new();
    for (i, im) in [&a, &b].into_iter().enumerate() {
        list.push(Item::Image {
            image: im.clone(),
            rect: Rect::new(i as f32 * 4.0, 0.0, i as f32 * 4.0 + 4.0, 4.0),
            radii: CornerRadii::ZERO,
            sampling: Sampling::Linear,
        });
    }
    let _ = r.render(&list, 8, 4, 1.0, Color::WHITE);
    assert_eq!(r.cached_images(), 2);
    // The list and one image go away; the other is still held by the app.
    drop(list);
    drop(a);
    let _ = r.render(&DisplayList::new(), 8, 4, 1.0, Color::WHITE);
    assert_eq!(r.cached_images(), 1, "the dropped image is freed, the held one kept");
    drop(b);
    let _ = r.render(&DisplayList::new(), 8, 4, 1.0, Color::WHITE);
    assert_eq!(r.cached_images(), 0);
}
