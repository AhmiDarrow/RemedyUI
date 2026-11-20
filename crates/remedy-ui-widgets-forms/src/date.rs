//! The calendar, the date picker and the time picker.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::StylePatch;
use remedy_ui_view::{Align, El, FieldKind, Justify, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

use remedy_ui_services_i18n::{CalendarSymbols, Date, NumberSymbols, Time, month_grid};

/// A chevron button pointing back or forward in reading order: mirrored in right-to-left
/// locales, where "next" is to the left.
fn step_button(ui: &Ui, forward: bool, label: &str, on_press: impl Fn() + 'static) -> El {
    let b = w::icon_button(ui, Icon::ChevronRight, label, on_press);
    let (b2, u) = (b.clone(), ui.clone());
    b.bind(move || {
        let rtl = remedy_ui_text::is_rtl_locale(&u.locale());
        let glyph = if forward != rtl { Icon::ChevronRight } else { Icon::ChevronLeft };
        untrack(|| {
            b2.clear();
            let _ = b2.clone().child(w::icon(&u, glyph, u.theme().type_scale.body + 3.0));
        });
    })
}

/// A month calendar for choosing a day. Arrow keys move by day and week, Page Up and Page
/// Down by month (with Shift, by year), Home and End to the week's ends, Enter or Space
/// chooses. Days outside `min..=max` cannot be chosen.
#[track_caller]
pub fn calendar(
    ui: &Ui,
    label: impl Into<String>,
    value: Signal<Date>,
    min: Option<Date>,
    max: Option<Date>,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let cursor = Signal::new(value.get_untracked().unwrap_or_else(|_| untrack(|| Date::today(ui))));
    let allowed = move |d: Date| min.is_none_or(|m| d >= m) && max.is_none_or(|m| d <= m);

    // Month names, weekdays and dates come formatted from the locale's data: data text.
    let title = w::text(ui, "").variant("strong").grow(1.0).data_text();
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
        .child(step_button(ui, false, &ui.word("Previous month"), move || {
            cursor.update(|c| *c = c.add_months(-1))
        }))
        .child(title)
        .child(step_button(ui, true, &ui.word("Next month"), move || {
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
            let today = Date::today(&u3);
            let sym = CalendarSymbols::for_locale(&u3.locale());
            let num = NumberSymbols::for_locale(&u3.locale());
            untrack(|| {
                g2.clear();
                let days = month_grid(c, sym.first_weekday);
                let mut head = w::row(&u3).fill_width();
                for i in 0..7 {
                    let name = sym.weekday(sym.first_weekday.plus(i)).to_owned();
                    head = head.child(
                        w::text(&u3, name)
                            .variant("muted")
                            .data_text()
                            .grow(1.0)
                            .style(StylePatch::default().text_size(u3.theme().type_scale.small)),
                    );
                }
                let mut g = g2.clone().child(head);
                for week in days.chunks(7) {
                    let mut row = w::row(&u3).fill_width().gap(unit / 2.0);
                    for d in week {
                        let d = *d;
                        let text =
                            num.format(remedy_ui_services_i18n::Decimal::from_int(i64::from(d.day)), 0);
                        let variant = if d == chosen { "primary" } else { "quiet" };
                        // The keyboard cursor gets a ring in the accent color.
                        let ring = {
                            let mut v_ = StylePatch::default();
                            v_.border = (d == c).then_some(u3.theme().colors.accent);
                            v_.border_width = (d == c).then_some(2.0);
                            v_
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
                            .child(
                                w::text(&u3, text)
                                    .variant(if d.month == c.month { "" } else { "muted" })
                                    // Today is bold, with a line under it.
                                    .style(if d == today {
                                        StylePatch::default().weight(700.0)
                                    } else {
                                        StylePatch::default()
                                    }),
                            )
                            .child(if d == today {
                                u3.el("today_mark").width(14.0).height(2.0).style(
                                    StylePatch::default().background(u3.theme().colors.accent).radius(1.0),
                                )
                            } else {
                                u3.el("spacer").height(2.0)
                            })
                            .disabled(!ok)
                            .semantics(
                                Semantics::new(Role::GridCell)
                                    .label(if d == today {
                                        remedy_ui_view::words::fill(
                                            &u3.word("{date}, today"),
                                            &[("date", &sym.long_date(d))],
                                        )
                                    } else {
                                        sym.long_date(d)
                                    })
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
    let grid = grid.semantics(move || {
        let c = cursor.get();
        Semantics::new(Role::Grid).label(l2.clone()).value(c.to_string())
    });
    w::column(ui).gap(unit).child(header).child(grid)
}

/// A date field: shows the chosen date as the locale writes it and opens a [`calendar`] in a
/// popover. The date can also be typed as `YYYY-MM-DD`.
#[track_caller]
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
    let text = button.child_elements().into_iter().next().map(El::data_text);
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
#[track_caller]
pub fn time_picker(ui: &Ui, label: impl Into<String>, value: Signal<Time>, step_minutes: u8) -> El {
    let label = label.into();
    let step = i32::from(step_minutes.max(1));
    let field = ui.text_field("text_input", FieldKind::Single).label(label.clone());
    let invalid = Signal::new(false);
    let u2 = ui.clone();
    let show = {
        let f = field.clone();
        move || {
            let sym = CalendarSymbols::for_locale(&untrack(|| u2.locale()));
            if let Ok(t) = value.get_untracked() {
                f.set_value(&sym.format_time(t));
            }
            invalid.set_if_changed(false);
        }
    };
    let (f2, u5) = (field.clone(), ui.clone());
    let field = field.on_change(move || {
        let tag = untrack(|| u5.locale());
        let parsed =
            CalendarSymbols::for_locale(&tag).parse_time(&f2.value(), &NumberSymbols::for_locale(&tag));
        match parsed {
            Some(t) => {
                value.set_if_changed(t);
                invalid.set_if_changed(false);
            }
            None => invalid.set_if_changed(true),
        }
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
        let sym = CalendarSymbols::for_locale(&u3.locale());
        let shown = sym.format_time(t);
        if !e2.is_focused() {
            untrack(|| field.set_value(&shown));
        }
        e2.state(remedy_ui_style::States::INVALID, bad);
        let _ = e2.clone().semantics(
            Semantics::new(Role::TimeInput)
                .label(label.clone())
                .value(shown)
                .invalid(bad)
                .action(Action::Increment)
                .action(Action::Decrement)
                .action(Action::Focus),
        );
    })
}
