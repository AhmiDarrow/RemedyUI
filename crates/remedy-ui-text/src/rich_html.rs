//! Rich documents to and from HTML, for the clipboard.
//!
//! [`from_html`] reads what other apps put on the clipboard (a web page, a word processor, a
//! mail) and keeps only what a [`RichDoc`] can hold: bold, italic, strikethrough, code, links
//! to web and mail addresses, headings, lists, tasks, quotes and code blocks. Nothing in the
//! HTML runs or survives as markup: scripts, styles and embedded content are dropped with their
//! content, other tags keep only their text, and links to anything but `http`, `https` and
//! `mailto` become plain text. Mentions are never read from HTML (anyone can write the markup);
//! an editor pasting its own copy keeps them by recognizing that copy.
//!
//! [`to_html`] writes a document for other apps to paste.

use std::ops::Range;

use crate::rich_edit::{Block, Mark, RichDoc, SOFT_BREAK};

/// Tags dropped together with everything inside them.
const DROPPED: &[&str] = &[
    "script", "style", "head", "title", "template", "noscript", "iframe", "object", "embed", "svg", "math",
    "select", "textarea", "button",
];

/// Tags with no content and no end tag.
const VOID: &[&str] =
    &["br", "img", "input", "hr", "meta", "link", "wbr", "col", "source", "area", "base", "track", "param"];

/// Tags that start a new line.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "table",
    "tr",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "dl",
    "dt",
    "dd",
    "address",
    "details",
    "summary",
    "hr",
    "form",
    "fieldset",
];

/// A destination a pasted link may keep.
fn safe_link(url: &str) -> Option<String> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|s| lower.starts_with(s)).then(|| u.to_owned())
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let n = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        // Controls other than tab and line breaks, and surrogates, are not text.
        return char::from_u32(n).filter(|c| !c.is_control() || matches!(c, '\t' | '\n'));
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{A0}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "bull" => '•',
        "middot" => '·',
        "times" => '×',
        "divide" => '÷',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "deg" => '°',
        "plusmn" => '±',
        "laquo" => '«',
        "raquo" => '»',
        "shy" => '\u{AD}',
        "zwj" => '\u{200D}',
        "zwnj" => '\u{200C}',
        _ => return None,
    })
}

/// `s` with its character references decoded.
fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        // A name is at most 10 bytes: looking no further keeps a run of `&` from each
        // searching the rest of the text.
        let semi = after.bytes().take(11).position(|b| b == b';');
        let named = semi.and_then(|i| Some((after.get(..i)?, after.get(i + 1..)?)));
        match named.and_then(|(name, tail)| decode_entity(name).map(|c| (c, tail))) {
            Some((c, tail)) => {
                out.push(c);
                rest = tail;
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A tag's attributes, names lowercased, values decoded.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    let at = |it: &mut std::iter::Peekable<std::str::CharIndices<'_>>| it.peek().map_or(s.len(), |(i, _)| *i);
    loop {
        while it.next_if(|(_, c)| c.is_whitespace() || *c == '/').is_some() {}
        let start = at(&mut it);
        while it.next_if(|(_, c)| !c.is_whitespace() && *c != '=' && *c != '/').is_some() {}
        let end = at(&mut it);
        if start == end {
            break;
        }
        let name = s.get(start..end).unwrap_or("").to_ascii_lowercase();
        while it.next_if(|(_, c)| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if it.next_if(|(_, c)| *c == '=').is_some() {
            while it.next_if(|(_, c)| c.is_whitespace()).is_some() {}
            match it.next_if(|(_, c)| *c == '"' || *c == '\'') {
                Some((_, q)) => {
                    let vs = at(&mut it);
                    while it.next_if(|(_, c)| *c != q).is_some() {}
                    value = decode(s.get(vs..at(&mut it)).unwrap_or(""));
                    let _ = it.next();
                }
                None => {
                    let vs = at(&mut it);
                    while it.next_if(|(_, c)| !c.is_whitespace()).is_some() {}
                    value = decode(s.get(vs..at(&mut it)).unwrap_or(""));
                }
            }
        }
        out.push((name, value));
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// The marks an inline `style` attribute asks for, and whether it turns bold off.
fn style_marks(style: &str) -> (Vec<Mark>, bool) {
    let mut marks = Vec::new();
    let mut not_bold = false;
    for decl in style.split(';') {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
        match k.as_str() {
            "font-weight" => {
                let heavy = v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|w| w >= 600);
                if heavy {
                    marks.push(Mark::Bold);
                } else {
                    not_bold = true;
                }
            }
            "font-style" if v.contains("italic") || v.contains("oblique") => marks.push(Mark::Italic),
            "text-decoration" | "text-decoration-line" if v.contains("line-through") => {
                marks.push(Mark::Strike)
            }
            "font-family" if ["mono", "courier", "consolas", "menlo"].iter().any(|f| v.contains(f)) => {
                marks.push(Mark::Code);
            }
            _ => {}
        }
    }
    (marks, not_bold)
}

/// An element open while reading: its tag, the marks it gave and where they started.
struct Open {
    tag: String,
    marks: Vec<Mark>,
    start: usize,
}

/// Block context: lists, quotes, code and headings open around the text.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    List { ordered: bool },
    Item { task: Option<bool> },
    Quote,
    Pre,
    Heading(u8),
}

struct Reader {
    text: String,
    marks: Vec<(Range<usize>, Mark)>,
    blocks: Vec<Block>,
    open: Vec<Open>,
    ctx: Vec<(String, Ctx)>,
    /// The current line has text.
    line_has_text: bool,
}

impl Reader {
    fn block(&self) -> Block {
        let mut level: u8 = 0;
        let mut item: Option<(bool, Option<bool>)> = None;
        let mut out = Block::Paragraph;
        for (_, c) in &self.ctx {
            match c {
                Ctx::List { ordered } => {
                    if item.is_some() {
                        level = level.saturating_add(1);
                    }
                    item = Some((*ordered, None));
                }
                Ctx::Item { task } => {
                    if let Some((o, _)) = item {
                        item = Some((o, *task));
                    } else {
                        item = Some((false, *task));
                    }
                }
                Ctx::Quote => out = Block::Quote,
                Ctx::Pre => out = Block::Code,
                Ctx::Heading(n) => out = Block::Heading(*n),
            }
        }
        if out == Block::Code || matches!(out, Block::Heading(_)) {
            return out;
        }
        match item {
            Some((_, Some(done))) => {
                Block::Task { done, level: level.min(crate::rich_edit::max_list_level()) }
            }
            Some((true, None)) => Block::Numbered(level.min(crate::rich_edit::max_list_level())),
            Some((false, None)) => Block::Bullet(level.min(crate::rich_edit::max_list_level())),
            None => out,
        }
    }

    fn in_pre(&self) -> bool {
        self.ctx.iter().any(|(_, c)| *c == Ctx::Pre)
    }

    /// Starts a new line if the current one has text.
    fn break_line(&mut self) {
        if self.line_has_text {
            self.text.push('\n');
            self.blocks.push(Block::Paragraph);
            self.line_has_text = false;
        }
    }

    fn push_text(&mut self, s: &str) {
        let pre = self.in_pre();
        for c in s.chars() {
            if !pre && c.is_whitespace() && c != '\u{A0}' {
                // Runs of space collapse to one; none at a line's start.
                if !self.line_has_text || self.text.ends_with(' ') {
                    continue;
                }
                self.put(' ');
            } else if pre && c == '\n' {
                self.line_has_text = true;
                self.break_line();
            } else if pre && c == '\r' {
                continue;
            } else {
                self.put(c);
            }
        }
    }

    fn put(&mut self, c: char) {
        if !self.line_has_text {
            let block = self.block();
            if let Some(b) = self.blocks.last_mut() {
                *b = block;
            }
            self.line_has_text = true;
        }
        self.text.push(c);
    }

    fn close(&mut self, tag: &str) {
        let Some(i) = self.open.iter().rposition(|o| o.tag == tag) else { return };
        for o in self.open.drain(i..).collect::<Vec<_>>().into_iter().rev() {
            for m in o.marks {
                self.marks.push((o.start..self.text.len(), m));
            }
        }
    }
}

/// The document an HTML fragment shows, keeping only the formatting a [`RichDoc`] holds.
pub fn from_html(html: &str) -> RichDoc {
    // Word processors wrap the copied part in fragment markers; only that part was copied.
    let html = html
        .split_once("<!--StartFragment-->")
        .and_then(|(_, a)| a.split_once("<!--EndFragment-->"))
        .map_or(html, |(fragment, _)| fragment);
    let mut r = Reader {
        text: String::new(),
        marks: Vec::new(),
        blocks: vec![Block::Paragraph],
        open: Vec::new(),
        ctx: Vec::new(),
        line_has_text: false,
    };
    let mut rest = html;
    // No '>' is left in the text: every further '<' is text, without searching again.
    let mut no_close = false;
    while !rest.is_empty() {
        let Some((text, tail)) = rest.split_once('<') else {
            r.push_text(&decode(rest));
            break;
        };
        if !text.is_empty() {
            r.push_text(&decode(text));
        }
        rest = tail;
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.split_once("-->").map_or("", |(_, t)| t);
            continue;
        }
        if rest.starts_with('!') || rest.starts_with('?') {
            rest = rest.split_once('>').map_or("", |(_, t)| t);
            continue;
        }
        let Some((inner, tail)) = (if no_close { None } else { rest.split_once('>') }) else {
            // A '<' that starts no tag is text.
            no_close = true;
            r.push_text("<");
            continue;
        };
        rest = tail;
        let closing = inner.starts_with('/');
        let body = inner.trim_start_matches('/');
        let split = body.find(|c: char| c.is_ascii_whitespace() || c == '/').unwrap_or(body.len());
        let (name, after_name) = body.split_at(split);
        let tag = name.to_ascii_lowercase();
        if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            // Not a tag after all ("a < b > c"): the text it was.
            r.push_text(&decode(&format!("<{inner}>")));
            continue;
        }
        if closing {
            if BLOCKS.contains(&tag.as_str()) {
                r.break_line();
            }
            if let Some(i) = r.ctx.iter().rposition(|(t, _)| *t == tag) {
                r.ctx.truncate(i);
            }
            r.close(&tag);
            continue;
        }
        if DROPPED.contains(&tag.as_str()) {
            // Skip to the matching end tag, whatever its case (compared in place: a copy of
            // the rest per dropped tag would make many of them quadratic).
            let end = format!("</{tag}");
            rest = rest
                .as_bytes()
                .windows(end.len())
                .position(|w| w.eq_ignore_ascii_case(end.as_bytes()))
                .and_then(|e| rest.get(e..))
                .and_then(|t| t.split_once('>'))
                .map_or("", |(_, t)| t);
            continue;
        }
        let attrs = attributes(after_name);
        if BLOCKS.contains(&tag.as_str()) {
            r.break_line();
        }
        match tag.as_str() {
            "br" => {
                if r.ctx.iter().any(|(_, c)| matches!(c, Ctx::Item { .. })) {
                    r.put(SOFT_BREAK);
                } else {
                    r.line_has_text = true;
                    r.break_line();
                }
                continue;
            }
            "img" => {
                if let Some(alt) = attr(&attrs, "alt").filter(|a| !a.trim().is_empty()) {
                    r.push_text(alt);
                }
                continue;
            }
            "input" => {
                if attr(&attrs, "type").is_some_and(|t| t.eq_ignore_ascii_case("checkbox"))
                    && let Some((_, Ctx::Item { task })) =
                        r.ctx.iter_mut().rev().find(|(_, c)| matches!(c, Ctx::Item { .. }))
                {
                    *task = Some(attr(&attrs, "checked").is_some());
                }
                continue;
            }
            // Table cells side by side, a tab between them.
            "td" | "th" if r.line_has_text => r.put('\t'),
            _ => {}
        }
        if VOID.contains(&tag.as_str()) {
            continue;
        }
        let ctx = match tag.as_str() {
            "ul" => Some(Ctx::List { ordered: false }),
            "ol" => Some(Ctx::List { ordered: true }),
            "li" => Some(Ctx::Item { task: None }),
            "blockquote" => Some(Ctx::Quote),
            "pre" => Some(Ctx::Pre),
            "h1" => Some(Ctx::Heading(1)),
            "h2" => Some(Ctx::Heading(2)),
            "h3" => Some(Ctx::Heading(3)),
            "h4" => Some(Ctx::Heading(4)),
            "h5" => Some(Ctx::Heading(5)),
            "h6" => Some(Ctx::Heading(6)),
            _ => None,
        };
        if let Some(c) = ctx {
            r.ctx.push((tag.clone(), c));
        }
        let (mut marks, not_bold) = attr(&attrs, "style").map(style_marks).unwrap_or_default();
        match tag.as_str() {
            "b" | "strong" if !not_bold => marks.push(Mark::Bold),
            "i" | "em" | "cite" | "dfn" | "var" => marks.push(Mark::Italic),
            "s" | "strike" | "del" => marks.push(Mark::Strike),
            "code" | "kbd" | "samp" | "tt" => {
                if !r.in_pre() {
                    marks.push(Mark::Code);
                }
            }
            "a" => {
                if let Some(url) = attr(&attrs, "href").and_then(safe_link) {
                    marks.push(Mark::Link(url));
                }
            }
            _ => {}
        }
        let start = r.text.len();
        r.open.push(Open { tag, marks, start });
    }
    while let Some(o) = r.open.last() {
        let tag = o.tag.clone();
        r.close(&tag);
    }
    // A trailing empty line from a closing block is not part of the content.
    if !r.line_has_text && r.text.ends_with('\n') {
        r.text.pop();
        r.blocks.pop();
    }
    RichDoc { text: r.text, marks: r.marks, blocks: r.blocks }.normalized()
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            SOFT_BREAK => out.push_str("<br>"),
            c => out.push(c),
        }
    }
}

/// The opening and closing tags for a mark.
fn tags(m: &Mark) -> (String, &'static str) {
    match m {
        Mark::Bold => ("<strong>".into(), "</strong>"),
        Mark::Italic => ("<em>".into(), "</em>"),
        Mark::Strike => ("<s>".into(), "</s>"),
        Mark::Code => ("<code>".into(), "</code>"),
        Mark::Link(url) => {
            let mut h = String::from("<a href=\"");
            escape(url, &mut h);
            h.push_str("\">");
            (h, "</a>")
        }
        Mark::Mention(_) => ("<span class=\"mention\">".into(), "</span>"),
    }
}

/// One line's text with its marks as nested tags.
fn inline(doc: &RichDoc, line: Range<usize>, out: &mut String) {
    // Cut the line where any mark starts or ends; each piece opens the marks covering it.
    let mut cuts: Vec<usize> = vec![line.start, line.end];
    for (r, _) in &doc.marks {
        for p in [r.start, r.end] {
            if p > line.start && p < line.end {
                cuts.push(p);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let order = |m: &Mark| match m {
        Mark::Link(_) => 0,
        Mark::Mention(_) => 1,
        Mark::Bold => 2,
        Mark::Italic => 3,
        Mark::Strike => 4,
        Mark::Code => 5,
    };
    for w in cuts.windows(2) {
        let &[a, b] = w else { continue };
        let Some(piece) = doc.text.get(a..b) else { continue };
        let mut on: Vec<&Mark> =
            doc.marks.iter().filter(|(r, _)| r.start <= a && b <= r.end).map(|(_, m)| m).collect();
        on.sort_by_key(|m| order(m));
        let mut closes = Vec::new();
        for m in &on {
            let (open, close) = tags(m);
            out.push_str(&open);
            closes.push(close);
        }
        escape(piece, out);
        for c in closes.into_iter().rev() {
            out.push_str(c);
        }
    }
}

/// HTML for `doc`, for other apps to paste.
pub fn to_html(doc: &RichDoc) -> String {
    let mut out = String::new();
    // Open lists, innermost last: (ordered, level).
    let mut lists: Vec<(bool, u8)> = Vec::new();
    let mut in_code = false;
    let mut in_quote = false;
    // Every line's range in one pass (asking the document for each walks it from the start).
    let mut start = 0;
    let lines: Vec<Range<usize>> = doc
        .text
        .split('\n')
        .map(|l| {
            let r = start..start + l.len();
            start = r.end + 1;
            r
        })
        .collect();
    for (i, line) in lines.into_iter().enumerate() {
        let block = doc.blocks.get(i).copied().unwrap_or_default();
        if in_code && block != Block::Code {
            out.push_str("</code></pre>");
            in_code = false;
        }
        if in_quote && block != Block::Quote {
            out.push_str("</blockquote>");
            in_quote = false;
        }
        let (want, ordered) = match block {
            Block::Bullet(l) | Block::Task { level: l, .. } => (Some(l), false),
            Block::Numbered(l) => (Some(l), true),
            _ => (None, false),
        };
        // Close lists deeper than this line, or of the other kind at its level.
        while let Some(&(o, l)) = lists.last() {
            match want {
                Some(w) if l < w || (l == w && o == ordered) => break,
                _ => {
                    out.push_str(if o { "</ol>" } else { "</ul>" });
                    lists.pop();
                }
            }
        }
        if let Some(w) = want {
            while lists.last().is_none_or(|&(_, l)| l < w) {
                let level = lists.last().map_or(0, |&(_, l)| l + 1).min(w);
                out.push_str(if ordered && level == w { "<ol>" } else { "<ul>" });
                lists.push((ordered && level == w, level));
                if level == w {
                    break;
                }
            }
        }
        match block {
            Block::Paragraph => {
                out.push_str("<p>");
                inline(doc, line, &mut out);
                out.push_str("</p>");
            }
            Block::Heading(n) => {
                let n = n.clamp(1, 6);
                out.push_str(&format!("<h{n}>"));
                inline(doc, line, &mut out);
                out.push_str(&format!("</h{n}>"));
            }
            Block::Bullet(_) | Block::Numbered(_) => {
                out.push_str("<li>");
                inline(doc, line, &mut out);
                out.push_str("</li>");
            }
            Block::Task { done, .. } => {
                out.push_str(if done {
                    "<li><input type=\"checkbox\" checked disabled> "
                } else {
                    "<li><input type=\"checkbox\" disabled> "
                });
                inline(doc, line, &mut out);
                out.push_str("</li>");
            }
            Block::Quote => {
                if !in_quote {
                    out.push_str("<blockquote>");
                    in_quote = true;
                }
                out.push_str("<p>");
                inline(doc, line, &mut out);
                out.push_str("</p>");
            }
            Block::Code => {
                if in_code {
                    out.push('\n');
                } else {
                    out.push_str("<pre><code>");
                    in_code = true;
                }
                escape(doc.text.get(line).unwrap_or(""), &mut out);
            }
        }
    }
    if in_code {
        out.push_str("</code></pre>");
    }
    if in_quote {
        out.push_str("</blockquote>");
    }
    while let Some((o, _)) = lists.pop() {
        out.push_str(if o { "</ol>" } else { "</ul>" });
    }
    out
}
