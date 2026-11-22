//! Every script of the top languages draws with the bundled faces alone (no system fonts):
//! no glyph comes out as the missing-glyph box, Han text takes its language's forms, and
//! each script lands in the face made for it.

use std::time::Instant;

use remedy_ui_text::{FontCollection, FontFile, ParagraphOptions, RichText, TextEngine, TextStyle};
use skrifa::raw::TableProvider;

/// The bundled faces and their script fallbacks, as an app's windows get them.
fn fonts() -> FontCollection {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all().into_iter().chain(remedy_ui_fonts_cjk::all()) {
        assert!(f.add_static(face.data).is_ok(), "{} {}", face.family, face.weight);
    }
    for fb in remedy_ui_fonts::script_fallbacks().into_iter().chain(remedy_ui_fonts_cjk::script_fallbacks()) {
        assert!(f.add_script_fallback(fb.script, fb.locale, fb.family), "{fb:?}");
    }
    f
}

fn family(bytes: &[u8]) -> String {
    let Ok(font) = skrifa::FontRef::new(bytes) else { return String::new() };
    let Ok(name) = font.name() else { return String::new() };
    name.name_record()
        .iter()
        .find(|r| r.name_id().to_u16() == 1)
        .and_then(|r| r.string(name.string_data()).ok())
        .map(|s| s.chars().collect())
        .unwrap_or_default()
}

/// Each run's family, and the text of any glyph that came out as the missing-glyph box.
fn shape(e: &mut TextEngine, locale: &str, text: &str, weight: f32) -> (Vec<(String, String)>, Vec<String>) {
    e.set_locale(locale);
    let mut style = TextStyle::default();
    style.weight = weight;
    let p = e.layout(&RichText::plain(text), &style, ParagraphOptions::default());
    let mut runs = Vec::new();
    let mut tofu = Vec::new();
    for r in p.glyph_runs() {
        let piece = text.get(r.range.clone()).unwrap_or_default().to_owned();
        if r.glyphs.iter().any(|g| g.id == 0) {
            tofu.push(piece.clone());
        }
        runs.push((family(r.font.bytes()), piece));
    }
    (runs, tofu)
}

/// A line of interface text in each of the top languages, with digits, punctuation and the
/// script's hardest shaping (Arabic joining, Devanagari and Bengali conjuncts, kana and
/// kanji, Cyrillic, accents).
const LINES: &[(&str, &str, &str)] = &[
    ("en", "Save 3 files? You can undo this.", "Remedy Sans UI"),
    ("es", "¿Guardar 3 archivos? Año, señal, acción.", "Remedy Sans UI"),
    ("fr", "Enregistrer « 3 fichiers » ? Œuvre, garçon, déjà.", "Remedy Sans UI"),
    ("pt-BR", "Salvar 3 arquivos? Não, ação, você.", "Remedy Sans UI"),
    ("de", "3 Dateien speichern? Größe, Übersicht, „Straße“.", "Remedy Sans UI"),
    ("ru", "Сохранить 3 файла? Отменить — ещё «можно».", "Remedy Sans UI"),
    ("ar", "حفظ ٣ ملفات؟ يمكنك التراجع عن ذلك، لاحقًا.", "Noto Sans Arabic"),
    ("hi", "३ फ़ाइलें सहेजें? आप इसे पूर्ववत कर सकते हैं। क्षत्रिय", "Noto Sans Devanagari"),
    ("bn", "৩টি ফাইল সংরক্ষণ করবেন? আপনি এটি পূর্বাবস্থায় ফেরাতে পারেন। ক্ষ", "Noto Sans Bengali"),
    ("zh-Hans", "保存 3 个文件？您可以撤消此操作。", "Noto Sans SC"),
    ("ja", "3 個のファイルを保存しますか？元に戻せます。", "Noto Sans JP"),
];

#[test]
fn every_top_language_draws_without_missing_glyphs() {
    let mut e = TextEngine::new(fonts());
    for weight in [400.0, 700.0] {
        for (locale, text, face) in LINES {
            let (runs, tofu) = shape(&mut e, locale, text, weight);
            assert!(tofu.is_empty(), "{locale} at {weight}: missing glyphs for {tofu:?} in {runs:?}");
            // The script's letters come from the face made for them.
            let letters: Vec<&(String, String)> =
                runs.iter().filter(|(_, t)| t.chars().any(char::is_alphabetic)).collect();
            assert!(letters.iter().any(|(f, _)| f == face), "{locale}: expected {face} among {runs:?}");
        }
    }
}

#[test]
fn japanese_and_chinese_take_their_own_forms_of_shared_characters() {
    // 直 and 骨 are the same code points in both languages, drawn differently in each.
    let mut e = TextEngine::new(fonts());
    let (ja, _) = shape(&mut e, "ja", "直骨", 400.0);
    let (zh, _) = shape(&mut e, "zh-Hans", "直骨", 400.0);
    let (none, _) = shape(&mut e, "", "直骨", 400.0);
    assert!(ja.iter().all(|(f, _)| f == "Noto Sans JP"), "{ja:?}");
    assert!(zh.iter().all(|(f, _)| f == "Noto Sans SC"), "{zh:?}");
    assert!(none.iter().all(|(f, _)| f == "Noto Sans SC"), "no language: Unicode's default, {none:?}");
    // Kana belong to Japanese whatever the window's language.
    let (kana, tofu) = shape(&mut e, "en", "ひらがな カタカナ", 400.0);
    assert!(tofu.is_empty());
    assert!(kana.iter().filter(|(_, t)| !t.trim().is_empty()).all(|(f, _)| f == "Noto Sans JP"), "{kana:?}");
}

#[test]
fn mixed_scripts_in_one_line_each_find_their_face() {
    let mut e = TextEngine::new(fonts());
    let (runs, tofu) = shape(&mut e, "en", "Hello مرحبا नमस्ते নমস্কার 你好 こんにちは Привет", 400.0);
    assert!(tofu.is_empty(), "{runs:?}");
    for face in
        ["Remedy Sans UI", "Noto Sans Arabic", "Noto Sans Devanagari", "Noto Sans Bengali", "Noto Sans JP"]
    {
        assert!(runs.iter().any(|(f, _)| f == face), "{face} in {runs:?}");
    }
}

#[test]
fn chinese_and_japanese_faces_load_when_text_first_needs_them() {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let l2 = loads.clone();
    let cjk = remedy_ui_fonts_cjk::script_fallbacks();
    f.add_on_demand(
        remedy_ui_fonts_cjk::needs,
        move || {
            l2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            remedy_ui_fonts_cjk::all().into_iter().filter_map(|f| FontFile::new(f.data).ok()).collect()
        },
        cjk.iter().map(|fb| (fb.script, fb.locale, fb.family)),
    );
    let mut e = TextEngine::new(f);
    let _ = shape(&mut e, "en", "Nothing here needs them: مرحبا", 400.0);
    assert_eq!(loads.load(std::sync::atomic::Ordering::Relaxed), 0, "no Chinese or Japanese yet");
    let (ja, tofu) = shape(&mut e, "ja", "直す", 400.0);
    assert!(tofu.is_empty() && ja.iter().all(|(f, _)| f == "Noto Sans JP"), "{ja:?}");
    let (zh, _) = shape(&mut e, "zh-Hans", "直", 700.0);
    assert!(zh.iter().all(|(f, _)| f == "Noto Sans SC"), "{zh:?}");
    assert_eq!(loads.load(std::sync::atomic::Ordering::Relaxed), 1, "loaded once");
    assert!(!e.fonts().has_on_demand());
}

#[test]
fn the_cjk_faces_unpack_quickly_enough_to_read_once_at_startup() {
    // WOFF2 is unpacked once per process (FontFile); it must not hold up the first window.
    let started = Instant::now();
    for face in remedy_ui_fonts_cjk::all() {
        let file = FontFile::new(face.data);
        assert!(file.is_ok_and(|f| f.families() == [face.family]), "{}", face.family);
    }
    let took = started.elapsed();
    eprintln!("unpacking the CJK faces took {took:?}");
    // Generous for debug builds on slow CI machines; release builds take a fraction.
    assert!(took.as_secs_f32() < 30.0, "{took:?}");
}

/// With the system's fonts reachable too, the bundled faces still draw every script and the
/// punctuation between them (a Japanese bracket beside an Arabic name), so what an app was
/// checked with is what people see, whatever is installed.
#[cfg(feature = "system-fonts")]
#[test]
fn bundled_faces_come_before_the_systems() {
    let mut f = FontCollection::with_system_fonts();
    for face in remedy_ui_fonts::all().into_iter().chain(remedy_ui_fonts_cjk::all()) {
        let _ = f.add_static(face.data);
    }
    for fb in remedy_ui_fonts::script_fallbacks().into_iter().chain(remedy_ui_fonts_cjk::script_fallbacks()) {
        assert!(f.add_script_fallback(fb.script, fb.locale, fb.family), "{fb:?}");
    }
    let bundled: Vec<&str> =
        remedy_ui_fonts::all().into_iter().chain(remedy_ui_fonts_cjk::all()).map(|f| f.family).collect();
    let mut e = TextEngine::new(f);
    for (locale, text, _) in
        LINES.iter().chain([&("ja", "「مريم」に発信（3）", ""), &("en", "Hello مرحبا नमस्ते নমস্কার 你好", "")])
    {
        let (runs, tofu) = shape(&mut e, locale, text, 400.0);
        assert!(tofu.is_empty(), "{locale}: {runs:?}");
        for (family, piece) in &runs {
            assert!(bundled.contains(&family.as_str()), "{locale}: {piece:?} drawn in the system's {family}");
        }
    }
}

#[test]
fn faces_still_on_their_way_are_asked_for_again_until_they_arrive() {
    // A browser page fetches its fallback faces: the first texts that need them find none.
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    let arrived = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (a2, n2) = (arrived.clone(), asks.clone());
    let cjk = remedy_ui_fonts_cjk::script_fallbacks();
    f.add_on_demand(
        remedy_ui_fonts_cjk::needs,
        move || {
            n2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if !a2.load(std::sync::atomic::Ordering::Relaxed) {
                return Vec::new();
            }
            remedy_ui_fonts_cjk::all().into_iter().filter_map(|f| FontFile::new(f.data).ok()).collect()
        },
        cjk.iter().map(|fb| (fb.script, fb.locale, fb.family)),
    );
    let mut e = TextEngine::new(f);
    let (_, tofu) = shape(&mut e, "ja", "直す", 400.0);
    assert!(!tofu.is_empty(), "nothing to draw it with yet");
    assert!(e.fonts().has_on_demand(), "still waiting");
    arrived.store(true, std::sync::atomic::Ordering::Relaxed);
    let (ja, tofu) = shape(&mut e, "ja", "直す", 400.0);
    assert!(tofu.is_empty() && ja.iter().all(|(f, _)| f == "Noto Sans JP"), "{ja:?}");
    assert!(!e.fonts().has_on_demand());
    assert_eq!(asks.load(std::sync::atomic::Ordering::Relaxed), 2);
}

#[test]
fn a_generic_family_draws_in_the_faces_it_is_set_to() {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    assert!(f.set_generic_family("system-ui", &["AhmiPrint"]));
    assert!(!f.set_generic_family("system-ui", &["No Such Face"]), "every family must be there");
    assert!(!f.set_generic_family("Remedy Sans UI", &["AhmiPrint"]), "not a generic name");
    assert_eq!(f.generic_family("system-ui"), ["AhmiPrint"]);
    let mut e = TextEngine::new(f);
    e.set_locale("en");
    let mut style = TextStyle::default();
    style.families = vec!["system-ui".into(), "Remedy Sans UI".into()];
    let p = e.layout(&RichText::plain("System"), &style, ParagraphOptions::default());
    let fams: Vec<String> = p.glyph_runs().iter().map(|r| family(r.font.bytes())).collect();
    assert!(!fams.is_empty() && fams.iter().all(|f| f == "AhmiPrint"), "{fams:?}");
}
