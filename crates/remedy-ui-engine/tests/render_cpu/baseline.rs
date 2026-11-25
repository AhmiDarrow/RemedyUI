//! Every letter of a line sits on the same baseline, at every scale, hinting and edge mode.

use remedy_ui_core::{Color, Point};
use remedy_ui_engine::paint::{DisplayList, GlyphRun, Item};
use remedy_ui_engine::render_cpu::{CpuRenderer, Frame};
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

const TEXT: &str = "Online  Just now  Cedar Finch  Sage Gold 3";

/// Every bundled family with letters on a baseline (Ahmi Pixel is drawn on whole pixels), and
/// whether its stems end flat. Remedy Sans and AhmiPrint are drawn with round terminals, so
/// unhinted, the tip of a thin stem that falls between two pixel columns covers less than half
/// of each: only hinting puts their stems and curves on one row.
const FAMILIES: &[(&str, bool)] = &[("Remedy Sans UI", true), ("Remedy Sans", false), ("AhmiPrint", false)];

/// Letters whose ink ends on the baseline with a stem: in a family whose stems end flat, their
/// bottom edge is the baseline itself, in every hinting mode.
const FLAT: &str = "nlihdF";
/// Letters whose bottom is a curve with overshoot: hinting puts it on the baseline row too.
/// (A font whose curves dip a few units further than its stems, as Remedy Sans and AhmiPrint
/// before 1.201 did, fails this at some sizes in every hinter, FreeType's included.)
const ROUND: &str = "eoascCOSG3";

struct Line {
    frame: Frame,
    /// (character, left and right device column) of every glyph.
    glyphs: Vec<(char, u32, u32)>,
}

fn line(family: &str, scale: f32, origin: Point, hinting: Hinting, aa: Antialias) -> Line {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let mut style = TextStyle::default();
    style.families = vec![family.into()];
    style.size = 13.0;
    let p = engine.layout(&RichText::plain(TEXT), &style, ParagraphOptions::default().scale(scale));
    let mut list = DisplayList::new();
    let mut glyphs = Vec::new();
    for run in p.glyph_runs() {
        let xs: Vec<f32> = run.glyphs.iter().map(|g| (origin.x + g.x) * scale).collect();
        for (i, ch) in TEXT.chars().take(run.glyphs.len()).enumerate() {
            let left = xs.get(i).copied().unwrap_or(0.0);
            let right = xs.get(i + 1).copied().unwrap_or(left + 13.0 * scale / 2.0);
            // Stay a pixel clear of the neighbours' ink.
            glyphs.push((ch, left.floor() as u32 + 1, (right.floor() as u32).saturating_sub(1)));
        }
        list.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color: Color::BLACK,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(origin.x + g.x, origin.y + g.y))).collect(),
            hinting,
            antialias: aa,
        }));
    }
    let (w, h) = ((p.width() * scale) as u32 + 40, ((p.height() + origin.y) * scale) as u32 + 20);
    let frame = match CpuRenderer::new().render(&list, w, h, scale, Color::WHITE) {
        Ok(f) => f,
        Err(e) => unreachable!("{e}"),
    };
    Line { frame, glyphs }
}

/// The lowest device row in columns `x0..=x1` with at least half coverage.
fn bottom(f: &Frame, x0: u32, x1: u32) -> Option<u32> {
    (0..f.height)
        .rev()
        .find(|&y| (x0..=x1).any(|x| f.pixel(x, y).is_some_and(|p| p[0].min(p[1]).min(p[2]) < 128)))
}

#[test]
fn every_letter_sits_on_the_lines_baseline() {
    let mut failures = Vec::new();
    for &(family, flat_stems) in FAMILIES {
        for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
            let unhinted = if flat_stems { vec![(Hinting::None, FLAT.to_string())] } else { Vec::new() };
            for (hinting, letters) in [(Hinting::Full, [FLAT, ROUND].concat())].into_iter().chain(unhinted) {
                for aa in [Antialias::Grayscale, Antialias::Lcd { bgr: false }] {
                    for oy in [4.0, 4.2, 4.4, 4.5, 4.6, 4.8, 5.1, 7.33] {
                        for ox in [3.0, 3.3, 3.7] {
                            let l = line(family, scale, Point::new(ox, oy), hinting, aa);
                            let rows: Vec<(char, Option<u32>)> = l
                                .glyphs
                                .iter()
                                .filter(|(c, _, _)| letters.contains(*c))
                                .map(|&(c, x0, x1)| (c, bottom(&l.frame, x0, x1)))
                                .collect();
                            let first = rows.first().and_then(|r| r.1);
                            if rows.iter().any(|r| r.1 != first || r.1.is_none()) {
                                failures.push(format!(
                                    "{family} scale {scale} {hinting:?} {aa:?} origin ({ox}, {oy}): {rows:?}"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} lines with letters off the baseline:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
