//! A number field with exact decimals, units and the locale's way of writing numbers.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::States;
use remedy_ui_view::{Align, El, FieldKind, Ui};
use remedy_ui_widgets_core as w;

use remedy_ui_services_i18n::{Decimal, DecimalError, NumberSymbols};

/// The limits and look of a [`number_input`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NumberOptions {
    /// The smallest allowed value.
    pub min: Option<Decimal>,
    /// The largest allowed value.
    pub max: Option<Decimal>,
    /// What Up and Down add and take away (Shift: ten times as much).
    pub step: Decimal,
    /// Fraction digits kept and shown (values are rounded to it).
    pub precision: u8,
    /// Fraction digits always shown; trailing zeros beyond them are dropped (`None`: all
    /// `precision` digits are shown).
    pub min_fraction_digits: Option<u8>,
    /// A unit written after the number ("g", "%", "ms").
    pub unit: Option<String>,
}

impl NumberOptions {
    /// Sets `min`: the smallest allowed value.
    pub fn min(mut self, value: Decimal) -> Self {
        self.min = Some(value);
        self
    }

    /// Sets `max`: the largest allowed value.
    pub fn max(mut self, value: Decimal) -> Self {
        self.max = Some(value);
        self
    }

    /// Sets `step`: what Up and Down add and take away (Shift: ten times as much).
    pub fn step(mut self, value: Decimal) -> Self {
        self.step = value;
        self
    }

    /// Sets `precision`: fraction digits kept and shown (values are rounded to it).
    pub fn precision(mut self, value: u8) -> Self {
        self.precision = value;
        self
    }

    /// Sets `min_fraction_digits`: fraction digits always shown, with trailing zeros beyond them dropped.
    pub fn min_fraction_digits(mut self, value: u8) -> Self {
        self.min_fraction_digits = Some(value);
        self
    }

    /// Writes `value` the locale's way: rounded to `precision`, and with trailing zeros
    /// beyond `min_fraction_digits` dropped when that is set.
    pub fn format(&self, sym: &NumberSymbols, value: Decimal) -> String {
        let r = value.round(self.precision);
        let places = match self.min_fraction_digits {
            // `Decimal` keeps no trailing zeros, so its scale is the digits that matter.
            Some(min) => r.scale().max(min).min(self.precision.max(min)),
            None => self.precision,
        };
        sym.format(r, places)
    }
}

impl Default for NumberOptions {
    fn default() -> Self {
        NumberOptions {
            min: None,
            max: None,
            step: Decimal::ONE,
            precision: 0,
            min_fraction_digits: None,
            unit: None,
        }
    }
}

impl NumberOptions {
    /// Values from `min` to `max` in steps of `step`, shown with `precision` fraction digits.
    pub fn range(min: Decimal, max: Decimal, step: Decimal, precision: u8) -> Self {
        NumberOptions {
            min: Some(min),
            max: Some(max),
            step,
            precision,
            min_fraction_digits: None,
            unit: None,
        }
    }

    /// Writes `unit` after the number.
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

/// Why typed text was not accepted, for the field's error line.
fn problem(ui: &Ui, text: &str, sym: &NumberSymbols, o: &NumberOptions) -> Result<Decimal, String> {
    let v = sym.parse(text).map_err(|e| match e {
        DecimalError::Empty => ui.word("Enter a number"),
        DecimalError::Invalid => ui.word("Not a number"),
        DecimalError::Overflow => ui.word("Too many digits"),
        other => other.to_string(),
    })?;
    let v = v.round(o.precision);
    if let Some(m) = o.min
        && v < m
    {
        return Err(remedy_ui_view::words::fill(
            &ui.word("At least {number}"),
            &[("number", &o.format(sym, m))],
        ));
    }
    if let Some(m) = o.max
        && v > m
    {
        return Err(remedy_ui_view::words::fill(
            &ui.word("At most {number}"),
            &[("number", &o.format(sym, m))],
        ));
    }
    Ok(v)
}

/// A number field. Typing shows an error until the text is a number in range; the value
/// signal only ever holds valid, rounded numbers. Up and Down step (Shift: ×10), Page Up and
/// Page Down step ×10, and leaving the field or pressing Enter writes the value the
/// locale's way (`1,234.5` in en, `1.234,5` in de).
/// Free decimal entry: `NumberOptions::precision(10).min_fraction_digits(0)` keeps up to ten
/// fraction digits exactly and shows `5` and `12.25`, not `5.0000000000`.
#[track_caller]
pub fn number_input(ui: &Ui, label: impl Into<String>, value: Signal<Decimal>, options: NumberOptions) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let o = std::rc::Rc::new(options);
    let field = ui.text_field("text_input", FieldKind::Single).label(label.clone());
    let error = Signal::new(None::<String>);

    let show = {
        let (f, u, o) = (field.clone(), ui.clone(), o.clone());
        move || {
            let sym = NumberSymbols::for_locale(&untrack(|| u.locale()));
            if let Ok(v) = value.get_untracked() {
                f.set_value(&o.format(&sym, v));
            }
            error.set_if_changed(None);
        }
    };
    let field = {
        let (f, u, o) = (field.clone(), ui.clone(), o.clone());
        field.on_change(move || {
            let sym = NumberSymbols::for_locale(&untrack(|| u.locale()));
            match problem(&u, &f.value(), &sym, &o) {
                Ok(v) => {
                    value.set_if_changed(v);
                    error.set_if_changed(None);
                }
                Err(e) => error.set_if_changed(Some(e)),
            }
        })
    };
    let s2 = show.clone();
    let field = field.on_submit(s2);
    let el = field.el().grow(1.0);
    let e2 = el.clone();
    let s3 = show.clone();
    let (o2, o3, u2, l2) = (o.clone(), o.clone(), ui.clone(), label.clone());
    let el = el
        .on_focus_change(move |focused| {
            if !focused {
                s3();
            }
        })
        .on_key(move |e: &KeyEvent| {
            if !e.pressed {
                return false;
            }
            let times = |n: i64| o2.step.checked_mul(Decimal::from_int(n)).unwrap_or(o2.step);
            let by = match e.key {
                Key::ArrowUp => times(if e.modifiers.shift { 10 } else { 1 }),
                Key::ArrowDown => times(if e.modifiers.shift { -10 } else { -1 }),
                Key::PageUp => times(10),
                Key::PageDown => times(-10),
                Key::Home if o2.min.is_some() && e.modifiers.ctrl => {
                    value.set(o2.min.unwrap_or_default());
                    show();
                    return true;
                }
                Key::End if o2.max.is_some() && e.modifiers.ctrl => {
                    value.set(o2.max.unwrap_or_default());
                    show();
                    return true;
                }
                _ => return false,
            };
            let cur = value.get_untracked().unwrap_or_default();
            if let Some(next) = cur.checked_add(by) {
                value.set(next.round(o2.precision).clamp_to(o2.min, o2.max));
            }
            show();
            true
        })
        .bind(move || {
            let v = value.get();
            let bad = error.get();
            let sym = NumberSymbols::for_locale(&u2.locale());
            let text = o3.format(&sym, v);
            if !e2.is_focused() {
                untrack(|| field.set_value(&text));
            }
            e2.state(States::INVALID, bad.is_some());
            let spoken = match &o3.unit {
                Some(u) => format!("{text} {u}"),
                None => text,
            };
            let mut s = Semantics::new(Role::SpinButton)
                .label(l2.clone())
                .value(spoken)
                .numeric(
                    v.to_f64(),
                    o3.min.map_or(f64::MIN, Decimal::to_f64),
                    o3.max.map_or(f64::MAX, Decimal::to_f64),
                )
                .invalid(bad.is_some())
                .action(Action::Increment)
                .action(Action::Decrement)
                // Assistive technology can type a number in, as people can.
                .action(Action::SetValue)
                .action(Action::Focus);
            if let Some(b) = bad {
                s = s.description(b);
            }
            let _ = e2.clone().semantics(s);
        });

    let mut row = w::row(ui).gap(unit).align(Align::Center).fill_width().child(el);
    if let Some(u) = &o.unit {
        row = row.child(w::text(ui, u.clone()).variant("muted"));
    }
    let err = w::column(ui);
    let e3 = err.clone();
    let u3 = ui.clone();
    let err = err.bind(move || {
        let msg = error.get();
        untrack(|| {
            e3.clear();
            if let Some(m) = msg {
                let _ = e3.clone().child(w::text(&u3, m).variant("error"));
            }
        });
    });
    w::column(ui).gap(unit / 2.0).fill_width().child(row).child(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse(s).unwrap_or_else(|e| unreachable!("{s}: {e}"))
    }

    fn shown(locale: &str, o: &NumberOptions, v: &str) -> String {
        o.format(&NumberSymbols::for_locale(locale), d(v))
    }

    #[test]
    fn default_shows_every_precision_digit() {
        let o = NumberOptions::default().precision(10);
        assert_eq!(shown("en", &o, "5"), "5.0000000000");
        assert_eq!(shown("en", &o, "12.25"), "12.2500000000");
    }

    #[test]
    fn trims_trailing_zeros_in_en() {
        let o = NumberOptions::default().precision(10).min_fraction_digits(0);
        assert_eq!(shown("en", &o, "5"), "5");
        assert_eq!(shown("en", &o, "12.25"), "12.25");
        assert_eq!(shown("en", &o, "0.1"), "0.1");
        assert_eq!(shown("en", &o, "-3.5000"), "-3.5");
        assert_eq!(shown("en", &o, "1234.5"), "1,234.5");
        assert_eq!(shown("en", &o, "0.1234567891234"), "0.1234567891", "still rounded to precision");
    }

    #[test]
    fn trims_trailing_zeros_in_de() {
        let o = NumberOptions::default().precision(10).min_fraction_digits(0);
        assert_eq!(shown("de", &o, "5"), "5");
        assert_eq!(shown("de", &o, "12.25"), "12,25");
        assert_eq!(shown("de", &o, "-3.5000"), "-3,5");
        assert_eq!(shown("de", &o, "1234.5"), "1.234,5");
    }

    #[test]
    fn minimum_digits_are_kept() {
        let o = NumberOptions::default().precision(10).min_fraction_digits(2);
        assert_eq!(shown("en", &o, "5"), "5.00");
        assert_eq!(shown("en", &o, "12.25"), "12.25");
        assert_eq!(shown("en", &o, "0.125"), "0.125");
        assert_eq!(shown("de", &o, "1234.5"), "1.234,50");
        let tight = NumberOptions::default().precision(1).min_fraction_digits(3);
        assert_eq!(shown("en", &tight, "2"), "2.000", "a minimum above precision still pads");
    }
}
