//! RemedyUI paged documents: the page view for PDF readers and anything else made of pages.
//!
//! The app supplies its pages through a [`PageSource`]: how many there are, each one's size
//! in points, a callback that draws a region of a page at a scale into pixels, and
//! optionally each page's text as positioned runs. Parsing stays in the app (MuPDF, an e-book
//! engine); the engine never reads a document.
//!
//! - **Virtual layout.** [`PageLayout`] places pages in one continuous column of rows, one
//!   page or a two-page [`Spread`] per row, [`Rotation`] in quarter turns, fitted to the
//!   width or the page ([`Fit`]) or at any zoom from 1% to 6400%. Row positions are a prefix
//!   sum, so a 100,000-page document finds what shows by binary search.
//! - **Tiles off the UI thread.** Only tiles on screen render, at the scale shown, on page
//!   workers (one per core but one), nearest the middle of the view first. Every page shown
//!   gets a small preview first. Work for tiles that scrolled away or zooms that moved on is
//!   cancelled: dropped from the queue, and running renders see [`Cancel`].
//! - **Progressive zoom.** While a pinch, Ctrl+wheel, Ctrl+Plus/Minus or a scroll moves,
//!   the view keeps its picture in a compositor layer of its own and the compositor scales
//!   and moves it: tiles of the old scale stand in while the new scale rasters within the
//!   compositor's per-frame budget (`remedy_ui_engine::compositor::RasterBudget`), so the UI thread
//!   does almost nothing per frame and nothing renders. When the movement settles
//!   ([`SETTLE`]), or the picture no longer covers the view, the view redraws at the new
//!   zoom: page tiles already rendered at other scales are drawn scaled in place of missing
//!   ones (the nearest scale that covers them, else the page's preview) while the exact
//!   scale renders, so no tile shows blank once its preview exists.
//! - **Memory.** The tile cache holds a share of the machine's memory and never less than
//!   three screens of tiles; least recently drawn tiles go first, and memory pressure
//!   (`remedy_ui_engine::view::device`) drops everything not on screen.
//! - **Text.** Selection by dragging (Shift extends), Ctrl+A for the page, Ctrl+C to copy,
//!   search across every page off the UI thread with hits highlighted
//!   ([`PageView::find`]).
//! - **Navigation and access.** [`PageView::go_to_page`], a [`thumbnails`] rail, the
//!   keyboard (arrows, Page Up/Down, Space, Home/End, Ctrl+Plus/Minus/0), touch (drag to
//!   pan, two fingers to pinch) and screen readers (the view is a document whose value is
//!   the page and zoom; the current page's text is its child).
//!
//! ```no_run
//! use std::sync::Arc;
//! use remedy_ui_widgets::doc_pages::*;
//! # fn demo(ui: &remedy_ui_engine::view::Ui, source: Arc<dyn PageSource>) {
//! let view = PageView::new(source);
//! let shown = page_view(ui, &view, "Report.pdf");
//! let rail = thumbnails(ui, &view);
//! view.set_zoom(2.0, None);
//! view.go_to_page(41);
//! # let _ = (shown, rail);
//! # }
//! ```

mod annot;
mod input;
mod layout;
mod raster;
mod source;
mod text;
mod view;

pub use annot::{Annotation, AnnotationKind, AnnotationStyle, Shape, Tool};
pub use input::{PageCommand, PageHit, PageInput, PageMode, Swipe, ZoomStep};
pub use layout::{Fit, PageLayout, Rotation, Spread};
pub use raster::PageStats;
pub use source::{Cancel, PageSource, PageText, RenderError, RenderRequest, TextRun};
pub use text::{Match, SearchStatus, TextPos};
pub use view::{MAX_ZOOM, MIN_ZOOM, PageView, SETTLE};

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Insets, Vec2};
use remedy_ui_core::input::Key;
use remedy_ui_core::layout::{Align, Length, Position};
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::style::States;
use remedy_ui_engine::view::strings::StrArg;
use remedy_ui_engine::view::{El, Ui};

/// The pack's own words (page names, annotation kinds, undo names, what screen readers
/// hear) in every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a
/// language, with [`remedy_ui_engine::view::strings::add_messages`] under the name `doc-pages`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("doc-pages");

/// Message `id` with placeholder values in `locale` (not tracked). What this pack says is
/// read by screen readers or names an undo step, so the direction-isolation marks around
/// placeholders, which only matter on screen, are left out.
pub(crate) fn say(locale: &str, id: &str, args: &[(&str, StrArg)]) -> String {
    STRINGS.format_in(locale, id, args).replace(['\u{2068}', '\u{2069}'], "")
}

/// Shows `view`'s pages: a focusable document named `label` that fills the space it is
/// given.
#[track_caller]
pub fn page_view(ui: &Ui, view: &PageView, label: impl Into<String>) -> El {
    let label: String = label.into();
    let text = view.page_text();
    let current = view.current_page();
    let u = ui.clone();
    // The current page, for screen readers: its number and its text.
    let reader = ui
        .el("page_text")
        .layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets::ZERO;
        })
        .pass_through()
        .semantics(move || {
            let page = current.get() + 1;
            Semantics::new(Role::Page)
                .label(say(&u.locale(), "page-number", &[("page", page.into())]))
                .value(text.get())
        });
    let (a, b, c, d, f, g, h) =
        (view.clone(), view.clone(), view.clone(), view.clone(), view.clone(), view.clone(), view.clone());
    let (u, u2) = (ui.clone(), ui.clone());
    let el = ui
        .el("page_view")
        .focusable(true)
        .layout(|l| {
            // A viewport onto huge content: it takes what it is given and never its size from
            // the pages, so a plain column or row needs nothing set to hold it.
            l.grow = 1.0;
            l.shrink = 1.0;
            l.basis = Length::Px(0.0);
            l.min_width = Length::Px(0.0);
            l.min_height = Length::Px(0.0);
        })
        .clip()
        // Its own layer: repainting it re-rasters only it, and scrolls and zooms move and
        // scale it on the compositor until they settle.
        .isolate_paint()
        .paint(view::painter(view))
        .semantics(move || view::semantics(&a, &label, &u2.locale()))
        .on_resize(move |s| b.resized(s))
        .on_wheel(move |e| c.wheel(e))
        .on_pinch(move |e| d.pinch(e))
        .on_pointer(move |e| f.pointer(e))
        .on_key(move |k| g.key(k))
        // The annotations on the page are read in the window's language.
        .bind(move || {
            let _ = u.locale();
            remedy_ui_core::reactive::untrack(|| h.update_page_text());
        })
        .child(reader);
    view.attach(ui.downgrade(), el.id());
    el
}

/// The height of a thumbnail row, logical pixels: the picture and its page number.
const THUMB_ROW: f32 = 184.0;
/// The rail's width: a thumbnail, room for the scrollbar and the margins around it.
const THUMB_RAIL: f32 = 144.0;
/// The box a thumbnail fits in.
const THUMB_WIDTH: f32 = 120.0;
const THUMB_HEIGHT: f32 = 150.0;

/// A rail of page thumbnails for `view`: click one (or press Enter on the screen reader's
/// item) to go to that page; with the rail focused, the arrows, Page Up/Down, Home and End
/// move through the pages. The current page is selected and kept in sight. The rail is a
/// strip of its own width that fills the height it is given, in any row or column.
#[track_caller]
pub fn thumbnails(ui: &Ui, view: &PageView) -> El {
    let n = view.page_count();
    let items = Signal::new((0..n).collect::<Vec<usize>>());
    let (ui2, v) = (ui.clone(), view.clone());
    let rail = crate::base::virtual_list(ui, "", items, THUMB_ROW, move |_, &page| {
        thumb_row(&ui2, &v, page)
    });
    let u = ui.clone();
    let rail = rail.semantics(move || Semantics::new(Role::List).label(say(&u.locale(), "pages-list", &[])));
    let keys = view.clone();
    let rail = rail.focusable(true).on_key(move |k| {
        if !k.pressed {
            return false;
        }
        let cur = keys.current_page().get_untracked().unwrap_or(0);
        let last = keys.page_count().saturating_sub(1);
        let to = match k.key {
            Key::ArrowDown | Key::ArrowRight => cur.saturating_add(1).min(last),
            Key::ArrowUp | Key::ArrowLeft => cur.saturating_sub(1),
            Key::PageDown => cur.saturating_add(5).min(last),
            Key::PageUp => cur.saturating_sub(5),
            Key::Home => 0,
            Key::End => last,
            _ => return false,
        };
        keys.go_to_page(to);
        true
    });
    // A document that gains or loses pages has as many rows.
    let count = view.page_count_signal();
    let rail = rail.bind(move || {
        let n = count.get();
        if items.get_untracked().map(|v| v.len()).ok() != Some(n) {
            let _ = items.try_set((0..n).collect());
        }
    });
    // Keep the current page's row in sight.
    let follow = view.current_page();
    let scroller = rail.clone();
    let rail = rail.bind(move || {
        let page = follow.get();
        let Some(b) = scroller.bounds() else { return };
        let off = scroller.ui().scroll_offset(scroller.id()).y;
        let top = page as f32 * THUMB_ROW;
        if top < off || top + THUMB_ROW > off + b.height() {
            let y = (top - (b.height() - THUMB_ROW) / 2.0).max(0.0);
            scroller.scroll_to(Vec2::new(0.0, y));
        }
    });
    // The strip is a box of its own natural width and no height of its own: it takes the
    // height it is given, and the list (one row per page, so very tall) lies in it, never
    // giving it its height. A plain row or column holds it with nothing set.
    ui.el("page_thumbs")
        .layout(|l| {
            l.width = Length::Px(THUMB_RAIL);
            l.grow = 0.0;
            l.shrink = 0.0;
            l.min_height = Length::Px(0.0);
        })
        .child(rail.layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets::ZERO;
        }))
}

fn thumb_row(ui: &Ui, view: &PageView, page: usize) -> El {
    let v = view.clone();
    let picture = ui
        .el("page_thumb_image")
        .width(THUMB_WIDTH)
        .height(THUMB_HEIGHT)
        .paint(move |cx| v.paint_thumb(cx, page));
    let number = crate::base::text(ui, format!("{}", page + 1)).variant("small");
    let current = view.current_page();
    let go = view.clone();
    let u = ui.clone();
    let row = ui
        .el("page_thumb")
        .column()
        .align(Align::Center)
        .gap(4.0)
        .height(THUMB_ROW)
        .fill_width()
        .child(picture.clone())
        .child(number)
        .on_click(move || go.go_to_page(page))
        // The rail takes the keys; rows are for the pointer and screen readers.
        .focusable(false)
        .semantics(move || {
            Semantics::new(Role::ListItem)
                .label(say(&u.locale(), "page-number", &[("page", (page + 1).into())]))
                .selected(current.get() == page)
                .action(Action::Click)
        });
    let shown = row.clone();
    let row = row.bind(move || {
        let on = current.get() == page;
        shown.state(States::SELECTED, on);
    });
    let gone = view.clone();
    view.thumb_shown(page, ui.downgrade(), picture.id());
    row.on_removed(move || gone.thumb_gone(page))
}
