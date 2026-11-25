//! What a locale needs for numbers, dates and times, read from ICU's data once per locale.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use icu_calendar::Gregorian;
use icu_calendar::types::Weekday as IcuWeekday;
use icu_datetime::FixedCalendarDateTimeFormatter;
use icu_datetime::fieldsets;
use icu_decimal::input::Decimal as IcuDecimal;
use icu_decimal::{CompactDecimalFormatter, DecimalFormatter};
use icu_locale_core::Locale;

use crate::services_i18n::calendar::{Date, Weekday};
use crate::services_i18n::decimal::{Decimal, DecimalError};

/// How a locale writes numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberSymbols {
    /// The decimal separator (`.` in en, `,` in de).
    pub decimal: char,
    /// The grouping separator (`,` in en, `.` in de, a narrow space in fr), if any.
    pub group: Option<char>,
    /// Digits in the primary group (3), and in the groups before it (3, or 2 in en-IN).
    pub grouping: (usize, usize),
    /// The minus sign.
    pub minus: char,
    /// The locale's digits, zero to nine.
    pub digits: [char; 10],
    /// The locale these are for, which [`NumberSymbols::compact`] formats in.
    tag: String,
}

impl Default for NumberSymbols {
    fn default() -> Self {
        NumberSymbols {
            decimal: '.',
            group: Some(','),
            grouping: (3, 3),
            minus: '-',
            digits: ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'],
            tag: String::from("und"),
        }
    }
}

fn locale(tag: &str) -> Locale {
    tag.parse().unwrap_or(Locale::UNKNOWN)
}

impl NumberSymbols {
    /// The symbols for a BCP 47 locale. Unknown locales get the root locale's.
    pub fn for_locale(tag: &str) -> NumberSymbols {
        thread_local!(static CACHE: RefCell<HashMap<String, NumberSymbols>> = RefCell::new(HashMap::new()));
        if let Some(s) = CACHE.with(|c| c.borrow().get(tag).cloned()) {
            return s;
        }
        let s =
            Self::read(tag).unwrap_or_else(|| NumberSymbols { tag: tag.to_owned(), ..Default::default() });
        CACHE.with(|c| c.borrow_mut().insert(tag.to_owned(), s.clone()));
        s
    }

    /// Reads them by formatting known numbers with ICU and looking at the result.
    fn read(tag: &str) -> Option<NumberSymbols> {
        let f = DecimalFormatter::try_new(locale(tag).into(), Default::default()).ok()?;
        // 1234567890 shows the digits in order and where groups split.
        let whole: Vec<char> = f.format(&IcuDecimal::from(1_234_567_890i64)).to_string().chars().collect();
        let mut digits = ['0'; 10];
        let mut found = 0usize;
        let mut group = None;
        let mut sizes = Vec::new();
        let mut run = 0usize;
        for c in &whole {
            if found < 10 && (c.is_numeric() || c.is_ascii_digit()) {
                // 1,2,…,9 then 0.
                let slot = (found + 1) % 10;
                if let Some(d) = digits.get_mut(slot) {
                    *d = *c;
                }
                found += 1;
                run += 1;
            } else if !c.is_control() && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}') {
                group.get_or_insert(*c);
                sizes.push(run);
                run = 0;
            }
        }
        sizes.push(run);
        if found != 10 {
            return None;
        }
        let primary = sizes.last().copied().filter(|n| *n > 0).unwrap_or(3);
        let secondary =
            sizes.iter().rev().nth(1).copied().filter(|n| *n > 0 && sizes.len() > 2).unwrap_or(primary);
        // -1.5 shows the minus sign and the decimal separator.
        let mut minus_half = IcuDecimal::from(-15i64);
        minus_half.multiply_pow10(-1);
        let half = f.format(&minus_half).to_string();
        let mut minus = '-';
        let mut decimal = '.';
        for c in half.chars() {
            if c == digits[1] || c == digits[5] || matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}') {
                continue;
            }
            if half.find(c) < half.find(digits[1]) {
                minus = c;
            } else {
                decimal = c;
            }
        }
        Some(NumberSymbols {
            decimal,
            group,
            grouping: (primary, secondary),
            minus,
            digits,
            tag: tag.to_owned(),
        })
    }

    /// A count written short, the locale's way: `1.2K` and `3.4M` in en, `1234` and `1.2万`
    /// in ja, `১.২ হা` in bn. Below a thousand it is the number itself; at most one fraction
    /// digit, and only below ten. [`NumberSymbols::compact_with`] tunes both.
    pub fn compact(&self, v: Decimal) -> String {
        self.compact_with(v, CompactOptions::default())
    }

    /// [`NumberSymbols::compact`] with `options`: where shortening starts and how many digits
    /// stay.
    pub fn compact_with(&self, v: Decimal, options: CompactOptions) -> String {
        type Formatters = Option<Rc<(CompactDecimalFormatter, DecimalFormatter)>>;
        thread_local!(static CACHE: RefCell<HashMap<String, Formatters>> = RefCell::new(HashMap::new()));
        let fs = CACHE.with(|c| {
            c.borrow_mut()
                .entry(self.tag.clone())
                .or_insert_with(|| {
                    let loc = locale(&self.tag);
                    let compact =
                        CompactDecimalFormatter::try_new_short((&loc).into(), Default::default()).ok()?;
                    let plain = DecimalFormatter::try_new((&loc).into(), Default::default()).ok()?;
                    Some(Rc::new((compact, plain)))
                })
                .clone()
        });
        let (m, s) = v.parts();
        let below = options.threshold.is_some_and(|t| Decimal::new(m.abs(), s).is_some_and(|a| a < t));
        let (Some(fs), Ok(mut d)) = (fs, v.to_string().parse::<IcuDecimal>()) else {
            return self.format(v, v.scale());
        };
        let (f, plain) = (&fs.0, &fs.1);
        if below {
            return plain.format(&d).to_string();
        }
        // Twice at most: rounding 999.95K up to 1000K moves it into the next unit (1M).
        for _ in 0..2 {
            let magnitude = d.nonzero_magnitude_start();
            let exponent = f.compact_exponent_for_magnitude(magnitude);
            if exponent == 0 {
                // Below the locale's first unit: the count itself, grouped as CLDR groups
                // compact numbers (1234 in de, 15.000).
                return f.format(&d).to_string();
            }
            let whole = magnitude - i16::from(exponent) + 1;
            let mut sig = d.clone();
            sig.multiply_pow10(-i16::from(exponent));
            let keep = match options.max_fraction_digits {
                Some(n) => i16::from(n),
                None => (i16::from(options.significant_digits) - whole).max(0),
            };
            sig.round(-keep);
            if f.compact_exponent_for_magnitude(sig.nonzero_magnitude_start() + i16::from(exponent))
                != exponent
            {
                sig.multiply_pow10(i16::from(exponent));
                d = sig;
                continue;
            }
            sig.trim_end();
            if let Ok(w) = f.format_with_exponent(&sig, exponent) {
                return w.to_string();
            }
            break;
        }
        f.format(&d).to_string()
    }

    /// Writes `v` with exactly `places` fraction digits (rounded), grouped.
    pub fn format(&self, v: Decimal, places: u8) -> String {
        let (neg, int, frac) = v.digits(places);
        let mut out = String::new();
        if neg && (int.bytes().any(|b| b != b'0') || frac.bytes().any(|b| b != b'0')) {
            out.push(self.minus);
        }
        let n = int.len();
        for (i, c) in int.chars().enumerate() {
            let left = n - i;
            out.push(self.digit(c));
            if let Some(g) = self.group
                && left > 1
            {
                let after = left - 1;
                let (p, s) = self.grouping;
                // Separators sit after the primary group, then after every secondary group.
                let at_boundary = after == p || (after > p && s > 0 && (after - p) % s == 0);
                if at_boundary {
                    out.push(g);
                }
            }
        }
        if !frac.is_empty() {
            out.push(self.decimal);
            out.extend(frac.chars().map(|c| self.digit(c)));
        }
        out
    }

    fn digit(&self, ascii: char) -> char {
        ascii.to_digit(10).and_then(|d| self.digits.get(d as usize).copied()).unwrap_or(ascii)
    }

    /// Reads what a person typed: the locale's digits or ASCII ones, its decimal separator,
    /// and group separators where groups belong. A lone `.` or `,` that cannot be a group
    /// separator is read as the decimal point, so either convention works.
    pub fn parse(&self, text: &str) -> Result<Decimal, DecimalError> {
        let spaces = |c: char| matches!(c, ' ' | '\u{00A0}' | '\u{202F}' | '\u{2009}');
        let mut plain = String::new();
        let chars: Vec<char> =
            text.trim().chars().filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}')).collect();
        if chars.is_empty() {
            return Err(DecimalError::Empty);
        }
        let group_is_space = self.group.is_some_and(spaces);
        let decimals = chars.iter().filter(|c| **c == self.decimal).count();
        for (i, c) in chars.iter().enumerate() {
            let c = *c;
            if let Some(d) = self.digits.iter().position(|d| *d == c) {
                plain.push(char::from(b'0' + d as u8));
            } else if c.is_ascii_digit() {
                plain.push(c);
            } else if c == self.minus || c == '-' || c == '\u{2212}' {
                if !plain.is_empty() {
                    return Err(DecimalError::Invalid);
                }
                plain.push('-');
            } else if c == '+' && plain.is_empty() {
            } else if c == self.decimal {
                plain.push('.');
            } else if Some(c) == self.group || (group_is_space && spaces(c)) {
                // A group separator must be followed by a whole group of digits.
                let run = chars
                    .iter()
                    .skip(i + 1)
                    .take_while(|d| d.is_ascii_digit() || self.digits.contains(d))
                    .count();
                let next = chars.get(i + 1 + run).copied();
                let group_ok = run == self.grouping.0 || run == self.grouping.1;
                let ends_ok = next.is_none() || next == Some(self.decimal) || next == self.group;
                if group_ok && ends_ok {
                    continue;
                }
                // Typed the other convention ("1.5" in de): one separator, no decimal.
                if decimals == 0 && matches!(c, '.' | ',') && chars.iter().filter(|x| **x == c).count() == 1 {
                    plain.push('.');
                } else {
                    return Err(DecimalError::Invalid);
                }
            } else if matches!(c, '.' | ',')
                && decimals == 0
                && chars.iter().filter(|x| **x == c).count() == 1
            {
                plain.push('.');
            } else {
                return Err(DecimalError::Invalid);
            }
        }
        Decimal::parse(&plain)
    }
}

/// Which way [`CalendarSymbols::format_date_time`] writes a date with a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateTimeStyle {
    /// The month abbreviated where the locale does, to the minute: "Nov 21, 2026, 2:30 PM"
    /// ([`CalendarSymbols::date_time`]).
    #[default]
    Medium,
    /// All numbers, to the second, as a browser's `Date.toLocaleString()` writes it:
    /// "11/21/2026, 1:32:31 PM" in en-US, "21.11.2026, 13:32:31" in de.
    Numeric,
}

/// How a locale writes dates and times.
#[derive(Clone, Debug)]
pub struct CalendarSymbols {
    /// The first day of the week.
    pub first_weekday: Weekday,
    /// Short weekday names, Monday first.
    pub weekdays: [String; 7],
    /// True when times are written with AM/PM.
    pub twelve_hour: bool,
    /// The locale's AM and PM markers, when it writes 12-hour times.
    pub day_periods: Option<(String, String)>,
    month_year: Option<Rc<FixedCalendarDateTimeFormatter<Gregorian, fieldsets::YM>>>,
    time: Option<Rc<FixedCalendarDateTimeFormatter<Gregorian, fieldsets::T>>>,
    full_date: Option<Rc<FixedCalendarDateTimeFormatter<Gregorian, fieldsets::YMD>>>,
    medium_date: Option<Rc<FixedCalendarDateTimeFormatter<Gregorian, fieldsets::YMD>>>,
    date_time: Option<Rc<FixedCalendarDateTimeFormatter<Gregorian, fieldsets::YMDT>>>,
    numeric: Option<Rc<FixedCalendarDateTimeFormatter<Gregorian, fieldsets::YMDT>>>,
}

impl CalendarSymbols {
    /// The symbols for a BCP 47 locale.
    pub fn for_locale(tag: &str) -> CalendarSymbols {
        thread_local!(static CACHE: RefCell<HashMap<String, CalendarSymbols>> = RefCell::new(HashMap::new()));
        if let Some(s) = CACHE.with(|c| c.borrow().get(tag).cloned()) {
            return s;
        }
        let s = Self::read(tag);
        CACHE.with(|c| c.borrow_mut().insert(tag.to_owned(), s.clone()));
        s
    }

    fn read(tag: &str) -> CalendarSymbols {
        let loc = locale(tag);
        let first_weekday = icu_calendar::week::WeekInformation::try_new((&loc).into())
            .map(|w| Weekday::from_icu(w.first_weekday))
            .unwrap_or(Weekday::Monday);
        let names =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new((&loc).into(), fieldsets::E::short())
                .ok();
        let all = [
            IcuWeekday::Monday,
            IcuWeekday::Tuesday,
            IcuWeekday::Wednesday,
            IcuWeekday::Thursday,
            IcuWeekday::Friday,
            IcuWeekday::Saturday,
            IcuWeekday::Sunday,
        ];
        let english = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let weekdays: [String; 7] = std::array::from_fn(|i| match (&names, all.get(i)) {
            (Some(f), Some(d)) => f.format(d).to_string(),
            _ => english.get(i).copied().unwrap_or_default().to_owned(),
        });
        let time = FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new((&loc).into(), fieldsets::T::hm())
            .ok()
            .map(Rc::new);
        let fmt = |h: u8| -> Option<String> {
            let t = icu_datetime::input::Time::try_new(h, 0, 0, 0).ok()?;
            Some(time.as_ref()?.format(&t).to_string())
        };
        let digits = NumberSymbols::for_locale(tag).digits;
        // What is left of a formatted hour once digits and separators go: the day period.
        let marker = |s: &str| -> String {
            s.chars()
                .filter(|c| {
                    !c.is_ascii_digit()
                        && !digits.contains(c)
                        && !matches!(c, ':' | '.' | '\u{200E}' | '\u{200F}' | '\u{061C}')
                })
                .collect::<String>()
                .trim()
                .to_owned()
        };
        let one = fmt(1).unwrap_or_default();
        let thirteen = fmt(13).unwrap_or_default();
        let twelve_hour = !marker(&one).is_empty() && marker(&one) != marker(&thirteen);
        let day_periods = twelve_hour.then(|| (marker(&one), marker(&thirteen)));
        let month_year =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new((&loc).into(), fieldsets::YM::long())
                .ok()
                .map(Rc::new);
        let full_date =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new((&loc).into(), fieldsets::YMD::long())
                .ok()
                .map(Rc::new);
        let medium_date =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new((&loc).into(), fieldsets::YMD::medium())
                .ok()
                .map(Rc::new);
        let date_time = FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
            (&loc).into(),
            fieldsets::YMDT::medium().with_time_precision(icu_datetime::options::TimePrecision::Minute),
        )
        .ok()
        .map(Rc::new);
        // `toLocaleString`'s numeric year, month, day, hour, minute and second.
        let numeric = FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
            (&loc).into(),
            fieldsets::YMDT::short()
                .with_year_style(icu_datetime::options::YearStyle::Full)
                .with_time_precision(icu_datetime::options::TimePrecision::Second),
        )
        .ok()
        .map(Rc::new);
        CalendarSymbols {
            first_weekday,
            weekdays,
            twelve_hour,
            day_periods,
            month_year,
            time,
            full_date,
            medium_date,
            date_time,
            numeric,
        }
    }

    /// "November 2026" for a month.
    pub fn month_year(&self, year: i32, month: u8) -> String {
        let d = icu_calendar::Date::try_new_gregorian(year, month, 1).ok();
        match (&self.month_year, d) {
            (Some(f), Some(d)) => f.format(&d).to_string(),
            _ => format!("{year}-{month:02}"),
        }
    }

    /// "November 17, 2026" for a date.
    pub fn long_date(&self, date: Date) -> String {
        let d = icu_calendar::Date::try_new_gregorian(date.year, date.month, date.day).ok();
        match (&self.full_date, d) {
            (Some(f), Some(d)) => f.format(&d).to_string(),
            _ => date.to_string(),
        }
    }

    /// "Nov 21, 2026" for a date: the month abbreviated where the locale does (`21.11.2026`
    /// in de, `2026/11/21` in ja).
    pub fn medium_date(&self, date: Date) -> String {
        let d = icu_calendar::Date::try_new_gregorian(date.year, date.month, date.day).ok();
        match (&self.medium_date, d) {
            (Some(f), Some(d)) => f.format(&d).to_string(),
            _ => date.to_string(),
        }
    }

    /// A date and a time to the minute, as the locale writes them together: "Nov 21, 2026,
    /// 2:30 PM" in en-US, "21.11.2026, 14:30" in de, with the locale's 12- or 24-hour clock.
    pub fn date_time(&self, date: Date, time: crate::services_i18n::calendar::Time) -> String {
        let d = icu_calendar::Date::try_new_gregorian(date.year, date.month, date.day).ok();
        let t = icu_datetime::input::Time::try_new(time.hour, time.minute, 0, 0).ok();
        match (&self.date_time, d, t) {
            (Some(f), Some(date), Some(time)) => {
                f.format(&icu_datetime::input::DateTime { date, time }).to_string()
            }
            _ => format!("{}, {}", self.medium_date(date), self.format_time(time)),
        }
    }

    /// A date and a time at `second` past the minute, written the `style` way.
    pub fn format_date_time(
        &self,
        date: Date,
        time: crate::services_i18n::calendar::Time,
        second: u8,
        style: DateTimeStyle,
    ) -> String {
        if style == DateTimeStyle::Medium {
            return self.date_time(date, time);
        }
        let second = second.min(59);
        let d = icu_calendar::Date::try_new_gregorian(date.year, date.month, date.day).ok();
        let t = icu_datetime::input::Time::try_new(time.hour, time.minute, second, 0).ok();
        match (&self.numeric, d, t) {
            // Browsers write the narrow no-break space CLDR puts before AM and PM as a space.
            (Some(f), Some(date), Some(time)) => {
                f.format(&icu_datetime::input::DateTime { date, time }).to_string().replace('\u{202F}', " ")
            }
            _ => format!("{date}, {:02}:{:02}:{second:02}", time.hour, time.minute),
        }
    }

    /// The instant `local_seconds` (seconds since 1970 on the local wall clock, as
    /// `LocalTime::local_seconds` gives them) written the `style` way.
    pub fn format_instant(&self, local_seconds: i64, style: DateTimeStyle) -> String {
        let date = Date::from_days_since_epoch(local_seconds.div_euclid(86_400));
        let s = local_seconds.rem_euclid(86_400);
        let time =
            crate::services_i18n::calendar::Time { hour: (s / 3600) as u8, minute: (s / 60 % 60) as u8 };
        self.format_date_time(date, time, (s % 60) as u8, style)
    }

    /// A time as the locale writes it (`14:30`, `2:30 PM`, or with Arabic digits and the
    /// Arabic day period).
    pub fn format_time(&self, t: crate::services_i18n::calendar::Time) -> String {
        let it = icu_datetime::input::Time::try_new(t.hour, t.minute, 0, 0).ok();
        match (&self.time, it) {
            (Some(f), Some(it)) => f.format(&it).to_string(),
            _ => t.format(self.twelve_hour),
        }
    }

    /// Reads a time typed the locale's way, or as `13:05` / `1:05 pm`.
    pub fn parse_time(
        &self,
        s: &str,
        numbers: &NumberSymbols,
    ) -> Option<crate::services_i18n::calendar::Time> {
        let mut t: String = s
            .chars()
            .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}'))
            .map(|c| numbers.digits.iter().position(|d| *d == c).map_or(c, |d| char::from(b'0' + d as u8)))
            .collect();
        if let Some((am, pm)) = &self.day_periods {
            // The marker may come before the time (Korean) or after it.
            if !pm.is_empty() && t.contains(pm.as_str()) {
                t = format!("{} pm", t.replace(pm.as_str(), "").trim());
            } else if !am.is_empty() && t.contains(am.as_str()) {
                t = format!("{} am", t.replace(am.as_str(), "").trim());
            }
        }
        crate::services_i18n::calendar::Time::parse(&t)
    }

    /// The short name of a weekday.
    pub fn weekday(&self, d: Weekday) -> &str {
        self.weekdays.get(d.index_from_monday()).map_or("", String::as_str)
    }
}

/// How [`NumberSymbols::compact_with`] shortens a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactOptions {
    /// Counts below this (ignoring the sign) are written whole and grouped, `5,432`; `None`,
    /// the default, shortens from the locale's first unit (a thousand in en, ten thousand in
    /// ja, a million in de).
    pub threshold: Option<Decimal>,
    /// Significant digits kept, whole digits never dropped: 2, the default, gives `1.2K`,
    /// `12K`, `123K`; 3 gives `1.23K`, `12.3K`, `123K`; 1 gives `1K`.
    pub significant_digits: u8,
    /// At most this many digits after the point, whatever the size, as `Intl.NumberFormat`'s
    /// `maximumFractionDigits` does with compact notation: 1 gives `1.2K`, `12.3K`, `123.5K`.
    /// `None`, the default, follows [`CompactOptions::significant_digits`].
    pub max_fraction_digits: Option<u8>,
}

impl Default for CompactOptions {
    fn default() -> Self {
        CompactOptions { threshold: None, significant_digits: 2, max_fraction_digits: None }
    }
}

impl CompactOptions {
    /// Sets [`CompactOptions::threshold`].
    #[must_use]
    pub fn threshold(mut self, at: Decimal) -> Self {
        self.threshold = Some(at);
        self
    }

    /// Sets [`CompactOptions::significant_digits`].
    #[must_use]
    pub fn significant_digits(mut self, n: u8) -> Self {
        self.significant_digits = n;
        self
    }

    /// Sets [`CompactOptions::max_fraction_digits`].
    #[must_use]
    pub fn max_fraction_digits(mut self, n: u8) -> Self {
        self.max_fraction_digits = Some(n);
        self
    }
}

/// A language's name in `locale` (the window's language): `language_name_in("de", "fr")` is
/// "Französisch", `language_name_in("en", "pt-BR")` "Brazilian Portuguese",
/// `language_name_in("en", "ger")` "German". Codes are BCP 47 or ISO 639-2 (`ger`, `fre`,
/// `chi`, as media files tag tracks). Names begin with a capital, as a menu or a list shows
/// them. The `language-<code>` messages come first: the engine's in `i18n/*.ftl`, and any an
/// app adds with [`remedy_ui_engine::view::strings::add_messages`] under `services-i18n`. With the
/// `language-names` feature (on in `remedy-ui`), every language CLDR names is named in every
/// locale CLDR has (in a browser page, by the browser's `Intl.DisplayNames`, from its own
/// CLDR data); without it, only the engine's shipped languages, `pt-BR` and `zh-Hans`.
/// A region or script with no name of its own gets the language's; a language with no name
/// comes back as given.
pub fn language_name_in(locale: &str, code: &str) -> String {
    let tag = language_code(code);
    let base = tag.split('-').next().unwrap_or_default().to_owned();
    let source = crate::services_i18n::STRINGS.locales().first().map_or("en", |l| *l);
    let message = |in_locale: &str, t: &str| {
        let id = format!("language-{t}");
        let name = crate::services_i18n::STRINGS.format_in(in_locale, &id, &[]);
        (name != format!("\u{27E8}{id}\u{27E9}")).then_some(name)
    };
    // A message the window's language has, not the source language's reached by fallback.
    let lang = |t: &str| t.split(['-', '_']).next().map(str::to_ascii_lowercase);
    let own = |t: &str| {
        message(locale, t).filter(|n| lang(locale) == lang(source) || Some(n) != message(source, t).as_ref())
    };
    for t in [&tag, &base] {
        if let Some(n) = own(t).or_else(|| cldr_name(locale, t)) {
            return n;
        }
    }
    message(locale, &tag).or_else(|| message(locale, &base)).unwrap_or_else(|| code.to_owned())
}

/// `code` as a BCP 47 tag, ISO 639-2 and other aliases replaced (`ger` and `deu` are `de`).
#[cfg(all(feature = "services-i18n-language-names", not(target_arch = "wasm32")))]
fn language_code(code: &str) -> String {
    let tag = crate::services_i18n::canonical_locale(code);
    let Ok(mut l) = tag.parse::<Locale>() else { return tag };
    icu_locale::LocaleCanonicalizer::new_common().canonicalize(&mut l);
    l.to_string()
}

/// `code` as a BCP 47 tag, the browser's CLDR aliases replaced (`Intl.getCanonicalLocales`:
/// `ger` and `deu` are `de`), then the ISO 639-2 codes of the shipped languages for any it
/// left.
#[cfg(all(feature = "services-i18n-language-names", target_arch = "wasm32"))]
fn language_code(code: &str) -> String {
    use js_sys::{Array, Function, Reflect};
    use wasm_bindgen::JsCast as _;
    let tag = crate::services_i18n::canonical_locale(code);
    // Through `Reflect`, so a tag the browser rejects (a `RangeError`) is an error, not a trap.
    let canonical = Reflect::get(&js_sys::global(), &"Intl".into())
        .and_then(|intl| {
            let f: Function = Reflect::get(&intl, &"getCanonicalLocales".into())?.dyn_into()?;
            f.call1(&intl, &tag.as_str().into())
        })
        .ok()
        .and_then(|list| list.dyn_into::<Array>().ok())
        .and_then(|list| list.get(0).as_string());
    shipped_code(&canonical.unwrap_or(tag))
}

/// `code` as a BCP 47 tag, the ISO 639-2 codes of the shipped languages replaced.
#[cfg(not(feature = "services-i18n-language-names"))]
fn language_code(code: &str) -> String {
    shipped_code(&crate::services_i18n::canonical_locale(code))
}

/// `tag` with the ISO 639-2 codes of the shipped languages replaced.
#[cfg(any(not(feature = "services-i18n-language-names"), target_arch = "wasm32"))]
fn shipped_code(tag: &str) -> String {
    let tag = tag.to_owned();
    let (lang, rest) = tag.split_once('-').unwrap_or((tag.as_str(), ""));
    let short = match lang.to_ascii_lowercase().as_str() {
        "ara" => "ar",
        "ben" => "bn",
        "ger" | "deu" => "de",
        "eng" => "en",
        "spa" => "es",
        "fre" | "fra" => "fr",
        "hin" => "hi",
        "jpn" => "ja",
        "por" => "pt",
        "rus" => "ru",
        "chi" | "zho" => "zh",
        _ => return tag,
    };
    if rest.is_empty() { short.to_owned() } else { format!("{short}-{rest}") }
}

/// CLDR's name for `tag` in `locale`, capitalized; `None` where CLDR does not name all of it.
#[cfg(all(feature = "services-i18n-language-names", not(target_arch = "wasm32")))]
fn cldr_name(locale: &str, tag: &str) -> Option<String> {
    use icu_locale::names::{
        DisplayNamesPreferences, LanguageIdentifierDisplayName, LanguageIdentifierDisplayNameOptions,
    };
    use writeable::TryWriteable;
    let id = tag.parse::<icu_locale_core::LanguageIdentifier>().ok()?;
    let prefs = DisplayNamesPreferences::from(&self::locale(locale));
    let name = LanguageIdentifierDisplayName::try_new_heavy(
        prefs,
        id,
        LanguageIdentifierDisplayNameOptions::default(),
    )
    .ok()?;
    let name = name.as_borrowed().try_write_to_string().ok()?.into_owned();
    let mut chars = name.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// The browser's CLDR name for `tag` in `locale` (`Intl.DisplayNames`, with dialect names as
/// ICU4X gives them, and `fallback: "none"` so an unnamed tag is `None`), capitalized.
#[cfg(all(feature = "services-i18n-language-names", target_arch = "wasm32"))]
fn cldr_name(locale: &str, tag: &str) -> Option<String> {
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsCast as _;
    let intl = Reflect::get(&js_sys::global(), &"Intl".into()).ok()?;
    let ctor: Function = Reflect::get(&intl, &"DisplayNames".into()).ok()?.dyn_into().ok()?;
    let options = Object::new();
    for (k, v) in [("type", "language"), ("fallback", "none")] {
        Reflect::set(&options, &k.into(), &v.into()).ok()?;
    }
    let names = Reflect::construct(&ctor, &Array::of2(&locale.into(), &options)).ok()?;
    let of: Function = Reflect::get(&names, &"of".into()).ok()?.dyn_into().ok()?;
    let name = of.call1(&names, &tag.into()).ok()?.as_string()?;
    let mut chars = name.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

#[cfg(not(feature = "services-i18n-language-names"))]
fn cldr_name(_: &str, _: &str) -> Option<String> {
    None
}

/// [`language_name_in`] the window's language (tracked: it follows the locale).
pub fn language_name(ui: &remedy_ui_engine::view::Ui, code: &str) -> String {
    language_name_in(&ui.locale(), code)
}
