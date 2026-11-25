//! Calendar dates, times of day and weekdays, with the arithmetic forms and messages need.

use std::fmt;

use remedy_ui_engine::view::Ui;

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: a week has seven days.
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

    /// Today on the user's local calendar (the host's wall clock and time zone, tracked:
    /// what reads it follows the date across midnight).
    pub fn today(ui: &Ui) -> Date {
        Date::from_days_since_epoch(ui.local_time().local_days())
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
