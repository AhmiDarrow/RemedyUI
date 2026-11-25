//! Markdown shown as RemedyUI elements: headings people can move between, paragraphs whose
//! links are real links, lists (ordered, unordered, tasks, nested), quotes, tables, code and
//! rules. Everything follows the theme, the text can be selected and copied, and nothing in
//! the source runs: raw HTML shows as the text it is, and a link only reaches the app's
//! `open` callback, which decides what to do with it.

use std::rc::Rc;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_core::layout::{Display, Track};
use remedy_ui_text::{RichText, TextStylePatch};
use remedy_ui_view::{El, IntoProp, Prop, Ui};

/// What following a link calls: the link's destination as the source wrote it.
pub type OpenLink = Rc<dyn Fn(&str)>;

/// Shows Markdown `source`: a string set once, or a signal, memo or closure it follows
/// (a reply still arriving, a page being edited), rebuilt when the text changes. Following
/// a link calls `open` with its destination.
///
/// ```
/// # use remedy_ui_view::Ui;
/// # use remedy_ui_core::reactive::Signal;
/// # fn demo(ui: &Ui) {
/// let note = remedy_ui_doc_rich::markdown(ui, "# Notes\n\nPlain *Markdown*.", |_| {});
/// let reply = Signal::new(String::from("Thinking"));
/// let live = remedy_ui_doc_rich::markdown(ui, reply, |_| {});
/// ui.root().child(note).child(live);
/// # }
/// ```
#[track_caller]
pub fn markdown(ui: &Ui, source: impl IntoProp<String>, open: impl Fn(&str) + 'static) -> El {
    let root = ui.el("markdown").column().fill_width();
    let open: OpenLink = Rc::new(open);
    match source.into_prop() {
        Prop::Value(s) => {
            fill(ui, &root, &s, &open);
            root
        }
        Prop::Dynamic(source) => {
            let me = root.clone();
            let ui2 = ui.clone();
            let mut last: Option<String> = None;
            root.bind(move || {
                let s = source();
                if last.as_deref() == Some(s.as_str()) {
                    return;
                }
                me.clear();
                fill(&ui2, &me, &s, &open);
                last = Some(s);
            })
        }
    }
}

/// A heading's id, as web renderers make it: its text in lower case, letters and digits
/// kept, spaces and hyphens as hyphens, everything else dropped ("What's new?" is
/// `whats-new`). It is what a link to `#whats-new` names.
pub fn heading_id(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() || c == '_' {
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() || c == '-' {
            out.push('-');
        }
    }
    out
}

/// The ids of headings in order, with a second "Setup" as `setup-1` and a third as
/// `setup-2`, so every heading has its own.
fn unique_ids<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    texts
        .map(|t| {
            let id = heading_id(t);
            let n = seen.entry(id.clone()).or_insert(0);
            *n += 1;
            if *n == 1 { id } else { format!("{id}-{}", *n - 1) }
        })
        .collect()
}

/// The headings of Markdown `source` in order: level (1 to 6), text and id, for a table of
/// contents whose entries call [`scroll_to_heading`].
pub fn headings(source: &str) -> Vec<(u8, String, String)> {
    let mut found: Vec<(u8, String)> = Vec::new();
    let mut open: Option<(u8, String)> = None;
    for ev in Parser::new_ext(source, options()) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => open = Some((level as u8, String::new())),
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, text)) = open.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => found.extend(open.take()),
            _ => {}
        }
    }
    let ids = unique_ids(found.iter().map(|(_, t)| t.as_str()));
    found.into_iter().zip(ids).map(|((level, text), id)| (level, text, id)).collect()
}

/// Scrolls the heading with `id` ([`heading_id`]; a leading `#` is fine) of a [`markdown`]
/// element into view, in whatever scrolls around it. `false` when it has no such heading.
/// Links inside the document to `#an-id` do this themselves.
pub fn scroll_to_heading(markdown: &El, id: &str) -> bool {
    let id = id.trim_start_matches('#');
    let mut found = Vec::new();
    let mut stack = vec![markdown.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == "heading") {
            found.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    let texts: Vec<String> = found.iter().map(|e| e.text_content().unwrap_or_default()).collect();
    let ids = unique_ids(texts.iter().map(String::as_str));
    match ids.iter().position(|have| have == id) {
        Some(at) => {
            if let Some(e) = found.get(at) {
                e.scroll_into_view();
            }
            true
        }
        None => false,
    }
}

/// The plain text of Markdown `source`, as it reads: for previews, notifications and
/// search.
pub fn plain_text(source: &str) -> String {
    let mut out = String::new();
    for ev in Parser::new_ext(source, options()) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => out.push_str(&t),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak | Event::Rule => out.push('\n'),
            Event::TaskListMarker(done) => out.push_str(if done { "[x] " } else { "[ ] " }),
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock | TagEnd::TableRow,
            ) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => out.push('\t'),
            _ => {}
        }
    }
    out.trim_end().to_owned()
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// A block being built: where its children go and what kind it is.
enum Open {
    /// A container of blocks (the document, a quote, a list item, a table cell's parent).
    Blocks(El),
    /// A list: its element, and the next number for an ordered one.
    List(El, Option<u64>),
    /// A table: its element, the alignments, and whether the head row is being read.
    Table(El, Vec<Alignment>, bool),
    /// A table row (cells go straight into the table's grid).
    Row,
}

/// Inline text being gathered for a paragraph, heading or cell.
struct Inline {
    text: RichText,
    /// Open style spans: where each began and its patch.
    styles: Vec<(usize, TextStylePatch)>,
    /// Links in this text, by id: their destinations.
    links: Vec<String>,
    /// The open link's start and destination.
    link: Option<(usize, String)>,
}

impl Inline {
    fn new() -> Self {
        Inline { text: RichText::new(), styles: Vec::new(), links: Vec::new(), link: None }
    }

    fn push(&mut self, s: &str) {
        self.text.push_text(s);
    }

    fn open(&mut self, patch: TextStylePatch) {
        self.styles.push((self.text.len(), patch));
    }

    fn close(&mut self) {
        if let Some((start, patch)) = self.styles.pop() {
            let end = self.text.len();
            if end > start {
                self.text.add_span(start..end, patch);
            }
        }
    }
}

struct Builder<'a> {
    ui: &'a Ui,
    open_link: &'a OpenLink,
    stack: Vec<Open>,
    inline: Option<Inline>,
    /// The inline text's kind once finished: a heading level, a table cell, or a paragraph.
    heading: Option<u8>,
    cell: Option<(bool, Alignment)>,
    /// A code block's text and language.
    code: Option<(String, String)>,
    /// An image's alt text being gathered, and its destination.
    image: Option<(usize, String)>,
    unit: f32,
}

fn fill(ui: &Ui, root: &El, source: &str, open: &OpenLink) {
    let mut b = Builder {
        ui,
        open_link: open,
        stack: vec![Open::Blocks(root.clone())],
        inline: None,
        heading: None,
        cell: None,
        code: None,
        image: None,
        unit: ui.theme().spacing.unit,
    };
    let _ = root.clone().gap(b.unit * 3.0);
    for ev in Parser::new_ext(source, options()) {
        b.event(ev);
    }
    b.flush();
}

impl Builder<'_> {
    /// Where the next block goes.
    fn container(&self) -> Option<El> {
        self.stack.iter().rev().find_map(|o| match o {
            Open::Blocks(e) => Some(e.clone()),
            _ => None,
        })
    }

    fn add(&self, el: El) {
        if let Some(c) = self.container() {
            let _ = c.child(el);
        }
    }

    fn inline(&mut self) -> &mut Inline {
        self.inline.get_or_insert_with(Inline::new)
    }

    /// Finishes the inline text gathered so far as a paragraph (or the heading or cell it
    /// belongs to).
    fn flush(&mut self) {
        let Some(mut inl) = self.inline.take() else { return };
        while !inl.styles.is_empty() {
            inl.close();
        }
        let text = inl.text;
        let plain = text.as_str().trim().to_owned();
        if plain.is_empty() && self.cell.is_none() {
            return;
        }
        let links = Rc::new(inl.links);
        let open = self.open_link.clone();
        let root = match self.stack.first() {
            Some(Open::Blocks(root)) => Some(root.clone()),
            _ => None,
        };
        let follow = move |id: u64| {
            if let Some(dest) = usize::try_from(id).ok().and_then(|i| links.get(i)) {
                // A link to a heading of this document goes there; anything else is the app's.
                let here =
                    dest.strip_prefix('#').zip(root.as_ref()).is_some_and(|(a, r)| scroll_to_heading(r, a));
                if !here {
                    open(dest);
                }
            }
        };
        let el = match (self.heading, self.cell) {
            (Some(level), _) => {
                let variant = match level {
                    1 => "1",
                    2 => "2",
                    _ => "3",
                };
                self.ui
                    .el("heading")
                    .variant(variant)
                    .text(text)
                    .semantics(Semantics::new(Role::Heading(level)).label(plain))
            }
            (None, Some((head, align))) => {
                let cell =
                    self.ui.el("md_cell").variant(if head { "head" } else { "" }).text(text).semantics(
                        Semantics::new(if head { Role::ColumnHeader } else { Role::Cell }).label(plain),
                    );
                match align {
                    Alignment::Right => cell.layout(|l| l.align_self = Some(remedy_ui_core::layout::Align::End)),
                    Alignment::Center => {
                        cell.layout(|l| l.align_self = Some(remedy_ui_core::layout::Align::Center))
                    }
                    _ => cell,
                }
            }
            (None, None) => self.ui.el("label").text(text),
        };
        let el = el.selectable(true).data_text().on_link(follow);
        match self.stack.last() {
            Some(Open::Row) => {
                if let Some(Open::Table(t, ..)) =
                    self.stack.iter().rev().find(|o| matches!(o, Open::Table(..)))
                {
                    let _ = t.clone().child(el);
                }
            }
            _ => self.add(el),
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        if let Some((code, _)) = self.code.as_mut() {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => self.end_code(),
                _ => {}
            }
            return;
        }
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.inline().push(&t),
            Event::Code(t) => {
                let mono = self.ui.theme().mono_families.clone();
                let i = self.inline();
                i.open({
                    let mut v_ = TextStylePatch::default();
                    v_.families = Some(mono);
                    v_
                });
                i.push(&t);
                i.close();
            }
            // Raw HTML is shown as the text it is; nothing in it runs or styles anything.
            Event::Html(t) | Event::InlineHtml(t) => self.inline().push(&t),
            Event::SoftBreak => self.inline().push(" "),
            Event::HardBreak => self.inline().push("\n"),
            Event::Rule => {
                self.flush();
                self.add(remedy_ui_widgets_core::divider(self.ui, false));
            }
            // A task item starts with its state.
            Event::TaskListMarker(done) => self.inline().push(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.inline().push(&t)
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.flush(),
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Tag::BlockQuote(_) => {
                self.flush();
                let body = self.ui.el("md_quote_body").column().grow(1.0).gap(self.unit * 2.0);
                let quote = self
                    .ui
                    .el("md_quote")
                    .row()
                    .fill_width()
                    .gap(self.unit * 3.0)
                    .child(self.ui.el("md_quote_bar").width(self.unit.max(2.0) * 0.75))
                    .child(body.clone())
                    .semantics({
                        let u = self.ui.clone();
                        move || Semantics::new(Role::Group).label(crate::tr(&u, "markdown-quote"))
                    });
                self.add(quote);
                self.stack.push(Open::Blocks(body));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((String::new(), lang));
            }
            Tag::List(start) => {
                self.flush();
                let list = self
                    .ui
                    .el("md_list")
                    .column()
                    .fill_width()
                    .gap(self.unit)
                    .semantics(Semantics::new(Role::List));
                self.add(list.clone());
                self.stack.push(Open::List(list, start));
            }
            Tag::Item => {
                self.flush();
                let marker = match self.stack.last_mut() {
                    Some(Open::List(_, Some(n))) => {
                        let m = format!("{n}.");
                        *n += 1;
                        m
                    }
                    _ => "\u{2022}".to_owned(),
                };
                let body = self.ui.el("md_item_body").column().grow(1.0).gap(self.unit);
                let mark = self.ui.el("md_marker").text(RichText::plain(marker)).data_text().nowrap();
                let item = self
                    .ui
                    .el("md_item")
                    .row()
                    .fill_width()
                    .gap(self.unit * 2.0)
                    .child(mark)
                    .child(body.clone())
                    .semantics(Semantics::new(Role::ListItem));
                if let Some(Open::List(l, _)) = self.stack.last() {
                    let _ = l.clone().child(item);
                }
                self.stack.push(Open::Blocks(body));
            }
            Tag::Table(aligns) => {
                self.flush();
                let n = aligns.len().max(1);
                let table = self
                    .ui
                    .el("md_table")
                    .layout(|l| {
                        l.display = Display::Grid;
                        l.columns =
                            vec![Track::MinMax(Box::new(Track::MinContent), Box::new(Track::Fr(1.0))); n];
                    })
                    .fill_width()
                    .semantics(Semantics::new(Role::Table));
                let scroller = self.ui.el("md_table_scroll").fill_width().scroll_x().child(table.clone());
                self.add(scroller);
                self.stack.push(Open::Table(table, aligns, false));
            }
            Tag::TableHead => {
                if let Some(Open::Table(_, _, head)) = self.stack.last_mut() {
                    *head = true;
                }
                self.stack.push(Open::Row);
            }
            Tag::TableRow => self.stack.push(Open::Row),
            Tag::TableCell => {
                let (head, align) = self
                    .stack
                    .iter()
                    .rev()
                    .find_map(|o| match o {
                        Open::Table(t, a, h) => {
                            let col = t.child_elements().len() % a.len().max(1);
                            Some((*h, a.get(col).copied().unwrap_or(Alignment::None)))
                        }
                        _ => None,
                    })
                    .unwrap_or((false, Alignment::None));
                self.cell = Some((head, align));
                self.inline = Some(Inline::new());
            }
            Tag::Emphasis => self.inline().open(TextStylePatch::italic()),
            Tag::Strong => self.inline().open(TextStylePatch::bold()),
            Tag::Strikethrough => self.inline().open(TextStylePatch::strikethrough()),
            Tag::Link { dest_url, .. } => {
                let i = self.inline();
                let at = i.text.len();
                i.link = Some((at, dest_url.to_string()));
            }
            Tag::Image { dest_url, .. } => {
                // The app loads images; the document shows what the picture is and links to it.
                let i = self.inline();
                let at = i.text.len();
                i.push("[");
                self.image = Some((at, dest_url.to_string()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) | TagEnd::Item => {
                self.flush();
                let _ = self.stack.pop();
            }
            TagEnd::List(_) | TagEnd::Table => {
                self.flush();
                let _ = self.stack.pop();
            }
            TagEnd::TableHead => {
                let _ = self.stack.pop();
                if let Some(Open::Table(_, _, head)) = self.stack.last_mut() {
                    *head = false;
                }
            }
            TagEnd::TableRow => {
                let _ = self.stack.pop();
            }
            TagEnd::TableCell => {
                self.flush();
                self.cell = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.inline().close(),
            TagEnd::Link => self.end_link(),
            TagEnd::Image => {
                if let Some((start, dest)) = self.image.take() {
                    let i = self.inline();
                    i.push("]");
                    let id = i.links.len() as u64;
                    i.links.push(dest);
                    let end = i.text.len();
                    i.text.add_span(start..end, TextStylePatch::link(id));
                }
            }
            _ => {}
        }
    }

    fn end_link(&mut self) {
        let accent = self.ui.theme().colors.accent;
        let bg = self.ui.theme().colors.bg;
        let color = remedy_ui_core::style::ensure_contrast(accent, bg, 4.5);
        let i = self.inline();
        let Some((start, dest)) = i.link.take() else { return };
        let end = i.text.len();
        if end <= start {
            return;
        }
        let id = i.links.len() as u64;
        i.links.push(dest);
        i.text.add_span(start..end, {
            let mut v_ = TextStylePatch::link(id);
            v_.color = Some(color);
            v_
        });
    }

    fn end_code(&mut self) {
        let Some((code, lang)) = self.code.take() else { return };
        let code = code.strip_suffix('\n').unwrap_or(&code).to_owned();
        let u = self.ui.clone();
        let label = move || {
            if lang.is_empty() {
                crate::tr(&u, "markdown-code")
            } else {
                crate::trf(&u, "markdown-code-language", &[("language", lang.as_str().into())])
            }
        };
        let text = self.ui.el("md_code").text(RichText::plain(code)).data_text().selectable(true).nowrap();
        let block = self
            .ui
            .el("md_code_scroll")
            .fill_width()
            .scroll_x()
            .padding(Insets::ZERO)
            .child(text)
            .semantics(move || Semantics::new(Role::Group).label(label()));
        self.add(block);
    }
}
