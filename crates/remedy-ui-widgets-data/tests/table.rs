//! Tables on a real view with the theme sheet: virtual rows, sorting (on and off the UI
//! thread), selection, keyboard, column resize and reorder, inline editing, accessibility,
//! and the 50,000-file explorer frame budget.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui_core::{Point, Vec2};
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_data::*;

mod common;
use common::*;

#[derive(Clone, Debug, PartialEq)]
struct File {
    id: u64,
    name: String,
    size: u64,
    kind: &'static str,
}

fn files(n: usize) -> Vec<File> {
    const KINDS: [&str; 5] = ["Document", "Image", "Folder", "Archive", "Text"];
    (0..n)
        .map(|i| {
            // A spread of names and sizes that does not come pre-sorted.
            let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17);
            File {
                id: i as u64,
                name: format!("file{}", h % 100_000),
                size: h % 10_000_000,
                kind: KINDS.get((h % 5) as usize).copied().unwrap_or("File"),
            }
        })
        .collect()
}

struct Explorer {
    ui: Ui,
    table: El,
    rows: Signal<Vec<File>>,
    state: TableState<u64>,
    opened: Rc<RefCell<Vec<u64>>>,
}

fn explorer(n: usize) -> Explorer {
    let ui = ui();
    let rows = Signal::new(files(n));
    let state = TableState::new(true);
    let opened: Rc<RefCell<Vec<u64>>> = Rc::default();
    let o2 = opened.clone();
    let columns = vec![
        Column::text("name", "Name", 240.0, |f: &File| f.name.clone()).editable(
            |f| f.name.clone(),
            move |f, new| {
                if new.trim().is_empty() {
                    return Err("A name cannot be empty".into());
                }
                rows.update(|v| {
                    if let Some(r) = v.iter_mut().find(|r| r.id == f.id) {
                        r.name = new;
                    }
                });
                Ok(())
            },
        ),
        Column::new("size", "Size", 120.0, |ui, f: &File| {
            w::text(ui, format!("{} KB", f.size / 1024)).data_text().nowrap()
        })
        .sort_by(|f| SortValue::Number(f.size as f64))
        .align_end(),
        Column::text("kind", "Kind", 160.0, |f: &File| f.kind.to_owned()),
    ];
    let table =
        table(&ui, "Files", rows, |f: &File| f.id, columns, &state, move |k| o2.borrow_mut().push(*k));
    let table = table.fill_width();
    ui.root().child(table.clone());
    frame(&ui);
    frame(&ui);
    table.focus();
    frame(&ui);
    Explorer { ui, table, rows, state, opened }
}

fn body(t: &Explorer) -> El {
    // The main pane: the last scroll container with rows (the frozen pane comes first).
    find(&t.table, "scroll")
        .into_iter()
        .rfind(|s| find(s, "table_row").len() > 1)
        .unwrap_or_else(|| unreachable!("no body"))
}

fn shown_names(t: &Explorer) -> Vec<String> {
    find(&body(t), "table_row").iter().map(|r| texts(r).first().cloned().unwrap_or_default()).collect()
}

#[test]
fn fifty_thousand_rows_build_only_what_shows() {
    let t = explorer(50_000);
    let built = find(&t.table, "table_row").len();
    assert!(built > 10 && built < 120, "built {built} rows for a 600 px window");
    // Scrolling to the middle builds the rows there, and drops the ones at the top.
    let b = body(&t);
    let h = remedy_ui_widgets_core::MIN_TARGET.max(t.ui.theme().spacing.control_height);
    t.ui.set_scroll_offset(b.id(), Vec2::new(0.0, 25_000.0 * h));
    frame(&t.ui);
    let names = shown_names(&t);
    let want = files(50_000).get(25_000).map(|f| f.name.clone()).unwrap_or_default();
    assert!(names.contains(&want), "row 25000 not built after scrolling to it");
    assert!(find(&t.table, "table_row").len() < 120);
    // The content is as tall as every row.
    let content = b.child_elements().first().and_then(El::bounds).unwrap_or_default();
    assert!((content.height() - 50_000.0 * h).abs() < 1.0, "content height {}", content.height());
}

#[test]
fn headings_sort_and_say_how() {
    let t = explorer(200);
    let headings = find(&t.table, "table_heading");
    assert_eq!(headings.len(), 3);
    let name = headings.first().cloned().unwrap_or_else(|| unreachable!());
    click(&t.ui, centre(&name), Duration::from_secs(1));
    assert_eq!(t.state.sort.get_untracked().ok().flatten(), Some(("name".into(), SortDirection::Ascending)));
    let mut all: Vec<String> = files(200).into_iter().map(|f| f.name).collect();
    all.sort_by(|a, b| natural_cmp(a, b));
    let shown = shown_names(&t);
    assert_eq!(shown.get(..5), all.get(..5));
    let heading = find(&t.table, "table_heading").into_iter().next().unwrap_or_else(|| unreachable!());
    let desc =
        heading.info().and_then(|i| i.semantics).map(|s| s.description_text().unwrap_or_default().to_owned());
    assert_eq!(desc.as_deref(), Some("sorted ascending"));
    // Again: descending, then unsorted.
    click(&t.ui, centre(&heading), Duration::from_secs(3));
    assert_eq!(t.state.sort.get_untracked().ok().flatten(), Some(("name".into(), SortDirection::Descending)));
    let heading = find(&t.table, "table_heading").into_iter().next().unwrap_or_else(|| unreachable!());
    click(&t.ui, centre(&heading), Duration::from_secs(5));
    assert_eq!(t.state.sort.get_untracked().ok().flatten(), None);
    assert_eq!(shown_names(&t).first(), files(200).first().map(|f| &f.name));
}

#[test]
fn large_sorts_run_off_the_ui_thread() {
    let t = explorer(SORT_OFF_THREAD + 5_000);
    let before = shown_names(&t);
    t.state.sort.set(Some(("size".into(), SortDirection::Descending)));
    frame(&t.ui);
    // The UI thread did not sort: the old order still shows until the result arrives.
    let start = Instant::now();
    while !remedy_ui_reactive::has_posted() {
        assert!(start.elapsed() < Duration::from_secs(20), "the sort never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shown_names(&t), before);
    let _ = remedy_ui_reactive::run_posted();
    frame(&t.ui);
    let biggest = files(SORT_OFF_THREAD + 5_000).into_iter().max_by_key(|f| f.size).map(|f| f.name);
    assert_eq!(shown_names(&t).first(), biggest.as_ref());
}

#[test]
fn a_newer_sort_wins_over_one_still_running() {
    let t = explorer(SORT_OFF_THREAD + 5_000);
    t.state.sort.set(Some(("size".into(), SortDirection::Descending)));
    frame(&t.ui);
    t.state.sort.set(Some(("size".into(), SortDirection::Ascending)));
    frame(&t.ui);
    let start = Instant::now();
    let mut done = 0;
    while done < 2 {
        assert!(start.elapsed() < Duration::from_secs(20), "the sorts never finished");
        if remedy_ui_reactive::has_posted() {
            done += remedy_ui_reactive::run_posted().ran;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    frame(&t.ui);
    let smallest = files(SORT_OFF_THREAD + 5_000).into_iter().min_by_key(|f| (f.size, f.id)).map(|f| f.name);
    assert_eq!(shown_names(&t).first(), smallest.as_ref());
}

#[test]
fn clicks_and_keys_select() {
    let t = explorer(100);
    let rows = find(&body(&t), "table_row");
    let third = rows.get(2).cloned().unwrap_or_else(|| unreachable!());
    click(&t.ui, centre(&third), Duration::from_secs(1));
    assert_eq!(t.state.selection.selected.get_untracked().ok(), Some(vec![2]));
    assert!(third.info().is_some_and(|i| i.states.contains(remedy_ui_style::States::SELECTED)));
    // Arrows move the selection; Shift extends it from the anchor.
    assert!(key(&t.ui, Key::ArrowDown, NONE));
    assert_eq!(t.state.selection.selected.get_untracked().ok(), Some(vec![3]));
    assert!(key(&t.ui, Key::ArrowDown, SHIFT));
    assert!(key(&t.ui, Key::ArrowDown, SHIFT));
    assert_eq!(t.state.selection.selected.get_untracked().ok(), Some(vec![3, 4, 5]));
    // Ctrl moves the cursor alone, Space adds the row.
    assert!(key(&t.ui, Key::ArrowDown, CTRL));
    assert!(key(&t.ui, Key::ArrowDown, CTRL));
    assert!(key(&t.ui, Key::Space, NONE));
    assert_eq!(t.state.selection.selected.get_untracked().ok(), Some(vec![3, 4, 5, 7]));
    assert!(key(&t.ui, Key::Character("a".into()), CTRL));
    assert_eq!(t.state.selection.selected.get_untracked().ok().map(|s| s.len()), Some(100));
    // Enter opens the cursor row; so does a double click.
    assert!(key(&t.ui, Key::Enter, NONE));
    assert_eq!(*t.opened.borrow(), vec![7]);
    let first = find(&body(&t), "table_row").into_iter().next().unwrap_or_else(|| unreachable!());
    // On the Size column, which is not editable.
    let b = first.bounds().unwrap_or_default();
    let at = Point::new(b.min.x + 300.0, (b.min.y + b.max.y) / 2.0);
    click(&t.ui, at, Duration::from_secs(10));
    click(&t.ui, at, Duration::from_millis(10_100));
    assert_eq!(*t.opened.borrow(), vec![7, 0]);
}

#[test]
fn the_cursor_stays_in_view() {
    let t = explorer(1_000);
    assert!(key(&t.ui, Key::Home, NONE));
    assert!(key(&t.ui, Key::End, NONE));
    assert_eq!(t.state.selection.cursor.get_untracked().ok().flatten(), Some(999));
    assert!(shown_names(&t).contains(&files(1_000).last().map(|f| f.name.clone()).unwrap_or_default()));
    assert!(key(&t.ui, Key::PageUp, NONE));
    let cursor = t.state.selection.cursor.get_untracked().ok().flatten().unwrap_or_default();
    assert!(cursor < 999 && cursor > 950, "page up went to {cursor}");
}

#[test]
fn columns_resize_and_move_by_pointer_and_keyboard() {
    let t = explorer(50);
    let name = find(&t.table, "table_heading").into_iter().next().unwrap_or_else(|| unreachable!());
    let w0 = name.bounds().map(|b| b.width()).unwrap_or_default();
    // Drag the grip 60 px right; the press is captured so the drag can leave the grip.
    let grip = find(&name, "column_grip").into_iter().next().unwrap_or_else(|| unreachable!());
    let g = centre(&grip);
    pointer(&t.ui, g, PointerPhase::Down, Buttons::PRIMARY, Duration::from_secs(1));
    pointer(
        &t.ui,
        Point::new(g.x + 60.0, g.y + 200.0),
        PointerPhase::Move,
        Buttons::PRIMARY,
        Duration::from_secs(1),
    );
    pointer(
        &t.ui,
        Point::new(g.x + 60.0, g.y + 200.0),
        PointerPhase::Up,
        Buttons::default(),
        Duration::from_secs(1),
    );
    frame(&t.ui);
    let name = find(&t.table, "table_heading").into_iter().next().unwrap_or_else(|| unreachable!());
    let w1 = name.bounds().map(|b| b.width()).unwrap_or_default();
    assert!((w1 - w0 - 60.0).abs() < 1.0, "{w0} -> {w1}");
    // The cells follow their heading.
    let cell = find(&body(&t), "table_cell").into_iter().next().unwrap_or_else(|| unreachable!());
    assert!((cell.bounds().map(|b| b.width()).unwrap_or_default() - w1).abs() < 1.0);
    // Keyboard: Ctrl+Left narrows, Alt+Shift+Right moves.
    name.focus();
    assert!(key(&t.ui, Key::ArrowLeft, CTRL));
    let name = find(&t.table, "table_heading").into_iter().next().unwrap_or_else(|| unreachable!());
    assert!((name.bounds().map(|b| b.width()).unwrap_or_default() - (w1 - 16.0)).abs() < 1.0);
    name.focus();
    assert!(key(&t.ui, Key::ArrowRight, ALT_SHIFT));
    assert_eq!(t.state.order.get_untracked().ok(), Some(vec!["size".into(), "name".into(), "kind".into()]));
    // Drag the Kind heading onto Size.
    let hs = find(&t.table, "table_heading");
    let (size, kind) = (
        hs.first().cloned().unwrap_or_else(|| unreachable!()),
        hs.get(2).cloned().unwrap_or_else(|| unreachable!()),
    );
    pointer(&t.ui, centre(&kind), PointerPhase::Down, Buttons::PRIMARY, Duration::from_secs(3));
    pointer(&t.ui, centre(&size), PointerPhase::Move, Buttons::PRIMARY, Duration::from_secs(3));
    pointer(&t.ui, centre(&size), PointerPhase::Up, Buttons::default(), Duration::from_secs(3));
    frame(&t.ui);
    assert_eq!(t.state.order.get_untracked().ok(), Some(vec!["kind".into(), "size".into(), "name".into()]));
}

#[test]
fn frozen_columns_stay_while_the_rest_scrolls_sideways() {
    let t = explorer(50);
    t.state.frozen.set(1);
    t.state.widths.update(|m| {
        m.insert("kind".into(), 900.0);
    });
    frame(&t.ui);
    let b = body(&t);
    let first_cell_x = |t: &Explorer| {
        find(&body(t), "table_cell")
            .into_iter()
            .next()
            .and_then(|c| c.bounds())
            .map(|r| r.min.x)
            .unwrap_or_default()
    };
    let frozen_x = |t: &Explorer| {
        find(&t.table, "scroll")
            .into_iter()
            .find(|s| s.id() != body(t).id() && find(s, "table_row").len() > 1)
            .and_then(|s| find(&s, "table_cell").into_iter().next())
            .and_then(|c| c.bounds())
            .map(|r| r.min.x)
    };
    let (main0, frozen0) = (first_cell_x(&t), frozen_x(&t));
    assert!(frozen0.is_some(), "no frozen pane");
    t.ui.set_scroll_offset(b.id(), Vec2::new(300.0, 0.0));
    frame(&t.ui);
    assert!((first_cell_x(&t) - (main0 - 300.0)).abs() < 1.0);
    assert_eq!(frozen_x(&t), frozen0);
}

#[test]
fn cells_edit_in_place() {
    let t = explorer(20);
    assert!(key(&t.ui, Key::Home, NONE));
    assert!(key(&t.ui, Key::F(2), NONE));
    let field = find(&t.table, "text_input").into_iter().next().unwrap_or_else(|| unreachable!("no editor"));
    assert!(field.is_focused());
    // An empty name is refused and the field says so.
    assert!(key(&t.ui, Key::Character("a".into()), CTRL));
    assert!(key(&t.ui, Key::Backspace, NONE));
    assert!(key(&t.ui, Key::Enter, NONE));
    assert!(field.info().is_some_and(|i| i.states.contains(remedy_ui_style::States::INVALID)));
    assert!(key(&t.ui, Key::Character("a".into()), CTRL));
    t.ui.paste("renamed.txt");
    frame(&t.ui);
    assert!(key(&t.ui, Key::Enter, NONE));
    assert!(find(&t.table, "text_input").is_empty());
    assert_eq!(t.rows.with(|r| r.first().map(|f| f.name.clone())), Some("renamed.txt".into()));
    assert_eq!(shown_names(&t).first().map(String::as_str), Some("renamed.txt"));
    // Escape leaves without saving.
    assert!(key(&t.ui, Key::F(2), NONE));
    assert!(key(&t.ui, Key::Escape, NONE));
    assert!(find(&t.table, "text_input").is_empty());
}

#[test]
fn the_table_passes_the_accessibility_audit() {
    let t = explorer(300);
    let found = audit(&t.ui);
    assert!(found.is_empty(), "{found:#?}");
}

/// The 50,000-file explorer scene's frame budget: scrolling through it costs at most 8 ms of
/// UI-thread work a frame (build, layout and paint), however far it goes. Measured in
/// release builds; debug builds are not what ships.
#[test]
#[cfg_attr(debug_assertions, ignore = "frame budgets are measured in release builds")]
fn the_fifty_thousand_file_explorer_meets_its_frame_budget() {
    let t = explorer(50_000);
    t.state.sort.set(Some(("name".into(), SortDirection::Ascending)));
    let start = Instant::now();
    while !remedy_ui_reactive::has_posted() {
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = remedy_ui_reactive::run_posted();
    frame(&t.ui);
    let b = body(&t);
    let h = remedy_ui_widgets_core::MIN_TARGET.max(t.ui.theme().spacing.control_height);
    let mut times = Vec::new();
    // A steady scroll (a few rows a frame), then flicks (two and a half screens a frame,
    // so every row is new every frame).
    let mut y = 0.0f32;
    for i in 0..400 {
        y += if i < 200 { h * 3.0 } else { 600.0 * 2.5 };
        let y = y.min(50_000.0 * h - 600.0);
        let s = Instant::now();
        t.ui.set_scroll_offset(b.id(), Vec2::new(0.0, y));
        frame(&t.ui);
        times.push(s.elapsed());
    }
    let steady_worst = times.get(..200).and_then(|s| s.iter().max()).copied().unwrap_or_default();
    eprintln!("50k explorer steady scroll worst {steady_worst:?}");
    assert!(steady_worst <= Duration::from_millis(8), "a steady-scroll frame took {steady_worst:?}");
    times.sort();
    let p95 = times.get(times.len() * 95 / 100).copied().unwrap_or_default();
    let worst = times.last().copied().unwrap_or_default();
    eprintln!("50k explorer scroll: p50 {:?}, p95 {p95:?}, worst {worst:?}", times.get(times.len() / 2));
    assert!(p95 <= Duration::from_millis(8), "p95 frame {p95:?} over the 8 ms budget");
}

#[test]
fn rows_drag_onto_rows_and_files_drop_on_the_table() {
    use remedy_ui_view::{Drag, DragData, DragEvent, DropEffect};
    let ui = ui();
    let rows = Signal::new(files(10));
    let state: TableState<u64> = TableState::new(true);
    /// What was dropped (rows carried) and on which row.
    type Drops = Vec<(Vec<u64>, Option<u64>)>;
    let moved: Rc<RefCell<Drops>> = Rc::default();
    state.set_drag(|keys| Some(Drag::new(DragData::text(format!("{keys:?}"))).with_local(keys.to_vec())));
    let m2 = moved.clone();
    state.set_drop(
        |d, target| match (d.local::<Vec<u64>>(), target) {
            // Rows move onto even rows only (as onto folders); files land on the table.
            (Some(_), Some(k)) if k % 2 == 0 => DropEffect::Move,
            (None, None) if !d.data.files.is_empty() => DropEffect::Copy,
            _ => DropEffect::None,
        },
        move |d, target, _| {
            m2.borrow_mut().push((d.local::<Vec<u64>>().cloned().unwrap_or_default(), target.copied()))
        },
    );
    let columns = vec![Column::text("name", "Name", 240.0, |f: &File| f.name.clone())];
    let t = table(&ui, "Files", rows, |f: &File| f.id, columns, &state, |_| {});
    ui.root().child(t.clone().fill_width());
    frame(&ui);
    frame(&ui);
    let body_rows = find(&t, "table_row");
    let at = |i: usize| body_rows.get(i).map(centre).unwrap_or_else(|| unreachable!("row {i}"));
    let (r1, r2, r4) = (at(1), at(2), at(4));
    // Rows 1 and 3 are selected; dragging from row 1 carries both, onto row 4.
    state.selection.selected.set(vec![1, 3]);
    frame(&ui);
    pointer(&ui, r1, PointerPhase::Down, Buttons::PRIMARY, Duration::from_secs(5));
    pointer(
        &ui,
        remedy_ui_core::Point::new(r1.x, r1.y + 12.0),
        PointerPhase::Move,
        Buttons::PRIMARY,
        Duration::from_secs(5),
    );
    // Past row 2, then onto row 4.
    pointer(&ui, r2, PointerPhase::Move, Buttons::PRIMARY, Duration::from_secs(5));
    pointer(&ui, r4, PointerPhase::Move, Buttons::PRIMARY, Duration::from_secs(5));
    pointer(&ui, r4, PointerPhase::Up, Buttons::default(), Duration::from_secs(5));
    assert_eq!(*moved.borrow(), vec![(vec![1, 3], Some(4))]);
    assert_eq!(
        state.selection.selected.get_untracked().ok(),
        Some(vec![1, 3]),
        "the drag kept the selection"
    );
    // Files from another app, over a row that refuses them: the table takes them.
    let data = DragData::files(vec!["D:/a.png".into()]);
    assert_eq!(ui.drag_event(&DragEvent::Enter { position: Some(r2), data: data.clone() }), DropEffect::Copy);
    assert_eq!(ui.drag_event(&DragEvent::Drop { position: Some(r2), data }), DropEffect::Copy);
    assert_eq!(moved.borrow().last(), Some(&(Vec::new(), None)));
}
