//! Tree views and property grids on a real view with the theme sheet.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::{Point, Vec2};
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_data::*;

mod common;
use common::*;

/// A little file system: folders hold names; `load` answers from it.
struct Disk {
    folders: RefCell<HashMap<String, Vec<(String, bool)>>>,
}

impl Disk {
    fn new() -> Rc<Disk> {
        let mut f = HashMap::new();
        f.insert(
            "/".to_owned(),
            vec![("docs".to_owned(), true), ("music".to_owned(), true), ("readme.txt".to_owned(), false)],
        );
        f.insert(
            "/docs".to_owned(),
            vec![("letters".to_owned(), true), ("cv.pdf".to_owned(), false), ("notes.md".to_owned(), false)],
        );
        f.insert("/docs/letters".to_owned(), vec![("dear.txt".to_owned(), false)]);
        f.insert("/music".to_owned(), vec![("song.ogg".to_owned(), false)]);
        Rc::new(Disk { folders: RefCell::new(f) })
    }

    fn items(&self, dir: &str) -> Vec<TreeItem<String>> {
        let base = if dir == "/" { String::new() } else { dir.to_owned() };
        self.folders
            .borrow()
            .get(dir)
            .map(|v| {
                v.iter()
                    .map(|(n, folder)| {
                        let k = format!("{base}/{n}");
                        if *folder { TreeItem::branch(k, n.clone()) } else { TreeItem::leaf(k, n.clone()) }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct T {
    ui: Ui,
    tree: El,
    state: TreeState<String>,
    loads: Rc<RefCell<Vec<String>>>,
    opened: Rc<RefCell<Vec<String>>>,
    disk: Rc<Disk>,
}

/// A tree whose loads are answered by `answer` (immediately, or held back by the test).
fn tree(answer: bool) -> T {
    let ui = ui();
    let disk = Disk::new();
    let state = TreeState::new(disk.items("/"), true);
    let loads: Rc<RefCell<Vec<String>>> = Rc::default();
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let (l2, o2, d2, s2, d3, s3) =
        (loads.clone(), opened.clone(), disk.clone(), state.clone(), disk.clone(), state.clone());
    let mover: MoveBox<String> = Box::new(move |items: &[String], target: &String| {
        if target.starts_with("/music") && items.iter().any(|i| i.ends_with(".pdf")) {
            return Err("Music holds only songs".into());
        }
        // Move on the "disk", then refresh the folders involved.
        let mut touched = vec![target.clone()];
        for it in items {
            let (parent, name) = it
                .rsplit_once('/')
                .map(|(p, n)| (if p.is_empty() { "/".to_owned() } else { p.to_owned() }, n.to_owned()))
                .unwrap_or_default();
            let mut f = d3.folders.borrow_mut();
            let entry =
                f.get_mut(&parent).and_then(|v| v.iter().position(|(n, _)| *n == name).map(|i| v.remove(i)));
            if let Some(e) = entry {
                f.entry(target.clone()).or_default().push(e);
            }
            touched.push(parent);
        }
        for t in touched {
            if t == "/" {
                s3.roots.set(d3.items("/"));
            } else {
                s3.set_children(t.clone(), d3.items(&t));
            }
        }
        Ok(())
    });
    let tree = tree_view(
        &ui,
        "Files",
        &state,
        move |k: &String| {
            l2.borrow_mut().push(k.clone());
            if answer {
                s2.set_children(k.clone(), d2.items(k));
            }
        },
        move |k: &String| o2.borrow_mut().push(k.clone()),
        Some(mover),
    );
    ui.root().child(tree.clone().fill_width());
    frame(&ui);
    tree.focus();
    frame(&ui);
    T { ui, tree, state, loads, opened, disk }
}

fn labels(t: &T) -> Vec<String> {
    find(&t.tree, "tree_item").iter().map(|r| texts(r).first().cloned().unwrap_or_default()).collect()
}

fn cursor(t: &T) -> Option<String> {
    t.state.selection.cursor.get_untracked().ok().flatten()
}

#[test]
fn items_open_and_close_from_the_keyboard_and_children_load_on_demand() {
    let t = tree(true);
    assert_eq!(labels(&t), ["docs", "music", "readme.txt"]);
    assert!(t.loads.borrow().is_empty(), "nothing loads before it opens");
    assert!(key(&t.ui, Key::ArrowDown, NONE));
    assert_eq!(cursor(&t).as_deref(), Some("/docs"));
    // Right opens, then moves to the first child.
    assert!(key(&t.ui, Key::ArrowRight, NONE));
    assert_eq!(*t.loads.borrow(), ["/docs"]);
    assert_eq!(labels(&t), ["docs", "letters", "cv.pdf", "notes.md", "music", "readme.txt"]);
    assert!(key(&t.ui, Key::ArrowRight, NONE));
    assert_eq!(cursor(&t).as_deref(), Some("/docs/letters"));
    // Left on a closed item goes to its parent; Left again closes it.
    assert!(key(&t.ui, Key::ArrowLeft, NONE));
    assert_eq!(cursor(&t).as_deref(), Some("/docs"));
    assert!(key(&t.ui, Key::ArrowLeft, NONE));
    assert_eq!(labels(&t), ["docs", "music", "readme.txt"]);
    // Opening it again does not load again: the children are known.
    assert!(key(&t.ui, Key::ArrowRight, NONE));
    assert_eq!(t.loads.borrow().len(), 1);
    // Enter activates; End and Home jump.
    assert!(key(&t.ui, Key::End, NONE));
    assert!(key(&t.ui, Key::Enter, NONE));
    assert_eq!(*t.opened.borrow(), ["/readme.txt"]);
    assert!(key(&t.ui, Key::Home, NONE));
    assert_eq!(cursor(&t).as_deref(), Some("/docs"));
}

#[test]
fn children_that_have_not_arrived_show_loading_until_they_do() {
    let t = tree(false);
    assert!(key(&t.ui, Key::ArrowDown, NONE));
    assert!(key(&t.ui, Key::ArrowRight, NONE));
    let docs = find(&t.tree, "tree_item").into_iter().next().unwrap_or_else(|| unreachable!());
    assert!(texts(&docs).iter().any(|s| s == "Loading…"), "{:?}", texts(&docs));
    let sem = docs.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!());
    assert_eq!(sem.description_text(), Some("Loading"));
    // The app answers later.
    t.state.set_children("/docs".into(), t.disk.items("/docs"));
    frame(&t.ui);
    assert_eq!(labels(&t).len(), 6);
    let docs = find(&t.tree, "tree_item").into_iter().next().unwrap_or_else(|| unreachable!());
    assert!(!texts(&docs).iter().any(|s| s == "Loading…"));
    assert_eq!(t.loads.borrow().len(), 1, "asked once");
}

#[test]
fn rows_tell_screen_readers_their_level_position_and_state() {
    let t = tree(true);
    t.state.expand("/docs".into());
    frame(&t.ui);
    let rows = find(&t.tree, "tree_item");
    let letters = rows.get(1).and_then(El::info).and_then(|i| i.semantics).unwrap_or_else(|| unreachable!());
    assert_eq!(letters.role(), remedy_ui_core::a11y::Role::TreeItem);
    let docs = rows.first().and_then(El::info).and_then(|i| i.semantics).unwrap_or_else(|| unreachable!());
    let dbg = format!("{docs:?} {letters:?}");
    assert!(dbg.contains("level: Some(1)") || dbg.contains("level: 1"), "{dbg}");
    assert!(dbg.contains("level: Some(2)") || dbg.contains("level: 2"), "{dbg}");
    assert!(dbg.contains("expanded: Some(true)"), "{dbg}");
    let problems = audit(&t.ui);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn typing_jumps_to_the_matching_item() {
    let t = tree(true);
    let typed = |c: &str| {
        t.ui.key(
            &remedy_ui_core::input::KeyEvent::press(Key::Character(c.into()))
                .with_modifiers(NONE)
                .with_text(Some(c.into())),
        )
    };
    assert!(typed("r"));
    frame(&t.ui);
    assert_eq!(cursor(&t).as_deref(), Some("/readme.txt"));
    // Keys typed in quick succession add up to one search ("re").
    assert!(typed("e"));
    frame(&t.ui);
    assert_eq!(cursor(&t).as_deref(), Some("/readme.txt"));
    // After a pause the search starts over.
    std::thread::sleep(Duration::from_millis(1_100));
    assert!(typed("m"));
    assert!(typed("u"));
    frame(&t.ui);
    assert_eq!(cursor(&t).as_deref(), Some("/music"));
}

#[test]
fn cut_and_paste_moves_items_and_refusals_are_announced() {
    let t = tree(true);
    t.state.expand("/docs".into());
    frame(&t.ui);
    // Select notes.md, cut it, paste into music.
    t.state.selection.select_only("/docs/notes.md".into());
    frame(&t.ui);
    assert!(key(&t.ui, Key::Character("x".into()), CTRL));
    assert_eq!(t.state.cut.get_untracked().ok(), Some(vec!["/docs/notes.md".to_owned()]));
    let notes = find(&t.tree, "tree_item")
        .into_iter()
        .find(|r| texts(r).first().is_some_and(|s| s == "notes.md"))
        .unwrap_or_else(|| unreachable!());
    assert_eq!(notes.info().map(|i| i.variant), Some("cut"));
    t.state.selection.select_only("/music".into());
    frame(&t.ui);
    assert!(key(&t.ui, Key::Character("v".into()), CTRL));
    assert_eq!(t.disk.folders.borrow().get("/music").map(Vec::len), Some(2));
    assert!(t.state.cut.get_untracked().is_ok_and(|c| c.is_empty()));
    // The app can refuse; the tree says why, as an alert.
    t.state.selection.select_only("/docs/cv.pdf".into());
    frame(&t.ui);
    assert!(key(&t.ui, Key::Character("x".into()), CTRL));
    t.state.selection.select_only("/music".into());
    frame(&t.ui);
    assert!(key(&t.ui, Key::Character("v".into()), CTRL));
    let alert = find(&t.tree, "label").into_iter().find(|l| {
        l.info().and_then(|i| i.semantics).is_some_and(|s| s.role() == remedy_ui_core::a11y::Role::Alert)
    });
    assert_eq!(alert.and_then(|a| a.info()).and_then(|i| i.text).as_deref(), Some("Music holds only songs"));
    // An item cannot move into itself.
    t.state.selection.select_only("/docs".into());
    frame(&t.ui);
    assert!(key(&t.ui, Key::Character("x".into()), CTRL));
    t.state.selection.select_only("/docs/letters".into());
    frame(&t.ui);
    assert!(key(&t.ui, Key::Character("v".into()), CTRL));
    assert!(t.disk.folders.borrow().get("/docs/letters").is_some_and(|v| v.len() == 1));
}

#[test]
fn dragging_items_onto_a_folder_moves_them() {
    let t = tree(true);
    let rows = find(&t.tree, "tree_item");
    let (readme, docs) = (
        rows.get(2).cloned().unwrap_or_else(|| unreachable!()),
        rows.first().cloned().unwrap_or_else(|| unreachable!()),
    );
    let (from, to) = (centre(&readme), centre(&docs));
    pointer(&t.ui, from, PointerPhase::Down, Buttons::PRIMARY, Duration::from_secs(1));
    pointer(
        &t.ui,
        Point::new(from.x, from.y - 10.0),
        PointerPhase::Move,
        Buttons::PRIMARY,
        Duration::from_secs(1),
    );
    pointer(&t.ui, to, PointerPhase::Move, Buttons::PRIMARY, Duration::from_secs(1));
    frame(&t.ui);
    let docs_now = find(&t.tree, "tree_item").into_iter().next().unwrap_or_else(|| unreachable!());
    assert!(
        docs_now.info().is_some_and(|i| i.states.contains(remedy_ui_core::style::States::DROP_TARGET)),
        "the folder under the drag lights up"
    );
    pointer(&t.ui, to, PointerPhase::Up, Buttons::default(), Duration::from_secs(1));
    frame(&t.ui);
    assert_eq!(labels(&t), ["docs", "music"]);
    assert!(t.disk.folders.borrow().get("/docs").is_some_and(|v| v.iter().any(|(n, _)| n == "readme.txt")));
}

#[test]
fn a_deep_tree_of_fifty_thousand_items_builds_only_what_shows() {
    let ui = ui();
    let roots: Vec<TreeItem<u32>> = (0..50_000).map(|i| TreeItem::leaf(i, format!("item {i}"))).collect();
    let state = TreeState::new(roots, false);
    let tree = tree_view(&ui, "Many", &state, |_| {}, |_| {}, None);
    ui.root().child(tree.clone().fill_width());
    frame(&ui);
    frame(&ui);
    let built = find(&tree, "tree_item").len();
    assert!(built > 5 && built < 120, "{built}");
    let body = find(&tree, "scroll").into_iter().next().unwrap_or_else(|| unreachable!());
    ui.set_scroll_offset(body.id(), Vec2::new(0.0, 1_000_000.0));
    frame(&ui);
    assert!(find(&tree, "tree_item").len() < 120);
}

#[test]
fn property_grids_group_and_filter() {
    let ui = ui();
    let name = Signal::new(String::from("Hearth"));
    let on = Signal::new(true);
    let groups = vec![
        PropertyGroup::new(
            "General",
            vec![
                Property::new("Name", w::text_input(&ui, "Name").el()).describe("What peers see"),
                Property::new("Start at login", w::switch(&ui, "Start at login", on)),
            ],
        ),
        PropertyGroup::new("Network", vec![Property::new("Port", w::text_input(&ui, "Port").el())])
            .collapsed(),
    ];
    let _ = name;
    let grid = property_grid(&ui, "Settings", groups);
    ui.root().child(grid.clone().fill_width());
    frame(&ui);
    let shown = |g: &El| -> Vec<String> {
        find(g, "property_row")
            .iter()
            .filter(|r| r.bounds().is_some_and(|b| b.height() > 0.0))
            .filter_map(|r| r.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)))
            .collect()
    };
    assert_eq!(shown(&grid), ["Name", "Start at login"], "Network starts collapsed");
    // Opening a group from its heading.
    let heading = find(&grid, "property_group").get(1).cloned().unwrap_or_else(|| unreachable!());
    click(&ui, centre(&heading), Duration::from_secs(1));
    assert_eq!(shown(&grid), ["Name", "Start at login", "Port"]);
    // The filter matches names and descriptions, any case, and hides empty groups.
    let field = find(&grid, "text_input").into_iter().next().unwrap_or_else(|| unreachable!());
    field.focus();
    ui.paste("PEERS");
    frame(&ui);
    assert_eq!(shown(&grid), ["Name"]);
    assert!(find(&grid, "property_section").get(1).and_then(El::bounds).is_none_or(|b| b.height() == 0.0));
    let problems = audit(&ui);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn files_from_other_apps_drop_into_folders_and_items_drag_out_as_files() {
    use remedy_ui_view::{DragData, DragEvent, DropEffect, Request};
    let t = tree(true);
    /// Files dropped, and the folder they went into.
    type Imports = Vec<(Vec<std::path::PathBuf>, String)>;
    let got: Rc<RefCell<Imports>> = Rc::default();
    let g2 = got.clone();
    t.state.set_import(move |d, folder| {
        g2.borrow_mut().push((d.files.clone(), folder.clone()));
        Ok(())
    });
    t.state.set_export(|keys| {
        DragData::files(keys.iter().map(|k| std::path::PathBuf::from(format!("C:/disk{k}"))).collect())
    });
    frame(&t.ui);
    let rows = find(&t.tree, "tree_item");
    let music = centre(rows.get(1).unwrap_or_else(|| unreachable!()));
    let readme = centre(rows.get(2).unwrap_or_else(|| unreachable!()));
    let data = DragData::files(vec!["D:/song.ogg".into()]);
    assert_eq!(
        t.ui.drag_event(&DragEvent::Enter { position: Some(music), data: data.clone() }),
        DropEffect::Copy
    );
    // A file (no children) takes nothing.
    assert_eq!(t.ui.drag_event(&DragEvent::Over { position: readme }), DropEffect::None);
    assert_eq!(t.ui.drag_event(&DragEvent::Drop { position: Some(music), data }), DropEffect::Copy);
    assert_eq!(*got.borrow(), vec![(vec![std::path::PathBuf::from("D:/song.ogg")], "/music".to_owned())]);
    // Dragging readme.txt out of the window hands its file to the platform.
    let _ = t.ui.take_requests();
    pointer(&t.ui, readme, PointerPhase::Down, Buttons::PRIMARY, Duration::from_secs(1));
    pointer(
        &t.ui,
        Point::new(readme.x, readme.y + 10.0),
        PointerPhase::Move,
        Buttons::PRIMARY,
        Duration::from_secs(1),
    );
    pointer(&t.ui, Point::new(900.0, readme.y), PointerPhase::Move, Buttons::PRIMARY, Duration::from_secs(1));
    let out = t.ui.take_requests().into_iter().find_map(|r| match r {
        Request::StartDrag { data, .. } => Some(data.files),
        _ => None,
    });
    assert_eq!(out, Some(vec![std::path::PathBuf::from("C:/disk/readme.txt")]));
    t.ui.drag_finished(DropEffect::Copy);
}

#[test]
fn assistive_technology_selects_opens_and_closes_rows() {
    let t = tree(true);
    let row = |t: &T, i: usize| find(&t.tree, "tree_item").get(i).map(El::node_id).unwrap_or_default();
    // Click selects the row, as a press does.
    let music = row(&t, 1);
    assert!(t.ui.accessibility_action(music, remedy_ui_core::a11y::Action::Click));
    frame(&t.ui);
    assert_eq!(cursor(&t).as_deref(), Some("/music"));
    assert_eq!(t.state.selection.selected.get(), ["/music".to_string()]);
    // Expand and Collapse open and close a folder.
    let docs = row(&t, 0);
    assert!(t.ui.accessibility_action(docs, remedy_ui_core::a11y::Action::Expand));
    frame(&t.ui);
    assert_eq!(labels(&t), ["docs", "letters", "cv.pdf", "notes.md", "music", "readme.txt"]);
    let docs = row(&t, 0);
    let sem = find(&t.tree, "tree_item").first().and_then(El::info).and_then(|i| i.semantics);
    assert!(sem.is_some_and(
        |s| s.expanded_state() == Some(true) && s.actions().contains(&remedy_ui_core::a11y::Action::Collapse)
    ));
    assert!(t.ui.accessibility_action(docs, remedy_ui_core::a11y::Action::Collapse));
    frame(&t.ui);
    assert_eq!(labels(&t), ["docs", "music", "readme.txt"]);
    // A file has nothing to open.
    let readme = row(&t, 2);
    assert!(!t.ui.accessibility_action(readme, remedy_ui_core::a11y::Action::Expand));
}
