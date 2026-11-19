//! File explorer: a folder tree beside a sortable table of the chosen folder's contents,
//! read from the file system as folders open.
//!
//! Run it with `cargo run -p remedy-ui --example file_explorer -- <folder>`; without a
//! folder it starts in the current one.

use std::path::{Path, PathBuf};

use remedy_ui::data::{Column, SortValue, TableState, TreeItem, TreeState, natural_cmp, table, tree_view};
use remedy_ui::layout::Axis;
use remedy_ui::reactive::Signal;
use remedy_ui::show;
use remedy_ui::widgets::{self as w, Icon};
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// One entry of a folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Its full path (the table's key).
    pub path: PathBuf,
    /// Its name.
    pub name: String,
    /// A folder.
    pub folder: bool,
    /// Its size in bytes (files only).
    pub size: Option<u64>,
}

/// The entries of `dir`, folders first, each group in natural order (`file2` before
/// `file10`), or why it could not be read.
pub fn read_folder(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|d| {
            let meta = d.metadata().ok();
            let folder = meta.as_ref().is_some_and(std::fs::Metadata::is_dir);
            Entry {
                path: d.path(),
                name: d.file_name().to_string_lossy().into_owned(),
                folder,
                size: if folder { None } else { meta.map(|m| m.len()) },
            }
        })
        .collect();
    entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| natural_cmp(&a.name, &b.name)));
    Ok(entries)
}

/// A size for people: bytes, KB, MB or GB.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    let name = UNITS.get(unit).copied().unwrap_or("bytes");
    if unit == 0 { format!("{bytes} {name}") } else { format!("{v:.1} {name}") }
}

/// The folders of `dir`, as tree items.
fn folder_items(dir: &Path) -> Vec<TreeItem<PathBuf>> {
    read_folder(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.folder)
        .map(|e| TreeItem::branch(e.path, e.name).icon(Icon::Folder))
        .collect()
}

/// Opens the explorer on the folder given on the command line, or the current folder.
pub fn app(app: &App) {
    let root = std::env::args().nth(1).map_or_else(|| PathBuf::from("."), PathBuf::from);
    app.open(WindowSpec::new("Files").size(820.0, 520.0).persist_geometry("explorer"), move |ui, _| {
        ui.root().child(explorer(ui, &root));
    });
}

/// The explorer over `root`: the tree of its folders and the table of the chosen one.
pub fn explorer(ui: &Ui, root: &Path) -> El {
    let root_name =
        root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
    let tree_state =
        TreeState::new(vec![TreeItem::branch(root.to_path_buf(), root_name).icon(Icon::Folder)], false);
    tree_state.expand(root.to_path_buf());
    tree_state.set_children(root.to_path_buf(), folder_items(root));
    tree_state.selection.select_only(root.to_path_buf());

    // The tree asks for a folder's children the first time it opens.
    let loader = tree_state.clone();
    let load = move |dir: &PathBuf| loader.set_children(dir.clone(), folder_items(dir));
    let folders = tree_view(ui, "Folders", &tree_state, load, |_| {}, None);

    // The table shows the folder chosen in the tree.
    let rows: Signal<Vec<Entry>> = Signal::new(Vec::new());
    let problem: Signal<Option<String>> = Signal::new(None);
    let table_state: TableState<PathBuf> = TableState::new(true);
    let columns = vec![
        Column::text("name", "Name", 260.0, |e: &Entry| e.name.clone()),
        Column::text(
            "kind",
            "Kind",
            90.0,
            |e: &Entry| if e.folder { "Folder".into() } else { "File".into() },
        ),
        Column::text("size", "Size", 100.0, |e: &Entry| e.size.map(human_size).unwrap_or_default())
            .sort_by(|e: &Entry| e.size.map_or(SortValue::Missing, |s| SortValue::Number(s as f64)))
            .align_end(),
    ];
    // Opening a folder in the table opens it in the tree too.
    let (opener, entries) = (tree_state.clone(), rows);
    let open = move |path: &PathBuf| {
        let is_folder = entries.with(|e| e.iter().any(|x| &x.path == path && x.folder));
        if is_folder {
            if let Some(parent) = path.parent() {
                opener.expand(parent.to_path_buf());
            }
            opener.selection.select_only(path.clone());
        }
    };
    let files = table(ui, "Contents", rows, |e: &Entry| e.path.clone(), columns, &table_state, open);
    // Why the folder could not be read, shown only while there is a reason.
    let u = ui.clone();
    let error = show(
        ui,
        move || problem.get().is_some(),
        move || w::text(&u, move || problem.get().unwrap_or_default()).variant("error"),
    )
    .el();
    let selected = tree_state.selection.selected;
    let right = w::column(ui).grow(1.0).fill_width().child(error).child(files).bind(move || {
        let Some(dir) = selected.get().last().cloned() else { return };
        match read_folder(&dir) {
            Ok(list) => {
                rows.set(list);
                problem.set(None);
            }
            Err(e) => {
                rows.set(Vec::new());
                problem.set(Some(e));
            }
        }
    });
    w::split(ui, Axis::Row, folders, right, Signal::new(0.3))
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("File explorer"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
