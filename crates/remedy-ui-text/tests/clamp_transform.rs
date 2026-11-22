//! Line clamping (CSS `line-clamp`) and text transforms (CSS `text-transform`), measured with
//! the real shaper.

use remedy_ui_core::Point;
use remedy_ui_text::{
    Align, Direction, FontCollection, Paragraph, ParagraphOptions, RichText, TextEngine, TextStyle,
    TextStylePatch, TextTransform,
};

fn engine() -> TextEngine {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    for fb in remedy_ui_fonts::script_fallbacks() {
        let _ = f.add_script_fallback(fb.script, fb.locale, fb.family);
    }
    TextEngine::new(f)
}

const LONG: &str =
    "The quick brown fox jumps over the lazy dog while the cat watches from the old stone wall";
const HEBREW: &str = "שלום עולם זהו משפט ארוך מאוד שצריך להיחתך אחרי שתי שורות בלבד כדי לבדוק";

fn clamp(e: &mut TextEngine, text: &str, n: usize, dir: Direction, align: Align, spacing: f32) -> Paragraph {
    let mut o = ParagraphOptions::default().max_width(120.0).max_lines(n).align(align);
    o.direction = dir;
    let mut style = TextStyle::default();
    style.letter_spacing = spacing;
    e.layout(&RichText::plain(text), &style, o)
}

/// The x of the "…" glyph on the last line.
fn ellipsis_x(e: &mut TextEngine, p: &Paragraph) -> Option<f32> {
    let dots = e.layout(&RichText::plain("\u{2026}"), &TextStyle::default(), ParagraphOptions::default());
    let id = dots.glyph_runs().first()?.glyphs.first()?.id;
    let last = p.lines().last()?.clone();
    p.glyph_runs()
        .iter()
        .flat_map(|r| r.glyphs.clone())
        .filter(|g| g.y > last.top && g.y <= last.bottom && g.id == id)
        .map(|g| g.x)
        .next()
}

#[test]
fn clamped_text_takes_at_most_n_lines_and_ends_in_an_ellipsis() {
    let mut e = engine();
    for spacing in [0.0, 3.0] {
        for align in [Align::Start, Align::Center, Align::End, Align::Justify] {
            for n in 1..=3 {
                let p = clamp(&mut e, LONG, n, Direction::Ltr, align, spacing);
                let lines = p.lines();
                assert_eq!(lines.len(), n, "{n} lines, {align:?}, spacing {spacing}");
                let last = lines.last().cloned();
                let width = last.as_ref().map_or(0.0, |l| l.width);
                assert!(width <= 120.5, "the last line fits: {width}");
                // The "…" is the last glyph on the last line, at its right end.
                let x = ellipsis_x(&mut e, &p);
                let right = last.map_or(0.0, |l| l.left + l.width);
                assert!(
                    x.is_some_and(|x| x > right - 20.0 && x < right),
                    "… at the right end: {x:?} vs {right}, {align:?}, spacing {spacing}"
                );
            }
        }
    }
    // Text that fits is left alone.
    let p = clamp(&mut e, "Short", 2, Direction::Ltr, Align::Start, 0.0);
    assert_eq!(p.lines().len(), 1);
    assert_eq!(p.lines().first().map(|l| l.range.end), Some(5));
    assert_eq!(ellipsis_x(&mut e, &p), None);
}

#[test]
fn right_to_left_puts_the_ellipsis_on_the_left() {
    let mut e = engine();
    for spacing in [0.0, 3.0] {
        for align in [Align::Start, Align::Center, Align::End] {
            let p = clamp(&mut e, HEBREW, 2, Direction::Rtl, align, spacing);
            let lines = p.lines();
            assert_eq!(lines.len(), 2);
            let left = lines.last().map_or(0.0, |l| l.left);
            let x = ellipsis_x(&mut e, &p);
            assert!(
                x.is_some_and(|x| (x - left).abs() < 1.0),
                "… at the left end: {x:?} vs {left}, {align:?}, spacing {spacing}"
            );
        }
    }
}

#[test]
fn a_cut_never_splits_a_grapheme() {
    let mut e = engine();
    // Each "e" carries a combining accent: a cut lands before an e or after its accent.
    let text = "e\u{301}".repeat(80);
    let p = clamp(&mut e, &text, 1, Direction::Ltr, Align::Start, 0.0);
    let end = p.lines().first().map(|l| l.range.end).unwrap_or(0);
    // The cut text is the kept prefix and "…" (3 bytes); each grapheme is 3 bytes too.
    assert_eq!((end - 3) % 3, 0, "kept whole graphemes: {end}");
}

#[test]
fn transformed_text_keeps_offsets_of_the_text_as_written() {
    let mut e = engine();
    let mut style = TextStyle::default();
    style.transform = TextTransform::Upper;
    let text = RichText::plain("straße");
    let p = e.layout(&text, &style, ParagraphOptions::default());
    let plain = e.layout(&RichText::plain("STRASSE"), &TextStyle::default(), ParagraphOptions::default());
    assert!((p.width() - plain.width()).abs() < 0.01, "drawn as STRASSE");
    assert_eq!(p.lines().first().map(|l| l.range.clone()), Some(0..text.len()));
    let end = p.hit_test(Point::new(p.width() + 10.0, 5.0));
    assert_eq!(end.index, text.len(), "hit tests give offsets in the original");
    assert!(!p.selection_rects(4..6).is_empty(), "selecting ß covers its SS");
    // A span can turn it off, and the locale changes how i is cased.
    let mixed = RichText::new().text("up ").styled("next", TextStylePatch::transform(TextTransform::None));
    let m = e.layout(&mixed, &style, ParagraphOptions::default());
    let check = e.layout(&RichText::plain("UP next"), &TextStyle::default(), ParagraphOptions::default());
    assert!((m.width() - check.width()).abs() < 0.01);
    e.set_locale("tr-TR");
    let tr = e.layout(&RichText::plain("i"), &style, ParagraphOptions::default());
    let dotted = e.layout(&RichText::plain("İ"), &TextStyle::default(), ParagraphOptions::default());
    assert!((tr.width() - dotted.width()).abs() < 0.01, "Turkish i capitalizes to İ");
    // Greek capitals drop the tonos under the Greek locale, and Dutch capitalizes IJ.
    e.set_locale("el");
    let el = e.layout(&RichText::plain("άλφα"), &style, ParagraphOptions::default());
    let plain = e.layout(&RichText::plain("ΑΛΦΑ"), &TextStyle::default(), ParagraphOptions::default());
    assert!((el.width() - plain.width()).abs() < 0.01, "Greek capitals without accents");
    e.set_locale("nl");
    let mut title = TextStyle::default();
    title.transform = TextTransform::Capitalize;
    let nl = e.layout(&RichText::plain("ijsland"), &title, ParagraphOptions::default());
    let plain = e.layout(&RichText::plain("IJsland"), &TextStyle::default(), ParagraphOptions::default());
    assert!((nl.width() - plain.width()).abs() < 0.01, "IJsland");
    e.set_locale("tr-TR");
    // Clamping composes with casing.
    let clamped =
        e.layout(&RichText::plain(LONG), &style, ParagraphOptions::default().max_width(120.0).max_lines(2));
    assert_eq!(clamped.lines().len(), 2);
}

#[test]
fn clamping_a_huge_paragraph_lays_out_only_its_first_lines() {
    let mut e = engine();
    let text: String = LONG.chars().chain([' ']).cycle().take(50_000).collect();
    let wrap = ParagraphOptions::default().max_width(300.0);
    let whole = e.layout(&RichText::plain(&text), &TextStyle::default(), wrap);
    let first: Vec<_> = whole.lines_in(0.0..60.0).into_iter().take(2).map(|l| l.range).collect();
    let started = std::time::Instant::now();
    let p = e.layout(&RichText::plain(&text), &TextStyle::default(), wrap.max_lines(3));
    let took = started.elapsed();
    eprintln!("clamping 50,000 characters to 3 lines took {took:?}");
    let lines = p.lines();
    assert_eq!(lines.len(), 3);
    // The first two lines are the whole text's first two.
    let got: Vec<_> = lines.iter().take(2).map(|l| l.range.clone()).collect();
    assert_eq!(got, first);
    assert!(ellipsis_x(&mut e, &p).is_some());
}
