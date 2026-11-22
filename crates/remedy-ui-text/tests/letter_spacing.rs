//! Letter spacing (CSS `letter-spacing`): extra space after every cluster, never inside one
//! (a letter with its combining mark, an Arabic ligature's parts stay together), in either
//! direction.

use remedy_ui_text::{
    Direction, FontCollection, Paragraph, ParagraphOptions, RichText, TextEngine, TextStyle,
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

fn layout(e: &mut TextEngine, text: &str, dir: Direction, spacing: f32) -> Paragraph {
    let mut o = ParagraphOptions::default();
    o.direction = dir;
    let mut style = TextStyle::default();
    style.letter_spacing = spacing;
    e.layout(&RichText::plain(text), &style, o)
}

/// How much wider `text` grows with 4 px of spacing.
fn growth(e: &mut TextEngine, text: &str, dir: Direction) -> f32 {
    layout(e, text, dir, 4.0).width() - layout(e, text, dir, 0.0).width()
}

#[test]
fn spacing_goes_between_clusters_in_both_directions() {
    let mut e = engine();
    // Six letters and a space: seven clusters, seven spacings.
    let ltr = growth(&mut e, "UP NEXT", Direction::Ltr);
    assert!((ltr - 28.0).abs() < 0.5, "seven clusters with the space: {ltr}");
    // An e with a combining acute is one cluster: one spacing, not two.
    let combined = growth(&mut e, "e\u{301}e\u{301}", Direction::Ltr);
    assert!((combined - 8.0).abs() < 0.5, "two clusters: {combined}");
    // Right to left the same: each cluster once, and the paragraph still starts at the right.
    let rtl = growth(&mut e, "abc", Direction::Rtl);
    assert!((rtl - 12.0).abs() < 0.5, "three clusters right to left: {rtl}");
    let p = layout(&mut e, "\u{5e9}\u{5dc}\u{5d5}\u{5dd}", Direction::Rtl, 4.0);
    let q = layout(&mut e, "\u{5e9}\u{5dc}\u{5d5}\u{5dd}", Direction::Rtl, 0.0);
    assert!((p.width() - q.width() - 16.0).abs() < 0.5, "Hebrew: {} vs {}", p.width(), q.width());
}
