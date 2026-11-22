//! The text limits the app sets: lazy layout, list depth and undo grouping. One test, so no
//! other test in this binary sees them changed.

use remedy_ui_text::{
    Block, FontCollection, LAZY_MIN_BYTES, LazyLimits, MAX_LEVEL, ParagraphOptions, RichText, TextEditor,
    TextEngine, TextStyle, from_html, lazy_limits, max_list_level, set_lazy_limits, set_max_list_level,
    set_undo_pause_ms,
};

#[test]
fn the_app_moves_the_text_limits() {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let mut style = TextStyle::default();
    style.families = vec!["Remedy Sans UI".into()];
    let text = RichText::plain("word ".repeat(LAZY_MIN_BYTES / 5 + 10));
    let mut options = ParagraphOptions::default();
    options.max_width = Some(300.0);
    assert_eq!(lazy_limits(), LazyLimits::DEFAULT);
    assert!(engine.layout(&text, &style, options).is_lazy());
    set_lazy_limits(LazyLimits::default().min_bytes(usize::MAX));
    assert!(!engine.layout(&text, &style, options).is_lazy(), "laid out whole");
    set_lazy_limits(LazyLimits::default().min_bytes(1024).chunk_bytes(512).kept_pieces(2));
    let small = RichText::plain("word ".repeat(400));
    let p = engine.layout(&small, &style, options);
    assert!(p.is_lazy());
    let whole = engine.layout_whole(&small, &style, options);
    let ranges = |p: &remedy_ui_text::Paragraph| p.lines().into_iter().map(|l| l.range).collect::<Vec<_>>();
    assert_eq!(ranges(&p), ranges(&whole), "small pieces lay out the same lines");
    set_lazy_limits(LazyLimits::default());

    // Lists nest as deep as the app lets them.
    let nested =
        |depth: usize| format!("{}<li>deep</li>{}", "<ul><li>x".repeat(depth), "</li></ul>".repeat(depth));
    let deepest = |doc: remedy_ui_text::RichDoc| doc.blocks.iter().map(|b| b.level()).max();
    assert_eq!(max_list_level(), MAX_LEVEL);
    assert_eq!(deepest(from_html(&nested(9))), Some(MAX_LEVEL));
    set_max_list_level(8);
    assert_eq!(deepest(from_html(&nested(9))), Some(8));
    set_max_list_level(MAX_LEVEL);
    assert!(Block::Bullet(0).is_list());

    // Typing joins one undo step unless a pause is longer than the app's.
    let mut ed = TextEditor::new("");
    ed.insert("a", 0);
    ed.insert("b", 500);
    assert_eq!(ed.undo_depth(), 1);
    set_undo_pause_ms(100);
    ed.insert("c", 1000);
    ed.insert("d", 1050);
    ed.insert("e", 1500);
    set_undo_pause_ms(remedy_ui_text::UNDO_PAUSE_MS);
    assert_eq!(ed.undo_depth(), 3, "c and d join; e is a step of its own");
}
