//! Golden images of the same screen in each of the top languages, drawn by the CPU renderer
//! with the bundled fonts only: every script in its own face (no empty boxes), right-to-left
//! layouts mirrored, mixed-direction lines in the right order, and the engine's own words,
//! dates and numbers in each language. `tests/golden/languages-*.png`; rewrite them with
//! `REMEDY_UPDATE_GOLDEN=1` after a deliberate change, and look at every one.

use remedy_ui_core::Insets;
use remedy_ui_devtools::{GoldenError, Preview, check_golden};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_forms::{CalendarSymbols, Date, Decimal, NumberSymbols};

/// The top languages, as a person in each would have their system set.
const LOCALES: &[&str] =
    &["en-US", "es-ES", "zh-CN", "hi-IN", "ar-EG", "fr-FR", "pt-BR", "ru-RU", "ja-JP", "de-DE", "bn-BD"];

/// An app's own words for the sample screen, per language: a title, a sentence, and a line
/// that mixes directions: a name in another script, isolated (U+2068 … U+2069) as the
/// localization services insert every placeholder, so it neither takes the number and
/// punctuation after it into its own direction nor sets the line's direction.
fn words(locale: &str) -> (&'static str, &'static str, &'static str) {
    match locale.split('-').next().unwrap_or("en") {
        "es" => ("Ajustes de la llamada", "Las notificaciones llegan aunque la app esté cerrada.", "Llamar a «\u{2068}مريم\u{2069}» (3)"),
        "zh" => ("通话设置", "即使应用已关闭，您也会收到通知。", "呼叫 “\u{2068}مريم\u{2069}”（3）"),
        "hi" => ("कॉल सेटिंग", "ऐप बंद होने पर भी सूचनाएँ आती हैं।", "“\u{2068}مريم\u{2069}” को कॉल करें (3)"),
        "ar" => ("إعدادات المكالمة", "تصلك الإشعارات حتى عند إغلاق التطبيق.", "اتصل بـ «\u{2068}Maria\u{2069}» (3)"),
        "fr" => ("Réglages d’appel", "Les notifications arrivent même quand l’app est fermée.", "Appeler « \u{2068}مريم\u{2069} » (3)"),
        "pt" => ("Configurações de chamada", "As notificações chegam mesmo com o app fechado.", "Ligar para “\u{2068}مريم\u{2069}” (3)"),
        "ru" => ("Настройки звонка", "Уведомления приходят, даже если приложение закрыто.", "Позвонить «\u{2068}مريم\u{2069}» (3)"),
        "ja" => ("通話の設定", "アプリを閉じていても通知が届きます。", "「\u{2068}مريم\u{2069}」に発信（3）"),
        "de" => ("Anrufeinstellungen", "Benachrichtigungen kommen auch bei geschlossener App.", "„\u{2068}مريم\u{2069}“ anrufen (3)"),
        "bn" => ("কল সেটিংস", "অ্যাপ বন্ধ থাকলেও বিজ্ঞপ্তি আসে।", "“\u{2068}مريم\u{2069}”-কে কল করুন (3)"),
        _ => ("Call settings", "Notifications arrive even when the app is closed.", "Call “\u{2068}مريم\u{2069}” (3)"),
    }
}

/// A settings screen of the kind every app has: a back button and title, a sentence, a
/// mixed-direction line, a switch, a slider and a progress bar (which mirror), a date and a
/// number in the locale's own forms, and buttons and a stepper with the engine's own words.
fn screen(ui: &Ui) -> El {
    let locale = ui.locale();
    let (title, body, mixed) = words(&locale);
    let unit = ui.theme().spacing.unit;
    let cal = CalendarSymbols::for_locale(&locale);
    let date = Date::new(2026, 3, 5).map(|d| cal.long_date(d)).unwrap_or_default();
    let number = Decimal::from_f64(1_234_567.89, 2).map(|d| NumberSymbols::for_locale(&locale).format(d, 2));
    let header = w::row(ui)
        .gap(unit * 2.0)
        .align(remedy_ui_layout::Align::Center)
        .child(w::icon_button(ui, w::Icon::ArrowLeft, w::Icon::ArrowLeft.localized_name(ui), || {}))
        .child(w::heading(ui, 2, title))
        .child(w::spacer(ui))
        .child(w::icon(ui, w::Icon::ChevronRight, 18.0));
    let on = Signal::new(true);
    let volume = Signal::new(0.7f64);
    let copies = Signal::new(2.0f64);
    let progress = Signal::new(Some(0.35f32));
    let buttons = w::row(ui)
        .gap(unit * 2.0)
        .child(w::button(ui, w::STRINGS.get(ui, "prompt-deny"), || {}))
        .child(w::button(ui, w::STRINGS.get(ui, "prompt-allow"), || {}).variant("primary"));
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 4.0))
        .fill_width()
        .child(header)
        .child(w::text(ui, body))
        .child(w::text(ui, mixed))
        .child(w::switch(ui, w::Icon::Volume.localized_name(ui), on))
        .child(w::slider(ui, w::Icon::Volume.localized_name(ui), volume, 0.0, 1.0, 0.0))
        .child(w::progress(ui, w::STRINGS.get(ui, "loading"), progress))
        .child(w::text(ui, date))
        .child(w::text(ui, number.unwrap_or_default()))
        .child(w::stepper(ui, w::Icon::Copy.localized_name(ui), copies, 1.0, 9.0, 1.0))
        .child(w::row(ui).gap(unit * 2.0).children([
            w::icon(ui, w::Icon::ListNumber, 20.0),
            w::icon(ui, w::Icon::Send, 20.0),
            w::icon(ui, w::Icon::Clock, 20.0),
        ]))
        .child(buttons)
}

/// Paragraphs that mix directions, the hard cases of the bidirectional algorithm: numbers
/// and punctuation at run edges, a left-to-right name inside right-to-left text and the
/// reverse, a URL, and parentheses (which mirror).
fn mixed_lines(ui: &Ui) -> El {
    let lines = [
        "Order #1234 for مريم أحمد ships today.",
        "تم شحن الطلب رقم 1234 إلى Maria Silva اليوم.",
        "Visit https://example.com/مساعدة for help.",
        "النسخة 2.5 (تجريبية) جاهزة: 3 ملفات، 12 MB.",
        "हिन्दी और English और বাংলা और 中文 और 日本語 — one line.",
        "«مرحبا» — „Hallo“ — 「こんにちは」 — “Olá”",
    ];
    let unit = ui.theme().spacing.unit;
    lines.into_iter().fold(w::column(ui).gap(unit * 2.0).padding(Insets::all(unit * 4.0)).fill_width(), |c, l| {
        c.child(w::text(ui, l))
    })
}

fn report(r: Result<(), GoldenError>) {
    match r {
        Ok(()) => {}
        // A new image is written for review: commit it after looking at it.
        Err(GoldenError::Missing(path)) => eprintln!("wrote {} for review", path.display()),
        Err(e) => unreachable!("{e:?}"),
    }
}

fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[test]
fn the_same_screen_in_every_top_language() {
    let p = Preview::new("languages", screen).themes(&["light"]).locales(LOCALES).sizes(&[(360.0, 640.0)]);
    report(check_golden(&p, &golden_dir()));
}

#[test]
fn mixed_direction_lines_in_both_paragraph_directions() {
    let p = Preview::new("languages-bidi", mixed_lines).themes(&["light"]).locales(&["en-US", "ar-EG"]).sizes(&[(420.0, 360.0)]);
    report(check_golden(&p, &golden_dir()));
}

#[test]
fn right_to_left_screens_are_mirror_images_in_layout() {
    // The back button sits at the start: the left in English, the right in Arabic.
    let p = Preview::new("mirror", screen).themes(&["light"]).locales(&["en-US", "ar-EG"]).sizes(&[(360.0, 640.0)]);
    let shots: Vec<_> = p.variations().iter().map(|v| p.render(v)).collect();
    let (Some(en), Some(ar)) = (shots.first(), shots.get(1)) else { unreachable!("two renderings") };
    // The header row: ink near the left edge in English, near the right edge in Arabic.
    let ink = |s: &remedy_ui_devtools::Snapshot, x0: u32, x1: u32| {
        let bg = s.pixel(2, 2).unwrap_or([0; 4]);
        (x0..x1).flat_map(|x| (20..60).map(move |y| (x, y))).filter(|(x, y)| s.pixel(*x, *y).is_some_and(|p| p != bg)).count()
    };
    assert!(ink(en, 16, 40) > ink(en, 320, 344), "English: the back button at the left");
    assert!(ink(ar, 320, 344) > ink(ar, 16, 40), "Arabic: the back button at the right");
}

#[test]
fn short_chinese_and_japanese_labels_stay_on_one_line() {
    // A label sized to its text must not wrap it again at paint time: Chinese and
    // Japanese may break between any two characters, so a rounding difference would.
    for locale in ["ja-JP", "zh-CN"] {
        let p = Preview::new("label", |ui| {
            w::column(ui).child(w::switch(ui, w::Icon::Volume.localized_name(ui), Signal::new(true)))
        })
        .themes(&["light"])
        .locales(&[locale])
        .sizes(&[(200.0, 80.0)]);
        for v in p.variations() {
            let shot = p.render(&v);
            let bg = shot.pixel(199, 79).unwrap_or([0; 4]);
            let rows: Vec<u32> = (0..80)
                .filter(|y| (44..120).any(|x| shot.pixel(x, *y).is_some_and(|q| q != bg && q[0] < 120)))
                .collect();
            let (Some(top), Some(bottom)) = (rows.first(), rows.last()) else { unreachable!("{locale}: no label") };
            assert!(bottom - top < 24, "{locale}: the label is one line ({top}..{bottom})");
        }
    }
}
