//! RemedyUI for apps.
//!
//! ```no_run
//! use remedy_ui::{App, AppConfig, WindowSpec};
//!
//! remedy_ui::run(AppConfig::new("Example"), |app: &App| {
//!     app.open(WindowSpec::new("Hello"), |ui, _window| {
//!         ui.root().child(ui.el("label").text("Hello"));
//!     });
//! })
//! .ok();
//! ```
//!
//! [`run`] starts the platform's bridge (Windows, macOS or Linux) and drives every window:
//! input goes to the window's [`Ui`], each frame runs the view pipeline into the window's
//! compositor, and frames are presented with the GPU renderer (feature `gpu`, on by default)
//! or, when no GPU works, the CPU renderer with damage-only presents. App code acts through an
//! [`App`] handle and [`Window`] handles; what they ask happens right after the event that
//! asked. Window geometry can persist under a key, timers run app code later, and the app
//! keeps running with no windows when it says so (tray apps).

mod geometry;
mod instance;
mod runner;

pub use geometry::GeometryStore;
pub use runner::{
    App, AppConfig, GPU_PROMOTE_FRAME_COST, GPU_PROMOTE_FRAME_STALL, RendererChoice, RendererKind, Runner,
    Window, WindowSpec,
};

pub use remedy_ui_core as core;
#[cfg(feature = "devtools")]
pub use remedy_ui_devtools as devtools;
pub use remedy_ui_host as host;
pub use remedy_ui_input as input;
pub use remedy_ui_layout as layout;
pub use remedy_ui_reactive as reactive;
pub use remedy_ui_style as style;
pub use remedy_ui_text as text;
pub use remedy_ui_view as view;
pub use remedy_ui_view::{El, FieldKind, PaintCx, TextField, Ui};

/// Why the app could not run.
#[derive(Debug)]
pub struct RunError(pub String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the app could not run: {}", self.0)
    }
}

impl std::error::Error for RunError {}

/// Runs the app on this platform's bridge until it quits. `setup` runs once the platform is
/// ready, to open the first windows.
pub fn run(config: AppConfig, setup: impl FnOnce(&App) + 'static) -> Result<(), RunError> {
    let inbox = instance::SharedInbox::default();
    if config.single_instance {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if !instance::claim(&config.name, &args, inbox.clone()).map_err(RunError)? {
            // Another instance runs and has our arguments.
            return Ok(());
        }
    }
    let runner = Runner::new(config, setup).with_inbox(inbox);
    #[cfg(windows)]
    {
        remedy_ui_bridge_windows::run(runner).map_err(|e| RunError(e.to_string()))
    }
    #[cfg(target_os = "macos")]
    {
        remedy_ui_bridge_macos::run(runner).map_err(|e| RunError(e.to_string()))
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        remedy_ui_bridge_linux::run(runner).map_err(|e| RunError(e.to_string()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        drop(runner);
        Err(RunError("no bridge for this platform yet".into()))
    }
}

/// Opens a window showing every preview's variations side by side (devtools).
#[cfg(feature = "devtools")]
pub fn show_previews(previews: Vec<remedy_ui_devtools::Preview>) -> Result<(), RunError> {
    let (theme, _) = remedy_ui_devtools::tool_style();
    let config = AppConfig::new("RemedyUI previews").theme(theme, remedy_ui_themes_sheet);
    run(config, move |app| {
        app.open(WindowSpec::new("Previews").size(1100.0, 800.0), move |ui, _| {
            let (theme, sheet) = remedy_ui_devtools::tool_style();
            ui.set_theme(theme, sheet);
            ui.root().child(remedy_ui_devtools::gallery(ui, &previews));
        });
    })
}

#[cfg(feature = "devtools")]
fn remedy_ui_themes_sheet(_: &remedy_ui_style::Theme) -> remedy_ui_style::StyleSheet {
    remedy_ui_devtools::tool_style().1
}
