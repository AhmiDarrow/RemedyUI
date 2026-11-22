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
//! [`run`] starts the platform's bridge (Windows, macOS, Linux, Android, or a browser page with
//! the `web` feature, where it returns once the app has started) and drives every window:
//! input goes to the window's [`Ui`], each frame runs the view pipeline into the window's
//! compositor, and frames are presented with the GPU renderer (feature `gpu`, on by default)
//! or, when no GPU works, the CPU renderer with damage-only presents. App code acts through an
//! [`App`] handle and [`Window`] handles; what they ask happens right after the event that
//! asked. Window geometry can persist under a key, timers run app code later, and the app
//! keeps running with no windows when it says so (tray apps).
//!
//! The [`guide`] holds the guides, porting guides and recipes, starting with
//! [`guide::getting_started`].

#[cfg(feature = "customize")]
mod customizations;
mod geometry;
pub mod guide;
mod instance;

#[cfg(feature = "widgets")]
mod movable;
#[cfg(feature = "widgets")]
pub mod router;
mod runner;
#[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
mod web_fonts;
/// Driving an app in tests on the in-memory host: find by role and name, press, type.
#[cfg(all(feature = "headless", feature = "widgets"))]
pub mod testing;

// The README's code is compiled with the doctests, so it cannot drift from the API either.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeDoctests;

/// The runner's own words (what the place of content popped out to its own window says) in
/// every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a language,
/// with [`remedy_ui_view::strings::add_messages`] under the name `remedy-ui`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("remedy-ui");

pub use geometry::{GEOMETRY_KEY, GeometryStore};
#[cfg(feature = "widgets")]
pub use movable::{Movable, movable};
/// Editions: which default behaviors the app follows ([`AppConfig::edition`]).
pub use remedy_ui_core::Edition;
pub use runner::{
    App, AppConfig, BackgroundJob, Cancelled, Completion, GPU_PROMOTE_FRAME_COST, GPU_PROMOTE_FRAME_STALL,
    GraphicsAdapter, RendererChoice, RendererKind, Runner, Window, WindowSpec, bundled_fonts,
};

/// Semantics: roles, names, states and actions that screen readers, tests and agents read
/// (`Semantics`, `Role`, `Action`).
pub use remedy_ui_a11y as a11y;
/// Agents: assistants read and operate the window under the person's consent.
#[cfg(feature = "agents")]
pub use remedy_ui_automation as agents;
/// The Android bridge: the app's entry point (`remedy_ui::android::main!`), the system log.
#[cfg(target_os = "android")]
pub use remedy_ui_bridge_android as android;
#[cfg(feature = "headless")]
pub use remedy_ui_bridge_headless as headless;
/// The web bridge: where the app shows in the page ([`web::WebConfig`], [`web::Mount`], set
/// with [`AppConfig::web`]), and the page's files (picked, dropped, downloaded). Its types
/// are on every target, so app code sets them without `cfg`; starting an app in a page is
/// [`run`]'s, on `wasm32-unknown-unknown`.
#[cfg(feature = "web")]
pub use remedy_ui_bridge_web as web;
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
/// Prism, the effects system (feature `effects`): the `.rfx` language and its compiler
/// ([`effects::Effect::compile`]), the built-in effect library ([`effects::library`]), and
/// effects as elements ([`effects::effect_view`]) drawn by the GPU kernel or the CPU renderer.
#[cfg(feature = "effects")]
pub mod effects {
    pub use remedy_ui_effects::*;
    pub use remedy_ui_media_effects::*;

    /// What the `effect` markup element takes ([`register_markup`]): `name`, a built-in effect
    /// ([`LIBRARY`]); and, optionally, `param`, one of its parameters, driven live by `value`,
    /// a number signal the app exposes.
    #[cfg(feature = "markup")]
    pub const MARKUP_PARAMS: &[crate::markup::spec::Param] = &[
        crate::markup::spec::Param { name: "name", kind: crate::markup::spec::Kind::Word, required: true },
        crate::markup::spec::Param { name: "param", kind: crate::markup::spec::Kind::Word, required: false },
        crate::markup::spec::Param {
            name: "value",
            kind: crate::markup::spec::Kind::Number,
            required: false,
        },
    ];

    /// Builds the `effect` markup element from its call: the built-in effect `name` in an
    /// element, under the window's ambient policy, with what the file put inside it drawn over
    /// it. A name that is not a built-in effect draws nothing. Markup is data from anyone, so
    /// only built-in effects are offered: no effect source reaches the compiler from a file.
    /// For build-time markup, pass it as the custom element's path.
    #[cfg(feature = "markup")]
    pub fn markup_element(ui: &crate::Ui, call: crate::markup::Call) -> crate::El {
        let effect = call.label("name").and_then(library);
        let mut el = match effect {
            Some(e) => {
                let (el, view) = effect_view(ui, Instance::new(e));
                match (call.label("param").map(str::to_owned), call.number("value")) {
                    (Some(param), Some(value)) => el.bind_named("effect value", move || {
                        let _ = view.set(&param, value.get() as f32);
                    }),
                    _ => el,
                }
            }
            None => ui.el("effect"),
        };
        for c in call.children {
            el = el.child(c);
        }
        el
    }

    /// Adds the `effect` element to what `.rui` files may use:
    /// `effect(name: "fire", param: "intensity", value: $heat) { … }`.
    ///
    /// ```
    /// use remedy_ui::markup::{Markup, Scope, Widgets, build_markup};
    /// let mut widgets = Widgets::new();
    /// remedy_ui::effects::register_markup(&mut widgets);
    /// let m = Markup::parse("hero.rui", r#"effect(name: "aurora") { text(text: "Hello") }"#)
    ///     .map_err(|e| e.to_string())?;
    /// assert!(remedy_ui::markup::check(&m, &Scope::new(), &widgets).is_ok());
    /// # let _ = build_markup;
    /// # Ok::<(), String>(())
    /// ```
    #[cfg(feature = "markup")]
    pub fn register_markup(widgets: &mut crate::markup::Widgets) {
        widgets.register("effect", MARKUP_PARAMS, true, markup_element);
    }
}
#[cfg(feature = "webview")]
pub use remedy_ui_embed_webview as webview;
pub use remedy_ui_host as host;
pub use remedy_ui_input as input;
pub use remedy_ui_layout as layout;
/// `#[derive(Store)]`: field accessors for a reactive [`reactive::Store`].
#[cfg(feature = "macros")]
pub use remedy_ui_macros::Store;
#[cfg(feature = "anim")]
pub use remedy_ui_media_anim as anim;
#[cfg(feature = "image")]
pub use remedy_ui_media_image as image;
/// Decoding in a sandboxed helper process: images and animations whose decoder crashes,
/// hangs or runs away with memory take down only the helper.
#[cfg(feature = "isolated")]
pub use remedy_ui_media_isolated as isolated;
#[cfg(feature = "playback")]
pub use remedy_ui_media_playback as playback;
#[cfg(feature = "svg")]
pub use remedy_ui_media_svg as svg;
#[cfg(feature = "video")]
pub use remedy_ui_media_video as video;
/// Drawing: display lists, paths, brushes, transforms and images, for painters
/// (`El::paint`), canvases and page sources.
pub use remedy_ui_paint as paint;
/// The GPU renderer's own bounds (feature `gpu`): errors kept, the health file, how long it
/// waits for swapchains, presents and video ([`gpu::set_gpu_limits`]). What both renderers
/// draw is bounded by [`paint::set_paint_limits`].
#[cfg(feature = "gpu")]
pub mod gpu {
    pub use remedy_ui_render_gpu::{
        GpuLimits, MAX_HEALTH_ENTRIES, MAX_HEALTH_LINE, MAX_KEPT_ERRORS, MAX_VIDEO_IMPORTS, SWAPCHAIN_WAIT,
        VIDEO_FENCE_WAIT, gpu_limits, health::PRESENT_HANG, set_gpu_limits,
    };
}
/// Motion: elements animate a property, style value, offset or scale to a target with a
/// spring or an easing curve on the frame clock (`El::animate`), interruptibly and instantly
/// under reduced motion; `Transition`s for content that appears and leaves.
pub mod motion {
    pub use remedy_ui_view::Transition;
    pub use remedy_ui_view::motion::*;
}
pub use remedy_ui_reactive as reactive;
/// `.rui` markup: screens as data, compiled at build time or loaded for people's own layouts.
#[cfg(feature = "markup")]
pub use remedy_ui_rui as markup;
/// Scripted widgets, independent of any language: the widget interface, the element protocol,
/// limits and the engine registry.
#[cfg(feature = "board")]
pub use remedy_ui_script as script;
/// Scripted widgets in Rhai.
#[cfg(feature = "script-rhai")]
pub use remedy_ui_script_rhai as script_rhai;
/// Scripted widgets in WebAssembly.
#[cfg(feature = "script-wasm")]
pub use remedy_ui_script_wasm as script_wasm;
/// Desktop services: starting with the session, URL schemes, jump lists and recent files,
/// power (battery, sleep and resume, keeping awake), showing files in the file manager, and
/// the user's usual folders. Notifications, badges and the tray are on [`App`].
#[cfg(feature = "desktop")]
pub use remedy_ui_services_desktop as desktop;
#[cfg(feature = "i18n")]
pub use remedy_ui_services_i18n as i18n;
/// The HTTP client, natively and in a browser page.
#[cfg(feature = "net")]
pub use remedy_ui_services_net as net;
#[cfg(feature = "spell")]
pub use remedy_ui_services_spell as spell;
#[cfg(feature = "update")]
pub use remedy_ui_services_update as update;
/// Importing the localStorage a web version of the app saved: a web view's store on disk
/// (WebView2, Android System WebView), read losslessly and read-only, or the page's own in a
/// browser.
#[cfg(feature = "webstorage")]
pub use remedy_ui_services_webstorage as webstorage;
pub use remedy_ui_style as style;
pub use remedy_ui_text as text;
#[cfg(feature = "widgets")]
pub use remedy_ui_themes as themes;
pub use remedy_ui_view as view;
/// Content that comes and goes, fails or loads, where apps build screens: [`show`] (content
/// shown while a condition holds), [`catch_errors`] (an error boundary that takes a builder
/// closure for its content, so a retry builds it again) and [`suspense`] (a placeholder
/// while resources load), with what they return and hand to callbacks.
pub use remedy_ui_view::{CaughtError, Retry, Show, SuspenseBoundary, catch_errors, show, suspense};
pub use remedy_ui_view::{
    El, EnterKey, FieldKind, InputPurpose, PaintCx, PointerCaps, TextField, Ui, WindowDirection,
};
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
#[cfg(feature = "stage")]
pub use remedy_ui_widgets_stage as stage;
#[cfg(feature = "term")]
pub use remedy_ui_widgets_term as term;

/// Why the app could not run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// [`AppConfig::single_instance`] could not claim or reach the running instance.
    SingleInstance {
        /// Why, in the platform's words.
        reason: String,
    },
    /// The platform's bridge could not start its event loop, or the loop failed.
    Bridge {
        /// Why, in the platform's words.
        reason: String,
    },
    /// RemedyUI has no bridge for this platform.
    UnsupportedPlatform,
    /// This platform's bridge is a feature the app did not turn on (`web` for browser pages).
    NeedsFeature {
        /// The `remedy-ui` feature to turn on.
        feature: &'static str,
    },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::SingleInstance { reason } => {
                write!(f, "the app could not run as a single instance: {reason}")
            }
            RunError::Bridge { reason } => write!(f, "the app could not run: {reason}"),
            RunError::UnsupportedPlatform => {
                write!(f, "the app could not run: RemedyUI has no bridge for this platform yet")
            }
            RunError::NeedsFeature { feature } => {
                write!(f, "the app could not run: this platform needs remedy-ui's `{feature}` feature")
            }
        }
    }
}

impl std::error::Error for RunError {}

/// Runs the app on this platform's bridge until it quits. `setup` runs once the platform is
/// ready, to open the first windows.
pub fn run(config: AppConfig, setup: impl FnOnce(&App) + 'static) -> Result<(), RunError> {
    // Keepalive requests (an app's last saves) finish as the app quits, as in a closing page.
    #[cfg(all(feature = "net", not(target_arch = "wasm32")))]
    let _keepalive = KeepaliveOnExit(config.keepalive_grace);
    // On Android they also finish when the app goes out of sight: the process is kept from
    // being frozen until they end.
    #[cfg(all(feature = "net", target_os = "android"))]
    remedy_ui_services_net::on_keepalive(remedy_ui_bridge_android::hold_process);
    #[cfg(target_os = "android")]
    remedy_ui_bridge_android::set_held_grace(config.keepalive_grace);
    // SVG files show wherever images do.
    #[cfg(feature = "svg")]
    remedy_ui_media_svg::install();
    let inbox = instance::SharedInbox::default();
    // A browser settles the instance across tabs asynchronously: the first tab starts the
    // app once the browser grants it the instance.
    #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
    if config.single_instance {
        let name = config.name.clone();
        let web = config.web.clone();
        let runner = Runner::new(config, setup).with_inbox(inbox.clone());
        let args = remedy_ui_bridge_web::launch_args();
        return remedy_ui_bridge_web::run_single_instance(&name, args, web, runner, move |a| {
            instance::deliver(&inbox, a)
        })
        .map_err(|e| RunError::SingleInstance { reason: e.reason().to_owned() });
    }
    #[cfg(windows)]
    remedy_ui_bridge_windows::set_instance_wait(config.single_instance_wait);
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    remedy_ui_bridge_linux::set_drop_waits(config.drop_waits.0, config.drop_waits.1);
    if config.single_instance {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if !instance::claim(&config.name, &args, inbox.clone())
            .map_err(|reason| RunError::SingleInstance { reason })?
        {
            // Another instance runs and has our arguments.
            return Ok(());
        }
    }
    #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
    let web = config.web.clone();
    let runner = Runner::new(config, setup).with_inbox(inbox);
    #[cfg(windows)]
    {
        remedy_ui_bridge_windows::run(runner).map_err(|e| RunError::Bridge { reason: e.reason().to_owned() })
    }
    #[cfg(target_os = "macos")]
    {
        remedy_ui_bridge_macos::run(runner).map_err(|e| RunError::Bridge { reason: e.reason().to_owned() })
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        remedy_ui_bridge_linux::run(runner).map_err(|e| RunError::Bridge { reason: e.reason().to_owned() })
    }
    // The activity comes from the app's `remedy_ui::android::main!` entry point.
    #[cfg(target_os = "android")]
    {
        remedy_ui_bridge_android::run(runner).map_err(|e| RunError::Bridge { reason: e.reason().to_owned() })
    }
    #[cfg(target_os = "ios")]
    {
        drop(runner);
        Err(RunError::UnsupportedPlatform)
    }
    // A browser page: the app starts and this returns; the browser runs it from then on.
    #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
    {
        remedy_ui_bridge_web::run_with(web, runner)
            .map_err(|e| RunError::Bridge { reason: e.reason().to_owned() })
    }
    #[cfg(all(not(feature = "web"), target_arch = "wasm32", target_os = "unknown"))]
    {
        drop(runner);
        Err(RunError::NeedsFeature { feature: "web" })
    }
    #[cfg(not(any(windows, unix, all(target_arch = "wasm32", target_os = "unknown"))))]
    {
        drop(runner);
        Err(RunError::UnsupportedPlatform)
    }
}

#[cfg(all(feature = "net", not(target_arch = "wasm32")))]
struct KeepaliveOnExit(std::time::Duration);

#[cfg(all(feature = "net", not(target_arch = "wasm32")))]
impl Drop for KeepaliveOnExit {
    fn drop(&mut self) {
        remedy_ui_services_net::finish_keepalive(self.0);
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
