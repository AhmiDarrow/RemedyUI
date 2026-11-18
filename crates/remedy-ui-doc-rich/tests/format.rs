//! Rich documents to Markdown and back.

use remedy_ui_doc_rich::{MarkdownFormat, RichFormat};
use remedy_ui_text::{Block, Mark, RichDoc};

fn round_trip(doc: RichDoc) {
    let doc = doc.normalized();
    let md = MarkdownFormat.write(&doc);
    assert_eq!(MarkdownFormat.read(&md), doc, "Markdown was:\n{md}");
}

#[test]
fn marks_links_and_mentions_survive() {
    let text = "Say hi to @Alex and read the guide in bold italic struck code now";
    let at = |w: &str| text.find(w).unwrap_or_default();
    let span = |w: &str| at(w)..at(w) + w.len();
    round_trip(RichDoc {
        text: text.into(),
        marks: vec![
            (span("@Alex"), Mark::Mention("u1".into())),
            (span("the guide"), Mark::Link("https://example.org/a_b?c=(d)".into())),
            (span("bold italic"), Mark::Bold),
            (span("italic"), Mark::Italic),
            (span("struck"), Mark::Strike),
            (span("code"), Mark::Code),
        ],
        blocks: vec![Block::Paragraph],
    });
}

#[test]
fn blocks_survive() {
    round_trip(RichDoc {
        text: "Title\nfirst para\nsecond para\none\ntwo\nnested\nthree\ndone\nopen\nwise\nwords\nfn x() {\n\n    `y`\n}\nend".into(),
        marks: Vec::new(),
        blocks: vec![
            Block::Heading(2),
            Block::Paragraph,
            Block::Paragraph,
            Block::Numbered(0),
            Block::Numbered(0),
            Block::Bullet(1),
            Block::Numbered(0),
            Block::Task { done: true, level: 0 },
            Block::Task { done: false, level: 0 },
            Block::Quote,
            Block::Quote,
            Block::Code,
            Block::Code,
            Block::Code,
            Block::Code,
            Block::Paragraph,
        ],
    });
}

#[test]
fn text_that_looks_like_markdown_stays_text() {
    round_trip(RichDoc::plain(
        "# not a heading\n- not a list\n1. not numbered\n*not emphasis* [not a link](x) <b>not html</b> &amp; `not code` > not a quote",
    ));
    // Line breaks inside a list item.
    round_trip(RichDoc {
        text: "item\u{2028}more of it\nnext".into(),
        marks: vec![(0..4, Mark::Bold)],
        blocks: vec![Block::Bullet(0), Block::Bullet(0)],
    });
}

#[test]
fn spaces_at_a_marks_edges_stay_outside_it() {
    let doc =
        RichDoc { text: "a bold b".into(), marks: vec![(1..7, Mark::Bold)], blocks: vec![Block::Paragraph] };
    let md = MarkdownFormat.write(&doc);
    assert_eq!(md, "a **bold** b");
    assert_eq!(MarkdownFormat.read(&md).marks, [(2..6, Mark::Bold)]);
}

#[test]
fn ordinary_markdown_reads_in() {
    let doc = MarkdownFormat.read("Hello *world*\nsame paragraph\n\n- a\n- b\n\n> q");
    assert_eq!(doc.text, "Hello world same paragraph\na\nb\nq");
    assert_eq!(doc.blocks, [Block::Paragraph, Block::Bullet(0), Block::Bullet(0), Block::Quote]);
    assert_eq!(doc.marks, [(6..11, Mark::Italic)]);
}
