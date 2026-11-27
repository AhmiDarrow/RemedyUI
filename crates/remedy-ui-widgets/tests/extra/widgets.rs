//! The QR code, rating and swatch grid on a real window: pixels, keyboard, pointer,
//! semantics, right-to-left and the accessibility audit.

mod common;

use common::*;
use remedy_ui_core::a11y::{Action, Role};
use remedy_ui_core::input::{Key, Modifiers};
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::{Color, Point};
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_widgets::extra::{
    Ecc, QrCode, QrOptions, RatingOptions, Swatch, SwatchOptions, color_swatch_grid, qr_code, rating,
};

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(el.clone());
    let _ = frame(ui);
}

fn theme_rgb(ui: &Ui, f: impl Fn(&remedy_ui_core::style::Theme) -> Color) -> [u8; 3] {
    rgb(f(&ui.theme()))
}

// --- qr_code ---

fn dark_light(ui: &Ui) -> ([u8; 3], [u8; 3]) {
    let t = ui.theme();
    let (text, bg) = (t.derive().text, t.colors.bg);
    if bg.luminance() >= text.luminance() { (rgb(text), rgb(bg)) } else { (rgb(bg), rgb(text)) }
}

#[test]
fn the_qr_code_is_drawn_dark_on_light_from_the_theme() {
    let ui = ui();
    let data = Signal::new("https://remedy.example/pair?code=482913".to_owned());
    let el = qr_code(&ui, "Pairing code", data, QrOptions::default());
    mount(&ui, &el);
    let expected = QrCode::encode_text(&data.get_untracked().unwrap_or_default(), Ecc::Medium)
        .unwrap_or_else(|e| unreachable!("{e}"));
    let b = el.bounds().unwrap_or_default();
    let quiet = 4.0;
    let total = expected.size() as f32 + 2.0 * quiet;
    assert!((b.width() - total * 6.0).abs() < 0.5, "sized from the module count: {}", b.width());
    assert!((b.width() - b.height()).abs() < 0.5, "square");
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let (dark, light) = dark_light(&ui);
    let m = b.width() / total;
    // Every module on screen is what the generator made, and the quiet zone is light.
    let mut wrong = 0;
    for y in 0..expected.size() {
        for x in 0..expected.size() {
            let px = (b.min.x + (x as f32 + quiet + 0.5) * m) as u32;
            let py = (b.min.y + (y as f32 + quiet + 0.5) * m) as u32;
            let want = if expected.module(x, y) { dark } else { light };
            if !near(shot.at(px, py), want, 4) {
                wrong += 1;
            }
        }
    }
    assert_eq!(wrong, 0, "all modules drawn as generated");
    let corner = shot.at((b.min.x + 2.0) as u32, (b.min.y + 2.0) as u32);
    assert!(near(corner, light, 4), "quiet zone is light");
    assert!(
        dark.iter().map(|c| u32::from(*c)).sum::<u32>() < light.iter().map(|c| u32::from(*c)).sum::<u32>()
    );
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn a_qr_code_wider_than_its_place_shrinks_to_fit_and_stays_square() {
    use remedy_ui_core::layout::Length;
    let ui = ui();
    let data = Signal::new("x".repeat(400));
    let el = qr_code(&ui, "Pairing code", data, QrOptions::default());
    // A narrow panel: the code's own size is several times this.
    let panel = ui.el("panel").layout(|l| l.width = Length::Px(180.0)).child(el.clone());
    let _ = ui.root().child(panel.clone());
    let _ = frame(&ui);
    let (code, place) = (el.bounds().unwrap_or_default(), panel.bounds().unwrap_or_default());
    assert!(code.width() <= place.width() + 0.5, "the code is {} wide in a {} panel", code.width(), place.width());
    assert!((code.width() - code.height()).abs() < 0.5, "not square: {} x {}", code.width(), code.height());
    assert!(code.width() > 100.0, "it should fill the panel, not collapse: {}", code.width());
}

#[test]
fn the_qr_code_follows_its_signal_and_grows_with_the_data() {
    let ui = ui();
    let data = Signal::new("short".to_owned());
    let el = qr_code(&ui, "Share link", data, QrOptions::default());
    mount(&ui, &el);
    let small = el.bounds().unwrap_or_default().width();
    data.set("x".repeat(400));
    let _ = frame(&ui);
    let large = el.bounds().unwrap_or_default().width();
    assert!(large > small * 2.0, "{small} -> {large}");
    let described = el.info().and_then(|i| i.semantics).and_then(|s| s.description_text().map(str::to_owned));
    assert!(described.as_deref().is_some_and(|d| d.starts_with("QR code, version ")), "{described:?}");
    assert!(!described.unwrap_or_default().contains("xxxx"), "the data is not read out");
    assert_eq!(role_of(&el), Some(Role::Image));
    assert_eq!(name_of(&el), "Share link");
    // Back to short.
    data.set("short".into());
    let _ = frame(&ui);
    assert!((el.bounds().unwrap_or_default().width() - small).abs() < 0.5);
}

#[test]
fn a_qr_code_with_a_description_says_it_and_too_much_data_is_invalid() {
    let ui = ui();
    let data = Signal::new("abc".to_owned());
    let el = qr_code(&ui, "Pair", data, {
        let mut v_ = QrOptions::default();
        v_.description = Some("Scan with the phone app".into());
        v_.module_size = Some(4.0);
        v_
    });
    mount(&ui, &el);
    let d = el.info().and_then(|i| i.semantics).and_then(|s| s.description_text().map(str::to_owned));
    assert_eq!(d.as_deref(), Some("Scan with the phone app"));
    data.set("y".repeat(5000));
    let _ = frame(&ui);
    let sem = el.info().and_then(|i| i.semantics);
    assert!(sem.as_ref().is_some_and(|s| s.is_invalid()), "too long is reported invalid");
    assert!(
        sem.and_then(|s| s.description_text().map(str::to_owned)).is_some_and(|t| t.contains("too long"))
    );
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    assert!(el.bounds().is_some_and(|b| b.width() > 100.0), "there is room for the explanation");
}

#[test]
fn a_larger_box_gets_whole_pixel_modules() {
    let ui = ui();
    let data = Signal::new("HELLO".to_owned());
    let el = qr_code(&ui, "Hello", data, QrOptions::default());
    mount(&ui, &el);
    let el = el.width(250.0).height(250.0);
    let _ = frame(&ui);
    let b = el.bounds().unwrap_or_default();
    assert!((b.width() - 250.0).abs() < 0.5);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    // The code stays centered with equal margins on every side.
    let (dark, light) = dark_light(&ui);
    let mut first_dark = None;
    for x in (b.min.x as u32)..(b.max.x as u32) {
        if near(shot.at(x, (b.min.y + b.height() / 2.0) as u32), dark, 4) {
            first_dark = Some(x as f32 - b.min.x);
            break;
        }
    }
    assert!(first_dark.is_some_and(|f| f > 10.0), "the quiet zone is kept: {first_dark:?}");
    assert!(near(shot.at(b.min.x as u32 + 1, b.min.y as u32 + 1), light, 4), "the margin is light too");
}

// --- rating ---

fn star_center(el: &El, index: usize, star: f32, gap: f32) -> Point {
    let b = el.bounds().unwrap_or_default();
    Point::new(b.min.x + index as f32 * (star + gap) + star / 2.0, b.min.y + star * 0.53)
}

#[test]
fn a_rating_fills_its_stars_from_the_theme() {
    let ui = ui();
    let value = Signal::new(3.0f32);
    let el = rating(&ui, "Quality", value, RatingOptions::default());
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let star = 24.0;
    let gap = (star * 0.15f32).round();
    let warn = theme_rgb(&ui, |t| t.colors.warn);
    let line = theme_rgb(&ui, |t| t.derive().line);
    for i in 0..3 {
        let p = star_center(&el, i, star, gap);
        assert!(near(shot.at(p.x as u32, p.y as u32), warn, 6), "star {i} is filled");
    }
    for i in 3..5 {
        let p = star_center(&el, i, star, gap);
        assert!(near(shot.at(p.x as u32, p.y as u32), line, 6), "star {i} is empty");
    }
    value.set(1.0);
    shot.take(&ui);
    let p = star_center(&el, 2, star, gap);
    assert!(near(shot.at(p.x as u32, p.y as u32), line, 6), "repaints when the value changes");
}

#[test]
fn a_half_star_fills_only_its_start_side() {
    let ui = ui();
    let value = Signal::new(2.5f32);
    let el = rating(&ui, "Taste", value, {
        let mut v_ = RatingOptions::default();
        v_.half_steps = true;
        v_
    });
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let star = 24.0;
    let gap = (star * 0.15f32).round();
    let c = star_center(&el, 2, star, gap);
    let warn = theme_rgb(&ui, |t| t.colors.warn);
    let line = theme_rgb(&ui, |t| t.derive().line);
    // Left of the middle: filled; right of it: not.
    assert!(near(shot.at((c.x - 4.0) as u32, c.y as u32), warn, 6));
    assert!(near(shot.at((c.x + 4.0) as u32, c.y as u32), line, 6));
}

#[test]
fn a_rating_works_from_the_keyboard_and_says_its_value() {
    let ui = ui();
    let value = Signal::new(2.0f32);
    let el = rating(&ui, "Quality", value, RatingOptions::default());
    mount(&ui, &el);
    el.focus();
    assert_eq!(role_of(&el), Some(Role::Slider));
    assert_eq!(value_text(&el), "2 of 5 stars");
    assert!(actions_of(&el).contains(&Action::Increment) && actions_of(&el).contains(&Action::Decrement));
    assert!(key(&ui, Key::ArrowRight));
    assert_eq!(value.get_untracked().ok(), Some(3.0));
    assert!(key(&ui, Key::ArrowUp));
    assert_eq!(value.get_untracked().ok(), Some(4.0), "up is the same as right (and what Increment sends)");
    assert!(key(&ui, Key::ArrowDown));
    assert!(key(&ui, Key::ArrowLeft));
    assert_eq!(value.get_untracked().ok(), Some(2.0));
    assert!(key(&ui, Key::End));
    assert_eq!(value.get_untracked().ok(), Some(5.0));
    key(&ui, Key::ArrowRight);
    assert_eq!(value.get_untracked().ok(), Some(5.0), "stops at the top");
    assert!(key(&ui, Key::Home));
    assert_eq!(value.get_untracked().ok(), Some(0.0));
    assert!(key(&ui, Key::char('4')));
    assert_eq!(value.get_untracked().ok(), Some(4.0), "a digit sets the value");
    assert!(!key(&ui, Key::char('9')), "a digit past the top is not taken");
    assert_eq!(value_text(&el), "4 of 5 stars");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn half_steps_move_by_halves() {
    let ui = ui();
    let value = Signal::new(0.0f32);
    let el = rating(&ui, "Taste", value, {
        let mut v_ = RatingOptions::default();
        v_.half_steps = true;
        v_.max = 4;
        v_
    });
    mount(&ui, &el);
    el.focus();
    key(&ui, Key::ArrowRight);
    assert_eq!(value.get_untracked().ok(), Some(0.5));
    key(&ui, Key::ArrowRight);
    assert_eq!(value.get_untracked().ok(), Some(1.0));
    assert_eq!(value_text(&el), "1 of 4 stars");
    value.set(2.5);
    let _ = frame(&ui);
    assert_eq!(value_text(&el), "2.5 of 4 stars");
}

#[test]
fn clicking_stars_sets_and_clears_the_rating_and_hover_previews() {
    let ui = ui();
    let value = Signal::new(0.0f32);
    let el = rating(&ui, "Quality", value, RatingOptions::default());
    mount(&ui, &el);
    let (star, gap) = (24.0, 4.0);
    let p4 = star_center(&el, 3, star, gap);
    click(&ui, p4);
    assert_eq!(value.get_untracked().ok(), Some(4.0));
    click(&ui, p4);
    assert_eq!(value.get_untracked().ok(), Some(0.0), "choosing the same value clears it");
    // Hover previews without changing the value.
    pointer(
        &ui,
        star_center(&el, 1, star, gap),
        remedy_ui_core::input::PointerPhase::Move,
        remedy_ui_core::input::Buttons(0),
    );
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let warn = theme_rgb(&ui, |t| t.colors.warn);
    let c = star_center(&el, 1, star, gap);
    assert!(near(shot.at(c.x as u32, c.y as u32), warn, 6), "the hovered star previews");
    assert_eq!(value.get_untracked().ok(), Some(0.0));
}

#[test]
fn a_rating_that_cannot_be_cleared_stays() {
    let ui = ui();
    let value = Signal::new(0.0f32);
    let el = rating(&ui, "Quality", value, {
        let mut v_ = RatingOptions::default();
        v_.clearable = false;
        v_
    });
    mount(&ui, &el);
    let p = star_center(&el, 2, 24.0, 4.0);
    click(&ui, p);
    click(&ui, p);
    assert_eq!(value.get_untracked().ok(), Some(3.0));
}

#[test]
fn a_half_star_can_be_picked_with_the_pointer() {
    let ui = ui();
    let value = Signal::new(0.0f32);
    let el = rating(&ui, "Taste", value, {
        let mut v_ = RatingOptions::default();
        v_.half_steps = true;
        v_
    });
    mount(&ui, &el);
    let c = star_center(&el, 1, 24.0, 4.0);
    click(&ui, Point::new(c.x - 6.0, c.y));
    assert_eq!(value.get_untracked().ok(), Some(1.5));
    click(&ui, Point::new(c.x + 6.0, c.y));
    assert_eq!(value.get_untracked().ok(), Some(2.0));
}

#[test]
fn a_read_only_rating_is_an_image_that_says_its_value() {
    let ui = ui();
    let value = Signal::new(3.5f32);
    let el = rating(&ui, "Average", value, {
        let mut v_ = RatingOptions::default();
        v_.read_only = true;
        v_.half_steps = true;
        v_
    });
    mount(&ui, &el);
    assert_eq!(role_of(&el), Some(Role::Image));
    assert_eq!(value_text(&el), "3.5 of 5 stars");
    let focusable = el.info().is_some_and(|i| i.focusable);
    assert!(!focusable, "a display is not a tab stop");
    click(&ui, center(&el));
    assert_eq!(value.get_untracked().ok(), Some(3.5), "it cannot be changed");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn a_rating_mirrors_in_a_right_to_left_window() {
    let ui = ui();
    let _ = ui.root().layout(|l| l.direction = remedy_ui_core::layout::Direction::Rtl);
    let value = Signal::new(2.0f32);
    let el = rating(&ui, "Quality", value, RatingOptions::default());
    mount(&ui, &el);
    let _ = frame(&ui);
    assert!(el.is_rtl(), "the window lays out right to left");
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    let warn = theme_rgb(&ui, |t| t.colors.warn);
    let line = theme_rgb(&ui, |t| t.derive().line);
    let right_star = Point::new(b.max.x - 12.0, b.min.y + 12.7);
    let left_star = Point::new(b.min.x + 12.0, b.min.y + 12.7);
    assert!(near(shot.at(right_star.x as u32, right_star.y as u32), warn, 6), "fills from the right");
    assert!(near(shot.at(left_star.x as u32, left_star.y as u32), line, 6));
    // Left arrow goes on, as the stars read right to left; clicking the leftmost star is 5.
    el.focus();
    key(&ui, Key::ArrowLeft);
    assert_eq!(value.get_untracked().ok(), Some(3.0));
    key(&ui, Key::ArrowRight);
    assert_eq!(value.get_untracked().ok(), Some(2.0));
    click(&ui, left_star);
    assert_eq!(value.get_untracked().ok(), Some(5.0));
}

// --- color_swatch_grid ---

fn palette() -> Vec<Swatch> {
    vec![
        Swatch::new(Color::from_hex("#c0392b").unwrap_or(Color::BLACK), "Signal red"),
        Swatch::new(Color::from_hex("#e67e22").unwrap_or(Color::BLACK), "Amber"),
        Swatch::new(Color::from_hex("#f1c40f").unwrap_or(Color::BLACK), "Gold"),
        Swatch::new(Color::from_hex("#27ae60").unwrap_or(Color::BLACK), "Moss"),
        Swatch::new(Color::from_hex("#2980b9").unwrap_or(Color::BLACK), "Lake"),
        Swatch::new(Color::from_hex("#8e44ad").unwrap_or(Color::BLACK), "Plum"),
        Swatch::new(Color::from_hex("#ecf0f1").unwrap_or(Color::BLACK), "Snow"),
    ]
}

fn grid(ui: &Ui, columns: Option<usize>) -> (El, Signal<Vec<Swatch>>, Signal<Option<usize>>) {
    let swatches = Signal::new(palette());
    let selected = Signal::new(None::<usize>);
    let el = color_swatch_grid(ui, "Accent color", swatches, selected, {
        let mut v_ = SwatchOptions::default();
        v_.columns = columns;
        v_.size = None;
        v_
    });
    mount(ui, &el);
    (el, swatches, selected)
}

#[test]
fn the_swatches_show_their_colors_and_have_names() {
    let ui = ui();
    let (el, _, _) = grid(&ui, Some(3));
    let cells = find(&el, "swatch_cell");
    assert_eq!(cells.len(), 7);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    for (cell, sw) in cells.iter().zip(palette()) {
        let c = center(cell);
        // A little off the middle, away from the selection mark.
        assert!(near(shot.at(c.x as u32 + 8, c.y as u32 + 8), rgb(sw.color), 4), "{}", sw.name);
        assert_eq!(name_of(cell), sw.name);
        assert_eq!(role_of(cell), Some(Role::GridCell));
    }
    assert_eq!(role_of(&el), Some(Role::Grid));
    assert_eq!(name_of(&el), "Accent color");
    // Three columns: the fourth swatch starts the second row.
    let (a, d) = (
        cells.first().and_then(El::bounds).unwrap_or_default(),
        cells.get(3).and_then(El::bounds).unwrap_or_default(),
    );
    assert!((a.min.x - d.min.x).abs() < 0.5 && d.min.y > a.max.y);
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn arrow_keys_move_over_the_swatches_and_enter_chooses() {
    let ui = ui();
    let (el, _, selected) = grid(&ui, Some(3));
    el.focus();
    assert!(value_text(&el).starts_with("Signal red, #C0392B, row 1 column 1"), "{}", value_text(&el));
    key(&ui, Key::ArrowRight);
    assert!(value_text(&el).starts_with("Amber"));
    key(&ui, Key::ArrowDown);
    assert!(value_text(&el).starts_with("Lake, #2980B9, row 2 column 2"), "{}", value_text(&el));
    key(&ui, Key::ArrowLeft);
    assert!(value_text(&el).starts_with("Moss"), "{}", value_text(&el));
    key(&ui, Key::End);
    assert!(value_text(&el).starts_with("Plum"), "End: last of the row");
    key(&ui, Key::Home);
    assert!(value_text(&el).starts_with("Moss"), "Home: first of the row");
    key_mod(&ui, Key::End, Modifiers { ctrl: true, ..Modifiers::NONE });
    assert!(value_text(&el).starts_with("Snow"), "Ctrl+End: the last swatch");
    key(&ui, Key::ArrowDown);
    assert!(value_text(&el).starts_with("Snow"), "no row below");
    key_mod(&ui, Key::Home, Modifiers { ctrl: true, ..Modifiers::NONE });
    assert!(value_text(&el).starts_with("Signal red"));
    assert_eq!(selected.get_untracked().ok(), Some(None));
    key(&ui, Key::ArrowRight);
    assert!(key(&ui, Key::Enter));
    assert_eq!(selected.get_untracked().ok(), Some(Some(1)));
    assert!(value_text(&el).ends_with("selected"), "{}", value_text(&el));
    key(&ui, Key::ArrowRight);
    assert!(key(&ui, Key::Space));
    assert_eq!(selected.get_untracked().ok(), Some(Some(2)));
    assert!(!key(&ui, Key::char('x')), "other keys pass");
}

#[test]
fn clicking_a_swatch_chooses_it_and_marks_it() {
    let ui = ui();
    let (el, _, selected) = grid(&ui, None);
    let cells = find(&el, "swatch_cell");
    let target = cells.get(4).cloned().unwrap_or_else(|| unreachable!("cell"));
    click(&ui, center(&target));
    assert_eq!(selected.get_untracked().ok(), Some(Some(4)));
    assert!(el.is_focused(), "the grid takes focus, not the swatch");
    let sem = target.info().and_then(|i| i.semantics);
    assert_eq!(sem.and_then(|s| s.selected_state()), Some(true));
    let other =
        cells.first().and_then(|c| c.info()).and_then(|i| i.semantics).and_then(|s| s.selected_state());
    assert_eq!(other, Some(false));
    // The mark is drawn on the chosen swatch only.
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let c = center(&target);
    let plain = rgb(palette().get(4).map(|s| s.color).unwrap_or(Color::BLACK));
    let mut marked = false;
    for dy in -8..8 {
        for dx in -8..8 {
            if !near(shot.at((c.x + dx as f32) as u32, (c.y + dy as f32) as u32), plain, 30) {
                marked = true;
            }
        }
    }
    assert!(marked, "a check mark shows on the chosen swatch");
}

#[test]
fn the_swatches_can_be_replaced_and_wrap_to_the_width() {
    let ui = ui();
    let (el, swatches, selected) = grid(&ui, None);
    let _ = el.clone().width(100.0);
    let _ = frame(&ui);
    let cells = find(&el, "swatch_cell");
    let rows: std::collections::BTreeSet<i32> =
        cells.iter().filter_map(|c| c.bounds()).map(|b| b.min.y as i32).collect();
    assert!(rows.len() >= 2, "a narrow grid wraps onto rows: {rows:?}");
    // Arrow down uses the measured columns.
    el.focus();
    let per_row = cells
        .iter()
        .filter_map(|c| c.bounds())
        .filter(|b| (b.min.y - cells.first().and_then(El::bounds).map_or(0.0, |f| f.min.y)).abs() < 1.0)
        .count();
    key(&ui, Key::ArrowDown);
    let row2_first = palette().get(per_row).map(|s| s.name.clone()).unwrap_or_default();
    assert!(value_text(&el).starts_with(&row2_first), "{} vs {row2_first}", value_text(&el));
    selected.set(Some(1));
    swatches.set(vec![Swatch::new(Color::WHITE, "White")]);
    let _ = frame(&ui);
    assert_eq!(find(&el, "swatch_cell").len(), 1);
    assert!(
        value_text(&el).starts_with("White"),
        "the cursor stays on a swatch that exists: {}",
        value_text(&el)
    );
}

#[test]
fn the_swatch_grid_mirrors_in_a_right_to_left_window() {
    let ui = ui();
    let _ = ui.root().layout(|l| l.direction = remedy_ui_core::layout::Direction::Rtl);
    let (el, _, _) = grid(&ui, Some(3));
    let _ = frame(&ui);
    let cells = find(&el, "swatch_cell");
    let (a, b) = (
        cells.first().and_then(El::bounds).unwrap_or_default(),
        cells.get(1).and_then(El::bounds).unwrap_or_default(),
    );
    assert!(a.min.x > b.min.x, "the first swatch is at the right");
    el.focus();
    key(&ui, Key::ArrowLeft);
    assert!(
        value_text(&el).starts_with("Amber"),
        "Left goes on in a right to left grid: {}",
        value_text(&el)
    );
    key(&ui, Key::ArrowRight);
    assert!(value_text(&el).starts_with("Signal red"));
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_widgets::extra::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn a_rating_reads_in_the_window_language() {
    let ui = ui();
    let el = rating(&ui, "Quality", Signal::new(2.5f32), RatingOptions::default());
    mount(&ui, &el);
    ui.set_locale("de");
    let _ = frame(&ui);
    assert_eq!(value_text(&el), "2.5 von 5 Sternen");
    ui.set_locale("ru");
    let _ = frame(&ui);
    assert_eq!(value_text(&el), "2.5 из 5 звёзд");
    let hearts = rating(&ui, "Love", Signal::new(1.0f32), RatingOptions::default().unit("hearts".into()));
    mount(&ui, &hearts);
    ui.set_locale("en");
    let _ = frame(&ui);
    assert_eq!(value_text(&hearts), "1 of 5 hearts");
}
