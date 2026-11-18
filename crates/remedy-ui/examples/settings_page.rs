//! Settings page: a nav rail of sections, each a column of labeled controls bound to the
//! app's settings. The theme choice restyles every window as soon as it changes, and every
//! setting is saved as it changes and restored at the next start (persisted signals).
//!
//! Run it with `cargo run -p remedy-ui --example settings_page`.

use remedy_ui::nav::{NavEntry, nav_rail};
use remedy_ui::reactive::Signal;
use remedy_ui::themes::{PALETTES, high_contrast, theme};
use remedy_ui::view::{Space, show};
use remedy_ui::widgets::{self as w, Icon};
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// The app's settings, each a signal the controls read and write.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// The palette, an index into [`PALETTES`].
    pub palette: Signal<usize>,
    /// High contrast on top of the palette.
    pub high_contrast: Signal<bool>,
    /// Show notifications at all.
    pub notifications: Signal<bool>,
    /// Play a sound with each notification.
    pub sound: Signal<bool>,
    /// The sound's volume, 0 to 100.
    pub volume: Signal<f64>,
}

impl Settings {
    /// The settings as last saved, or the defaults (the first palette, notifications on
    /// with a sound at 70). Built inside a window, where the app's persisted-state store is
    /// provided, each one saves itself as it changes.
    pub fn load() -> Settings {
        Settings {
            palette: Signal::new(0).persist("settings.palette"),
            high_contrast: Signal::new(false).persist("settings.high-contrast"),
            notifications: Signal::new(true).persist("settings.notifications"),
            sound: Signal::new(true).persist("settings.sound"),
            volume: Signal::new(70.0).persist("settings.volume"),
        }
    }
}

/// Opens the settings window.
pub fn app(app: &App) {
    app.open(WindowSpec::new("Settings").size(720.0, 480.0), |ui, window| {
        let settings = Settings::load();
        // The chosen theme applies to every window the app has.
        let app = window.app().clone();
        let root = ui.root().child(page(ui, settings));
        let _ = root.bind(move || {
            let Some(p) = PALETTES.get(settings.palette.get()) else { return };
            let t = theme(p);
            app.set_theme(if settings.high_contrast.get() { high_contrast(&t) } else { t });
        });
    });
}

/// The page: the rail on the start side, the chosen section beside it.
pub fn page(ui: &Ui, settings: Settings) -> El {
    let section = Signal::new("appearance".to_owned());
    let rail = nav_rail(
        ui,
        "Settings",
        vec![
            NavEntry::new("appearance", Icon::Settings, "Appearance"),
            NavEntry::new("notifications", Icon::Info, "Notifications"),
        ],
        section,
        Signal::new(false),
    );
    // Each section is built when it is chosen and disposed when another is.
    let (u1, u2) = (ui.clone(), ui.clone());
    let content = w::column(ui)
        .padding(Space::S6)
        .grow(1.0)
        .fill_width()
        .child(show(ui, move || section.get() == "appearance", move || appearance(&u1, settings)).el())
        .child(show(ui, move || section.get() == "notifications", move || notifications(&u2, settings)).el());
    w::row(ui).grow(1.0).fill_width().child(rail).child(content)
}

/// Theme and contrast.
pub fn appearance(ui: &Ui, s: Settings) -> El {
    let names = PALETTES.iter().map(|p| p.name.to_owned()).collect();
    w::column(ui)
        .gap(Space::S3)
        .child(w::heading(ui, 2, "Appearance"))
        .child(w::select(ui, "Theme", names, s.palette))
        .child(w::switch(ui, "High contrast", s.high_contrast))
}

/// Notifications, their sound and its volume. The sound settings mean nothing while
/// notifications are off, so they are disabled then.
pub fn notifications(ui: &Ui, s: Settings) -> El {
    let on = s.notifications;
    w::column(ui)
        .gap(Space::S3)
        .child(w::heading(ui, 2, "Notifications"))
        .child(w::switch(ui, "Show notifications", on))
        .child(w::switch(ui, "Play a sound", s.sound).disabled(move || !on.get()))
        .child(
            w::slider(ui, "Volume", s.volume, 0.0, 100.0, 5.0).disabled(move || !on.get() || !s.sound.get()),
        )
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Settings page"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
