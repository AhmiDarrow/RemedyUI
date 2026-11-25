//! Element animation: a property, style value or offset moves to a target with a spring or
//! an easing curve from `remedy-ui-core::motion`, sampled at each frame's time.
//!
//! [`El::animate`] starts a move. A new target for the same property starts from the value
//! and velocity it has, so an interrupted move turns smoothly instead of jumping. When the
//! window asks for reduced motion ([`Ui::set_reduced_motion`]), moves are instant. Moves
//! run on the window's frame clock ([`Ui::frame_time`]), which the runner sets to each
//! frame's predicted presentation time; nothing needs a timer, and a window with nothing
//! moving asks for no frames.
//!
//! ```
//! use remedy_ui_engine::view::motion::{Motion, Spring, Target};
//! # use remedy_ui_engine::view::Ui;
//! # fn demo(ui: &Ui) {
//! let card = ui.el("card");
//! card.animate(Target::Opacity(0.0), Motion::Spring(Spring::smooth(std::time::Duration::from_millis(200))));
//! # }
//! ```

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub use remedy_ui_core::motion::{Animatable, Animation, CubicBezier, Easing, FrameClock, Motion, Spring};
use remedy_ui_core::style::StylePatch;
use remedy_ui_core::time::Instant;
use remedy_ui_core::tree::Dirty;
use remedy_ui_core::{Color, Insets, Vec2};

use crate::view::element::{El, ElData, Ui};

/// How often [`El::animate_keyframes`] plays its steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Repeat {
    /// Until it is stopped.
    Forever,
    /// This many times through (at least once).
    Times(u32),
}

/// A running [`El::animate_keyframes`].
#[derive(Clone, Debug)]
pub struct Keyframes(Rc<Cell<bool>>);

impl Keyframes {
    /// Stops after the step now playing: the property stays where that step leaves it.
    pub fn stop(&self) {
        self.0.set(true);
    }

    /// True once it was stopped or has played as often as asked.
    pub fn is_stopped(&self) -> bool {
        self.0.get()
    }
}

/// Plays step `at`, then the one after it (or the first again, while `left` allows).
fn play(el: El, steps: Rc<Vec<(Target, Motion)>>, at: usize, left: Repeat, handle: Keyframes) {
    if handle.is_stopped() {
        return;
    }
    let Some((target, motion)) = steps.get(at).copied() else { return };
    let next = el.clone();
    el.animate_then(target, motion, move || {
        if at + 1 < steps.len() {
            return play(next, steps, at + 1, left, handle);
        }
        match left {
            Repeat::Forever => play(next, steps, 0, left, handle),
            Repeat::Times(n) if n > 1 => play(next, steps, 0, Repeat::Times(n - 1), handle),
            Repeat::Times(_) => handle.stop(),
        }
    });
}

/// What an animation moves, and to what.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Target {
    /// Opacity, 0 to 1.
    Opacity(f32),
    /// Background fill.
    Background(Color),
    /// Text and icon color.
    Foreground(Color),
    /// Border color.
    Border(Color),
    /// Border width.
    BorderWidth(f32),
    /// Corner radius.
    Radius(f32),
    /// Padding.
    Padding(Insets),
    /// Where it is drawn, offset from its laid-out place (layout does not move).
    Translate(Vec2),
    /// How large it is drawn, about its center (1 is its laid-out size).
    Scale(f32),
    /// How far its box shadow grows before it blurs (a glow that swells). The rest of the
    /// shadow is the element's own as it is when the move starts.
    ShadowSpread(f32),
}

impl Target {
    fn key(&self) -> u8 {
        match self {
            Target::Opacity(_) => 0,
            Target::Background(_) => 1,
            Target::Foreground(_) => 2,
            Target::Border(_) => 3,
            Target::BorderWidth(_) => 4,
            Target::Radius(_) => 5,
            Target::Padding(_) => 6,
            Target::Translate(_) => 7,
            Target::Scale(_) => 8,
            Target::ShadowSpread(_) => 9,
        }
    }

    /// The targets a style patch names (its animatable properties).
    pub fn from_style(p: &StylePatch) -> Vec<Target> {
        let mut out = Vec::new();
        out.extend(p.opacity.map(Target::Opacity));
        out.extend(p.background.map(Target::Background));
        out.extend(p.foreground.map(Target::Foreground));
        out.extend(p.border.map(Target::Border));
        out.extend(p.border_width.map(Target::BorderWidth));
        out.extend(p.radius.map(Target::Radius));
        out.extend(p.padding.map(Target::Padding));
        out
    }
}

/// How fast a theme's motion token moves ([`Ui::motion`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pace {
    /// Hover and press feedback.
    Quick,
    /// Most transitions.
    Standard,
    /// Large moves and page transitions.
    Emphasized,
}

enum Slot {
    F32(Animation<f32>),
    Color(Animation<Color>),
    Vec2(Animation<Vec2>),
    Insets(Animation<Insets>),
}

/// One property animating on an element.
pub(crate) struct Running {
    key: u8,
    slot: Slot,
    done: Vec<Box<dyn FnOnce()>>,
}

/// The window's frame clock: set by the runner each frame, or wall time since the view
/// was made when nothing sets it (tools, simple hosts).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    set: Option<Duration>,
    start: Instant,
}

impl Default for Clock {
    fn default() -> Self {
        Clock { set: None, start: Instant::now() }
    }
}

impl Clock {
    pub(crate) fn now(&self) -> Duration {
        self.set.unwrap_or_else(|| self.start.elapsed())
    }
}

/// The value `t` names, as the element has it now.
fn current(d: &ElData, t: &Target) -> Target {
    let s = &d.resolved;
    let a = &d.anim_style;
    match t {
        Target::Opacity(_) => Target::Opacity(a.opacity.unwrap_or(s.opacity)),
        Target::Background(_) => Target::Background(a.background.unwrap_or(s.background)),
        Target::Foreground(_) => Target::Foreground(a.foreground.unwrap_or(s.foreground)),
        Target::Border(_) => Target::Border(a.border.unwrap_or(s.border)),
        Target::BorderWidth(_) => Target::BorderWidth(a.border_width.unwrap_or(s.border_width)),
        Target::Radius(_) => Target::Radius(a.radius.unwrap_or(s.radius)),
        Target::Padding(_) => Target::Padding(a.padding.unwrap_or(s.padding)),
        Target::Translate(_) => Target::Translate(d.shift.0),
        Target::Scale(_) => Target::Scale(d.shift.1),
        Target::ShadowSpread(_) => Target::ShadowSpread(a.shadow.unwrap_or(s.shadow).spread),
    }
}

fn slot_for(from: Target) -> Slot {
    match from {
        Target::Opacity(v)
        | Target::BorderWidth(v)
        | Target::Radius(v)
        | Target::Scale(v)
        | Target::ShadowSpread(v) => Slot::F32(Animation::new(v)),
        Target::Background(c) | Target::Foreground(c) | Target::Border(c) => Slot::Color(Animation::new(c)),
        Target::Padding(i) => Slot::Insets(Animation::new(i)),
        Target::Translate(v) => Slot::Vec2(Animation::new(v)),
    }
}

impl Slot {
    fn retarget(&mut self, t: Target, motion: Motion, now: Duration, reduced: bool) {
        match (self, t) {
            (
                Slot::F32(a),
                Target::Opacity(v)
                | Target::BorderWidth(v)
                | Target::Radius(v)
                | Target::Scale(v)
                | Target::ShadowSpread(v),
            ) => {
                a.set_reduced_motion(reduced);
                a.animate_to(v, motion, now);
            }
            (Slot::Color(a), Target::Background(c) | Target::Foreground(c) | Target::Border(c)) => {
                a.set_reduced_motion(reduced);
                a.animate_to(c, motion, now);
            }
            (Slot::Insets(a), Target::Padding(i)) => {
                a.set_reduced_motion(reduced);
                a.animate_to(i, motion, now);
            }
            (Slot::Vec2(a), Target::Translate(v)) => {
                a.set_reduced_motion(reduced);
                a.animate_to(v, motion, now);
            }
            _ => {}
        }
    }

    fn settled(&self, now: Duration) -> bool {
        match self {
            Slot::F32(a) => a.is_settled(now),
            Slot::Color(a) => a.is_settled(now),
            Slot::Vec2(a) => a.is_settled(now),
            Slot::Insets(a) => a.is_settled(now),
        }
    }

    /// Writes the value at `now` into the element; true when the style changed.
    fn apply(&self, key: u8, d: &mut ElData, now: Duration) -> bool {
        let s = &mut d.anim_style;
        match (key, self) {
            (0, Slot::F32(a)) => s.opacity = Some(a.sample(now).clamp(0.0, 1.0)),
            (4, Slot::F32(a)) => s.border_width = Some(a.sample(now).max(0.0)),
            (5, Slot::F32(a)) => s.radius = Some(a.sample(now).max(0.0)),
            (8, Slot::F32(a)) => {
                d.shift.1 = a.sample(now);
                return false;
            }
            (9, Slot::F32(a)) => {
                let base = s.shadow.unwrap_or(d.resolved.shadow);
                s.shadow = Some(base.spread(a.sample(now)));
            }
            (1, Slot::Color(a)) => s.background = Some(a.sample(now)),
            (2, Slot::Color(a)) => s.foreground = Some(a.sample(now)),
            (3, Slot::Color(a)) => s.border = Some(a.sample(now)),
            (6, Slot::Insets(a)) => s.padding = Some(a.sample(now)),
            (7, Slot::Vec2(a)) => {
                d.shift.0 = a.sample(now);
                return false;
            }
            _ => return false,
        }
        true
    }
}

/// Starts a pending FLIP of `d`, now laid out at `now_at`: it is drawn at its old place and
/// moves to the new one. True when it moved.
pub(crate) fn start_flip(d: &mut ElData, now_at: remedy_ui_core::Point, now: Duration) -> bool {
    let Some((old, motion)) = d.flip.take() else { return false };
    let delta = Vec2::new(old.x - now_at.x, old.y - now_at.y);
    if delta.x.abs() < 0.5 && delta.y.abs() < 0.5 {
        return false;
    }
    // From where it was drawn (its old place, plus any offset still playing) to rest.
    let from = Vec2::new(delta.x + d.shift.0.x, delta.y + d.shift.0.y);
    let mut a = Animation::new(from);
    a.animate_to(Vec2::ZERO, motion, now);
    d.shift.0 = from;
    let key = Target::Translate(Vec2::ZERO).key();
    let done =
        d.anims.iter_mut().find(|r| r.key == key).map(|r| std::mem::take(&mut r.done)).unwrap_or_default();
    d.anims.retain(|r| r.key != key);
    d.anims.push(Running { key, slot: Slot::Vec2(a), done });
    true
}

impl Ui {
    /// The frame clock's time: the predicted presentation time of the frame being built
    /// (what animations and painters sample).
    pub fn frame_time(&self) -> Duration {
        self.0.borrow().clock.now()
    }

    /// Sets the frame clock (the runner does, before each frame, to the frame's predicted
    /// presentation time; tests step animations with it).
    pub fn set_frame_time(&self, t: Duration) {
        self.0.borrow_mut().clock.set = Some(t);
    }

    /// The theme's motion for `pace`: a spring of the token's duration with no bounce.
    pub fn motion(&self, pace: Pace) -> Motion {
        let m = self.0.borrow().theme.motion;
        let d = match pace {
            Pace::Quick => m.quick,
            Pace::Standard => m.standard,
            Pace::Emphasized => m.emphasized,
        };
        Motion::Spring(Spring::smooth(d))
    }

    /// True while some element is animating.
    pub fn is_animating(&self) -> bool {
        self.0.borrow().els.values().any(|d| !d.anims.is_empty())
    }

    /// Advances every animation to the frame's time, writing their values; returns the
    /// completion callbacks of those that finished. Called at the start of each frame.
    pub(crate) fn tick_animations(&self) -> Vec<Box<dyn FnOnce()>> {
        let mut done = Vec::new();
        let mut i = self.0.borrow_mut();
        let now = i.clock.now();
        let mut restyle = Vec::new();
        let mut moved = false;
        let mut running = false;
        for (id, d) in &mut i.els {
            if d.anims.is_empty() {
                continue;
            }
            let anims = std::mem::take(&mut d.anims);
            let mut keep = Vec::with_capacity(anims.len());
            for mut r in anims {
                if r.slot.apply(r.key, d, now) {
                    restyle.push(*id);
                } else {
                    moved = true;
                }
                if r.slot.settled(now) {
                    done.append(&mut r.done);
                } else {
                    keep.push(r);
                }
            }
            running |= !keep.is_empty();
            d.anims = keep;
        }
        for id in restyle {
            i.tree.mark(id, Dirty::STYLE | Dirty::PAINT);
        }
        if moved {
            i.layers_changed = true;
        }
        drop(i);
        if running {
            self.request(crate::view::Request::Frame);
        }
        done
    }
}

impl El {
    /// Moves `target`'s property to its value with `motion`, from wherever it is now (with
    /// its velocity, when it was moving). Under reduced motion the move is instant.
    pub fn animate(&self, target: Target, motion: Motion) {
        self.animate_then(target, motion, || {});
    }

    /// Like [`El::animate`], calling `done` once the value has arrived (or the move was
    /// replaced by none: it is called when this property's motion settles).
    pub fn animate_then(&self, target: Target, motion: Motion, done: impl FnOnce() + 'static) {
        let reduced = self.ui.reduced_motion();
        {
            let mut i = self.ui.0.borrow_mut();
            let now = i.clock.now();
            let Some(d) = i.els.get_mut(&self.id) else { return };
            let key = target.key();
            let at = match d.anims.iter().position(|r| r.key == key) {
                Some(at) => at,
                None => {
                    let from = current(d, &target);
                    d.anims.push(Running { key, slot: slot_for(from), done: Vec::new() });
                    d.anims.len() - 1
                }
            };
            if let Some(r) = d.anims.get_mut(at) {
                r.slot.retarget(target, motion, now, reduced);
                r.done.push(Box::new(done));
            }
        }
        self.ui.request(crate::view::Request::Frame);
    }

    /// Plays `steps` one after the other (each a value and the motion that takes the
    /// property there), then again from the first, as often as `repeat` says: a pulse, a
    /// breathing glow, a shake. Give the first step the value the last one leaves from to
    /// loop without a jump. It stops with the returned handle, or when the element is
    /// removed. Under reduced motion nothing moves: the property takes the last step's value.
    pub fn animate_keyframes(&self, steps: Vec<(Target, Motion)>, repeat: Repeat) -> Keyframes {
        let handle = Keyframes(Rc::new(Cell::new(false)));
        match steps.last() {
            None => handle.stop(),
            Some((last, _)) if self.ui.reduced_motion() => {
                self.set_animated(*last);
                handle.stop();
            }
            Some(_) => play(self.clone(), Rc::new(steps), 0, repeat, handle.clone()),
        }
        handle
    }

    /// Animates every animatable property `patch` sets (opacity, colors, border, radius,
    /// padding) with `motion`.
    pub fn animate_style(&self, patch: &StylePatch, motion: Motion) {
        for t in Target::from_style(patch) {
            self.animate(t, motion);
        }
    }

    /// Jumps `target`'s property to its value, stopping any move of it.
    pub fn set_animated(&self, target: Target) {
        let mut i = self.ui.0.borrow_mut();
        let Some(d) = i.els.get_mut(&self.id) else { return };
        let key = target.key();
        d.anims.retain(|r| r.key != key);
        let mut slot = slot_for(target);
        slot.retarget(target, Motion::Instant, Duration::ZERO, true);
        let styled = slot.apply(key, d, Duration::ZERO);
        let id = self.id;
        if styled {
            i.tree.mark(id, Dirty::STYLE | Dirty::PAINT);
        } else {
            i.layers_changed = true;
        }
        drop(i);
        self.ui.request(crate::view::Request::Frame);
    }

    /// Glides the element through the layout change about to happen (FLIP: first, last,
    /// invert, play). Call it before changing what moves the element (reordering, inserting
    /// or removing siblings); the next frame draws it where it was and moves it to its new
    /// place with `motion`. An element that does not move, has not been laid out yet, or
    /// is shown under reduced motion simply appears in its new place.
    pub fn flip(&self, motion: Motion) {
        let Some(b) = self.bounds() else { return };
        let reduced = self.ui.reduced_motion();
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.flip = (!reduced).then_some((b.min, motion));
        }
    }

    /// How far from its laid-out place it is drawn now, and at what scale (moved by
    /// [`Target::Translate`], [`Target::Scale`] or [`El::flip`]).
    pub fn drawn_offset(&self) -> (Vec2, f32) {
        self.ui.0.borrow().els.get(&self.id).map_or((Vec2::ZERO, 1.0), |d| d.shift)
    }

    /// True while any of its properties is moving.
    pub fn is_animating(&self) -> bool {
        self.ui.0.borrow().els.get(&self.id).is_some_and(|d| !d.anims.is_empty())
    }
}
