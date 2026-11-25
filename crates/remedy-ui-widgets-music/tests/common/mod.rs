//! Helpers shared by the viz tests.
#![allow(dead_code)]

use std::time::Duration;

use std::cell::RefCell;

use remedy_ui_engine::compositor::{Compositor, LayerTree};
use remedy_ui_core::{Color, Point, Size};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase, ScrollPhysics};
use remedy_ui_engine::render_cpu::CpuCompositor;
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, Request, Ui};

pub const SIZE: Size = Size { width: 720.0, height: 640.0 };

pub fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

thread_local! {
    /// What the last frame that drew something new made, until a screenshot takes it.
    static LATEST: RefCell<Option<LayerTree>> = const { RefCell::new(None) };
}

/// A frame; true when it drew something new.
pub fn frame(ui: &Ui) -> bool {
    match ui.frame(SIZE, 1.0).layers {
        Some(l) => {
            LATEST.with(|c| *c.borrow_mut() = Some(l));
            true
        }
        None => false,
    }
}

pub fn key(ui: &Ui, k: Key) -> bool {
    let r = ui.key(&KeyEvent::press(k).with_text(None));
    let _ = frame(ui);
    r
}

pub fn pointer(ui: &Ui, at: Point, phase: PointerPhase, buttons: Buttons) {
    ui.pointer(&PointerEvent::new(phase, at).with_buttons(buttons));
    let _ = frame(ui);
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

/// What screen readers get from an element.
pub fn value_text(el: &El) -> String {
    el.info().and_then(|i| i.semantics).and_then(|s| s.value_text().map(str::to_owned)).unwrap_or_default()
}

/// Problems the accessibility audit finds, described.
pub fn audit(ui: &Ui) -> Vec<String> {
    let _ = frame(ui);
    remedy_ui_devtools::audit(ui)
        .iter()
        .map(|p| {
            format!(
                "{} [{}]",
                p.message(),
                ui.element(p.element).info().map(|i| remedy_ui_devtools::describe(&i)).unwrap_or_default()
            )
        })
        .collect()
}

/// Draws a window through the compositor and the CPU renderer, frame after frame.
pub struct Shot {
    comp: Compositor,
    cpu: CpuCompositor,
    bg: Color,
    n: u64,
    pub pixels: Vec<u8>,
}

impl Shot {
    pub fn new(ui: &Ui) -> Self {
        Shot {
            comp: Compositor::new(SIZE.width as u32, SIZE.height as u32, 1.0, ScrollPhysics::CLAMPED),
            cpu: CpuCompositor::new(),
            bg: ui.theme().colors.bg,
            n: 0,
            pixels: Vec::new(),
        }
    }

    /// Draws the window as it is now.
    pub fn take(&mut self, ui: &Ui) {
        let _ = frame(ui);
        if let Some(l) = LATEST.with(|c| c.borrow_mut().take()) {
            self.comp.commit(l);
        }
        self.n += 1;
        let plan = self.comp.frame(Duration::from_millis(16 * self.n));
        if let Ok(fr) = self.cpu.present(&plan, SIZE.width as u32, SIZE.height as u32, self.bg) {
            self.pixels = fr.pixels;
        }
    }

    /// The pixel at `x`, `y` as RGB.
    pub fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * SIZE.width as u32 + x) * 4) as usize;
        match self.pixels.get(i..i + 3) {
            Some([r, g, b]) => [*r, *g, *b],
            _ => [0, 0, 0],
        }
    }
}

/// A color as the renderer writes it.
pub fn rgb(c: Color) -> [u8; 3] {
    let b = |v: f32| (v * 255.0).round() as u8;
    [b(c.r), b(c.g), b(c.b)]
}

/// True when two pixels are within `tol` on every channel.
pub fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

pub fn key_mod(ui: &Ui, k: Key, modifiers: Modifiers) -> bool {
    let r = ui.key(&KeyEvent::press(k).with_modifiers(modifiers).with_text(None));
    let _ = frame(ui);
    r
}

/// A press and release at `at`.
pub fn click(ui: &Ui, at: Point) {
    pointer(ui, at, PointerPhase::Move, Buttons(0));
    pointer(ui, at, PointerPhase::Down, Buttons::PRIMARY);
    pointer(ui, at, PointerPhase::Up, Buttons(0));
}

/// The middle of an element on screen.
pub fn center(el: &El) -> Point {
    el.bounds().map(|b| b.center()).unwrap_or(Point::ZERO)
}

/// What screen readers get as a name.
pub fn name_of(el: &El) -> String {
    el.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default()
}

pub fn role_of(el: &El) -> Option<remedy_ui_core::a11y::Role> {
    el.info().and_then(|i| i.semantics).map(|s| s.role())
}

pub fn actions_of(el: &El) -> Vec<remedy_ui_core::a11y::Action> {
    el.info().and_then(|i| i.semantics).map(|s| s.actions().to_vec()).unwrap_or_default()
}

/// The first element under `el` made by `widget`.
pub fn first(el: &El, widget: &str) -> El {
    find(el, widget).into_iter().next().unwrap_or_else(|| unreachable!("no {widget}"))
}

/// The timers waiting that were asked to fire after exactly `delay`.
pub fn timers_after(ui: &Ui, delay: Duration) -> Vec<u64> {
    ui.take_requests()
        .into_iter()
        .filter_map(|r| match r {
            Request::Timer { id, delay: d } if d == delay => Some(id),
            _ => None,
        })
        .collect()
}

/// Writes the last drawn window as a PNG (for looking at while developing): only when the
/// REMEDY_SNAP_DIR environment variable names a folder.
pub fn snap(shot: &Shot, name: &str) {
    let Ok(dir) = std::env::var("REMEDY_SNAP_DIR") else { return };
    let (w, h) = (SIZE.width as usize, SIZE.height as usize);
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w {
            raw.extend_from_slice(&shot.at(x as u32, y as u32));
        }
    }
    let mut z = vec![0x78u8, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push(u8::from(i + 1 == chunks.len()));
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for v in &raw {
        a = (a + u32::from(*v)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for byte in data {
            c ^= u32::from(*byte);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    };
    let mut head = Vec::new();
    head.extend_from_slice(&(w as u32).to_be_bytes());
    head.extend_from_slice(&(h as u32).to_be_bytes());
    head.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &head);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), out);
}
