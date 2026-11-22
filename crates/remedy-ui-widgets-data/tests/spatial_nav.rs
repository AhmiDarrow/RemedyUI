//! Spatial navigation over a grid of tiles and a page of shelves: arrows move to the nearest
//! focusable that way, focus groups remember where they were left, dialogs keep focus, and
//! widgets that take arrows themselves keep them.

mod common;

use common::{NONE, audit, frame, key, ui};
use remedy_ui_input::Key;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

fn tile(ui: &Ui, name: String) -> El {
    w::button(ui, name, || {}).width(120.0).height(80.0)
}

/// Tiles by row and column.
struct Cells(Vec<Vec<El>>);

impl Cells {
    fn row(&self, r: usize) -> &[El] {
        self.0.get(r).map_or(&[], Vec::as_slice)
    }

    fn at(&self, r: usize, c: usize) -> &El {
        self.row(r).get(c).unwrap_or_else(|| unreachable!("tile {r}-{c}"))
    }
}

/// A 3 by 3 grid of tiles.
fn grid(ui: &Ui) -> (El, Cells) {
    let mut tiles = Vec::new();
    let mut grid = w::column(ui).gap(10.0);
    for r in 0..3 {
        let mut row = w::row(ui).gap(10.0);
        let mut cells = Vec::new();
        for c in 0..3 {
            let t = tile(ui, format!("Tile {r}-{c}"));
            row = row.child(t.clone());
            cells.push(t);
        }
        grid = grid.child(row);
        tiles.push(cells);
    }
    (grid, Cells(tiles))
}

fn on(ui: &Ui) -> &Ui {
    ui.set_spatial_navigation(true);
    ui
}

#[test]
fn arrows_walk_a_grid() {
    let ui = ui();
    let (g, t) = grid(on(&ui));
    ui.root().child(g);
    frame(&ui);
    // With nothing focused, the first arrow starts at the first tile.
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(t.at(0, 0).is_focused());
    assert!(key(&ui, Key::ArrowRight, NONE));
    assert!(t.at(0, 1).is_focused());
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(t.at(1, 1).is_focused(), "straight down, not diagonally");
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(t.at(2, 1).is_focused());
    assert!(!key(&ui, Key::ArrowDown, NONE), "nothing below the last row: the key goes on");
    assert!(t.at(2, 1).is_focused());
    assert!(key(&ui, Key::ArrowLeft, NONE));
    assert!(t.at(2, 0).is_focused());
    assert!(key(&ui, Key::ArrowUp, NONE));
    assert!(t.at(1, 0).is_focused());
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn off_by_default_and_modified_arrows_stay_keys() {
    let ui = ui();
    let (g, t) = grid(&ui);
    ui.root().child(g);
    frame(&ui);
    t.at(0, 0).focus();
    assert!(!ui.spatial_navigation(), "desktop default");
    assert!(!key(&ui, Key::ArrowRight, NONE));
    assert!(t.at(0, 0).is_focused());
    ui.set_spatial_navigation(true);
    assert!(!key(&ui, Key::ArrowRight, common::SHIFT), "Shift+arrow is for selection");
    assert!(t.at(0, 0).is_focused());
    // The app's choice wins over the system's.
    ui.set_system_spatial_navigation(false);
    assert!(ui.spatial_navigation());
}

#[test]
fn the_system_turns_it_on_until_the_app_chooses() {
    let ui = ui();
    ui.set_system_spatial_navigation(true);
    assert!(ui.spatial_navigation(), "a TV or touch-only browser");
    ui.set_spatial_navigation(false);
    assert!(!ui.spatial_navigation());
}

/// Shelves: rows of cards one under another in a scrolling page, the second row shifted so
/// nothing lines up, each row a focus group.
#[test]
fn shelves_pick_the_nearest_and_remember_their_place() {
    let ui = ui();
    on(&ui);
    let mut page = w::column(&ui).gap(20.0).scroll().height(300.0);
    let mut shelves = Vec::new();
    for s in 0..6 {
        let mut row = w::row(&ui).gap(10.0).focus_group();
        let mut cards = Vec::new();
        if s % 2 == 1 {
            row = row.child(ui.el("box").width(70.0).height(80.0));
        }
        for c in 0..5 {
            let t = tile(&ui, format!("Card {s}-{c}"));
            row = row.child(t.clone());
            cards.push(t);
        }
        page = page.child(row);
        shelves.push(cards);
    }
    let shelves = Cells(shelves);
    ui.root().child(page.clone());
    frame(&ui);
    shelves.at(0, 2).focus();
    frame(&ui);
    // Card 0-2 spans 260..380; on the shifted shelf card 1-1 spans 330..450 and 1-2 spans
    // 460..580: the one that overlaps most is nearest.
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(shelves.at(1, 1).is_focused());
    assert!(key(&ui, Key::ArrowRight, NONE));
    assert!(key(&ui, Key::ArrowRight, NONE));
    assert!(shelves.at(1, 3).is_focused());
    // Up to shelf 0 returns to where it was left, not to the card nearest 1-3.
    assert!(key(&ui, Key::ArrowUp, NONE));
    assert!(shelves.at(0, 2).is_focused(), "the shelf remembered its card");
    // And back down returns to 1-3.
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(shelves.at(1, 3).is_focused());
    // Walking down past the bottom of the page scrolls the card into view.
    for _ in 0..4 {
        assert!(key(&ui, Key::ArrowDown, NONE));
    }
    let focused = shelves.row(5).iter().find(|c| c.is_focused()).cloned();
    let b = focused.unwrap_or_else(|| unreachable!("a card on the last shelf")).bounds().unwrap_or_default();
    let view = page.bounds().unwrap_or_default();
    assert!(ui.scroll_offset(page.id()).y > 0.0, "the page scrolled");
    assert!(b.min.y >= view.min.y - 0.5 && b.max.y <= view.max.y + 0.5, "card at {b:?} in {view:?}");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn dialogs_keep_arrow_focus_and_sliders_keep_their_arrows() {
    let ui = ui();
    on(&ui);
    let volume = Signal::new(0.5);
    let slider = w::slider(&ui, "Volume", volume, 0.0, 1.0, 0.1);
    let below = w::button(&ui, "Below", || {});
    let behind = w::button(&ui, "Behind", || {});
    ui.root()
        .child(w::column(&ui).gap(10.0).child(behind.clone()).child(slider.clone()).child(below.clone()));
    frame(&ui);
    slider.focus();
    // The slider takes every arrow: its value moves and focus stays.
    assert!(key(&ui, Key::ArrowRight, NONE));
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(key(&ui, Key::ArrowUp, NONE));
    assert!(slider.is_focused());
    assert!((volume.get_untracked().unwrap_or(0.0) - 0.6).abs() < 1e-9);

    let ok = w::button(&ui, "OK", || {});
    let cancel = w::button(&ui, "Cancel", || {});
    let _dialog =
        w::modal(&ui, "Confirm", w::row(&ui).gap(10.0).child(ok.clone()).child(cancel.clone()), true);
    frame(&ui);
    assert!(ok.is_focused() || cancel.is_focused(), "the dialog took focus");
    ok.focus();
    assert!(key(&ui, Key::ArrowRight, NONE));
    assert!(cancel.is_focused());
    // Nothing in the dialog below or above: the window behind is out of reach.
    assert!(!key(&ui, Key::ArrowDown, NONE));
    assert!(!key(&ui, Key::ArrowUp, NONE));
    assert!(cancel.is_focused() && !below.is_focused() && !behind.is_focused());
}

/// A sidebar group keeps Down inside it though the content beside it is nearer; a wrapping
/// row comes round at its ends and still lets Down out.
#[test]
fn groups_keep_arrows_in_and_wrapping_rows_come_round() {
    let ui = ui();
    on(&ui);
    let (a, b) = (tile(&ui, "A".into()), tile(&ui, "B".into()));
    let beside = tile(&ui, "Beside".into());
    let sidebar = w::column(&ui)
        .gap(10.0)
        .focus_group()
        .child(a.clone())
        .child(ui.el("box").height(200.0))
        .child(b.clone());
    let content = w::column(&ui).gap(10.0).child(ui.el("box").height(90.0)).child(beside.clone());
    let cards: Vec<El> = (0..3).map(|c| tile(&ui, format!("Card {c}"))).collect();
    let mut carousel = w::row(&ui).gap(10.0).focus_wrap();
    for c in &cards {
        carousel = carousel.child(c.clone());
    }
    let cards = Cells(vec![cards]);
    let after = tile(&ui, "After".into());
    ui.root().child(
        w::column(&ui)
            .gap(10.0)
            .child(w::row(&ui).gap(10.0).child(sidebar).child(content))
            .child(carousel)
            .child(after.clone()),
    );
    frame(&ui);
    a.focus();
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(b.is_focused(), "down stays in the sidebar");
    cards.at(0, 2).focus();
    assert!(key(&ui, Key::ArrowRight, NONE));
    assert!(cards.at(0, 0).is_focused(), "past the last card comes round to the first");
    assert!(key(&ui, Key::ArrowLeft, NONE));
    assert!(cards.at(0, 2).is_focused());
    assert!(key(&ui, Key::ArrowDown, NONE));
    assert!(after.is_focused(), "down leaves the row");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}
