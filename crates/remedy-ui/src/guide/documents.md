# Documents: pages, annotations, code and rich text

This guide covers the document packs:

- paged documents (PDF readers, e-books, anything made of pages): the app supplies pages,
  the engine shows them with deep zoom, selection, search, thumbnails and annotations
  ([`crate::pages`], feature `pages`)
- code: syntax colors from the theme, a code view with folding, a code editor with several
  cursors, and diffs ([`crate::code`], feature `code`)
- rich text: Markdown shown as themed text, a rich text editor that saves Markdown, and a
  find bar ([`crate::rich`], feature `rich`)

## Paged documents

The engine never parses a document. The app wraps its PDF library or e-book engine in a
[`crate::pages::PageSource`]:

- [`page_count`](crate::pages::PageSource::page_count) and
  [`page_size`](crate::pages::PageSource::page_size) (in points, 1/72 inch) are called on
  the UI thread while laying out, so they must be quick;
- [`render`](crate::pages::PageSource::render) draws a region of a page at a scale into
  pixels. It runs on page workers, never on the UI thread, and should give up early when
  its [`crate::pages::Cancel`] says the tile is no longer wanted;
- [`text`](crate::pages::PageSource::text), optional, gives the page's text as positioned
  runs, for selection, copying, search and screen readers.

[`crate::pages::PageView`] holds the view's state (zoom, fit, spread, rotation, the current
page, the selection, search and annotations), and [`crate::pages::page_view`] shows it.
[`crate::pages::thumbnails`] is a rail of page thumbnails for the same view: a strip of its
own width that fills the height it is given. The page view fills the space it is given too;
neither takes its size from the document, so both sit in any row or column as they are.

`render` returns a [`crate::paint::Image`] (premultiplied RGBA pixels). Here is a source of three blank pages with one line of text each:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::core::{Rect, Size};
use remedy_ui::pages::{
    Cancel, PageSource, PageText, PageView, RenderError, RenderRequest, TextRun, page_view,
};
use remedy_ui::paint::Image;
use std::sync::Arc;
use std::time::Duration;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

struct Report;

impl PageSource for Report {
    fn page_count(&self) -> usize {
        3
    }
    fn page_size(&self, _page: usize) -> Size {
        Size::new(612.0, 792.0) // US Letter
    }
    fn render(&self, req: &RenderRequest, cancel: &Cancel) -> Result<Image, RenderError> {
        if cancel.is_cancelled() {
            return Err(RenderError::Cancelled);
        }
        // A real source draws the page here; this one is white paper.
        let bytes = vec![255u8; (req.width * req.height * 4) as usize];
        Ok(Image::new(req.width, req.height, Arc::from(bytes)))
    }
    fn text(&self, page: usize) -> Option<PageText> {
        let line = format!("Quarterly report, page {}", page + 1);
        let run = TextRun::new(line, Rect::new(72.0, 72.0, 300.0, 86.0));
        Some(PageText { runs: vec![run] })
    }
}

let view = PageView::new(Arc::new(Report));
ui.root().child(page_view(&ui, &view, "Report.pdf"));
# app.frames(2);
assert_eq!(view.page_count(), 3);

view.go_to_page(2);
assert_eq!(view.current_page().get(), 2);

// Search runs on the page workers; hits arrive in `search_status`.
view.find("quarterly");
for _ in 0..400 {
    app.frames(1);
    if view.search_status().get().done {
        break;
    }
    std::thread::sleep(Duration::from_millis(5));
}
assert_eq!(view.search_status().get().matches, 3, "once on every page");

// Screen readers meet a document named for the file.
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Report.pdf")));
```

### Zoom, fit and layout

[`PageView::set_zoom`](crate::pages::PageView::set_zoom) takes any zoom from
[`MIN_ZOOM`](crate::pages::MIN_ZOOM) (1%) to [`MAX_ZOOM`](crate::pages::MAX_ZOOM) (6400%),
around a point or the middle of the view. [`PageView::set_fit`](crate::pages::PageView::set_fit)
fits the width or the page ([`crate::pages::Fit`]), and the zoom follows the window as it
resizes. [`PageView::set_spread`](crate::pages::PageView::set_spread) shows one page or two
per row ([`crate::pages::Spread`], with or without a cover), and
[`PageView::set_rotation`](crate::pages::PageView::set_rotation) turns pages in quarter turns
([`crate::pages::Rotation`]).

People zoom with a pinch, Ctrl+wheel and Ctrl+Plus, Minus and 0, around the pointer. While a
zoom or scroll moves, the compositor scales and moves the view's picture and old tiles stand
in while new ones render, so the UI thread does almost nothing per frame. When it settles
([`SETTLE`](crate::pages::SETTLE)) the view redraws at the new zoom. Only tiles on screen
render, nearest the middle first, and work that scrolled away is cancelled. The tile cache
holds a share of the machine's memory and lets go under memory pressure.

The keyboard works throughout: the arrows, Page Up and Down, Space, Home and End move
through the document, and the rail answers the same keys.

### Selection and copying

Dragging selects text across runs and pages, Shift extends, Ctrl+A selects the page and
Ctrl+C copies. [`PageView::select`](crate::pages::PageView::select),
[`PageView::selected_text`](crate::pages::PageView::selected_text) and
[`PageView::copy`](crate::pages::PageView::copy) do the same from code, with places given as
[`crate::pages::TextPos`] (page, run, characters into the run).

## Annotations

The annotation layer draws marks inside each page's transform, in page points, so they stay
exact at every zoom and rotation. [`crate::pages::AnnotationKind`] covers highlights,
underlines, strikeouts and squiggles made from the text selection (carrying the words they
mark), freehand ink, rectangles, ellipses, lines and arrows ([`crate::pages::Shape`]), notes
and text boxes.

- People pick a [`crate::pages::Tool`] with [`PageView::set_tool`](crate::pages::PageView::set_tool):
  a text tool marks the selected text; ink, shapes, notes and text boxes are drawn by
  dragging or pressing. [`PageView::set_annotation_style`](crate::pages::PageView::set_annotation_style)
  sets the color and widths of new ones.
- With the select tool, pressing an annotation selects it; dragging moves it; the arrows
  nudge it; `]` and `[` move through annotations in reading order; Delete removes it; Enter
  or double-click opens it, which calls the app's
  [`PageView::on_open_annotation`](crate::pages::PageView::on_open_annotation) to show its
  comment.
- Every edit goes into the window's undo history.
- The app loads its annotations with
  [`PageView::set_annotations`](crate::pages::PageView::set_annotations) (not an edit) and
  saves what [`PageView::annotations`](crate::pages::PageView::annotations) holds. An
  annotation's `key` carries the app's own identifier (a PDF object number) through edits.
- Screen readers hear each page's annotations with the page
  ([`Annotation::describe`](crate::pages::Annotation::describe)).

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::core::{Color, Point, Size};
use remedy_ui::pages::{
    Annotation, AnnotationKind, Cancel, PageSource, PageView, RenderError, RenderRequest,
    Shape, page_view,
};
use remedy_ui::paint::Image;
use std::sync::Arc;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };
# struct Paper;
# impl PageSource for Paper {
#     fn page_count(&self) -> usize { 2 }
#     fn page_size(&self, _: usize) -> Size { Size::new(612.0, 792.0) }
#     fn render(&self, r: &RenderRequest, _: &Cancel) -> Result<Image, RenderError> {
#         Ok(Image::new(r.width, r.height, Arc::from(vec![255u8; (r.width * r.height * 4) as usize])))
#     }
# }

let view = PageView::new(Arc::new(Paper));
ui.root().child(page_view(&ui, &view, "Plan.pdf"));
# app.frames(1);

// The app opens the document with the annotations it saved: not an edit.
let mut note = Annotation::new(0, AnnotationKind::Note { at: Point::new(500.0, 60.0) }, Color::rgb8(255, 214, 0));
note.comment = "Check these figures".into();
note.key = Some("obj 12".into());
view.set_annotations(vec![note]);
assert!(ui.next_undo().is_none());

// Someone draws an arrow: an edit, undoable.
let arrow = AnnotationKind::Shape {
    shape: Shape::Arrow,
    from: Point::new(100.0, 400.0),
    to: Point::new(300.0, 300.0),
    width: 2.0,
};
let id = view.add_annotation(Annotation::new(1, arrow, Color::rgb8(220, 40, 40)));
assert_eq!(view.annotations().get().len(), 2);
assert_eq!(ui.next_undo().map(|h| h.label).as_deref(), Some("Add arrow"));

view.update_annotation(id, |a| a.comment = "This one".into());
assert_eq!(view.annotation(id).map(|a| a.describe()).as_deref(), Some("Arrow, comment: This one"));

assert!(ui.undo_change());
assert!(ui.undo_change());
assert_eq!(view.annotations().get().len(), 1, "back to what was opened");

// What the app saves keeps its own keys.
let saved = view.annotations().get();
assert_eq!(saved.first().and_then(|a| a.key.as_deref()), Some("obj 12"));
```

## Code

[`crate::code::highlight`] colors code by Sublime Text grammars, read by syntect with a
pure-Rust regex engine. Each piece of code gets a [`crate::code::Kind`] (keyword, string,
comment...) and each kind a color from the theme ([`crate::code::kind_color`]), so a theme
and a person's edits to it color code like everything else.
[`crate::code::add_grammar`] adds a grammar of the app's own, and
[`crate::code::languages`] lists what is known. A language is named or given by file
extension (`"Rust"` or `"rs"`).

```
use remedy_ui::code::{Kind, highlight, knows};

assert!(knows("rs"));
let lines = highlight("fn main() {\n    // hi\n}", "rs");
assert_eq!(lines.len(), 3);
let first = lines.first().map(Vec::as_slice).unwrap_or_default();
assert!(first.iter().any(|(range, kind)| *kind == Kind::Keyword && *range == (0..2)), "`fn`");
let second = lines.get(1).map(Vec::as_slice).unwrap_or_default();
assert!(second.iter().any(|(_, kind)| *kind == Kind::Comment));
```

Three elements show code:

- [`crate::code::code_view`]: read-only code from a `Signal<String>`, re-colored when it
  changes, with line numbers and folding by indentation ([`crate::code::CodeOptions`]). It is
  a virtual list, so a long file costs only the lines on screen.
- [`crate::code::code_editor`]: an editable code field with several cursors, indentation that
  follows the code, bracket pairs and grouped undo, colored as it is typed. It returns the
  element and its [`crate::TextField`], for the text and changes.
- [`crate::code::diff_view`]: one text turned into another, removed and added lines marked
  (and named for screen readers). [`crate::code::diff_lines`] is the diff on its own (Myers'
  algorithm).

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::code::{CodeOptions, DiffLine, code_editor, code_view, diff_lines, diff_view};
use remedy_ui::reactive::Signal;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let rust = CodeOptions::default().language("rs".into());
let source = Signal::new("fn add(a: i32, b: i32) -> i32 {\n    a + b\n}".to_owned());
let (editor, field) = code_editor(&ui, "main.rs", "let x = 1;", rust.clone());
ui.root()
    .child(code_view(&ui, "lib.rs", source, rust))
    .child(editor)
    .child(diff_view(&ui, "Changes", "a\nb\nc", "a\nB\nc", "txt"));
# app.frames(1);
assert_eq!(field.value(), "let x = 1;");

assert_eq!(
    diff_lines("a\nb\nc", "a\nB\nc"),
    [
        DiffLine::Same("a".into(), 1, 1),
        DiffLine::Removed("b".into(), 2),
        DiffLine::Added("B".into(), 2),
        DiffLine::Same("c".into(), 3, 3),
    ]
);
let reader = ui.reader_view();
assert!(reader.iter().any(|n| n.name.as_deref() == Some("lib.rs")));
assert!(reader.iter().any(|n| n.name.as_deref() == Some("Changes")));
```

## Markdown

[`crate::rich::markdown`] shows Markdown as elements: headings people can move between,
paragraphs whose links are real links, lists (ordered, unordered, tasks, nested), quotes,
tables, code and rules. Everything follows the theme, and the text can be selected and
copied. Nothing in the source runs: raw HTML shows as the text it is, and a link only calls
the app's `open` callback, which decides what to do with it.
The source is a string, or a signal, memo or closure, like every property: a reply still
arriving rebuilds the Markdown as its text grows. [`crate::rich::plain_text`] gives the text
as it reads, for previews, notifications and search.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::rich::{markdown, plain_text};
use std::cell::RefCell;
use std::rc::Rc;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let source = "# Release notes\n\nSee the [changelog](https://example.org/changes).\n\n- [x] Faster start\n- [ ] Dark mode";
assert_eq!(plain_text("**Bold** and `code`"), "Bold and code");

let opened = Rc::new(RefCell::new(Vec::new()));
let o = opened.clone();
ui.root().child(markdown(&ui, source, move |url| o.borrow_mut().push(url.to_owned())));
# app.frames(1);

let reader = ui.reader_view();
assert!(reader.iter().any(|n| n.name.as_deref() == Some("Release notes")));
let Some(link) = reader.iter().find(|n| n.name.as_deref() == Some("changelog")) else {
    unreachable!("links are real links")
};
assert!(ui.accessibility_action(link.id, remedy_ui::view::A11yAction::Click));
assert_eq!(*opened.borrow(), ["https://example.org/changes"]);

// A reply still arriving: the Markdown follows the signal.
let reply = remedy_ui::reactive::Signal::new(String::from("Thinking"));
ui.root().child(markdown(&ui, reply, |_| {}));
reply.set("## Done\n\nHere it is.".into());
# app.frames(1);
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Done")));
```

## The rich text editor

[`crate::rich::rich_editor`] edits formatted text: marks, links, headings, lists, tasks,
quotes and code, with an optional formatting toolbar and mentions suggested as people type
"@" ([`crate::rich::RichEditorOptions`], [`crate::rich::MentionCandidate`]). It returns its
element and its [`crate::TextField`]. The document is a
[`crate::text::RichDoc`]; [`crate::rich::MarkdownFormat`] (a [`crate::rich::RichFormat`])
saves it to Markdown and reads it back, with mentions written as `mention:` links so the file
stays ordinary Markdown.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::rich::{MarkdownFormat, RichEditorOptions, RichFormat, rich_editor};
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let options = RichEditorOptions::default().label("Note".into());
let (editor, field) = rich_editor(&ui, options);
ui.root().child(editor);

// Open a saved note, then save it again.
field.set_doc(MarkdownFormat.read("Buy **milk** and *bread*"));
# app.frames(1);
assert_eq!(field.value(), "Buy milk and bread");
let Some(doc) = field.doc() else { unreachable!("a rich field holds a document") };
assert_eq!(MarkdownFormat.write(&doc), "Buy **milk** and *bread*");
```

For a chat composer, set `submit_on_enter` (Shift+Enter starts a new line) and handle
[`TextField::on_submit`](crate::TextField::on_submit).

## Finding text

[`crate::rich::find_bar`] is a find bar for any text on screen: a field, "3 of 12", previous,
next and close. Its [`crate::rich::FindState`] searches every text inside a scope element (a
document, a chat, a page), marks each match in the theme's mark color, selects the current
one and scrolls it into view. Matching ignores case in every script. Enter and F3 go to the
next match, Shift+Enter and Shift+F3 to the previous one, and Escape closes the bar.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::rich::{FindState, find_bar};
use remedy_ui::widgets as w;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let page = w::column(&ui)
    .child(w::text(&ui, "The fox jumps."))
    .child(w::text(&ui, "A FOX sleeps."));
let find = FindState::new(&page);
let bar = find_bar(&ui, &find, || {});
ui.root().child(bar).child(page);
# app.frames(1);

find.search("fox");
assert_eq!(find.count.get(), 2);
assert_eq!(find.status(), "1 of 2");
find.next();
assert_eq!(find.current.get(), 1);
```

When the content changes while the bar is open,
[`FindState::refresh`](crate::rich::FindState::refresh) searches again and keeps the current
match where it can.
