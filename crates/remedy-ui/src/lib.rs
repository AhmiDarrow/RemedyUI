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
//! [`run`] starts the platform's bridge (Windows, macOS, Linux or Android) and drives every window:
//! input goes to the window's [`Ui`], each frame runs the view pipeline into the window's
//! compositor, and frames are presented with the GPU renderer (feature `gpu`, on by default)
//! or, when no GPU works, the CPU renderer with damage-only presents. App code acts through an
//! [`App`] handle and [`Window`] handles; what they ask happens right after the event that
//! asked. Window geometry can persist under a key, timers run app code later, and the app
//! keeps running with no windows when it says so (tray apps).

#[cfg(feature = "customize")]
mod customizations;
mod geometry;
mod instance;
#[cfg(feature = "widgets")]
mod movable;
mod runner;

pub use geometry::GeometryStore;
#[cfg(feature = "widgets")]
pub use movable::{Movable, movable};
pub use runner::{
    App, AppConfig, Completion, GPU_PROMOTE_FRAME_COST, GPU_PROMOTE_FRAME_STALL, RendererChoice,
    RendererKind, Runner, Window, WindowSpec,
};

/// Agents: assistants read and operate the window under the person's consent.
#[cfg(feature = "agents")]
pub use remedy_ui_automation as agents;
/// The Android bridge: the app's entry point (`remedy_ui::android::main!`), the system log.
#[cfg(target_os = "android")]
pub use remedy_ui_bridge_android as android;
#[cfg(feature = "headless")]
pub use remedy_ui_bridge_headless as headless;
pub use remedy_ui_core as core;
/// What people change about the app: theme, shortcuts, toolbars and menus, and text.
#[cfg(feature = "customize")]
pub use remedy_ui_customize as customize;
#[cfg(feature = "devtools")]
pub use remedy_ui_devtools as devtools;
#[cfg(feature = "code")]
pub use remedy_ui_doc_code as code;
#[cfg(feature = "pages")]
pub use remedy_ui_doc_pages as pages;
#[cfg(feature = "rich")]
pub use remedy_ui_doc_rich as rich;
#[cfg(feature = "gpu-embed")]
pub use remedy_ui_embed_gpu as gpu_embed;
#[cfg(feature = "webview")]
pub use remedy_ui_embed_webview as webview;
pub use remedy_ui_host as host;
pub use remedy_ui_input as input;
pub use remedy_ui_layout as layout;
#[cfg(feature = "anim")]
pub use remedy_ui_media_anim as anim;
#[cfg(feature = "image")]
pub use remedy_ui_media_image as image;
#[cfg(feature = "playback")]
pub use remedy_ui_media_playback as playback;
#[cfg(feature = "svg")]
pub use remedy_ui_media_svg as svg;
#[cfg(feature = "video")]
pub use remedy_ui_media_video as video;
pub use remedy_ui_reactive as reactive;
/// `.rui` markup: screens as data, compiled at build time or loaded for people's own layouts.
#[cfg(feature = "markup")]
pub use remedy_ui_rui as markup;
#[cfg(feature = "i18n")]
pub use remedy_ui_services_i18n as i18n;
#[cfg(feature = "spell")]
pub use remedy_ui_services_spell as spell;
#[cfg(feature = "update")]
pub use remedy_ui_services_update as update;
pub use remedy_ui_style as style;
pub use remedy_ui_text as text;
#[cfg(feature = "widgets")]
pub use remedy_ui_themes as themes;
pub use remedy_ui_view as view;
pub use remedy_ui_view::{El, FieldKind, PaintCx, TextField, Ui};
#[cfg(feature = "viz")]
pub use remedy_ui_viz as viz;
#[cfg(feature = "board")]
pub use remedy_ui_widgets_board as board;
#[cfg(feature = "widgets")]
pub use remedy_ui_widgets_core as widgets;
#[cfg(feature = "data")]
pub use remedy_ui_widgets_data as data;
#[cfg(feature = "extra")]
pub use remedy_ui_widgets_extra as extra;
#[cfg(feature = "forms")]
pub use remedy_ui_widgets_forms as forms;
#[cfg(feature = "music")]
pub use remedy_ui_widgets_music as music;
#[cfg(feature = "nav")]
pub use remedy_ui_widgets_nav as nav;
#[cfg(feature = "term")]
pub use remedy_ui_widgets_term as term;

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
    // SVG files show wherever images do.
    #[cfg(feature = "svg")]
    remedy_ui_media_svg::install();
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
    // The activity comes from the app's `remedy_ui::android::main!` entry point.
    #[cfg(target_os = "android")]
    {
        remedy_ui_bridge_android::run(runner).map_err(|e| RunError(e.to_string()))
    }
    #[cfg(target_os = "ios")]
    {
        drop(runner);
        Err(RunError("no bridge for this platform yet".into()))
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
