//! Paragraph layout with the bundled Remedy fonts.

use remedy_ui_core::Point;
use remedy_ui_text::*;

fn engine() -> TextEngine {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        assert!(fonts.add_static(face.data).is_ok(), "{} {}", face.family, face.weight);
    }
    TextEngine::new(fonts)
}

fn opts(width: Option<f32>) -> ParagraphOptions {
    ParagraphOptions { max_width: width, ..Default::default() }
}

const PANGRAM: &str =
    "The quick brown fox jumps over the lazy dog while the five boxing wizards jump quickly.";

#[test]
fn fonts_register_by_family() {
    let mut e = engine();
    let families = e.fonts().families();
    assert!(families.iter().any(|f| f == "Remedy Sans UI"), "{families:?}");
    assert!(families.iter().any(|f| f == "AhmiPrint"), "{families:?}");
    assert!(e.fonts().add_owned(b"not a font".to_vec()).is_err());
}

#[test]
fn wraps_to_the_width_and_rewraps_without_reshaping() {
    let mut e = engine();
    let mut p = e.layout(&RichText::plain(PANGRAM), &TextStyle::default(), opts(None));
    assert_eq!(p.lines().len(), 1, "no width: one line");
    let one_line = p.width();
    p.rewrap(Some(one_line / 3.0));
    let lines = p.lines();
    assert!(lines.len() >= 3, "{} lines", lines.len());
    assert!(p.width() <= one_line / 3.0 + 0.5);
    // Lines cover the text in order with no gaps.
    let mut at = 0;
    for l in &lines {
        assert_eq!(l.range.start, at);
        at = l.range.end;
        assert!(l.bottom > l.top && l.baseline > l.top && l.baseline <= l.bottom);
    }
    assert_eq!(at, PANGRAM.len());
}

#[test]
fn bold_spans_draw_with_the_bold_face() {
    let mut e = engine();
    let text = RichText::new().text("plain ").styled("bold", StylePatch::bold()).text(" plain");
    let p = e.layout(&text, &TextStyle::default(), opts(None));
    let runs = p.glyph_runs();
    let plain = runs.iter().find(|r| r.range.start == 0).map(|r| r.font.id());
    let bold = runs.iter().find(|r| r.range.contains(&6)).map(|r| r.font.id());
    assert!(plain.is_some() && bold.is_some());
    assert_ne!(plain, bold, "bold text uses a different face");
    assert!(runs.iter().all(|r| !r.glyphs.is_empty()));
}

#[test]
fn colors_and_links_come_back_with_their_runs() {
    let mut e = engine();
    let red = remedy_ui_core::Color::rgb8(200, 30, 30);
    let text = RichText::new()
        .text("see ")
        .styled("this link", StylePatch { color: Some(red), ..StylePatch::link(42) })
        .text(" now");
    let p = e.layout(&text, &TextStyle::default(), opts(None));
    let link_run = p.glyph_runs().into_iter().find(|r| r.link == Some(42));
    assert_eq!(link_run.as_ref().map(|r| r.color), Some(red));
    assert_eq!(
        link_run.as_ref().map(|r| r.range.clone()),
        Some(4..13),
        "exactly the span, not the shaping run"
    );
    // A point over the link finds it; a point over plain text does not.
    let rects = p.selection_rects(4..13);
    let over = rects.first().map(|r| r.center());
    assert_eq!(over.and_then(|pt| p.link_at(pt)), Some(42));
    assert_eq!(p.link_at(Point::new(1.0, 5.0)), None);
}

#[test]
fn inline_objects_take_space_and_report_their_place() {
    let mut e = engine();
    let text = RichText::new()
        .text("wave ")
        .object(InlineObject { id: 7, width: 20.0, height: 20.0, baseline: None })
        .text(" hello");
    let p = e.layout(&text, &TextStyle::default(), opts(None));
    let objects = p.inline_objects();
    assert_eq!(objects.len(), 1);
    let (id, rect) = objects.first().copied().unwrap_or((0, remedy_ui_core::Rect::ZERO));
    assert_eq!(id, 7);
    assert!((rect.width() - 20.0).abs() < 0.01);
    assert!(rect.min.x > 0.0, "after the word");
    let without = e.layout(&RichText::plain("wave  hello"), &TextStyle::default(), opts(None));
    assert!(p.width() > without.width() + 15.0);
}

#[test]
fn carets_and_hit_tests_agree() {
    let mut e = engine();
    let p = e.layout(&RichText::plain("abcdef"), &TextStyle::default(), opts(None));
    let mut last_x = -1.0;
    for i in 0..=6 {
        let c = p.caret(TextPosition { index: i, affinity: Affinity::Downstream }, 1.0);
        assert!(c.min.x > last_x, "caret {i} moves right");
        last_x = c.min.x;
        let back = p.hit_test(Point::new(c.min.x + 0.2, c.center().y));
        assert_eq!(back.index, i);
    }
    assert_eq!(p.hit_test(Point::new(-50.0, 5.0)).index, 0);
    assert_eq!(p.hit_test(Point::new(10_000.0, 5.0)).index, 6);
}

#[test]
fn selections_cover_each_line_they_touch() {
    let mut e = engine();
    let mut p = e.layout(&RichText::plain(PANGRAM), &TextStyle::default(), opts(None));
    p.rewrap(Some(p.width() / 3.0));
    let lines = p.lines().len();
    let rects = p.selection_rects(0..PANGRAM.len());
    assert_eq!(rects.len(), lines);
    let word = p.word_at(Point::new(2.0, 5.0));
    assert_eq!(&PANGRAM[word], "The");
}

#[test]
fn right_to_left_runs_are_marked_and_moved_through_visually() {
    let mut e = engine();
    // Hebrew is not in the bundled fonts, so it draws as .notdef, but bidi still applies.
    let text = "abc \u{05D0}\u{05D1}\u{05D2} def";
    let p = e.layout(&RichText::plain(text), &TextStyle::default(), opts(None));
    assert!(!p.is_rtl(), "the first strong character is Latin");
    let runs = p.glyph_runs();
    assert!(runs.iter().any(|r| r.rtl), "the Hebrew run reads right to left");
    assert!(runs.iter().any(|r| !r.rtl));

    let rtl = e.layout(&RichText::plain("\u{05D0}\u{05D1} abc"), &TextStyle::default(), opts(None));
    assert!(rtl.is_rtl(), "the first strong character is Hebrew");
    let forced = e.layout(
        &RichText::plain("abc"),
        &TextStyle::default(),
        ParagraphOptions { direction: Direction::Rtl, ..Default::default() },
    );
    assert!(forced.is_rtl());

    // Moving right through the whole line visits every position and ends at the far right.
    let mut pos = TextPosition::default();
    let mut steps = 0;
    loop {
        let next = p.move_visual(pos, true);
        if next == pos || steps > 50 {
            break;
        }
        pos = next;
        steps += 1;
    }
    assert!(steps >= 10, "{steps} steps");
}

#[test]
fn spans_are_clamped_to_character_boundaries() {
    let mut t = RichText::plain("héllo");
    t.add_span(2..100, StylePatch::bold()); // 2 is inside 'é'
    let span = t.spans().first().map(|s| s.range.clone());
    assert_eq!(span, Some(1..6));
    let mut e = engine();
    let p = e.layout(&t, &TextStyle::default(), opts(None));
    assert!(p.width() > 0.0);
}

#[test]
fn thai_breaks_inside_words_only_with_the_complex_scripts_feature() {
    // "Thai is a beautiful language" written, as Thai is, without spaces between words.
    let thai = "ภาษาไทยเป็นภาษาที่สวยงามมากภาษาไทยเป็นภาษาที่สวยงามมาก";
    let mut e = engine();
    let mut p = e.layout(&RichText::plain(thai), &TextStyle::default(), opts(None));
    let one_line = p.width();
    p.rewrap(Some(one_line / 4.0));
    let lines = p.lines().len();
    if cfg!(feature = "complex-scripts") {
        assert!(lines >= 3, "dictionary breaks: {lines} lines");
    } else {
        assert_eq!(lines, 1, "without dictionaries there is no break opportunity");
    }
    // Chinese breaks between ideographs either way.
    let zh = "汉字之间可以换行汉字之间可以换行汉字之间可以换行";
    let mut p = e.layout(&RichText::plain(zh), &TextStyle::default(), opts(None));
    let w = p.width();
    p.rewrap(Some(w / 4.0));
    assert!(p.lines().len() >= 3, "{} lines", p.lines().len());
}

#[test]
fn paragraphs_say_where_they_sit_in_a_wider_box() {
    let mut e = engine();
    let o =
        |align, container, max_width| ParagraphOptions { align, container, max_width, ..Default::default() };
    let short = RichText::plain("Hi");
    let p = e.layout(&short, &TextStyle::default(), o(Align::Start, Direction::Ltr, None));
    assert_eq!(p.x_offset(200.0), 0.0, "start of a left-to-right box");
    let w = p.width();
    let p = e.layout(&short, &TextStyle::default(), o(Align::Start, Direction::Rtl, None));
    assert!((p.x_offset(200.0) - (200.0 - w)).abs() < 0.5, "start of a right-to-left box is its right edge");
    let p = e.layout(&short, &TextStyle::default(), o(Align::Center, Direction::Ltr, None));
    assert!((p.x_offset(200.0) - (200.0 - w) / 2.0).abs() < 0.5);
    let p = e.layout(&short, &TextStyle::default(), o(Align::End, Direction::Ltr, None));
    assert!((p.x_offset(200.0) - (200.0 - w)).abs() < 0.5);
    // Wrapped paragraphs were aligned within their wrap width already.
    let p = e.layout(
        &RichText::plain(PANGRAM),
        &TextStyle::default(),
        o(Align::Start, Direction::Rtl, Some(150.0)),
    );
    assert_eq!(p.x_offset(150.0), 0.0);
    assert!(p.lines().len() > 2);
    // Auto container: start follows the paragraph's own direction (Hebrew starts at the right).
    let p = e.layout(&RichText::plain("שלום"), &TextStyle::default(), o(Align::Start, Direction::Auto, None));
    assert!(p.x_offset(200.0) > 100.0, "{}", p.x_offset(200.0));
}

#[test]
fn wrapped_lines_take_their_alignment() {
    let mut e = engine();
    let mut lefts = |align| {
        let o = ParagraphOptions { align, max_width: Some(160.0), ..Default::default() };
        let p = e.layout(&RichText::plain(PANGRAM), &TextStyle::default(), o);
        p.lines().iter().map(|l| l.left).collect::<Vec<f32>>()
    };
    let start = lefts(Align::Start);
    assert!(start.iter().all(|x| x.abs() < 0.5), "{start:?}");
    let center = lefts(Align::Center);
    assert!(center.iter().any(|x| *x > 2.0), "centered lines move in: {center:?}");
    let end = lefts(Align::End);
    assert!(end.iter().zip(&center).all(|(e, c)| e >= c), "end is further right than center: {end:?}");
}
