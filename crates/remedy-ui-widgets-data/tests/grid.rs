//! Icon grids on a real view with the theme sheet: wrapping to the width, virtual rows,
//! selection by click, keyboard and rubber band, activation, and accessibility.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::{Point, Size, Vec2};
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_data::*;

mod common;
use common::*;

const TILE: Size = Size { width: 120.0, height: 96.0 };

struct G {
    ui: Ui,
    grid: El,
    items: Signal<Vec<u32>>,
    state: GridState<u32>,
    opened: Rc<RefCell<Vec<u32>>>,
}

fn grid(n: u32) -> G {
    let ui = ui();
    let items = Signal::new((0..n).collect::<Vec<u32>>());
    let state = GridState::new(true);
    let opened: Rc<RefCell<Vec<u32>>> = Rc::default();
    let o2 = opened.clone();
    let tiles = Tiles::new(TILE, |ui: &Ui, i: &u32| w::text(ui, format!("item {i}")).nowrap());
    let g = grid_view(&ui, "Files", items, |i: &u32| *i, tiles, &state, move |k| o2.borrow_mut().push(*k));
    let g = g.fill_width();
    ui.root().child(g.clone());
    frame(&ui);
    frame(&ui);
    g.focus();
    frame(&ui);
    G { ui, grid: g, items, state, opened }
}

fn tiles(g: &G) -> Vec<El> {
    find(&g.grid, "grid_tile")
}

fn tile(g: &G, i: u32) -> El {
    tiles(g)
        .into_iter()
        .find(|t| texts(t).first().is_some_and(|s| *s == format!("item {i}")))
        .unwrap_or_else(|| unreachable!("item {i} not built"))
}

fn selected(g: &G) -> Vec<u32> {
    g.state.selection.selected.get_untracked().unwrap_or_default()
}

/// Tiles in a row, measured.
fn columns(g: &G) -> usize {
    let y0 = tile(g, 0).bounds().map(|b| b.min.y);
    tiles(g).iter().filter(|t| t.bounds().map(|b| b.min.y) == y0).count()
}

#[test]
fn tiles_wrap_to_the_width_and_only_visible_rows_exist() {
    let g = grid(20_000);
    let cols = columns(&g);
    let unit = g.ui.theme().spacing.unit;
    assert_eq!(cols, GridGeometry::columns_for(800.0, TILE, unit * 2.0, unit * 2.0));
    let built = tiles(&g).len();
    assert!(built >= cols * 4 && built < cols * 30, "built {built} tiles for a 600 px window");
    // Scrolling far down builds the tiles there.
    let body = find(&g.grid, "scroll").into_iter().next().unwrap_or_else(|| unreachable!());
    let row = 10_000 / cols as u32;
    let y = row as f32 * (TILE.height + unit * 2.0);
    g.ui.set_scroll_offset(body.id(), Vec2::new(0.0, y));
    frame(&g.ui);
    let _ = tile(&g, row * cols as u32);
    assert!(tiles(&g).len() < cols * 30);
}

#[test]
fn clicks_and_keys_select_in_two_dimensions() {
    let g = grid(50);
    let cols = columns(&g) as u32;
    click(&g.ui, centre(&tile(&g, 1)), Duration::from_secs(1));
    assert_eq!(selected(&g), vec![1]);
    assert!(tile(&g, 1).info().is_some_and(|i| i.states.contains(remedy_ui_core::style::States::SELECTED)));
    assert!(key(&g.ui, Key::ArrowRight, NONE));
    assert_eq!(selected(&g), vec![2]);
    assert!(key(&g.ui, Key::ArrowDown, NONE));
    assert_eq!(selected(&g), vec![2 + cols]);
    assert!(key(&g.ui, Key::ArrowUp, SHIFT));
    assert_eq!(selected(&g).len(), cols as usize + 1, "Shift selects the range back to the anchor");
    assert!(key(&g.ui, Key::End, NONE));
    assert_eq!(selected(&g), vec![49]);
    // Down from the last row stays put; Ctrl+A takes everything.
    assert!(key(&g.ui, Key::ArrowDown, NONE));
    assert_eq!(selected(&g), vec![49]);
    assert!(key(&g.ui, Key::Character("a".into()), CTRL));
    assert_eq!(selected(&g).len(), 50);
    // Enter activates the cursor's item; a double click activates too.
    assert!(key(&g.ui, Key::Home, NONE));
    assert!(key(&g.ui, Key::Enter, NONE));
    let at = centre(&tile(&g, 3));
    click(&g.ui, at, Duration::from_millis(5000));
    click(&g.ui, at, Duration::from_millis(5100));
    assert_eq!(*g.opened.borrow(), vec![0, 3]);
}

#[test]
fn a_band_selects_what_it_meets_and_a_click_between_tiles_clears() {
    let g = grid(30);
    let cols = columns(&g) as u32;
    let unit = g.ui.theme().spacing.unit;
    // From the gap left of item 0 to the middle of item 1 + cols (one row down).
    let a = tile(&g, 0).bounds().unwrap_or_default();
    let b = centre(&tile(&g, 1 + cols));
    let start = Point::new(a.min.x - unit, a.min.y - unit);
    pointer(&g.ui, start, PointerPhase::Down, Buttons::PRIMARY, Duration::from_secs(1));
    pointer(
        &g.ui,
        Point::new(start.x + 10.0, start.y + 10.0),
        PointerPhase::Move,
        Buttons::PRIMARY,
        Duration::from_secs(1),
    );
    pointer(&g.ui, b, PointerPhase::Move, Buttons::PRIMARY, Duration::from_secs(1));
    frame(&g.ui);
    let band = find(&g.grid, "grid_band").into_iter().next().unwrap_or_else(|| unreachable!());
    assert!(band.info().is_some_and(|i| !i.hidden), "the band shows while dragging");
    pointer(&g.ui, b, PointerPhase::Up, Buttons::default(), Duration::from_secs(1));
    frame(&g.ui);
    let mut got = selected(&g);
    got.sort_unstable();
    assert_eq!(got, vec![0, 1, cols, cols + 1]);
    assert!(band.info().is_some_and(|i| i.hidden), "and goes after");
    // A click on the space between tiles selects nothing.
    click(&g.ui, start, Duration::from_secs(3));
    assert!(selected(&g).is_empty());
}

#[test]
fn removed_items_leave_the_selection_and_the_grid_reads_well() {
    let g = grid(12);
    click(&g.ui, centre(&tile(&g, 4)), Duration::from_secs(1));
    g.items.set((0..4).collect());
    frame(&g.ui);
    assert!(selected(&g).is_empty());
    assert_eq!(tiles(&g).len(), 4);
    let problems = audit(&g.ui);
    assert!(problems.is_empty(), "{problems:#?}");
    let t = tile(&g, 2);
    let s = t.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!());
    assert_eq!(s.role(), remedy_ui_core::a11y::Role::ListItem);
}

#[test]
fn a_grid_and_a_table_shown_after_starting_hidden_settle() {
    let ui = ui();
    let items = Signal::new(Vec::<u32>::new());
    let state = GridState::new(true);
    let tiles = Tiles::new(TILE, |ui: &Ui, i: &u32| w::text(ui, format!("item {i}")).nowrap());
    let g = grid_view(&ui, "Files", items, |i: &u32| *i, tiles, &state, |_| {});
    let tstate: TableState<u32> = TableState::new(true);
    let cols = vec![Column::text("n", "Name", 200.0, |i: &u32| i.to_string())];
    let t = table(&ui, "Files", items, |i: &u32| *i, cols, &tstate, |_| {});
    let holder = w::column(&ui).grow(1.0).fill_width().hidden(true).child(g.clone()).child(t.hidden(true));
    ui.root().child(holder.clone());
    frame(&ui);
    frame(&ui);
    items.set((0..40).collect());
    let _ = holder.clone().hidden(false);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d2 = done.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        if !d2.load(std::sync::atomic::Ordering::SeqCst) {
            eprintln!("the frame did not finish in 10 s");
            std::process::exit(3);
        }
    });
    frame(&ui);
    frame(&ui);
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(find(&g, "grid_tile").len() >= 10);
}

#[test]
fn tiles_take_their_own_gap_and_style_variant() {
    let ui = ui();
    let items = Signal::new((0..4).collect::<Vec<u32>>());
    let state = GridState::new(true);
    let tiles =
        Tiles::fluid(10_000.0, 0.0, 40.0, |ui: &Ui, i: &u32| w::text(ui, format!("item {i}")).nowrap())
            .gap(2.0)
            .variant("row");
    let g = grid_view(&ui, "Files", items, |i: &u32| *i, tiles, &state, |_| {}).fill_width();
    ui.root().child(g.clone());
    frame(&ui);
    frame(&ui);
    let rows = find(&g, "grid_tile");
    let (a, b) = (
        rows.first().and_then(El::bounds).unwrap_or_default(),
        rows.get(1).and_then(El::bounds).unwrap_or_default(),
    );
    assert_eq!(b.min.y - a.max.y, 2.0, "rows sit 2 apart, not the theme's spacing: {a:?} {b:?}");
    assert!(
        rows.iter().all(|r| r.info().is_some_and(|i| i.variant == "row")),
        "each tile box has the variant"
    );
}
