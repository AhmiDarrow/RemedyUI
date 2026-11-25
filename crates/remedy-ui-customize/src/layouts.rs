//! The person's screen layouts: their own versions of the app's `.rui` screens, in the
//! `layouts` folder of the config folder, through [`remedy_ui_rui::Layouts`]. Off until the
//! app turns it on and builds its screens through [`Customizations::layout`].

use std::path::PathBuf;

use remedy_ui_rui::{Layout, Layouts, RuiError, SaveError, Scope, WeakLayout, Widgets};
use remedy_ui_engine::view::{El, Ui};

use crate::{Area, Customizations, CustomizeError, Notice, Words, files, tr_in, tr_now};

/// The layouts folder's name in the config folder.
pub(crate) const LAYOUTS_DIR: &str = "layouts";

/// A screen the app built through the customizations.
#[derive(Clone)]
pub(crate) struct Screen {
    pub(crate) file: String,
    /// The app's own version of the file, shown when the person has none.
    pub(crate) source: String,
    pub(crate) layout: WeakLayout,
}

/// The layouts as customized.
pub(crate) struct LayoutArea {
    pub(crate) layouts: Layouts,
    pub(crate) screens: Vec<Screen>,
}

impl Customizations {
    /// The folder of the person's layouts.
    pub fn layouts_dir(&self) -> PathBuf {
        self.0.dir.join(LAYOUTS_DIR)
    }

    /// Lets the person rearrange the app's `.rui` screens with `widgets` (the built-in
    /// elements and the app's own). Build each screen with [`Customizations::layout`].
    pub fn enable_layouts(&self, widgets: Widgets) {
        let screens = self.0.layouts.borrow_mut().take().map(|a| a.screens).unwrap_or_default();
        *self.0.layouts.borrow_mut() =
            Some(LayoutArea { layouts: Layouts::new(self.layouts_dir(), widgets), screens });
        self.reload(&[Area::Layouts]);
    }

    /// The layouts, while they are on.
    pub fn layouts(&self) -> Option<Layouts> {
        self.0.layouts.borrow().as_ref().map(|a| a.layouts.clone())
    }

    /// Builds the screen `file` (`settings_voice.rui`): the person's version when they have
    /// one that loads, else `built_in` (the compiled file), whose text is `built_in_source`
    /// (the compiled module's `SOURCE`, what the layout editor starts from). A version that
    /// does not load joins the notices, with the one-click reset. `None` while layouts are
    /// not on.
    pub fn layout(
        &self,
        ui: &Ui,
        file: &str,
        scope: &Scope,
        built_in_source: &str,
        built_in: impl Fn(&Ui) -> El + 'static,
    ) -> Option<Layout> {
        let layouts = self.layouts()?;
        let layout = layouts.build(ui, file, scope, built_in);
        let me = std::rc::Rc::downgrade(&self.0);
        layout.on_load(move |l| {
            if let Some(inner) = me.upgrade() {
                let c = Customizations(inner);
                c.layout_notice(l);
                c.0.revision.update(|r| *r += 1);
            }
        });
        if let Some(a) = self.0.layouts.borrow_mut().as_mut() {
            a.screens.retain(|s| s.file != file && s.layout.upgrade().is_some());
            a.screens.push(Screen {
                file: file.to_owned(),
                source: built_in_source.to_owned(),
                layout: layout.downgrade(),
            });
        }
        self.layout_notice(&layout);
        self.0.revision.update(|r| *r += 1);
        Some(layout)
    }

    /// The screens built through [`Customizations::layout`] that are still shown.
    pub fn layout_files(&self) -> Vec<String> {
        self.live_screens().into_iter().map(|(s, _)| s.file).collect()
    }

    fn live_screens(&self) -> Vec<(Screen, Layout)> {
        self.0
            .layouts
            .borrow()
            .as_ref()
            .map(|a| a.screens.iter().filter_map(|s| s.layout.upgrade().map(|l| (s.clone(), l))).collect())
            .unwrap_or_default()
    }

    fn screen(&self, file: &str) -> Result<(Screen, Layout), CustomizeError> {
        if self.0.layouts.borrow().is_none() {
            return Err(CustomizeError::NotEnabled(Area::Layouts));
        }
        self.live_screens()
            .into_iter()
            .find(|(s, _)| s.file == file)
            .ok_or_else(|| CustomizeError::Invalid(tr_in("en", "no-screen", &[("file", file.into())])))
    }

    /// The text of screen `file` as it stands: the person's version, else the app's (tracked).
    pub fn layout_source(&self, file: &str) -> Option<String> {
        self.0.revision.get();
        let (screen, layout) = self.screen(file).ok()?;
        Some(layout.layouts().read(file).unwrap_or(screen.source))
    }

    /// Whether the person has their own version of `file` (tracked).
    pub fn is_layout_customized(&self, file: &str) -> bool {
        self.0.revision.get();
        self.layouts().is_some_and(|l| l.read(file).is_some())
    }

    /// Checks `source` as a version of screen `file`, without changing anything: what is
    /// wrong, at which line and column.
    pub fn check_layout(&self, file: &str, source: &str) -> Result<(), CustomizeError> {
        let (_, layout) = self.screen(file)?;
        layout
            .layouts()
            .check(file, source, layout.scope())
            .map_err(|e: RuiError| CustomizeError::Invalid(e.to_string()))
    }

    /// Saves `source` as the person's version of screen `file` and shows it, through
    /// [`Layout::save`] (refused when it does not load; undoable in the screen's window).
    pub fn save_layout(&self, file: &str, source: &str) -> Result<(), CustomizeError> {
        let (_, layout) = self.screen(file)?;
        layout.save(source).map_err(|e| match e {
            SaveError::Invalid(e) => CustomizeError::Invalid(e.to_string()),
            SaveError::Io(e) => CustomizeError::io(&layout.path(), &e),
            other => CustomizeError::Invalid(other.to_string()),
        })
    }

    /// Goes back to the app's version of screen `file` (undoable).
    pub fn reset_layout(&self, ui: &Ui, file: &str) -> Result<(), CustomizeError> {
        let (_, layout) = self.screen(file)?;
        let path = layout.path();
        if !path.exists() {
            return Ok(());
        }
        let label = tr_now(ui, "reset-layout-named", &[("file", file.into())]);
        self.change(ui, label, None, &[Area::Layouts], |_| {
            files::remove(&path).map_err(|e| CustomizeError::io(&path, &e))
        })
    }

    /// Goes back to the app's version of every screen (undoable).
    pub fn reset_layouts(&self, ui: &Ui) -> Result<(), CustomizeError> {
        if self.0.layouts.borrow().is_none() {
            return Err(CustomizeError::NotEnabled(Area::Layouts));
        }
        let all = self.files_of(Area::Layouts);
        self.change(ui, tr_now(ui, "reset-all-layouts", &[]), None, &[Area::Layouts], |_| {
            for p in &all {
                files::remove(p).map_err(|e| CustomizeError::io(p, &e))?;
            }
            Ok(())
        })
    }

    /// The notice for a screen's last load.
    fn layout_notice(&self, layout: &Layout) {
        let path = layout.path();
        let notices = layout
            .error()
            .map(|e| Notice::new(Area::Layouts, path.clone(), Words::text(e.to_string())))
            .into_iter()
            .collect();
        self.set_notices(Area::Layouts, Some(&path), notices);
    }

    /// Loads every shown screen again from its file.
    pub(crate) fn reload_layouts(&self) {
        if let Some(a) = self.0.layouts.borrow_mut().as_mut() {
            a.screens.retain(|s| s.layout.upgrade().is_some());
        }
        for (_, layout) in self.live_screens() {
            layout.reload();
        }
    }
}
