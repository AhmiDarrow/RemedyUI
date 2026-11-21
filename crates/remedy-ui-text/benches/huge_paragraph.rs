//! Time to first frame and time per keystroke in one huge paragraph, and layout time of
//! paragraphs of ordinary size.
//!
//! Run with `cargo bench -p remedy-ui-text --bench huge_paragraph` (a release build). Each
//! figure is the median of several runs. "First frame" is what a view does before it can
//! draw a paragraph shown in a window 800 px wide and 600 px tall: lay it out and take the
//! glyphs of the lines in the window. "Keystroke" is typing one character in the middle of
//! the paragraph with the caret there: lay the edited text out again, place the caret and
//! take the glyphs of the lines around it.

use std::time::{Duration, Instant};

use remedy_ui_text::{
    FontCollection, Paragraph, ParagraphOptions, RichText, TextEngine, TextPosition, TextStyle,
};

const WIDTH: f32 = 800.0;
const VIEW: f32 = 600.0;

fn engine() -> TextEngine {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        fonts.add_static(face.data).unwrap_or_else(|e| unreachable!("a bundled face: {e}"));
    }
    TextEngine::new(fonts)
}

/// About `bytes` of words of different lengths, as prose has.
fn prose(bytes: usize) -> String {
    const WORDS: &[&str] = &[
        "the",
        "quick",
        "brown",
        "fox",
        "jumps",
        "over",
        "a",
        "lazy",
        "dog",
        "while",
        "seventeen",
        "typographers",
        "argue",
        "about",
        "kerning",
        "and",
        "ligatures",
        "in",
        "paragraphs",
        "of",
        "unusual",
        "length,",
        "which",
        "nobody",
        "reads",
        "to",
        "the",
        "end.",
    ];
    let mut s = String::with_capacity(bytes + 16);
    let mut x: u32 = 12345;
    while s.len() < bytes {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        s.push_str(WORDS.get((x >> 16) as usize % WORDS.len()).copied().unwrap_or("a"));
        s.push(' ');
    }
    s
}

fn options() -> ParagraphOptions {
    ParagraphOptions::default().max_width(WIDTH)
}

/// The glyphs a view draws for the window at `top`.
fn shown(p: &Paragraph, top: f32) -> usize {
    p.glyph_runs_in(top..top + VIEW).iter().map(|r| r.glyphs.len()).sum()
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v.get(v.len() / 2).copied().unwrap_or_default()
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn main() {
    // `cargo bench` passes `--bench`; anything else is a filter it may pass too.
    let mut engine = engine();
    let style = TextStyle::default();
    println!("paragraph | first frame | keystroke");
    for (name, bytes, runs) in [("1 MB", 1usize << 20, 5), ("10 MB", 10usize << 20, 3)] {
        let text = prose(bytes);
        let rich = RichText::plain(text.clone());
        let mut first = Vec::new();
        for _ in 0..runs {
            let t = Instant::now();
            let p = engine.layout(&rich, &style, options());
            std::hint::black_box(shown(&p, 0.0));
            first.push(t.elapsed());
        }
        // Typing in the middle: the caret is there already (its lines laid out once).
        let mid = text.len() / 2;
        let mid = (mid..).find(|&i| text.is_char_boundary(i)).unwrap_or(mid);
        let mut p = engine.layout(&rich, &style, options());
        let caret = p.caret(TextPosition { index: mid, ..Default::default() }, 1.0);
        std::hint::black_box(shown(&p, caret.min.y));
        let mut keys = Vec::new();
        let mut edited = text.clone();
        for i in 0..runs.max(5) {
            edited.insert(mid + i, 'x');
            let rich = RichText::plain(edited.clone());
            let t = Instant::now();
            p = engine.layout_after_edit(&p, &rich, &style, options());
            let c = p.caret(TextPosition { index: mid + i + 1, ..Default::default() }, 1.0);
            std::hint::black_box(shown(&p, (c.min.y - VIEW / 2.0).max(0.0)));
            keys.push(t.elapsed());
        }
        println!("{name} | {} | {}", ms(median(first)), ms(median(keys)));
    }
    println!();
    println!("ordinary paragraph | layout and draw");
    for bytes in [100usize, 1_000, 10_000, 30_000] {
        let rich = RichText::plain(prose(bytes));
        let runs = 200_000 / bytes.max(1000) + 5;
        let mut v = Vec::new();
        for _ in 0..runs {
            let t = Instant::now();
            let p = engine.layout(&rich, &style, options());
            std::hint::black_box(p.glyph_runs().len());
            v.push(t.elapsed());
        }
        println!("{bytes} B | {}", ms(median(v)));
    }
}
