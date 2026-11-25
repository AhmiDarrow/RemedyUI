//! Physical-looking buttons: a round arcade button, a plate key and a toggle switch with a
//! lamp.

use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_core::input::{Key, PointerPhase};
use remedy_ui_core::layout::Align;
use remedy_ui_engine::paint::{Brush, Item, Path};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_core::style::ensure_contrast;
use remedy_ui_engine::view::{El, IntoProp, PaintCx, Prop, RichText, TextPaint, Ui};

/// What a [`big_button`] or [`big_toggle`] looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ButtonLook {
    /// A round arcade button in a bezel.
    #[default]
    Round,
    /// A raised plate, like a key or a desk button.
    Plate,
    /// A toggle switch with a lamp beside it, and its label as text next to it.
    Switch,
}

/// How a [`big_button`] or [`big_toggle`] looks and behaves.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct BigButtonOptions {
    /// Its look.
    pub look: ButtonLook,
    /// Its size: a round button's diameter, a plate's height (it is twice as wide), a
    /// switch's height. `None` takes it from the theme's control height.
    pub size: Option<f32>,
    /// The theme variant naming its color (`accent` by default; `danger`, `ok`, `warn`).
    pub variant: Option<&'static str>,
    /// Whether it is lit (a lamp in the cap, a live indicator). A toggle is lit while on and a
    /// held button while held, unless this says otherwise.
    pub lit: Option<Prop<bool>>,
    /// Set true while the button is held down (by the pointer, or Space or Enter), false when
    /// let go: hold to talk.
    pub held: Option<Signal<bool>>,
    /// Draw the label on a round button's cap or a plate (on by default for those looks).
    /// A switch always shows its label beside it.
    pub hide_label: bool,
}

impl BigButtonOptions {
    /// Sets `look`.
    pub fn look(mut self, value: ButtonLook) -> Self {
        self.look = value;
        self
    }

    /// Sets `size`.
    pub fn size(mut self, value: f32) -> Self {
        self.size = Some(value.max(16.0));
        self
    }

    /// Sets `variant`: the theme variant naming its color.
    pub fn variant(mut self, value: &'static str) -> Self {
        self.variant = Some(value);
        self
    }

    /// Sets `lit` from a constant, signal or closure.
    pub fn lit(mut self, value: impl IntoProp<bool>) -> Self {
        self.lit = Some(value.into_prop());
        self
    }

    /// Sets `held`: true while the button is held down.
    pub fn held(mut self, value: Signal<bool>) -> Self {
        self.held = Some(value);
        self
    }

    /// Sets `hide_label`: keep the label off the cap (it is still the accessible name).
    pub fn hide_label(mut self, value: bool) -> Self {
        self.hide_label = value;
        self
    }
}

#[derive(Clone)]
enum Kind {
    Press(Rc<dyn Fn()>),
    Toggle(Signal<bool>),
}

/// A big physical-looking button that calls `on_press` when pressed and let go over it (or
/// with Enter, Space, or assistive technology's click). It sinks while held, and lights while
/// [`BigButtonOptions::lit`] says so or while [`BigButtonOptions::held`] is down.
///
/// For hold-to-talk, give it a `held` signal: it is true from the moment of the press until
/// the release, from the pointer or the keyboard.
#[track_caller]
pub fn big_button(
    ui: &Ui,
    label: impl IntoProp<String>,
    on_press: impl Fn() + 'static,
    options: BigButtonOptions,
) -> El {
    build(ui, label.into_prop(), Kind::Press(Rc::new(on_press)), options)
}

/// A big physical-looking button that latches: pressing flips `on`, and it stays down and lit
/// while on. A switch look is a switch to assistive technology; the others are toggle buttons
/// (pressed or not).
#[track_caller]
pub fn big_toggle(ui: &Ui, label: impl IntoProp<String>, on: Signal<bool>, options: BigButtonOptions) -> El {
    build(ui, label.into_prop(), Kind::Toggle(on), options)
}

#[track_caller]
fn build(ui: &Ui, label: Prop<String>, kind: Kind, options: BigButtonOptions) -> El {
    let theme = ui.theme();
    let ch = theme.spacing.control_height;
    let look = options.look;
    let size = options.size.unwrap_or(match look {
        ButtonLook::Round => ch * 2.5,
        ButtonLook::Plate => ch * 1.5,
        ButtonLook::Switch => ch * 0.75,
    });
    let down = Signal::new(false);
    let held = options.held;
    let lit = options.lit.clone();
    let show_label = !options.hide_label && look != ButtonLook::Switch;
    let shown_label = label.clone();
    let k2 = kind.clone();

    let el = ui.el("big_button").variant(options.variant.unwrap_or("accent")).focusable(true);
    let el = match look {
        ButtonLook::Round => el.width(size).height(size),
        ButtonLook::Plate => el.width(size * 2.0).height(size),
        ButtonLook::Switch => el.width(size * 3.0).height(size),
    };
    let el = el.paint(move |cx| {
        let pressed = down.get();
        let on = match &k2 {
            Kind::Toggle(s) => s.get(),
            Kind::Press(_) => false,
        };
        let is_lit = match &lit {
            Some(p) => p.get(),
            None => on || held.is_some_and(|h| h.get()),
        };
        let text = if show_label { shown_label.get() } else { String::new() };
        match look {
            ButtonLook::Round => round(cx, pressed || on, is_lit, &text),
            ButtonLook::Plate => plate(cx, pressed || on, is_lit, &text),
            ButtonLook::Switch => switch(cx, on, is_lit),
        }
    });

    let activate = {
        let kind = kind.clone();
        move || match &kind {
            Kind::Press(f) => f(),
            Kind::Toggle(s) => s.update(|v| *v = !*v),
        }
    };
    let set_down = move |v: bool| {
        down.set_if_changed(v);
        if let Some(h) = held {
            h.set_if_changed(v);
        }
    };
    let (act, me) = (activate.clone(), el.clone());
    let act2 = activate.clone();
    let el = el
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down => {
                me.focus();
                set_down(true);
                true
            }
            PointerPhase::Move => untrack(|| down.get()),
            PointerPhase::Up => {
                let was = untrack(|| down.get());
                set_down(false);
                if was && me.bounds().is_some_and(|b| b.contains(e.position)) {
                    act();
                }
                was
            }
            PointerPhase::Cancel => {
                set_down(false);
                true
            }
        })
        .on_key(move |e| {
            if !matches!(e.key, Key::Space | Key::Enter) {
                return false;
            }
            if e.pressed {
                if !e.repeat {
                    set_down(true);
                }
            } else if untrack(|| down.get()) {
                set_down(false);
                act2();
            }
            true
        })
        .on_focus_change(move |focused| {
            // Focus leaving mid-press lets go without pressing.
            if !focused {
                set_down(false);
            }
        })
        .on_accessibility_action(move |a| {
            if a == Action::Click {
                activate();
                true
            } else {
                false
            }
        });

    let me = el.clone();
    let l2 = label.clone();
    let el = el.bind(move || {
        let name = l2.get();
        let s = match &kind {
            Kind::Toggle(on) if look == ButtonLook::Switch => {
                Semantics::new(Role::Switch).label(name).checked(on.get())
            }
            Kind::Toggle(on) => Semantics::new(Role::Button).label(name).checked(on.get()),
            Kind::Press(_) => Semantics::new(Role::Button).label(name),
        };
        let _ = me.clone().semantics(s.action(Action::Click));
    });
    if look != ButtonLook::Switch {
        return el;
    }
    // A switch's label is text beside it.
    let text = ui.el("big_button_label").text(label.map(RichText::plain)).nowrap();
    ui.el("big_switch").row().gap(theme.spacing.unit * 2.0).align(Align::Center).child(el).child(text)
}

/// The cap's color: the variant's color, brighter when lit, dimmer when not.
fn cap_color(cx: &PaintCx<'_>, lit: bool) -> Color {
    let base = cx.style.foreground;
    if lit { base.mix_oklab(Color::WHITE, 0.18) } else { base.mix_oklab(cx.theme.colors.bg, 0.35) }
}

fn glow(cx: &mut PaintCx<'_>, rect: Rect, radii: CornerRadii, color: Color, blur: f32) {
    cx.list.push(Item::Shadow {
        rect,
        radii,
        offset: Vec2::ZERO,
        blur,
        spread: blur * 0.15,
        color: color.with_alpha(0.55),
    });
}

fn label_on(cx: &mut PaintCx<'_>, text: &str, at: Point, fill: Color, size: f32) {
    if text.is_empty() {
        return;
    }
    let color = ensure_contrast(cx.theme.colors.text, fill, 4.5);
    let _ = cx.text(text, at, Vec2::new(0.5, 0.5), TextPaint { size, weight: 700.0, color, mono: false });
}

fn round(cx: &mut PaintCx<'_>, pressed: bool, lit: bool, text: &str) {
    let d = cx.theme.derive();
    let s = cx.size.width.min(cx.size.height);
    let c = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
    let r = s / 2.0;
    let depth = (s * 0.05).max(2.0);
    // Bezel.
    cx.list.push(Item::FillPath {
        path: Path::circle(c, r),
        brush: Brush::Solid(cx.theme.colors.panel),
        even_odd: false,
    });
    cx.list.push(Item::StrokePath {
        path: Path::circle(c, r - 0.5),
        stroke: remedy_ui_engine::paint::Stroke::new(cx.theme.stroke.max(1.0)),
        brush: Brush::Solid(d.line),
    });
    let cap_r = r * 0.78;
    let fill = cap_color(cx, lit);
    let lift = if pressed { depth * 0.25 } else { depth };
    // The cap's side, then its face lifted above it.
    let side = Point::new(c.x, c.y + depth * 0.5);
    cx.list.push(Item::FillPath {
        path: Path::circle(side, cap_r),
        brush: Brush::Solid(fill.mix_oklab(Color::BLACK, 0.35)),
        even_odd: false,
    });
    let face = Point::new(c.x, side.y - lift);
    if lit {
        glow(
            cx,
            Rect::new(face.x - cap_r, face.y - cap_r, face.x + cap_r, face.y + cap_r),
            CornerRadii::all(cap_r),
            cx.style.foreground,
            r * 0.5,
        );
    }
    cx.list.push(Item::FillPath {
        path: Path::circle(face, cap_r),
        brush: Brush::Solid(fill),
        even_odd: false,
    });
    // A highlight on the face's upper half.
    cx.list.push(Item::FillPath {
        path: Path::circle(Point::new(face.x, face.y - cap_r * 0.25), cap_r * 0.6),
        brush: Brush::Solid(Color::WHITE.with_alpha(if lit { 0.16 } else { 0.08 })),
        even_odd: false,
    });
    let size =
        (cx.theme.type_scale.body * s / 80.0).clamp(cx.theme.type_scale.small, cx.theme.type_scale.title);
    label_on(cx, text, face, fill, size);
}

fn plate(cx: &mut PaintCx<'_>, pressed: bool, lit: bool, text: &str) {
    let w = cx.size.width;
    let h = cx.size.height;
    let depth = (h * 0.08).max(2.0);
    let radius = CornerRadii::all(cx.theme.radii.medium.min(h / 3.0));
    let fill = cap_color(cx, lit);
    let lift = if pressed { depth * 0.25 } else { depth };
    let side = Rect::new(0.0, depth, w, h);
    cx.list.fill_rounded(side, radius, Brush::Solid(fill.mix_oklab(Color::BLACK, 0.35)));
    let face = Rect::new(0.0, depth - lift, w, h - lift);
    if lit {
        glow(cx, face, radius, cx.style.foreground, h * 0.4);
    }
    cx.list.fill_rounded(face, radius, Brush::Solid(fill));
    cx.list.border(face, radius, cx.theme.stroke.max(1.0), Color::WHITE.with_alpha(0.12));
    let size = cx.theme.type_scale.body.max(h * 0.28).min(cx.theme.type_scale.title);
    label_on(cx, text, face.center(), fill, size);
}

fn switch(cx: &mut PaintCx<'_>, on: bool, lit: bool) {
    let d = cx.theme.derive();
    let h = cx.size.height;
    // The lamp, then the track and its knob.
    let lamp_r = h * 0.3;
    let lamp = Point::new(h * 0.5, h * 0.5);
    let lamp_color = if lit { cx.style.foreground } else { d.line };
    if lit {
        glow(
            cx,
            Rect::new(lamp.x - lamp_r, lamp.y - lamp_r, lamp.x + lamp_r, lamp.y + lamp_r),
            CornerRadii::all(lamp_r),
            lamp_color,
            h * 0.5,
        );
    }
    cx.list.push(Item::FillPath {
        path: Path::circle(lamp, lamp_r),
        brush: Brush::Solid(lamp_color),
        even_odd: false,
    });
    let track = Rect::new(h * 1.2, 0.0, cx.size.width, h);
    let r = CornerRadii::all(h / 2.0);
    cx.list.fill_rounded(
        track,
        r,
        Brush::Solid(if on {
            cx.style.foreground.mix_oklab(cx.theme.colors.bg, 0.55)
        } else {
            cx.theme.colors.bg
        }),
    );
    cx.list.border(track, r, cx.theme.stroke.max(1.0), if on { cx.style.foreground } else { d.line });
    let kr = h * 0.36;
    let kx = if on { track.max.x - h / 2.0 } else { track.min.x + h / 2.0 };
    let knob = if on { cx.style.foreground } else { d.text };
    cx.list.push(Item::FillPath {
        path: Path::circle(Point::new(kx, h / 2.0), kr),
        brush: Brush::Solid(knob),
        even_odd: false,
    });
}
