//! Form widgets on a real view with the theme sheet, and the values and locale data under them.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_core::{Color, Size};
use remedy_ui_input::{Chord, Key, KeyEvent, Modifiers};
use remedy_ui_reactive::Signal;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_forms::*;

fn d(s: &str) -> Decimal {
    Decimal::parse(s).unwrap_or_else(|e| unreachable!("{s}: {e}"))
}

struct T {
    ui: Ui,
}

impl T {
    fn new(locale: &str) -> T {
        let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts);
        ui.set_locale(locale);
        T { ui }
    }

    fn frame(&self) {
        let _ = self.ui.frame(Size::new(500.0, 600.0), 1.0);
        let _ = self.ui.take_requests();
    }

    fn key_with(&self, key: Key, modifiers: Modifiers) -> bool {
        let r = self.ui.key(&KeyEvent {
            key,
            physical: String::new(),
            modifiers,
            pressed: true,
            repeat: false,
            composing: false,
            text: None,
        });
        self.frame();
        r
    }

    fn key(&self, key: Key) -> bool {
        self.key_with(key, Modifiers::NONE)
    }

    /// Replaces the focused field's text, as typing over a selection does.
    fn type_text(&self, s: &str) {
        self.key_with(Key::Character("a".into()), Modifiers { ctrl: true, ..Modifiers::NONE });
        self.ui.paste(s);
        self.frame();
    }

    fn tree(&self) -> String {
        format!("{:?}", self.ui.take_accessibility_update())
    }
}

/// The first focusable element under `el` (the text field inside a widget).
fn field_in(t: &T, el: &El) -> El {
    t.frame();
    let mut todo = vec![el.clone()];
    while let Some(e) = todo.pop() {
        if e.focus_first() {
            return e;
        }
        todo.extend(e.child_elements());
    }
    unreachable!("no focusable element")
}

#[test]
fn decimals_are_exact() {
    assert_eq!(d("0.1").checked_add(d("0.2")), Some(d("0.3")));
    assert_eq!(d("1.10"), d("1.1"), "normalized");
    assert_eq!(d("-2.5").round(0), d("-3"), "halves away from zero");
    assert_eq!(d("2.45").round(1), d("2.5"));
    assert!(d("-1.5") < d("-1.2") && d("-0.5") < d("0.3") && d("10") > d("9.999"));
    assert_eq!(d("1.5").checked_mul(d("0.2")), Some(d("0.3")));
    assert_eq!(d("12.345").to_string(), "12.345");
    assert_eq!(d("-0.05").digits(2), (true, "0".into(), "05".into()));
    assert_eq!(Decimal::parse("1.2.3"), Err(DecimalError::Invalid));
    assert_eq!(Decimal::parse("  "), Err(DecimalError::Empty));
    assert_eq!(Decimal::parse(&"9".repeat(60)), Err(DecimalError::Overflow));
    assert_eq!(d("3.75").clamp_to(Some(d("0")), Some(d("2"))), d("2"));
    assert_eq!(Decimal::from_f64(0.1 + 0.2, 2), Some(d("0.3")));
}

#[test]
fn locales_write_and_read_numbers_their_own_way() {
    let v = d("1234567.891");
    let en = NumberSymbols::for_locale("en-US");
    assert_eq!(en.format(v, 2), "1,234,567.89");
    let de = NumberSymbols::for_locale("de-DE");
    assert_eq!(de.format(v, 2), "1.234.567,89");
    let fr = NumberSymbols::for_locale("fr-FR");
    assert_eq!(fr.decimal, ',');
    assert!(fr.group.is_some_and(|g| g.is_whitespace() || g == '\u{202F}'), "{:?}", fr.group);
    let india = NumberSymbols::for_locale("en-IN");
    assert_eq!(india.grouping, (3, 2));
    assert_eq!(india.format(d("1234567"), 0), "12,34,567");
    let arabic = NumberSymbols::for_locale("ar-EG");
    assert_eq!(arabic.digits[1], '١', "Arabic-Indic digits");
    assert_eq!(arabic.parse(&arabic.format(d("42.5"), 1)), Ok(d("42.5")), "reads its own output");
    // Reading what people type.
    assert_eq!(de.parse("1.234,5"), Ok(d("1234.5")));
    assert_eq!(de.parse("1,5"), Ok(d("1.5")));
    assert_eq!(de.parse("1.5"), Ok(d("1.5")), "the other convention, unambiguous");
    assert_eq!(en.parse("1,234.5"), Ok(d("1234.5")));
    assert_eq!(en.parse("1,5"), Ok(d("1.5")));
    assert_eq!(en.parse("-12"), Ok(d("-12")));
    assert_eq!(en.parse("1,2,3"), Err(DecimalError::Invalid));
    assert_eq!(en.parse("12a"), Err(DecimalError::Invalid));
    assert_eq!(en.format(d("-0.001"), 2), "0.00", "no minus on a rounded zero");
}

#[test]
fn dates_and_times_follow_the_calendar() {
    let Some(day) = Date::new(2026, 11, 17) else { unreachable!() };
    assert_eq!(day.weekday(), Weekday::Tuesday);
    assert_eq!(Date::from_days_since_epoch(0).to_string(), "1970-01-01");
    assert_eq!(Date::from_days_since_epoch(day.days_since_epoch()), day);
    assert_eq!(Date::new(2024, 2, 29).map(|d| d.add_months(12).to_string()), Some("2025-02-28".into()));
    assert_eq!(Date::new(2026, 1, 31).map(|d| d.add_months(1).to_string()), Some("2026-02-28".into()));
    assert_eq!(Date::new(2023, 2, 29), None);
    assert!(is_leap(2000) && !is_leap(1900) && is_leap(2024));
    assert_eq!(Date::parse_iso("2026-11-17"), Some(day));
    let grid = month_grid(day, Weekday::Sunday);
    assert_eq!((grid[0].to_string(), grid[0].weekday()), ("2026-11-01".into(), Weekday::Sunday));
    let grid = month_grid(day, Weekday::Monday);
    assert_eq!(grid[0].to_string(), "2026-10-26");
    assert_eq!(Time::parse("1:05 pm"), Time::new(13, 5));
    assert_eq!(Time::parse("12 AM"), Time::new(0, 0));
    assert_eq!(Time::parse("1305"), Time::new(13, 5));
    assert_eq!(Time::parse("9"), Time::new(9, 0));
    assert_eq!(Time::parse("13 pm"), None);
    assert_eq!(Time::new(23, 50).map(|t| t.add_minutes(15)), Time::new(0, 5));
    assert_eq!(Time::new(0, 7).map(|t| t.format(true)), Some("12:07 AM".into()));
}

#[test]
fn calendars_speak_the_locale() {
    let us = CalendarSymbols::for_locale("en-US");
    assert_eq!(us.first_weekday, Weekday::Sunday);
    assert!(us.twelve_hour);
    assert_eq!(us.month_year(2026, 11), "November 2026");
    let de = CalendarSymbols::for_locale("de-DE");
    assert_eq!(de.first_weekday, Weekday::Monday);
    assert!(!de.twelve_hour);
    assert!(de.month_year(2026, 11).contains("November"), "{}", de.month_year(2026, 11));
    assert!(de.weekday(Weekday::Monday).starts_with("Mo"), "{}", de.weekday(Weekday::Monday));
    let Some(day) = Date::new(2026, 11, 17) else { unreachable!() };
    assert_eq!(us.long_date(day), "November 17, 2026");
}

#[test]
fn number_input_keeps_only_valid_rounded_values() {
    let t = T::new("de-DE");
    t.ui.connect_accessibility();
    let v = Signal::new(d("10"));
    let el = number_input(&t.ui, "Lye", v, NumberOptions::range(d("0"), d("500"), d("0.5"), 2).unit("g"));
    t.ui.root().child(el.clone());
    let f = field_in(&t, &el);
    t.type_text("12,345");
    assert_eq!(v.get(), d("12.35"), "rounded to two places");
    t.type_text("abc");
    assert_eq!(v.get(), d("12.35"), "bad text never reaches the value");
    let tree = t.tree();
    assert!(tree.contains("Not a number") && tree.contains("SpinButton"), "{tree}");
    t.type_text("600");
    assert_eq!(v.get(), d("12.35"), "out of range");
    assert!(t.tree().contains("At most 500,00"));
    t.type_text("1.234,5");
    assert_eq!(v.get(), d("12.35"), "1234.5 is still out of range");
    t.type_text("2,5");
    assert_eq!(v.get(), d("2.5"));
    t.key(Key::ArrowUp);
    assert_eq!(v.get(), d("3"));
    t.key_with(Key::ArrowDown, Modifiers { shift: true, ..Modifiers::NONE });
    assert_eq!(v.get(), d("0"), "clamped at the minimum");
    f.blur();
    t.frame();
    assert!(t.tree().contains("0,00 g"), "written the German way with its unit");
}

#[test]
fn color_picker_moves_by_keyboard_and_reports_contrast() {
    let t = T::new("en-US");
    t.ui.connect_accessibility();
    let c = Signal::new(Color::from_hex("#336699").unwrap_or(Color::BLACK));
    let el = color_picker(&t.ui, "Accent", c, Some(Color::WHITE));
    t.ui.root().child(el.clone());
    let area = field_in(&t, &el);
    let before = Hsv::from_color(c.get());
    t.key(Key::ArrowUp);
    let after = Hsv::from_color(c.get());
    assert!((after.v - before.v - 0.01).abs() < 0.002, "brightness up 1%");
    assert!((after.h - before.h).abs() < 0.5, "hue kept");
    area.blur();
    let tree = t.tree();
    assert!(tree.contains("Contrast") && tree.contains("ColorWell"), "{tree}");
    // Gray keeps the hue it had.
    c.set(Color::new(0.5, 0.5, 0.5, 1.0));
    t.frame();
    assert_eq!(contrast_grade(4.6), "AA");
    assert_eq!(contrast_grade(2.0), "Fails");
    let round = Hsv::from_color(Color::from_hex("#ff8000").unwrap_or(Color::BLACK)).to_color();
    assert_eq!(round.to_hex().to_lowercase(), "#ff8000");
}

#[test]
fn key_capture_records_a_chord_and_clears() {
    let t = T::new("en-US");
    let v = Signal::new(None::<Chord>);
    let el = key_capture(&t.ui, "Open notes", v);
    t.ui.root().child(el.clone());
    let _ = field_in(&t, &el);
    // Modifiers alone record nothing.
    t.key_with(Key::Named("Control".into()), Modifiers { ctrl: true, ..Modifiers::NONE });
    assert_eq!(v.get(), None);
    let m = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    t.key_with(Key::Character("k".into()), m);
    assert_eq!(v.get(), Some(Chord::new(m, Key::Character("k".into()))));
    t.key(Key::Backspace);
    assert_eq!(v.get(), None);
}

#[test]
fn calendars_choose_days_from_the_keyboard_within_limits() {
    let t = T::new("en-US");
    t.ui.connect_accessibility();
    let (Some(start), Some(max)) = (Date::new(2026, 11, 17), Date::new(2026, 12, 5)) else { unreachable!() };
    let v = Signal::new(start);
    let el = calendar(&t.ui, "Due", v, None, Some(max));
    t.ui.root().child(el.clone());
    t.frame();
    // The grid under the month header.
    let grid = el.child_elements().get(1).cloned().unwrap_or_else(|| unreachable!("a grid"));
    grid.focus();
    let _ = t.tree();
    t.key(Key::ArrowRight);
    t.key(Key::ArrowDown);
    t.key(Key::Enter);
    assert_eq!(v.get().to_string(), "2026-11-25");
    t.key(Key::PageDown);
    t.key(Key::Enter);
    assert_eq!(v.get().to_string(), "2026-12-05", "the cursor stops at the limit");
    let tree = t.tree();
    assert!(tree.contains("December 2026") && tree.contains("GridCell"), "{tree}");
}

#[test]
fn time_picker_reads_and_steps() {
    let t = T::new("en-US");
    let v = Signal::new(Time::new(9, 0).unwrap_or_default());
    let el = time_picker(&t.ui, "Remind at", v, 15);
    t.ui.root().child(el.clone());
    let _ = field_in(&t, &el);
    t.type_text("2:30 pm");
    assert_eq!(v.get(), Time::new(14, 30).unwrap_or_default());
    t.key(Key::ArrowUp);
    assert_eq!(v.get(), Time::new(14, 45).unwrap_or_default());
}

#[test]
fn forms_validate_track_changes_and_submit() {
    let t = T::new("en-US");
    let name = Signal::new(String::from("Lavender"));
    let grams = Signal::new(d("100"));
    let form = Form::new();
    let name_f = form.field(
        "Name",
        name,
        |n: &String| if n.trim().is_empty() { Err("Required".into()) } else { Ok(()) },
    );
    let _g = form.field(
        "Grams",
        grams,
        |g: &Decimal| if *g > d("0") { Ok(()) } else { Err("Must be more than 0".into()) },
    );
    let body = form_view(&t.ui, "Recipe").child(form_field(
        &t.ui,
        &name_f,
        remedy_ui_widgets_core::text(&t.ui, "name"),
    ));
    t.ui.root().child(body).child(form_status(&t.ui, &form));
    t.frame();
    assert!(form.is_valid() && !form.is_dirty());
    assert_eq!(name_f.error(), None);
    name.set(String::new());
    t.frame();
    assert!(!form.is_valid() && form.is_dirty() && name_f.error_shown());
    let saves = Rc::new(Cell::new(0));
    let s2 = saves.clone();
    assert!(!form.submit(move |_| s2.set(s2.get() + 1)), "invalid forms do not save");
    assert_eq!((form.state(), saves.get()), (SubmitState::Invalid, 0));
    name.set("Rose".into());
    t.frame();
    assert_eq!(form.state(), SubmitState::Idle, "editing clears the failed submit");
    let pending = Rc::new(std::cell::RefCell::new(None));
    let p2 = pending.clone();
    assert!(form.submit(move |s| *p2.borrow_mut() = Some(s)));
    assert_eq!(form.state(), SubmitState::Submitting);
    assert!(!form.submit(|_| {}), "no second save while one runs");
    if let Some(s) = pending.borrow_mut().take() {
        s.succeeded();
    }
    assert_eq!(form.state(), SubmitState::Saved);
    assert!(!form.is_dirty(), "saved values are the new baseline");
    grams.set(d("120"));
    assert!(form.is_dirty());
    form.reset();
    assert_eq!((grams.get(), name.get()), (d("100"), "Rose".to_owned()));
}

#[test]
fn times_are_written_and_read_the_locale_s_way() {
    let t = Time::new(14, 30).unwrap_or_default();
    for tag in ["en-US", "de-DE", "ar-EG", "ko-KR", "hi-IN"] {
        let cal = CalendarSymbols::for_locale(tag);
        let num = NumberSymbols::for_locale(tag);
        let shown = cal.format_time(t);
        assert_eq!(cal.parse_time(&shown, &num), Some(t), "{tag}: {shown}");
        assert_eq!(cal.parse_time("14:30", &num), Some(t), "{tag}: plain 24-hour always works");
    }
    assert_eq!(CalendarSymbols::for_locale("de-DE").format_time(t), "14:30");
    let ar = CalendarSymbols::for_locale("ar-EG").format_time(t);
    assert!(ar.contains('٢') && ar.contains('٣'), "Arabic digits: {ar}");
}

#[test]
fn the_calendar_marks_today_on_the_local_calendar() {
    let t = T::new("en-US");
    t.ui.connect_accessibility();
    // 2026-11-17 22:30 UTC is already the 18th at UTC+3.
    t.ui.set_local_time(remedy_ui_host::LocalTime { unix_seconds: 1_794_954_600, utc_offset: 3 * 3600 });
    assert_eq!(Date::today(&t.ui).to_string(), "2026-11-18");
    let v = Signal::new(Date::new(2026, 11, 2).unwrap_or(Date { year: 2026, month: 11, day: 2 }));
    t.ui.root().child(calendar(&t.ui, "Due", v, None, None));
    t.frame();
    let tree = t.tree();
    assert!(
        tree.contains("November 18, 2026, today") && !tree.contains("November 17, 2026, today"),
        "{tree}"
    );
}
