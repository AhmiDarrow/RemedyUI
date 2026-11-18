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

/// A kind of customization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
    /// What people call it.
    pub fn name(self) -> &'static str {
        match self {
            Area::Theme => "Theme",
            Area::Shortcuts => "Shortcuts",
            Area::Toolbars => "Toolbars and menus",
            Area::Text => "Text",
            Area::Layouts => "Screen layouts",
        }
    }

    const ALL: [Area; 5] = [Area::Theme, Area::Shortcuts, Area::Toolbars, Area::Text, Area::Layouts];
}

/// A customization that could not be loaded or saved. The app goes on with its defaults
/// for it; [`Customizations::reset_notice`] clears the file in one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Which kind.
    pub area: Area,
    /// The file.
    pub path: PathBuf,
    /// What went wrong, in words for the person.
    pub message: String,
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        write!(f, "{} ({file}): {}", self.area.name(), self.message)
    }
}

/// Why a customization could not be made.
#[derive(Clone, Debug, PartialEq)]
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
    /// Not a tokens, shortcuts or toolbars document, or a value of the wrong kind.
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

impl std::fmt::Display for CustomizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CustomizeError::NotEnabled(a) => write!(f, "{} customization is not on in this app", a.name()),
            CustomizeError::Io { path, detail } => write!(f, "{}: {detail}", path.display()),
            CustomizeError::Invalid(why) => f.write_str(why),
            CustomizeError::Token(e) => write!(f, "{e}"),
            CustomizeError::Clashes(c) => {
                let parts: Vec<String> =
                    c.iter().map(|c| format!("{} is used by {}", c.chord, c.commands.join(", "))).collect();
                f.write_str(&parts.join("; "))
            }
            CustomizeError::Refused(r) => {
                let parts: Vec<String> = r.iter().map(Refusal::to_string).collect();
                write!(f, "refused: {}", parts.join("; "))
            }
            CustomizeError::UnknownCommand(id) => write!(f, "no command {id}"),
            CustomizeError::UnknownSlot(id) => write!(f, "no toolbar {id}"),
        }
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
        self.change(ui, format!("Reset {name}"), None, &[notice.area], |_| {
            files::remove(&path).map_err(|e| CustomizeError::io(&path, &e))
        })
    }

    /// Takes every customization back to the app's defaults, as one undoable change.
    pub fn reset_all(&self, ui: &Ui) -> Result<(), CustomizeError> {
        let areas = self.areas();
        let all: Vec<PathBuf> = areas.iter().flat_map(|a| self.files_of(*a)).collect();
        self.change(ui, "Reset all customizations", None, &areas, |_| {
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
            self.push_notice(Notice {
                area: areas.first().copied().unwrap_or(Area::Theme),
                path: match &e {
                    CustomizeError::Io { path, .. } => path.clone(),
                    _ => self.0.dir.clone(),
                },
                message: format!("could not save: {e}"),
            });
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
            self.push_notice(Notice {
                area: areas.first().copied().unwrap_or(Area::Theme),
                path,
                message: format!("could not restore: {e}"),
            });
        }
    }
}
