//! What moves the visuals over time: a clock, smoothing and the timer loop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui_view::El;

/// How often a moving visual is redrawn (about 60 times a second).
const STEP: Duration = Duration::from_millis(16);

/// A value closer than this to its target counts as there.
const SETTLE: f32 = 0.002;

enum Kind {
    System(Instant),
    Manual(Cell<Duration>),
}

/// Where the visuals read the time from.
///
/// The default is the system clock. A manual clock only moves when told to, so tests (and
/// apps that replay a recording) can step smoothing and peak hold exactly.
#[derive(Clone)]
pub struct Clock(Rc<Kind>);

impl Clock {
    /// The system clock, counting from now.
    pub fn system() -> Self {
        Clock(Rc::new(Kind::System(Instant::now())))
    }

    /// A clock that stands still until [`Clock::advance`] moves it.
    pub fn manual() -> Self {
        Clock(Rc::new(Kind::Manual(Cell::new(Duration::ZERO))))
    }

    /// The time since the clock was made.
    pub fn now(&self) -> Duration {
        match &*self.0 {
            Kind::System(start) => start.elapsed(),
            Kind::Manual(t) => t.get(),
        }
    }

    /// Moves a manual clock forward. Does nothing to the system clock.
    pub fn advance(&self, by: Duration) {
        if let Kind::Manual(t) = &*self.0 {
            t.set(t.get() + by);
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::system()
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &*self.0 {
            Kind::System(_) => f.write_str("Clock::system"),
            Kind::Manual(t) => write!(f, "Clock::manual({:?})", t.get()),
        }
    }
}

/// How fast a level rises and falls on screen.
///
/// A fast attack and a slower release keep a voice level from flickering: the display jumps
/// up with the voice and settles back gently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Smoothing {
    /// The time constant when the target is above the shown value.
    pub attack: Duration,
    /// The time constant when the target is below the shown value.
    pub release: Duration,
}

impl Smoothing {
    /// No smoothing: the shown value is the target.
    pub const NONE: Smoothing = Smoothing { attack: Duration::ZERO, release: Duration::ZERO };

    /// Fast rise, slower fall: for a voice ring.
    pub const fn voice() -> Self {
        Smoothing { attack: Duration::from_millis(35), release: Duration::from_millis(200) }
    }

    /// A rise that is almost instant and a fall that takes a moment: for meters and bars.
    pub const fn meter() -> Self {
        Smoothing { attack: Duration::from_millis(8), release: Duration::from_millis(260) }
    }

    /// The shown value after `dt`, moving from `current` toward `target`.
    pub fn step(&self, current: f32, target: f32, dt: Duration) -> f32 {
        smooth(current, target, dt, self.attack, self.release)
    }
}

/// One step of exponential smoothing toward `target` over `dt`, with separate time constants
/// for rising (`attack`) and falling (`release`). A zero constant jumps at once. A value
/// within a hair of the target lands on it, so smoothing always finishes.
pub fn smooth(current: f32, target: f32, dt: Duration, attack: Duration, release: Duration) -> f32 {
    if !current.is_finite() || !target.is_finite() {
        return if target.is_finite() { target } else { 0.0 };
    }
    let tau = if target > current { attack } else { release }.as_secs_f32();
    if tau <= 0.0 {
        return target;
    }
    let k = 1.0 - (-dt.as_secs_f32() / tau).exp();
    let next = current + (target - current) * k;
    if (target - next).abs() < SETTLE { target } else { next }
}

/// Decides when someone is speaking from a level, with hysteresis and a hang time so a
/// pause between words does not blink the ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    on_at: f32,
    off_at: f32,
    hang: Duration,
    open: bool,
    below: bool,
    quiet: Duration,
}

impl Gate {
    /// A closed gate that opens at `on_at` and starts closing when the level falls under
    /// 60 percent of that, closing after `hang`.
    pub fn new(on_at: f32, hang: Duration) -> Self {
        let on_at = on_at.clamp(0.0, 1.0);
        Gate { on_at, off_at: on_at * 0.6, hang, open: false, below: false, quiet: Duration::ZERO }
    }

    /// Takes the next level and the time since the last one. Returns whether it is open.
    pub fn feed(&mut self, level: f32, dt: Duration) -> bool {
        if level >= self.on_at {
            self.open = true;
            self.below = false;
            self.quiet = Duration::ZERO;
        } else if self.open {
            if level >= self.off_at {
                self.below = false;
                self.quiet = Duration::ZERO;
            } else {
                self.below = true;
                self.quiet += dt;
                if self.quiet >= self.hang {
                    self.open = false;
                    self.below = false;
                    self.quiet = Duration::ZERO;
                }
            }
        }
        self.open
    }

    /// True while the gate is open only because of the hang time, so a timer must keep
    /// feeding it.
    pub fn is_hanging(&self) -> bool {
        self.open && self.below
    }

    /// Whether the gate is open.
    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// What one step of a moving visual did.
#[derive(Clone, Copy, Debug, Default)]
pub struct Moved {
    /// Something on screen changed, so repaint.
    pub changed: bool,
    /// More steps are needed.
    pub pending: bool,
}

struct Inner {
    el: El,
    clock: Clock,
    running: Cell<bool>,
    last: Cell<Duration>,
    step: RefCell<Box<dyn FnMut(Duration) -> Moved>>,
}

/// Runs a step function on a timer, only while it asks for more.
///
/// Nothing is scheduled while a visual is at rest: a silent ring costs nothing. The loop ends
/// by itself when the element leaves the window.
#[derive(Clone)]
pub struct Ticker(Rc<Inner>);

impl Ticker {
    /// A ticker that repaints `el` after each step that changed something. `step` gets the
    /// time since its last call.
    pub fn new(el: &El, clock: Clock, step: impl FnMut(Duration) -> Moved + 'static) -> Self {
        Ticker(Rc::new(Inner {
            el: el.clone(),
            clock,
            running: Cell::new(false),
            last: Cell::new(Duration::ZERO),
            step: RefCell::new(Box::new(step)),
        }))
    }

    /// Takes a step now and keeps stepping while the step function wants more. Does nothing
    /// if the loop already runs.
    pub fn kick(&self) {
        if self.0.running.replace(true) {
            return;
        }
        self.0.last.set(self.0.clock.now());
        run(self.0.clone());
    }
}

fn run(inner: Rc<Inner>) {
    let ui = inner.el.ui().clone();
    if !ui.tree().contains(inner.el.id()) {
        inner.running.set(false);
        return;
    }
    let now = inner.clock.now();
    let dt = now.saturating_sub(inner.last.replace(now));
    let moved = match inner.step.try_borrow_mut() {
        Ok(mut f) => f(dt),
        Err(_) => Moved::default(),
    };
    if moved.changed {
        inner.el.repaint();
    }
    if moved.pending {
        let next = inner.clone();
        ui.after(STEP, move || run(next));
    } else {
        inner.running.set(false);
    }
}
