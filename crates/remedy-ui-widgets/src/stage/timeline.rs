//! A horizontal track of items placed by time, scrubbed back through history and virtualized.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use crate::viz::{Clock, Moved, smooth};
use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::input::{Key, PointerPhase};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_core::{Rect, Size};
use remedy_ui_engine::view::{El, Ui};

use crate::stage::common::{Motion, Place, Slot, layer};

/// Which end of a [`timeline`] the newest items are at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NewestAt {
    /// The right: time flows from left to right, and history is to the left.
    #[default]
    Right,
    /// The left: time flows from right to left (a river running toward the reader).
    Left,
}

/// How a [`timeline`] is scaled and moves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TimelineOptions {
    /// Logical pixels per unit of time (the unit is whatever the item times are in; seconds
    /// usually). `None` takes four theme units.
    pub scale: Option<f64>,
    /// Which end the newest items are at.
    pub newest_at: NewestAt,
    /// Space kept between the newest item's time and the track's end, in logical pixels.
    /// `None` takes sixteen theme units.
    pub lead: Option<f32>,
    /// Where each item sits around its time along the track: `Start` begins at its time (on
    /// the side away from the newest end), `Center` is centered on it, `End` ends at it.
    pub along: Place,
    /// Where items sit across the track.
    pub across: Place,
    /// New items glide in: the track flows toward them instead of jumping. Never under
    /// reduced motion.
    pub flow: bool,
    /// Where time comes from (for the flow).
    pub clock: Clock,
}

impl Default for TimelineOptions {
    fn default() -> Self {
        TimelineOptions {
            scale: None,
            newest_at: NewestAt::Right,
            lead: None,
            along: Place::End,
            across: Place::Center,
            flow: true,
            clock: Clock::system(),
        }
    }
}

impl TimelineOptions {
    /// Sets `scale`: logical pixels per unit of time.
    pub fn scale(mut self, value: f64) -> Self {
        self.scale = (value.is_finite() && value > 0.0).then_some(value);
        self
    }

    /// Sets `newest_at`: which end the newest items are at.
    pub fn newest_at(mut self, value: NewestAt) -> Self {
        self.newest_at = value;
        self
    }

    /// Sets `lead`: space between the newest time and the track's end.
    pub fn lead(mut self, value: f32) -> Self {
        self.lead = Some(value.max(0.0));
        self
    }

    /// Sets `along`: where each item sits around its time.
    pub fn along(mut self, value: Place) -> Self {
        self.along = value;
        self
    }

    /// Sets `across`: where items sit across the track.
    pub fn across(mut self, value: Place) -> Self {
        self.across = value;
        self
    }

    /// Sets `flow`: new items glide in.
    pub fn flow(mut self, value: bool) -> Self {
        self.flow = value;
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

struct Shown {
    slot: Slot,
    time: f64,
}

struct State<T> {
    items: Signal<Vec<T>>,
    time: Box<dyn Fn(&T) -> f64>,
    key: Box<dyn Fn(&T) -> u64>,
    build: Box<dyn Fn(&T) -> El>,
    scale: f64,
    lead: f32,
    newest_at: NewestAt,
    along: Place,
    across: Place,
    flow: bool,
    back: Signal<f64>,
    /// The newest time the track is drawn at (gliding toward `head`).
    shown_head: Cell<f64>,
    head: Cell<f64>,
    oldest: Cell<f64>,
    size: Cell<Option<Size>>,
    shown: RefCell<HashMap<u64, Shown>>,
    order: RefCell<Vec<u64>>,
    near_oldest: RefCell<Option<Rc<dyn Fn()>>>,
    asked_at: Cell<Option<f64>>,
    content: El,
    el: El,
}

impl<T: Clone + 'static> State<T> {
    fn span(&self) -> f64 {
        self.size.get().map_or(0.0, |s| f64::from(s.width) / self.scale)
    }

    fn max_back(&self) -> f64 {
        (self.head.get() - self.oldest.get()).max(0.0)
    }

    /// The x of time `t` in the track, before right-to-left mirroring (the timeline is a
    /// picture of time, so its direction is its own).
    fn x_of(&self, t: f64, width: f32) -> f32 {
        let edge = self.shown_head.get() - untrack(|| self.back.get());
        let from_edge = ((edge - t) * self.scale) as f32;
        match self.newest_at {
            NewestAt::Right => width - self.lead - from_edge,
            NewestAt::Left => self.lead + from_edge,
        }
    }

    /// Builds the items in view (and half a view either side), drops the rest, and places
    /// them.
    fn refresh(&self) {
        let Some(size) = self.size.get() else { return };
        let span = self.span();
        let edge = self.shown_head.get() - untrack(|| self.back.get());
        let lead_t = f64::from(self.lead) / self.scale;
        let (lo, hi) = (edge - span - span * 0.5, edge + lead_t + span * 0.5);
        let mut want: Vec<(u64, f64, Option<T>)> = Vec::new();
        let have: Vec<u64> = self.shown.borrow().keys().copied().collect();
        let _ = self.items.with_untracked(|items| {
            let start = items.partition_point(|it| (self.time)(it) < lo);
            for it in items.iter().skip(start) {
                let t = (self.time)(it);
                if t > hi {
                    break;
                }
                let k = (self.key)(it);
                // Only items not shown yet are cloned, to be built.
                let fresh = !have.contains(&k);
                want.push((k, t, fresh.then(|| it.clone())));
            }
        });
        // Drop what left the view.
        {
            let mut shown = self.shown.borrow_mut();
            shown.retain(|k, s| {
                let keep = want.iter().any(|(w, _, _)| w == k);
                if !keep {
                    s.slot.el.remove();
                }
                keep
            });
        }
        // Build what entered, in time order, so reading order is time order.
        let rtl = self.el.is_rtl();
        let mut order = Vec::with_capacity(want.len());
        for (i, (k, t, item)) in want.into_iter().enumerate() {
            order.push(k);
            let mut shown = self.shown.borrow_mut();
            match shown.get_mut(&k) {
                Some(s) => s.time = t,
                None => {
                    let Some(item) = item else { continue };
                    drop(shown);
                    let child = untrack(|| (self.build)(&item));
                    let slot = Slot::new(self.el.ui(), child, self.along);
                    self.content.insert_child(i, &slot.el);
                    self.shown.borrow_mut().insert(k, Shown { slot, time: t });
                }
            }
        }
        *self.order.borrow_mut() = order;
        let (along, across) = match (self.along, self.newest_at) {
            // `Start` is the older side: the left when the newest are at the right.
            (Place::Start, NewestAt::Left) => (Place::End, self.across),
            (Place::End, NewestAt::Left) => (Place::Start, self.across),
            (p, _) => (p, self.across),
        };
        for s in self.shown.borrow().values() {
            let x = self.x_of(s.time, size.width);
            s.slot.put(Rect::new(x, 0.0, x, size.height), along, across, rtl, size.width);
        }
        // Near the oldest item: ask for more history, once per oldest item.
        let oldest = self.oldest.get();
        if lo < oldest
            && self.asked_at.get() != Some(oldest)
            && untrack(|| self.items.with(|v| !v.is_empty()))
        {
            self.asked_at.set(Some(oldest));
            let f = self.near_oldest.borrow().clone();
            if let Some(f) = f {
                let ui = self.el.ui().clone();
                // Outside this layout pass: the app may add items right away.
                let _ = ui.after(Duration::ZERO, move || f());
            }
        }
    }
}

/// A timeline made by [`timeline`].
pub struct Timeline<T: 'static> {
    state: Rc<State<T>>,
}

impl<T: 'static> Clone for Timeline<T> {
    fn clone(&self) -> Self {
        Timeline { state: self.state.clone() }
    }
}

impl<T: 'static> std::fmt::Debug for Timeline<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Timeline").field("head", &self.state.head.get()).finish_non_exhaustive()
    }
}

/// A horizontal track of `items` placed by `time` (any unit; seconds usually), newest at one
/// end ([`TimelineOptions::newest_at`]): messages drifting down a river, clips on a track.
///
/// `items` must be in time order, oldest first. Each item's element comes from `build` and is
/// kept by `key` while it stays near the view; only items in view and half a view either side
/// exist, so a long history stays fast. Items are read and focused in time order.
///
/// Drag the empty track or use the wheel to scrub back through time; Left and Right move by a
/// quarter of the view, Home goes to the oldest item and End back to the newest. While at the
/// newest end, new items flow in (gliding unless [`TimelineOptions::flow`] is off or motion is
/// reduced); while scrubbed back, the view stays put as items arrive. [`Timeline::back`] is
/// how far back the view is (0 at the newest end), for a "back to now" button.
#[track_caller]
pub fn timeline<T: Clone + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<T>>,
    time: impl Fn(&T) -> f64 + 'static,
    key: impl Fn(&T) -> u64 + 'static,
    build: impl Fn(&T) -> El + 'static,
    options: TimelineOptions,
) -> Timeline<T> {
    let label = label.into();
    let theme = ui.theme();
    let u = theme.spacing.unit;
    let content = layer(ui, "timeline_content");
    let el = ui
        .el("timeline")
        .fill_width()
        .clip()
        .focusable(true)
        .layout(|l| l.min_height = remedy_ui_core::layout::Length::Px(theme.spacing.control_height * 2.0))
        .child(content.clone());
    let head = untrack(|| items.with(|v| v.last().map_or(0.0, &time)));
    let state = Rc::new(State {
        items,
        time: Box::new(time),
        key: Box::new(key),
        build: Box::new(build),
        // As `TimelineOptions::scale` keeps it: no scale would put every item in view.
        scale: options.scale.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(f64::from(u * 4.0)),
        lead: options.lead.unwrap_or(u * 16.0),
        newest_at: options.newest_at,
        along: options.along,
        across: options.across,
        flow: options.flow,
        back: Signal::new(0.0),
        shown_head: Cell::new(head),
        head: Cell::new(head),
        oldest: Cell::new(head),
        size: Cell::new(None),
        shown: RefCell::new(HashMap::new()),
        order: RefCell::new(Vec::new()),
        near_oldest: RefCell::new(None),
        asked_at: Cell::new(None),
        content,
        el: el.clone(),
    });

    let s = state.clone();
    let el = el.on_resize(move |size| {
        s.size.set(Some(size));
        s.refresh();
    });
    let s = state.clone();
    // The glide follows data, so it settles even while ambient motion is off.
    let (el, motion) =
        Motion::attach_kind(el, options.clock, remedy_ui_engine::view::TickKind::Live, move |dt| {
            let (from, to) = (s.shown_head.get(), s.head.get());
            // A glide in screen pixels, so it settles the same at any scale.
            let px = smooth(
                (from - to) as f32 * s.scale as f32,
                0.0,
                dt,
                Duration::from_millis(90),
                Duration::from_millis(90),
            );
            let next = if px == 0.0 { to } else { to + f64::from(px) / s.scale };
            s.shown_head.set(next);
            s.refresh();
            Moved { changed: false, pending: next != to }
        });

    // Scrubbing.
    let sign = match state.newest_at {
        NewestAt::Right => 1.0,
        NewestAt::Left => -1.0,
    };
    let scrub = {
        let s = state.clone();
        move |by_px: f64| {
            let b = untrack(|| s.back.get()) + sign * by_px / s.scale;
            s.back.set_if_changed(b.clamp(0.0, s.max_back()));
        }
    };
    let grab: Rc<Cell<Option<(f32, f64)>>> = Rc::new(Cell::new(None));
    let (s, me, sc) = (state.clone(), el.clone(), scrub.clone());
    let sc2 = scrub.clone();
    let el = el
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down => {
                me.focus();
                grab.set(Some((e.position.x, untrack(|| s.back.get()))));
                true
            }
            PointerPhase::Move => match grab.get() {
                Some((x0, b0)) => {
                    let b = b0 + sign * f64::from(e.position.x - x0) / s.scale;
                    s.back.set_if_changed(b.clamp(0.0, s.max_back()));
                    true
                }
                None => false,
            },
            PointerPhase::Up | PointerPhase::Cancel => grab.replace(None).is_some(),
        })
        .on_wheel(move |e| {
            // Up or left goes back in time when the newest are at the right.
            let d = f64::from(-e.delta.y - sign as f32 * e.delta.x);
            if d == 0.0 {
                return false;
            }
            sc(d);
            true
        });
    let s = state.clone();
    let el = el.on_key(move |e| {
        if !e.pressed {
            return false;
        }
        let quarter = f64::from(s.size.get().map_or(0.0, |z| z.width)) / 4.0;
        match e.key {
            Key::ArrowLeft => sc2(quarter),
            Key::ArrowRight => sc2(-quarter),
            Key::Home => s.back.set_if_changed(s.max_back()),
            Key::End => s.back.set_if_changed(0.0),
            _ => return false,
        }
        true
    });

    // Items arriving: the newest end moves; a view scrubbed back stays where it is.
    let s = state.clone();
    let m2 = motion.clone();
    let el = el.bind_named("items", move || {
        let (oldest, newest) =
            s.items.with(|v| (v.first().map_or(0.0, |i| (s.time)(i)), v.last().map_or(0.0, |i| (s.time)(i))));
        let old_head = s.head.replace(newest);
        s.oldest.set(oldest);
        let back = untrack(|| s.back.get());
        if back > 0.0 && newest > old_head {
            s.back.set_if_changed(back + (newest - old_head));
        }
        let reduced = untrack(|| s.el.ui().reduced_motion());
        if !s.flow || reduced || back > 0.0 || s.size.get().is_none() {
            s.shown_head.set(newest);
        }
        s.refresh();
        m2.kick();
    });
    let s = state.clone();
    let el = el.bind_named("back", move || {
        let _ = s.back.get();
        s.refresh();
    });
    let s = state.clone();
    let _ = el.bind_named("semantics", move || {
        let back = s.back.get();
        let sem = Semantics::new(Role::List).label(label.clone());
        let sem = if back > 0.0 {
            sem.description(crate::stage::STRINGS.get(s.el.ui(), "timeline-scrolled-back"))
        } else {
            sem
        };
        let _ = s.el.clone().semantics(sem);
    });
    Timeline { state }
}

impl<T: Clone + 'static> Timeline<T> {
    /// How far back the view is, in time units (0 at the newest end). Set it to scrub (it is
    /// kept between 0 and the oldest item).
    pub fn back(&self) -> Signal<f64> {
        self.state.back
    }

    /// Returns to the newest end.
    pub fn go_live(&self) {
        self.state.back.set_if_changed(0.0);
    }

    /// Calls `f` when the view comes within half a view of the oldest item, once per oldest
    /// item: the moment to load more history and prepend it.
    pub fn on_near_oldest(self, f: impl Fn() + 'static) -> Self {
        *self.state.near_oldest.borrow_mut() = Some(Rc::new(f));
        self.state.asked_at.set(None);
        self.state.refresh();
        self
    }

    /// The keys of the items that exist now (in view or near it), in time order.
    pub fn shown_keys(&self) -> Vec<u64> {
        self.state.order.borrow().clone()
    }

    /// The timeline's element.
    pub fn el(&self) -> &El {
        &self.state.el
    }
}

impl<T: 'static> From<Timeline<T>> for El {
    fn from(t: Timeline<T>) -> El {
        t.state.el.clone()
    }
}
