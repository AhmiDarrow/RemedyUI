//! Menu bar: a document reader's File, Edit, View, Go and Help menus, as desktop apps have
//! them. Commands give the menus, the shortcuts and the command palette one source (label,
//! shortcut, whether it can run now); View has check entries (Book Mode, Full Screen) and
//! submenus of radio entries (Page Appearance, UI Scale). Every title and entry has an access
//! key: Alt+V opens View from anywhere, F10 or a lone Alt reaches the bar, and letters choose
//! inside an open menu.
//!
//! Run it with `cargo run -p remedy-ui --example menu_bar`.

use remedy_ui::input::{Chord, Key, Modifiers};
use remedy_ui::nav::Commands;
use remedy_ui::reactive::Signal;
use remedy_ui::view::Space;
use remedy_ui::widgets::{self as w, MenuItem};
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// What the reader shows, each a signal the menus read and write.
#[derive(Clone, Copy, Debug)]
pub struct Reader {
    /// The zoom, in percent.
    pub zoom: Signal<u32>,
    /// Two pages side by side.
    pub book_mode: Signal<bool>,
    /// The window fills the screen.
    pub full_screen: Signal<bool>,
    /// The page color: `light`, `sepia` or `night`.
    pub appearance: Signal<&'static str>,
    /// The interface scale, in percent.
    pub ui_scale: Signal<u32>,
    /// The page shown, from 1.
    pub page: Signal<u32>,
    /// Whether a document is open (Save and Close need one).
    pub open: Signal<bool>,
    /// What the last command did, for the status line.
    pub status: Signal<String>,
}

impl Reader {
    /// A reader with a document open at 100%.
    pub fn new() -> Reader {
        Reader {
            zoom: Signal::new(100),
            book_mode: Signal::new(false),
            full_screen: Signal::new(false),
            appearance: Signal::new("light"),
            ui_scale: Signal::new(100),
            page: Signal::new(1),
            open: Signal::new(true),
            status: Signal::new(String::new()),
        }
    }
}

impl Default for Reader {
    fn default() -> Self {
        Reader::new()
    }
}

fn ctrl(c: char) -> Vec<Chord> {
    vec![Chord::new(Modifiers::CTRL, Key::char(c))]
}

/// Registers the reader's commands: the shortcuts work wherever focus is, and the menus and
/// the palette show the same labels and shortcuts.
pub fn commands(r: Reader) -> Commands {
    let c = Commands::new();
    let say = move |s: &'static str| move || r.status.set(s.to_owned());
    c.add("file.open", "Open…", ctrl('o'), None, say("Open a document"));
    c.add("file.save", "Save", ctrl('s'), None, say("Saved"));
    c.add("file.close", "Close Document", ctrl('w'), None, move || {
        r.open.set(false);
        r.status.set("Closed".to_owned());
    });
    c.add("app.quit", "Quit", ctrl('q'), None, say("Quit"));
    c.add("edit.find", "Find…", ctrl('f'), None, say("Find"));
    c.add("view.zoom_in", "Zoom In", vec![Chord::new(Modifiers::CTRL, Key::char('+'))], None, move || {
        r.zoom.update(|z| *z = (*z + 10).min(800));
    });
    c.add("view.zoom_out", "Zoom Out", ctrl('-'), None, move || {
        r.zoom.update(|z| *z = z.saturating_sub(10).max(10))
    });
    c.add("view.book", "Book Mode", ctrl('b'), None, move || r.book_mode.update(|b| *b = !*b));
    c.add(
        "view.full_screen",
        "Full Screen",
        vec![Chord::new(Modifiers::NONE, Key::F(11))],
        None,
        move || {
            r.full_screen.update(|f| *f = !*f);
        },
    );
    c.add("go.next", "Next Page", vec![Chord::new(Modifiers::NONE, Key::PageDown)], None, move || {
        r.page.update(|p| *p += 1);
    });
    c.add("go.previous", "Previous Page", vec![Chord::new(Modifiers::NONE, Key::PageUp)], None, move || {
        r.page.update(|p| *p = p.saturating_sub(1).max(1));
    });
    c
}

/// The menus, built from the commands: `Commands::menu_item` takes each one's label,
/// shortcut, enabled state and action; check and radio entries show the reader's signals.
pub fn menus(c: &Commands, r: Reader) -> Vec<MenuItem> {
    let file = MenuItem::submenu(
        "File",
        vec![
            c.menu_item("file.open").mnemonic('o'),
            c.menu_item("file.save").mnemonic('s'),
            c.menu_item("file.close").mnemonic('c'),
            MenuItem::separator(),
            c.menu_item("app.quit").mnemonic('q'),
        ],
    )
    .mnemonic('f');
    let edit = MenuItem::submenu("Edit", vec![c.menu_item("edit.find").mnemonic('f')]).mnemonic('e');
    let appearance = MenuItem::submenu(
        "Page Appearance",
        vec![
            MenuItem::radio_signal("Light", r.appearance, "light").mnemonic('l'),
            MenuItem::radio_signal("Sepia", r.appearance, "sepia").mnemonic('s'),
            MenuItem::radio_signal("Night", r.appearance, "night").mnemonic('n'),
        ],
    )
    .mnemonic('a');
    let scale = MenuItem::submenu(
        "UI Scale",
        [90, 100, 125, 150]
            .into_iter()
            .map(|p| MenuItem::radio_signal(format!("{p}%"), r.ui_scale, p))
            .collect(),
    )
    .mnemonic('u');
    let book = r.book_mode;
    let full = r.full_screen;
    let view = MenuItem::submenu(
        "View",
        vec![
            c.menu_item("view.zoom_in").mnemonic('i'),
            c.menu_item("view.zoom_out").mnemonic('o'),
            MenuItem::separator(),
            MenuItem::check_signal("Book Mode", book)
                .chords({
                    let c = c.clone();
                    move || c.info("view.book").map(|i| i.chords).unwrap_or_default()
                })
                .mnemonic('b'),
            MenuItem::check_signal("Full Screen", full)
                .chord(Chord::new(Modifiers::NONE, Key::F(11)))
                .mnemonic('f'),
            MenuItem::separator(),
            appearance,
            scale,
        ],
    )
    .mnemonic('v');
    let go = MenuItem::submenu(
        "Go",
        vec![c.menu_item("go.next").mnemonic('n'), c.menu_item("go.previous").mnemonic('p')],
    )
    .mnemonic('g');
    let help = MenuItem::submenu(
        "Help",
        vec![MenuItem::new("About", move || r.status.set("RemedyUI".to_owned())).mnemonic('a')],
    )
    .mnemonic('h');
    vec![file, edit, view, go, help]
}

/// The window: the menu bar, then the page with a status line.
pub fn page(ui: &Ui, r: Reader) -> El {
    let c = commands(r);
    // Save and Close need a document.
    let c2 = c.clone();
    let _ = ui.root().bind(move || {
        let open = r.open.get();
        remedy_ui::reactive::untrack(|| {
            c2.set_enabled("file.save", open);
            c2.set_enabled("file.close", open);
        });
    });
    let _ = c.attach(ui);
    let status = move || {
        let s = r.status.get();
        let shown = format!("Page {} · {}% · {}", r.page.get(), r.zoom.get(), r.appearance.get());
        if s.is_empty() { shown } else { format!("{shown} · {s}") }
    };
    w::column(ui).fill_width().grow(1.0).child(w::menu_bar(ui, menus(&c, r))).child(
        w::column(ui)
            .padding(Space::S4)
            .gap(Space::S2)
            .child(w::heading(ui, 2, "Document"))
            .child(w::text(ui, status)),
    )
}

/// Opens the reader window.
pub fn app(app: &App) {
    app.open(WindowSpec::new("Reader").size(720.0, 480.0), |ui, _| {
        let r = Reader::new();
        ui.root().child(page(ui, r));
    });
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Menu bar"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
