//! Customizations end to end, headless, in temporary folders: files written, read back and
//! round-tripped; broken files skipped with a notice and reset in one step; every change
//! undone and redone; shortcut clashes; and the editors operated through the accessibility
//! tree as a screen reader or an agent would.

use std::path::{Path, PathBuf};
use std::time::Duration;

use remedy_ui_a11y::Action;
use remedy_ui_core::{Color, Size};
use remedy_ui_customize::*;
use remedy_ui_input::{Chord, Key, KeyEvent, Modifiers, Platform};
use remedy_ui_services_i18n::{Catalog, MessageEntry, Messages};
use remedy_ui_style::Theme;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_nav::Commands;

/// Unwraps what a test needs, failing it with `why`.
trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

/// A fresh folder under the system's temporary folder, removed afterwards.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("remedy-customize-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).must("a temporary folder");
        TempDir(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.file(name)).ok()
    }

    fn write(&self, name: &str, text: &str) {
        let p = self.file(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).must("the file's folder");
        }
        std::fs::write(p, text).must("a test file");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn base() -> Theme {
    remedy_ui_themes::ThemeChoice::default().resolve(None, false)
}

fn window() -> Ui {
    let theme = base();
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(1000.0, 800.0), 1.0);
    let _ = ui.take_requests();
}

fn custom(dir: &TempDir) -> Customizations {
    let c = Customizations::new(dir.path());
    c.enable_theme(base());
    c
}

fn ctrl(k: char) -> Chord {
    Chord::primary(Platform::Standard, Key::char(k))
}

fn commands() -> Commands {
    let cmds = Commands::new();
    cmds.add("file.save", "Save", vec![ctrl('s')], None, || {});
    cmds.add("message.send", "Send", vec![ctrl('e')], None, || {});
    cmds.add("edit.find", "Find", vec![ctrl('f')], None, || {});
    cmds
}

/// The element named `name` whose semantics pass `pred`, anywhere in the window.
fn find_where(ui: &Ui, name: &str, pred: impl Fn(&remedy_ui_a11y::Semantics) -> bool) -> Option<El> {
    let mut stack: Vec<El> = ui.layers().into_iter().collect();
    while let Some(e) = stack.pop() {
        if e.composed_semantics().is_some_and(|s| s.name() == Some(name) && pred(&s)) {
            return Some(e);
        }
        stack.extend(e.child_elements());
    }
    None
}

fn find(ui: &Ui, name: &str) -> Option<El> {
    find_where(ui, name, |_| true)
}

/// Does `action` on the element named `name`, as assistive technology would.
fn act(ui: &Ui, name: &str, action: Action) -> bool {
    let el = find_where(ui, name, |s| s.actions().contains(&action)).must(name);
    ui.accessibility_action(el.node_id(), action)
}

/// Sets the text of the field named `name`, as assistive technology would.
fn type_into(ui: &Ui, name: &str, text: &str) -> bool {
    let el = find_where(ui, name, |s| s.actions().contains(&Action::SetValue)).must(name);
    ui.accessibility_set_value(el.node_id(), text)
}

fn press(ui: &Ui, modifiers: Modifiers, key: Key) {
    let e = KeyEvent::press(key).with_modifiers(modifiers).with_text(None);
    let _ = ui.key(&e);
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).must("the file is JSON")
}

// --- Theme ---------------------------------------------------------------------------------

#[test]
fn a_theme_change_is_saved_applied_live_and_undoable() {
    let dir = TempDir::new("theme");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    assert!(dir.read("theme.json").is_none(), "nothing is written until something changes");
    let orange = Color::rgb8(0xff, 0x88, 0x00);
    c.set_token(&ui, &Token::color("accent"), TokenValue::Color(orange)).must("set the accent");
    assert_eq!(ui.theme().colors.accent, orange, "applied to the window");
    let file = json(&dir.read("theme.json").must("the theme file"));
    assert_eq!(
        file,
        json(r##"{ "color": { "accent": { "$type": "color", "$value": "#FF8800" } } }"##),
        "only the change"
    );
    assert!(c.is_customized(&Token::color("accent")));

    // A drag across the picker is one entry.
    for g in [0x80, 0x78, 0x70] {
        c.set_token(&ui, &Token::color("accent"), TokenValue::Color(Color::rgb8(0xff, g, 0))).must("drag");
    }
    assert_eq!(ui.history().len(), 1);
    assert_eq!(ui.history().first().map(|h| h.label.clone()).as_deref(), Some("Change Accent"));
    c.set_token(&ui, &Token::size("body"), TokenValue::Px(16.0)).must("body size");
    assert_eq!(ui.theme().type_scale.body, 16.0);
    assert_eq!(ui.history().len(), 2);

    // Read back by a new start of the app.
    let again = custom(&dir);
    let t = again.theme().must("a theme");
    assert_eq!(t.colors.accent, Color::rgb8(0xff, 0x70, 0));
    assert_eq!(t.type_scale.body, 16.0);

    assert!(ui.undo_change());
    assert_eq!(ui.theme().type_scale.body, base().type_scale.body);
    assert!(ui.undo_change());
    assert_eq!(ui.theme().colors.accent, base().colors.accent, "back to before the whole drag");
    assert!(dir.read("theme.json").is_none(), "and the file is gone again");
    assert!(ui.redo_change());
    assert_eq!(ui.theme().colors.accent, Color::rgb8(0xff, 0x70, 0));

    // A value equal to the app's is not a customization.
    c.set_token(&ui, &Token::color("accent"), TokenValue::Color(base().colors.accent))
        .must("back to default");
    assert!(!c.is_customized(&Token::color("accent")));
    assert!(c.set_token(&ui, &Token::color("accent"), TokenValue::Px(3.0)).is_err(), "a size is not a color");
}

#[test]
fn reset_per_token_for_the_theme_and_for_everything_is_undoable() {
    let dir = TempDir::new("theme-reset");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    c.set_token(&ui, &Token::radius("medium"), TokenValue::Px(12.0)).must("radius");
    c.set_token(&ui, &Token::motion("quick"), TokenValue::Duration(Duration::from_millis(40))).must("motion");
    c.set_token(&ui, &Token::density(), TokenValue::Density(remedy_ui_style::Density::Compact))
        .must("density");
    c.set_token(&ui, &Token::font_family(), TokenValue::Families(vec!["Remedy Sans".into(), "serif".into()]))
        .must("font");
    c.reset_token(&ui, &Token::radius("medium")).must("reset one");
    assert_eq!(ui.theme().radii.medium, base().radii.medium);
    assert_eq!(ui.theme().motion.quick, Duration::from_millis(40), "the others stay");
    assert_eq!(c.customized_tokens().len(), 3);
    c.reset_theme(&ui).must("reset theme");
    assert_eq!(ui.theme(), base());
    assert!(ui.undo_change());
    assert_eq!(ui.theme().motion.quick, Duration::from_millis(40));
    assert_eq!(ui.theme().font_families, ["Remedy Sans", "serif"]);
    assert!(ui.undo_change());
    assert_eq!(ui.theme().radii.medium, 12.0);

    let cmds = commands();
    c.enable_shortcuts(&cmds);
    c.add_shortcut(&ui, "edit.find", Chord::new(Modifiers::NONE, Key::F(3)), false).must("F3");
    c.reset_all(&ui).must("reset all");
    assert_eq!(ui.theme(), base());
    assert!(!c.is_rebound("edit.find"));
    assert!(dir.read("theme.json").is_none() && dir.read("shortcuts.json").is_none());
    assert_eq!(ui.next_undo().map(|h| h.label).as_deref(), Some("Reset all customizations"));
    assert!(ui.undo_change());
    assert_eq!(ui.theme().radii.medium, 12.0);
    assert!(c.is_rebound("edit.find"));
}

#[test]
fn a_broken_theme_file_is_skipped_with_a_notice_and_reset_in_one_step() {
    let dir = TempDir::new("theme-broken");
    dir.write("theme.json", "{ \"color\": { \"accent\": ");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    assert_eq!(c.theme(), Some(base()), "the app starts with its own theme");
    let notices = c.notices();
    assert_eq!(notices.len(), 1);
    let n = notices.first().must("a notice");
    assert_eq!(n.area, Area::Theme);
    assert!(n.path.ends_with("theme.json"));

    c.reset_notice(&ui, n).must("reset");
    assert!(c.notices().is_empty());
    assert!(dir.read("theme.json").is_none());
    assert!(ui.undo_change(), "even the reset can be taken back");
    assert_eq!(dir.read("theme.json").as_deref(), Some("{ \"color\": { \"accent\": "));
    assert_eq!(c.notices().len(), 1);

    // A value that is not what its token needs says where.
    dir.write("theme.json", r#"{ "color": { "bg": { "$value": "teal-ish" } } }"#);
    c.reload_all();
    let n = c.notices();
    assert!(n.iter().any(|n| n.message.contains("color.bg")), "{n:?}");
    // Dismissing hides it without touching the file.
    for x in &n {
        c.dismiss(x);
    }
    assert!(c.notices().is_empty());
    assert!(dir.read("theme.json").is_some());
}

#[test]
fn shared_themes_are_untrusted_data() {
    let dir = TempDir::new("theme-import");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    let shared = r##"{
        "$name": "Ocean",
        "color": { "bg": { "$type": "color", "$value": "#0d1b2a" } },
        "glow": { "$type": "shader", "$value": "fn main() {}" },
        "backdrop": { "$type": "string", "$value": "url(https://images.example/wave.png)" }
    }"##;
    let found = inspect_theme(shared).must("inspect");
    assert_eq!(found.len(), 2);
    let Err(CustomizeError::Refused(refused)) = c.import_theme(&ui, shared, ImportPolicy::default()) else {
        unreachable!("a shader and a link are refused by default")
    };
    assert!(refused.iter().any(|r| r.kind == RefusalKind::Shader && r.token == "glow"));
    assert!(refused.iter().any(|r| r.kind == RefusalKind::ExternalUrl && r.token == "backdrop.$value"));
    assert!(dir.read("theme.json").is_none(), "nothing changed");
    assert!(ui.history().is_empty());

    // Leaving them out.
    let report = c
        .import_theme(&ui, shared, {
            let mut v_ = ImportPolicy::default();
            v_.drop_refused = true;
            v_
        })
        .must("import the rest");
    assert_eq!(report.name, "Ocean");
    assert_eq!(report.dropped.len(), 2);
    assert_eq!(ui.theme().colors.bg, Color::rgb8(0x0d, 0x1b, 0x2a));
    let file = dir.read("theme.json").must("written");
    assert!(!file.contains("shader") && !file.contains("https"), "{file}");

    // The person allows links: the link is kept in the file, the shader still left out.
    let policy = {
        let mut v_ = ImportPolicy::default();
        v_.allow_external_urls = true;
        v_.drop_refused = true;
        v_
    };
    let report = c.import_theme(&ui, shared, policy).must("import with links");
    assert_eq!(report.dropped.len(), 1);
    let file = dir.read("theme.json").must("written");
    assert!(file.contains("https://images.example/wave.png") && !file.contains("shader"), "{file}");

    // A bad token refuses the whole theme, with nothing changed.
    let before = dir.read("theme.json");
    assert!(matches!(
        c.import_theme(&ui, r#"{ "size": { "body": "huge" } }"#, ImportPolicy::default()),
        Err(CustomizeError::Token(_))
    ));
    assert!(matches!(
        c.import_theme(&ui, "[1, 2]", ImportPolicy::default()),
        Err(CustomizeError::Invalid(_))
    ));
    assert_eq!(dir.read("theme.json"), before);

    // Exported whole, it imports the same into another app's customizations.
    let out = dir.file("shared/ocean.json");
    c.export_theme(&out).must("export");
    let other_dir = TempDir::new("theme-import-other");
    let other = custom(&other_dir);
    let other_ui = window();
    other
        .import_theme_file(&other_ui, &out, {
            let mut v_ = ImportPolicy::default();
            v_.allow_external_urls = true;
            v_
        })
        .must("import the export");
    assert_eq!(other.theme(), c.theme());
    assert!(ui.undo_change());
    assert!(ui.undo_change());
    assert!(dir.read("theme.json").is_none(), "imports undo like any change");
}

#[test]
fn contrast_warnings_name_the_pairs_below_wcag() {
    for t in remedy_ui_themes::all() {
        assert!(contrast_warnings(&t).is_empty(), "{}: {:?}", t.name, contrast_warnings(&t));
    }
    // Light cards in a dark theme: the text, kept readable on the background, is not on them.
    let mut t = base();
    t.colors.card = Color::rgb8(0xf0, 0xf0, 0xf0);
    let w = contrast_warnings(&t);
    let cards = w.iter().find(|w| w.pair == "Text on cards").must("text on cards is flagged");
    assert!(cards.ratio < cards.needed);
    assert_eq!(cards.background, Token::color("card"));
    assert!(!w.iter().any(|w| w.pair == "Text on the background"), "the engine keeps that one readable");
}

// --- Shortcuts -----------------------------------------------------------------------------

#[test]
fn shortcuts_are_saved_checked_for_clashes_and_undoable() {
    let dir = TempDir::new("shortcuts");
    let ui = window();
    let c = Customizations::new(dir.path());
    let cmds = commands();
    c.enable_shortcuts(&cmds);

    // Ctrl+S already saves: refused with the clash, nothing changed.
    let Err(CustomizeError::Clashes(clashes)) = c.add_shortcut(&ui, "message.send", ctrl('s'), false) else {
        unreachable!("Ctrl+S is Save's")
    };
    assert_eq!(clashes.first().map(|c| c.commands.clone()), Some(vec!["file.save".to_string()]));
    assert!(dir.read("shortcuts.json").is_none());
    assert_eq!(c.shortcut_conflicts("message.send", &ctrl('s')).len(), 1);

    // The person gives it to Send: Save loses it, in one change.
    c.add_shortcut(&ui, "message.send", ctrl('s'), true).must("take Ctrl+S");
    assert_eq!(cmds.info("message.send").map(|i| i.chords), Some(vec![ctrl('e'), ctrl('s')]));
    assert_eq!(cmds.info("file.save").map(|i| i.chords), Some(vec![]));
    assert_eq!(
        json(&dir.read("shortcuts.json").must("written")),
        json(r#"{ "file.save": [], "message.send": ["Ctrl+E", "Ctrl+S"] }"#)
    );
    assert_eq!(ui.history().len(), 1);
    assert!(ui.undo_change());
    assert_eq!(cmds.info("file.save").map(|i| i.chords), Some(vec![ctrl('s')]));
    assert_eq!(cmds.info("message.send").map(|i| i.chords), Some(vec![ctrl('e')]));
    assert!(ui.redo_change());

    c.remove_shortcut(&ui, "message.send", &ctrl('e')).must("remove");
    assert_eq!(cmds.info("message.send").map(|i| i.chords), Some(vec![ctrl('s')]));
    // Save's default (Ctrl+S) is Send's now: resetting Save would clash.
    assert!(matches!(c.reset_shortcut(&ui, "file.save"), Err(CustomizeError::Clashes(_))));
    c.reset_shortcut(&ui, "message.send").must("reset send");
    c.reset_shortcut(&ui, "file.save").must("reset save");
    assert!(dir.read("shortcuts.json").is_none(), "defaults everywhere: no file");
    assert!(ui.undo_change());
    assert!(c.is_rebound("file.save"));

    c.set_shortcuts(&ui, "edit.find", vec![Chord::new(Modifiers::NONE, Key::F(3))]).must("set");
    assert!(matches!(c.set_shortcuts(&ui, "edit.find", vec![ctrl('e')]), Err(CustomizeError::Clashes(_))));
    c.reset_shortcuts(&ui).must("reset all");
    assert!(cmds.bindings().is_empty());
    assert!(ui.undo_change());
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![Chord::new(Modifiers::NONE, Key::F(3))]));

    // A new start of the app reads them back, including commands registered later.
    let cmds2 = Commands::new();
    cmds2.add("edit.find", "Find", vec![ctrl('f')], None, || {});
    let c2 = Customizations::new(dir.path());
    c2.enable_shortcuts(&cmds2);
    assert_eq!(cmds2.info("edit.find").map(|i| i.chords), Some(vec![Chord::new(Modifiers::NONE, Key::F(3))]));
    cmds2.add("file.save", "Save", vec![ctrl('s')], None, || {});
    assert_eq!(
        cmds2.info("file.save").map(|i| i.chords),
        Some(vec![]),
        "the person took Save's shortcut away"
    );
}

#[test]
fn a_broken_shortcuts_file_is_skipped_and_a_clashing_one_loses_only_its_clashes() {
    let dir = TempDir::new("shortcuts-broken");
    let ui = window();
    dir.write("shortcuts.json", r#"{ "file.save": ["Hyper+S"] }"#);
    let c = Customizations::new(dir.path());
    let cmds = commands();
    c.enable_shortcuts(&cmds);
    assert_eq!(cmds.info("file.save").map(|i| i.chords), Some(vec![ctrl('s')]), "defaults apply");
    let n = c.notices();
    assert!(n.iter().any(|n| n.area == Area::Shortcuts && n.message.contains("Hyper")), "{n:?}");

    // Send would share Ctrl+S with Save: only that shortcut is left out, the rest applies.
    dir.write("shortcuts.json", r#"{ "message.send": ["Ctrl+S", "F6"], "edit.find": ["F7"] }"#);
    c.reload_all();
    let n = c.notices();
    assert_eq!(n.len(), 1, "{n:?}");
    assert!(
        n.iter().any(|n| n.message.contains("file.save and message.send would both answer to Ctrl+S")),
        "{n:?}"
    );
    assert!(!n.iter().any(|n| n.message.contains("F6") || n.message.contains("F7")), "only the clash: {n:?}");
    assert_eq!(
        cmds.info("message.send").map(|i| i.chords),
        Some(vec![Chord::new(Modifiers::NONE, Key::F(6))])
    );
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![Chord::new(Modifiers::NONE, Key::F(7))]));
    assert_eq!(cmds.info("file.save").map(|i| i.chords), Some(vec![ctrl('s')]));
    let first = n.first().must("a notice").clone();
    c.reset_notice(&ui, &first).must("reset");
    assert!(c.notices().is_empty());
    assert!(dir.read("shortcuts.json").is_none());
    assert_eq!(cmds.info("message.send").map(|i| i.chords), Some(vec![ctrl('e')]));
}

#[test]
fn the_apps_own_bindings_stay_beneath_the_persons() {
    let dir = TempDir::new("shortcuts-layers");
    let ui = window();
    let cmds = commands();
    // The app restores its own binding (from its settings) before customization is on.
    cmds.rebind("edit.find", vec![Chord::new(Modifiers::NONE, Key::F(3))]).must("app binding");
    let c = Customizations::new(dir.path());
    c.enable_shortcuts(&cmds);
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![Chord::new(Modifiers::NONE, Key::F(3))]));
    assert!(!c.is_rebound("edit.find"), "the app's binding is not the person's");

    c.set_shortcuts(&ui, "edit.find", vec![Chord::new(Modifiers::NONE, Key::F(4))]).must("person");
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![Chord::new(Modifiers::NONE, Key::F(4))]));
    // Setting it back to the app's binding is no customization at all.
    c.set_shortcuts(&ui, "edit.find", vec![Chord::new(Modifiers::NONE, Key::F(3))]).must("back");
    assert!(dir.read("shortcuts.json").is_none());
    c.set_shortcuts(&ui, "edit.find", vec![Chord::new(Modifiers::NONE, Key::F(4))]).must("person again");
    c.reset_shortcut(&ui, "edit.find").must("reset");
    assert_eq!(
        cmds.info("edit.find").map(|i| i.chords),
        Some(vec![Chord::new(Modifiers::NONE, Key::F(3))]),
        "a reset goes back to the app's binding, not the built-in one"
    );
    // The app changing its binding later still shows while the person has none.
    cmds.rebind("edit.find", vec![Chord::new(Modifiers::NONE, Key::F(5))]).must("app again");
    c.reload_all();
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![Chord::new(Modifiers::NONE, Key::F(5))]));
    cmds.reset("edit.find");
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![ctrl('f')]), "the built-in one beneath");
}

// --- Toolbars ------------------------------------------------------------------------------

#[test]
fn toolbars_keep_the_persons_order_and_reset_to_the_apps() {
    let dir = TempDir::new("toolbars");
    let ui = window();
    let c = Customizations::new(dir.path());
    let cmds = commands();
    c.enable_toolbars(&cmds);
    let slot = c.toolbar_slot(&ui, "main", "Main toolbar", &["file.save", "edit.find"]).must("a slot");
    assert_eq!(slot.get_untracked().must("value"), ["file.save", "edit.find"]);
    assert!(c.toolbar_slot(&ui, "main", "Main toolbar", &[]).is_some_and(|s| s == slot), "declared once");

    // The toolbar editor writes straight into the slot: that is saved and undoable.
    slot.set(vec!["edit.find".into(), "file.save".into(), "message.send".into()]);
    assert_eq!(
        json(&dir.read("toolbars.json").must("written")),
        json(r#"{ "main": ["edit.find", "file.save", "message.send"] }"#)
    );
    assert!(c.is_toolbar_customized("main"));
    assert_eq!(ui.next_undo().map(|h| h.label).as_deref(), Some("Change Main toolbar"));
    assert!(ui.undo_change());
    assert_eq!(slot.get_untracked().must("value"), ["file.save", "edit.find"]);
    assert!(dir.read("toolbars.json").is_none());
    assert_eq!(ui.history().len(), 0, "undoing is not itself a change");
    assert!(ui.redo_change());
    assert_eq!(slot.get_untracked().must("value").len(), 3);

    // Read back by a new start of the app.
    let c2 = Customizations::new(dir.path());
    c2.enable_toolbars(&cmds);
    let s2 = c2.toolbar_slot(&ui, "main", "Main toolbar", &["file.save", "edit.find"]).must("slot");
    assert_eq!(s2.get_untracked().must("value").len(), 3);

    c.set_toolbar(&ui, "main", vec!["file.save".into(), "edit.find".into()]).must("back to the defaults");
    assert!(!c.is_toolbar_customized("main"), "the app's own list is not a customization");
    slot.set(vec![]);
    c.reset_toolbar(&ui, "main").must("reset");
    assert_eq!(slot.get_untracked().must("value"), ["file.save", "edit.find"]);
    assert!(matches!(c.reset_toolbar(&ui, "nope"), Err(CustomizeError::UnknownSlot(_))));

    dir.write("toolbars.json", r#"{ "main": "file.save" }"#);
    c.reload_all();
    assert!(c.notices().iter().any(|n| n.area == Area::Toolbars));
    assert_eq!(slot.get_untracked().must("value"), ["file.save", "edit.find"], "defaults apply");
    c.reset_toolbars(&ui).must("reset all");
    assert!(c.notices().is_empty());
}

// --- Text ----------------------------------------------------------------------------------

#[test]
fn every_string_can_be_rewritten_in_message_files() {
    let dir = TempDir::new("text");
    let ui = window();
    let catalog = Catalog::new("en").embedded_json("fr", r#"{"Share a folder": "Partager un dossier"}"#);
    let c = Customizations::new(dir.path());
    c.enable_text(&catalog);
    assert_eq!(catalog.text_in("fr", "Share a folder"), "Partager un dossier");
    c.set_text(&ui, "fr-FR", "Share a folder", "Partager un répertoire").must("rewrite");
    assert_eq!(catalog.text_in("fr", "Share a folder"), "Partager un répertoire");
    assert_eq!(
        json(&dir.read("text/fr-FR.json").must("written")),
        json(r#"{"Share a folder": "Partager un répertoire"}"#)
    );
    // Typing is one entry.
    c.set_text(&ui, "fr-FR", "Share a folder", "Partager un rép").must("typing");
    assert_eq!(ui.history().len(), 1);
    c.set_text(&ui, "en", "Share a folder", "Share a directory").must("the source language too");
    assert_eq!(catalog.text_in("en-US", "Share a folder"), "Share a directory");
    assert!(ui.undo_change());
    assert!(dir.read("text/en.json").is_none(), "a file the change made goes on undo");
    assert!(ui.undo_change());
    assert_eq!(catalog.text_in("fr", "Share a folder"), "Partager un dossier");
    assert!(ui.redo_change());
    assert_eq!(c.text_override("fr-FR", "Share a folder").as_deref(), Some("Partager un rép"));
    c.reset_text(&ui, "fr-FR", "Share a folder").must("reset one");
    assert_eq!(catalog.text_in("fr", "Share a folder"), "Partager un dossier");

    // A broken file is skipped with a notice; resetting it removes it.
    dir.write("text/de.json", "{ broken");
    c.reload_text();
    let n = c.notices();
    let broken = n.iter().find(|n| n.path.ends_with("de.json")).must("a notice for de.json").clone();
    c.reset_notice(&ui, &broken).must("reset");
    assert!(dir.read("text/de.json").is_none());
    c.set_text(&ui, "fr", "Share a folder", "Partager").must("again");
    c.reset_texts(&ui).must("reset all");
    assert!(catalog.override_locales().is_empty());
    assert!(ui.undo_change());
    assert_eq!(catalog.text_in("fr", "Share a folder"), "Partager");
}

// --- Editors through the accessibility tree --------------------------------------------------

#[test]
fn the_theme_editor_works_from_assistive_technology() {
    let dir = TempDir::new("theme-editor");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    let _ = ui.root().child(theme_editor(&ui, &c));
    frame(&ui);
    assert!(type_into(&ui, "Body text size", "17"));
    assert_eq!(ui.theme().type_scale.body, 17.0);
    assert!(c.is_customized(&Token::size("body")));
    assert!(act(&ui, "Reset Body text size", Action::Click));
    assert_eq!(ui.theme().type_scale.body, base().type_scale.body);
    assert!(ui.undo_change());
    assert_eq!(ui.theme().type_scale.body, 17.0);
    frame(&ui);

    // Density and colors.
    assert!(act(&ui, "Compact", Action::Click));
    assert_eq!(ui.theme().density, remedy_ui_style::Density::Compact);
    assert!(act(&ui, "Edit Cards", Action::Click));
    frame(&ui);
    assert!(type_into(&ui, "Cards hex", "#f0f0f0"));
    assert_eq!(ui.theme().colors.card, Color::rgb8(0xf0, 0xf0, 0xf0));
    frame(&ui);
    let contrast =
        find_where(&ui, "Contrast", |s| s.role() == remedy_ui_a11y::Role::Status).must("the contrast status");
    let value = contrast.composed_semantics().and_then(|s| s.value_text().map(str::to_owned)).must("a value");
    assert!(value.contains("below WCAG"), "light cards under light text are flagged: {value}");
    assert!(act(&ui, "Reset theme", Action::Click));
    assert_eq!(ui.theme(), base());
    let value = find_where(&ui, "Contrast", |s| s.role() == remedy_ui_a11y::Role::Status)
        .and_then(|e| e.composed_semantics())
        .and_then(|s| s.value_text().map(str::to_owned));
    assert_eq!(value.as_deref(), Some("Every color pair meets WCAG contrast"));

    // Importing from a path, with a refusal to decide on.
    let shared = dir.file("ocean.json");
    std::fs::write(
        &shared,
        r##"{ "$name": "Ocean", "color": { "accent": "#3080ff" }, "x": "https://example.com/a.png" }"##,
    )
    .must("write");
    assert!(type_into(&ui, "Theme file", &shared.to_string_lossy()));
    assert!(act(&ui, "Import from path", Action::Click));
    frame(&ui);
    assert!(find(&ui, "This theme carries more than colors and sizes").is_some(), "the person decides");
    assert_eq!(ui.theme().colors.accent, base().colors.accent, "nothing changed yet");
    assert!(act(&ui, "Import without them", Action::Click));
    assert_eq!(ui.theme().colors.accent, Color::rgb8(0x30, 0x80, 0xff));
    assert_eq!(ui.theme().name, "Ocean");
}

#[test]
fn the_shortcut_editor_records_keys_and_resolves_clashes() {
    let dir = TempDir::new("shortcut-editor");
    let ui = window();
    let c = Customizations::new(dir.path());
    let cmds = commands();
    c.enable_shortcuts(&cmds);
    let _ = ui.root().child(shortcut_editor(&ui, &c));
    frame(&ui);
    // Record F3 for Find.
    assert!(act(&ui, "Add a shortcut to Find", Action::Focus));
    press(&ui, Modifiers::NONE, Key::F(3));
    assert_eq!(cmds.info("edit.find").map(|i| i.chords.len()), Some(2));
    frame(&ui);
    assert!(act(&ui, "Remove F3 from Find", Action::Click));
    assert_eq!(cmds.info("edit.find").map(|i| i.chords), Some(vec![ctrl('f')]));

    // Ctrl+S for Send clashes with Save: nothing changes until the person decides.
    assert!(act(&ui, "Add a shortcut to Send", Action::Focus));
    press(&ui, Modifiers { ctrl: true, ..Modifiers::NONE }, Key::char('s'));
    frame(&ui);
    assert!(find(&ui, "Ctrl+S already runs Save.").is_some(), "the clash is shown");
    assert_eq!(cmds.info("message.send").map(|i| i.chords), Some(vec![ctrl('e')]));
    assert!(act(&ui, "Use Ctrl+S for Send", Action::Click));
    assert_eq!(cmds.info("message.send").map(|i| i.chords), Some(vec![ctrl('e'), ctrl('s')]));
    assert_eq!(cmds.info("file.save").map(|i| i.chords), Some(vec![]));
    frame(&ui);
    assert!(find(&ui, "Ctrl+S already runs Save.").is_none());

    // Search narrows the list.
    assert!(type_into(&ui, "Search commands", "find"));
    frame(&ui);
    assert!(find(&ui, "Add a shortcut to Send").is_none());
    assert!(find(&ui, "Add a shortcut to Find").is_some());
    assert!(type_into(&ui, "Search commands", ""));
    frame(&ui);
    assert!(act(&ui, "Reset all shortcuts", Action::Click));
    assert!(cmds.bindings().is_empty());
    assert!(ui.undo_change());
    assert_eq!(cmds.info("file.save").map(|i| i.chords), Some(vec![]));
}

#[test]
fn the_settings_page_shows_notices_toolbars_and_text() {
    let dir = TempDir::new("settings");
    dir.write("theme.json", "not json");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    let cmds = commands();
    c.enable_shortcuts(&cmds);
    c.enable_toolbars(&cmds);
    let slot = c.toolbar_slot(&ui, "main", "Main toolbar", &["file.save"]).must("slot");
    let catalog = Catalog::new("en");
    let _ = catalog.text_in("en", "Share a folder");
    c.enable_text(&catalog);
    let _ = ui.root().child(settings_page(&ui, &c));
    frame(&ui);

    // The broken theme is reported and fixed from the page.
    assert!(find(&ui, "Theme could not be used (theme.json): not a design tokens file: expected ident at line 1 column 2").is_some());
    assert!(act(&ui, "Reset theme.json", Action::Click));
    assert!(c.notices().is_empty());
    assert!(dir.read("theme.json").is_none());
    frame(&ui);

    // Toolbars.
    assert!(act(&ui, "Toolbars and menus", Action::Click));
    frame(&ui);
    assert!(act(&ui, "Find", Action::Click), "the switch for Find in the toolbar editor");
    assert_eq!(slot.get_untracked().must("value"), ["file.save", "edit.find"]);
    assert!(dir.read("toolbars.json").is_some());
    frame(&ui);
    assert!(act(&ui, "Reset Main toolbar", Action::Click));
    assert_eq!(slot.get_untracked().must("value"), ["file.save"]);

    // Text.
    assert!(act(&ui, "Text", Action::Click));
    frame(&ui);
    assert!(type_into(&ui, "Share a folder", "Share a directory"));
    assert_eq!(catalog.text_in("en", "Share a folder"), "Share a directory");
    assert!(act(&ui, "Reset “Share a folder”", Action::Click));
    assert_eq!(catalog.text_in("en", "Share a folder"), "Share a folder");

    // Everything at once.
    c.set_token(&ui, &Token::stroke(), TokenValue::Px(3.0)).must("stroke");
    assert!(act(&ui, "Reset all customizations", Action::Click));
    assert_eq!(ui.theme(), base());
    assert!(ui.undo_change());
    assert_eq!(ui.theme().stroke, 3.0);
}

// --- Screen layouts ------------------------------------------------------------------------

const SETTINGS_RUI: &str = "column { switch(label: \"Noise suppression\", checked: $noise) }\n";

/// A screen the app builds through the customizations, with its built-in version.
fn screen(c: &Customizations, ui: &Ui, noise: remedy_ui_reactive::Signal<bool>) -> remedy_ui_rui::Layout {
    let scope = remedy_ui_rui::Scope::new().flag("noise", noise);
    let built_in = move |ui: &Ui| remedy_ui_widgets_core::switch(ui, "Noise suppression", noise);
    c.layout(ui, "settings.rui", &scope, SETTINGS_RUI, built_in).must("layouts are on")
}

#[test]
fn people_rearrange_screens_with_notices_resets_and_undo() {
    let dir = TempDir::new("layouts");
    let ui = window();
    let c = Customizations::new(dir.path());
    c.enable_layouts(remedy_ui_rui::Widgets::new());
    let noise = remedy_ui_reactive::Signal::new(false);
    let layout = screen(&c, &ui, noise);
    let _ = ui.root().child(layout.el());
    frame(&ui);
    assert_eq!(c.layout_files(), ["settings.rui"]);
    assert_eq!(c.layout_source("settings.rui").as_deref(), Some(SETTINGS_RUI), "the app's own to start from");
    assert!(!c.is_layout_customized("settings.rui"));

    let mine = "row { switch(label: \"Quiet\", checked: $noise) }\n";
    let Err(CustomizeError::Invalid(why)) =
        c.check_layout("settings.rui", "row { switch(label: \"Q\", checked: $nope) }")
    else {
        unreachable!("an unknown binding is refused")
    };
    assert!(why.contains("settings.rui:1:"), "it says where: {why}");
    c.check_layout("settings.rui", mine).must("valid");
    c.save_layout("settings.rui", mine).must("saved through the layout");
    frame(&ui);
    assert!(layout.is_custom());
    assert!(c.is_layout_customized("settings.rui"));
    assert_eq!(dir.read("layouts/settings.rui").as_deref(), Some(mine));
    let quiet = find(&ui, "Quiet").must("their switch");
    assert!(ui.accessibility_action(quiet.node_id(), Action::Click));
    assert_eq!(noise.get_untracked().ok(), Some(true), "bound to the app's signal");

    // A broken version joins the notices, and resets in one step, undoably.
    dir.write("layouts/settings.rui", "row { switch(label: \"Quiet\" }");
    c.reload_all();
    let n = c.notices();
    let broken = n.iter().find(|n| n.area == Area::Layouts).must("a notice for the layout").clone();
    assert!(broken.path.ends_with("settings.rui"));
    assert!(find(&ui, "Noise suppression").is_some(), "the app's screen shows meanwhile");
    c.reset_notice(&ui, &broken).must("reset");
    assert!(c.notices().is_empty());
    assert!(dir.read("layouts/settings.rui").is_none());
    assert!(ui.undo_change());
    assert!(
        c.notices().iter().any(|n| n.area == Area::Layouts),
        "the broken file is back, and so is its notice"
    );

    // Reset everything covers layouts.
    c.save_layout("settings.rui", mine).must("saved");
    c.reset_all(&ui).must("reset all");
    assert!(!c.is_layout_customized("settings.rui"));
    assert!(!layout.is_custom());
    assert!(ui.undo_change());
    assert!(layout.is_custom());
    c.reset_layout(&ui, "settings.rui").must("reset one");
    assert!(!layout.is_custom());
    c.reset_layouts(&ui).must("reset all layouts");
}

#[test]
fn the_layout_editor_checks_as_the_person_types_and_saves() {
    let dir = TempDir::new("layouts-editor");
    let ui = window();
    let c = Customizations::new(dir.path());
    c.enable_layouts(remedy_ui_rui::Widgets::new());
    let noise = remedy_ui_reactive::Signal::new(false);
    let layout = screen(&c, &ui, noise);
    let _ = ui.root().child(layout.el()).child(settings_page(&ui, &c));
    frame(&ui);
    let check = |ui: &Ui| {
        find_where(ui, "Layout check", |s| s.role() == remedy_ui_a11y::Role::Status)
            .and_then(|e| e.composed_semantics())
            .and_then(|s| s.value_text().map(str::to_owned))
            .unwrap_or_default()
    };
    assert!(check(&ui).starts_with("No problems"), "{}", check(&ui));
    assert!(type_into(&ui, "Layout settings.rui", "row { switch(label: \"Quiet\", checked: $nope) }"));
    frame(&ui);
    assert!(check(&ui).contains("settings.rui:1:"), "what is wrong and where: {}", check(&ui));
    assert!(act(&ui, "Save layout settings.rui", Action::Click));
    assert!(!layout.is_custom(), "a layout that does not load is not saved");
    assert!(type_into(&ui, "Layout settings.rui", "row { switch(label: \"Quiet\", checked: $noise) }"));
    frame(&ui);
    assert!(check(&ui).starts_with("No problems"));
    assert!(act(&ui, "Save layout settings.rui", Action::Click));
    assert!(layout.is_custom());
    frame(&ui);
    assert!(act(&ui, "Reset layout settings.rui", Action::Click));
    assert!(!layout.is_custom());
}

// --- Fluent messages -----------------------------------------------------------------------

#[test]
fn fluent_messages_are_rewritten_one_at_a_time_with_checks() {
    let dir = TempDir::new("fluent");
    let ui = window();
    let messages = Messages::new("en").embedded(
        "en",
        "share = Share a folder\n    .tooltip = Share it with people\nhello = Hello, { $name }!\n",
    );
    let c = Customizations::new(dir.path());
    c.enable_messages(&messages);
    let all = c.message_entries("en");
    let (share, app, mine) = all.iter().find(|(e, _, _)| e.id == "share").must("listed").clone();
    assert_eq!(share, app);
    assert!(!mine);
    assert_eq!(share.attributes, [("tooltip".to_owned(), "Share it with people".to_owned())]);

    let mut changed = share.clone();
    changed.value = Some("Share a directory".into());
    c.set_message(&ui, "en", &changed).must("rewrite");
    assert_eq!(messages.format_in("en", "share", None, &[]), "Share a directory");
    assert_eq!(
        messages.format_in("en", "share", Some("tooltip"), &[]),
        "Share it with people",
        "the rest stays"
    );
    let file = dir.read("text/en.ftl").must("the override file");
    assert!(file.contains("share = Share a directory"), "{file}");
    assert!(c.message_entries("en").iter().any(|(e, _, mine)| e.id == "share" && *mine));

    // Placeholders stay; a message that does not read back is refused.
    let hello =
        MessageEntry { id: "hello".into(), value: Some("Hi, { $name }!".into()), attributes: Vec::new() };
    c.set_message(&ui, "en", &hello).must("placeholders");
    assert!(messages.format_in("en", "hello", None, &[("name", "Ada".into())]).contains("Ada"));
    let broken =
        MessageEntry { id: "hello".into(), value: Some("Hi, { $name".into()), attributes: Vec::new() };
    assert!(matches!(c.set_message(&ui, "en", &broken), Err(CustomizeError::Invalid(_))));

    assert!(ui.undo_change());
    assert!(messages.format_in("en", "hello", None, &[("name", "Ada".into())]).starts_with("Hello"));
    c.reset_message(&ui, "en", "share").must("reset");
    assert_eq!(messages.format_in("en", "share", None, &[]), "Share a folder");
    assert!(dir.read("text/en.ftl").is_none(), "an empty override file goes");
    assert!(ui.undo_change());
    assert_eq!(messages.format_in("en", "share", None, &[]), "Share a directory");
}

#[test]
fn the_text_editor_edits_fluent_messages_and_their_attributes() {
    let dir = TempDir::new("fluent-editor");
    let ui = window();
    ui.set_locale("en");
    let messages =
        Messages::new("en").embedded("en", "share = Share a folder\n    .tooltip = Share it with people\n");
    let c = Customizations::new(dir.path());
    c.enable_messages(&messages);
    let _ = ui.root().child(text_editor(&ui, &c));
    frame(&ui);
    assert!(type_into(&ui, "share.tooltip", "Send it to people"));
    assert_eq!(messages.format_in("en", "share", Some("tooltip"), &[]), "Send it to people");
    assert!(type_into(&ui, "share", "Share { $oops"));
    frame(&ui);
    let check = find_where(&ui, "share check", |s| s.role() == remedy_ui_a11y::Role::Status)
        .and_then(|e| e.composed_semantics())
        .and_then(|s| s.value_text().map(str::to_owned))
        .unwrap_or_default();
    assert!(check.starts_with("Not saved"), "the check says why: {check}");
    assert_eq!(messages.format_in("en", "share", None, &[]), "Share a folder", "nothing broken was written");
    assert!(act(&ui, "Reset share", Action::Click));
    assert_eq!(messages.format_in("en", "share", Some("tooltip"), &[]), "Share it with people");
}

// --- Theme files ---------------------------------------------------------------------------

/// A 1×1 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0, 0, 0, 181,
    28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 96, 0, 0, 0, 6, 0, 2, 48, 129, 208, 47, 0, 0,
    0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// A font file and its family name.
fn a_font() -> (&'static [u8], &'static str) {
    let face = remedy_ui_fonts::all().into_iter().next().must("a bundled font");
    (face.data, face.family)
}

/// Whether the window shows the theme's background (an image element first in the root).
fn has_background(ui: &Ui) -> bool {
    ui.root().child_elements().first().is_some_and(|e| e.info().is_some_and(|i| i.widget == "image"))
}

#[test]
fn a_shared_themes_files_come_only_when_allowed_and_are_used() {
    let dir = TempDir::new("assets-import");
    let shared = TempDir::new("assets-shared");
    let (font, family) = a_font();
    std::fs::create_dir_all(shared.file("files")).must("folder");
    std::fs::write(shared.file("files/waves.png"), PNG).must("image");
    std::fs::write(shared.file("files/Ocean.ttf"), font).must("font");
    shared.write(
        "ocean.json",
        r#"{ "$name": "Ocean",
             "asset": { "background": { "$type": "image", "$value": "files/waves.png" },
                        "font": { "Ocean": { "$type": "font", "$value": "files/Ocean.ttf" } } },
             "font": { "family": { "$type": "fontFamily", "$value": ["Ocean"] } } }"#,
    );
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    let Err(CustomizeError::Refused(r)) =
        c.import_theme_file(&ui, &shared.file("ocean.json"), ImportPolicy::default())
    else {
        unreachable!("files are refused unless allowed")
    };
    assert_eq!(r.iter().filter(|r| r.kind == RefusalKind::Asset).count(), 2);
    assert!(dir.read("theme.json").is_none());

    // Without them: the colors and names come, the files do not.
    let policy = {
        let mut v_ = ImportPolicy::default();
        v_.drop_refused = true;
        v_
    };
    c.import_theme_file(&ui, &shared.file("ocean.json"), policy).must("import without files");
    assert!(!has_background(&ui));
    assert!(ui.undo_change());

    // Allowed: copied in, checked, and used.
    let policy = {
        let mut v_ = ImportPolicy::default();
        v_.allow_assets = true;
        v_
    };
    c.import_theme_file(&ui, &shared.file("ocean.json"), policy).must("import with files");
    let file = dir.read("theme.json").must("written");
    assert!(file.contains("theme-assets/waves.png") && file.contains("theme-assets/Ocean.ttf"), "{file}");
    assert!(dir.file("theme-assets/waves.png").exists() && dir.file("theme-assets/Ocean.ttf").exists());
    let t = c.theme().must("theme");
    assert_eq!(t.font_families, [family], "the font token resolves to the family the file holds");
    assert!(ui.font_families().iter().any(|f| f == family));
    frame(&ui);
    assert!(has_background(&ui), "the background shows behind the content");
    // Undo takes the copies away too.
    assert!(ui.undo_change());
    assert!(!dir.file("theme-assets/waves.png").exists());
    frame(&ui);
    assert!(!has_background(&ui));

    // A file outside the theme's folder is refused; so is a path in a theme without its file.
    shared.write("escape.json", r#"{ "asset": { "background": "../../waves.png" } }"#);
    let allow = {
        let mut v_ = ImportPolicy::default();
        v_.allow_assets = true;
        v_
    };
    assert!(matches!(
        c.import_theme_file(&ui, &shared.file("escape.json"), allow),
        Err(CustomizeError::Invalid(_))
    ));
    assert!(matches!(
        c.import_theme(&ui, r#"{ "asset": { "background": "files/waves.png" } }"#, allow),
        Err(CustomizeError::Invalid(_))
    ));
    // Inline images come in text.
    let inline = r#"{ "asset": { "background": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=" } }"#;
    c.import_theme(&ui, inline, allow).must("inline image");
    frame(&ui);
    assert!(has_background(&ui));
    // Something that is not an image is refused before anything changes.
    shared.write("files/fake.png", "not an image");
    shared.write("fake.json", r#"{ "asset": { "background": "files/fake.png" } }"#);
    assert!(matches!(
        c.import_theme_file(&ui, &shared.file("fake.json"), allow),
        Err(CustomizeError::Invalid(_))
    ));
}

#[test]
fn backgrounds_and_fonts_are_chosen_exported_and_reset() {
    let dir = TempDir::new("assets-own");
    let elsewhere = TempDir::new("assets-own-files");
    let (font, family) = a_font();
    std::fs::write(elsewhere.file("bg.png"), PNG).must("image");
    std::fs::write(elsewhere.file("Mine.ttf"), font).must("font");
    let ui = window();
    let c = custom(&dir);
    c.attach(&ui);
    c.set_background(&ui, Some(&elsewhere.file("bg.png"))).must("background");
    assert!(dir.file("theme-assets/bg.png").exists(), "a copy, so the theme keeps working");
    frame(&ui);
    assert!(has_background(&ui));
    c.add_theme_font(&ui, "Mine", &elsewhere.file("Mine.ttf"), true).must("font");
    assert_eq!(c.theme().map(|t| t.font_families.first().cloned()), Some(Some(family.to_owned())));
    assert!(
        c.add_theme_font(&ui, "Bad", &elsewhere.file("bg.png"), false).is_err(),
        "an image is not a font"
    );

    // Exported with its files beside it, it imports whole in another app.
    let out = elsewhere.file("export/mine.json");
    c.export_theme(&out).must("export");
    assert!(
        elsewhere.file("export/mine-assets/bg.png").exists()
            && elsewhere.file("export/mine-assets/Mine.ttf").exists()
    );
    let other_dir = TempDir::new("assets-own-other");
    let other = custom(&other_dir);
    let allow = {
        let mut v_ = ImportPolicy::default();
        v_.allow_assets = true;
        v_
    };
    other.import_theme_file(&window(), &out, allow).must("import the export");
    assert_eq!(other.theme().map(|t| t.font_families), c.theme().map(|t| t.font_families));

    c.remove_theme_font(&ui, "Mine").must("remove font");
    assert!(!dir.file("theme-assets/Mine.ttf").exists(), "a file nothing uses goes");
    c.set_background(&ui, None).must("no background");
    assert!(!dir.file("theme-assets/bg.png").exists());
    frame(&ui);
    assert!(!has_background(&ui));
    assert!(ui.undo_change());
    frame(&ui);
    assert!(has_background(&ui), "undo brings the image back");

    // A theme file naming a file that is not there still starts, with a notice.
    dir.write("theme.json", r#"{ "asset": { "background": "theme-assets/gone.png" } }"#);
    c.reload_all();
    assert!(c.notices().iter().any(|n| n.message.contains("background image")), "{:?}", c.notices());
    frame(&ui);
    assert!(!has_background(&ui));
    c.reset_theme(&ui).must("reset");
    assert!(c.notices().is_empty());
    assert!(
        dir.read("theme.json").is_none()
            && !dir.file("theme-assets").read_dir().is_ok_and(|mut d| d.next().is_some())
    );
}
