//! A synthetic document for the page view's tests: vector pages drawn by the CPU renderer on
//! the page workers, with text runs where the drawn lines are.
#![allow(dead_code)]

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size};
use remedy_ui_widgets::doc_pages::*;
use remedy_ui_core::input::{
    Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerKind, PointerPhase, WheelEvent,
};
use remedy_ui_engine::paint::{Affine, Brush, DisplayList, Image, Item, Path, Stroke};
use remedy_ui_engine::render_cpu::CpuRenderer;
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, Request, Ui};

/// US Letter, in points.
pub const LETTER: Size = Size { width: 612.0, height: 792.0 };

/// Lines of text on each page.
pub const LINES: usize = 30;

/// A document of vector pages: a header band in a color of its own per page, lines of
/// "text", a circle and a diagonal, so tiles at any scale have edges to compare.
pub struct Synthetic {
    pub pages: usize,
    /// Every tenth page is landscape when true.
    pub mixed: bool,
    /// Extra time each render takes (a slow document), checking for cancellation.
    pub delay: Duration,
    /// Pages that fail to draw.
    pub broken: Option<usize>,
    pub rendered: AtomicU64,
    pub pixels: AtomicU64,
}

impl Synthetic {
    pub fn new(pages: usize) -> Self {
        Synthetic {
            pages,
            mixed: false,
            delay: Duration::ZERO,
            broken: None,
            rendered: AtomicU64::new(0),
            pixels: AtomicU64::new(0),
        }
    }
}

/// The color of a page's header band (tests find pages by it).
pub fn header_color(page: usize) -> Color {
    let h = (page as u32).wrapping_mul(2_654_435_761);
    Color::rgb8(40 + (h % 180) as u8, 40 + ((h >> 8) % 180) as u8, 40 + ((h >> 16) % 180) as u8)
}

/// Where line `k` of a page is, in points.
pub fn line_rect(k: usize, text: &str) -> Rect {
    let y = 120.0 + k as f32 * 20.0;
    Rect::new(72.0, y, 72.0 + text.chars().count() as f32 * 7.0, y + 12.0)
}

pub fn line_text(page: usize, k: usize) -> String {
    format!("Page {} line {} the quick brown fox", page + 1, k + 1)
}

fn page_list(page: usize, size: Size) -> DisplayList {
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(36.0, 36.0, size.width - 36.0, 96.0), Brush::Solid(header_color(page)));
    for k in 0..LINES {
        let r = line_rect(k, &line_text(page, k));
        l.fill_rounded(r, CornerRadii::all(2.0), Brush::Solid(Color::rgb8(60, 60, 60)));
    }
    let mut circle = Path::new();
    let (cx, cy, r) = (size.width - 150.0, size.height - 150.0, 80.0);
    circle.move_to(Point::new(cx + r, cy));
    for i in 1..=32 {
        let a = i as f32 / 32.0 * std::f32::consts::TAU;
        circle.line_to(Point::new(cx + r * a.cos(), cy + r * a.sin()));
    }
    circle.close();
    l.push(Item::FillPath { path: circle, brush: Brush::Solid(Color::rgb8(200, 60, 60)), even_odd: false });
    let mut diag = Path::new();
    diag.move_to(Point::new(36.0, size.height - 36.0)).line_to(Point::new(size.width - 36.0, 110.0));
    l.push(Item::StrokePath {
        path: diag,
        stroke: Stroke::new(1.5),
        brush: Brush::Solid(Color::rgb8(30, 90, 200)),
    });
    l
}

thread_local! {
    static RENDERER: RefCell<CpuRenderer> = RefCell::new(CpuRenderer::new());
}

impl PageSource for Synthetic {
    fn page_count(&self) -> usize {
        self.pages
    }

    fn page_size(&self, page: usize) -> Size {
        if self.mixed && page % 10 == 9 { Size::new(LETTER.height, LETTER.width) } else { LETTER }
    }

    fn render(&self, req: &RenderRequest, cancel: &Cancel) -> Result<Image, RenderError> {
        if self.broken == Some(req.page) {
            return Err(RenderError::Failed("damaged page".into()));
        }
        let until = Instant::now() + self.delay;
        while Instant::now() < until {
            if cancel.is_cancelled() {
                return Err(RenderError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let list = page_list(req.page, self.page_size(req.page));
        let base = Affine(req.page_to_pixels());
        let frame = RENDERER
            .with(|r| r.borrow_mut().render_with(&list, req.width, req.height, base, Color::WHITE))
            .map_err(|e| RenderError::Failed(format!("{e:?}")))?;
        self.rendered.fetch_add(1, Ordering::Relaxed);
        self.pixels.fetch_add(u64::from(req.width) * u64::from(req.height), Ordering::Relaxed);
        Ok(Image::new(req.width, req.height, Arc::from(frame.pixels)))
    }

    fn text(&self, page: usize) -> Option<PageText> {
        let runs = (0..LINES)
            .map(|k| {
                let t = line_text(page, k);
                let r = line_rect(k, &t);
                TextRun::new(t, r)
            })
            .collect();
        Some(PageText { runs })
    }
}

pub fn ui() -> Ui {
    let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts)
}

/// A window running a page view: frames, posted results and timers, as the runner does.
pub struct Win {
    pub ui: Ui,
    pub view: PageView,
    pub el: El,
    pub size: Size,
    pub scale: f32,
    timers: Vec<(Instant, u64)>,
    pub copied: Vec<String>,
    pub last: Option<remedy_ui_engine::compositor::LayerTree>,
}

impl Win {
    pub fn new(source: Arc<dyn PageSource>, size: Size) -> Win {
        let ui = ui();
        let view = PageView::new(source);
        let el = page_view(&ui, &view, "Synthetic.pdf");
        ui.root().child(el.clone());
        el.focus();
        let mut w =
            Win { ui, view, el, size, scale: 1.0, timers: Vec::new(), copied: Vec::new(), last: None };
        w.frame();
        w.frame();
        w
    }

    /// One frame: results from the workers, timers that are due, then the view.
    pub fn frame(&mut self) {
        let _ = self.frame_layers();
    }

    /// One frame, returning the layers it committed (none when nothing changed).
    pub fn frame_layers(&mut self) -> Option<remedy_ui_engine::compositor::LayerTree> {
        let _ = remedy_ui_core::reactive::run_posted();
        let now = Instant::now();
        let due: Vec<u64> = self.timers.iter().filter(|(t, _)| *t <= now).map(|(_, id)| *id).collect();
        self.timers.retain(|(t, _)| *t > now);
        for id in due {
            self.ui.fire_timer(id);
        }
        let f = self.ui.frame(self.size, self.scale);
        if f.layers.is_some() {
            self.last = f.layers.clone();
        }
        for r in self.ui.take_requests() {
            match r {
                Request::Timer { id, delay } => self.timers.push((Instant::now() + delay, id)),
                Request::CancelTimer(id) => self.timers.retain(|(_, t)| *t != id),
                Request::Copy { text, .. } => self.copied.push(text),
                _ => {}
            }
        }
        f.layers
    }

    /// Frames until the view shows every tile at its own scale (or `limit` passes).
    pub fn settle(&mut self, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            self.frame();
            if self.view.is_sharp() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let r = self.ui.key(&KeyEvent::press(key).with_modifiers(modifiers).with_text(None));
        self.frame();
        r
    }

    pub fn wheel(&mut self, at: Point, delta: f32, modifiers: Modifiers) -> bool {
        let r = self
            .ui
            .wheel(&WheelEvent::new(at, remedy_ui_core::Vec2::new(0.0, delta)).with_modifiers(modifiers));
        self.frame();
        r
    }

    pub fn pointer(&mut self, id: u32, kind: PointerKind, phase: PointerPhase, at: Point, buttons: Buttons) {
        self.ui.pointer(&PointerEvent::new(phase, at).with_id(id).with_kind(kind).with_buttons(buttons));
        self.frame();
    }
}

pub fn ctrl() -> Modifiers {
    Modifiers { ctrl: true, ..Modifiers::NONE }
}
