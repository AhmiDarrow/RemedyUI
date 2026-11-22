//! Text under a turned, skewed or unevenly stretched transform: its glyphs turn with it, on
//! the CPU renderer (the GPU renderer draws through the same painter; see its tests). Text
//! drawn upright stays exactly as it was.

use remedy_ui_core::{Color, Point, Vec2};
use remedy_ui_paint::{Affine, DisplayList, GlyphRun, Item};
use remedy_ui_render_cpu::{CpuRenderer, Frame};
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

/// "HHHHHHHH" at 20 px from (0, 0) on its baseline, under `t`.
pub fn text(t: Affine, hinting: Hinting, aa: Antialias) -> DisplayList {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let mut style = TextStyle::default();
    style.size = 20.0;
    let p = engine.layout(&RichText::plain("HHHHHHHH"), &style, ParagraphOptions::default());
    let mut list = DisplayList::new();
    list.push(Item::PushTransform(t));
    for run in p.glyph_runs() {
        list.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color: Color::BLACK,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x, g.y))).collect(),
            hinting,
            antialias: aa,
        }));
    }
    list.push(Item::PopTransform);
    list
}

/// A turn by `deg` degrees (clockwise on screen) about the point `at`... applied after the
/// text's own origin is moved there.
fn turned(deg: f32, at: (f32, f32)) -> Affine {
    Affine::rotate(deg.to_radians()).then(Affine::translate(Vec2::new(at.0, at.1)))
}

fn render(list: &DisplayList, w: u32, h: u32, scale: f32) -> Frame {
    CpuRenderer::new().render(list, w, h, scale, Color::WHITE).unwrap_or_else(|e| unreachable!("{e}"))
}

/// The ink's direction: the angle of its long axis in degrees (0 across, 90 down), and how
/// much longer than wide it is.
fn ink_axis(f: &Frame) -> (f32, f32) {
    let mut pts = Vec::new();
    for y in 0..f.height {
        for x in 0..f.width {
            if f.pixel(x, y).is_some_and(|p| p[0] < 128) {
                pts.push((x as f32, y as f32));
            }
        }
    }
    let n = pts.len() as f32;
    assert!(n > 50.0, "ink");
    let (mx, my) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
    let (sxx, syy, sxy) = pts.iter().fold((0.0, 0.0, 0.0), |a, p| {
        let (dx, dy) = (p.0 - mx, p.1 - my);
        (a.0 + dx * dx, a.1 + dy * dy, a.2 + dx * dy)
    });
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let tr = sxx + syy;
    let det = sxx * syy - sxy * sxy;
    let l1 = tr / 2.0 + (tr * tr / 4.0 - det).max(0.0).sqrt();
    let l2 = tr / 2.0 - (tr * tr / 4.0 - det).max(0.0).sqrt();
    (angle.to_degrees(), (l1 / l2.max(1e-3)).sqrt())
}

/// Each letter H's two stems and bar: with turned glyphs, the ink of one letter is as long
/// across its line as up it; with upright glyphs placed along a turned line, letters stack
/// as upright boxes. A 90° line of turned H's is a tall column one H-height wide.
#[test]
fn text_turned_90_and_30_degrees_turns_its_glyphs() {
    let upright = render(
        &text(Affine::translate(Vec2::new(10.0, 40.0)), Hinting::Full, Antialias::Grayscale),
        220,
        60,
        1.0,
    );
    let (a0, r0) = ink_axis(&upright);
    assert!(a0.abs() < 2.0 && r0 > 4.0, "upright: {a0}° {r0}");

    let f = render(&text(turned(90.0, (30.0, 10.0)), Hinting::Full, Antialias::Grayscale), 60, 220, 1.0);
    let (a, r) = ink_axis(&f);
    assert!((a.abs() - 90.0).abs() < 2.0 && (r - r0).abs() < 0.6, "90°: {a}° {r} (upright {r0})");
    // The column is one H-height wide (the H's turned), not one H-width.
    let cols: Vec<u32> =
        (0..f.width).filter(|&x| (0..f.height).any(|y| f.pixel(x, y).is_some_and(|p| p[0] < 128))).collect();
    let rows: Vec<u32> = (0..upright.height)
        .filter(|&y| (0..upright.width).any(|x| upright.pixel(x, y).is_some_and(|p| p[0] < 128)))
        .collect();
    assert!(
        cols.len().abs_diff(rows.len()) <= 1,
        "turned width {} against upright height {}",
        cols.len(),
        rows.len()
    );

    for scale in [1.0, 2.0] {
        let f = render(
            &text(turned(30.0, (20.0, 20.0)), Hinting::Full, Antialias::Grayscale),
            (240.0 * scale) as u32,
            (150.0 * scale) as u32,
            scale,
        );
        let (a, r) = ink_axis(&f);
        assert!((a - 30.0).abs() < 1.5 && (r - r0).abs() < 0.6, "30° at {scale}x: {a}° {r} (upright {r0})");
    }
}

/// A skew and an uneven stretch reach the glyphs too, and LCD text falls back to grayscale
/// there.
#[test]
fn skew_and_uneven_scale_reach_the_glyphs() {
    let wide = render(
        &text(
            Affine::scale(2.0, 1.0).then(Affine::translate(Vec2::new(5.0, 5.0))),
            Hinting::Full,
            Antialias::Lcd { bgr: false },
        ),
        420,
        60,
        1.0,
    );
    let plain = render(
        &text(Affine::translate(Vec2::new(5.0, 5.0)), Hinting::Full, Antialias::Grayscale),
        420,
        60,
        1.0,
    );
    let span = |f: &Frame| {
        (0..f.width).filter(|&x| (0..f.height).any(|y| f.pixel(x, y).is_some_and(|p| p[0] < 128))).max()
    };
    let (sw, sp) = (span(&wide).unwrap_or(0), span(&plain).unwrap_or(0));
    assert!((sw as f32 - 5.0) > 1.9 * (sp as f32 - 5.0), "twice as wide: {sw} against {sp}");
    // Same height: only x is stretched.
    let height = |f: &Frame| {
        (0..f.height).filter(|&y| (0..f.width).any(|x| f.pixel(x, y).is_some_and(|p| p[0] < 128))).count()
    };
    assert!(height(&wide).abs_diff(height(&plain)) <= 1);
    // Grayscale under the stretch: no colored fringes.
    assert!(wide.pixels.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]), "gray");
    // A skew leans the stems: the ink's top row starts right of its bottom row.
    let skew = Affine([1.0, 0.0, -0.5, 1.0, 20.0, 5.0]);
    let f = render(&text(skew, Hinting::Full, Antialias::Grayscale), 260, 60, 1.0);
    let row_start = |y: u32| (0..f.width).find(|&x| f.pixel(x, y).is_some_and(|p| p[0] < 128));
    let rows: Vec<u32> = (0..f.height).filter(|&y| row_start(y).is_some()).collect();
    let (top, bottom) = (rows.first().and_then(|&y| row_start(y)), rows.last().and_then(|&y| row_start(y)));
    assert!(
        top.zip(bottom).is_some_and(|(t, b)| t >= b + 5),
        "leans right going up: {top:?} {bottom:?}; {:?}",
        rows.iter().map(|&y| (y, row_start(y))).collect::<Vec<_>>()
    );
}

/// FNV-1a of a frame's pixels.
fn hash(f: &Frame) -> u64 {
    f.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Upright text is drawn exactly as before turned glyphs came (the hashes were taken from the
/// renderer before): hinting, subpixel bins and LCD unchanged, at whole and fractional
/// scales and positions.
#[test]
fn upright_text_is_pixel_identical() {
    let cases = [
        (Affine::translate(Vec2::new(4.0, 26.0)), 1.0, Hinting::Full, Antialias::Grayscale),
        (Affine::translate(Vec2::new(4.3, 26.6)), 1.0, Hinting::None, Antialias::Grayscale),
        (Affine::translate(Vec2::new(4.0, 26.0)), 1.5, Hinting::Light, Antialias::Lcd { bgr: false }),
        (
            Affine::scale(1.25, 1.25).then(Affine::translate(Vec2::new(2.5, 30.0))),
            2.0,
            Hinting::Full,
            Antialias::Lcd { bgr: true },
        ),
    ];
    let got: Vec<u64> = cases
        .iter()
        .map(|&(t, scale, h, aa)| {
            hash(&render(&text(t, h, aa), (200.0 * scale) as u32, (40.0 * scale) as u32, scale))
        })
        .collect();
    if std::env::var_os("REMEDY_PRINT_HASHES").is_some() {
        eprintln!("{got:#x?}");
    }
    assert_eq!(got, UPRIGHT);
}

/// From the renderer before this change (`REMEDY_PRINT_HASHES=1`).
const UPRIGHT: [u64; 4] =
    [0x6527_7630_33b2_b555, 0x40de_eb8b_06f1_21a5, 0xe7dc_cde8_4980_4a31, 0x28e4_c00b_fd4b_db65];
