//! Deceptive text: hidden controls, look-alikes, and pathological input.

use remedy_ui_text::*;

#[test]
fn bidi_controls_are_revealed_and_emoji_joiners_left_alone() {
    let s = "a\u{202E}b\u{2066}c\u{2069}";
    assert!(has_hidden_controls(s, Reveal::default()));
    assert_eq!(reveal_controls(s, Reveal::default()), "a⟨RLO⟩b⟨LRI⟩c⟨PDI⟩");
    let family = "\u{1F468}\u{200D}\u{1F469}";
    assert!(!has_hidden_controls(family, Reveal { zero_width: true }), "ZWJ is part of emoji");
    let hidden_space = "pass\u{200B}word";
    assert!(!has_hidden_controls(hidden_space, Reveal::default()));
    assert_eq!(reveal_controls(hidden_space, Reveal { zero_width: true }), "pass⟨ZWSP⟩word");
}

#[test]
fn look_alike_names_are_caught() {
    assert!(looks_like("admin", "\u{0430}dmin"), "Cyrillic a");
    assert!(looks_like("rn", "m") || skeleton("rn") == skeleton("m"));
    assert!(!looks_like("alice", "bob"));
    assert_eq!(skeleton("\u{0430}"), skeleton("a"));
}

#[test]
fn piles_of_combining_marks_lay_out_in_linear_time() {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        assert!(fonts.add_static(face.data).is_ok());
    }
    let mut e = TextEngine::new(fonts);
    let zalgo: String = std::iter::once('a').chain(std::iter::repeat_n('\u{0301}', 20_000)).collect();
    let t = std::time::Instant::now();
    let p = e.layout(
        &RichText::plain(zalgo),
        &TextStyle::default(),
        ParagraphOptions { max_width: Some(300.0), ..Default::default() },
    );
    // Measured at about 4 ms in release and well under a second in debug; the bound leaves
    // room for slow CI machines while catching anything worse than linear.
    assert!(t.elapsed().as_secs_f32() < 2.0, "{:?}", t.elapsed());
    assert_eq!(p.lines().len(), 1, "one grapheme stays on one line");
}
