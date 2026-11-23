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
    {
        let mut v_ = ParagraphOptions::default();
        v_.max_width = width;
        v_
    }
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
    let text = RichText::new().text("plain ").styled("bold", TextStylePatch::bold()).text(" plain");
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
        .styled("this link", {
            let mut v_ = TextStylePatch::link(42);
            v_.color = Some(red);
            v_
        })
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
    let forced = e.layout(&RichText::plain("abc"), &TextStyle::default(), {
        let mut v_ = ParagraphOptions::default();
        v_.direction = Direction::Rtl;
        v_
    });
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
    t.add_span(2..100, TextStylePatch::bold()); // 2 is inside 'é'
    let span = t.spans().first().map(|s| s.range.clone());
    assert_eq!(span, Some(1..6));
    let mut e = engine();
    let p = e.layout(&t, &TextStyle::default(), opts(None));
    assert!(p.width() > 0.0);
}

#[test]
fn thai_wraps_to_its_width_with_or_without_dictionaries() {
    // "Thai is a beautiful language" written, as Thai is, without spaces between words.
    let thai = "ภาษาไทยเป็นภาษาที่สวยงามมากภาษาไทยเป็นภาษาที่สวยงามมาก";
    let mut e = engine();
    let mut p = e.layout(&RichText::plain(thai), &TextStyle::default(), opts(None));
    let one_line = p.width();
    p.rewrap(Some(one_line / 4.0));
    let lines = p.lines().len();
    // With dictionaries Thai breaks between words; without them there is no break between
    // words, and the line still breaks where it must (overflow-wrap) rather than running past
    // its width.
    assert!(lines >= 3, "{lines} lines");
    assert!(p.width() <= one_line / 4.0 + 0.5, "inside the width: {} of {}", p.width(), one_line / 4.0);
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
    let o = |align, container, max_width| {
        let mut v_ = ParagraphOptions::default();
        v_.align = align;
        v_.container = container;
        v_.max_width = max_width;
        v_
    };
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
        let o = {
            let mut v_ = ParagraphOptions::default();
            v_.align = align;
            v_.max_width = Some(160.0);
            v_
        };
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

#[test]
fn underlines_sit_below_the_text_and_strikes_through_its_middle() {
    let mut e = engine();
    let text = RichText::new()
        .text("plain ")
        .styled("linked", TextStylePatch::link(1))
        .text(" ")
        .styled("struck", TextStylePatch::strikethrough());
    let p = e.layout(
        &text,
        &{
            let mut v_ = TextStyle::default();
            v_.size = 20.0;
            v_
        },
        opts(None),
    );
    let runs = p.glyph_runs();
    let run = |w: &str| {
        let at = text.as_str().find(w).unwrap_or_default();
        runs.iter()
            .find(|r| r.range.start <= at && at < r.range.end)
            .cloned()
            .unwrap_or_else(|| unreachable!("{w}"))
    };
    assert!(run("plain").underline.is_none() && run("plain").strikethrough.is_none());
    let line = p.lines().into_iter().next().unwrap_or_else(|| unreachable!("a line"));
    let under = run("linked").underline.unwrap_or_else(|| unreachable!("an underline"));
    assert!(under.min.y > line.baseline - 0.5, "below the baseline: {under:?} vs {}", line.baseline);
    assert!(under.height() >= 1.0 && under.width() > 20.0, "{under:?}");
    let strike = run("struck").strikethrough.unwrap_or_else(|| unreachable!("a strike"));
    assert!(strike.max.y < line.baseline && strike.min.y > line.baseline - 20.0, "{strike:?}");
    assert!(run("struck").underline.is_none());
}

fn at_scale(width: Option<f32>, scale: f32) -> ParagraphOptions {
    let mut o = opts(width);
    o.scale = scale;
    o
}

#[test]
fn the_device_scale_leaves_every_metric_in_logical_pixels() {
    // Android at 420 dpi is 2.625 device pixels per logical pixel; the paragraph must take the
    // same logical room there as at 1, apart from rounding to the device's pixel grid.
    let mut e = engine();
    let style = TextStyle::default();
    let text = RichText::plain("Body text at fourteen");
    let one = e.layout(&text, &style, at_scale(None, 1.0));
    for scale in [1.25, 2.0, 2.625, 3.5] {
        let p = e.layout(&text, &style, at_scale(None, scale));
        let tol = 1.0 + 1.0 / scale;
        assert!((p.width() - one.width()).abs() <= tol, "{scale}: width {} vs {}", p.width(), one.width());
        assert!(
            (p.height() - one.height()).abs() <= tol,
            "{scale}: height {} vs {}",
            p.height(),
            one.height()
        );
        let (b, b1) = (p.first_baseline().unwrap_or(0.0), one.first_baseline().unwrap_or(0.0));
        assert!((b - b1).abs() <= tol, "{scale}: baseline {b} vs {b1}");
        let lines = p.lines();
        assert_eq!(lines.len(), 1, "{scale}");
        for line in &lines {
            assert!((line.bottom - line.top - one.height()).abs() <= tol, "{scale}: line {line:?}");
        }
        for run in p.glyph_runs() {
            assert!(
                (run.size - style.size).abs() < 0.01,
                "{scale}: run drawn at {} not {}",
                run.size,
                style.size
            );
            let last = run.glyphs.last().map_or(0.0, |g| g.x);
            assert!(last < p.width(), "{scale}: glyph at {last} outside {}", p.width());
        }
    }
}

#[test]
fn wrapping_hit_testing_and_carets_take_logical_pixels_at_any_scale() {
    let mut e = engine();
    let text =
        RichText::plain(PANGRAM).object(InlineObject { id: 3, width: 20.0, height: 10.0, baseline: None });
    let one = e.layout(&text, &TextStyle::default(), at_scale(Some(160.0), 1.0));
    let mut p = e.layout(&text, &TextStyle::default(), at_scale(Some(160.0), 2.625));
    assert_eq!(p.lines().len(), one.lines().len(), "wrapped at the same logical width");
    assert!(p.width() <= 160.5, "{}", p.width());
    p.rewrap(Some(320.0));
    assert!(p.width() > 160.5 && p.width() <= 320.5, "rewrap is logical too: {}", p.width());
    let objects = p.inline_objects();
    assert_eq!(objects.len(), 1);
    for (_, obj) in objects {
        assert!((obj.width() - 20.0).abs() < 0.5 && (obj.height() - 10.0).abs() < 0.5, "{obj:?}");
    }
    // A point in the second word is in the second word at any scale.
    let at = p.hit_test(Point::new(
        one.caret(TextPosition { index: 5, affinity: Affinity::Downstream }, 1.0).min.x,
        5.0,
    ));
    assert!((4..=6).contains(&at.index), "{at:?}");
    let caret = p.caret(TextPosition { index: 4, affinity: Affinity::Downstream }, 2.0);
    assert!((caret.width() - 2.0).abs() < 0.01, "{caret:?}");
    assert!(caret.height() < 30.0, "a 14 px line's caret: {caret:?}");
    let sel = p.selection_rects(0..9);
    assert!(sel.iter().all(|r| r.height() < 30.0 && r.max.x <= 320.5), "{sel:?}");
}

/// A word longer than the wrap width breaks onto more lines rather than running past it (an
/// invite or a key pasted into a narrow field), while a short word still wraps whole.
#[test]
fn a_word_longer_than_its_line_breaks_inside_the_width() {
    let mut e = engine();
    let long = "freespeak-invite:RlNJTlYxAdVnw0mWdMxYcUCoNzSuYAIGhvM8bnCouqdDCYTfBUEcWnpoKcetKb52hmKubsvp";
    let p =
        e.layout(&RichText::plain(long), &TextStyle::default(), ParagraphOptions::default().max_width(120.0));
    assert!(p.width() <= 120.5, "it stays inside: {}", p.width());
    assert!(p.lines().len() > 2, "it took several lines: {}", p.lines().len());
    let words = e.layout(
        &RichText::plain("short words wrap whole"),
        &TextStyle::default(),
        ParagraphOptions::default().max_width(60.0),
    );
    assert!(words.width() <= 60.5);
}

fn covers(text: &str, lines: &[LineInfo]) {
    let mut at = 0;
    for line in lines {
        assert_eq!(line.range.start, at, "a gap or an overlap at {at}: {lines:?}");
        assert!(line.range.end <= text.len());
        at = line.range.end;
    }
    assert_eq!(at, text.len());
}

/// Arrow keys visit every original boundary once. A chip's padding is an extra cluster, so a
/// step that stops there would repeat a position or skip a letter.
fn arrows_visit_every_boundary(p: &Paragraph, len: usize) {
    let mut pos = TextPosition::default();
    let mut seen = vec![pos.index];
    for _ in 0..len + 8 {
        let next = p.move_visual(pos, true);
        if next == pos {
            break;
        }
        if next.index != pos.index {
            assert!(next.index > pos.index, "left to right moves forward: {pos:?} -> {next:?}");
            seen.push(next.index);
        }
        pos = next;
    }
    assert_eq!(seen, (0..=len).collect::<Vec<_>>(), "every boundary, once");
    let end = TextPosition { index: len, affinity: Affinity::Upstream };
    let mut pos = end;
    let mut back = vec![pos.index];
    for _ in 0..len + 8 {
        let next = p.move_visual(pos, false);
        if next == pos {
            break;
        }
        if next.index != pos.index {
            assert!(next.index < pos.index, "moving left goes back: {pos:?} -> {next:?}");
            back.push(next.index);
        }
        pos = next;
    }
    back.reverse();
    assert_eq!(back, (0..=len).collect::<Vec<_>>());
}

#[test]
#[allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "the chip fixture is built in this test and then indexed"
)]
fn a_span_background_paints_a_chip_around_the_callers_text() {
    let mut e = engine();
    let color = remedy_ui_core::Color::rgb8(40, 40, 48);
    let chip = SpanBackground::chip(color, 14.0);
    let text = RichText::new().text("use ").styled("code", TextStylePatch::background(chip)).text(" here");
    let src = text.as_str();
    let code_at = src.find("code").expect("code");
    let plain = e.layout(&RichText::plain(src), &TextStyle::default(), opts(None));
    let p = e.layout(&text, &TextStyle::default(), opts(None));
    covers(src, &p.lines());
    let side = 14.0 * 0.4;
    assert!(
        (p.width() - plain.width() - 2.0 * side).abs() < 1.5,
        "two sides of room: {} vs {}",
        p.width(),
        plain.width()
    );

    let runs = p.glyph_runs();
    let code = runs.iter().find(|r| r.range == (code_at..code_at + 4)).expect("the code run");
    let bg = code.background.expect("a chip");
    assert_eq!(bg.color, color);
    assert!((bg.radius - 4.0).abs() < 0.01);
    let word = e.layout(&RichText::plain("code"), &TextStyle::default(), opts(None)).width();
    assert!(
        (bg.rect.width() - word - 2.0 * side).abs() < 1.5,
        "the fill is the word plus its side room: {} vs {}",
        bg.rect.width(),
        word + 2.0 * side
    );
    let before = runs.iter().find(|r| r.range.end == code_at).expect("the words before");
    let before_right =
        before.glyphs.last().map(|g| g.x).unwrap_or(0.0) + before.advances.last().copied().unwrap_or(0.0);
    assert!(
        before_right <= bg.rect.min.x + 0.6,
        "the chip starts after 'use ': {before_right} {:?}",
        bg.rect
    );
    let first = code.glyphs.first().map(|g| g.x).unwrap_or(0.0);
    assert!(first >= bg.rect.min.x + side - 1.0, "the word sits inside the side room: {first} {:?}", bg.rect);
    assert!(p.inline_objects().is_empty(), "padding is not one of the caller's objects");

    // A point on the word, and one in the side room, both belong to the caller's text.
    let mid = p.hit_test(Point::new((bg.rect.min.x + bg.rect.max.x) / 2.0, bg.rect.center().y));
    assert!(code.range.contains(&mid.index), "{mid:?} on {code:?}");
    let edge = p.hit_test(Point::new(bg.rect.min.x + 0.5, bg.rect.center().y));
    assert_eq!(edge.index, code_at, "the side room is the span's edge: {edge:?}");
    arrows_visit_every_boundary(&p, src.len());
    let sel = p.selection_rects(code.range.clone());
    assert_eq!(sel.len(), 1, "{sel:?}");
    let last = code.glyphs.last().map(|g| g.x).unwrap_or(0.0) + code.advances.last().copied().unwrap_or(0.0);
    assert!(
        sel[0].min.x <= first + 0.6 && sel[0].max.x >= last - 0.6,
        "the highlight covers the word: {sel:?}"
    );
    assert!(
        sel[0].min.x >= bg.rect.min.x - 0.6 && sel[0].max.x <= bg.rect.max.x + 0.6,
        "and stays inside the chip: {sel:?} {bg:?}"
    );

    // Block padding is paint only: the line stays as tall, the fill grows.
    let tall = SpanBackground::new(color).padding_block(4.0).radius(2.0);
    let flat = SpanBackground::new(color);
    let tall_p = e.layout(
        &RichText::new().styled("code", TextStylePatch::background(tall)),
        &TextStyle::default(),
        opts(None),
    );
    let flat_p = e.layout(
        &RichText::new().styled("code", TextStylePatch::background(flat)),
        &TextStyle::default(),
        opts(None),
    );
    assert!((tall_p.width() - flat_p.width()).abs() < 0.6, "no side room, no extra width");
    assert!((tall_p.height() - flat_p.height()).abs() < 0.6, "block padding does not change the line");
    let tall_h = tall_p.glyph_runs()[0].background.expect("tall").rect.height();
    let flat_h = flat_p.glyph_runs()[0].background.expect("flat").rect.height();
    assert!((tall_h - flat_h - 8.0).abs() < 0.6, "4px above and below: {tall_h} vs {flat_h}");

    // A link and a chip share a run. A caller's own object keeps its id.
    let mut linked = TextStylePatch::link(7);
    linked.background = Some(chip);
    let mixed = RichText::new()
        .text("a")
        .object(InlineObject { id: 7, width: 12.0, height: 12.0, baseline: None })
        .styled("code", linked);
    let mixed_p = e.layout(&mixed, &TextStyle::default(), opts(None));
    let link = mixed_p.glyph_runs().into_iter().find(|r| r.link == Some(7));
    assert!(link.is_some_and(|r| r.background.is_some()), "the link is also a chip");
    let ids: Vec<u64> = mixed_p.inline_objects().into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec![7], "the pad is not reported: {ids:?}");

    // Wrapped, the pieces still cover the original text and the chip still has a fill.
    let wrapped = e.layout(&text, &TextStyle::default(), opts(Some(plain.width() * 0.55)));
    assert!(wrapped.lines().len() >= 2, "{}", wrapped.lines().len());
    covers(src, &wrapped.lines());
    assert!(wrapped.glyph_runs().iter().any(|r| r.range == (code_at..code_at + 4) && r.background.is_some()));
    arrows_visit_every_boundary(&wrapped, src.len());

    // The same room at a device scale, still in logical pixels.
    let scaled = e.layout(&text, &TextStyle::default(), at_scale(None, 2.0));
    let scaled_bg = scaled
        .glyph_runs()
        .into_iter()
        .find(|r| r.range == (code_at..code_at + 4))
        .and_then(|r| r.background)
        .expect("scaled chip");
    assert!((scaled_bg.rect.width() - bg.rect.width()).abs() < 1.0, "{scaled_bg:?} vs {bg:?}");
}
