//! A step-by-step flow, such as a setup wizard.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Live, Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect, Vec2};
use remedy_ui_input::Key;
use remedy_ui_layout::{Align, Length};
use remedy_ui_paint::{Brush, Item, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_reactive::Signal;
use remedy_ui_style::States;
use remedy_ui_view::{El, RichText, TextPaint, Ui};
use remedy_ui_widgets_core as w;

/// One step of a [`stepper_flow`].
#[derive(Clone)]
pub struct Step {
    title: String,
    description: Option<String>,
    content: El,
    valid: Option<Signal<bool>>,
    optional: bool,
}

impl Step {
    /// A step called `title` showing `content`. Until told otherwise it can always be left.
    pub fn new(title: impl Into<String>, content: El) -> Step {
        Step { title: title.into(), description: None, content, valid: None, optional: false }
    }

    /// A line of explanation under the title.
    pub fn description(mut self, text: impl Into<String>) -> Step {
        self.description = Some(text.into());
        self
    }

    /// Whether the step is complete: while this is false, Next stays disabled and the steps
    /// after this one cannot be jumped to.
    pub fn valid(mut self, valid: Signal<bool>) -> Step {
        self.valid = Some(valid);
        self
    }

    /// Lets people skip the step, whatever its validity.
    pub fn optional(mut self, optional: bool) -> Step {
        self.optional = optional;
        self
    }
}

/// The words on the flow's buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StepperLabels {
    /// Back to the step before.
    pub back: String,
    /// On to the next step.
    pub next: String,
    /// Skip an optional step.
    pub skip: String,
    /// Finish on the last step.
    pub finish: String,
}

impl Default for StepperLabels {
    fn default() -> Self {
        StepperLabels {
            back: "Back".into(),
            next: "Next".into(),
            skip: "Skip".into(),
            finish: "Finish".into(),
        }
    }
}

/// Whether leaving step `i` is allowed (its validity, if it has one).
fn passes(steps: &[Step], i: usize) -> bool {
    steps.get(i).is_none_or(|s| s.valid.is_none_or(|v| v.get_untracked().unwrap_or(false)))
}

/// Whether step `to` can be reached from step `from`: any step before, or ahead through steps
/// that are all valid.
fn reachable(steps: &[Step], from: usize, to: usize) -> bool {
    to <= from || (from..to).all(|i| passes(steps, i))
}

/// A guided flow through `steps`, shown one at a time under a progress indicator of numbered
/// markers, with Back and Next (Finish on the last step) below. `current` is the step shown.
///
/// A step with a `valid` signal gates Next while it is false, and the markers of later steps
/// cannot be pressed past it; steps before the current one can always be returned to. An
/// optional step also has Skip. `on_finish` runs when Finish is pressed. When the step
/// changes by Back, Next or a marker, focus moves into the new step. Alt with the Left and Right
/// arrow keys go back and on. To assistive technology the flow reads "Step 2 of 4: Title" and
/// announces changes politely; markers are named with their state.
#[track_caller]
pub fn stepper_flow(
    ui: &Ui,
    label: impl Into<String>,
    steps: Vec<Step>,
    current: Signal<usize>,
    labels: StepperLabels,
    on_finish: impl Fn() + 'static,
) -> El {
    let label = label.into();
    let n = steps.len();
    let unit = ui.theme().spacing.unit;
    let last = n.saturating_sub(1);
    let steps = Rc::new(steps);
    let at = move || current.get().min(last);
    let at_untracked = move || current.get_untracked().unwrap_or(0).min(last);

    // Moves to a step; the binding below puts focus inside it once it is shown.
    let content: Vec<El> = steps.iter().map(|s| s.content.clone()).collect();
    let refocus = Rc::new(Cell::new(false));
    let goto = {
        let refocus = refocus.clone();
        move |to: usize| {
            let to = to.min(last);
            if to != at_untracked() {
                refocus.set(true);
                current.set(to);
            }
        }
    };

    // Progress: how far through the flow, as a bar.
    let fraction: Signal<Option<f32>> = Signal::new(Some(0.0));
    let bar = w::progress(ui, format!("{label} progress"), fraction);

    // The markers.
    let markers: Vec<El> = (0..n)
        .map(|i| {
            let steps2 = steps.clone();
            let title = steps.get(i).map(|s| s.title.clone()).unwrap_or_default();
            let t2 = title.clone();
            let go = goto.clone();
            let dot = ui.el("step_marker").width(unit * 7.0).height(unit * 7.0).paint(move |cx| {
                let cur = current.get().min(last);
                let c = &cx.theme.colors;
                let d = cx.theme.derive();
                let r = cx.size.width.min(cx.size.height) / 2.0;
                let mid = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
                let disc = Rect::new(mid.x - r, mid.y - r, mid.x + r, mid.y + r);
                let (fill, ink) = if i < cur {
                    (c.accent, d.on_accent)
                } else if i == cur {
                    (c.card, d.text)
                } else {
                    (c.card, d.muted)
                };
                cx.list.fill_rounded(disc, CornerRadii::all(r), Brush::Solid(fill));
                let edge = if i <= cur { c.accent } else { d.line };
                cx.list.border(disc, CornerRadii::all(r), 2.0, edge);
                if i < cur {
                    let mut p = Path::new();
                    p.move_to(Point::new(mid.x - r * 0.4, mid.y));
                    p.line_to(Point::new(mid.x - r * 0.1, mid.y + r * 0.3));
                    p.line_to(Point::new(mid.x + r * 0.45, mid.y - r * 0.3));
                    let mut stroke = Stroke::new(2.0);
                    stroke.cap = StrokeCap::Round;
                    stroke.join = StrokeJoin::Round;
                    cx.list.push(Item::StrokePath { path: p, stroke, brush: Brush::Solid(ink) });
                } else {
                    cx.text(
                        &(i + 1).to_string(),
                        mid,
                        Vec2::new(0.5, 0.5),
                        TextPaint { size: cx.theme.type_scale.small, weight: 600.0, color: ink, mono: true },
                    );
                }
            });
            let caption = w::text(ui, title).nowrap();
            ui.el("step_marker_cell")
                .row()
                .gap(unit * 2.0)
                .align(Align::Center)
                .layout(|l| l.min_height = Length::Px(w::MIN_TARGET))
                .child(dot)
                .child(caption)
                .on_click(move || {
                    if reachable(&steps2, at_untracked(), i) {
                        go(i);
                    }
                })
                .semantics({
                    let steps3 = steps.clone();
                    move || {
                        let cur = at();
                        let state = if i < cur {
                            "completed"
                        } else if i == cur {
                            "current"
                        } else if reachable(&steps3, cur, i) {
                            "not started"
                        } else {
                            "locked"
                        };
                        Semantics::new(Role::Button)
                            .label(format!("Step {}: {t2}, {state}", i + 1))
                            .selected(i == cur)
                            .action(Action::Click)
                    }
                })
        })
        .collect();
    let mut header = w::row(ui).gap(unit * 4.0).align(Align::Center);
    for m in &markers {
        header = header.child(m.clone());
    }

    // The step bodies: only the current one is shown.
    let status = w::text(ui, "").variant("muted").data_text();
    let mut body = w::column(ui).gap(unit * 3.0);
    let mut bodies = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let mut part = w::column(ui).gap(unit * 2.0).child(w::heading(ui, 2, s.title.clone()));
        if let Some(d) = &s.description {
            part = part.child(w::text(ui, d.clone()).variant("muted"));
        }
        let part = part.child(s.content.clone()).hidden(i != 0);
        bodies.push(part.clone());
        body = body.child(part);
    }

    // Back, Skip, Next and Finish.
    let back = {
        let go = goto.clone();
        w::button(ui, labels.back.clone(), move || go(at_untracked().saturating_sub(1)))
    };
    let next = {
        let (go, steps) = (goto.clone(), steps.clone());
        w::button(ui, labels.next.clone(), move || {
            if passes(&steps, at_untracked()) {
                go(at_untracked() + 1);
            }
        })
        .variant("primary")
    };
    let skip = {
        let go = goto.clone();
        w::button(ui, labels.skip.clone(), move || go(at_untracked() + 1)).variant("quiet")
    };
    let finish = {
        let steps = steps.clone();
        w::button(ui, labels.finish.clone(), move || {
            if passes(&steps, at_untracked()) {
                on_finish();
            }
        })
        .variant("primary")
    };
    let footer = w::row(ui)
        .gap(unit * 2.0)
        .align(Align::Center)
        .child(back.clone())
        .child(w::spacer(ui))
        .child(skip.clone())
        .child(next.clone())
        .child(finish.clone());

    let root = w::column(ui)
        .gap(unit * 4.0)
        .child(bar)
        .child(header)
        .child(status.clone())
        .child(body)
        .child(w::divider(ui, false))
        .child(footer);
    let (b_root, b_steps) = (root.clone(), steps.clone());
    let root = root.bind(move || {
        let cur = at();
        for (i, b) in bodies.iter().enumerate() {
            let _ = b.clone().hidden(i != cur);
        }
        let is_last = cur >= last;
        let ok = b_steps.get(cur).is_none_or(|s| s.valid.is_none_or(|v| v.get()));
        let optional = b_steps.get(cur).is_some_and(|s| s.optional);
        let _ = back.clone().disabled(cur == 0);
        let _ = next.clone().hidden(is_last).disabled(!ok);
        let _ = skip.clone().hidden(!optional || is_last);
        let _ = finish.clone().hidden(!is_last).disabled(!ok);
        if refocus.replace(false) {
            // Into the new step, or failing that to the button that carries on, so the
            // keyboard is not left on a button that just went away.
            let inside = content.get(cur).is_some_and(El::focus_first);
            if !inside {
                let enabled =
                    |b: &El| b.info().is_some_and(|i| !i.hidden && !i.states.contains(States::DISABLED));
                for b in [&next, &finish, &skip, &back] {
                    if enabled(b) {
                        b.focus();
                        break;
                    }
                }
            }
        }
        fraction.set_if_changed(Some(if n == 0 { 0.0 } else { cur as f32 / n as f32 }));
        let title = b_steps.get(cur).map(|s| s.title.clone()).unwrap_or_default();
        let said = format!("Step {} of {n}: {title}", cur + 1);
        let _ = status.clone().text(RichText::plain(format!("Step {} of {n}", cur + 1)));
        let _ = b_root
            .clone()
            .semantics(Semantics::new(Role::Group).label(label.clone()).value(said).live(Live::Polite));
    });

    let key_steps = steps.clone();
    let key_el = root.clone();
    root.on_key(move |e| {
        if !e.pressed || !e.modifiers.alt {
            return false;
        }
        let (prev, onward) = if key_el.is_rtl() {
            (Key::ArrowRight, Key::ArrowLeft)
        } else {
            (Key::ArrowLeft, Key::ArrowRight)
        };
        if e.key == prev {
            goto(at_untracked().saturating_sub(1));
            true
        } else if e.key == onward {
            if passes(&key_steps, at_untracked()) {
                goto(at_untracked() + 1);
            }
            true
        } else {
            false
        }
    })
}
