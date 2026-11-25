//! Children on circles and ellipses, optionally turning.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::{Point, Rect, Size};
use remedy_ui_view::{El, Ui};
use remedy_ui_viz::{Clock, Moved};

use crate::common::{LineStyle, Motion, Place, Slot, ellipse, layer};

/// How an [`orbit`] lays out and turns.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct OrbitOptions {
    /// Where the first child of each ring sits, in degrees clockwise from the top.
    pub start_angle: f32,
    /// How long one full turn takes; `None` stands still. Turning stops under reduced motion
    /// and while the orbit is not seen.
    pub period: Option<Duration>,
    /// Turn counterclockwise instead of clockwise.
    pub counterclockwise: bool,
    /// Each ring's size as a fraction of the largest ellipse that fits, innermost first.
    /// Empty spaces the rings evenly (with three rings: 1/3, 2/3 and 1).
    pub rings: Vec<f32>,
    /// Space kept between the outer ring and the box's edge, so children on it stay inside.
    /// `None` takes eight theme units.
    pub inset: Option<f32>,
    /// Draws each ring that has children (a constellation's orbit lines). `None` draws none.
    pub ring_line: Option<LineStyle>,
    /// Where time comes from.
    pub clock: Clock,
}

impl Default for OrbitOptions {
    fn default() -> Self {
        OrbitOptions {
            start_angle: 0.0,
            period: None,
            counterclockwise: false,
            rings: Vec::new(),
            inset: None,
            ring_line: None,
            clock: Clock::system(),
        }
    }
}

impl OrbitOptions {
    /// Sets `start_angle`: where the first child sits, in degrees clockwise from the top.
    pub fn start_angle(mut self, value: f32) -> Self {
        self.start_angle = if value.is_finite() { value } else { 0.0 };
        self
    }

    /// Sets `period`: how long one full turn takes.
    pub fn period(mut self, value: Duration) -> Self {
        self.period = (!value.is_zero()).then_some(value);
        self
    }

    /// Sets `counterclockwise`.
    pub fn counterclockwise(mut self, value: bool) -> Self {
        self.counterclockwise = value;
        self
    }

    /// Sets `rings`: each ring's size as a fraction of the largest that fits, innermost first.
    pub fn rings(mut self, value: Vec<f32>) -> Self {
        self.rings = value;
        self
    }

    /// Sets `inset`: space between the outer ring and the box's edge.
    pub fn inset(mut self, value: f32) -> Self {
        self.inset = Some(value.max(0.0));
        self
    }

    /// Sets `ring_line`: draws the rings that have children.
    pub fn ring_line(mut self, value: LineStyle) -> Self {
        self.ring_line = Some(value);
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

struct Member {
    slot: Slot,
    ring: Option<usize>,
}

struct State {
    options: OrbitOptions,
    inset: f32,
    members: RefCell<Vec<Member>>,
    turn: Cell<f32>,
    size: Cell<Option<Size>>,
    content: El,
    el: El,
}

impl State {
    fn ring_count(&self) -> usize {
        let used = self.members.borrow().iter().filter_map(|m| m.ring).max().map_or(0, |r| r + 1);
        used.max(self.options.rings.len()).max(1)
    }

    fn fraction(&self, ring: usize) -> f32 {
        match self.options.rings.get(ring) {
            Some(f) if f.is_finite() => f.max(0.0),
            _ => (ring + 1) as f32 / self.ring_count() as f32,
        }
    }

    /// The center and the radii of `ring` in a box of `size`.
    fn ellipse(&self, size: Size, ring: usize) -> (Point, f32, f32) {
        let c = Point::new(size.width / 2.0, size.height / 2.0);
        let f = self.fraction(ring);
        let rx = (size.width / 2.0 - self.inset).max(0.0) * f;
        let ry = (size.height / 2.0 - self.inset).max(0.0) * f;
        (c, rx, ry)
    }

    fn put_all(&self) {
        let Some(size) = self.size.get() else { return };
        let rtl = self.el.is_rtl();
        let members = self.members.borrow();
        let rings = self.ring_count();
        let sign = if self.options.counterclockwise { -1.0 } else { 1.0 };
        for ring in 0..rings {
            let on: Vec<&Member> = members.iter().filter(|m| m.ring == Some(ring)).collect();
            let n = on.len().max(1) as f32;
            let (c, rx, ry) = self.ellipse(size, ring);
            for (k, m) in on.iter().enumerate() {
                let deg = self.options.start_angle + sign * self.turn.get() + 360.0 * k as f32 / n;
                let a = deg.to_radians();
                let p = Point::new(c.x + rx * a.sin(), c.y - ry * a.cos());
                m.slot.put(Rect::new(p.x, p.y, p.x, p.y), Place::Center, Place::Center, rtl, size.width);
            }
        }
        let c = Point::new(size.width / 2.0, size.height / 2.0);
        for m in members.iter().filter(|m| m.ring.is_none()) {
            m.slot.put(Rect::new(c.x, c.y, c.x, c.y), Place::Center, Place::Center, rtl, size.width);
        }
    }
}

/// Children on concentric circles or ellipses filling the box (a circle in a square box),
/// made by [`orbit`].
#[derive(Clone)]
pub struct Orbit {
    state: Rc<State>,
    motion: Motion,
}

impl std::fmt::Debug for Orbit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Orbit").field("turn", &self.state.turn.get()).finish_non_exhaustive()
    }
}

/// Children laid on rings around a center: the members of a call around its room, moons
/// around a planet. Each ring spaces its children evenly, starting at
/// [`OrbitOptions::start_angle`]; rings are nested (ring 0 innermost) and a child can sit at
/// the center. With a period, the rings turn slowly while seen, never under reduced motion,
/// and a still orbit does nothing.
///
/// Children keep their own size and are centered on their spot. They are read and focused in
/// the order added, whatever their angle. The orbit is a group named `label`.
#[track_caller]
pub fn orbit(ui: &Ui, label: impl Into<String>, options: OrbitOptions) -> Orbit {
    let label = label.into();
    let inset = options.inset.unwrap_or(ui.theme().spacing.unit * 8.0);
    let period = options.period;
    let clock = options.clock.clone();
    let content = layer(ui, "orbit_content");
    let el = ui.el("orbit").child(content.clone());
    let state = Rc::new(State {
        options,
        inset,
        members: RefCell::new(Vec::new()),
        turn: Cell::new(0.0),
        size: Cell::new(None),
        content,
        el: el.clone(),
    });
    let s = state.clone();
    let el = el.paint(move |cx| {
        let Some(line) = &s.options.ring_line else { return };
        let rings: Vec<usize> = {
            let mut r: Vec<usize> = s.members.borrow().iter().filter_map(|m| m.ring).collect();
            r.sort_unstable();
            r.dedup();
            r
        };
        for ring in rings {
            let (c, rx, ry) = s.ellipse(cx.size, ring);
            line.stroke(cx, ellipse(c, rx, ry));
        }
    });
    let s = state.clone();
    let el = el.on_resize(move |size| {
        s.size.set(Some(size));
        s.put_all();
    });
    let s = state.clone();
    let (el, motion) = Motion::attach(el, clock, move |dt| {
        let Some(period) = period else { return Moved::default() };
        let deg = 360.0 * dt.as_secs_f32() / period.as_secs_f32();
        s.turn.set((s.turn.get() + deg) % 360.0);
        s.put_all();
        Moved { changed: false, pending: true }
    });
    if !label.is_empty() {
        let _ = el.semantics(Semantics::new(Role::Group).label(label));
    }
    Orbit { state, motion }
}

impl Orbit {
    /// Adds a child to ring 0 (the only ring of a simple orbit).
    pub fn child(self, child: impl Into<El>) -> Self {
        self.child_on(0, child)
    }

    /// Adds a child to ring `ring` (0 is the innermost).
    pub fn child_on(self, ring: usize, child: impl Into<El>) -> Self {
        self.add(child.into(), Some(ring));
        self
    }

    /// Adds a child at the center.
    pub fn center(self, child: impl Into<El>) -> Self {
        self.add(child.into(), None);
        self
    }

    fn add(&self, child: El, ring: Option<usize>) {
        let slot = Slot::new(self.state.el.ui(), child, Place::Center);
        let _ = self.state.content.clone().child(slot.el.clone());
        self.state.members.borrow_mut().push(Member { slot, ring });
        self.state.put_all();
        self.state.el.repaint();
        self.motion.kick();
    }

    /// How far the rings have turned, in degrees.
    pub fn turn(&self) -> f32 {
        self.state.turn.get()
    }

    /// The orbit's element.
    pub fn el(&self) -> &El {
        &self.state.el
    }
}

impl From<Orbit> for El {
    fn from(o: Orbit) -> El {
        o.state.el.clone()
    }
}
