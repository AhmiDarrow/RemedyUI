//! Bars: a small group of bars, such as a spectrum.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Rect};
use remedy_ui_paint::Brush;
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Ui};

use crate::drive::{Clock, Moved, Smoothing, Ticker};
use crate::series::{Range, Samples, extent, fraction, group_max};

/// How the bar group looks and behaves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BarsOptions {
    /// The span of values shown. `None` is 0 to 1. Auto fits the values, from zero up.
    pub range: Option<Range>,
    /// How the bars rise and fall. Ignored under reduced motion.
    pub smoothing: Smoothing,
    /// In place of `smoothing`, move every bar in a straight line to its new value in
    /// exactly this long (values that arrive on a beat, an equaliser's presets). Ignored
    /// under reduced motion.
    pub tween: Option<Duration>,
    /// The least height a bar is drawn with, in logical pixels: above 0, a silent bar still
    /// shows as a stub this tall. 0 hides bars with nothing to show.
    pub min_bar: f32,
    /// Space between bars. `None` takes it from the theme. With more values than pixels the
    /// bars share pixels and the highest value of each group shows.
    pub gap: Option<f32>,
    /// The width. `None` fills the available width.
    pub width: Option<f32>,
    /// The height. `None` takes it from the theme.
    pub height: Option<f32>,
    /// Where time comes from.
    pub clock: Clock,
}

impl BarsOptions {
    /// Sets `tween`: every bar reaches its new value in a straight line in this long.
    pub fn tween(mut self, value: Duration) -> Self {
        self.tween = Some(value);
        self
    }

    /// Sets `min_bar`: the least height a bar is drawn with.
    pub fn min_bar(mut self, value: f32) -> Self {
        self.min_bar = value;
        self
    }

    /// Sets `range`: the span of values shown.
    pub fn range(mut self, value: Range) -> Self {
        self.range = Some(value);
        self
    }

    /// Sets `smoothing`: how the bars rise and fall.
    pub fn smoothing(mut self, value: Smoothing) -> Self {
        self.smoothing = value;
        self
    }

    /// Sets `gap`: space between bars.
    pub fn gap(mut self, value: f32) -> Self {
        self.gap = Some(value);
        self
    }

    /// Sets `width`: the width.
    pub fn width(mut self, value: f32) -> Self {
        self.width = Some(value);
        self
    }

    /// Sets `height`: the height.
    pub fn height(mut self, value: f32) -> Self {
        self.height = Some(value);
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

impl Default for BarsOptions {
    fn default() -> Self {
        BarsOptions {
            range: None,
            smoothing: Smoothing::meter(),
            tween: None,
            min_bar: 0.0,
            gap: None,
            width: None,
            height: None,
            clock: Clock::default(),
        }
    }
}

struct State {
    shown: Vec<f32>,
    target: Vec<f32>,
    /// For a tween: where each bar was when the target last changed, and how long ago.
    from: Vec<f32>,
    since: Duration,
}

/// A bar's place `since` into a straight move from `from` to `target` that takes `tween`.
fn tweened(from: f32, target: f32, since: Duration, tween: Duration) -> f32 {
    if tween.is_zero() || since >= tween {
        return target;
    }
    from + (target - from) * (since.as_secs_f32() / tween.as_secs_f32())
}

/// The values as fractions of their range: the range is 0 to 1 unless the options say
/// otherwise, and Auto fits the highest value (and the lowest if it is below zero).
fn fractions<S: Samples + ?Sized>(s: &S, range: Option<Range>) -> Vec<f32> {
    let (lo, hi) = match range {
        None => (0.0, 1.0),
        Some(Range::Fixed(a, b)) => Range::Fixed(a, b).resolve(a, b),
        Some(Range::Auto) => match extent(s) {
            Some((lowest, highest)) if highest > lowest.min(0.0) => (lowest.min(0.0), highest),
            _ => (0.0, 1.0),
        },
    };
    (0..s.len()).map(|i| s.get(i).filter(|v| v.is_finite()).map_or(0.0, |v| fraction(v, lo, hi))).collect()
}

/// A group of bars for `values`, one bar each, with fast rise and gentle fall.
///
/// It repaints only while bars move. Screen readers hear `label` and the highest bar.
#[track_caller]
pub fn bars<S: Samples + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    values: Signal<S>,
    options: BarsOptions,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let gap = options.gap.unwrap_or(unit * 0.5).max(0.0);
    let el = ui.el("bars").height(options.height.unwrap_or(unit * 10.0));
    let el = match options.width {
        Some(w) => el.width(w),
        None => el.fill_width(),
    };
    let state = Rc::new(RefCell::new(State {
        shown: Vec::new(),
        target: Vec::new(),
        from: Vec::new(),
        since: Duration::ZERO,
    }));
    let paint_state = state.clone();
    let min_bar = if options.min_bar.is_finite() { options.min_bar.max(0.0) } else { 0.0 };
    let el = el.semantics(Semantics::new(Role::Image).label(label.clone())).paint(move |cx| {
        let Ok(s) = paint_state.try_borrow() else { return };
        // Bars at least a pixel wide: more values than that share bars.
        let columns = (((cx.size.width + gap) / (1.0 + gap)).floor() as usize).max(1);
        let shown = group_max(&s.shown, columns);
        let k = shown.len();
        if k == 0 {
            return;
        }
        let gap = if k > 1 { gap } else { 0.0 };
        let bar = ((cx.size.width - gap * (k - 1) as f32) / k as f32).max(0.0);
        let round = (bar / 2.0).min(2.0);
        let radii = CornerRadii { top_start: round, top_end: round, bottom_end: 0.0, bottom_start: 0.0 };
        for (i, v) in shown.iter().enumerate() {
            let h = v * cx.size.height;
            if *v > 0.004 || min_bar > 0.0 {
                let x = i as f32 * (bar + gap);
                let top = cx.size.height - h.max(min_bar).max(1.0).min(cx.size.height);
                cx.list.fill_rounded(
                    Rect::new(x, top, x + bar, cx.size.height),
                    radii,
                    Brush::Solid(cx.style.foreground),
                );
            }
        }
    });

    let step_state = state.clone();
    let step_el = el.clone();
    let smoothing = options.smoothing;
    let tween = options.tween;
    let ticker = Ticker::new(&el, options.clock, move |dt| {
        let reduced = step_el.ui().reduced_motion();
        let Ok(mut guard) = step_state.try_borrow_mut() else { return Moved::default() };
        let s = &mut *guard;
        let mut changed = false;
        let mut pending = false;
        s.since = s.since.saturating_add(dt);
        let since = s.since;
        for ((shown, target), from) in s.shown.iter_mut().zip(&s.target).zip(&s.from) {
            let next = match tween {
                _ if reduced => *target,
                Some(t) => tweened(*from, *target, since, t),
                None => smoothing.step(*shown, *target, dt),
            };
            changed |= (next - *shown).abs() > f32::EPSILON;
            pending |= (next - *target).abs() > f32::EPSILON;
            *shown = next;
        }
        Moved { changed, pending }
    });

    // Screen readers hear the highest bar in twentieths of the range.
    let heard = Rc::new(RefCell::new((usize::MAX, -1i32, String::new())));
    let sem_el = el.clone();
    let u = ui.clone();
    let range = options.range;
    el.bind(move || {
        let target = values.with(|s| fractions(s, range));
        let count = target.len();
        let high = target.iter().copied().fold(0.0f32, f32::max);
        if let Ok(mut s) = state.try_borrow_mut() {
            // Bars that did not exist before start at their value instead of rising from zero.
            let have = s.shown.len();
            s.shown.truncate(count);
            s.shown.extend(target.iter().skip(have).copied());
            // A tween starts over from where the bars are now.
            if s.target != target {
                s.from = s.shown.clone();
                s.since = Duration::ZERO;
            }
            s.target = target;
        }
        let tick = (high * 20.0).round() as i32;
        // Said again when the window's language changes too.
        let now = (count, tick, u.locale());
        if heard.replace(now.clone()) != now {
            let said = crate::STRINGS.format(
                &u,
                "bars-value",
                &[("count", count.into()), ("percent", (tick * 5).into())],
            );
            let _ = sem_el.clone().semantics(Semantics::new(Role::Image).label(label.clone()).value(said));
        }
        ticker.kick();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tween_moves_in_a_straight_line_and_lands_on_time() {
        let t = Duration::from_millis(200);
        assert_eq!(tweened(0.2, 1.0, Duration::ZERO, t), 0.2);
        assert!((tweened(0.2, 1.0, Duration::from_millis(50), t) - 0.4).abs() < 1e-6);
        assert_eq!(tweened(0.2, 1.0, t, t), 1.0);
        assert_eq!(tweened(0.2, 1.0, Duration::from_secs(9), t), 1.0);
        assert_eq!(tweened(0.2, 1.0, Duration::ZERO, Duration::ZERO), 1.0, "no time: at once");
    }
}
