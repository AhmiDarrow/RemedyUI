//! Mixed-direction paragraphs: selections cover exactly the selected characters wherever
//! the bidirectional algorithm drew them, hit testing finds the character under a point in
//! either direction, and carets at a direction change sit on the side they belong to.

use remedy_ui_text::{
    Affinity, Direction, FontCollection, Paragraph, ParagraphOptions, RichText, TextEngine, TextPosition, TextStyle,
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

fn layout(e: &mut TextEngine, text: &str, dir: Direction) -> Paragraph {
    let mut o = ParagraphOptions::default();
    o.direction = dir;
    e.layout(&RichText::plain(text), &TextStyle::default(), o)
}

fn caret_x(p: &Paragraph, index: usize, affinity: Affinity) -> f32 {
    p.caret(TextPosition { index, affinity }, 1.0).min.x
}

#[test]
fn a_selection_across_a_direction_change_covers_both_runs() -> Result<(), Box<dyn std::error::Error>> {
    let mut e = engine();
    let text = "abc مرحبا def";
    let p = layout(&mut e, text, Direction::Auto);
    assert!(!p.is_rtl());
    let arabic = text.find('م').unwrap_or(0)..text.find(" def").unwrap_or(0);
    // The Arabic word alone: one rectangle, between the English words.
    let r = p.selection_rects(arabic.clone());
    assert_eq!(r.len(), 1, "{r:?}");
    let abc = p.selection_rects(0..3);
    let def = p.selection_rects(text.len() - 3..text.len());
    let (r0, a, d) = (r.first().ok_or("arabic")?, abc.first().ok_or("abc")?, def.first().ok_or("def")?);
    assert!(r0.min.x >= a.max.x - 0.5 && r0.max.x <= d.min.x + 0.5, "{r0:?} between {a:?} and {d:?}");
    // From inside "abc" into the Arabic word: the selected Arabic letters are at the run's
    // right (its start), so the selection is two pieces with a gap where the unselected
    // Arabic letters are drawn.
    let partial = p.selection_rects(1..arabic.start + "مر".len());
    assert_eq!(partial.len(), 2, "{partial:?}");
    let total: f32 = partial.iter().map(|r| r.width()).sum();
    assert!(total > 0.0);
    let (first, second) = (partial.first().ok_or("first")?, partial.get(1).ok_or("second")?);
    assert!(second.min.x > first.max.x + 1.0, "a gap between the pieces: {partial:?}");
    Ok(())
}

#[test]
fn hit_testing_finds_characters_in_both_directions() -> Result<(), Box<dyn std::error::Error>> {
    let mut e = engine();
    for (text, dir) in [("abc مرحبا def", Direction::Auto), ("مرحبا report الآن", Direction::Rtl)] {
        let p = layout(&mut e, text, dir);
        let y = p.height() / 2.0;
        for (i, c) in text.char_indices() {
            if c == ' ' {
                continue;
            }
            // The middle of each character's own box hits that character's boundaries.
            let r = p.selection_rects(i..i + c.len_utf8()).into_iter().next().ok_or("a box")?;
            let hit = p.hit_test(remedy_ui_core::Point::new((r.min.x + r.max.x) / 2.0, y));
            assert!(hit.index == i || hit.index == i + c.len_utf8(), "{text}: {c:?} at {i}, hit {hit:?}");
        }
    }
    Ok(())
}

#[test]
fn right_to_left_paragraphs_start_at_the_right() {
    let mut e = engine();
    let text = "مرحبا بكم";
    let p = layout(&mut e, text, Direction::Auto);
    assert!(p.is_rtl());
    let start = caret_x(&p, 0, Affinity::Downstream);
    let end = caret_x(&p, text.len(), Affinity::Downstream);
    assert!(start > end + 20.0, "the start is drawn right of the end: {start} {end}");
    // Each next character is drawn further left.
    let mut last = f32::INFINITY;
    for (i, _) in text.char_indices() {
        let x = caret_x(&p, i, Affinity::Downstream);
        assert!(x <= last + 0.01, "{i}");
        last = x;
    }
    // A forced left-to-right paragraph still draws the Arabic run right to left inside it.
    let ltr = layout(&mut e, text, Direction::Ltr);
    assert!(!ltr.is_rtl());
    assert!(ltr.glyph_runs().iter().all(|r| r.rtl || r.glyphs.is_empty() || text.get(r.range.clone()) == Some(" ")));
}

#[test]
fn a_caret_at_a_direction_change_has_two_places() {
    // Index 4 ends "abc " and starts the Arabic run: drawn after the space (upstream) or at
    // the Arabic run's right edge (downstream).
    let mut e = engine();
    let text = "abc مرحبا";
    let p = layout(&mut e, text, Direction::Ltr);
    let up = caret_x(&p, 4, Affinity::Upstream);
    let down = caret_x(&p, 4, Affinity::Downstream);
    assert!((up - down).abs() > 10.0, "two places: {up} {down}");
    // Moving visually right from the start lands on every place exactly once.
    let mut pos = TextPosition::default();
    let mut xs = vec![caret_x(&p, 0, Affinity::Downstream)];
    for _ in 0..40 {
        let n = p.move_visual(pos, true);
        if n == pos {
            break;
        }
        pos = n;
        xs.push(p.caret(pos, 1.0).min.x);
    }
    for w in xs.windows(2) {
        assert!(w.last() > w.first(), "{xs:?}");
    }
    assert_eq!(xs.len(), text.chars().count() + 1, "{xs:?}");
}
