//! Ambient motion with a budget: decorative animation (a theme's fireflies, a flickering
//! fire, a drifting cloud) that runs at a throttled rate, only while it can be seen, and
//! stops costing anything the moment it cannot.
//!
//! - [`AmbientPolicy`] says how much ambient motion a window may have: full, light or off;
//!   how many frames a second while focused and while in the background; what to do on battery
//!   or power saver; and whether reduced motion stops it (it does by default).
//! - [`PaintCx::ambient`] is how a painter animates under it: it returns `None` while
//!   ambient motion is paused (draw a still frame) and otherwise the time to draw at, and it
//!   asks for the next repaint at the policy's rate, not every vsync. An element that calls it
//!   gets a compositor layer of its own, so its repaint re-rasters only itself.
//! - [`PaintCx::request_frame_in`] asks for a repaint after a delay (a blink, a flicker
//!   step). Both, and [`PaintCx::request_next_frame`], keep no frames coming while their
//!   element is scrolled out, clipped away or its window is hidden; they resume when it shows.
//! - [`El::ambient_loop`] runs pure transform and opacity motion (a beam sweep, a glow
//!   pulse, a drift) on the compositor: no UI-thread work per frame at all.
//! - [`El::resume_when`] and [`Ui::tick_interval`] are the same rules for timer-driven
//!   loops (the stage widgets and `viz` tickers use them).
//!
//! See `docs/design/09-performance-and-resources.md` (Ambient motion).

use std::collections::HashMap;
use std::time::Duration;

use remedy_ui_core::{Point, Rect};
use remedy_ui_tree::{Dirty, ElementId};

use crate::Request;
use crate::element::{El, Inner, PaintCx, Ui};

pub use remedy_ui_compositor::{AmbientLoop, LoopMotion, Wave};

/// How much ambient motion a window may have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AmbientMode {
    /// Ambient motion at the policy's full rate.
    #[default]
    Full,
    /// Fewer frames ([`AmbientPolicy::light_fps`]), and painters are told to draw lighter
    /// ([`AmbientTick::light`]: fewer particles, no secondary effects).
    Light,
    /// No ambient motion: everything shows a still frame.
    Off,
}

/// The budget for a window's ambient motion. Set it for the whole app with
/// `App::set_ambient_policy`, or for one view with [`Ui::set_ambient_policy`].
///
/// ```
/// use remedy_ui_view::{AmbientMode, AmbientPolicy};
///
/// let mut p = AmbientPolicy::default();
/// assert_eq!((p.fps, p.unfocused_fps), (30, 10));
/// p.mode = AmbientMode::Light;
/// p.power_saving_fps = Some(0); // still on battery
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AmbientPolicy {
    /// Full, light or off.
    pub mode: AmbientMode,
    /// Frames a second while the window has focus (default 30).
    pub fps: u32,
    /// The most frames a second in [`AmbientMode::Light`] (default 15).
    pub light_fps: u32,
    /// The most frames a second while the window is in the background but seen (default
    /// 10); 0 pauses ambient motion there.
    pub unfocused_fps: u32,
    /// The most frames a second while the system saves power (on battery, battery saver):
    /// `Some(0)` stills ambient motion then, `None` ignores power (default `Some(15)`, and
    /// painters are told to draw light).
    pub power_saving_fps: Option<u32>,
    /// Reduced motion (the system's or the app's) stops ambient motion (default true).
    pub follow_reduced_motion: bool,
    /// The longest step an [`AmbientTick::dt`] reports, so motion integrated with it does not
    /// jump after a pause (default [`MAX_AMBIENT_DT`], a quarter second).
    pub max_dt: Duration,
}

impl Default for AmbientPolicy {
    fn default() -> Self {
        AmbientPolicy {
            mode: AmbientMode::Full,
            fps: 30,
            light_fps: 15,
            unfocused_fps: 10,
            power_saving_fps: Some(15),
            follow_reduced_motion: true,
            max_dt: MAX_AMBIENT_DT,
        }
    }
}

/// The rate ambient motion runs at in a window now ([`Ui::ambient`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmbientRate {
    /// Time between ambient frames.
    pub interval: Duration,
    /// Draw lighter: fewer particles, no secondary effects.
    pub light: bool,
    /// The longest step a tick reports ([`AmbientPolicy::max_dt`]).
    pub max_dt: Duration,
}

/// What [`PaintCx::ambient`] gives a painter for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct AmbientTick {
    /// The frame's time on the window's frame clock: sample loops with it.
    pub t: Duration,
    /// Time since this element's last ambient frame (zero on the first, and never more than
    /// [`AmbientPolicy::max_dt`], a quarter second by default, so a step after a pause does
    /// not jump): integrate with it.
    pub dt: Duration,
    /// Draw lighter (the window is in light mode or saving power).
    pub light: bool,
}

/// The longest `dt` an [`AmbientTick`] reports unless the policy says otherwise
/// ([`AmbientPolicy::max_dt`]).
pub const MAX_AMBIENT_DT: Duration = Duration::from_millis(250);
/// Wakes this close together are served by one frame.
const SLACK: Duration = Duration::from_millis(4);
/// How often live data visuals step while their window has focus.
const LIVE_STEP: Duration = Duration::from_millis(16);

/// What kind of loop a timer-driven animation is ([`Ui::tick_interval`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TickKind {
    /// Decoration (a sprite's frames, an orbit, a marquee): follows the [`AmbientPolicy`].
    Ambient,
    /// Live data (a level meter): about 60 a second with focus, slower in the background,
    /// paused while the window is hidden; ambient mode and reduced motion do not stop it.
    Live,
    /// Anything that only needs to be seen (a blink, a timed repaint).
    Seen,
}

/// The window facts the policy decides from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Activity {
    pub(crate) focused: bool,
    pub(crate) occluded: bool,
    pub(crate) reduced_motion: bool,
    pub(crate) power_saving: bool,
}

impl AmbientPolicy {
    /// Ambient motion off.
    pub fn off() -> Self {
        AmbientPolicy { mode: AmbientMode::Off, ..AmbientPolicy::default() }
    }

    pub(crate) fn rate(&self, a: Activity) -> Option<AmbientRate> {
        if a.occluded || (self.follow_reduced_motion && a.reduced_motion) {
            return None;
        }
        let mut fps = match self.mode {
            AmbientMode::Off => return None,
            AmbientMode::Full => self.fps,
            AmbientMode::Light => self.fps.min(self.light_fps),
        };
        if !a.focused {
            fps = fps.min(self.unfocused_fps);
        }
        let saving = a.power_saving && self.power_saving_fps.is_some();
        if saving {
            fps = fps.min(self.power_saving_fps.unwrap_or(fps));
        }
        (fps > 0).then(|| AmbientRate {
            interval: Duration::from_secs(1) / fps,
            light: self.mode == AmbientMode::Light || saving,
            max_dt: self.max_dt,
        })
    }

    pub(crate) fn live(&self, a: Activity) -> Option<Duration> {
        if a.occluded {
            None
        } else if a.focused {
            Some(LIVE_STEP)
        } else {
            Some(Duration::from_secs(1) / self.unfocused_fps.clamp(10, 60))
        }
    }
}

type Parked = Box<dyn FnOnce(&Ui)>;

/// A view's ambient state.
pub(crate) struct AmbientState {
    pub(crate) policy: AmbientPolicy,
    pub(crate) focused: bool,
    pub(crate) power_saving: bool,
    pub(crate) rate: remedy_ui_reactive::Signal<Option<AmbientRate>>,
    /// Painters due again, by when (frame clock).
    pub(crate) wakes: HashMap<ElementId, Duration>,
    /// The timer serving the earliest wake, and its time.
    pub(crate) timer: Option<(u64, Duration)>,
    /// Painters waiting to be seen (or for ambient motion to resume).
    pub(crate) parked_paint: HashMap<ElementId, TickKind>,
    /// Loops waiting the same way.
    pub(crate) parked: Vec<(ElementId, TickKind, Parked)>,
}

impl Default for AmbientState {
    fn default() -> Self {
        let policy = AmbientPolicy::default();
        let a = Activity { focused: true, occluded: false, reduced_motion: false, power_saving: false };
        AmbientState {
            policy,
            focused: true,
            power_saving: false,
            rate: remedy_ui_reactive::Signal::new(policy.rate(a)),
            wakes: HashMap::new(),
            timer: None,
            parked_paint: HashMap::new(),
            parked: Vec::new(),
        }
    }
}

pub(crate) fn activity(i: &Inner) -> Activity {
    Activity {
        focused: i.ambient.focused,
        occluded: i.occluded,
        reduced_motion: i.reduced_motion.get_untracked().unwrap_or(false),
        power_saving: i.ambient.power_saving,
    }
}

/// True when a loop of `kind` may run in `i` now (whether or not its element is seen).
fn may_run(i: &Inner, kind: TickKind) -> bool {
    let a = activity(i);
    match kind {
        TickKind::Ambient => i.ambient.policy.rate(a).is_some(),
        TickKind::Live => i.ambient.policy.live(a).is_some(),
        TickKind::Seen => !a.occluded,
    }
}

impl Ui {
    /// Sets how much ambient motion this window may have (the runner sets the app's).
    pub fn set_ambient_policy(&self, policy: AmbientPolicy) {
        self.0.borrow_mut().ambient.policy = policy;
        self.refresh_ambient();
    }

    /// The window's ambient policy.
    pub fn ambient_policy(&self) -> AmbientPolicy {
        self.0.borrow().ambient.policy
    }

    /// Tells the view the system is saving power (on battery or in a power-saver mode; the
    /// runner does, from the platform where it can tell).
    pub fn set_power_saving(&self, on: bool) {
        self.0.borrow_mut().ambient.power_saving = on;
        self.refresh_ambient();
    }

    /// True while the system saves power, as last told.
    pub fn is_power_saving(&self) -> bool {
        self.0.borrow().ambient.power_saving
    }

    /// The rate ambient motion runs at now, or `None` while it is paused (off, reduced
    /// motion, a hidden window, a background window whose policy pauses it). Tracked: a
    /// binding that reads it runs again when it changes.
    pub fn ambient(&self) -> Option<AmbientRate> {
        let s = self.0.borrow().ambient.rate;
        s.get()
    }

    /// How long a timer-driven loop of `kind` waits between steps now, or `None` while it
    /// should not run (untracked). It says nothing about whether its element is seen: see
    /// [`El::is_seen`].
    pub fn tick_interval(&self, kind: TickKind) -> Option<Duration> {
        let i = self.0.borrow();
        let a = activity(&i);
        match kind {
            TickKind::Ambient => i.ambient.policy.rate(a).map(|r| r.interval),
            TickKind::Live => i.ambient.policy.live(a),
            TickKind::Seen => (!a.occluded).then_some(LIVE_STEP),
        }
    }

    /// Recomputes the ambient rate after something it depends on changed, and wakes what
    /// waited for it.
    pub(crate) fn refresh_ambient(&self) {
        let (signal, rate) = {
            let i = self.0.borrow();
            (i.ambient.rate, i.ambient.policy.rate(activity(&i)))
        };
        if signal.get_untracked().ok() != Some(rate) {
            signal.set(rate);
        }
        self.wake_parked();
    }

    pub(crate) fn note_window_focused(&self, focused: bool) {
        let changed = {
            let mut i = self.0.borrow_mut();
            std::mem::replace(&mut i.ambient.focused, focused) != focused
        };
        if changed {
            self.refresh_ambient();
        }
    }

    /// Runs (or repaints) what was parked and may go now: its element is seen and its kind
    /// may run. Parked entries of removed elements are dropped.
    pub(crate) fn wake_parked(&self) {
        let (ready, paint) = {
            let mut guard = self.0.borrow_mut();
            let i = &mut *guard;
            if i.ambient.parked.is_empty() && i.ambient.parked_paint.is_empty() {
                return;
            }
            let window = Rect::from_origin_size(Point::ZERO, i.size);
            let parked = std::mem::take(&mut i.ambient.parked);
            let mut ready = Vec::new();
            let mut keep = Vec::new();
            for (id, kind, f) in parked {
                if !i.els.contains_key(&id) {
                    continue;
                }
                if may_run(i, kind) && crate::frame::is_seen(i, id, window) {
                    ready.push(f);
                } else {
                    keep.push((id, kind, f));
                }
            }
            i.ambient.parked = keep;
            let mut paint = Vec::new();
            let pp = std::mem::take(&mut i.ambient.parked_paint);
            for (id, kind) in pp {
                if !i.els.contains_key(&id) {
                    continue;
                }
                if may_run(i, kind) && crate::frame::is_seen(i, id, window) {
                    paint.push(id);
                } else {
                    i.ambient.parked_paint.insert(id, kind);
                }
            }
            (ready, paint)
        };
        for id in paint {
            self.mark(id, Dirty::PAINT);
        }
        for f in ready {
            f(self);
        }
    }

    /// Makes sure a timer serves the earliest painter wake.
    pub(crate) fn arm_wakes(&self, now: Duration) {
        let (earliest, current) = {
            let i = self.0.borrow();
            (i.ambient.wakes.values().min().copied(), i.ambient.timer)
        };
        let Some(due) = earliest else { return };
        if current.is_some_and(|(_, at)| at <= due + SLACK) {
            return;
        }
        if let Some((id, _)) = current {
            self.cancel_timer(id);
        }
        let weak = self.downgrade();
        let id = self.after(due.saturating_sub(now), move || {
            if let Some(ui) = weak.upgrade() {
                ui.fire_wakes(due);
            }
        });
        self.0.borrow_mut().ambient.timer = Some((id, due));
    }

    fn fire_wakes(&self, due: Duration) {
        let ids: Vec<ElementId> = {
            let mut i = self.0.borrow_mut();
            if i.ambient.timer.is_some_and(|(_, at)| at == due) {
                i.ambient.timer = None;
            }
            let mut ids = Vec::new();
            i.ambient.wakes.retain(|id, at| {
                let go = *at <= due + SLACK;
                if go {
                    ids.push(*id);
                }
                !go
            });
            ids.retain(|id| i.els.contains_key(id));
            ids
        };
        for id in ids {
            self.mark(id, Dirty::PAINT);
        }
        self.arm_wakes(due);
    }

    /// Ambient painters waiting for their next frame, and painters and loops waiting to be
    /// seen (for tests and the performance overlay).
    pub fn ambient_waiting(&self) -> (usize, usize) {
        let i = self.0.borrow();
        (i.ambient.wakes.len(), i.ambient.parked.len() + i.ambient.parked_paint.len())
    }
}

/// What one painter asked of the frame clock ([`PaintCx::ambient`] and friends).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PaintAsk {
    /// Draw again after this long.
    pub(crate) wake_in: Option<Duration>,
    /// It asked for ambient motion (whether or not it got it).
    pub(crate) ambient: bool,
    /// It got an ambient tick.
    pub(crate) ticked: bool,
}

impl PaintCx<'_> {
    /// Ambient motion for this frame: `None` while it is paused (draw a still frame; the
    /// element is drawn again when motion resumes), otherwise the time to draw at. The
    /// element is drawn again at the window's ambient rate ([`AmbientPolicy`]) while it is
    /// seen, and gets a compositor layer of its own (clipped to its box), so drawing it again
    /// re-rasters only it.
    ///
    /// ```ignore
    /// el.paint(|cx| {
    ///     let t = cx.ambient().map_or(0.0, |a| a.t.as_secs_f32());
    ///     draw_flame(cx, flicker(t, 7));
    /// })
    /// ```
    pub fn ambient(&mut self) -> Option<AmbientTick> {
        self.ambient_at(u32::MAX)
    }

    /// Like [`PaintCx::ambient`], at most `fps` frames a second (a flame that flickers 12
    /// times a second needs no more, whatever the policy allows).
    pub fn ambient_at(&mut self, fps: u32) -> Option<AmbientTick> {
        self.ask.ambient = true;
        let rate = self.ambient_rate?;
        let floor = if fps == 0 || fps == u32::MAX { Duration::ZERO } else { Duration::from_secs(1) / fps };
        self.wake(rate.interval.max(floor));
        self.ask.ticked = true;
        let dt = self.last_tick.map_or(Duration::ZERO, |l| self.time.saturating_sub(l)).min(rate.max_dt);
        Some(AmbientTick { t: self.time, dt, light: rate.light })
    }

    /// Draws this element again after `delay` (once; ask again to keep going). Nothing is
    /// scheduled while the element is not seen: it is drawn again when it shows.
    pub fn request_frame_in(&mut self, delay: Duration) {
        self.wake(delay);
    }

    fn wake(&mut self, d: Duration) {
        self.ask.wake_in = Some(self.ask.wake_in.map_or(d, |w| w.min(d)));
    }
}

impl El {
    /// True when some of the element shows: it and its parents are not hidden or
    /// transparent, it is inside the window and every clipping parent, and the window is not
    /// minimized or covered (as of the last frame).
    pub fn is_seen(&self) -> bool {
        let i = self.ui.0.borrow();
        let window = Rect::from_origin_size(Point::ZERO, i.size);
        crate::frame::is_seen(&i, self.id, window)
    }

    /// Runs `f` once the element is seen and a loop of `kind` may run (at once if both hold
    /// now, at the end of the frame or policy change that makes them hold otherwise): a timer
    /// loop that found itself unseen or paused parks here instead of polling.
    pub fn resume_when(&self, kind: TickKind, f: impl FnOnce() + 'static) {
        self.ui.0.borrow_mut().ambient.parked.push((self.id, kind, Box::new(move |_| f())));
        self.ui.wake_parked();
    }

    /// Runs `l` on the compositor, forever: a transform or opacity loop of this element (a
    /// beam's sweep, a glow's pulse, a slow drift) with no work on the UI thread per frame.
    /// The element is painted into a layer of its own, clipped to its box; draw what moves
    /// inside it. Loops run at the window's ambient rate and only while seen; they hold still
    /// while ambient motion is paused and rest at their start under reduced motion. Several
    /// loops compose in order.
    ///
    /// ```ignore
    /// let beam = ui.el("beam").fill().paint(draw_beam)
    ///     .ambient_loop(AmbientLoop::spin(pivot, Duration::from_secs(8)));
    /// ```
    pub fn ambient_loop(self, l: AmbientLoop) -> Self {
        self.edit(Dirty::PAINT, |d| {
            d.isolate.get_or_insert(remedy_ui_paint::Affine::IDENTITY);
            d.loops.push(l);
        });
        self.ui.0.borrow_mut().layers_changed = true;
        self
    }

    /// Replaces the element's compositor loops (none stops them).
    pub fn set_ambient_loops(&self, loops: Vec<AmbientLoop>) {
        let changed = {
            let mut i = self.ui.0.borrow_mut();
            let changed = match i.els.get_mut(&self.id) {
                Some(d) if d.loops != loops => {
                    if !loops.is_empty() {
                        d.isolate.get_or_insert(remedy_ui_paint::Affine::IDENTITY);
                    }
                    d.loops = loops;
                    true
                }
                _ => false,
            };
            i.layers_changed |= changed;
            changed
        };
        if changed {
            self.ui.request(Request::Frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(focused: bool, occluded: bool, reduced: bool, power: bool) -> Activity {
        Activity { focused, occluded, reduced_motion: reduced, power_saving: power }
    }

    fn fps(r: Option<AmbientRate>) -> u32 {
        r.map_or(0, |r| (1.0 / r.interval.as_secs_f64()).round() as u32)
    }

    #[test]
    fn the_policy_throttles_pauses_and_lightens() {
        let p = AmbientPolicy::default();
        assert_eq!(fps(p.rate(act(true, false, false, false))), 30);
        assert_eq!(fps(p.rate(act(false, false, false, false))), 10);
        assert_eq!(p.rate(act(true, true, false, false)), None, "hidden");
        assert_eq!(p.rate(act(true, false, true, false)), None, "reduced motion wins");
        let r = p.rate(act(true, false, false, true));
        assert_eq!(fps(r), 15);
        assert!(r.is_some_and(|r| r.light), "power saving draws light");
        let mut q = p;
        q.follow_reduced_motion = false;
        assert_eq!(fps(q.rate(act(true, false, true, false))), 30);
        q.mode = AmbientMode::Light;
        assert!(q.rate(act(true, false, false, false)).is_some_and(|r| r.light));
        assert_eq!(fps(q.rate(act(true, false, false, false))), 15);
        q.power_saving_fps = Some(0);
        assert_eq!(q.rate(act(true, false, false, true)), None, "still on battery");
        q.unfocused_fps = 0;
        assert_eq!(q.rate(act(false, false, false, false)), None, "paused in the background");
        assert_eq!(AmbientPolicy::off().rate(act(true, false, false, false)), None);
    }

    #[test]
    fn live_data_ignores_mode_and_reduced_motion_but_not_hiding() {
        let p = AmbientPolicy::off();
        assert_eq!(p.live(act(true, false, true, true)), Some(LIVE_STEP));
        assert_eq!(p.live(act(false, false, false, false)), Some(Duration::from_secs(1) / 10));
        assert_eq!(p.live(act(true, true, false, false)), None);
    }
}
