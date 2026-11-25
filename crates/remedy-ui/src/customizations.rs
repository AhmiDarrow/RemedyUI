//! Customizations kept in the app's data folder, with the platform's file dialogs.

use remedy_ui_widgets::customize::Customizations;

use crate::App;

impl App {
    /// The person's customizations for this app, kept in a `customizations` folder in the
    /// app's data folder, whose theme import and export use the platform's file dialogs.
    /// Nothing is on until the app turns each kind on ([`Customizations::enable_theme`] and
    /// the rest). Make one when the app starts and share it (it is a cheap handle). `None`
    /// when the platform has no data folder for the app.
    pub fn customizations(&self) -> Option<Customizations> {
        let dir = self.data_dir()?.join("customizations");
        let c = Customizations::new(dir);
        let app = self.clone();
        c.set_file_picker(move |dialog, done| app.file_dialog(None, dialog, done));
        Some(c)
    }
}
