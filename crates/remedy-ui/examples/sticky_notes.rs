//! Sticky notes: each note is its own small frameless window that stays on top, moves by
//! its header, and opens where it was left. Notes are saved as text files as they are
//! written; "New note" opens another, and "Delete note" removes one.
//!
//! Run it with `cargo run -p remedy-ui --example sticky_notes`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use remedy_ui::layout::Align;
use remedy_ui::reactive::Signal;
use remedy_ui::show;
use remedy_ui::view::Space;
use remedy_ui::widgets::{self as w, Icon};
use remedy_ui::{App, AppConfig, El, Ui, Window, WindowSpec};

/// Where notes live: one `<n>.txt` file each.
#[derive(Clone, Debug)]
pub struct Notes {
    dir: PathBuf,
}

impl Notes {
    /// Notes kept in `dir` (created when the first note is saved).
    pub fn new(dir: PathBuf) -> Notes {
        Notes { dir }
    }

    /// The numbers of the saved notes, in order.
    pub fn saved(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = std::fs::read_dir(&self.dir)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter_map(|e| {
                        e.path().file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok())
                    })
                    .collect()
            })
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    }

    /// The file of note `n`.
    pub fn path(&self, n: u32) -> PathBuf {
        self.dir.join(format!("{n}.txt"))
    }

    /// Note `n`'s text (empty when it was never saved).
    pub fn read(&self, n: u32) -> String {
        std::fs::read_to_string(self.path(n)).unwrap_or_default()
    }

    /// Saves note `n`.
    pub fn write(&self, n: u32, text: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.path(n), text)
    }

    /// Deletes note `n`.
    pub fn delete(&self, n: u32) -> std::io::Result<()> {
        match std::fs::remove_file(self.path(n)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// A number no note has yet.
    pub fn next(&self, open: &[u32]) -> u32 {
        self.saved().into_iter().chain(open.iter().copied()).max().map_or(1, |n| n + 1)
    }
}

/// Opens the saved notes, kept in the app's data folder, or one empty note.
pub fn app(app: &App) {
    let dir = app.data_dir().map_or_else(|| PathBuf::from("notes"), |d| d.join("notes"));
    open_notes(app, &dir);
}

/// Opens the notes saved in `dir`, or one empty note when there are none.
pub fn open_notes(app: &App, dir: &Path) {
    let notes = Rc::new(Notes::new(dir.to_path_buf()));
    let saved = notes.saved();
    if saved.is_empty() {
        let _ = open_note(app, &notes, 1);
    }
    for n in saved {
        let _ = open_note(app, &notes, n);
    }
}

/// Opens note `n` in its own window, where it was last time.
pub fn open_note(app: &App, notes: &Rc<Notes>, n: u32) -> Window {
    let spec = WindowSpec::new(format!("Note {n}"))
        .size(260.0, 240.0)
        .min_size(180.0, 140.0)
        .frameless()
        .always_on_top(true)
        .persist_geometry(format!("note-{n}"));
    let notes = notes.clone();
    app.open(spec, move |ui, window| {
        ui.root().child(note(ui, window, &notes, n));
    })
}

/// A note: a header that moves the window, and the text.
pub fn note(ui: &Ui, window: &Window, notes: &Rc<Notes>, n: u32) -> El {
    let body = w::text_area(ui, "Note").placeholder("Write a note");
    body.set_value(&notes.read(n));
    let (field, store) = (body.clone(), notes.clone());
    let failed = Signal::new(None::<String>);
    let body = body.on_change(move || {
        // Saved as it is written, so nothing is lost if the app is closed.
        failed.set(store.write(n, &field.value()).err().map(|e| e.to_string()));
    });
    // Why the last save failed, shown only while it is failing.
    let u = ui.clone();
    let error = show(
        ui,
        move || failed.get().is_some(),
        move || w::text(&u, move || failed.get().unwrap_or_default()).variant("error"),
    )
    .el();

    let (app, store) = (window.app().clone(), notes.clone());
    let new_note = w::icon_button(ui, Icon::Plus, "New note", move || {
        let open: Vec<u32> = app
            .windows()
            .iter()
            .filter_map(|w| w.title().strip_prefix("Note ").and_then(|s| s.parse().ok()))
            .collect();
        let _ = open_note(&app, &store, store.next(&open));
    });
    let (win, store) = (window.clone(), notes.clone());
    let delete = w::icon_button(ui, Icon::Trash, "Delete note", move || {
        if store.delete(n).is_ok() {
            win.app().forget_geometry(format!("note-{n}"));
            win.close();
        }
    });
    let mover = window.clone();
    let header = w::row(ui)
        .align(Align::Center)
        .gap(Space::S1)
        .padding(Space::S1)
        .fill_width()
        .child(w::text(ui, format!("Note {n}")).variant("muted").grow(1.0))
        .child(new_note)
        .child(delete)
        .drag_region(move || mover.begin_move());
    w::column(ui).grow(1.0).fill_width().child(header).child(error).child(body.el().grow(1.0).fill_width())
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Sticky notes"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
