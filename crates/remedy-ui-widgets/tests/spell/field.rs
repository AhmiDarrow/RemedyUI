//! Spell checking in a real text field: marks, the word being typed, locale changes, and the
//! context menu.

use std::time::{Duration, Instant};

use remedy_ui_core::input::{Buttons, Key, KeyEvent, PointerEvent, PointerPhase};
use remedy_ui_core::{Point, Size};
use remedy_ui_engine::view::{FieldKind, Request, TextField, Ui};
use remedy_ui_text::FontCollection;
use remedy_ui_widgets::base::MenuItem;
use remedy_ui_widgets::spell::{
    MenuLabels, SpellCheckField, SpellChecker, SpellError, SpellHook, SpellOptions,
};

const EN_AFF: &str = "SET UTF-8\n";
const EN_DIC: &str = "6\nhello\nworld\nhere\nthe\nquick\nbrown\n";
const DE_AFF: &str = "SET UTF-8\n";
const DE_DIC: &str = "3\nFuchs\nHund\nder\n";

fn checker() -> SpellChecker {
    SpellChecker::builder()
        .platform(false)
        .system_dictionaries(false)
        .dictionary("en-US", EN_AFF.into(), EN_DIC.into())
        .dictionary("de", DE_AFF.into(), DE_DIC.into())
        .build()
}

fn options() -> SpellOptions {
    {
        let mut v_ = SpellOptions::default();
        v_.debounce = Duration::from_millis(10);
        v_.settle = Duration::from_secs(2);
        v_
    }
}

struct T {
    ui: Ui,
    timers: Vec<(u64, Duration)>,
}

impl T {
    fn new() -> T {
        let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        let ui = Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts);
        T { ui, timers: Vec::new() }
    }

    fn collect(&mut self) {
        for r in self.ui.take_requests() {
            match r {
                Request::Timer { id, delay } => self.timers.push((id, delay)),
                Request::CancelTimer(id) => self.timers.retain(|t| t.0 != id),
                _ => {}
            }
        }
    }

    fn frame(&mut self) {
        let _ = self.ui.frame(Size::new(400.0, 300.0), 1.0);
        self.collect();
    }

    /// Fires the pending timers `which` picks, as the runner does when they come due.
    fn fire(&mut self, which: impl Fn(Duration) -> bool) {
        self.collect();
        let (due, keep): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.timers).into_iter().partition(|t| which(t.1));
        self.timers = keep;
        for (id, _) in due {
            self.ui.fire_timer(id);
        }
        self.collect();
    }

    /// Runs frames, the short timers and the posts from the checking thread until `done`.
    fn until(&mut self, done: impl Fn() -> bool) {
        let started = Instant::now();
        loop {
            self.frame();
            self.fire(|d| d < Duration::from_secs(1));
            remedy_ui_core::reactive::run_posted();
            self.frame();
            if done() {
                return;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn field(&mut self, text: &str) -> TextField {
        let f = self.ui.text_field("text_input", FieldKind::Multi);
        f.set_value(text);
        self.ui.root().child(f.el().layout(|l| l.width = remedy_ui_core::layout::Length::Px(300.0)));
        self.frame();
        f
    }

    fn click(&mut self, p: Point) {
        self.ui.pointer(&pointer(PointerPhase::Down, p, Buttons::PRIMARY));
        self.ui.pointer(&pointer(PointerPhase::Up, p, Buttons::default()));
        self.frame();
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let s = c.to_string();
            self.ui.key(&KeyEvent::press(Key::Character(s.clone())).with_text(Some(s)));
        }
        self.frame();
    }

    fn press(&mut self, k: Key) {
        self.ui.key(&KeyEvent::press(k).with_text(None));
        self.frame();
    }
}

fn pointer(phase: PointerPhase, p: Point, buttons: Buttons) -> PointerEvent {
    PointerEvent::new(phase, p).with_buttons(buttons)
}

fn hook(f: &TextField, c: &SpellChecker) -> SpellHook {
    f.spell_check_with(c, options()).unwrap_or_else(|| unreachable!("a plain field is checked"))
}

fn labels(items: &[MenuItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| format!("{i:?}").trim_start_matches("MenuItem(").trim_end_matches(')').to_owned())
        .collect()
}

/// A window point over the marked word (the first decoration).
fn over_mark(f: &TextField) -> Point {
    let b = f.el().bounds().unwrap_or_default();
    let mut y = b.min.y + 2.0;
    while y < b.max.y {
        let mut x = b.min.x;
        while x < b.max.x {
            let p = Point::new(x, y);
            if f.decoration_at(p).is_some() {
                return p;
            }
            x += 1.0;
        }
        y += 2.0;
    }
    unreachable!("no point is over a mark")
}

#[test]
fn misspellings_are_marked_once_the_text_has_settled() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("hello wrld here");
    let h = hook(&f, &c);
    assert!(f.decorations().is_empty(), "nothing is marked before the check");
    assert!(h.is_checking());
    t.until(|| !h.is_checking());
    assert_eq!(h.misspellings(), [span(6, 10)]);
    assert_eq!(h.last_error(), None);
    assert_eq!(h.languages(), ["en-US"], "the window's locale");
    // Editing the text again checks it again.
    f.set_value("hello world her");
    t.until(|| !h.is_checking());
    assert_eq!(h.misspellings(), [span(12, 15)]);
    // And clearing it clears the marks.
    f.set_value("");
    t.until(|| !h.is_checking());
    assert!(h.misspellings().is_empty());
}

#[test]
fn the_word_being_typed_is_marked_when_typing_pauses() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("");
    let b = f.el().bounds().unwrap_or_default();
    t.click(Point::new(b.min.x + 2.0, b.min.y + 2.0));
    assert!(f.el().is_focused());
    let h = hook(&f, &c);
    t.type_text("hello wrld");
    t.until(|| !h.is_checking());
    assert!(f.decorations().is_empty(), "still typing that word: {:?}", f.decorations());
    t.fire(|d| d >= Duration::from_secs(1));
    assert_eq!(h.misspellings(), [span(6, 10)], "typing paused");
    // Typing on moves the marks along without a new check.
    t.type_text(" x");
    assert_eq!(h.misspellings(), [span(6, 10)]);
}

#[test]
fn a_new_locale_changes_the_languages_and_checks_again() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("Fuchs hello");
    let h = hook(&f, &c);
    t.until(|| !h.is_checking());
    assert_eq!(h.misspellings(), [span(0, 5)], "German is not English");
    t.ui.set_locale("de-DE");
    t.until(|| !h.is_checking() && h.languages() == ["de-DE"]);
    t.until(|| !h.is_checking());
    assert_eq!(h.misspellings(), [span(6, 11)], "now English is the stranger");
    // A multilingual user: the checker's other languages count too.
    ok(c.set_languages(&["en-US"]));
    assert_eq!(h.languages(), ["de-DE", "en-US"]);
    h.recheck();
    t.until(|| !h.is_checking());
    assert!(h.misspellings().is_empty(), "both words are right in one language or the other");
}

#[test]
fn the_menu_speaks_the_window_language_unless_the_app_gave_its_words() {
    let mut t = T::new();
    t.ui.set_locale("de-DE");
    let c = checker();
    let f = t.field("Fuchs zzyzx");
    let h = hook(&f, &c);
    t.until(|| !h.is_checking());
    t.frame();
    let at = over_mark(&f);
    let items = labels(&h.menu_items(Some(at)));
    assert_eq!(
        items,
        ["Keine Vorschläge".to_owned(), "Zum Wörterbuch hinzufügen".to_owned(), "Ignorieren".to_owned()]
    );
    // Words the app set are its own, whatever the language.
    let mut own = MenuLabels::default();
    own.ignore = "Skip".into();
    let mut opts = options();
    opts.labels = own;
    let h2 = f.spell_check_with(&c, opts).unwrap_or_else(|| unreachable!("a plain field is checked"));
    t.until(|| !h2.is_checking());
    t.frame();
    let items = labels(&h2.menu_items(Some(over_mark(&f))));
    assert_eq!(items.last().map(String::as_str), Some("Skip"), "{items:?}");
    assert_eq!(items.first().map(String::as_str), Some("No suggestions"), "{items:?}");
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_widgets::spell::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

fn span(start: usize, end: usize) -> std::ops::Range<usize> {
    start..end
}

fn ok<T>(r: Result<T, SpellError>) -> T {
    r.unwrap_or_else(|e| unreachable!("{e}"))
}

#[test]
fn a_language_no_engine_has_marks_nothing_and_says_why() {
    let mut t = T::new();
    t.ui.set_locale("tlh");
    let c = checker();
    let f = t.field("qapla teh");
    let h = hook(&f, &c);
    t.until(|| !h.is_checking());
    assert!(h.misspellings().is_empty());
    assert!(matches!(h.last_error(), Some(SpellError::Unavailable { .. })), "{:?}", h.last_error());
}

#[test]
fn secret_fields_are_never_checked() {
    let t = T::new();
    let f = t.ui.text_field("secret_input", FieldKind::Secret);
    assert!(f.spell_check(&checker()).is_none());
}

#[test]
fn detaching_stops_checking_and_clears_the_marks() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("hello wrld");
    let h = hook(&f, &c);
    t.until(|| !h.is_checking());
    assert_eq!(h.misspellings().len(), 1);
    h.detach();
    assert!(!h.is_attached() && f.decorations().is_empty());
    f.set_value("hello wrld again");
    t.until(|| !h.is_checking());
    assert!(f.decorations().is_empty(), "detached");
}

#[test]
fn right_clicking_a_misspelling_offers_corrections_and_applies_one() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("hello wrld here");
    let h = hook(&f, &c);
    let _ = h.context_menu(|| vec![MenuItem::new("Paste", || {})]);
    t.until(|| !h.is_checking());
    t.frame();
    let at = over_mark(&f);
    let items = labels(&h.menu_items(Some(at)));
    assert_eq!(
        items.get(items.len() - 2..),
        Some(["Add to dictionary".to_owned(), "Ignore".to_owned()].as_slice())
    );
    let first = items.first().cloned().unwrap_or_default();
    assert!(first == "world" || items.iter().any(|i| i == "world"), "{items:?}");
    // Not on a mark: no spelling entries.
    assert!(h.menu_items(Some(Point::new(0.0, 0.0))).is_empty());

    t.ui.pointer(&pointer(PointerPhase::Down, at, Buttons::SECONDARY));
    t.frame();
    t.press(Key::Enter);
    assert_eq!(f.value(), format!("hello {first} here"), "the first correction replaced the word");
    t.until(|| !h.is_checking());
    assert!(h.misspellings().is_empty());
}

#[test]
fn the_menu_adds_words_to_the_dictionary_and_ignores_them() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("hello zzyzx");
    let h = hook(&f, &c);
    t.until(|| !h.is_checking());
    t.frame();
    let at = over_mark(&f);
    let items = labels(&h.menu_items(Some(at)));
    assert_eq!(items.first().map(String::as_str), Some("No suggestions"), "{items:?}");
    // Choose "Add to dictionary" with the mouse-free route: the menu's own keys.
    let _ = h.context_menu(Vec::new);
    t.ui.pointer(&pointer(PointerPhase::Down, at, Buttons::SECONDARY));
    t.frame();
    t.press(Key::ArrowDown);
    t.press(Key::Enter);
    assert!(c.personal().contains("zzyzx"));
    assert_eq!(c.personal().to_text(), "zzyzx\n");
    t.until(|| !h.is_checking());
    assert!(h.misspellings().is_empty(), "the added word is no longer marked");

    // Ignore: only for the session.
    f.set_value("hello qqwx");
    t.until(|| !h.is_checking());
    // The right click focused the field, so the word at the caret is marked once typing pauses.
    t.fire(|_| true);
    t.frame();
    let at = over_mark(&f);
    t.ui.pointer(&pointer(PointerPhase::Down, at, Buttons::SECONDARY));
    t.frame();
    t.press(Key::ArrowDown);
    t.press(Key::ArrowDown);
    t.press(Key::Enter);
    assert!(c.personal().contains("qqwx") && !c.personal().to_text().contains("qqwx"));
    t.until(|| !h.is_checking());
    assert!(h.misspellings().is_empty());
}

#[test]
fn the_keyboard_opens_the_menu_for_the_word_at_the_caret() {
    let mut t = T::new();
    let c = checker();
    let f = t.field("hello wrld");
    let h = hook(&f, &c);
    t.until(|| !h.is_checking());
    // The caret is at the end of the text: right after the marked word.
    let items = labels(&h.menu_items(None));
    assert!(items.iter().any(|i| i == "world"), "{items:?}");
}

#[test]
fn a_word_added_anywhere_is_accepted_in_every_field_at_once() {
    let mut t = T::new();
    let c = checker();
    let a = t.field("hello Remedyy");
    let b = t.field("the Remedyy here");
    let (ha, hb) = (hook(&a, &c), hook(&b, &c));
    t.until(|| !ha.is_checking() && !hb.is_checking());
    assert_eq!(ha.misspellings(), [span(6, 13)]);
    assert_eq!(hb.misspellings(), [span(4, 11)]);
    // Added from one field's menu: the other field drops its mark too, with no edit.
    t.frame();
    let at = over_mark(&a);
    let _ = ha.context_menu(Vec::new);
    t.ui.pointer(&pointer(PointerPhase::Down, at, Buttons::SECONDARY));
    t.frame();
    t.press(Key::ArrowDown);
    t.press(Key::Enter);
    assert!(c.personal().contains("Remedyy"));
    t.until(|| hb.misspellings().is_empty() && ha.misspellings().is_empty());
    // A settings page removing it marks it again everywhere.
    assert!(c.personal().remove("Remedyy"));
    t.until(|| hb.misspellings() == [span(4, 11)]);
}

#[test]
fn the_personal_dictionary_matches_however_accents_were_typed() {
    let c = checker();
    // "café" with a single é, then looked up with e and a combining accent.
    assert!(c.personal().add("caf\u{e9}"));
    assert!(c.personal().contains("cafe\u{301}"));
    assert_eq!(c.personal().words(), ["caf\u{e9}"]);
    assert!(!c.personal().add("cafe\u{301}"), "the same word");
}
