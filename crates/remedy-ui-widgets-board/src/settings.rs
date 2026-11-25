//! The editors of widget settings in the board's panel, one for each [`SettingKind`]: text
//! fields (typing is one undo step and one settled change), switches, choices (with labels
//! apart from what is stored), moments with the form pack's date and time pickers, lists,
//! and pictures the app shows and chooses.

use std::cell::Cell;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::reactive::{Effect, Signal, untrack};
use remedy_ui_engine::script::{SettingKind, SettingSpec};
use remedy_ui_platform::services_i18n::{Date, Time};
use remedy_ui_engine::view::{Align, El, Ui};
use remedy_ui_widgets_core::{self as w, Icon};
use remedy_ui_widgets_forms::{date_picker, time_picker};
use serde_json::Value;

use crate::board::BoardState;
use crate::kinds::Context;
use crate::labels::fill;

const DAY: i64 = 86_400;

/// A setting's value as text for a field.
fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A moment setting's value: seconds since 1970 as a number, or as text of one.
pub(crate) fn seconds(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// The local date and time (to the minute) of `secs` at `offset` seconds east of UTC.
pub(crate) fn local_parts(secs: i64, offset: i32) -> (Date, Time) {
    let local = secs + i64::from(offset);
    let day = Date::from_days_since_epoch(local.div_euclid(DAY));
    let minutes = local.rem_euclid(DAY) / 60;
    let time = Time::new(u8::try_from(minutes / 60).unwrap_or(0), u8::try_from(minutes % 60).unwrap_or(0))
        .unwrap_or_default();
    (day, time)
}

/// Seconds since 1970 of local `date` and `time` at `offset` seconds east of UTC.
pub(crate) fn from_local(date: Date, time: Time, offset: i32) -> i64 {
    date.days_since_epoch() * DAY + i64::from(time.minutes()) * 60 - i64::from(offset)
}

/// A list setting's items: an array of values (each as text), or older text one a line.
pub(crate) fn items(v: &Value) -> Vec<String> {
    match v {
        Value::Array(list) => list.iter().map(as_text).collect(),
        Value::String(s) if !s.is_empty() => s.lines().map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

fn list_value(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

/// The editor of setting `spec` of widget `id`, now at `now`.
pub(crate) fn setting_editor(
    ui: &Ui,
    state: &BoardState,
    cx: &Context,
    id: &str,
    spec: &SettingSpec,
    now: &Value,
) -> El {
    let (st, wid, key) = (state.clone(), id.to_owned(), spec.key.clone());
    let words = &cx.labels;
    match &spec.kind {
        SettingKind::Bool => {
            let on = Signal::new(now.as_bool().unwrap_or(false));
            let first = Cell::new(true);
            let _ = Effect::new(move || {
                let v = on.get();
                if !first.replace(false) {
                    untrack(|| st.panel_set(&wid, &key, Value::Bool(v)));
                }
            });
            w::check(ui, spec.label.clone(), on)
        }
        SettingKind::Choice(options) => {
            let pairs: Vec<(String, String)> = options.iter().map(|o| (o.clone(), o.clone())).collect();
            choice(ui, state, words, id, spec, &pairs, now)
        }
        SettingKind::LabeledChoice(pairs) => choice(ui, state, words, id, spec, pairs, now),
        SettingKind::Time => moment(ui, state, words, id, spec, now),
        SettingKind::List => list(ui, state, words, id, spec, now),
        SettingKind::Picture => picture(ui, state, cx, id, spec, now),
        kind => {
            let long = matches!(kind, SettingKind::LongText);
            let field = if long {
                w::text_area(ui, spec.label.clone())
            } else {
                w::text_input(ui, spec.label.clone())
            };
            field.set_value(&as_text(now));
            let (f2, kind, u) = (field.clone(), kind.clone(), ui.clone());
            let field = field.on_change(move || {
                let text = f2.value();
                let v = match &kind {
                    SettingKind::Number { min, max } => match text.trim().parse::<f64>() {
                        Ok(n) => {
                            let n = min.map_or(n, |m| n.max(m));
                            Value::from(max.map_or(n, |m| n.min(m)))
                        }
                        // Not a number yet: nothing changes.
                        Err(_) => return,
                    },
                    _ => Value::String(text),
                };
                st.typed(&u, &wid, &key, v);
            });
            let field = if long {
                field
            } else {
                let s2 = state.clone();
                field.on_submit(move || s2.finish_typing())
            };
            let el = finish_on_blur(field.el(), state).fill_width();
            if long { el.height(96.0) } else { el }
        }
    }
}

/// Typing in `el` is finished when focus leaves it.
fn finish_on_blur(el: El, state: &BoardState) -> El {
    let st = state.clone();
    el.on_focus_change(move |on| {
        if !on {
            st.finish_typing();
        }
    })
}

/// One of `pairs` (stored value, label). A stored value no longer offered stays chosen and
/// says so, rather than changing silently.
fn choice(
    ui: &Ui,
    state: &BoardState,
    words: &crate::BoardLabels,
    id: &str,
    spec: &SettingSpec,
    pairs: &[(String, String)],
    now: &Value,
) -> El {
    let mut pairs = pairs.to_vec();
    let current = as_text(now);
    let at = match pairs.iter().position(|(v, _)| *v == current) {
        Some(i) => i,
        None if !current.is_empty() => {
            pairs
                .insert(0, (current.clone(), fill(&words.choice_unavailable, &[("value", current.clone())])));
            0
        }
        None => 0,
    };
    let labels: Vec<String> = pairs.iter().map(|(_, l)| l.clone()).collect();
    let idx = Signal::new(at);
    let (st, wid, key, first) = (state.clone(), id.to_owned(), spec.key.clone(), Cell::new(true));
    let _ = Effect::new(move || {
        let i = idx.get();
        if !first.replace(false)
            && let Some((v, _)) = pairs.get(i)
        {
            untrack(|| st.panel_set(&wid, &key, Value::String(v.clone())));
        }
    });
    w::select(ui, spec.label.clone(), labels, idx)
}

/// A moment: the form pack's date and time pickers in the viewer's local time, stored as
/// seconds since 1970; not set until either is changed, and clearable.
fn moment(
    ui: &Ui,
    state: &BoardState,
    words: &crate::BoardLabels,
    id: &str,
    spec: &SettingSpec,
    now: &Value,
) -> El {
    let unit = ui.theme().spacing.unit;
    let clock = ui.local_time();
    let offset = clock.utc_offset;
    let set = seconds(now);
    // Unset, the pickers start at this minute.
    let (d, t) = local_parts(set.unwrap_or(clock.unix_seconds), offset);
    let (date, time) = (Signal::new(d), Signal::new(t));
    let (st, wid, key, first) = (state.clone(), id.to_owned(), spec.key.clone(), Cell::new(true));
    let _ = Effect::new(move || {
        let (d, t) = (date.get(), time.get());
        if first.replace(false) {
            return;
        }
        let v = Value::from(from_local(d, t, offset));
        untrack(|| {
            if set.is_some() {
                st.panel_set(&wid, &key, v);
            } else {
                // From unset to set: the panel shows Clear now.
                st.apply(st.current().set_setting(&wid, &key, v));
            }
        });
    });
    let label = spec.label.clone();
    let mut col = w::column(ui)
        .gap(unit)
        .fill_width()
        .child(date_picker(ui, fill(&words.time_date, &[("label", label.clone())]), date, None, None))
        .child(time_picker(ui, fill(&words.time_time, &[("label", label.clone())]), time, 5));
    match set {
        Some(_) => {
            let (st, wid, key) = (state.clone(), id.to_owned(), spec.key.clone());
            col = col.child(
                w::button(ui, words.time_clear.clone(), move || {
                    st.apply(st.current().set_setting(&wid, &key, Value::Null));
                })
                .variant("quiet"),
            );
        }
        None => col = col.child(w::text(ui, words.time_not_set.clone()).variant("muted")),
    }
    col.semantics(Semantics::new(Role::Group).label(label))
}

/// A list: one field an item, each removable, and a button that adds one (and moves focus
/// to it).
fn list(
    ui: &Ui,
    state: &BoardState,
    words: &crate::BoardLabels,
    id: &str,
    spec: &SettingSpec,
    now: &Value,
) -> El {
    let unit = ui.theme().spacing.unit;
    let shown = items(now);
    let label = spec.label.clone();
    let mut col = w::column(ui).gap(unit).fill_width();
    // The list as the board has it now (other items may have been typed into since).
    let current = {
        let (st, wid, key, default) = (state.clone(), id.to_owned(), spec.key.clone(), spec.default.clone());
        move || {
            let b = st.current();
            items(b.widget(&wid).and_then(|w| w.settings.get(&key)).unwrap_or(&default))
        }
    };
    let current = std::rc::Rc::new(current);
    for (i, item) in shown.iter().enumerate() {
        let number = (i + 1).to_string();
        let field = w::text_input(
            ui,
            fill(&words.list_item, &[("label", label.clone()), ("number", number.clone())]),
        );
        field.set_value(item);
        let (f2, st, wid, key, cur, u) =
            (field.clone(), state.clone(), id.to_owned(), spec.key.clone(), current.clone(), ui.clone());
        let s2 = state.clone();
        let field = field
            .on_change(move || {
                let mut list = cur();
                if let Some(slot) = list.get_mut(i) {
                    *slot = f2.value();
                    st.typed(&u, &wid, &key, list_value(list));
                }
            })
            .on_submit(move || s2.finish_typing());
        let field_el = finish_on_blur(field.el(), state).grow(1.0);
        if state.take_focus_item(id, &spec.key, i) {
            state.focus_after_build(field_el.clone());
        }
        let (st, wid, key, cur) = (state.clone(), id.to_owned(), spec.key.clone(), current.clone());
        let remove = w::icon_button(
            ui,
            Icon::Close,
            fill(&words.list_remove, &[("label", label.clone()), ("number", number)]),
            move || {
                let mut list = cur();
                if i < list.len() {
                    list.remove(i);
                    st.apply(st.current().set_setting(&wid, &key, list_value(list)));
                }
            },
        );
        col = col.child(w::row(ui).gap(unit).align(Align::Center).fill_width().child(field_el).child(remove));
    }
    let (st, wid, key, cur) = (state.clone(), id.to_owned(), spec.key.clone(), current);
    let add = w::button(ui, fill(&words.list_add, &[("label", label.clone())]), move || {
        let mut list = cur();
        list.push(String::new());
        st.focus_item_next(&wid, &key, list.len() - 1);
        st.apply(st.current().set_setting(&wid, &key, list_value(list)));
    })
    .variant("quiet");
    col.child(add).semantics(Semantics::new(Role::Group).label(label))
}

/// A picture: the app's preview of its source, the source as text, and the app's chooser.
fn picture(ui: &Ui, state: &BoardState, cx: &Context, id: &str, spec: &SettingSpec, now: &Value) -> El {
    let unit = ui.theme().spacing.unit;
    let words = &cx.labels;
    let source = as_text(now);
    let mut col = w::column(ui).gap(unit).fill_width();
    if let Some(show) = &cx.picture
        && !source.trim().is_empty()
    {
        col = col.child(show(ui, &source));
    }
    let field = w::text_input(ui, fill(&words.picture_source, &[("label", spec.label.clone())]));
    field.set_value(&source);
    let (f2, st, wid, key, u) = (field.clone(), state.clone(), id.to_owned(), spec.key.clone(), ui.clone());
    let s2 = state.clone();
    let field = field
        .on_change(move || st.typed(&u, &wid, &key, Value::String(f2.value())))
        .on_submit(move || s2.finish_typing());
    col = col.child(finish_on_blur(field.el(), state).fill_width());
    if let Some(choose) = &cx.choose_picture {
        let (choose, st, wid, key) = (choose.clone(), state.clone(), id.to_owned(), spec.key.clone());
        col = col.child(
            w::button(ui, words.picture_choose.clone(), move || {
                let (st, wid, key) = (st.clone(), wid.clone(), key.clone());
                choose(Box::new(move |src| {
                    st.apply(st.current().set_setting(&wid, &key, Value::String(src)));
                }));
            })
            .variant("quiet"),
        );
    }
    col.semantics(Semantics::new(Role::Group).label(spec.label.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_are_unix_seconds_shown_in_local_time() {
        // 2026-11-19 13:05 UTC.
        let secs = 1_795_093_500;
        let (d, t) = local_parts(secs, 0);
        assert_eq!((d.year, d.month, d.day, t.hour, t.minute), (2026, 11, 19, 13, 5));
        let (d2, t2) = local_parts(secs, -5 * 3600);
        assert_eq!((d2.day, t2.hour), (19, 8));
        assert_eq!(from_local(d2, t2, -5 * 3600), secs);
        let (d3, t3) = local_parts(secs, 11 * 3600);
        assert_eq!((d3.day, t3.hour), (20, 0));
        assert_eq!(from_local(d3, t3, 11 * 3600), secs);
        assert_eq!(seconds(&Value::from("1795093500")), Some(secs), "text of seconds reads too");
        assert_eq!(seconds(&Value::Null), None);
    }

    #[test]
    fn lists_read_arrays_and_older_lines() {
        assert_eq!(items(&serde_json::json!(["a", 2])), ["a", "2"]);
        assert_eq!(items(&Value::from("x\ny")), ["x", "y"]);
        assert!(items(&Value::Null).is_empty());
    }
}
