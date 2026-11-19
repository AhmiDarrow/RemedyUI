//! Tray app: a break reminder that lives in the system tray. Closing its window hides it
//! there; every twenty minutes, unless paused, the window comes back to suggest a break. The
//! tray menu shows the window, pauses or resumes, and quits. Starting it again shows the one
//! already running.
//!
//! Run it with `cargo run -p remedy-ui --example tray_app`.

use std::time::Duration;

use remedy_ui::host::{Notification, NotificationResponse, TrayEvent, TrayIcon, TrayImage, TrayItem};
use remedy_ui::reactive::Signal;
use remedy_ui::view::Space;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, El, Ui, Window, WindowSpec};

/// Time between reminders.
pub const EVERY: Duration = Duration::from_secs(20 * 60);

/// Tray menu ids.
pub const SHOW: u32 = 1;
/// Pauses or resumes reminders.
pub const PAUSE: u32 = 2;
/// Quits the app.
pub const QUIT: u32 = 3;

/// The reminder's state.
#[derive(Clone, Copy, Debug)]
pub struct Reminder {
    /// Reminders are paused.
    pub paused: Signal<bool>,
    /// A break is due: the window says so until it is taken.
    pub due: Signal<bool>,
}

/// Starts the app: the window, the tray icon and its menu, and the reminder's timer.
pub fn app(app: &App) {
    app.keep_running(true);
    let state = Reminder { paused: Signal::new(false), due: Signal::new(false) };
    let window =
        app.open(WindowSpec::new("Break reminder").size(360.0, 220.0).hide_on_close(), move |ui, _| {
            ui.root().child(home(ui, state));
        });
    // The tray follows the state: its menu says "Pause" or "Resume" to match.
    let icon = tray_icon();
    app.tray_dyn(move || {
        let paused = state.paused.get();
        let item = |id, label: &str| TrayItem::Item { id, label: label.to_owned(), enabled: true };
        let menu = vec![
            item(SHOW, "Show"),
            item(PAUSE, if paused { "Resume reminders" } else { "Pause reminders" }),
            TrayItem::Separator,
            item(QUIT, "Quit"),
        ];
        Some(TrayIcon::new("Break reminder", icon.clone(), menu))
    });
    let (win, quit) = (window.clone(), app.clone());
    app.on_tray(move |e| match e {
        TrayEvent::Activate | TrayEvent::Menu(SHOW) => show(&win),
        TrayEvent::Menu(PAUSE) => state.paused.update(|p| *p = !*p),
        TrayEvent::Menu(QUIT) => quit.quit(),
        _ => {}
    });
    let again = window.clone();
    app.on_second_instance(move |_| show(&again));
    let (shown, done) = (window.clone(), state.due);
    app.on_notification(move |id, response| match (id, response) {
        (BREAK_NOTICE, NotificationResponse::Action(a)) if a == "taken" => done.set(false),
        (BREAK_NOTICE, NotificationResponse::Activated) => show(&shown),
        _ => {}
    });
    remind(app, window, state);
}

/// Shows the window and brings it forward.
fn show(window: &Window) {
    window.set_visible(true);
    window.focus();
}

/// The break reminder's notification (showing it again replaces it).
pub const BREAK_NOTICE: u64 = 1;

/// Brings the window back after [`EVERY`] unless paused, with a system notification that
/// can take the break from where it shows, then waits again.
fn remind(app: &App, window: Window, state: Reminder) {
    let a = app.clone();
    app.after(EVERY, move || {
        if !state.paused.get_untracked().unwrap_or(false) {
            state.due.set(true);
            show(&window);
            let notice = Notification::new(
                BREAK_NOTICE,
                "Time for a break",
                "Stand up and look away from the screen.",
            )
            .action("taken", "Break taken");
            // Where the platform has no notifications, the window alone does the reminding.
            a.notify(notice, |_| {});
        }
        remind(&a, window, state);
    });
}

/// The window's content: what the reminder is doing, and its controls.
pub fn home(ui: &Ui, state: Reminder) -> El {
    let Reminder { paused, due } = state;
    w::column(ui)
        .gap(Space::S3)
        .padding(Space::S6)
        .child(w::heading(ui, 2, "Break reminder"))
        .child(
            w::text(ui, move || {
                if due.get() {
                    "Time for a break: stand up and look away from the screen.".to_owned()
                } else if paused.get() {
                    "Reminders are paused.".to_owned()
                } else {
                    "A reminder comes every twenty minutes. Close this window and it waits in the tray."
                        .to_owned()
                }
            })
            .variant("muted"),
        )
        .child(w::switch(ui, "Pause reminders", paused))
        .child(w::button(ui, "Break taken", move || due.set(false)).disabled(move || !due.get()))
}

/// A 32 px tray icon drawn in code: a green dot, premultiplied RGBA.
pub fn tray_icon() -> TrayImage {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
            let a = (13.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let c = |v: f32| (v * a * 255.0).round() as u8;
            rgba.extend([c(0.24), c(0.55), c(0.37), (a * 255.0).round() as u8]);
        }
    }
    TrayImage { size, rgba }
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Break reminder").single_instance(), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
