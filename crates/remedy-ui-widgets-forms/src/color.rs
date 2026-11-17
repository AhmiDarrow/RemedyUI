//! A color picker with a contrast readout.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect};
use remedy_ui_input::{Buttons, Key, KeyEvent, PointerEvent, PointerPhase};
use remedy_ui_paint::{Brush, Stop};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::StylePatch;
use remedy_ui_view::{Align, El, FieldKind, Ui};
use remedy_ui_widgets_core as w;

/// Hue (0..360), saturation, value and alpha (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    /// Hue in degrees.
    pub h: f32,
    /// Saturation.
    pub s: f32,
    /// Value (brightness).
    pub v: f32,
    /// Opacity.
    pub a: f32,
}

impl Hsv {
    /// From an sRGB color.
    pub fn from_color(c: Color) -> Hsv {
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        let d = max - min;
        let h = if d <= f32::EPSILON {
            0.0
        } else if (max - c.r).abs() <= f32::EPSILON {
            60.0 * ((c.g - c.b) / d).rem_euclid(6.0)
        } else if (max - c.g).abs() <= f32::EPSILON {
            60.0 * ((c.b - c.r) / d + 2.0)
        } else {
            60.0 * ((c.r - c.g) / d + 4.0)
        };
        Hsv { h, s: if max <= 0.0 { 0.0 } else { d / max }, v: max, a: c.a }
    }

    /// To an sRGB color.
    pub fn to_color(self) -> Color {
        let c = self.v * self.s;
        let hp = self.h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        Color::new(r + m, g + m, b + m, self.a)
    }
}

/// What WCAG says about a contrast ratio.
pub fn contrast_grade(ratio: f32) -> &'static str {
    if ratio >= 7.0 {
        "AAA"
    } else if ratio >= 4.5 {
        "AA"
    } else if ratio >= 3.0 {
        "AA large text"
    } else {
        "Fails"
    }
}

fn hex(c: Color) -> String {
    let h = c.to_hex();
    if c.a >= 0.999 { h.chars().take(7).collect() } else { h }
}

/// A 2D area for saturation (across) and brightness (up), a hue strip, an opacity strip, a
/// hex field and a preview. With `against`, it shows the contrast ratio of the color on that
/// background and its WCAG grade. Every part works from the keyboard: arrows move by 1%
/// (Shift: 10%), and hue by 1° (Shift: 10°).
#[track_caller]
pub fn color_picker(ui: &Ui, label: impl Into<String>, value: Signal<Color>, against: Option<Color>) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    // Kept apart from the color so hue survives grays and black.
    let hsv = Signal::new(Hsv::from_color(value.get_untracked().unwrap_or(Color::BLACK)));
    let set = move |n: Hsv| {
        hsv.set(n);
        value.set(n.to_color());
    };

    // Saturation × brightness.
    let area = ui.el("color_area").width(220.0).height(140.0).focusable(true);
    let a2 = area.clone();
    let area = area
        .paint(move |cx| {
            let h = hsv.get();
            let r = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
            let pure = Hsv { h: h.h, s: 1.0, v: 1.0, a: 1.0 }.to_color();
            let radii = CornerRadii::all(6.0);
            cx.list.fill_rounded(
                r,
                radii,
                Brush::Linear {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(cx.size.width, 0.0),
                    stops: vec![Stop { offset: 0.0, color: Color::WHITE }, Stop { offset: 1.0, color: pure }],
                },
            );
            cx.list.fill_rounded(
                r,
                radii,
                Brush::Linear {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(0.0, cx.size.height),
                    stops: vec![
                        Stop { offset: 0.0, color: Color::TRANSPARENT },
                        Stop { offset: 1.0, color: Color::BLACK },
                    ],
                },
            );
            let p = Point::new(h.s * cx.size.width, (1.0 - h.v) * cx.size.height);
            let ring = if h.v > 0.5 && h.s < 0.5 { Color::BLACK } else { Color::WHITE };
            cx.list.fill_rounded(
                Rect::new(p.x - 7.0, p.y - 7.0, p.x + 7.0, p.y + 7.0),
                CornerRadii::all(7.0),
                Brush::Solid(ring),
            );
            cx.list.fill_rounded(
                Rect::new(p.x - 5.0, p.y - 5.0, p.x + 5.0, p.y + 5.0),
                CornerRadii::all(5.0),
                Brush::Solid(h.to_color().with_alpha(1.0)),
            );
        })
        .on_pointer(move |e: &PointerEvent| {
            let drag = e.phase == PointerPhase::Down
                || (e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY));
            if drag && let Some(b) = a2.bounds() {
                let s = ((e.position.x - b.min.x) / b.width().max(1.0)).clamp(0.0, 1.0);
                let v = 1.0 - ((e.position.y - b.min.y) / b.height().max(1.0)).clamp(0.0, 1.0);
                let cur = hsv.get_untracked().unwrap_or(Hsv { h: 0.0, s, v, a: 1.0 });
                set(Hsv { s, v, ..cur });
            }
            drag
        })
        .on_key(move |e: &KeyEvent| {
            if !e.pressed {
                return false;
            }
            let d = if e.modifiers.shift { 0.1 } else { 0.01 };
            let Ok(cur) = hsv.get_untracked() else { return false };
            let next = match e.key {
                Key::ArrowLeft => Hsv { s: (cur.s - d).max(0.0), ..cur },
                Key::ArrowRight => Hsv { s: (cur.s + d).min(1.0), ..cur },
                Key::ArrowUp => Hsv { v: (cur.v + d).min(1.0), ..cur },
                Key::ArrowDown => Hsv { v: (cur.v - d).max(0.0), ..cur },
                _ => return false,
            };
            set(next);
            true
        })
        .semantics_dyn(move || {
            let h = hsv.get();
            Semantics::new(Role::Slider)
                .label("Saturation and brightness")
                .value(format!("saturation {:.0}%, brightness {:.0}%", h.s * 100.0, h.v * 100.0))
                .action(Action::Focus)
        });

    // A horizontal strip for hue or opacity.
    let strip = |name: &'static str,
                 get: fn(Hsv) -> f32,
                 put: fn(Hsv, f32) -> Hsv,
                 max: f32,
                 step: f32,
                 unit_s: &'static str| {
        let el = ui.el("color_strip").height(18.0).fill_width().focusable(true);
        let e2 = el.clone();
        el.paint(move |cx| {
            let h = hsv.get();
            let r = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
            let stops: Vec<Stop> = if max > 1.0 {
                (0..=6)
                    .map(|i| Stop {
                        offset: i as f32 / 6.0,
                        color: Hsv { h: i as f32 * 60.0, s: 1.0, v: 1.0, a: 1.0 }.to_color(),
                    })
                    .collect()
            } else {
                let c = h.to_color();
                vec![
                    Stop { offset: 0.0, color: c.with_alpha(0.0) },
                    Stop { offset: 1.0, color: c.with_alpha(1.0) },
                ]
            };
            if max <= 1.0 {
                // A checkerboard shows through transparency.
                let sq = cx.size.height / 2.0;
                let mut x = 0.0;
                let mut i = 0u32;
                while x < cx.size.width {
                    for row in 0..2u32 {
                        if (i + row).is_multiple_of(2) {
                            cx.list.fill_rect(
                                Rect::new(
                                    x,
                                    row as f32 * sq,
                                    (x + sq).min(cx.size.width),
                                    (row as f32 + 1.0) * sq,
                                ),
                                Brush::Solid(Color::new(0.8, 0.8, 0.8, 1.0)),
                            );
                        }
                    }
                    x += sq;
                    i += 1;
                }
            }
            cx.list.fill_rounded(
                r,
                CornerRadii::all(4.0),
                Brush::Linear { start: Point::new(0.0, 0.0), end: Point::new(cx.size.width, 0.0), stops },
            );
            let x = get(h) / max * cx.size.width;
            cx.list.fill_rounded(
                Rect::new(x - 3.0, -2.0, x + 3.0, cx.size.height + 2.0),
                CornerRadii::all(3.0),
                Brush::Solid(Color::WHITE),
            );
        })
        .on_pointer(move |e: &PointerEvent| {
            let drag = e.phase == PointerPhase::Down
                || (e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY));
            if drag && let Some(b) = e2.bounds() {
                let t = ((e.position.x - b.min.x) / b.width().max(1.0)).clamp(0.0, 1.0);
                if let Ok(cur) = hsv.get_untracked() {
                    set(put(cur, t * max));
                }
            }
            drag
        })
        .on_key(move |e: &KeyEvent| {
            if !e.pressed {
                return false;
            }
            let d = if e.modifiers.shift { step * 10.0 } else { step };
            let Ok(cur) = hsv.get_untracked() else { return false };
            let now = get(cur);
            let next = match e.key {
                Key::ArrowRight | Key::ArrowUp => (now + d).min(max),
                Key::ArrowLeft | Key::ArrowDown => (now - d).max(0.0),
                Key::Home => 0.0,
                Key::End => max,
                _ => return false,
            };
            set(put(cur, next));
            true
        })
        .semantics_dyn(move || {
            let v = get(hsv.get());
            let shown = if max > 1.0 { v } else { v * 100.0 };
            Semantics::new(Role::Slider)
                .label(name)
                .numeric(f64::from(shown), 0.0, f64::from(if max > 1.0 { max } else { 100.0 }))
                .value(format!("{shown:.0}{unit_s}"))
                .action(Action::Increment)
                .action(Action::Decrement)
        })
    };
    let hue = strip("Hue", |h| h.h, |h, v| Hsv { h: v.min(359.99), ..h }, 360.0, 1.0, "°");
    let alpha = strip("Opacity", |h| h.a, |h, v| Hsv { a: v, ..h }, 1.0, 0.01, "%");

    // Hex field.
    let field = ui.text_field("text_input", FieldKind::Single).label(format!("{label} hex"));
    let bad = Signal::new(false);
    let f2 = field.clone();
    let field = field.on_change(move || match Color::from_hex(f2.value().trim()) {
        Ok(c) => {
            bad.set_if_changed(false);
            hsv.set(Hsv::from_color(c));
            value.set(c);
        }
        Err(_) => bad.set_if_changed(true),
    });
    let fe = field.el().width(120.0);
    let fe2 = fe.clone();
    let fe = fe.bind(move || {
        let c = value.get();
        let wrong = bad.get();
        if !fe2.is_focused() {
            untrack(|| field.set_value(&hex(c)));
        }
        fe2.state(remedy_ui_style::States::INVALID, wrong);
    });

    // Keep the HSV in step with outside changes to the color.
    let sync = w::column(ui);
    let sync = sync.bind(move || {
        let c = value.get();
        let cur = untrack(|| hsv.get_untracked().ok());
        if cur.map(Hsv::to_color) != Some(c) {
            let mut n = Hsv::from_color(c);
            if let Some(old) = cur
                && (n.s == 0.0 || n.v == 0.0)
            {
                n.h = old.h;
            }
            hsv.set(n);
        }
    });

    let preview = ui.el("swatch").width(40.0).height(40.0).style_dyn(move || StylePatch {
        background: Some(value.get()),
        radius: Some(6.0),
        ..StylePatch::default()
    });
    let mut info = w::column(ui).gap(unit).child(fe);
    if let Some(bg) = against {
        let readout = w::label(ui, "", "").text_dyn(move || {
            let c = value.get();
            let ratio = c.with_alpha(1.0).contrast(bg);
            format!("Contrast {ratio:.1}:1 · {}", contrast_grade(ratio))
        });
        info = info.child(readout.semantics_dyn(move || {
            let ratio = value.get().with_alpha(1.0).contrast(bg);
            Semantics::new(Role::Status).label(format!("Contrast {ratio:.1} to 1, {}", contrast_grade(ratio)))
        }));
    }
    let l2 = label.clone();
    w::column(ui)
        .gap(unit * 2.0)
        .child(area)
        .child(hue)
        .child(alpha)
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(preview).child(info))
        .child(sync)
        .semantics_dyn(move || Semantics::new(Role::ColorWell).label(l2.clone()).value(hex(value.get())))
}
