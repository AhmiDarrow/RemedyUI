//! Calendar dates and times of day, and the pickers for them.

use std::fmt;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::StylePatch;
use remedy_ui_view::{Align, El, FieldKind, Justify, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

use crate::locale::{CalendarSymbols, NumberSymbols};

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weekday {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl Weekday {
    const ALL: [Weekday; 7] = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];

    /// 0 for Monday … 6 for Sunday.
    pub fn index_from_monday(self) -> usize {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0)
    }

    /// The day `n` days later.
    pub fn plus(self, n: usize) -> Weekday {
        Self::ALL.get((self.index_from_monday() + n) % 7).copied().unwrap_or(self)
    }

    pub(crate) fn from_icu(d: icu_calendar::types::Weekday) -> Weekday {
        use icu_calendar::types::Weekday as I;
        match d {
            I::Monday => Weekday::Monday,
            I::Tuesday => Weekday::Tuesday,
            I::Wednesday => Weekday::Wednesday,
            I::Thursday => Weekday::Thursday,
            I::Friday => Weekday::Friday,
            I::Saturday => Weekday::Saturday,
            I::Sunday => Weekday::Sunday,
        }
    }
}

/// A date in the proleptic Gregorian calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    /// The year (astronomical: 0 is 1 BC).
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month, from 1.
    pub day: u8,
}

/// True in leap years.
pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in a month of a year.
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Date {
    /// A date, or `None` when the day does not exist.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        ((1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)).then_some(Date {
            year,
            month,
            day,
        })
    }

    /// Days since 1970-01-01 (negative before).
    pub fn days_since_epoch(self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// The date `days` after 1970-01-01.
    pub fn from_days_since_epoch(days: i64) -> Date {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Date { year, month, day }
    }

    /// The day of the week.
    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday.
        Weekday::Thursday.plus(self.days_since_epoch().rem_euclid(7) as usize)
    }

    /// `n` days later (earlier for negative `n`).
    pub fn add_days(self, n: i64) -> Date {
        Date::from_days_since_epoch(self.days_since_epoch() + n)
    }

    /// `n` months later, keeping the day where the month allows (Jan 31 + 1 month is the
    /// last day of February).
    pub fn add_months(self, n: i32) -> Date {
        let total = self.year * 12 + i32::from(self.month) - 1 + n;
        let year = total.div_euclid(12);
        let month = (total.rem_euclid(12) + 1) as u8;
        Date { year, month, day: self.day.min(days_in_month(year, month)) }
    }

    /// The first day of this date's month.
    pub fn first_of_month(self) -> Date {
        Date { day: 1, ..self }
    }

    /// Today in UTC, from the system clock.
    pub fn today_utc() -> Date {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Date::from_days_since_epoch(secs.div_euclid(86_400))
    }

    /// Reads `YYYY-MM-DD`.
    pub fn parse_iso(s: &str) -> Option<Date> {
        let mut parts = s.trim().split('-');
        let y = parts.next()?.parse().ok()?;
        let m = parts.next()?.parse().ok()?;
        let d = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Date::new(y, m, d)
    }

    /// Kept within `lo..=hi` (either bound optional).
    pub fn clamp_to(self, lo: Option<Date>, hi: Option<Date>) -> Date {
        let v = lo.map_or(self, |l| self.max(l));
        hi.map_or(v, |h| v.min(h))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A time of day to the minute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Time {
    /// Hours, 0 to 23.
    pub hour: u8,
    /// Minutes, 0 to 59.
    pub minute: u8,
}

impl Time {
    /// A time, or `None` out of range.
    pub fn new(hour: u8, minute: u8) -> Option<Time> {
        (hour < 24 && minute < 60).then_some(Time { hour, minute })
    }

    /// Minutes since midnight.
    pub fn minutes(self) -> u16 {
        u16::from(self.hour) * 60 + u16::from(self.minute)
    }

    /// `n` minutes later, wrapping around midnight.
    pub fn add_minutes(self, n: i32) -> Time {
        let m = (i32::from(self.minutes()) + n).rem_euclid(24 * 60);
        Time { hour: (m / 60) as u8, minute: (m % 60) as u8 }
    }

    /// Reads `13:05`, `1:05 pm`, `1 PM`, `1305` or `9`.
    pub fn parse(s: &str) -> Option<Time> {
        let lower = s.trim().to_lowercase();
        let (body, pm) = if let Some(b) = lower.strip_suffix("pm").or_else(|| lower.strip_suffix("p")) {
            (b.trim().to_owned(), Some(true))
        } else if let Some(b) = lower.strip_suffix("am").or_else(|| lower.strip_suffix("a")) {
            (b.trim().to_owned(), Some(false))
        } else {
            (lower.clone(), None)
        };
        let (h, m): (u8, u8) = match body.split_once([':', '.', 'h']) {
            Some((h, m)) => {
                (h.trim().parse().ok()?, if m.trim().is_empty() { 0 } else { m.trim().parse().ok()? })
            }
            None if body.len() >= 3 && body.chars().all(|c| c.is_ascii_digit()) => {
                let split = body.len() - 2;
                (body.get(..split)?.parse().ok()?, body.get(split..)?.parse().ok()?)
            }
            None => (body.parse().ok()?, 0),
        };
        let h = match pm {
            Some(_) if !(1..=12).contains(&h) => return None,
            Some(true) => h % 12 + 12,
            Some(false) => h % 12,
            None => h,
        };
        Time::new(h, m)
    }

    /// As the locale writes it: `13:05` or `1:05 PM`.
    pub fn format(self, twelve_hour: bool) -> String {
        if twelve_hour {
            let h = if self.hour.is_multiple_of(12) { 12 } else { self.hour % 12 };
            format!("{h}:{:02} {}", self.minute, if self.hour < 12 { "AM" } else { "PM" })
        } else {
            format!("{:02}:{:02}", self.hour, self.minute)
        }
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// The days a month grid shows: six weeks from the week holding the 1st.
pub fn month_grid(month: Date, first: Weekday) -> [Date; 42] {
    let start = month.first_of_month();
    let back = (start.weekday().index_from_monday() + 7 - first.index_from_monday()) % 7;
    let begin = start.add_days(-(back as i64));
    std::array::from_fn(|i| begin.add_days(i as i64))
}

/// A month calendar for choosing a day. Arrow keys move by day and week, Page Up and Page
/// Down by month (with Shift, by year), Home and End to the week's ends, Enter or Space
/// chooses. Days outside `min..=max` cannot be chosen.
pub fn calendar(
    ui: &Ui,
    label: impl Into<String>,
    value: Signal<Date>,
    min: Option<Date>,
    max: Option<Date>,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let cursor = Signal::new(value.get_untracked().unwrap_or_else(|_| Date::today_utc()));
    let allowed = move |d: Date| min.is_none_or(|m| d >= m) && max.is_none_or(|m| d <= m);

    let title = w::label(ui, "", "strong").grow(1.0);
    let t2 = title.clone();
    let u2 = ui.clone();
    let title = title.bind(move || {
        let c = cursor.get();
        let sym = CalendarSymbols::for_locale(&u2.locale());
        let text = sym.month_year(c.year, c.month);
        let _ = t2.clone().text(remedy_ui_view::RichText::plain(text.clone()));
        let _ = t2
            .clone()
            .semantics(Semantics::new(Role::Heading(3)).label(text).live(remedy_ui_a11y::Live::Polite));
    });
    let header = w::row(ui)
        .fill_width()
        .align(Align::Center)
        .gap(unit)
        .child(w::icon_button(ui, Icon::ChevronLeft, "Previous month", move || {
            cursor.update(|c| *c = c.add_months(-1))
        }))
        .child(title)
        .child(w::icon_button(ui, Icon::ChevronRight, "Next month", move || {
            cursor.update(|c| *c = c.add_months(1))
        }));

    let grid = ui.el("calendar").column().gap(unit / 2.0).fill_width().focusable(true);
    let g2 = grid.clone();
    let u3 = ui.clone();
    let l2 = label.clone();
    let u4 = ui.clone();
    let grid = grid
        .bind(move || {
            let c = cursor.get();
            let chosen = value.get();
            let sym = CalendarSymbols::for_locale(&u3.locale());
            let num = NumberSymbols::for_locale(&u3.locale());
            untrack(|| {
                g2.clear();
                let days = month_grid(c, sym.first_weekday);
                let mut head = w::row(&u3).fill_width();
                for i in 0..7 {
                    let name = sym.weekday(sym.first_weekday.plus(i)).to_owned();
                    head = head.child(w::label(&u3, name, "muted").grow(1.0).style(StylePatch {
                        text_size: Some(u3.theme().type_scale.small),
                        ..StylePatch::default()
                    }));
                }
                let mut g = g2.clone().child(head);
                for week in days.chunks(7) {
                    let mut row = w::row(&u3).fill_width().gap(unit / 2.0);
                    for d in week {
                        let d = *d;
                        let text = num.format(crate::decimal::Decimal::from_int(i64::from(d.day)), 0);
                        let variant = if d == chosen { "primary" } else { "quiet" };
                        // The keyboard cursor gets a ring in the accent color.
                        let ring = StylePatch {
                            border: (d == c).then_some(u3.theme().colors.accent),
                            border_width: (d == c).then_some(2.0),
                            ..StylePatch::default()
                        };
                        let ok = allowed(d);
                        let cell = u3
                            .el("button")
                            .variant(variant)
                            .style(ring)
                            .grow(1.0)
                            .padding(Insets::symmetric(unit, 0.0))
                            .justify(Justify::Center)
                            .align(Align::Center)
                            .child(w::label(&u3, text, if d.month == c.month { "" } else { "muted" }))
                            .disabled(!ok)
                            .semantics(
                                Semantics::new(Role::GridCell)
                                    .label(sym.long_date(d))
                                    .selected(d == chosen)
                                    .disabled(!ok)
                                    .action(Action::Click),
                            );
                        let cell = if ok {
                            cell.on_click(move || {
                                cursor.set(d);
                                value.set(d);
                            })
                        } else {
                            cell
                        };
                        row = row.child(cell);
                    }
                    g = g.child(row);
                }
                let _ = g;
            });
        })
        .on_key(move |e: &KeyEvent| {
            if !e.pressed {
                return false;
            }
            let Ok(c) = cursor.get_untracked() else { return false };
            let first = untrack(|| CalendarSymbols::for_locale(&u4.locale()).first_weekday);
            let next = match e.key {
                Key::ArrowLeft => c.add_days(-1),
                Key::ArrowRight => c.add_days(1),
                Key::ArrowUp => c.add_days(-7),
                Key::ArrowDown => c.add_days(7),
                Key::PageUp if e.modifiers.shift => c.add_months(-12),
                Key::PageDown if e.modifiers.shift => c.add_months(12),
                Key::PageUp => c.add_months(-1),
                Key::PageDown => c.add_months(1),
                Key::Home => {
                    let back = (c.weekday().index_from_monday() + 7 - first.index_from_monday()) % 7;
                    c.add_days(-(back as i64))
                }
                Key::End => {
                    let back = (c.weekday().index_from_monday() + 7 - first.index_from_monday()) % 7;
                    c.add_days(6 - back as i64)
                }
                Key::Enter | Key::Space => {
                    if allowed(c) {
                        value.set(c);
                    }
                    return true;
                }
                _ => return false,
            };
            cursor.set(next.clamp_to(min, max));
            true
        });
    let grid = grid.semantics_dyn(move || {
        let c = cursor.get();
        Semantics::new(Role::Grid).label(l2.clone()).value(c.to_string())
    });
    w::column(ui).gap(unit).child(header).child(grid)
}

/// A date field: shows the chosen date as the locale writes it and opens a [`calendar`] in a
/// popover. The date can also be typed as `YYYY-MM-DD`.
pub fn date_picker(
    ui: &Ui,
    label: impl Into<String>,
    value: Signal<Date>,
    min: Option<Date>,
    max: Option<Date>,
) -> El {
    let label = label.into();
    let ui2 = ui.clone();
    let l2 = label.clone();
    let open: std::rc::Rc<std::cell::RefCell<Option<El>>> = std::rc::Rc::default();
    let anchor: std::rc::Rc<std::cell::RefCell<Option<El>>> = std::rc::Rc::default();
    let a2 = anchor.clone();
    let button = w::button(ui, "", move || {
        if let Some(p) = open.borrow_mut().take() {
            p.remove();
            return;
        }
        if let Some(a) = a2.borrow().clone() {
            let cal = calendar(&ui2, l2.clone(), value, min, max);
            let p = w::popover(&ui2, &a, w::card(&ui2).child(cal));
            *open.borrow_mut() = Some(p);
        }
    })
    .child(w::icon(ui, Icon::ChevronDown, ui.theme().type_scale.small + 2.0));
    *anchor.borrow_mut() = Some(button.clone());
    let text = button.child_elements().into_iter().next();
    let u3 = ui.clone();
    let b2 = button.clone();
    let l3 = label.clone();
    button.bind(move || {
        let d = value.get();
        let s = CalendarSymbols::for_locale(&u3.locale()).long_date(d);
        if let Some(t) = &text {
            let _ = t.clone().text(remedy_ui_view::RichText::plain(s.clone()));
        }
        let _ = b2.clone().semantics(
            Semantics::new(Role::DateInput).label(l3.clone()).value(s).expanded(false).action(Action::Click),
        );
    })
}

/// A time field: type `13:05` or `1:05 PM`; Up and Down step by `step_minutes` (with Shift,
/// by an hour). Written in the locale's 12- or 24-hour style.
pub fn time_picker(ui: &Ui, label: impl Into<String>, value: Signal<Time>, step_minutes: u8) -> El {
    let label = label.into();
    let step = i32::from(step_minutes.max(1));
    let field = ui.text_field("text_input", FieldKind::Single).label(label.clone());
    let invalid = Signal::new(false);
    let u2 = ui.clone();
    let show = {
        let f = field.clone();
        move || {
            let twelve = CalendarSymbols::for_locale(&u2.locale()).twelve_hour;
            if let Ok(t) = value.get_untracked() {
                f.set_value(&t.format(twelve));
            }
            invalid.set_if_changed(false);
        }
    };
    let f2 = field.clone();
    let field = field.on_change(move || match Time::parse(&f2.value()) {
        Some(t) => {
            value.set_if_changed(t);
            invalid.set_if_changed(false);
        }
        None => invalid.set_if_changed(true),
    });
    let s2 = show.clone();
    let field = field.on_submit(s2);
    let el = field.el();
    let e2 = el.clone();
    let s3 = show.clone();
    let u3 = ui.clone();

    el.on_focus_change(move |focused| {
        if !focused {
            s3();
        }
    })
    .on_key(move |e: &KeyEvent| {
        if !e.pressed {
            return false;
        }
        let by = if e.modifiers.shift { 60 } else { step };
        let delta = match e.key {
            Key::ArrowUp => by,
            Key::ArrowDown => -by,
            _ => return false,
        };
        value.update(|t| *t = t.add_minutes(delta));
        show();
        true
    })
    .bind(move || {
        let t = value.get();
        let bad = invalid.get();
        let twelve = CalendarSymbols::for_locale(&u3.locale()).twelve_hour;
        if !e2.is_focused() {
            untrack(|| field.set_value(&t.format(twelve)));
        }
        e2.state(remedy_ui_style::States::INVALID, bad);
        let _ = e2.clone().semantics(
            Semantics::new(Role::TimeInput)
                .label(label.clone())
                .value(t.format(twelve))
                .invalid(bad)
                .action(Action::Increment)
                .action(Action::Decrement)
                .action(Action::Focus),
        );
    })
}
