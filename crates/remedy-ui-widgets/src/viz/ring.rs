//! The speaking ring: a live voice level drawn around an avatar.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Insets, Rect};
use remedy_ui_core::layout::Align;
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};

use crate::viz::drive::{Clock, Gate, Moved, Smoothing, Ticker};
use crate::viz::unit_level;

/// The ring's strength when someone has just started speaking quietly.
const BASE: f32 = 0.35;

/// Below this strength the ring is not worth drawing.
const FAINT: f32 = 0.003;

/// A radius far larger than any avatar: the renderer fits it to the box, so the ring is round.
const ROUND: f32 = 10_000.0;

/// How the speaking ring looks and behaves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RingOptions {
    /// How fast the ring rises and falls. Ignored under reduced motion.
    pub smoothing: Smoothing,
    /// The level at which someone counts as speaking.
    pub speaking_at: f32,
    /// How long the ring stays after the level drops, so pauses between words do not blink it.
    pub hang: Duration,
    /// The ring's thickest width when speaking loudly. `None` takes it from the theme.
    pub thickness: Option<f32>,
    /// Space between the content and the ring. `None` takes it from the theme.
    pub gap: Option<f32>,
    /// Corner radius of the ring. `None` makes it round (a circle for a square avatar).
    pub radius: Option<f32>,
    /// Draw a faint hairline while silent, so the avatar keeps its frame.
    pub show_when_silent: bool,
    /// Set to whether the person is speaking, for the app to use elsewhere.
    pub speaking: Option<Signal<bool>>,
    /// While true the person is muted: the ring ignores the level and shows the theme's
    /// muted outline, and screen readers hear "muted".
    pub muted: Option<Signal<bool>>,
    /// Where time comes from.
    pub clock: Clock,
}

impl RingOptions {
    /// Sets `smoothing`: how fast the ring rises and falls.
    pub fn smoothing(mut self, value: Smoothing) -> Self {
        self.smoothing = value;
        self
    }

    /// Sets `speaking_at`: the level at which someone counts as speaking.
    pub fn speaking_at(mut self, value: f32) -> Self {
        self.speaking_at = value;
        self
    }

    /// Sets `hang`: how long the ring stays after the level drops, so pauses between words do not blink it.
    pub fn hang(mut self, value: Duration) -> Self {
        self.hang = value;
        self
    }

    /// Sets `thickness`: the ring's thickest width when speaking loudly.
    pub fn thickness(mut self, value: f32) -> Self {
        self.thickness = Some(value);
        self
    }

    /// Sets `gap`: space between the content and the ring.
    pub fn gap(mut self, value: f32) -> Self {
        self.gap = Some(value);
        self
    }

    /// Sets `radius`: corner radius of the ring.
    pub fn radius(mut self, value: f32) -> Self {
        self.radius = Some(value);
        self
    }

    /// Sets `show_when_silent`: draw a faint hairline while silent, so the avatar keeps its frame.
    pub fn show_when_silent(mut self, value: bool) -> Self {
        self.show_when_silent = value;
        self
    }

    /// Sets `speaking`: set to whether the person is speaking, for the app to use elsewhere.
    pub fn speaking(mut self, value: Signal<bool>) -> Self {
        self.speaking = Some(value);
        self
    }

    /// Sets `muted`: while true the person is muted: the ring ignores the level and shows the theme's muted outline, and screen readers hear "muted".
    pub fn muted(mut self, value: Signal<bool>) -> Self {
        self.muted = Some(value);
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

impl Default for RingOptions {
    fn default() -> Self {
        RingOptions {
            smoothing: Smoothing::voice(),
            speaking_at: 0.08,
            hang: Duration::from_millis(250),
            thickness: None,
            gap: None,
            radius: None,
            show_when_silent: false,
            speaking: None,
            muted: None,
            clock: Clock::default(),
        }
    }
}

struct State {
    raw: f32,
    muted: bool,
    gate: Gate,
    strength: f32,
}

/// How strong the ring should be for a level, once someone is speaking: a base glow that
/// grows with the voice.
fn target_strength(raw: f32, speaking_at: f32) -> f32 {
    let span = (1.0 - speaking_at).max(f32::EPSILON);
    let loud = ((raw - speaking_at) / span).clamp(0.0, 1.0);
    BASE + (1.0 - BASE) * loud
}

/// A ring around `content` that shows a live voice level (`level`, 0 to 1).
///
/// The ring appears when the level passes `speaking_at`, swells with the voice, and fades
/// after the person stops. With reduced motion it only switches on and off. `label` names
/// the person for screen readers, which hear "speaking" or "silent".
#[track_caller]
pub fn speaking_ring(
    ui: &Ui,
    label: impl Into<String>,
    level: Signal<f32>,
    content: El,
    options: RingOptions,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let thickness = options.thickness.unwrap_or(unit * 0.75).max(1.0);
    let gap = options.gap.unwrap_or(unit * 0.5).max(0.0);
    let radius = options.radius;
    let show_silent = options.show_when_silent;
    let speaking_at = options.speaking_at.clamp(0.0, 1.0);
    let state = Rc::new(RefCell::new(State {
        raw: 0.0,
        muted: false,
        gate: Gate::new(speaking_at, options.hang),
        strength: 0.0,
    }));

    // What screen readers hear: the message of the state ("speaking", "silent", "muted"), read
    // in the window's language.
    let heard: Signal<&'static str> = Signal::new("ring-silent");
    let u = ui.clone();
    let paint_state = state.clone();
    let el = ui
        .el("speaking_ring")
        .row()
        .align(Align::Center)
        .justify(remedy_ui_core::layout::Justify::Center)
        .padding(Insets::all(thickness + gap))
        .layout(|l| l.align_self = Some(Align::Start))
        .semantics(move || {
            Semantics::new(Role::Status).label(label.clone()).value(crate::viz::STRINGS.get(&u, heard.get()))
        })
        .paint(move |cx| {
            let (strength, muted) = paint_state.try_borrow().map_or((0.0, false), |s| (s.strength, s.muted));
            let whole = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
            let radii = CornerRadii::all(radius.unwrap_or(ROUND));
            if muted {
                // The "muted" variant's color, a steady outline: muted is a state, not a level.
                cx.list.border(whole, radii, (thickness * 0.5).max(1.0), cx.style.foreground);
            } else if strength > FAINT {
                let width = 1.0 + (thickness - 1.0) * strength;
                let color = cx.style.foreground.with_alpha(0.3 + 0.7 * strength);
                cx.list.border(whole, radii, width, color);
            } else if show_silent {
                cx.list.border(whole, radii, 1.0, cx.theme.derive().line);
            }
        })
        .child(content);

    let step_state = state.clone();
    let step_el = el.clone();
    let speaking_out = options.speaking;
    let speaking_out_mute = options.speaking;
    let smoothing = options.smoothing;
    let ticker = Ticker::new(&el, options.clock, move |dt| {
        let reduced = step_el.ui().reduced_motion();
        let Ok(mut s) = step_state.try_borrow_mut() else { return Moved::default() };
        let was_open = s.gate.is_open();
        let raw = s.raw;
        let open = s.gate.feed(raw, dt);
        let target = if open { target_strength(raw, speaking_at) } else { 0.0 };
        let next =
            if reduced { if open { 1.0 } else { 0.0 } } else { smoothing.step(s.strength, target, dt) };
        let changed = (next - s.strength).abs() > f32::EPSILON;
        s.strength = next;
        let pending = s.gate.is_hanging() || (!reduced && (next - target).abs() > f32::EPSILON);
        let flipped = open != was_open;
        drop(s);
        if flipped {
            heard.set_if_changed(if open { "ring-speaking" } else { "ring-silent" });
            if let Some(out) = speaking_out {
                out.set_if_changed(open);
            }
        }
        Moved { changed: changed || flipped, pending }
    });

    let muted = options.muted;
    let hang = options.hang;
    let mute_el = el.clone();
    el.bind(move || {
        let is_muted = muted.is_some_and(|m| m.get());
        // A muted person's level is not shown, whatever the microphone hears.
        let v = if is_muted { 0.0 } else { unit_level(level.get()) };
        let mut flipped = false;
        if let Ok(mut s) = state.try_borrow_mut() {
            s.raw = v;
            if s.muted != is_muted {
                s.muted = is_muted;
                flipped = true;
                if is_muted {
                    s.strength = 0.0;
                    s.gate = Gate::new(speaking_at, hang);
                }
            }
        }
        if flipped {
            heard.set_if_changed(if is_muted { "ring-muted" } else { "ring-silent" });
            let _ = mute_el.clone().variant(if is_muted { "muted" } else { "" });
            if is_muted && let Some(out) = speaking_out_mute {
                out.set_if_changed(false);
            }
        }
        ticker.kick();
    })
}
