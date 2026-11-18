//! Document viewer: a paged document with a thumbnails rail, page and zoom controls, deep
//! zoom rendered off the UI thread, and the page's text for selection, search and screen
//! readers.
//!
//! The pages come from a [`PageSource`]. A real viewer wraps its PDF or e-book engine in
//! one; this recipe draws a small report itself, so it runs with nothing to open.
//!
//! Run it with `cargo run -p remedy-ui --example document_viewer`.

use std::sync::Arc;

use remedy_ui::core::{Rect, Size};
use remedy_ui::pages::{
    Cancel, Fit, PageSource, PageText, PageView, RenderError, RenderRequest, TextRun, page_view, thumbnails,
};
use remedy_ui::paint::Image;
use remedy_ui::view::Space;
use remedy_ui::widgets::{self as w, Icon};
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// A report of `pages` A5 pages: a colored band with the chapter's title, then lines of
/// text, each a dark bar where its words are.
#[derive(Debug)]
pub struct Report {
    /// How many pages.
    pub pages: usize,
}

/// A5, in points.
const PAGE: Size = Size { width: 420.0, height: 595.0 };
/// Lines of text on a page.
const LINES: usize = 22;

impl Report {
    /// The text of line `k` on `page`.
    pub fn line(page: usize, k: usize) -> String {
        format!("Chapter {} line {}: the quarterly figures held steady", page + 1, k + 1)
    }

    /// Where line `k` sits on the page, in points.
    fn line_box(k: usize, text: &str) -> Rect {
        let y = 110.0 + k as f32 * 20.0;
        Rect::new(40.0, y, (40.0 + text.chars().count() as f32 * 5.6).min(PAGE.width - 40.0), y + 10.0)
    }

    /// The title band.
    fn band() -> Rect {
        Rect::new(30.0, 30.0, PAGE.width - 30.0, 80.0)
    }
}

/// Fills the part of `rect` (page points) inside the tile with `rgba`.
fn fill(pixels: &mut [u8], req: &RenderRequest, rect: Rect, rgba: [u8; 4]) {
    let to_px = |v: f32, min: f32| ((v - min) * req.scale).round().max(0.0) as u32;
    let (x0, x1) = (
        to_px(rect.min.x, req.region.min.x).min(req.width),
        to_px(rect.max.x, req.region.min.x).min(req.width),
    );
    let (y0, y1) = (
        to_px(rect.min.y, req.region.min.y).min(req.height),
        to_px(rect.max.y, req.region.min.y).min(req.height),
    );
    for y in y0..y1 {
        let start = ((y * req.width + x0) * 4) as usize;
        let end = ((y * req.width + x1) * 4) as usize;
        if let Some(row) = pixels.get_mut(start..end) {
            for px in row.as_chunks_mut::<4>().0 {
                *px = rgba;
            }
        }
    }
}

impl PageSource for Report {
    fn page_count(&self) -> usize {
        self.pages
    }

    fn page_size(&self, _page: usize) -> Size {
        PAGE
    }

    fn render(&self, req: &RenderRequest, cancel: &Cancel) -> Result<Image, RenderError> {
        if cancel.is_cancelled() {
            return Err(RenderError::Cancelled);
        }
        let mut pixels = vec![255u8; (req.width as usize) * (req.height as usize) * 4];
        // Each chapter's band has its own hue, so pages are told apart in the thumbnails.
        let hue = (req.page * 47 % 160) as u8;
        fill(&mut pixels, req, Report::band(), [40 + hue, 90, 200 - hue, 255]);
        for k in 0..LINES {
            fill(&mut pixels, req, Report::line_box(k, &Report::line(req.page, k)), [60, 60, 60, 255]);
        }
        Ok(Image::new(req.width, req.height, Arc::from(pixels)))
    }

    fn text(&self, page: usize) -> Option<PageText> {
        let runs = (0..LINES)
            .map(|k| {
                let t = Report::line(page, k);
                let b = Report::line_box(k, &t);
                TextRun::new(t, b)
            })
            .collect();
        Some(PageText { runs })
    }
}

/// Opens the viewer on a twelve-page report.
pub fn app(app: &App) {
    app.open(WindowSpec::new("Quarterly report").size(900.0, 680.0), |ui, _| {
        ui.root().child(viewer(ui, Arc::new(Report { pages: 12 })));
    });
}

/// The viewer: controls over the thumbnails rail and the pages.
pub fn viewer(ui: &Ui, source: Arc<dyn PageSource>) -> El {
    let view = PageView::new(source);
    view.set_fit(Fit::Width);
    let pages = view.page_count();
    let (current, zoom) = (view.current_page(), view.zoom());

    let (v1, v2, v3, v4, v5) = (view.clone(), view.clone(), view.clone(), view.clone(), view.clone());
    let controls = w::row(ui)
        .gap(Space::S2)
        .padding(Space::S2)
        .align(remedy_ui::layout::Align::Center)
        .child(w::icon_button(ui, Icon::ChevronUp, "Previous page", move || {
            v1.go_to_page(v1.current_page().get_untracked().unwrap_or(0).saturating_sub(1));
        }))
        .child(w::icon_button(ui, Icon::ChevronDown, "Next page", move || {
            let next = v2.current_page().get_untracked().unwrap_or(0) + 1;
            v2.go_to_page(next.min(pages.saturating_sub(1)));
        }))
        .child(w::text(ui, move || format!("Page {} of {pages}", current.get() + 1)).data_text())
        .child(w::spacer(ui))
        .child(w::icon_button(ui, Icon::Minus, "Zoom out", move || v3.zoom_out()))
        .child(w::text(ui, move || format!("{:.0}%", zoom.get() * 100.0)).data_text())
        .child(w::icon_button(ui, Icon::Plus, "Zoom in", move || v4.zoom_in()))
        .child(w::button(ui, "Fit width", move || v5.set_fit(Fit::Width)));

    // The rail takes its own width, and the pages the rest; both scroll inside the window.
    let body = w::row(ui).grow(1.0).fill_width().child(thumbnails(ui, &view)).child(page_view(
        ui,
        &view,
        "Quarterly report",
    ));
    w::column(ui).grow(1.0).fill_width().child(controls).child(body)
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Document viewer"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
