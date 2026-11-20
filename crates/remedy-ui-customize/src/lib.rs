//! The code-fluid layer: what the person using a RemedyUI app can change about it, turned on
//! by the app with no further code (see `docs/design/07-styling-and-customization.md`).
//!
//! ```no_run
//! # use remedy_ui_customize::Customizations;
//! # fn setup(ui: &remedy_ui_view::Ui, commands: &remedy_ui_widgets_nav::Commands) {
//! let custom = Customizations::new("/path/to/the/app/config");
//! custom.enable_theme(ui.theme());
//! custom.enable_shortcuts(commands);
//! custom.enable_toolbars(commands);
//! custom.attach(ui);
//! let main = custom.toolbar_slot(ui, "main", "Main toolbar", &["file.save", "edit.undo"]);
//! // …and a settings page with every editor:
//! let page = remedy_ui_customize::settings_page(ui, &custom);
//! # let _ = (main, page);
//! # }
//! ```
//!
//! | Kind | Stored as (in the config folder) | Editor |
//! |---|---|---|
//! | Theme | `theme.json`: W3C design tokens, only those changed | [`theme_editor`] |
//! | Shortcuts | `shortcuts.json`: command id → chords (`"Ctrl+Shift+Z"`) | [`shortcut_editor`] |
//! | Toolbars and menus | `toolbars.json`: slot → command ids | [`toolbars_editor`] |
//! | Text | `text/<locale>.json` (catalogs) and `text/<locale>.ftl` (Fluent) | [`text_editor`] |
//! | Screen layouts | `layouts/<screen>.rui` | [`layouts_editor`] |
//!
//! A theme's files (a background image, fonts) are copied into `theme-assets/`.
//!
//! - **Files people can read.** Every file is plain, pretty-printed JSON (or Fluent) that can
//!   be edited by hand and diffed, and holds only what the person changed. Files are written
//!   atomically: a temporary file, then a rename.
//! - **Undoable.** Every change goes through the window's undo history
//!   ([`Ui::record_change`]); a run of changes to one value (a color dragged across its
//!   picker) is one entry. "Reset to default" exists for each item, each kind, and
//!   everything ([`Customizations::reset_all`]), and is undoable too.
//! - **The app always starts.** A file that does not load is skipped: the app's defaults
//!   apply, and a [`Notice`] says what went wrong, with a one-click reset
//!   ([`Customizations::reset_notice`], shown by [`notices_view`]).
//! - **Shared themes are untrusted.** [`Customizations::import_theme`] parses a theme from
//!   someone else as data: shaders and external links are refused unless the person turns
//!   them on ([`ImportPolicy`]), and every token is checked before anything changes.

mod assets;
mod files;
mod layouts;
mod page;
mod shortcut_editor;
mod shortcuts;
mod text;
mod theme;
mod theme_editor;
mod toolbars;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use remedy_ui_host::FileDialog;
use remedy_ui_input::Clash;
use remedy_ui_reactive::{Scope, Signal};
use remedy_ui_view::{Ui, WeakUi};

pub use page::{layouts_editor, notices_view, settings_page, text_editor, toolbars_editor};
pub use shortcut_editor::shortcut_editor;
pub use theme::{
    COLOR_ROLES, ContrastWarning, ImportPolicy, ImportReport, Refusal, RefusalKind, Token, TokenKind,
    TokenValue, contrast_warnings, density_name, inspect_theme,
};
pub use theme_editor::theme_editor;

use remedy_ui_view::strings::StrArg;

/// The Customize page's own words (headings, buttons, notices, undo history names, design
/// token names) in every language the engine ships, from `i18n/*.ftl`. Apps reword them, or
/// add a language, with [`remedy_ui_view::strings::add_messages`] under the name `customize`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("customize");

/// Message `id` in the window's locale (tracked: inside a property closure it follows the
/// locale).
pub(crate) fn tr(ui: &Ui, id: &str) -> String {
    tidy(STRINGS.get(ui, id))
}

/// Message `id` with placeholder values in the window's locale (tracked).
pub(crate) fn trf(ui: &Ui, id: &str, args: &[(&str, StrArg)]) -> String {
    tidy(STRINGS.format(ui, id, args))
}

/// The window's locale now (not tracked).
pub(crate) fn locale_of(ui: &Ui) -> String {
    remedy_ui_reactive::untrack(|| ui.locale())
}

/// Message `id` with placeholder values in the window's locale now (not tracked: for undo
/// history names, errors and status lines, said once).
pub(crate) fn tr_now(ui: &Ui, id: &str, args: &[(&str, StrArg)]) -> String {
    tr_in(&locale_of(ui), id, args)
}

/// Message `id` with placeholder values in `locale` (not tracked).
pub(crate) fn tr_in(locale: &str, id: &str, args: &[(&str, StrArg)]) -> String {
    tidy(STRINGS.format_in(locale, id, args))
}

/// Whether `c` is written right to left (Hebrew, Arabic and the scripts near them).
fn is_rtl(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
        || matches!(c, '\u{10800}'..='\u{10FFF}' | '\u{1E800}'..='\u{1EFFF}')
}

/// A formatted message without the isolation marks around its placeholders when nothing in
/// it is written right to left: they only matter where directions mix, and plain text
/// stays plain (for names compared, searched and logged).
fn tidy(s: String) -> String {
    if s.chars().any(is_rtl) { s } else { s.replace(['\u{2068}', '\u{2069}'], "") }
}

/// `items` as a list in `locale`: "a", "a and b", "a, b and c".
pub(crate) fn list_in(locale: &str, items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => tr_in(locale, "list-two", &[("first", first.into()), ("second", second.into())]),
        [rest @ .., before, last] => {
            let mut out = tr_in(locale, "list-end", &[("first", before.into()), ("last", last.into())]);
            for item in rest.iter().rev() {
                out = tr_in(locale, "list-middle", &[("first", item.into()), ("rest", out.into())]);
            }
            out
        }
    }
}

/// Whole sentences about separate things, joined in `locale` ("…; …").
pub(crate) fn clauses_in(locale: &str, items: &[String]) -> String {
    let mut it = items.iter().rev();
    let Some(last) = it.next() else { return String::new() };
    it.fold(last.clone(), |rest, first| {
        tr_in(locale, "clauses", &[("first", first.into()), ("rest", rest.into())])
    })
}

/// Words for the person, in whatever language they are shown in: a message of this crate's
/// catalog with its values (which may be words themselves), or text shown as it is (what
/// a system or a parser said).
#[derive(Clone)]
pub(crate) struct Words(Rc<dyn Fn(&str) -> String>);

/// A placeholder value of [`Words::msg`].
pub(crate) enum Part {
    Arg(StrArg),
    Words(Words),
}

impl From<&str> for Part {
    fn from(s: &str) -> Self {
        Part::Arg(s.into())
    }
}

impl From<String> for Part {
    fn from(s: String) -> Self {
        Part::Arg(s.into())
    }
}

impl From<&String> for Part {
    fn from(s: &String) -> Self {
        Part::Arg(s.into())
    }
}

impl From<usize> for Part {
    fn from(n: usize) -> Self {
        Part::Arg(n.into())
    }
}

impl From<Words> for Part {
    fn from(w: Words) -> Self {
        Part::Words(w)
    }
}

impl Words {
    /// Message `id` of this crate's catalog with `args`.
    pub(crate) fn msg(id: &'static str, args: Vec<(&'static str, Part)>) -> Words {
        Words(Rc::new(move |locale| {
            let values: Vec<(&str, StrArg)> = args
                .iter()
                .map(|(k, v)| {
                    (
                        *k,
                        match v {
                            Part::Arg(a) => a.clone(),
                            Part::Words(w) => StrArg::from(w.in_locale(locale)),
                        },
                    )
                })
                .collect();
            tr_in(locale, id, &values)
        }))
    }

    /// Text shown as it is, in every language (what a system or a parser said).
    pub(crate) fn text(s: impl Into<String>) -> Words {
        let s: String = s.into();
        Words(Rc::new(move |_| s.clone()))
    }

    /// Words made by `f` for a locale.
    pub(crate) fn new(f: impl Fn(&str) -> String + 'static) -> Words {
        Words(Rc::new(f))
    }

    /// The words in `locale`.
    pub(crate) fn in_locale(&self, locale: &str) -> String {
        (self.0)(locale)
    }

    /// The words in English, for logs, `Display` and the English fields of the API.
    pub(crate) fn english(&self) -> String {
        self.in_locale("en")
    }
}

impl std::fmt::Debug for Words {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.english())
    }
}

impl PartialEq for Words {
    fn eq(&self, other: &Words) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || self.english() == other.english()
    }
}

impl Eq for Words {}

/// A kind of customization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Area {
    /// Colors, fonts, sizes, spacing, radii, motion and density.
    Theme,
    /// Keyboard shortcuts.
    Shortcuts,
    /// Toolbars and menus.
    Toolbars,
    /// The app's text.
    Text,
    /// The person's own versions of the app's `.rui` screens.
    Layouts,
}

impl Area {
    /// What people call it, in English ([`Area::localized_name`] in the window's language).
    pub fn name(self) -> &'static str {
        match self {
            Area::Theme => "Theme",
            Area::Shortcuts => "Shortcuts",
            Area::Toolbars => "Toolbars and menus",
            Area::Text => "Text",
            Area::Layouts => "Screen layouts",
        }
    }

    /// What people call it in the window's language (tracked: inside a property closure it
    /// follows the locale).
    pub fn localized_name(self, ui: &Ui) -> String {
        tr(ui, self.message_id())
    }

    fn message_id(self) -> &'static str {
        match self {
            Area::Theme => "area-theme",
            Area::Shortcuts => "area-shortcuts",
            Area::Toolbars => "area-toolbars",
            Area::Text => "area-text",
            Area::Layouts => "area-layouts",
        }
    }

    const ALL: [Area; 5] = [Area::Theme, Area::Shortcuts, Area::Toolbars, Area::Text, Area::Layouts];
}

/// A customization that could not be loaded or saved. The app goes on with its defaults
/// for it; [`Customizations::reset_notice`] clears the file in one step.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notice {
    /// Which kind.
    pub area: Area,
    /// The file.
    pub path: PathBuf,
    /// What went wrong, in words for the person, in English
    /// ([`Notice::localized_message`] in the window's language).
    pub message: String,
    /// The same, in any language.
    words: Words,
}

impl Notice {
    /// A notice saying `words`.
    pub(crate) fn new(area: Area, path: PathBuf, words: Words) -> Notice {
        Notice { area, path, message: words.english(), words }
    }

    /// What went wrong in the window's language (tracked: inside a property closure it
    /// follows the locale).
    pub fn localized_message(&self, ui: &Ui) -> String {
        self.words.in_locale(&ui.locale())
    }
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        write!(f, "{} ({file}): {}", self.area.name(), self.message)
    }
}

/// Why a customization could not be made.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CustomizeError {
    /// The app has not turned this kind on.
    NotEnabled(Area),
    /// A file could not be read or written.
    Io {
        /// The file.
        path: PathBuf,
        /// What the system said.
        detail: String,
    },
    /// Not a tokens, shortcuts or toolbars document, or a value of the wrong kind. Said in the
    /// language of the window the change was made in, with what a parser said as it said it.
    Invalid(String),
    /// A token's value is not what its place needs.
    Token(remedy_ui_themes::TokenError),
    /// Shortcuts other commands already answer to.
    Clashes(Vec<Clash>),
    /// Parts of a shared theme its import policy refuses.
    Refused(Vec<Refusal>),
    /// No command has this id.
    UnknownCommand(String),
    /// No toolbar slot has this id.
    UnknownSlot(String),
}

impl CustomizeError {
    pub(crate) fn io(path: &Path, e: &std::io::Error) -> CustomizeError {
        CustomizeError::Io { path: path.to_owned(), detail: e.to_string() }
    }
}

impl CustomizeError {
    /// What went wrong, in the window's language (tracked: inside a property closure it
    /// follows the locale). `Display` says it in English.
    pub fn localized(&self, ui: &Ui) -> String {
        self.message_in(&ui.locale())
    }

    /// What went wrong, in `locale`.
    pub(crate) fn message_in(&self, locale: &str) -> String {
        match self {
            CustomizeError::NotEnabled(a) => tr_in(
                locale,
                match a {
                    Area::Theme => "error-not-enabled-theme",
                    Area::Shortcuts => "error-not-enabled-shortcuts",
                    Area::Toolbars => "error-not-enabled-toolbars",
                    Area::Text => "error-not-enabled-text",
                    Area::Layouts => "error-not-enabled-layouts",
                },
                &[],
            ),
            CustomizeError::Io { path, detail } => tr_in(
                locale,
                "error-io",
                &[("path", path.display().to_string().into()), ("detail", detail.into())],
            ),
            CustomizeError::Invalid(why) => why.clone(),
            CustomizeError::Token(e) => e.to_string(),
            CustomizeError::Clashes(c) => {
                let parts: Vec<String> = c
                    .iter()
                    .map(|c| {
                        let chord = ("chord", StrArg::from(c.chord.to_string()));
                        let commands = ("commands", StrArg::from(list_in(locale, &c.commands)));
                        tr_in(locale, "error-clash", &[chord, commands])
                    })
                    .collect();
                clauses_in(locale, &parts)
            }
            CustomizeError::Refused(r) => {
                let parts: Vec<String> = r.iter().map(|r| r.message_in(locale)).collect();
                tr_in(locale, "error-refused", &[("parts", clauses_in(locale, &parts).into())])
            }
            CustomizeError::UnknownCommand(id) => {
                tr_in(locale, "error-unknown-command", &[("id", id.into())])
            }
            CustomizeError::UnknownSlot(id) => tr_in(locale, "error-unknown-slot", &[("id", id.into())]),
        }
    }

    /// The error as words in any language (for a notice).
    pub(crate) fn words(&self) -> Words {
        let e = self.clone();
        Words::new(move |l| e.message_in(l))
    }
}

impl std::fmt::Display for CustomizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message_in("en"))
    }
}

impl std::error::Error for CustomizeError {}

/// Shows a native file dialog, then calls back with the chosen paths (empty: cancelled).
type Picker = Rc<dyn Fn(FileDialog, Box<dyn FnOnce(Vec<PathBuf>)>)>;

pub(crate) struct Inner {
    dir: PathBuf,
    /// Owns the signals and watchers made here, so they live as long as this does rather
    /// than as long as whatever was running when they were made.
    owner: Scope,
    windows: RefCell<Vec<WeakUi>>,
    notices: Signal<Vec<Notice>>,
    /// Bumped after every load: editors follow it.
    revision: Signal<u64>,
    picker: RefCell<Option<Picker>>,
    theme: RefCell<Option<theme::ThemeArea>>,
    shortcuts: RefCell<Option<remedy_ui_widgets_nav::Commands>>,
    toolbars: RefCell<Option<toolbars::ToolbarArea>>,
    text: RefCell<Option<text::TextArea>>,
    layouts: RefCell<Option<layouts::LayoutArea>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Watchers first, then everything else the scope owns.
        drop(self.toolbars.borrow_mut().take());
        self.owner.dispose();
    }
}

/// What the person can change about the app, stored in its config folder. Cheap to clone (a
/// shared handle); make one per app.
#[derive(Clone)]
pub struct Customizations(pub(crate) Rc<Inner>);

impl std::fmt::Debug for Customizations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Customizations({})", self.0.dir.display())
    }
}

impl Customizations {
    /// Customizations kept in `config_dir`, the app's user-config folder (created on the
    /// first save). Nothing is on until the app turns each kind on.
    pub fn new(config_dir: impl Into<PathBuf>) -> Customizations {
        let ((notices, revision), owner) = Scope::new(|| (Signal::new(Vec::new()), Signal::new(0u64)));
        Customizations(Rc::new(Inner {
            dir: config_dir.into(),
            owner,
            windows: RefCell::new(Vec::new()),
            notices,
            revision,
            picker: RefCell::new(None),
            theme: RefCell::new(None),
            shortcuts: RefCell::new(None),
            toolbars: RefCell::new(None),
            text: RefCell::new(None),
            layouts: RefCell::new(None),
        }))
    }

    /// The config folder.
    pub fn dir(&self) -> PathBuf {
        self.0.dir.clone()
    }

    /// Styles a window with the person's theme, now and after every change.
    pub fn attach(&self, ui: &Ui) {
        let mut w = self.0.windows.borrow_mut();
        w.retain(|w| w.upgrade().is_some());
        if !w.iter().any(|w| w.upgrade().is_some_and(|u| u.is_same(ui))) {
            w.push(ui.downgrade());
        }
        drop(w);
        let current = self.0.theme.borrow().as_ref().and_then(|a| a.current.get_untracked().ok());
        if let Some(t) = current {
            self.apply_theme_to_windows(&t);
        }
    }

    /// How the editors ask for files to import from and export to: the app's native file
    /// dialog (`remedy_ui::App::file_dialog`). Without one, they offer a path field.
    pub fn set_file_picker(&self, pick: impl Fn(FileDialog, Box<dyn FnOnce(Vec<PathBuf>)>) + 'static) {
        *self.0.picker.borrow_mut() = Some(Rc::new(pick));
    }

    /// Whether a file picker was given.
    pub fn has_file_picker(&self) -> bool {
        self.0.picker.borrow().is_some()
    }

    pub(crate) fn pick(&self, dialog: FileDialog, done: impl FnOnce(Vec<PathBuf>) + 'static) {
        let picker = self.0.picker.borrow().clone();
        if let Some(p) = picker {
            p(dialog, Box::new(done));
        }
    }

    /// The kinds the app turned on.
    pub fn areas(&self) -> Vec<Area> {
        Area::ALL.into_iter().filter(|a| self.is_enabled(*a)).collect()
    }

    /// Whether the app turned `area` on.
    pub fn is_enabled(&self, area: Area) -> bool {
        match area {
            Area::Theme => self.0.theme.borrow().is_some(),
            Area::Shortcuts => self.0.shortcuts.borrow().is_some(),
            Area::Toolbars => self.0.toolbars.borrow().is_some(),
            Area::Text => self.0.text.borrow().is_some(),
            Area::Layouts => self.0.layouts.borrow().is_some(),
        }
    }

    /// What could not be loaded or saved (tracked).
    pub fn notices(&self) -> Vec<Notice> {
        self.0.notices.try_get().unwrap_or_default()
    }

    /// Hides a notice without changing anything (it comes back if the file still fails on
    /// the next load).
    pub fn dismiss(&self, notice: &Notice) {
        self.0.notices.update(|n| n.retain(|x| x != notice));
    }

    /// The one-click reset for a notice: the failing file goes, so its kind is back to the
    /// app's defaults (undoable, so the file can be had back).
    pub fn reset_notice(&self, ui: &Ui, notice: &Notice) -> Result<(), CustomizeError> {
        let path = notice.path.clone();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.change(ui, tr_now(ui, "reset-named", &[("name", name.into())]), None, &[notice.area], |_| {
            files::remove(&path).map_err(|e| CustomizeError::io(&path, &e))
        })
    }

    /// Takes every customization back to the app's defaults, as one undoable change.
    pub fn reset_all(&self, ui: &Ui) -> Result<(), CustomizeError> {
        let areas = self.areas();
        let all: Vec<PathBuf> = areas.iter().flat_map(|a| self.files_of(*a)).collect();
        self.change(ui, tr_now(ui, "reset-all-customizations", &[]), None, &areas, |_| {
            for p in &all {
                files::remove(p).map_err(|e| CustomizeError::io(p, &e))?;
            }
            Ok(())
        })
    }

    /// Reads every file again (after they were edited by hand) and applies them.
    pub fn reload_all(&self) {
        self.reload(&self.areas());
    }

    /// Runs `f` with this as the owner of the signals and watchers it makes.
    pub(crate) fn in_owner<R>(&self, f: impl FnOnce() -> R) -> R {
        let mut f = Some(f);
        if let Ok(Some(r)) = self.0.owner.run(|| f.take().map(|f| f())) {
            return r;
        }
        // The owner was disposed (its runtime is going away): what `f` makes is unowned.
        match f.take() {
            Some(f) => f(),
            None => unreachable!("the owner ran the closure, so it returned its result"),
        }
    }

    /// The files `area` keeps now.
    pub(crate) fn files_of(&self, area: Area) -> Vec<PathBuf> {
        match area {
            Area::Theme => {
                let mut v = vec![self.theme_file()];
                v.extend(files::list_all(&self.theme_assets_dir()));
                v
            }
            Area::Shortcuts => vec![self.shortcuts_file()],
            Area::Toolbars => vec![self.toolbars_file()],
            Area::Text => files::list(&self.text_dir(), &["json", "ftl"]),
            Area::Layouts => files::list_tree(&self.layouts_dir(), &["rui"]),
        }
    }

    /// Replaces the notices of `area` (about `path` only, when given).
    pub(crate) fn set_notices(&self, area: Area, path: Option<&Path>, list: Vec<Notice>) {
        self.0.notices.update(|n| {
            n.retain(|x| x.area != area || path.is_some_and(|p| x.path != p));
            n.extend(list);
        });
    }

    pub(crate) fn push_notice(&self, notice: Notice) {
        self.0.notices.update(|n| {
            if !n.contains(&notice) {
                n.push(notice);
            }
        });
    }

    /// Loads `areas` from their files and applies them.
    pub(crate) fn reload(&self, areas: &[Area]) {
        for a in areas {
            match a {
                Area::Theme => self.reload_theme(),
                Area::Shortcuts => self.reload_shortcuts(),
                Area::Toolbars => self.reload_toolbars(),
                Area::Text => self.reload_text_area(),
                Area::Layouts => self.reload_layouts(),
            }
        }
        self.0.revision.update(|r| *r += 1);
    }

    /// Makes one change to the files of `areas` (`write`), loads them, and records it in
    /// `ui`'s history: undo puts the files back as they were and loads them, redo puts them
    /// as `write` left them. With `merge`, consecutive changes with that key are one entry.
    pub(crate) fn change(
        &self,
        ui: &Ui,
        label: impl Into<String>,
        merge: Option<String>,
        areas: &[Area],
        write: impl FnOnce(&Customizations) -> Result<(), CustomizeError>,
    ) -> Result<(), CustomizeError> {
        let mut paths: Vec<PathBuf> = areas.iter().flat_map(|a| self.files_of(*a)).collect();
        let before_paths = paths.clone();
        let mut before = files::snapshot(&before_paths);
        let written = write(self);
        // Files the change made (a new locale's text) were absent before it.
        for p in areas.iter().flat_map(|a| self.files_of(*a)) {
            if !paths.contains(&p) {
                before.push((p.clone(), None));
                paths.push(p);
            }
        }
        let after = files::snapshot(&paths);
        self.reload(areas);
        if let Err(e) = written {
            self.push_notice(Notice::new(
                areas.first().copied().unwrap_or(Area::Theme),
                match &e {
                    CustomizeError::Io { path, .. } => path.clone(),
                    _ => self.0.dir.clone(),
                },
                Words::msg("notice-could-not-save", vec![("error", e.words().into())]),
            ));
            return Err(e);
        }
        if before == after {
            return Ok(());
        }
        let areas: Rc<[Area]> = areas.into();
        let (me1, me2, a1, a2) = (Rc::downgrade(&self.0), Rc::downgrade(&self.0), areas.clone(), areas);
        let undo = move || {
            if let Some(inner) = me1.upgrade() {
                Customizations(inner).put_back(&before, &a1);
            }
        };
        let redo = move || {
            if let Some(inner) = me2.upgrade() {
                Customizations(inner).put_back(&after, &a2);
            }
        };
        match merge {
            Some(key) => ui.record_merged_change(label, None, key, undo, redo),
            None => ui.record_change(label, None, undo, redo),
        }
        Ok(())
    }

    /// Restores files for undo or redo and loads them.
    fn put_back(&self, set: &files::FileSet, areas: &[Area]) {
        let failed = files::restore(set);
        self.reload(areas);
        if let Err((path, e)) = failed {
            self.push_notice(Notice::new(
                areas.first().copied().unwrap_or(Area::Theme),
                path,
                Words::msg("notice-could-not-restore", vec![("detail", e.to_string().into())]),
            ));
        }
    }
}
