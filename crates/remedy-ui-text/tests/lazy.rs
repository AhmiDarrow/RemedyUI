//! A huge paragraph laid out lazily, a piece at a time, is the paragraph laid out whole: the
//! same lines and line breaks, every glyph at the same place, and the same hit tests, carets,
//! selections, words and movements, in Latin, in mixed Arabic, Hebrew and English (with
//! numbers and brackets, in either base direction) and in Chinese, asked for in any order;
//! and so is each paragraph after edits laid out from the one before.
//!
//! The Arabic, Hebrew and Chinese faces are the OFL fixture subsets in `fixtures/shaping`
//! (see `reference_shapes.rs`), so the text is made of the words they cover.

use remedy_ui_core::{Point, Rect};
use remedy_ui_text::{
    Affinity, Direction, FontCollection, GlyphRunInfo, LAZY_MIN_BYTES, Paragraph, ParagraphOptions, RichText,
    TextEngine, TextPosition, TextStyle, TextStylePatch,
};

const ARABIC: &[u8] = include_bytes!("fixtures/shaping/NotoSansArabic-Subset.ttf");
const HEBREW: &[u8] = include_bytes!("fixtures/shaping/NotoSansHebrew-Subset.ttf");
const CJK: &[u8] = include_bytes!("fixtures/shaping/NotoSansSC-Subset.ttf");

/// An engine with the Remedy faces, then the fixture faces in the style's family list.
fn engine() -> (TextEngine, TextStyle) {
    let mut fonts = FontCollection::new();
    let mut families = Vec::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    families.push("Remedy Sans UI".to_string());
    for data in [ARABIC, HEBREW, CJK] {
        let names = fonts.add_static(data).unwrap_or_else(|e| unreachable!("a fixture face: {e}"));
        families.extend(names);
    }
    let mut style = TextStyle::default();
    style.families = families;
    style.size = 15.0;
    (TextEngine::new(fonts), style)
}

/// Words joined until the text is `bytes` long, picked by a fixed sequence.
fn words(list: &[&str], bytes: usize, sep: &str) -> String {
    let mut s = String::new();
    let mut x: u32 = 7;
    while s.len() < bytes {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        s.push_str(list.get((x >> 16) as usize % list.len()).copied().unwrap_or(""));
        s.push_str(sep);
    }
    s
}

fn latin(bytes: usize) -> String {
    words(
        &[
            "the",
            "quick",
            "brown",
            "fox",
            "jumps",
            "over",
            "a",
            "lazy",
            "dog,",
            "while",
            "typographers",
            "argue",
            "about",
            "kerning",
            "(and",
            "ligatures)",
            "in",
            "1984;",
            "office",
            "efficient",
            "AVATAR",
            "Wolf",
            "To",
            "paragraphs",
            "of",
            "unusual",
            "length.",
            "\u{2014}",
            "supercalifragilisticexpialidociousandthensomemoreletters",
        ],
        bytes,
        " ",
    )
}

/// English with Hebrew and Arabic phrases, numbers and brackets among them.
fn mixed(bytes: usize) -> String {
    words(
        &[
            "Open",
            "now",
            "Remedy",
            "and",
            "files",
            "שלום עולם",
            "42",
            "קבצים",
            "ב-Remedy.",
            "(שָׁלוֹם)",
            "123",
            "يمكنك حفظ الملفات",
            "٣",
            "ملفات؟",
            "(الآن)",
            "لاحقا،",
            "3.14",
            "[עוֹלָם 7]",
            "x",
        ],
        bytes,
        " ",
    )
}

/// Arabic, mostly, with Hebrew, English and numbers.
fn arabic(bytes: usize) -> String {
    words(
        &[
            "يمكنك",
            "حفظ",
            "الملفات",
            "الآن",
            "أو",
            "لاحقا،",
            "ولا",
            "التراجع",
            "عن",
            "ذلك",
            "بعد",
            "الحفظ",
            "٣",
            "ملفات؟",
            "لا",
            "Remedy",
            "42",
            "שלום",
            "(عن ذلك)",
            "123.",
            "لاحقًا",
        ],
        bytes,
        " ",
    )
}

/// Chinese, with Latin numbers and words.
fn chinese(bytes: usize) -> String {
    words(
        &[
            "您可以撤消此操作。",
            "保存这些文件吗？",
            "可以。",
            "保存 3 个文件？",
            "Remedy ",
            "操作",
            "文件",
            "42",
        ],
        bytes,
        "",
    )
}

fn options(width: f32, direction: Direction) -> ParagraphOptions {
    let mut o = ParagraphOptions::default().max_width(width);
    o.direction = direction;
    o
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.01
}

fn near_rect(a: Rect, b: Rect) -> bool {
    near(a.min.x, b.min.x) && near(a.min.y, b.min.y) && near(a.max.x, b.max.x) && near(a.max.y, b.max.y)
}

fn same_runs(a: &[GlyphRunInfo], b: &[GlyphRunInfo], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: run count");
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x.range, y.range, "{what}: run range");
        assert_eq!(
            (x.font.id(), x.font.index(), x.rtl, x.link),
            (y.font.id(), y.font.index(), y.rtl, y.link),
            "{what}: run at {:?}",
            x.range
        );
        assert!(near(x.size, y.size) && x.color == y.color, "{what}: run style at {:?}", x.range);
        assert_eq!(x.clusters, y.clusters, "{what}: clusters at {:?}", x.range);
        assert_eq!(x.glyphs.len(), y.glyphs.len(), "{what}: glyphs at {:?}", x.range);
        for (g, h) in x.glyphs.iter().zip(&y.glyphs) {
            assert!(
                g.id == h.id && near(g.x, h.x) && near(g.y, h.y),
                "{what}: glyph {g:?} vs {h:?} in {:?}",
                x.range
            );
        }
        assert!(
            x.advances.iter().zip(&y.advances).all(|(p, q)| near(*p, *q)),
            "{what}: advances at {:?}",
            x.range
        );
        assert_eq!(x.underline.is_some(), y.underline.is_some(), "{what}: underline at {:?}", x.range);
    }
}

/// Every line, glyph and object of `lazy` is `whole`'s.
fn same_layout(lazy: &Paragraph, whole: &Paragraph, what: &str) {
    let (a, b) = (lazy.lines(), whole.lines());
    assert_eq!(a.len(), b.len(), "{what}: line count");
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.range, y.range, "{what}: line breaks");
        assert!(
            near(x.top, y.top)
                && near(x.bottom, y.bottom)
                && near(x.baseline, y.baseline)
                && near(x.width, y.width)
                && near(x.left, y.left),
            "{what}: line {x:?} vs {y:?}"
        );
    }
    same_runs(&lazy.glyph_runs(), &whole.glyph_runs(), what);
    let (oa, ob) = (lazy.inline_objects(), whole.inline_objects());
    assert_eq!(oa.len(), ob.len(), "{what}: objects");
    for ((i, r), (j, s)) in oa.iter().zip(&ob) {
        assert!(i == j && near_rect(*r, *s), "{what}: object {i}");
    }
    assert!(lazy.is_complete());
    assert!(near(lazy.height(), whole.height()) && near(lazy.width(), whole.width()), "{what}: size");
    assert_eq!(lazy.is_rtl(), whole.is_rtl(), "{what}: direction");
    assert_eq!(lazy.first_baseline(), whole.first_baseline(), "{what}: first baseline");
}

/// Character boundaries spread over the text, and around every `step`-th line break.
fn probes(text: &str, whole: &Paragraph, count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = (0..count)
        .map(|i| text.len() * i / count)
        .map(|i| (i..=text.len()).find(|&j| text.is_char_boundary(j)).unwrap_or(text.len()))
        .collect();
    for l in whole.lines().iter().step_by(7) {
        out.push(l.range.start);
        out.push(l.range.end);
        if let Some((i, _)) = text.get(l.range.start..).and_then(|t| t.char_indices().nth(1)) {
            out.push(l.range.start + i);
        }
    }
    out.push(text.len());
    out
}

/// The queries a view and an editor make give the same answers, asked of a paragraph laid
/// out lazily in a scattered order (so most land on pieces not laid out yet).
fn same_answers(text: &str, lazy: &Paragraph, whole: &Paragraph, what: &str) {
    let mut at = probes(text, whole, 60);
    // Far, then near, then back: pieces are laid out out of order of asking.
    at.sort_by_key(|i| (i * 7919) % (text.len() + 1));
    let lines = whole.lines();
    for &i in &at {
        for affinity in [Affinity::Downstream, Affinity::Upstream] {
            let pos = TextPosition { index: i, affinity };
            assert!(
                near_rect(lazy.caret(pos, 1.0), whole.caret(pos, 1.0)),
                "{what}: caret at {i} {affinity:?}"
            );
            for rightward in [true, false] {
                assert_eq!(
                    lazy.move_visual(pos, rightward),
                    whole.move_visual(pos, rightward),
                    "{what}: move from {i} {affinity:?} {rightward}"
                );
                assert_eq!(
                    lazy.move_word(pos, rightward),
                    whole.move_word(pos, rightward),
                    "{what}: word move from {i} {affinity:?} {rightward}"
                );
            }
        }
    }
    // Selections that cross pieces, lines, and none.
    for (&x, &y) in at.iter().zip(at.iter().skip(1)) {
        let (a, b) = (x.min(y), x.max(y));
        for r in [a..b, a..a, a..(b.min(a + 300)).max(a)] {
            let r = r.start..(r.start..=r.end).rev().find(|&j| text.is_char_boundary(j)).unwrap_or(r.start);
            let (x, y) = (lazy.selection_rects(r.clone()), whole.selection_rects(r.clone()));
            assert_eq!(x.len(), y.len(), "{what}: selection {r:?}");
            assert!(
                x.iter().zip(&y).all(|(p, q)| near_rect(*p, *q)),
                "{what}: selection {r:?}: {x:?} vs {y:?}"
            );
        }
    }
    // Points on lines, on the boundaries between them, and beyond the paragraph.
    let width = whole.width();
    for (n, l) in lines.iter().enumerate().step_by(5) {
        for y in [l.top, (l.top + l.bottom) / 2.0, l.bottom - 0.01] {
            for x in [-5.0, 0.0, width * 0.3, width * 0.77, width + 10.0] {
                let p = Point::new(x, y);
                assert_eq!(lazy.hit_test(p), whole.hit_test(p), "{what}: hit test at {p:?} (line {n})");
                assert_eq!(lazy.word_at(p), whole.word_at(p), "{what}: word at {p:?}");
            }
        }
    }
    for p in [Point::new(10.0, -20.0), Point::new(10.0, whole.height() + 40.0)] {
        assert_eq!(lazy.hit_test(p), whole.hit_test(p), "{what}: hit test at {p:?}");
    }
}

fn check(text: &str, style: &TextStyle, engine: &mut TextEngine, o: ParagraphOptions, what: &str) {
    let rich = RichText::plain(text);
    check_rich(&rich, style, engine, o, what);
}

fn check_rich(rich: &RichText, style: &TextStyle, engine: &mut TextEngine, o: ParagraphOptions, what: &str) {
    assert!(rich.len() >= LAZY_MIN_BYTES);
    let whole = engine.layout_whole(rich, style, o);
    assert!(!whole.is_lazy());
    // Showing the top lays out only the top.
    let shown = engine.layout(rich, style, o);
    assert!(shown.is_lazy(), "{what}: laid out lazily");
    let top: Vec<_> = whole.lines_in(0.0..400.0);
    let mine = shown.lines_in(0.0..400.0);
    assert_eq!(mine.len(), top.len(), "{what}: lines in the window");
    same_runs(&shown.glyph_runs_in(0.0..400.0), &whole.glyph_runs_in(0.0..400.0), what);
    assert!(!shown.is_complete(), "{what}: the window lays out only what it shows");
    // A band in the middle, asked for first.
    let mid = whole.height() / 2.0;
    let band = engine.layout(rich, style, o);
    same_runs(&band.glyph_runs_in(mid..mid + 300.0), &whole.glyph_runs_in(mid..mid + 300.0), what);
    let lazy = engine.layout(rich, style, o);
    same_answers(rich.as_str(), &lazy, &whole, what);
    same_layout(&lazy, &whole, what);
    same_layout(&band, &whole, what);
}

#[test]
fn latin_lays_out_lazily_as_whole() {
    let (mut e, style) = engine();
    let text = latin(120_000);
    check(&text, &style, &mut e, options(360.0, Direction::Auto), "latin");
    // Narrow enough that the long word breaks inside, and centered and justified.
    let mut o = options(140.0, Direction::Auto);
    o.align = remedy_ui_text::Align::Justify;
    check(&latin(60_000), &style, &mut e, o, "latin narrow justified");
    let mut o = options(300.0, Direction::Auto).scale(1.5);
    o.align = remedy_ui_text::Align::Center;
    check(&latin(60_000), &style, &mut e, o, "latin centered at 1.5x");
}

#[test]
fn mixed_bidi_lays_out_lazily_as_whole() {
    let (mut e, style) = engine();
    let text = mixed(100_000);
    check(&text, &style, &mut e, options(420.0, Direction::Auto), "mixed, left to right");
    check(&text, &style, &mut e, options(420.0, Direction::Rtl), "mixed, right to left");
    let text = arabic(100_000);
    check(&text, &style, &mut e, options(380.0, Direction::Auto), "arabic");
    // Left to right until well past the first pieces, then right-to-left text.
    let text = format!("{}{}", latin(50_000), arabic(40_000));
    check(&text, &style, &mut e, options(380.0, Direction::Auto), "latin then arabic");
}

#[test]
fn chinese_lays_out_lazily_as_whole() {
    let (mut e, style) = engine();
    let text = chinese(100_000);
    check(&text, &style, &mut e, options(333.0, Direction::Auto), "chinese");
}

#[test]
fn spans_objects_and_links_lay_out_lazily_as_whole() {
    let (mut e, style) = engine();
    let mut rich = RichText::new();
    let mut i = 0u64;
    while rich.len() < 80_000 {
        i += 1;
        rich.push_text(&latin(700 + (i as usize % 5) * 90));
        match i % 4 {
            0 => rich.push_styled("bold words here ", TextStylePatch::bold()),
            1 => rich.push_styled("a link to follow ", TextStylePatch::link(i)),
            2 => rich.push_object(remedy_ui_text::InlineObject {
                id: i,
                width: 30.0,
                height: 22.0,
                baseline: None,
            }),
            _ => rich.push_styled("שלום עולם ", TextStylePatch::italic()),
        }
    }
    check_rich(&rich, &style, &mut e, options(400.0, Direction::Auto), "spans and objects");
    let lazy = e.layout(&rich, &style, options(400.0, Direction::Auto));
    let whole = e.layout_whole(&rich, &style, options(400.0, Direction::Auto));
    for link in [5u64, 41, 77] {
        let (a, b) = (lazy.link_rects(link), whole.link_rects(link));
        assert_eq!(a.len(), b.len(), "link {link}");
        assert!(a.iter().zip(&b).all(|(p, q)| near_rect(*p, *q)), "link {link}");
        if let Some(r) = b.first() {
            let p = Point::new((r.min.x + r.max.x) / 2.0, (r.min.y + r.max.y) / 2.0);
            assert_eq!(lazy.link_at(p), whole.link_at(p), "link at {p:?}");
        }
    }
}

/// Edits laid out from the paragraph before them, compared with the edited text laid out
/// whole.
fn check_edits(
    text: &str,
    style: &TextStyle,
    e: &mut TextEngine,
    o: ParagraphOptions,
    inserts: &[&str],
    what: &str,
) {
    let mut text = text.to_string();
    let mut p = e.layout(&RichText::plain(text.clone()), style, o);
    let at =
        |t: &str, f: usize| (t.len() * f / 100..=t.len()).find(|&i| t.is_char_boundary(i)).unwrap_or(t.len());
    // Look at the middle first, as an editor with the caret there does.
    let _ = p.caret(TextPosition { index: at(&text, 50), ..Default::default() }, 1.0);
    let steps: Vec<(usize, usize, &str)> = vec![
        (50, 0, inserts.first().copied().unwrap_or("x")),
        (50, 0, inserts.get(1).copied().unwrap_or("y")),
        (50, 3, ""),
        (49, 0, inserts.get(2).copied().unwrap_or(" ")),
        (10, 0, inserts.first().copied().unwrap_or("x")),
        (90, 40, inserts.get(1).copied().unwrap_or("y")),
        (0, 0, inserts.get(2).copied().unwrap_or("z")),
        (100, 0, " end"),
        (30, 2000, ""),
    ];
    for (n, (f, remove, insert)) in steps.into_iter().enumerate() {
        let a = at(&text, f);
        let b = (a + remove..=text.len()).find(|&i| text.is_char_boundary(i)).unwrap_or(text.len());
        text.replace_range(a..b, insert);
        let rich = RichText::plain(text.clone());
        p = e.layout_after_edit(&p, &rich, style, o);
        let whole = e.layout_whole(&rich, style, o);
        let caret = TextPosition { index: a + insert.len(), affinity: Affinity::Downstream };
        assert!(near_rect(p.caret(caret, 1.0), whole.caret(caret, 1.0)), "{what}: caret after edit {n}");
        let c = whole.caret(caret, 1.0);
        let band = c.min.y - 200.0..c.max.y + 200.0;
        same_runs(
            &p.glyph_runs_in(band.clone()),
            &whole.glyph_runs_in(band),
            &format!("{what}: around edit {n}"),
        );
        if n % 3 == 2 {
            same_layout(&p, &whole, &format!("{what}: after edit {n}"));
            // Complete now, so lay out again from text not yet looked at.
            p = e.layout(&rich, style, o);
            let _ = p.caret(TextPosition { index: at(&text, 50), ..Default::default() }, 1.0);
        }
    }
    let whole = e.layout_whole(&RichText::plain(text.clone()), style, o);
    same_layout(&p, &whole, &format!("{what}: at the end"));
}

#[test]
fn edits_lay_out_as_whole() {
    let (mut e, style) = engine();
    let o = options(360.0, Direction::Auto);
    check_edits(&latin(100_000), &style, &mut e, o, &["x", "kerning ", " "], "latin");
    // Right-to-left text typed into left-to-right text, and numbers after it.
    check_edits(&latin(100_000), &style, &mut e, o, &["שלום ", "42 ", "(ملفات) "], "hebrew typed into latin");
    check_edits(&mixed(80_000), &style, &mut e, o, &["x", "123", "שלום"], "mixed");
    check_edits(&arabic(80_000), &style, &mut e, o, &["ملفات", "٣", "Open "], "arabic");
    check_edits(
        &chinese(80_000),
        &style,
        &mut e,
        options(333.0, Direction::Auto),
        &["文件", "3", "。"],
        "chinese",
    );
}

#[test]
fn typing_keeps_what_the_edit_did_not_touch() {
    // Typing in the middle lays out about one piece: the lines far below are kept.
    let (mut e, style) = engine();
    let o = options(360.0, Direction::Auto);
    let mut text = latin(400_000);
    let mut p = e.layout(&RichText::plain(text.clone()), &style, o);
    let _ = p.glyph_runs();
    assert!(p.is_complete());
    let mid = text.len() / 2;
    for i in 0..5 {
        text.insert(mid + i, 'q');
        let rich = RichText::plain(text.clone());
        p = e.layout_after_edit(&p, &rich, &style, o);
        let _ = p.caret(TextPosition { index: mid + i + 1, ..Default::default() }, 1.0);
        assert!(p.is_complete(), "the lines after the edit were taken from before it");
    }
    let whole = e.layout_whole(&RichText::plain(text), &style, o);
    same_layout(&p, &whole, "typing");
}

#[test]
fn short_or_unwrapped_paragraphs_lay_out_whole() {
    let (mut e, style) = engine();
    let short = RichText::plain(latin(LAZY_MIN_BYTES - 100));
    assert!(!e.layout(&short, &style, options(300.0, Direction::Auto)).is_lazy());
    let long = RichText::plain(latin(LAZY_MIN_BYTES * 2));
    assert!(!e.layout(&long, &style, ParagraphOptions::default()).is_lazy(), "one line: nothing to skip");
    let mut p = e.layout(&long, &style, options(300.0, Direction::Auto));
    assert!(p.is_lazy());
    p.rewrap(Some(500.0));
    let whole = e.layout_whole(&long, &style, options(500.0, Direction::Auto));
    same_layout(&p, &whole, "rewrapped");
    p.rewrap(None);
    assert!(!p.is_lazy());
}
