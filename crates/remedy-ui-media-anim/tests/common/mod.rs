//! Helpers for the animation tests: a window whose timers the test fires by hand, and files
//! made with the encoders in the dependency tree.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use remedy_ui_compositor::{Layer, LayerTree};
use remedy_ui_core::Size;
use remedy_ui_core::input::{Key, KeyEvent};
use remedy_ui_paint::{Image, Item};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};

#[path = "../../src/testing.rs"]
pub mod testing;

pub use testing::*;

pub const SIZE: Size = Size { width: 400.0, height: 300.0 };

pub fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn images(layer: &Layer, out: &mut Vec<Image>) {
    for item in layer.content.items() {
        if let Item::Image { image, .. } = item {
            out.push(image.clone());
        }
    }
    for c in &layer.children {
        images(c, out);
    }
}

/// A window the test drives: frames, the host's timers (fired when the test says), and what
/// was last drawn.
pub struct Screen {
    pub ui: Ui,
    /// Timers asked for and not yet fired or cancelled: id and delay.
    pub timers: Vec<(u64, Duration)>,
    /// The images in the last layers drawn.
    pub drawn: Vec<Image>,
    /// How many timers were ever asked for.
    pub asked: usize,
}

impl Screen {
    pub fn new() -> Screen {
        Screen { ui: ui(), timers: Vec::new(), drawn: Vec::new(), asked: 0 }
    }

    pub fn frame(&mut self) {
        let f = self.ui.frame(SIZE, 1.0);
        if let Some(tree) = &f.layers {
            self.drawn = drawn(tree);
        }
        for r in self.ui.take_requests() {
            match r {
                Request::Timer { id, delay } => {
                    self.asked += 1;
                    self.timers.push((id, delay));
                }
                Request::CancelTimer(id) => self.timers.retain(|(t, _)| *t != id),
                Request::ScrollTo { scroller, offset } => {
                    self.ui.set_scroll_offset(scroller, offset);
                    let f = self.ui.frame(SIZE, 1.0);
                    if let Some(tree) = &f.layers {
                        self.drawn = drawn(tree);
                    }
                    self.take();
                }
                _ => {}
            }
        }
    }

    fn take(&mut self) {
        for r in self.ui.take_requests() {
            match r {
                Request::Timer { id, delay } => {
                    self.asked += 1;
                    self.timers.push((id, delay));
                }
                Request::CancelTimer(id) => self.timers.retain(|(t, _)| *t != id),
                _ => {}
            }
        }
    }

    /// Runs what workers posted, then a frame.
    pub fn pump(&mut self) {
        let _ = remedy_ui_core::reactive::run_posted();
        self.frame();
    }

    /// Pumps until `ok`, up to ten seconds.
    pub fn wait(&mut self, what: &str, ok: impl Fn(&Screen) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if ok(self) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "waited for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Lets `ms` of real time pass, pumping.
    pub fn idle(&mut self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            self.pump();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The host's timer goes off: the one timer pending.
    pub fn fire(&mut self) {
        assert_eq!(self.timers.len(), 1, "one timer is pending: {:?}", self.timers);
        self.fire_nth(0);
    }

    pub fn fire_nth(&mut self, n: usize) {
        let (id, _) = self.timers.remove(n);
        self.ui.fire_timer(id);
        self.frame();
    }

    /// The first pixel of what is drawn (straight and premultiplied agree for opaque colors).
    pub fn color(&self) -> [u8; 4] {
        self.colors().first().copied().unwrap_or([9; 4])
    }

    /// The first pixel of each image drawn.
    pub fn colors(&self) -> Vec<[u8; 4]> {
        self.drawn
            .iter()
            .map(|i| i.pixels.get(..4).and_then(|s| s.try_into().ok()).unwrap_or([9; 4]))
            .collect()
    }

    /// The delay of the one pending timer, in milliseconds.
    pub fn delay_ms(&self) -> u128 {
        assert_eq!(self.timers.len(), 1, "one timer is pending: {:?}", self.timers);
        self.timers.first().map_or(0, |t| t.1.as_millis())
    }
}

fn drawn(tree: &LayerTree) -> Vec<Image> {
    let mut out = Vec::new();
    for r in &tree.roots {
        images(r, &mut out);
    }
    out
}

/// Whether `got` is `want` milliseconds, allowing the few the clock ate before the timer was set.
pub fn about(got: u128, want: u128) -> bool {
    got <= want && got + 30 >= want
}

/// Whether each of `got` is about the matching one of `want`.
pub fn all_about(got: &[u128], want: &[u128]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(g, w)| about(*g, *w))
}

pub fn press(ui: &Ui, key: Key) -> bool {
    ui.key(&KeyEvent::press(key).with_text(None))
}

/// Every element under `el` made by `widget`, in tree order.
pub fn find(el: &El, widget: &str) -> Vec<El> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == widget) {
            out.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

pub fn semantics(el: &El) -> String {
    // Without the isolation marks the string catalogs put around placeholders.
    format!("{:?}", el.info().and_then(|i| i.semantics))
        .replace(['\u{2068}', '\u{2069}'], "")
        .replace("\\u{2068}", "")
        .replace("\\u{2069}", "")
}

/// Problems the accessibility audit finds, described.
pub fn audit(s: &mut Screen) -> Vec<String> {
    s.frame();
    remedy_ui_devtools::audit(&s.ui)
        .iter()
        .map(|p| {
            format!(
                "{} [{}]",
                p.message(),
                s.ui.element(p.element).info().map(|i| remedy_ui_devtools::describe(&i)).unwrap_or_default()
            )
        })
        .collect()
}

pub const R: [u8; 4] = [255, 0, 0, 255];
pub const G: [u8; 4] = [0, 255, 0, 255];
pub const B: [u8; 4] = [0, 0, 255, 255];
