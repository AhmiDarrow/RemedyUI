//! The performance overlay: drawn over a window, it shows each frame's time by phase (UI
//! thread: events, bindings, style, layout, paint, layers, semantics; then the compositor,
//! raster and present) as stacked bars, the newest frame in numbers, how many frames missed
//! a refresh of the window's display (or went over a budget the developer sets), what the
//! newest frame did (elements restyled, laid out and repainted), what the engine's caches
//! hold, and optionally the areas the newest frame repainted, outlined on the window.
//!
//! Its numbers come from the engine's phase spans through the [`Recorder`]. Redrawing the
//! overlay is a frame too: the overlay redraws for frames the app made, never for a frame
//! that only redrew the overlay, so it does not keep the window busy on its own.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, Insets, Point, Rect, Size};
use remedy_ui_layout::Position;
use remedy_ui_paint::Brush;
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::StylePatch;
use remedy_ui_trace::phases;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

use crate::recorder::{FrameFeed, FrameSample, Recorder};

const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

/// The colors the phases are drawn in, in [`phases::ALL`] order: ten hues far enough apart
/// to tell neighbors in a stacked bar, readable on the overlay's dark panel.
pub const PHASE_COLORS: [Color; 10] = [
    rgba(0.93, 0.36, 0.36, 1.0),
    rgba(0.96, 0.62, 0.25, 1.0),
    rgba(0.94, 0.85, 0.32, 1.0),
    rgba(0.55, 0.82, 0.35, 1.0),
    rgba(0.30, 0.75, 0.62, 1.0),
    rgba(0.33, 0.68, 0.93, 1.0),
    rgba(0.52, 0.52, 0.96, 1.0),
    rgba(0.76, 0.48, 0.92, 1.0),
    rgba(0.93, 0.47, 0.75, 1.0),
    rgba(0.75, 0.75, 0.78, 1.0),
];

/// The overlay's graph: its size in logical pixels and the width of one frame's bar. The
/// graph holds as many frames as bars fit across it.
const GRAPH: Size = Size { width: 300.0, height: 84.0 };
const BAR: f32 = 3.0;

const PANEL: Color = rgba(0.06, 0.07, 0.09, 0.9);
const INK: Color = rgba(0.95, 0.96, 0.97, 1.0);
const DIM: Color = rgba(0.74, 0.77, 0.8, 1.0);
/// Repainted areas, outlined.
const DAMAGE: Color = rgba(0.98, 0.3, 0.55, 1.0);

/// How many frames the graph shows.
pub fn graph_frames() -> usize {
    (GRAPH.width / BAR).floor() as usize
}

/// The performance overlay over one window. Dropping the last handle removes it.
#[derive(Clone)]
pub struct PerfOverlay(Rc<Inner>);

struct Inner {
    target: Ui,
    feed: FrameFeed,
    history: RefCell<VecDeque<FrameSample>>,
    el: RefCell<Option<El>>,
    version: Signal<u64>,
    budget_ns: Cell<u64>,
    /// The budget follows the display's refresh rate (no budget was set).
    follow_display: Cell<bool>,
    seen: Cell<u64>,
    /// What the newest app frame did.
    work: RefCell<remedy_ui_view::FrameStats>,
    /// Where the newest app frame repainted, in window coordinates.
    damage: RefCell<Vec<Rect>>,
    show_damage: Cell<bool>,
    damage_el: RefCell<Option<El>>,
    over: Cell<u64>,
    /// The overlay asked to be redrawn: the next frame is its own.
    own_pending: Cell<bool>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(el) = self.el.borrow_mut().take() {
            el.remove();
        }
        if let Some(el) = self.damage_el.borrow_mut().take() {
            el.remove();
        }
    }
}

/// The frame time a display refreshing every `interval` gives, or 60 Hz's when unknown.
fn budget_for(interval: Option<Duration>) -> u64 {
    interval.and_then(|d| u64::try_from(d.as_nanos()).ok()).filter(|n| *n > 0).unwrap_or(16_666_667)
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

impl std::fmt::Debug for PerfOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PerfOverlay({} frames)", self.0.history.borrow().len())
    }
}

fn ms(ns: u64) -> String {
    format!("{:.1}", ns as f64 / 1_000_000.0)
}

impl PerfOverlay {
    /// Shows the overlay over `target`, measuring its frames against `budget`, or with
    /// `None` against the refresh rate of the display the window is on (following it as the
    /// window moves between displays).
    pub fn show(target: &Ui, recorder: &Recorder, budget: Option<Duration>) -> PerfOverlay {
        let feed = recorder.frames(graph_frames());
        let version = Signal::new(0u64);
        let inner = Rc::new(Inner {
            target: target.clone(),
            feed,
            history: RefCell::new(VecDeque::new()),
            el: RefCell::new(None),
            version,
            budget_ns: Cell::new(match budget {
                Some(b) => u64::try_from(b.as_nanos()).unwrap_or(u64::MAX),
                None => budget_for(untrack(|| target.refresh_interval())),
            }),
            follow_display: Cell::new(budget.is_none()),
            seen: Cell::new(0),
            work: RefCell::new(remedy_ui_view::FrameStats::default()),
            damage: RefCell::new(Vec::new()),
            show_damage: Cell::new(false),
            damage_el: RefCell::new(None),
            over: Cell::new(0),
            own_pending: Cell::new(false),
        });
        let me = PerfOverlay(inner);
        // Samples arrive on the window's thread at the end of each frame; the overlay takes
        // them up as posted work, after the frame.
        let ui_id = target.id();
        let arrived = Signal::new(0u64);
        if let Ok(waker) = remedy_ui_reactive::waker() {
            me.0.feed.on_sample(move |s: &FrameSample| {
                if s.ui == Some(ui_id) {
                    let _ = waker.post(move || {
                        let _ = arrived.try_update(|n| *n += 1);
                    });
                }
            });
        }
        let el = me.build();
        let weak = Rc::downgrade(&me.0);
        let el = el.bind(move || {
            let _ = arrived.get();
            untrack(|| {
                if let Some(inner) = weak.upgrade() {
                    PerfOverlay(inner).take_samples();
                }
            });
        });
        // A budget that follows the display changes with it.
        let (weak, t) = (Rc::downgrade(&me.0), target.clone());
        let el = el.bind(move || {
            let interval = t.refresh_interval();
            untrack(|| {
                if let Some(inner) = weak.upgrade()
                    && inner.follow_display.get()
                {
                    PerfOverlay(inner).apply_budget(budget_for(interval));
                }
            });
        });
        *me.0.el.borrow_mut() = Some(target.overlay(el));
        me
    }

    /// Measures frames against `budget`, or with `None` against the display's refresh.
    pub fn set_budget(&self, budget: Option<Duration>) {
        self.0.follow_display.set(budget.is_none());
        let ns = match budget {
            Some(b) => u64::try_from(b.as_nanos()).unwrap_or(u64::MAX),
            None => budget_for(untrack(|| self.0.target.refresh_interval())),
        };
        self.apply_budget(ns);
    }

    /// The frame budget now.
    pub fn budget(&self) -> Duration {
        Duration::from_nanos(self.0.budget_ns.get())
    }

    fn apply_budget(&self, ns: u64) {
        if self.0.budget_ns.replace(ns) == ns {
            return;
        }
        self.0.over.set(self.0.history.borrow().iter().filter(|s| s.total_ns() > ns).count() as u64);
        self.redraw();
    }

    /// What the newest frame of the app did.
    pub fn work(&self) -> remedy_ui_view::FrameStats {
        self.0.work.borrow().clone()
    }

    /// Where the newest frame of the app repainted, in window coordinates.
    pub fn damage(&self) -> Vec<Rect> {
        self.0.damage.borrow().clone()
    }

    /// Outlines on the window the areas each frame of the app repaints.
    pub fn show_damage(&self, on: bool) {
        self.0.show_damage.set(on);
        let mut slot = self.0.damage_el.borrow_mut();
        match (on, slot.is_some()) {
            (true, false) => {
                let weak = Rc::downgrade(&self.0);
                let version = self.0.version;
                let layer = self
                    .0
                    .target
                    .el("perf_damage")
                    .pass_through()
                    .layout(|l| {
                        l.position = Position::Absolute;
                        l.inset = Insets::ZERO;
                    })
                    .semantics(Semantics::new(Role::Group).hidden(true))
                    .paint(move |cx| {
                        let _ = version.get();
                        let Some(inner) = weak.upgrade() else { return };
                        for r in inner.damage.borrow().iter() {
                            cx.list.border(*r, remedy_ui_core::CornerRadii::ZERO, 1.5, DAMAGE);
                            cx.list.fill_rect(*r, Brush::Solid(DAMAGE.with_alpha(0.12)));
                        }
                    });
                *slot = Some(self.0.target.overlay(layer));
            }
            (false, true) => {
                if let Some(el) = slot.take() {
                    el.remove();
                }
            }
            _ => {}
        }
        drop(slot);
        self.redraw();
    }

    /// The frames the graph shows, oldest first.
    pub fn samples(&self) -> Vec<FrameSample> {
        self.0.history.borrow().iter().cloned().collect()
    }

    /// The overlay's element in the target window.
    pub fn element(&self) -> Option<El> {
        self.0.el.borrow().clone()
    }

    fn redraw(&self) {
        self.0.own_pending.set(true);
        let _ = self.0.version.try_update(|v| *v += 1);
    }

    fn take_samples(&self) {
        let new = self.0.feed.take();
        if new.is_empty() {
            return;
        }
        let ui = self.0.target.id();
        let mut fresh = 0usize;
        {
            let mut h = self.0.history.borrow_mut();
            for s in new.into_iter().filter(|s| s.ui == Some(ui)) {
                self.0.seen.set(self.0.seen.get() + 1);
                if s.total_ns() > self.0.budget_ns.get() {
                    self.0.over.set(self.0.over.get() + 1);
                }
                h.push_back(s);
                fresh += 1;
            }
            while h.len() > graph_frames() {
                h.pop_front();
            }
        }
        // The first frame after the overlay asked to be redrawn is that redraw: on its own
        // it is no reason to draw again.
        let own = self.0.own_pending.replace(false);
        if fresh > usize::from(own) {
            self.capture_work();
            self.redraw();
        }
    }

    /// Keeps what the window's newest frame did, leaving out the overlay's own elements.
    fn capture_work(&self) {
        let ui = &self.0.target;
        let mut stats = ui.last_frame_stats();
        let mine: Vec<El> =
            [self.0.el.borrow().clone(), self.0.damage_el.borrow().clone()].into_iter().flatten().collect();
        let inside_mine = |e: &El| {
            let mut cur = Some(e.clone());
            while let Some(c) = cur {
                if mine.iter().any(|m| m.id() == c.id()) {
                    return true;
                }
                cur = c.parent();
            }
            false
        };
        stats.repainted.retain(|id| !inside_mine(&ui.element(*id)));
        let damage: Vec<Rect> = stats.repainted.iter().filter_map(|id| ui.element(*id).bounds()).collect();
        *self.0.damage.borrow_mut() = damage;
        *self.0.work.borrow_mut() = stats;
    }

    fn build(&self) -> El {
        let ui = &self.0.target;
        let unit = ui.theme().spacing.unit;
        let text = |s: &str, color: Color| {
            w::text(ui, s.to_owned())
                .data_text()
                .nowrap()
                .style(StylePatch::default().foreground(color).text_size(12.0))
        };
        let weak = Rc::downgrade(&self.0);
        let version = self.0.version;
        let headline = text("Waiting for a frame…", INK).text_dyn({
            let weak = weak.clone();
            move || {
                let _ = version.get();
                let Some(inner) = weak.upgrade() else { return String::new() };
                let h = inner.history.borrow();
                match h.back() {
                    Some(s) => format!(
                        "Frame {} ms · UI {} · composite {} · raster {} · present {}",
                        ms(s.total_ns()),
                        ms(phases::ALL.iter().take(7).map(|p| s.phase(p)).sum::<u64>() + s.other_ns),
                        ms(s.phase(phases::COMPOSITE)),
                        ms(s.phase(phases::RASTER)),
                        ms(s.phase(phases::PRESENT)),
                    ),
                    None => "Waiting for a frame…".to_owned(),
                }
            }
        });
        let detail = text("", DIM).text_dyn({
            let weak = weak.clone();
            move || {
                let _ = version.get();
                let Some(inner) = weak.upgrade() else { return String::new() };
                let h = inner.history.borrow();
                let Some(s) = h.back() else { return String::new() };
                phases::ALL
                    .iter()
                    .take(7)
                    .map(|p| format!("{p} {}", ms(s.phase(p))))
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
        });
        let budget_line = text("", DIM).text_dyn({
            let weak = weak.clone();
            move || {
                let _ = version.get();
                let Some(inner) = weak.upgrade() else { return String::new() };
                if inner.follow_display.get() {
                    let known = untrack(|| inner.target.refresh_interval()).is_some();
                    let hz = 1_000_000_000.0 / inner.budget_ns.get().max(1) as f64;
                    format!(
                        "{} of {} frames missed a refresh ({:.0} Hz display{})",
                        inner.over.get(),
                        inner.seen.get(),
                        hz,
                        if known { "" } else { ", rate unknown: 60 Hz assumed" }
                    )
                } else {
                    format!(
                        "{} of {} frames over the {} ms budget",
                        inner.over.get(),
                        inner.seen.get(),
                        ms(inner.budget_ns.get())
                    )
                }
            }
        });
        let work_line = text("", DIM).text_dyn({
            let weak = weak.clone();
            move || {
                let _ = version.get();
                let Some(inner) = weak.upgrade() else { return String::new() };
                let w = inner.work.borrow();
                format!(
                    "Restyled {} · laid out {} · repainted {}{}{}",
                    w.restyled,
                    w.laid_out,
                    w.repainted.len(),
                    if w.layers { " · layers rebuilt" } else { "" },
                    if w.semantics { " · semantics updated" } else { "" },
                )
            }
        });
        let cache_line = text("", DIM).text_dyn({
            move || {
                let _ = version.get();
                let held = remedy_ui_trace::gauges::snapshot();
                if held.is_empty() {
                    return "Caches: nothing held yet".to_owned();
                }
                let parts: Vec<String> = held.iter().map(|(n, v)| format!("{n} {}", megabytes(*v))).collect();
                format!("Caches: {}", parts.join(" · "))
            }
        });
        let graph = ui.el("perf_graph").width(GRAPH.width).height(GRAPH.height).paint({
            let weak = weak.clone();
            move |cx| {
                let _ = version.get();
                let Some(inner) = weak.upgrade() else { return };
                let h = inner.history.borrow();
                let budget = inner.budget_ns.get().max(1);
                let top = h.iter().map(FrameSample::total_ns).max().unwrap_or(0).max(budget + budget / 2);
                let scale = cx.size.height / top as f32;
                let n = h.len();
                for (i, s) in h.iter().enumerate() {
                    let x = cx.size.width - (n - i) as f32 * BAR;
                    let mut y = cx.size.height;
                    for (ph, color) in s.phases.iter().zip(PHASE_COLORS.iter()) {
                        let hgt = *ph as f32 * scale;
                        if hgt <= 0.0 {
                            continue;
                        }
                        y -= hgt;
                        cx.list.fill_rect(
                            Rect::from_origin_size(Point::new(x, y), Size::new(BAR - 0.5, hgt)),
                            Brush::Solid(*color),
                        );
                    }
                }
                let by = cx.size.height - budget as f32 * scale;
                cx.list.fill_rect(
                    Rect::from_origin_size(Point::new(0.0, by), Size::new(cx.size.width, 1.0)),
                    Brush::Solid(INK.with_alpha(0.7)),
                );
            }
        });
        let mut legend = w::wrap(ui).gap(unit);
        for (p, c) in phases::ALL.iter().zip(PHASE_COLORS.iter()) {
            let chip = ui
                .el("perf_chip")
                .width(8.0)
                .height(8.0)
                .style(StylePatch::default().background(*c).radius(2.0));
            legend = legend.child(
                w::row(ui)
                    .gap(unit / 2.0)
                    .align(remedy_ui_view::Align::Center)
                    .child(chip)
                    .child(text(p, DIM)),
            );
        }
        ui.el("perf_overlay")
            .column()
            .gap(unit)
            .pass_through()
            .padding(Insets::all(unit * 2.0))
            .style(StylePatch::default().background(PANEL).radius(6.0))
            .layout(|l| {
                l.position = Position::Absolute;
                l.inset = Insets {
                    top: 8.0,
                    end: 8.0,
                    start: remedy_ui_layout::AUTO,
                    bottom: remedy_ui_layout::AUTO,
                };
                l.width = remedy_ui_layout::Length::Px(GRAPH.width + unit * 4.0);
            })
            .semantics(Semantics::new(Role::Group).hidden(true))
            .child(headline)
            .child(graph)
            .child(detail)
            .child(budget_line)
            .child(work_line)
            .child(cache_line)
            .child(legend)
    }
}
