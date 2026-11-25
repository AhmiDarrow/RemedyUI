//! Medium dates, dates with times, compact counts and language names in every language the
//! engine ships, as CLDR (through ICU4X's compiled data) writes them.

use remedy_ui_platform::services_i18n::{
    CalendarSymbols, CompactOptions, Date, DateTimeStyle, Decimal, NumberSymbols, Time, language_name_in,
};

const LOCALES: [&str; 11] = ["en", "de", "fr", "es", "pt-BR", "ru", "ja", "zh-Hans", "ar", "hi", "bn"];

const DATE: Date = Date { year: 2026, month: 11, day: 21 };

fn time() -> Time {
    Time::new(14, 30).unwrap_or_else(|| unreachable!("a valid time"))
}

fn medium(l: &str) -> String {
    CalendarSymbols::for_locale(l).medium_date(DATE)
}

fn date_time(l: &str) -> String {
    CalendarSymbols::for_locale(l).date_time(DATE, time())
}

fn compact(l: &str, n: i64) -> String {
    NumberSymbols::for_locale(l).compact(Decimal::from_int(n))
}

#[test]
fn medium_dates_in_every_language() {
    let want = [
        ("en", "Nov 21, 2026"),
        ("de", "21.11.2026"),
        ("fr", "21 nov. 2026"),
        ("es", "21 nov 2026"),
        ("pt-BR", "21 de nov. de 2026"),
        ("ru", "21 нояб. 2026\u{202F}г."),
        ("ja", "2026/11/21"),
        ("zh-Hans", "2026年11月21日"),
        ("ar", "21\u{200F}/11\u{200F}/2026"),
        ("hi", "21 नव॰ 2026"),
        ("bn", "২১ নভে, ২০২৬"),
    ];
    assert_eq!(want.map(|(l, _)| l), LOCALES);
    for (l, w) in want {
        assert_eq!(medium(l), w, "{l}");
    }
    assert_eq!(medium("en-GB"), "21 Nov 2026");
}

#[test]
fn dates_with_times_use_each_locales_clock() {
    let want = [
        ("en", "Nov 21, 2026, 2:30\u{202F}PM"),
        ("de", "21.11.2026, 14:30"),
        ("fr", "21 nov. 2026, 14:30"),
        ("es", "21 nov 2026, 14:30"),
        ("pt-BR", "21 de nov. de 2026, 14:30"),
        ("ru", "21 нояб. 2026\u{202F}г., 14:30"),
        ("ja", "2026/11/21 14:30"),
        ("zh-Hans", "2026年11月21日 14:30"),
        ("ar", "21\u{200F}/11\u{200F}/2026، 2:30 م"),
        ("hi", "21 नव॰ 2026, 2:30 pm"),
        ("bn", "২১ নভে, ২০২৬, ২:৩০ PM"),
    ];
    assert_eq!(want.map(|(l, _)| l), LOCALES);
    for (l, w) in want {
        assert_eq!(date_time(l), w, "{l}");
        // The time part is the one `format_time` writes, so the clock matches it.
        assert!(w.ends_with(&CalendarSymbols::for_locale(l).format_time(time())), "{l}");
    }
    assert_eq!(date_time("en-GB"), "21 Nov 2026, 14:30", "the same language, a 24-hour region");
    // Midnight and noon on a 12-hour clock.
    let cal = CalendarSymbols::for_locale("en-US");
    let at = |h| cal.date_time(DATE, Time::new(h, 5).unwrap_or_else(|| unreachable!("a valid time")));
    assert_eq!(at(0), "Nov 21, 2026, 12:05\u{202F}AM");
    assert_eq!(at(12), "Nov 21, 2026, 12:05\u{202F}PM");
}

#[test]
fn numeric_dates_with_seconds_match_the_browsers_to_locale_string() {
    // `new Date(Date.UTC(2026, 10, 21, 13, 32, 31)).toLocaleString(l, { timeZone: "UTC" })`.
    let want = [
        ("en", "11/21/2026, 1:32:31 PM"),
        ("de", "21.11.2026, 13:32:31"),
        ("fr", "21/11/2026 13:32:31"),
        ("es", "21/11/2026, 13:32:31"),
        ("pt-BR", "21/11/2026, 13:32:31"),
        ("ru", "21.11.2026, 13:32:31"),
        ("ja", "2026/11/21 13:32:31"),
        ("zh-Hans", "2026/11/21 13:32:31"),
        ("ar", "21\u{200F}/11\u{200F}/2026\u{060C} 1:32:31 \u{0645}"),
        ("hi", "21/11/2026, 1:32:31 pm"),
        ("bn", "২১/১১/২০২৬, ১:৩২:৩১ PM"),
    ];
    assert_eq!(want.map(|(l, _)| l), LOCALES);
    let at = DATE.days_since_epoch() * 86_400 + 13 * 3600 + 32 * 60 + 31;
    let t = Time::new(13, 32).unwrap_or_else(|| unreachable!("a valid time"));
    for (l, w) in want {
        let cal = CalendarSymbols::for_locale(l);
        assert_eq!(cal.format_instant(at, DateTimeStyle::Numeric), w, "{l}");
        assert_eq!(cal.format_date_time(DATE, t, 31, DateTimeStyle::Numeric), w, "{l}");
    }
    let numeric = |l| CalendarSymbols::for_locale(l).format_instant(at, DateTimeStyle::Numeric);
    assert_eq!(numeric("en-US"), "11/21/2026, 1:32:31 PM");
    assert_eq!(numeric("en-GB"), "21/11/2026, 13:32:31");
    assert_eq!(numeric("ko"), "2026. 11. 21. 오후 1:32:31");
    // Before 1970 and at midnight.
    assert_eq!(
        CalendarSymbols::for_locale("en-US").format_instant(-1, DateTimeStyle::Numeric),
        "12/31/1969, 11:59:59 PM"
    );
    // The medium style is the minute-precise `date_time`.
    let cal = CalendarSymbols::for_locale("en");
    assert_eq!(cal.format_date_time(DATE, time(), 9, DateTimeStyle::Medium), date_time("en"));
}

#[test]
fn compact_counts_in_every_language() {
    // 999, 1234, 3 400 000 and -15 000.
    let want: [(&str, [&str; 4]); 11] = [
        ("en", ["999", "1.2K", "3.4M", "-15K"]),
        // German shortens from a million.
        ("de", ["999", "1234", "3,4\u{A0}Mio.", "-15.000"]),
        ("fr", ["999", "1,2\u{A0}k", "3,4\u{A0}M", "-15\u{A0}k"]),
        ("es", ["999", "1,2\u{A0}mil", "3,4\u{A0}M", "-15\u{A0}mil"]),
        ("pt-BR", ["999", "1,2\u{A0}mil", "3,4\u{A0}mi", "-15\u{A0}mil"]),
        ("ru", ["999", "1,2\u{A0}тыс.", "3,4\u{A0}млн", "-15\u{A0}тыс."]),
        // Ten thousands (万) in Japanese and Chinese.
        ("ja", ["999", "1234", "340万", "-1.5万"]),
        ("zh-Hans", ["999", "1234", "340万", "-1.5万"]),
        ("ar", ["999", "1.2\u{A0}ألف", "3.4\u{A0}مليون", "\u{200E}-15\u{A0}ألف"]),
        // Lakhs in Hindi and Bengali.
        ("hi", ["999", "1.2\u{A0}हज\u{93C}ार", "34\u{A0}लाख", "-15\u{A0}हज\u{93C}ार"]),
        ("bn", ["৯৯৯", "১.২\u{A0}হা", "৩৪\u{A0}লা", "-১৫\u{A0}হা"]),
    ];
    assert_eq!(want.map(|(l, _)| l), LOCALES);
    for (l, w) in want {
        let got = [999, 1234, 3_400_000, -15_000].map(|n| compact(l, n));
        assert_eq!(got, w, "{l}");
    }
    assert_eq!(compact("en", 0), "0");
    assert_eq!(compact("en", 999_500), "1M", "rounds up into the next unit");
    assert_eq!(compact("en", 15_127), "15K", "no fraction digit from ten up");
    assert_eq!(compact("en", 2_500_000_000), "2.5B");
    // Fractions round to the same digits.
    let half = Decimal::parse("1250.5").unwrap_or_else(|_| unreachable!("a number"));
    assert_eq!(NumberSymbols::for_locale("en").compact(half), "1.3K");
    // An unknown locale gets the root's.
    assert_eq!(compact("xx", 1234), "1.2K");
}

#[test]
fn compact_counts_take_options() {
    let n = |v| Decimal::from_int(v);
    let with = |l, v, o| NumberSymbols::for_locale(l).compact_with(n(v), o);
    let three = CompactOptions::default().significant_digits(3);
    assert_eq!(with("en", 1234, three), "1.23K");
    assert_eq!(with("en", 12_345, three), "12.3K");
    assert_eq!(with("en", 123_456, three), "123K");
    assert_eq!(with("de", 3_456_789, three), "3,46\u{A0}Mio.");
    assert_eq!(with("fr", -1234, three), "-1,23\u{A0}k");
    assert_eq!(with("ja", 123_456, three), "12.3万");
    assert_eq!(with("ar", 1234, three), "1.23\u{A0}ألف");
    assert_eq!(with("en", 1234, CompactOptions::default().significant_digits(1)), "1K");
    assert_eq!(with("en", 999_999, three), "1M", "rounds up into the next unit");
    // At most one digit after the point, as `Intl.NumberFormat` with
    // `maximumFractionDigits: 1` writes compact counts.
    let one = CompactOptions::default().max_fraction_digits(1);
    assert_eq!(with("en", 1234, one), "1.2K");
    assert_eq!(with("en", 12_345, one), "12.3K");
    assert_eq!(with("en", 123_456, one), "123.5K");
    assert_eq!(with("en", 120_000, one), "120K");
    assert_eq!(with("en", 999_960, one), "1M", "rounds up into the next unit");
    assert_eq!(with("en", 1_234_567, one), "1.2M");
    // A threshold keeps smaller counts whole.
    let ten_k = CompactOptions::default().threshold(n(10_000));
    assert_eq!(with("en", 5432, ten_k), "5,432");
    assert_eq!(with("en", -5432, ten_k), "-5,432");
    assert_eq!(with("en", 10_000, ten_k), "10K");
    assert_eq!(with("de", 5432, ten_k), "5.432");
    // The defaults are `compact`'s.
    assert_eq!(with("en", 1234, CompactOptions::default()), compact("en", 1234));
}

#[cfg(feature = "services-i18n-language-names")]
#[test]
fn cldr_language_names() {
    // Every CLDR language, in the window's language; ISO 639-2 codes as media files tag tracks.
    let codes = ["ko", "it", "sv", "ger", "pt-BR"];
    let want: [(&str, [&str; 5]); 5] = [
        ("en", ["Korean", "Italian", "Swedish", "German", "Brazilian Portuguese"]),
        ("de", ["Koreanisch", "Italienisch", "Schwedisch", "Deutsch", "Brasilianisches Portugiesisch"]),
        ("fr", ["Coréen", "Italien", "Suédois", "Allemand", "Portugais brésilien"]),
        ("ja", ["韓国語", "イタリア語", "スウェーデン語", "ドイツ語", "ポルトガル語 (ブラジル)"]),
        ("ar", ["الكورية", "الإيطالية", "السويدية", "الألمانية", "البرتغالية البرازيلية"]),
    ];
    for (l, w) in want {
        assert_eq!(codes.map(|c| language_name_in(l, c)), w, "{l}");
    }
    assert_eq!(language_name_in("en", "fre"), "French");
    assert_eq!(language_name_in("en", "chi"), "Chinese");
    assert_eq!(language_name_in("en", "jpn"), "Japanese");
    assert_eq!(language_name_in("en", "pt-PT"), "European Portuguese");
    assert_eq!(language_name_in("en", "zh-Hant"), "Traditional Chinese");
    assert_eq!(language_name_in("en", "tlh"), "Klingon");
    // A window language the engine does not ship.
    assert_eq!(language_name_in("ko", "de"), "독일어");
    // A region with no dialect name of its own is written after the language.
    assert_eq!(language_name_in("en", "es-AR"), "Spanish (Argentina)");
    assert_eq!(language_name_in("fr", "en_ZA"), "Anglais (Afrique du Sud)");
}

#[test]
fn language_names_in_every_language() {
    // Each language's name for itself, then for English and Japanese.
    let want: [(&str, [&str; 3]); 11] = [
        ("en", ["English", "English", "Japanese"]),
        ("de", ["Deutsch", "Englisch", "Japanisch"]),
        ("fr", ["Français", "Anglais", "Japonais"]),
        ("es", ["Español", "Inglés", "Japonés"]),
        ("pt-BR", ["Português (Brasil)", "Inglês", "Japonês"]),
        ("ru", ["Русский", "Английский", "Японский"]),
        ("ja", ["日本語", "英語", "日本語"]),
        ("zh-Hans", ["简体中文", "英语", "日语"]),
        ("ar", ["العربية", "الإنجليزية", "اليابانية"]),
        ("hi", ["हिन्दी", "अंग्रेज\u{93C}ी", "जापानी"]),
        ("bn", ["বাংলা", "ইংরেজি", "জাপানি"]),
    ];
    assert_eq!(want.map(|(l, _)| l), LOCALES);
    for (l, w) in want {
        assert_eq!([l, "en", "ja"].map(|c| language_name_in(l, c)), w, "{l}");
        // Every shipped language has a name in every other, unlike the code itself.
        for c in LOCALES {
            assert_ne!(language_name_in(l, c), c, "{c} in {l}");
        }
    }
    assert_eq!(language_name_in("de", "fr"), "Französisch");
    assert_eq!(language_name_in("en", "pt-BR"), "Brazilian Portuguese");
    assert_eq!(language_name_in("en", "pt"), "Portuguese");
    // Regions and scripts with no name of their own get the language's.
    #[cfg(not(feature = "services-i18n-language-names"))]
    {
        assert_eq!(language_name_in("en", "pt-PT"), "Portuguese");
        assert_eq!(language_name_in("fr", "en_ZA"), "Anglais");
        assert_eq!(language_name_in("en", "zh-Hant"), "Chinese");
        assert_eq!(language_name_in("en", "ger"), "German");
        assert_eq!(language_name_in("fr", "chi"), "Chinois");
    }
    // A regional window language reads its language's names.
    assert_eq!(language_name_in("de-AT", "es"), "Spanisch");
    assert_eq!(language_name_in("fr-CA", "zh-Hans"), "Chinois simplifié");
    // Unknown codes come back as given; an unknown window language reads English.
    assert_eq!(language_name_in("en", "qaa"), "qaa");
    assert_eq!(language_name_in("xx", "de"), "German");
}

#[test]
fn apps_add_language_names() {
    remedy_ui_engine::view::strings::add_messages("services-i18n", "en", "language-ko = Korean\n");
    remedy_ui_engine::view::strings::add_messages("services-i18n", "de", "language-ko = Koreanisch\n");
    assert_eq!(language_name_in("en", "ko"), "Korean");
    // App names win over CLDR's.
    remedy_ui_engine::view::strings::add_messages("services-i18n", "en", "language-sv = Svenska");
    assert_eq!(language_name_in("en", "sv"), "Svenska");
    if cfg!(feature = "services-i18n-language-names") {
        assert_eq!(language_name_in("de", "ko-KR"), "Koreanisch (Südkorea)");
        assert_eq!(language_name_in("fr", "ko"), "Coréen", "no French message: CLDR's name");
    } else {
        assert_eq!(language_name_in("de", "ko-KR"), "Koreanisch");
        assert_eq!(language_name_in("fr", "ko"), "Korean", "no French name: the source language's");
    }
}
