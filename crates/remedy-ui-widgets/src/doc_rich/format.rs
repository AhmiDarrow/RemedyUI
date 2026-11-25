//! Formats a rich editor saves to and loads from. Apps pick one ([`RichFormat`]): Markdown
//! for chat and wikis ([`MarkdownFormat`]), or their own.
//!
//! Markdown keeps everything a [`RichDoc`] holds: marks, links, headings, nested lists, tasks,
//! quotes and code. A mention is written as a link to `mention:<id>` (`[@Alex](mention:u1)`),
//! so it stays ordinary Markdown that other tools show as its text.

use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use remedy_ui_text::{Block, Mark, RichDoc, SOFT_BREAK};

/// A way to save and load rich documents.
pub trait RichFormat {
    /// `doc` as text in this format.
    fn write(&self, doc: &RichDoc) -> String;
    /// The document `text` holds.
    fn read(&self, text: &str) -> RichDoc;
}

/// Markdown: CommonMark with strikethrough and task lists (tables are not read as tables).
#[derive(Clone, Copy, Debug, Default)]
pub struct MarkdownFormat;

/// The scheme mention links use.
pub const MENTION_SCHEME: &str = "mention:";

impl RichFormat for MarkdownFormat {
    fn write(&self, doc: &RichDoc) -> String {
        write_markdown(doc)
    }

    fn read(&self, text: &str) -> RichDoc {
        read_markdown(text)
    }
}

// ---------- reading ----------

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    List(Option<u64>),
    Item(Option<bool>),
    Quote,
}

struct Reader {
    text: String,
    marks: Vec<(Range<usize>, Mark)>,
    blocks: Vec<Block>,
    ctx: Vec<Ctx>,
    open: Vec<(usize, Mark)>,
    heading: Option<u8>,
    code: bool,
    /// A code block's text, gathered until it ends.
    code_text: Option<String>,
    line_open: bool,
}

impl Reader {
    fn block(&self) -> Block {
        if self.code {
            return Block::Code;
        }
        if let Some(h) = self.heading {
            return Block::Heading(h);
        }
        let lists = self.ctx.iter().filter(|c| matches!(c, Ctx::List(_))).count();
        let level =
            u8::try_from(lists.saturating_sub(1)).unwrap_or(u8::MAX).min(remedy_ui_text::max_list_level());
        let list = self.ctx.iter().rev().find_map(|c| match c {
            Ctx::List(o) => Some(*o),
            _ => None,
        });
        let task = self.ctx.iter().rev().find_map(|c| match c {
            Ctx::Item(t) => Some(*t),
            _ => None,
        });
        match (list, task) {
            (_, Some(Some(done))) => Block::Task { done, level },
            (Some(Some(_)), _) => Block::Numbered(level),
            (Some(None), _) => Block::Bullet(level),
            _ if self.ctx.contains(&Ctx::Quote) => Block::Quote,
            _ => Block::Paragraph,
        }
    }

    /// Starts a new line (unless the current one is still empty).
    fn new_line(&mut self) {
        if self.line_open {
            self.text.push('\n');
            self.blocks.push(Block::Paragraph);
            self.line_open = false;
        }
    }

    /// Opens the current line as the block the context says, even with nothing in it yet.
    fn open_line(&mut self) {
        if !self.line_open {
            let b = self.block();
            match self.blocks.last_mut() {
                Some(last) => *last = b,
                None => self.blocks.push(b),
            }
            self.line_open = true;
        }
    }

    fn push(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.open_line();
        self.text.push_str(s);
    }
}

fn read_markdown(source: &str) -> RichDoc {
    let mut r = Reader {
        text: String::new(),
        marks: Vec::new(),
        blocks: vec![Block::Paragraph],
        ctx: Vec::new(),
        open: Vec::new(),
        heading: None,
        code: false,
        code_text: None,
        line_open: false,
    };
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(source, options) {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => r.new_line(),
                Tag::Heading { level, .. } => {
                    r.new_line();
                    r.heading = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    });
                }
                Tag::BlockQuote(_) => {
                    r.new_line();
                    r.ctx.push(Ctx::Quote);
                }
                Tag::CodeBlock(_) => {
                    r.new_line();
                    r.code = true;
                    r.code_text = Some(String::new());
                }
                Tag::List(start) => {
                    r.new_line();
                    r.ctx.push(Ctx::List(start));
                }
                Tag::Item => {
                    r.new_line();
                    r.ctx.push(Ctx::Item(None));
                }
                Tag::Emphasis => r.open.push((r.text.len(), Mark::Italic)),
                Tag::Strong => r.open.push((r.text.len(), Mark::Bold)),
                Tag::Strikethrough => r.open.push((r.text.len(), Mark::Strike)),
                Tag::Link { dest_url, .. } => {
                    let mark = match dest_url.strip_prefix(MENTION_SCHEME) {
                        Some(id) => Mark::Mention(id.to_owned()),
                        None => Mark::Link(dest_url.to_string()),
                    };
                    r.open.push((r.text.len(), mark));
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => r.new_line(),
                TagEnd::Heading(_) => {
                    r.new_line();
                    r.heading = None;
                }
                TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => {
                    r.new_line();
                    let _ = r.ctx.pop();
                }
                TagEnd::CodeBlock => {
                    // Code keeps its lines, empty ones too: each is a code line.
                    let code = r.code_text.take().unwrap_or_default();
                    let code = code.strip_suffix('\n').unwrap_or(&code);
                    for (k, l) in code.split('\n').enumerate() {
                        if k > 0 {
                            r.new_line();
                        }
                        r.open_line();
                        r.push(l);
                    }
                    r.code = false;
                    r.new_line();
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    if let Some((start, m)) = r.open.pop() {
                        let end = r.text.len();
                        r.marks.push((start..end, m));
                    }
                }
                _ => {}
            },
            Event::Text(t) => match r.code_text.as_mut() {
                Some(code) => code.push_str(&t),
                None => r.push(&t),
            },
            Event::Code(t) => {
                let start = r.text.len();
                r.push(&t);
                r.marks.push((start..r.text.len(), Mark::Code));
            }
            Event::SoftBreak => r.push(" "),
            Event::HardBreak => {
                let mut b = [0u8; 4];
                r.push(SOFT_BREAK.encode_utf8(&mut b));
            }
            Event::TaskListMarker(done) => {
                if let Some(Ctx::Item(t)) = r.ctx.last_mut() {
                    *t = Some(done);
                }
            }
            Event::Html(t) | Event::InlineHtml(t) => r.push(t.trim_end_matches('\n')),
            Event::Rule => {
                r.new_line();
                r.push("---");
                r.new_line();
            }
            Event::FootnoteReference(t) | Event::InlineMath(t) | Event::DisplayMath(t) => r.push(&t),
        }
    }
    if !r.line_open && r.text.ends_with('\n') {
        r.text.pop();
        r.blocks.pop();
    }
    RichDoc { text: r.text, marks: r.marks, blocks: r.blocks }.normalized()
}

// ---------- writing ----------

/// `s` with what would mean something in Markdown escaped: emphasis, code, links, HTML and
/// entities anywhere, and at a line's start what would make a heading, list or quote.
fn escape(s: &str, at_line_start: bool, out: &mut String) {
    let digits = if at_line_start { s.chars().take_while(char::is_ascii_digit).count() } else { 0 };
    for (i, c) in s.chars().enumerate() {
        let special = matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '|' | '&');
        let starts_block = at_line_start && i == 0 && matches!(c, '#' | '-' | '+' | '=');
        // "1." or "1)" at a line's start would start a numbered list.
        let numbers = digits > 0 && i == digits && matches!(c, '.' | ')');
        if special || starts_block || numbers {
            out.push('\\');
        }
        out.push(c);
    }
}

/// A code span's text between enough backticks.
fn code_span(s: &str) -> String {
    let longest = s.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if s.starts_with('`') || s.ends_with('`') { " " } else { "" };
    format!("{fence}{pad}{s}{pad}{fence}")
}

fn opener(m: &Mark) -> &'static str {
    match m {
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Strike => "~~",
        Mark::Link(_) | Mark::Mention(_) => "[",
        Mark::Code => "",
    }
}

fn closer(m: &Mark) -> String {
    match m {
        Mark::Bold => "**".into(),
        Mark::Italic => "*".into(),
        Mark::Strike => "~~".into(),
        Mark::Link(url) => format!("](<{}>)", url.replace(['<', '>'], "")),
        Mark::Mention(id) => format!("]({MENTION_SCHEME}{})", id.replace([')', ' '], "")),
        Mark::Code => String::new(),
    }
}

/// One line's text with its marks as Markdown.
fn inline(doc: &RichDoc, line: Range<usize>, out: &mut String) {
    let text = &doc.text;
    // Emphasis cannot start or end on a space, so marks give up their edge spaces.
    let mut marks: Vec<(Range<usize>, Mark)> = doc
        .marks
        .iter()
        .filter(|(r, _)| r.start < line.end && r.end > line.start)
        .map(|(r, m)| (r.start.max(line.start)..r.end.min(line.end), m.clone()))
        .map(|(mut r, m)| {
            if matches!(m, Mark::Bold | Mark::Italic | Mark::Strike) {
                let piece = text.get(r.clone()).unwrap_or("");
                r.start += piece.len() - piece.trim_start().len();
                r.end -= piece.len() - piece.trim_end().len();
            }
            (r, m)
        })
        .filter(|(r, _)| r.start < r.end)
        .collect();
    // Links outermost, then mentions, bold, italic, strike; code spans are atoms.
    let rank = |m: &Mark| match m {
        Mark::Link(_) => 0,
        Mark::Mention(_) => 1,
        Mark::Bold => 2,
        Mark::Italic => 3,
        Mark::Strike => 4,
        Mark::Code => 5,
    };
    marks.sort_by_key(|(r, m)| (r.start, rank(m)));
    let mut cuts: Vec<usize> = vec![line.start, line.end];
    for (r, _) in &marks {
        cuts.push(r.start);
        cuts.push(r.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut open: Vec<Mark> = Vec::new();
    for w in cuts.windows(2) {
        let &[a, b] = w else { continue };
        let want: Vec<Mark> = {
            let mut v: Vec<Mark> =
                marks.iter().filter(|(r, _)| r.start <= a && b <= r.end).map(|(_, m)| m.clone()).collect();
            v.sort_by_key(rank);
            v
        };
        // Close what ends here (and everything opened after it), then open what starts.
        if let Some(i) = open.iter().position(|m| !want.contains(m)) {
            for m in open.drain(i..).rev() {
                out.push_str(&closer(&m));
            }
        }
        let opening: Vec<Mark> =
            want.iter().filter(|m| !open.contains(m) && **m != Mark::Code).cloned().collect();
        for m in opening {
            out.push_str(opener(&m));
            open.push(m);
        }
        let piece = text.get(a..b).unwrap_or("");
        let piece = piece.replace(SOFT_BREAK, "\n");
        if want.contains(&Mark::Code) {
            out.push_str(&code_span(&piece));
        } else {
            let mut first = true;
            for (k, part) in piece.split('\n').enumerate() {
                if k > 0 {
                    // A line break inside the block: a hard break.
                    out.push_str("\\\n");
                    first = false;
                }
                escape(part, a == line.start && first && k == 0, out);
            }
        }
    }
    for m in open.drain(..).rev() {
        out.push_str(&closer(&m));
    }
}

fn write_markdown(doc: &RichDoc) -> String {
    let mut out = String::new();
    let mut numbers = vec![0u32; usize::from(remedy_ui_text::max_list_level()) + 1];
    let mut prev: Option<Block> = None;
    let lines = doc.lines();
    let mut i = 0;
    while i < lines {
        let block = doc.blocks.get(i).copied().unwrap_or_default();
        let range = doc.line_range(i);
        // Blocks are separated by a blank line, except lines of one list, quote or code.
        if let Some(p) = prev {
            out.push_str(match (p, block) {
                (p, b) if p.is_list() && b.is_list() => "\n",
                // Lines of one quote: separate paragraphs inside it.
                (Block::Quote, Block::Quote) => "\n>\n",
                _ => "\n\n",
            });
        }
        match block {
            Block::Code => {
                // Lines of code until the block ends, inside a fence longer than any in them.
                let mut end = i;
                while doc.blocks.get(end + 1) == Some(&Block::Code) {
                    end += 1;
                }
                let body: Vec<&str> =
                    (i..=end).map(|k| doc.text.get(doc.line_range(k)).unwrap_or("")).collect();
                let longest =
                    body.iter().flat_map(|l| l.split(|c| c != '`')).map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(longest.max(2) + 1);
                out.push_str(&fence);
                out.push('\n');
                out.push_str(&body.join("\n"));
                out.push('\n');
                out.push_str(&fence);
                prev = Some(Block::Code);
                i = end + 1;
                continue;
            }
            Block::Heading(n) => {
                out.push_str(&"#".repeat(usize::from(n.clamp(1, 6))));
                out.push(' ');
                inline(doc, range, &mut out);
            }
            Block::Quote => {
                out.push_str("> ");
                inline(doc, range, &mut out);
            }
            Block::Bullet(l) | Block::Numbered(l) | Block::Task { level: l, .. } => {
                let level = usize::from(l);
                if !prev.is_some_and(Block::is_list) {
                    numbers.fill(0);
                }
                for deeper in numbers.iter_mut().skip(level + 1) {
                    *deeper = 0;
                }
                out.push_str(&"    ".repeat(level));
                match block {
                    Block::Numbered(_) => {
                        let n = numbers.get_mut(level).map_or(1, |n| {
                            *n += 1;
                            *n
                        });
                        out.push_str(&format!("{n}. "));
                    }
                    Block::Task { done, .. } => out.push_str(if done { "- [x] " } else { "- [ ] " }),
                    _ => out.push_str("- "),
                }
                let mut item = String::new();
                inline(doc, range, &mut item);
                // A hard break's next line keeps the item's indentation.
                out.push_str(&item.replace("\\\n", &format!("\\\n{}", "    ".repeat(level + 1))));
            }
            Block::Paragraph => {
                if !range.is_empty() {
                    inline(doc, range, &mut out);
                }
            }
        }
        prev = Some(block);
        i += 1;
    }
    out
}
