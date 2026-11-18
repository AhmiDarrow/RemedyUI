//! HTML for the clipboard: what other apps paste in is kept only as allowed formatting, and
//! what we copy out reads back the same.

use remedy_ui_text::{Block, Mark, RichDoc, from_html, to_html};

#[test]
fn formatting_comes_in_as_marks_and_blocks() {
    let doc = from_html(
        "<h2>Plan</h2><p>Some <b>bold</b>, <em>italic</em>, <s>gone</s> and <code>x = 1</code>.</p>\
         <ul><li>one</li><li>two<ul><li>deep</li></ul></li></ul><ol><li>first</li></ol>\
         <blockquote><p>wise words</p></blockquote><pre><code>let a = 1;\nlet b = 2;</code></pre>",
    );
    assert_eq!(
        doc.text,
        "Plan\nSome bold, italic, gone and x = 1.\none\ntwo\ndeep\nfirst\nwise words\nlet a = 1;\nlet b = 2;"
    );
    assert_eq!(
        doc.blocks,
        [
            Block::Heading(2),
            Block::Paragraph,
            Block::Bullet(0),
            Block::Bullet(0),
            Block::Bullet(1),
            Block::Numbered(0),
            Block::Quote,
            Block::Code,
            Block::Code,
        ]
    );
    let at = |w: &str| doc.text.find(w).unwrap_or_default();
    assert!(doc.marks.contains(&(at("bold")..at("bold") + 4, Mark::Bold)));
    assert!(doc.marks.contains(&(at("italic")..at("italic") + 6, Mark::Italic)));
    assert!(doc.marks.contains(&(at("gone")..at("gone") + 4, Mark::Strike)));
    assert!(doc.marks.contains(&(at("x = 1")..at("x = 1") + 5, Mark::Code)));
    // Code inside <pre> is a code block, not inline code as well.
    assert!(!doc.marks.iter().any(|(r, m)| *m == Mark::Code && r.start >= at("let a")));
}

#[test]
fn nothing_runs_and_only_safe_links_survive() {
    let doc = from_html(
        "<p onclick=\"steal()\">Hi<script>alert(1)</script><style>p{}</style> \
         <a href=\"javascript:alert(1)\">bad</a> <a href=\"data:text/html,x\">worse</a> \
         <a href=\"HTTPS://example.org/a?b=1&amp;c=2\">good</a> <a href='mailto:me@example.org'>mail</a> \
         <iframe src=\"https://evil\">framed</iframe><img src=x onerror=alert(1) alt=\"a cat\"></p>",
    );
    assert_eq!(doc.text, "Hi bad worse good mail a cat");
    let links: Vec<&Mark> = doc.marks.iter().map(|(_, m)| m).collect();
    assert_eq!(
        links,
        [&Mark::Link("HTTPS://example.org/a?b=1&c=2".into()), &Mark::Link("mailto:me@example.org".into())]
    );
    // Mentions are never read from HTML: anyone can write the markup.
    let doc = from_html("<span class=\"mention\" data-mention=\"admin\">@admin</span>");
    assert!(doc.marks.is_empty());
}

#[test]
fn word_processor_html_is_understood() {
    // Google Docs wraps everything in a bold tag that is not bold, and styles spans instead.
    let doc = from_html(
        "<html><body><!--StartFragment--><b style=\"font-weight:normal;\" id=\"docs-internal\">\
         <span style=\"font-weight:700\">Strong</span> <span style=\"font-style:italic\">slanted</span> \
         <span style=\"text-decoration:line-through\">struck</span></b><!--EndFragment--></body></html>",
    );
    assert_eq!(doc.text, "Strong slanted struck");
    assert_eq!(doc.marks, [(0..6, Mark::Bold), (7..14, Mark::Italic), (15..21, Mark::Strike)]);
    // Entities, whitespace and tasks.
    let doc = from_html(
        "<ul><li><input type=checkbox checked> fish &amp;   chips</li><li><input type=\"checkbox\">tea&nbsp;&#x1F375;</li></ul>",
    );
    assert_eq!(doc.text, "fish & chips\ntea\u{A0}\u{1F375}");
    assert_eq!(doc.blocks, [Block::Task { done: true, level: 0 }, Block::Task { done: false, level: 0 }]);
}

#[test]
fn copied_html_reads_back_the_same() {
    let doc = RichDoc {
        text: "Title\nsay <hi> & bye\none\ntwo\nfix it\nquoted\ncode <x>\nmore".into(),
        marks: vec![(10..14, Mark::Bold), (17..20, Mark::Link("https://example.org/?a=1&b=\"2\"".into()))],
        blocks: vec![
            Block::Heading(1),
            Block::Paragraph,
            Block::Numbered(0),
            Block::Bullet(1),
            Block::Task { done: true, level: 0 },
            Block::Quote,
            Block::Code,
            Block::Code,
        ],
    }
    .normalized();
    let html = to_html(&doc);
    assert!(html.contains("&lt;hi&gt;"), "{html}");
    assert!(!html.contains("<hi>"), "{html}");
    assert_eq!(from_html(&html), doc, "{html}");
}

#[test]
fn broken_html_still_gives_its_text() {
    let doc = from_html("<p>a < b and c > d <b>unclosed <i>nest</p>tail");
    assert_eq!(doc.text, "a < b and c > d unclosed nest\ntail");
    assert!(doc.marks.iter().any(|(_, m)| *m == Mark::Bold));
    assert_eq!(from_html("").text, "");
    assert_eq!(from_html("<script>never ends").text, "");
}
