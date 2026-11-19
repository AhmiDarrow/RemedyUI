//! Unlock screen: a vault whose content stays out of the frame and the accessibility tree
//! until the master password is given, and locks again after five minutes without input.
//! The password is typed into a secret field: masked unless revealed, held in a zeroizing
//! buffer, never copied, never given to an input method or a screen reader.
//!
//! A real vault derives a key from the password (Argon2, scrypt) on a background thread
//! with `App::background` and tries to open its file with it. This recipe takes the check as
//! a function; `main` compares with the password given on the command line.
//!
//! Run it with `cargo run -p remedy-ui --example unlock_screen -- <password>`.

use std::rc::Rc;
use std::time::Duration;

use remedy_ui::reactive::Signal;
use remedy_ui::show;
use remedy_ui::view::Space;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// How long without input before the vault locks.
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// Says whether a password opens the vault.
pub type Check = Rc<dyn Fn(&str) -> bool>;

/// Opens the vault, locked, with `check` deciding which password opens it.
pub fn app_with(app: &App, check: Check) {
    let locked = Signal::new(true);
    let a = app.clone();
    app.idle_lock(Some(IDLE), move || {
        // Anything secret the app copied goes with the lock.
        a.clear_secret();
        locked.set(true);
    });
    app.open(WindowSpec::new("Vault").size(420.0, 360.0).capture_protected(true), move |ui, _| {
        ui.root().child(vault(ui, locked, check));
    });
}

/// The vault's content, covered by the unlock form while `locked`.
pub fn vault(ui: &Ui, locked: Signal<bool>, check: Check) -> El {
    let content = w::column(ui)
        .gap(Space::S3)
        .padding(Space::S6)
        .child(w::heading(ui, 1, "Vault"))
        .child(w::text(ui, "3 passwords, 1 recovery key"))
        .child(w::button(ui, "Lock now", move || locked.set(true)));
    let u = ui.clone();
    let mut layer: Option<El> = None;
    content.bind(move || {
        if locked.get() {
            if layer.is_none() {
                layer = Some(remedy_ui::reactive::untrack(|| {
                    w::lock_screen(&u, unlock_form(&u, locked, check.clone()))
                }));
            }
        } else if let Some(l) = layer.take() {
            l.remove();
        }
    })
}

/// The form on the lock screen: the password, a reveal toggle, what went wrong, and Unlock.
pub fn unlock_form(ui: &Ui, locked: Signal<bool>, check: Check) -> El {
    let error: Signal<Option<String>> = Signal::new(None);
    let password = w::secret_input(ui, "Master password");
    let field = password.clone();
    let try_unlock: Rc<dyn Fn()> = Rc::new(move || {
        // The secret leaves the field (which empties) and is wiped when dropped.
        let secret = field.take_secret();
        if secret.is_empty() {
            error.set(Some("Enter the master password.".into()));
        } else if check(secret.expose()) {
            error.set(None);
            locked.set(false);
        } else {
            error.set(Some("That password does not open this vault.".into()));
        }
    });
    let on_enter = try_unlock.clone();
    let password = password.on_submit(move || on_enter());
    // What went wrong, shown only while something did.
    let u = ui.clone();
    let message = show(
        ui,
        move || error.get().is_some(),
        move || w::text(&u, move || error.get().unwrap_or_default()).variant("error"),
    )
    .el();
    w::card(ui)
        .column()
        .gap(Space::S3)
        .padding(Space::S6)
        .width(ui.theme().spacing.s(80))
        .child(w::heading(ui, 2, "Unlock vault"))
        .child(
            w::row(ui)
                .gap(Space::S1)
                .fill_width()
                .child(password.el().grow(1.0))
                .child(w::reveal_toggle(ui, &password)),
        )
        .child(message)
        .child(w::button(ui, "Unlock", move || try_unlock()).variant("primary"))
}

fn main() {
    let Some(password) = std::env::args().nth(1) else {
        eprintln!("usage: unlock_screen <password>");
        std::process::exit(2);
    };
    let check: Check = Rc::new(move |given: &str| given == password);
    if let Err(e) = remedy_ui::run(AppConfig::new("Unlock screen"), move |app: &App| app_with(app, check)) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
