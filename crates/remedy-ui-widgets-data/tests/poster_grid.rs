//! Fluid poster grids (a media library, Portal's `repeat(auto-fill, minmax(180px, 1fr))`):
//! columns at each width, virtual rows inside the page's scroll, tiles whose buttons the
//! keyboard and screen readers reach, the audit, previews at phone and desktop widths, and
//! the 2,000-poster frame budget.

use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_core::a11y::Role;
use remedy_ui_core::{Size, Vec2};
use remedy_ui_core::input::Key;
use remedy_ui_core::layout::Length;
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core::{self as w, Icon};
use remedy_ui_widgets_data::*;

mod common;
use common::*;

const MIN_WIDTH: f32 = 180.0;
const ASPECT: f32 = 1.5;
const EXTRA: f32 = 76.0;
const ACTIONS: [(Icon, &str); 5] = [
    (Icon::Play, "Play"),
    (Icon::ListBullet, "Details"),
    (Icon::Plus, "Watchlist"),
    (Icon::Pin, "Bookmark"),
    (Icon::Info, "Info"),
];

/// A decoded poster: a small gradient of its own, new each time its tile is built (as a
/// poster decoded on arrival would be).
fn poster_image(i: u32) -> remedy_ui_paint::Image {
    let (w, h) = (40u32, 60u32);
    let hue = (i * 47 % 255) as u8;
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let shade = (y * 160 / h) as u8 + 60;
        px.extend_from_slice(&[hue, shade, 255 - hue, 255].repeat(w as usize));
    }
    remedy_ui_paint::Image::new(w, h, Arc::from(px))
}

/// A poster card: its art, its title, and Play, Details, Watchlist, Bookmark and Info.
fn poster(ui: &Ui, i: &u32) -> El {
    let art = ui
        .el("poster_art")
        .fill_width()
        .grow(1.0)
        .layout(|l| l.min_height = Length::Px(0.0))
        .image(poster_image(*i));
    let mut actions = ui.el("poster_actions").row();
    for (icon, label) in ACTIONS {
        actions = actions.child(w::icon_button(ui, icon, format!("{label} film {i}"), || {}));
    }
    ui.el("poster")
        .column()
        .fill_width()
        .grow(1.0)
        .child(art)
        .child(w::text(ui, format!("Film {i}")).nowrap())
        .child(actions)
}

fn options(own_scroll: bool, interactive: bool) -> GridOptions {
    let mut o = GridOptions::default();
    o.own_scroll = own_scroll;
    o.interactive_tiles = interactive;
    o
}

fn posters(ui: &Ui, n: u32, own_scroll: bool, interactive: bool) -> (El, GridState<u32>) {
    let items = Signal::new((0..n).collect::<Vec<u32>>());
    let state = GridState::new(false);
    let tiles = Tiles::fluid(MIN_WIDTH, ASPECT, EXTRA, poster);
    let g = grid_view_with(
        ui,
        "Films",
        items,
        |i: &u32| *i,
        tiles,
        &state,
        options(own_scroll, interactive),
        |_| {},
    );
    (g, state)
}

/// A page that scrolls: a header, then the grid in the page's scroll.
fn page(ui: &Ui, n: u32, interactive: bool) -> (El, El, GridState<u32>) {
    let (g, state) = posters(ui, n, false, interactive);
    let page = ui
        .el("page")
        .scroll()
        .column()
        .fill_width()
        .grow(1.0)
        .layout(|l| l.min_height = Length::Px(0.0))
        .child(w::heading(ui, 1, "Films").height(300.0).layout(|l| l.shrink = 0.0))
        .child(g.clone());
    ui.root().child(page.clone());
    (page, g, state)
}

fn frame_at(ui: &Ui, width: f32) {
    for _ in 0..3 {
        let _ = ui.frame(Size::new(width, 800.0), 1.0);
        for r in ui.take_requests() {
            if let remedy_ui_view::Request::ScrollTo { scroller, offset } = r {
                ui.set_scroll_offset(scroller, offset);
            }
        }
    }
}

fn gap(ui: &Ui) -> f32 {
    ui.theme().spacing.unit * 2.0
}

/// Tiles in the first row, measured.
fn columns(g: &El) -> usize {
    let tiles = find(g, "grid_tile");
    let y0 = tiles.first().and_then(|t| t.bounds()).map(|b| b.min.y);
    tiles.iter().filter(|t| t.bounds().map(|b| b.min.y) == y0).count()
}

#[test]
fn fluid_tiles_fill_the_width_like_auto_fill() {
    // The geometry: CSS auto-fill of minmax(180px, 1fr) with 8 px gaps and padding.
    let g = |w: f32| GridGeometry::fluid(w, MIN_WIDTH, ASPECT, EXTRA, 8.0, 8.0);
    assert_eq!(g(390.0).cols, 2);
    assert_eq!(g(1280.0).cols, 6);
    assert_eq!(g(150.0).cols, 1, "never fewer than one");
    // (1280 - 16 - 5 * 8) / 6 = 204, and 1.5 times that plus the caption tall.
    assert_eq!(g(1280.0).tile, Size::new(204.0, 204.0 * 1.5 + EXTRA));

    // On a real view, across widths and as the width changes.
    let ui = ui();
    let (g0, _) = posters(&ui, 40, true, false);
    ui.root().child(g0.clone().fill_width());
    for width in [1280.0, 390.0, 800.0, 1280.0] {
        frame_at(&ui, width);
        let want = GridGeometry::fluid(width, MIN_WIDTH, ASPECT, EXTRA, gap(&ui), gap(&ui));
        assert_eq!(columns(&g0), want.cols, "columns at {width}");
        let tile = find(&g0, "grid_tile").first().and_then(|t| t.bounds()).unwrap_or_default();
        assert!(
            (tile.width() - want.tile.width).abs() < 0.5,
            "{} at {width}, want {}",
            tile.width(),
            want.tile.width
        );
        assert!((tile.height() - want.tile.height).abs() < 0.5);
    }
}

#[test]
fn in_the_page_scroll_only_the_rows_it_shows_exist() {
    let ui = ui();
    let (page, g, _) = page(&ui, 2_000, false);
    frame_at(&ui, 1280.0);
    let geo = GridGeometry::fluid(1280.0, MIN_WIDTH, ASPECT, EXTRA, gap(&ui), gap(&ui));
    let rows = 2_000usize.div_ceil(geo.cols);
    // The grid is as tall as all its rows: the page scrolls it.
    let h = g.bounds().map(|b| b.height()).unwrap_or_default();
    let want = rows as f32 * geo.row_height() + gap(&ui) * 2.0;
    assert!((h - want).abs() < 1.0, "grid {h} tall, want {want}");
    let built = find(&g, "grid_tile").len();
    assert!(built >= geo.cols * 2 && built <= geo.cols * 24, "built {built} tiles");
    // Scrolling the page far down builds the tiles there and drops the first.
    let row = 200;
    ui.set_scroll_offset(page.id(), Vec2::new(0.0, 300.0 + row as f32 * geo.row_height()));
    frame_at(&ui, 1280.0);
    let shown: Vec<String> = find(&g, "grid_tile").iter().flat_map(texts).collect();
    assert!(shown.contains(&format!("Film {}", row * geo.cols)), "row {row} not built");
    assert!(!shown.contains(&"Film 0".to_owned()), "the first row stayed built");
    assert!(find(&g, "grid_tile").len() <= geo.cols * 24);
}

#[test]
fn the_keyboard_moves_into_a_tile_and_back_out() {
    let ui = ui();
    let (page, g, state) = page(&ui, 200, true);
    frame_at(&ui, 1280.0);
    g.focus();
    frame(&ui);
    let _ = key(&ui, Key::ArrowRight, NONE);
    assert_eq!(state.selection.cursor.get_untracked().ok().flatten(), Some(0));
    let buttons = |i: usize| -> Vec<El> {
        let tiles = find(&g, "grid_tile");
        tiles.get(i).map(|t| find(t, "icon_button")).unwrap_or_default()
    };
    // Enter moves into the tile, the arrows between its buttons.
    let _ = key(&ui, Key::Enter, NONE);
    assert!(buttons(0).first().is_some_and(El::is_focused), "Enter focuses Play");
    let _ = key(&ui, Key::ArrowRight, NONE);
    assert!(buttons(0).get(1).is_some_and(El::is_focused), "Right reaches Details");
    let _ = key(&ui, Key::ArrowLeft, NONE);
    let _ = key(&ui, Key::ArrowLeft, NONE);
    assert!(buttons(0).get(4).is_some_and(El::is_focused), "Left wraps to Info");
    assert_eq!(state.selection.cursor.get_untracked().ok().flatten(), Some(0), "the cursor stays");
    // Escape goes back to the grid, where the arrows move between tiles again.
    let _ = key(&ui, Key::Escape, NONE);
    assert!(g.is_focused());
    let _ = key(&ui, Key::ArrowRight, NONE);
    assert_eq!(state.selection.cursor.get_untracked().ok().flatten(), Some(1));
    // Down to a row far below: the page scrolls to it.
    for _ in 0..12 {
        let _ = key(&ui, Key::ArrowDown, NONE);
    }
    frame_at(&ui, 1280.0);
    let off = ui.scroll_offset(page.id()).y;
    assert!(off > 1000.0, "the page scrolled to the cursor ({off})");
    let c = state.selection.cursor.get_untracked().ok().flatten().unwrap_or_default();
    let shown: Vec<String> = find(&g, "grid_tile").iter().flat_map(texts).collect();
    assert!(shown.contains(&format!("Film {c}")), "the cursor's tile is built");
}

#[test]
fn screen_readers_meet_each_tiles_buttons() {
    let names = |interactive: bool| -> Vec<(Role, String)> {
        let ui = ui();
        let _ = page(&ui, 12, interactive);
        frame_at(&ui, 1280.0);
        ui.reader_view().into_iter().map(|n| (n.role, n.name.unwrap_or_default())).collect()
    };
    let open = names(true);
    for (_, label) in ACTIONS {
        assert!(open.contains(&(Role::Button, format!("{label} film 3"))), "{label} missing: {open:#?}");
    }
    assert!(open.iter().any(|(r, _)| *r == Role::ListItem));
    // Folded tiles read as one item each, with no buttons of their own.
    let folded = names(false);
    assert!(!folded.iter().any(|(r, n)| *r == Role::Button && n.contains("film")));
}

#[test]
fn the_poster_grid_passes_the_accessibility_audit() {
    let found = |interactive: bool| {
        let ui = ui();
        let _ = page(&ui, 60, interactive);
        // The width the audit's own frame uses.
        frame_at(&ui, 800.0);
        audit(&ui)
    };
    let open = found(true);
    assert!(open.is_empty(), "{open:#?}");
    // Folding cards with buttons into one item hides the buttons: the audit says so.
    assert!(found(false).iter().any(|p| p.contains("folded into a merged node")));
}

#[test]
fn previews_at_phone_and_desktop_widths() {
    let p = remedy_ui_devtools::Preview::new("poster_grid", |ui: &Ui| {
        let (g, _) = posters(ui, 2_000, false, true);
        ui.el("page")
            .scroll()
            .column()
            .fill_width()
            .grow(1.0)
            .layout(|l| l.min_height = Length::Px(0.0))
            .child(w::heading(ui, 1, "Films"))
            .child(g)
    })
    .themes(&["portal:portal"])
    .sizes(&[(390.0, 844.0), (1280.0, 800.0)]);
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("poster_grid");
    let _ = std::fs::create_dir_all(&dir);
    for v in p.variations() {
        let snap = p.render(&v);
        // Posters were drawn: the art's blue-violet shows somewhere down the middle.
        let (w, h) = (v.size.0 as u32, v.size.1 as u32);
        let art =
            (0..h).step_by(4).any(|y| snap.pixel(w / 4, y).is_some_and(|[r, _, b, _]| b > 150 && r < 120));
        assert!(art, "no poster art at {:?}", v.size);
        if let Ok(png) = snap.to_png() {
            let path = dir.join(format!("{}.png", v.tag().replace(':', "-")));
            let _ = std::fs::write(&path, png);
            eprintln!("preview: {}", path.display());
        }
    }
}

/// The 2,000-poster library's frame budget: scrolling the page through every poster (each
/// with its art, title and five buttons) costs at most 8 ms of UI-thread work a frame (one
/// frame a display refresh, as the runner draws; rows the scroll brings in are built after
/// that frame's layout and laid out in the next). Best of three runs, as the reference
/// scenes are gated. Measured in release builds; debug builds are not what ships.
#[test]
#[cfg_attr(debug_assertions, ignore = "frame budgets are measured in release builds")]
fn the_two_thousand_poster_grid_meets_its_frame_budget() {
    let ui = ui();
    let (page, g, _) = page(&ui, 2_000, true);
    frame_at(&ui, 1280.0);
    let end = g.bounds().map(|b| b.height()).unwrap_or_default();
    let mut best = (Duration::MAX, Duration::MAX, Duration::MAX);
    for _ in 0..3 {
        let mut times = Vec::new();
        // A steady scroll (40 px a frame), then flicks of two screens a frame.
        let mut y = 0.0f32;
        for i in 0..400 {
            y += if i < 200 { 40.0 } else { 1600.0 };
            if y > end {
                y = 0.0;
            }
            let s = Instant::now();
            ui.set_scroll_offset(page.id(), Vec2::new(0.0, y));
            let _ = ui.frame(Size::new(1280.0, 800.0), 1.0);
            times.push(s.elapsed());
        }
        ui.set_scroll_offset(page.id(), Vec2::ZERO);
        frame_at(&ui, 1280.0);
        let steady = times.get(..200).and_then(|s| s.iter().max()).copied().unwrap_or_default();
        times.sort();
        let p95 = times.get(times.len() * 95 / 100).copied().unwrap_or_default();
        let p50 = times.get(times.len() / 2).copied().unwrap_or_default();
        eprintln!("2,000 posters: steady worst {steady:?}, p50 {p50:?}, p95 {p95:?}");
        best = (best.0.min(steady), best.1.min(p50), best.2.min(p95));
    }
    let (steady, _, p95) = best;
    assert!(steady <= Duration::from_millis(8), "a steady-scroll frame took {steady:?}");
    assert!(p95 <= Duration::from_millis(8), "p95 frame {p95:?} over the 8 ms budget");
}
