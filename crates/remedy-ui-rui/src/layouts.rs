//! People's own layouts: an app that turns overrides on reads a person's version of a
//! screen's `.rui` file from their config folder. A version that does not load falls back to
//! the built-in screen with a notice saying why and a Reset button, so the app always opens.
//! Saving and resetting go through the window's undo history. The same mechanism serves live
//! editing: point the folder at the app's own `ui` sources and edits show as they are saved.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use remedy_ui_core::Insets;
use remedy_ui_view::{El, Ui};

use crate::interp::{Scope, Widgets, build};
use crate::rt::w;
use crate::syntax::{Markup, RuiError};

/// How often a watched layout checks its file.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// Where people's layouts live, and the elements they may use.
#[derive(Clone, Debug)]
pub struct Layouts {
    dir: PathBuf,
    widgets: Widgets,
}

/// Why a layout could not be saved.
#[derive(Debug)]
pub enum SaveError {
    /// It is not valid for this screen.
    Invalid(RuiError),
    /// The file could not be written.
    Io(std::io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Invalid(e) => write!(f, "{e}"),
            SaveError::Io(e) => write!(f, "the layout could not be saved: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl Layouts {
    /// People's layouts in `dir` (the app's user-config folder, or its `ui` sources while
    /// developing), using `widgets`.
    pub fn new(dir: impl Into<PathBuf>, widgets: Widgets) -> Self {
        Layouts { dir: dir.into(), widgets }
    }

    /// Where a person's version of `file` lives.
    pub fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    /// The person's version of `file`, if they have one.
    pub fn read(&self, file: &str) -> Option<String> {
        std::fs::read_to_string(self.path(file)).ok()
    }

    /// Checks `source` as a version of `file` for a screen that exposes `scope`, without
    /// building anything: for editors that say what is wrong as people type.
    pub fn check(&self, file: &str, source: &str, scope: &Scope) -> Result<(), RuiError> {
        crate::interp::check(&Markup::parse(file, source)?, scope, &self.widgets)
    }

    /// Builds the screen `file`: the person's version if they have one that loads, else
    /// `built_in` (the compiled file) with a notice when theirs did not load.
    pub fn build(
        &self,
        ui: &Ui,
        file: &str,
        scope: &Scope,
        built_in: impl Fn(&Ui) -> El + 'static,
    ) -> Layout {
        let host = ui.el("rui_layout").column().fill_width().grow(1.0);
        let layout = Layout(Rc::new(Inner {
            ui: ui.clone(),
            layouts: self.clone(),
            file: file.to_owned(),
            scope: scope.clone(),
            built_in: Rc::new(built_in),
            host,
            error: std::cell::RefCell::new(None),
            seen: std::cell::Cell::new(None),
        }));
        layout.reload();
        layout
    }
}

struct Inner {
    ui: Ui,
    layouts: Layouts,
    file: String,
    scope: Scope,
    built_in: Rc<dyn Fn(&Ui) -> El>,
    host: El,
    error: std::cell::RefCell<Option<RuiError>>,
    /// The file's modification time when last loaded.
    seen: std::cell::Cell<Option<SystemTime>>,
}

/// A screen built from a layout. Cheap to clone.
#[derive(Clone)]
pub struct Layout(Rc<Inner>);

impl std::fmt::Debug for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Layout({})", self.0.file)
    }
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Layout {
    /// The screen's element.
    pub fn el(&self) -> El {
        self.0.host.clone()
    }

    /// Why the person's version did not load, if it did not.
    pub fn error(&self) -> Option<RuiError> {
        self.0.error.borrow().clone()
    }

    /// The person has their own version of the screen in use.
    pub fn is_custom(&self) -> bool {
        self.0.error.borrow().is_none() && self.0.layouts.read(&self.0.file).is_some()
    }

    /// Reads the person's version again and rebuilds the screen. The app's state lives in
    /// its signals, so it carries over.
    pub fn reload(&self) {
        let i = &self.0;
        let path = i.layouts.path(&i.file);
        i.seen.set(modified(&path));
        let ui = &i.ui;
        let (content, error) = match i.layouts.read(&i.file) {
            None => ((i.built_in)(ui), None),
            Some(src) => {
                match Markup::parse(&i.file, &src).and_then(|m| build(ui, &m, &i.scope, &i.layouts.widgets)) {
                    Ok(el) => (el, None),
                    Err(e) => ((i.built_in)(ui), Some(e)),
                }
            }
        };
        i.host.clear();
        if let Some(e) = &error {
            let _ = i.host.clone().child(self.notice(e));
        }
        let _ = i.host.clone().child(content.grow(1.0));
        *i.error.borrow_mut() = error;
    }

    fn notice(&self, e: &RuiError) -> El {
        let ui = &self.0.ui;
        let unit = ui.theme().spacing.unit;
        let me = self.clone();
        let reset = w::button(ui, "Reset", move || {
            let _ = me.reset();
        });
        w::row(ui)
            .variant("notice")
            .gap(unit * 2.0)
            .padding(Insets::all(unit * 2.0))
            .fill_width()
            .child(
                w::text(
                    ui,
                    format!("Your layout for {} could not load, so the original is shown: {e}", self.0.file),
                )
                .grow(1.0),
            )
            .child(reset)
    }

    /// Saves `source` as the person's version and shows it. Refuses a version that does not
    /// load; undoable.
    pub fn save(&self, source: &str) -> Result<(), SaveError> {
        let i = &self.0;
        i.layouts.check(&i.file, source, &i.scope).map_err(SaveError::Invalid)?;
        let before = i.layouts.read(&i.file);
        self.write(Some(source)).map_err(SaveError::Io)?;
        let (undo, redo) = (self.clone(), self.clone());
        let after = source.to_owned();
        i.ui.record_change(
            format!("Change layout {}", i.file),
            None,
            move || {
                let _ = undo.write(before.as_deref());
            },
            move || {
                let _ = redo.write(Some(&after));
            },
        );
        Ok(())
    }

    /// Goes back to the built-in screen, removing the person's version; undoable.
    pub fn reset(&self) -> std::io::Result<()> {
        let i = &self.0;
        let before = i.layouts.read(&i.file);
        if before.is_none() {
            return Ok(());
        }
        self.write(None)?;
        let (undo, redo) = (self.clone(), self.clone());
        i.ui.record_change(
            format!("Reset layout {}", i.file),
            None,
            move || {
                let _ = undo.write(before.as_deref());
            },
            move || {
                let _ = redo.write(None);
            },
        );
        Ok(())
    }

    /// Writes (or with `None` removes) the person's version and rebuilds.
    fn write(&self, source: Option<&str>) -> std::io::Result<()> {
        let path = self.0.layouts.path(&self.0.file);
        match source {
            Some(s) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, s)?;
            }
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            },
        }
        self.reload();
        Ok(())
    }

    /// Rebuilds whenever the file changes on disk (an editor saved it), while the screen is
    /// shown.
    pub fn watch(&self) {
        let me = self.clone();
        self.0.ui.after(WATCH_EVERY, move || {
            // A screen that was closed stops watching.
            if me.0.host.info().is_none() {
                return;
            }
            let now = modified(&me.0.layouts.path(&me.0.file));
            if now != me.0.seen.get() {
                me.reload();
            }
            me.watch();
        });
    }
}
