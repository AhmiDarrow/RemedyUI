//! Rich editing: marks that follow edits, blocks per line, mentions as units, one undo
//! history, and the display map.

use remedy_ui_core::Color;
use remedy_ui_text::{Block, Mark, RichDoc, RichEditor, RichLook};

fn look() -> RichLook {
    RichLook {
        link: Color::rgb8(0, 0, 255),
        mention: Color::rgb8(0, 128, 0),
        muted: Color::rgb8(128, 128, 128),
        code_families: vec!["Mono".into()],
        heading_sizes: [28.0, 22.0, 18.0],
    }
}

fn typed(e: &mut RichEditor, s: &str, t: &mut u64) {
    for c in s.chars() {
        e.insert(&c.to_string(), *t);
        *t += 50;
    }
}

fn marks(e: &RichEditor) -> Vec<(std::ops::Range<usize>, Mark)> {
    e.doc().marks
}

#[test]
fn bold_follows_typing_and_undo_takes_it_back_in_order() {
    let mut e = RichEditor::new(RichDoc::plain("hello world"));
    e.set_selection(0, 5);
    e.toggle(Mark::Bold);
    assert_eq!(marks(&e), [(0..5, Mark::Bold)]);
    assert_eq!(e.active_marks(), [Mark::Bold]);
    // Typing at the end of the bold word continues it.
    e.set_selection(5, 5);
    let mut t = 10_000;
    typed(&mut e, "!!", &mut t);
    assert_eq!(e.text(), "hello!! world");
    assert_eq!(marks(&e), [(0..7, Mark::Bold)]);
    // Typing before it does not.
    e.set_selection(0, 0);
    t += 5_000;
    typed(&mut e, ">", &mut t);
    assert_eq!(marks(&e), [(1..8, Mark::Bold)]);
    // Undo walks back: the ">", the "!!", then the bold.
    assert!(e.undo());
    assert_eq!(e.text(), "hello!! world");
    assert!(e.undo());
    assert_eq!(e.text(), "hello world");
    assert_eq!(marks(&e), [(0..5, Mark::Bold)]);
    assert!(e.undo());
    assert!(marks(&e).is_empty());
    assert!(e.redo());
    assert_eq!(marks(&e), [(0..5, Mark::Bold)]);
    // Toggling bold over a bold selection takes it off just there.
    e.set_selection(1, 3);
    e.toggle(Mark::Bold);
    assert_eq!(marks(&e), [(0..1, Mark::Bold), (3..5, Mark::Bold)]);
}

#[test]
fn toggling_at_the_caret_styles_what_is_typed_next() {
    let mut e = RichEditor::new(RichDoc::plain("say "));
    let mut t = 0;
    e.toggle(Mark::Italic);
    assert_eq!(e.active_marks(), [Mark::Italic]);
    typed(&mut e, "hi", &mut t);
    e.toggle(Mark::Italic);
    typed(&mut e, " there", &mut t);
    assert_eq!(e.text(), "say hi there");
    assert_eq!(marks(&e), [(4..6, Mark::Italic)]);
    // Moving the caret forgets a toggle nothing was typed with.
    e.toggle(Mark::Bold);
    e.move_char(false, false);
    assert!(e.active_marks().is_empty());
}

#[test]
fn links_cover_their_words_and_grow_when_typed_inside() {
    let mut e = RichEditor::new(RichDoc::plain("read the guide now"));
    e.set_selection(9, 14);
    e.set_link(Some("https://example.org".into()), None, 0);
    assert_eq!(marks(&e), [(9..14, Mark::Link("https://example.org".into()))]);
    e.set_selection(11, 11);
    assert_eq!(e.link_at_caret(), Some((9..14, "https://example.org".to_owned())));
    e.insert("x", 0);
    assert_eq!(marks(&e), [(9..15, Mark::Link("https://example.org".into()))]);
    // With the caret in it, removing the link removes all of it.
    e.set_link(None, None, 0);
    assert!(marks(&e).is_empty());
    // With nothing selected and no link there, the link is inserted.
    e.set_selection(0, 0);
    e.set_link(Some("https://a.example".into()), Some("A"), 0);
    assert!(e.text().starts_with("Aread"));
    assert_eq!(marks(&e), [(0..1, Mark::Link("https://a.example".into()))]);
}

#[test]
fn a_mention_is_one_unit() {
    let mut e = RichEditor::new(RichDoc::plain("hi "));
    e.insert_mention("u1", "@Alex", 0);
    assert_eq!(e.text(), "hi @Alex ");
    assert_eq!(marks(&e), [(3..8, Mark::Mention("u1".into()))]);
    // The caret steps over it.
    e.set_selection(8, 8);
    e.move_char(false, false);
    assert_eq!(e.selection().focus, 3);
    e.move_char(true, false);
    assert_eq!(e.selection().focus, 8);
    // Backspace after it deletes all of it.
    e.backspace(10_000);
    assert_eq!(e.text(), "hi  ");
    assert!(marks(&e).is_empty());
    assert!(e.undo());
    assert_eq!(e.text(), "hi @Alex ");
    assert_eq!(marks(&e), [(3..8, Mark::Mention("u1".into()))]);
    // Typing inside one leaves plain text.
    e.set_selection(5, 5);
    e.insert("x", 20_000);
    assert!(marks(&e).is_empty());
    // Two mentions side by side stay two.
    let mut e = RichEditor::new(RichDoc::plain(""));
    e.insert_mention("a", "@A", 0);
    e.set_selection(2, 2);
    e.insert_mention("a", "@A", 0);
    assert_eq!(e.text(), "@A@A  ");
    assert_eq!(marks(&e).len(), 2);
}

#[test]
fn lists_continue_end_nest_and_number() {
    let mut e = RichEditor::new(RichDoc::plain("one"));
    let mut t = 0;
    e.set_block(Block::Numbered(0));
    e.enter(t);
    typed(&mut e, "two", &mut t);
    e.enter(t);
    // Tab nests the new item.
    assert!(e.indent(true));
    typed(&mut e, "two a", &mut t);
    e.enter(t);
    assert!(e.indent(false));
    typed(&mut e, "three", &mut t);
    let doc = e.doc();
    assert_eq!(doc.text, "one\ntwo\ntwo a\nthree");
    assert_eq!(doc.blocks, [Block::Numbered(0), Block::Numbered(0), Block::Numbered(1), Block::Numbered(0)]);
    let v = e.view(&look());
    assert_eq!(v.text.as_str(), "1.\u{2002}one\n2.\u{2002}two\n\u{2003}1.\u{2002}two a\n3.\u{2002}three");
    // Enter on an empty item ends the list.
    e.enter(t);
    e.enter(t);
    assert_eq!(e.doc().blocks.last(), Some(&Block::Paragraph));
    assert_eq!(e.text(), "one\ntwo\ntwo a\nthree\n");
    // Backspace at the start of an item makes it a paragraph.
    e.set_selection(4, 4);
    e.backspace(t);
    assert_eq!(e.doc().blocks.get(1), Some(&Block::Paragraph));
    assert_eq!(e.text(), "one\ntwo\ntwo a\nthree\n", "no text was deleted");
    // Tab outside a list is not the editor's.
    assert!(!e.indent(true));
}

#[test]
fn tasks_tick_and_headings_survive_enter_at_their_start() {
    let mut e = RichEditor::new(RichDoc {
        text: "Title\nbuy milk".into(),
        marks: Vec::new(),
        blocks: vec![Block::Heading(1), Block::Task { done: false, level: 0 }],
    });
    let v = e.view(&look());
    assert_eq!(v.text.as_str(), "Title\n☐\u{2002}buy milk");
    let box_at = v.text.as_str().find('☐').unwrap_or_default();
    assert_eq!(v.task_at(box_at), Some(1));
    e.toggle_task(1);
    assert_eq!(e.doc().blocks.get(1), Some(&Block::Task { done: true, level: 0 }));
    // Enter at the start of the heading opens a paragraph above it.
    e.set_selection(0, 0);
    e.enter(0);
    assert_eq!(e.doc().blocks, [Block::Paragraph, Block::Heading(1), Block::Task { done: true, level: 0 }]);
    // A new task after a done one starts not done.
    let end = e.text().len();
    e.set_selection(end, end);
    e.enter(0);
    assert_eq!(e.doc().blocks.last(), Some(&Block::Task { done: false, level: 0 }));
}

#[test]
fn display_positions_map_both_ways_around_markers_and_compositions() {
    let mut e = RichEditor::new(RichDoc {
        text: "ab\ncd".into(),
        marks: vec![(3..5, Mark::Bold)],
        blocks: vec![Block::Paragraph, Block::Bullet(0)],
    });
    let v = e.view(&look());
    let marker = "•\u{2002}".len();
    assert_eq!(v.to_display(3), 3 + marker, "the second line's start sits after its bullet");
    assert_eq!(v.to_model(3), 3, "the bullet maps to its line's start");
    assert_eq!(v.to_model(3 + marker + 1), 4);
    for i in 0..=5 {
        assert_eq!(v.to_model(v.to_display(i)), i);
    }
    // The bold mark lands on "cd" in the display.
    let bold: Vec<_> =
        v.text.spans().iter().filter(|s| s.patch.weight == Some(700.0)).map(|s| s.range.clone()).collect();
    assert_eq!(bold, vec![std::ops::Range { start: 3 + marker, end: 5 + marker }]);
    // While composing, the composition shows in place and maps back to where it sits.
    e.set_selection(1, 1);
    e.set_composition("xyz", 3);
    let v = e.view(&look());
    assert_eq!(v.text.as_str(), "axyzb\n•\u{2002}cd");
    assert_eq!(v.composition, Some(1..4));
    assert_eq!(v.to_display(2), 5);
    assert_eq!(v.to_model(5), 2);
    e.commit_composition("xyz", 0);
    assert_eq!(e.text(), "axyzb\ncd");
    assert_eq!(marks(&e), [(6..8, Mark::Bold)]);
}

#[test]
fn pasting_keeps_marks_and_blocks_and_cut_returns_them() {
    let src = RichDoc {
        text: "bold\nitem".into(),
        marks: vec![(0..4, Mark::Bold)],
        blocks: vec![Block::Paragraph, Block::Bullet(0)],
    };
    let mut e = RichEditor::new(RichDoc::plain("x"));
    e.paste(&src, 0);
    let doc = e.doc();
    assert_eq!(doc.text, "xbold\nitem");
    assert_eq!(doc.marks, [(1..5, Mark::Bold)]);
    assert_eq!(doc.blocks, [Block::Paragraph, Block::Bullet(0)]);
    assert!(e.undo());
    assert_eq!(e.doc(), RichDoc::plain("x"));
    assert!(e.redo());
    assert_eq!(e.doc(), doc);
    e.set_selection(1, 10);
    let cut = e.cut(0);
    assert_eq!(cut, src);
    assert_eq!(e.text(), "x");
}

#[test]
fn a_soft_break_stays_in_the_block() {
    let mut e =
        RichEditor::new(RichDoc { text: "item".into(), marks: Vec::new(), blocks: vec![Block::Bullet(0)] });
    e.soft_break(0);
    e.insert("more", 0);
    assert_eq!(e.doc().blocks, [Block::Bullet(0)]);
    assert_eq!(e.text(), "item\u{2028}more");
}

#[test]
fn many_different_links_cost_one_pass_and_stay_in_order() {
    let n = 20_000;
    let marks = (0..n).rev().map(|i| (i * 2..i * 2 + 1, Mark::Link(format!("https://example.org/{i}"))));
    let doc = RichDoc { text: "ab".repeat(n), marks: marks.collect(), blocks: Vec::new() }.normalized();
    assert_eq!(doc.marks.len(), n);
    assert!(doc.marks.windows(2).all(|w| matches!(w, [a, b] if a.0.start < b.0.start)));
    // Touching runs of one mark still join; mentions side by side stay two.
    let joined = RichDoc {
        text: "abcd".into(),
        marks: vec![
            (2..4, Mark::Bold),
            (0..2, Mark::Bold),
            (0..2, Mark::Mention("a".into())),
            (2..4, Mark::Mention("a".into())),
        ],
        blocks: Vec::new(),
    }
    .normalized();
    assert_eq!(joined.marks.len(), 3, "{:?}", joined.marks);
}
