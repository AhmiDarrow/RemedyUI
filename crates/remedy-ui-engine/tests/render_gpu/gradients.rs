//! Gradients on the GPU match the CPU reference: linear at an angle, elliptical radial,
//! conic, and Portal's layered page background built from its CSS through the style crate
//! and the view's brushes, with pixel checks on the GPU's own picture.

use remedy_ui_core::style::Gradient;
use remedy_ui_core::{Color, Point, Rect, Size, Vec2};
use remedy_ui_engine::paint::{Brush, DisplayList, Stop};
use remedy_ui_engine::render_cpu::{CpuRenderer, Frame};
use remedy_ui_engine::render_gpu::*;

fn gpu() -> GpuRenderer {
    match GpuContext::new(&LadderOptions::default()) {
        Ok(ctx) => GpuRenderer::new(ctx, None),
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    }
}

fn ramp() -> Vec<Stop> {
    vec![Stop { offset: 0.0, color: Color::BLACK }, Stop { offset: 1.0, color: Color::WHITE }]
}

/// The largest channel difference between the two pictures.
fn max_diff(a: &Frame, b: &Frame) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.pixels.iter().zip(&b.pixels).map(|(p, q)| p.abs_diff(*q)).max().unwrap_or(0)
}

fn both(g: &mut GpuRenderer, list: &DisplayList, w: u32, h: u32, scale: f32) -> (Frame, Frame) {
    let cpu = CpuRenderer::new().render(list, w, h, scale, Color::WHITE);
    let gpu = g.render(list, w, h, scale, Color::WHITE);
    let (Ok(cpu), Ok(gpu)) = (cpu, gpu) else { unreachable!("both render") };
    (cpu, gpu)
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    f.pixel(x, y).unwrap_or_default()
}

#[test]
fn linear_elliptical_and_conic_gradients_match_the_cpu() {
    let mut g = gpu();
    let mut list = DisplayList::new();
    // A 45° linear, an ellipse four times wider than high, and a conic from straight up.
    list.fill_rect(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Brush::Linear { start: Point::new(0.0, 100.0), end: Point::new(100.0, 0.0), stops: ramp() },
    );
    list.fill_rect(
        Rect::new(100.0, 0.0, 300.0, 100.0),
        Brush::Elliptical { center: Point::new(200.0, 50.0), radii: Vec2::new(80.0, 20.0), stops: ramp() },
    );
    list.fill_rect(
        Rect::new(300.0, 0.0, 400.0, 100.0),
        Brush::Conic {
            center: Point::new(350.0, 50.0),
            start_angle: -std::f32::consts::FRAC_PI_2,
            stops: ramp(),
        },
    );
    for scale in [1.0f32, 2.0] {
        let (w, h) = ((400.0 * scale) as u32, (100.0 * scale) as u32);
        let (cpu, gpu_frame) = both(&mut g, &list, w, h, scale);
        let d = max_diff(&cpu, &gpu_frame);
        eprintln!("{} at {scale}x: max difference {d}", g.context().describe());
        assert!(d <= 3, "gradients differ by {d} at {scale}x on {}", g.context().describe());
        let at = |x: f32, y: f32| px(&gpu_frame, (x * scale) as u32, (y * scale) as u32)[0];
        // Linear: dark bottom left, light top right, mid grey on the diagonal.
        assert!(at(2.0, 97.0) < 15 && at(97.0, 2.0) > 240 && (110..150).contains(&at(50.0, 50.0)));
        // Elliptical: halfway along both axes is the same grey; outside is white.
        assert!(at(240.0, 50.0).abs_diff(at(200.0, 60.0)) <= 8, "{} {}", at(240.0, 50.0), at(200.0, 60.0));
        assert!(at(200.0, 75.0) > 250 && at(285.0, 50.0) > 250);
        // Conic: a quarter turn at the right, half at the bottom.
        assert!((50..80).contains(&at(390.0, 50.0)) && (115..140).contains(&at(350.0, 90.0)));
    }
}

#[test]
fn portals_layered_page_background_matches_the_cpu() {
    let theme = remedy_ui_widgets::themes::portal_theme("portal").unwrap_or_else(|| unreachable!());
    let (w, h) = (1280.0, 800.0);
    let mut list = DisplayList::new();
    let rect = Rect::new(0.0, 0.0, w, h);
    list.fill_rect(rect, Brush::Solid(theme.colors.bg));
    // CSS's first layer is on top: draw the last first.
    for layer in remedy_ui_widgets::themes::portal_page_layers().iter().rev() {
        list.fill_rect(rect, remedy_ui_engine::view::gradient_brush(layer, Size::new(w, h), &theme));
    }
    let gradient = Gradient::parse_layers(remedy_ui_widgets::themes::PORTAL_PAGE_CSS).unwrap_or_default();
    assert_eq!(gradient.len(), 2);
    let mut g = gpu();
    let (cpu, gpu_frame) = both(&mut g, &list, w as u32, h as u32, 1.0);
    let d = max_diff(&cpu, &gpu_frame);
    assert!(d <= 2, "Portal's page differs by {d} on {}", g.context().describe());
    // The gold glow at the top left, the plain background below it.
    let glow = px(&gpu_frame, 4, 4);
    let bg = theme.colors.bg.to_rgba8();
    assert!(glow[0] > bg[0] + 10 && glow[1] > bg[1] + 6, "gold at the top left: {glow:?}");
    assert!(px(&gpu_frame, 640, 700).iter().zip(bg).all(|(a, b)| a.abs_diff(b) <= 1), "the background below");
}
