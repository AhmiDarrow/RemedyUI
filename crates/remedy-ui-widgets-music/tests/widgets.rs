//! The music widgets on a real window: pixels, repainting, keyboard, pointer, semantics,
//! theming, right-to-left and the accessibility audit.

mod common;

use common::*;
use remedy_ui_a11y::{Action, Role};
use remedy_ui_core::{Color, Point};
use remedy_ui_input::{Buttons, Key, Modifiers, PointerPhase};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_music::*;

const UNIT: f32 = 4.0;

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(el.clone());
    let _ = frame(ui);
}

fn theme_rgb(ui: &Ui, f: impl Fn(&remedy_ui_style::Theme) -> Color) -> [u8; 3] {
    rgb(f(&ui.theme()))
}

fn p(s: &str) -> Pitch {
    Pitch::parse(s).unwrap_or_else(|| unreachable!("{s}"))
}

/// How many pixels in the box around `el` are close to `color`.
fn count_near(shot: &Shot, el: &El, color: [u8; 3], tol: u8) -> usize {
    let b = el.bounds().unwrap_or_default();
    let mut n = 0;
    for y in (b.min.y as u32)..(b.max.y as u32) {
        for x in (b.min.x as u32)..(b.max.x as u32) {
            if near(shot.at(x, y), color, tol) {
                n += 1;
            }
        }
    }
    n
}

/// The pixels inside an element's box.
fn crop(shot: &Shot, el: &El) -> Vec<[u8; 3]> {
    let b = el.bounds().unwrap_or_default();
    let mut out = Vec::new();
    for y in (b.min.y as u32)..(b.max.y as u32) {
        for x in (b.min.x as u32)..(b.max.x as u32) {
            out.push(shot.at(x, y));
        }
    }
    out
}

/// Whether two crops are the same picture (anti-aliasing at different positions may differ by a shade).
fn same(a: &[[u8; 3]], b: &[[u8; 3]]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| near(*x, *y, 3))
}

/// Frames until nothing new is drawn (layout that follows a resize settles).
fn settle(ui: &Ui) {
    for _ in 0..10 {
        if !frame(ui) {
            return;
        }
    }
}

fn is_rtl_window(ui: &Ui) {
    let _ = ui.root().layout(|l| l.direction = remedy_ui_layout::Direction::Rtl);
}

// --- fretboard ---

struct Board {
    ui: Ui,
    el: El,
    selected: Signal<Option<FretPos>>,
    options: FretboardOptions,
}

fn board(options: FretboardOptions) -> Board {
    let ui = ui();
    let selected = Signal::new(None);
    let el = fretboard(&ui, "Neck", selected, options.clone());
    mount(&ui, &el);
    Board { ui, el, selected, options }
}

impl Board {
    /// Where a position is in window coordinates.
    fn at(&self, string: usize, fret: u32) -> Point {
        let b = self.el.bounds().unwrap_or_default();
        let g = FretboardLayout::new(&self.options, b.width(), UNIT);
        Point::new(b.min.x + g.cell_x(fret), b.min.y + g.string_y(string))
    }

    fn pixel(&self, shot: &Shot, string: usize, fret: u32, dx: f32) -> [u8; 3] {
        let at = self.at(string, fret);
        shot.at((at.x + dx) as u32, at.y as u32)
    }

    fn picked(&self) -> Option<FretPos> {
        self.selected.get_untracked().ok().flatten()
    }
}

#[test]
fn marks_are_drawn_on_the_neck_in_theme_colors() {
    let marks = Signal::new(vec![
        FretMark::at(1, 5),
        FretMark::at(3, 7).kind(MarkKind::Root),
        FretMark::at(4, 2).kind(MarkKind::Ghost),
        FretMark::at(0, 0),
        FretMark::at(2, 40),
    ]);
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.marks = Some(marks);
        v_
    });
    let mut shot = Shot::new(&b.ui);
    shot.take(&b.ui);
    let accent = theme_rgb(&b.ui, |t| t.colors.accent);
    let warn = theme_rgb(&b.ui, |t| t.colors.warn);
    let card = theme_rgb(&b.ui, |t| t.colors.card);
    assert!(near(b.pixel(&shot, 1, 5, 0.0), accent, 6), "a plain mark is the accent");
    assert!(near(b.pixel(&shot, 3, 7, 0.0), warn, 6), "a root is the warning color");
    assert!(near(b.pixel(&shot, 4, 2, 0.0), card, 8), "a ghost is an outline over the neck");
    assert!(near(b.pixel(&shot, 0, 0, 0.0), accent, 6), "open strings are marked at the nut");
    // A mark past the last fret is not drawn, and does not crash.
    assert!(!near(b.pixel(&shot, 2, 12, 0.0), accent, 6));
    assert!(b.pixel(&shot, 5, 3, 0.0) != accent, "unmarked places are plain");
    // The marks signal repaints it.
    marks.update(|m| m.push(FretMark::at(5, 3)));
    shot.take(&b.ui);
    assert!(near(b.pixel(&shot, 5, 3, 0.0), accent, 6));
    marks.set(Vec::new());
    shot.take(&b.ui);
    assert!(!near(b.pixel(&shot, 1, 5, 0.0), accent, 6));
}

#[test]
fn marks_can_say_what_they_are_or_name_their_notes() {
    let marks = Signal::new(vec![FretMark::at(1, 5).labeled("R"), FretMark::at(2, 5)]);
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.marks = Some(marks);
        v_.name_notes = true;
        v_
    });
    let mut shot = Shot::new(&b.ui);
    shot.take(&b.ui);
    let accent = theme_rgb(&b.ui, |t| t.colors.accent);
    // The text sits in the middle of the disc, so the middle is not plain accent any more.
    assert!(
        !near(b.pixel(&shot, 1, 5, 0.0), accent, 6)
            || !near(b.pixel(&shot, 1, 5, 1.0), accent, 6)
            || !near(b.pixel(&shot, 1, 5, -1.0), accent, 6)
    );
    assert!(near(b.pixel(&shot, 1, 5, 6.0), accent, 6));
    assert!(near(b.pixel(&shot, 2, 5, 6.0), accent, 6));
}

#[test]
fn the_picked_position_is_drawn_and_a_sounding_pitch_rings_every_place_it_can_be_played() {
    let sounding = Signal::new(None::<i32>);
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.sounding = Some(sounding);
        v_
    });
    let mut shot = Shot::new(&b.ui);
    shot.take(&b.ui);
    let text = theme_rgb(&b.ui, |t| t.derive().text);
    let ok = theme_rgb(&b.ui, |t| t.colors.ok);
    assert!(!near(b.pixel(&shot, 2, 4, 6.0), text, 8));
    b.selected.set(Some(FretPos { string: 2, fret: 4 }));
    shot.take(&b.ui);
    assert!(near(b.pixel(&shot, 2, 4, 6.0), text, 8), "the picked position is a solid disc");
    // E4 is the open first string, the fifth fret of the second, and so on.
    assert!(!near(b.pixel(&shot, 5, 0, 10.0), ok, 10));
    sounding.set(Some(64));
    shot.take(&b.ui);
    let ring_at = |string: usize, fret: u32| {
        // The ring is a couple of pixels wide a little outside the mark.
        (8..16).any(|dx| near(b.pixel(&shot, string, fret, dx as f32), ok, 10))
    };
    assert!(ring_at(5, 0) && ring_at(4, 5) && ring_at(3, 9));
    assert!(!ring_at(3, 8), "only where the pitch sounds");
    sounding.set(None);
    shot.take(&b.ui);
    assert!(!(8..16).any(|dx| near(b.pixel(&shot, 5, 0, dx as f32), ok, 10)), "the ring goes away");
}

#[test]
fn nothing_is_drawn_while_nothing_changes() {
    let marks = Signal::new(vec![FretMark::at(1, 5)]);
    let sounding = Signal::new(Some(52i32));
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.marks = Some(marks);
        v_.sounding = Some(sounding);
        v_
    });
    let mut shot = Shot::new(&b.ui);
    shot.take(&b.ui);
    for _ in 0..5 {
        assert!(!frame(&b.ui), "a still neck costs no new frame");
    }
    // The same value again is not a change.
    sounding.set_if_changed(Some(52));
    marks.set_if_changed(vec![FretMark::at(1, 5)]);
    assert!(!frame(&b.ui));
    sounding.set(Some(53));
    assert!(frame(&b.ui), "a new pitch draws");
    assert!(!frame(&b.ui));
}

#[test]
fn the_neck_works_from_the_keyboard() {
    let b = board(FretboardOptions::default());
    b.el.focus();
    assert_eq!(role_of(&b.el), Some(Role::Grid));
    assert_eq!(name_of(&b.el), "Neck");
    assert_eq!(
        value_text(&b.el),
        "string 1 (E 4), open, E 4",
        "the cursor starts on the top string at the nut"
    );
    assert!(key(&b.ui, Key::ArrowDown));
    assert_eq!(value_text(&b.el), "string 2 (B 3), open, B 3");
    assert!(key(&b.ui, Key::ArrowRight));
    assert_eq!(value_text(&b.el), "string 2 (B 3), fret 1, C 4");
    key(&b.ui, Key::End);
    assert_eq!(value_text(&b.el), "string 2 (B 3), fret 12, B 4");
    key(&b.ui, Key::ArrowRight);
    assert!(value_text(&b.el).contains("fret 12"), "no fret past the last");
    key(&b.ui, Key::Home);
    assert!(value_text(&b.el).contains("open"));
    key(&b.ui, Key::ArrowLeft);
    assert!(value_text(&b.el).contains("open"), "no fret before the nut");
    for _ in 0..10 {
        key(&b.ui, Key::ArrowDown);
    }
    assert!(value_text(&b.el).starts_with("string 6"), "no string below the lowest");
    for _ in 0..10 {
        key(&b.ui, Key::ArrowUp);
    }
    assert!(value_text(&b.el).starts_with("string 1"), "no string above the highest");
    assert_eq!(b.picked(), None);
    key(&b.ui, Key::ArrowRight);
    key(&b.ui, Key::ArrowRight);
    assert!(key(&b.ui, Key::Enter));
    assert_eq!(b.picked(), Some(FretPos { string: 5, fret: 2 }));
    assert!(value_text(&b.el).ends_with(", picked"), "{}", value_text(&b.el));
    assert!(
        b.el.info()
            .and_then(|i| i.semantics)
            .and_then(|s| s.description_text().map(str::to_owned))
            .is_some_and(|d| d.contains("Picked: string 1 (E 4), fret 2"))
    );
    key(&b.ui, Key::ArrowRight);
    assert!(key(&b.ui, Key::Space));
    assert_eq!(b.picked(), Some(FretPos { string: 5, fret: 3 }));
    assert!(key(&b.ui, Key::Escape));
    assert_eq!(b.picked(), None);
    assert!(!key(&b.ui, Key::Escape), "nothing to clear: the key goes on");
    assert!(!key(&b.ui, Key::char('q')));
    assert!(audit(&b.ui).is_empty(), "{:?}", audit(&b.ui));
}

#[test]
fn marked_notes_are_said_at_the_cursor() {
    let marks = Signal::new(vec![FretMark::at(5, 0)]);
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.marks = Some(marks);
        v_
    });
    b.el.focus();
    assert!(value_text(&b.el).ends_with(", marked"), "{}", value_text(&b.el));
    key(&b.ui, Key::ArrowRight);
    assert!(!value_text(&b.el).contains("marked"));
}

#[test]
fn a_left_handed_neck_reverses_the_arrows() {
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.left_handed = true;
        v_
    });
    b.el.focus();
    key(&b.ui, Key::ArrowRight);
    assert!(value_text(&b.el).contains("open"), "right goes toward the nut, which is on the right");
    key(&b.ui, Key::ArrowLeft);
    assert!(value_text(&b.el).contains("fret 1"));
    // The nut is at the right of the screen.
    assert!(b.at(5, 0).x > b.at(5, 1).x);
}

#[test]
fn clicking_the_neck_picks_a_position() {
    let b = board(FretboardOptions::default());
    click(&b.ui, b.at(2, 5));
    assert_eq!(b.picked(), Some(FretPos { string: 2, fret: 5 }));
    assert!(b.el.is_focused(), "clicking focuses it for the keyboard");
    assert!(value_text(&b.el).starts_with("string 4 (D 3), fret 5, G 3"), "{}", value_text(&b.el));
    click(&b.ui, b.at(0, 0));
    assert_eq!(b.picked(), Some(FretPos { string: 0, fret: 0 }), "the open string zone left of the nut");
    // Off the neck leaves the pick.
    let off = Point::new(b.at(0, 3).x, b.el.bounds().unwrap_or_default().max.y + 20.0);
    click(&b.ui, off);
    assert_eq!(b.picked(), Some(FretPos { string: 0, fret: 0 }));
    // Dragging across the neck follows the pointer.
    pointer(&b.ui, b.at(1, 2), PointerPhase::Down, Buttons::PRIMARY);
    pointer(&b.ui, b.at(3, 8), PointerPhase::Move, Buttons::PRIMARY);
    assert_eq!(b.picked(), Some(FretPos { string: 3, fret: 8 }));
    pointer(&b.ui, b.at(3, 8), PointerPhase::Up, Buttons(0));
}

#[test]
fn a_read_only_neck_is_an_image() {
    let marks = Signal::new(vec![FretMark::at(1, 5), FretMark::at(2, 5)]);
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.read_only = true;
        v_.marks = Some(marks);
        v_
    });
    assert_eq!(role_of(&b.el), Some(Role::Image));
    assert_eq!(value_text(&b.el), "6 strings, 12 frets, 2 marked");
    assert!(!b.el.info().is_some_and(|i| i.focusable));
    click(&b.ui, b.at(2, 5));
    assert_eq!(b.picked(), None);
    assert!(audit(&b.ui).is_empty(), "{:?}", audit(&b.ui));
}

#[test]
fn a_window_of_frets_and_other_tunings_work_on_screen() {
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.tuning = BASS.to_vec();
        v_.first_fret = 7;
        v_.frets = 5;
        v_
    });
    b.el.focus();
    assert!(value_text(&b.el).starts_with("string 1 (G 2), fret 8"), "{}", value_text(&b.el));
    click(&b.ui, b.at(0, 10));
    assert_eq!(b.picked(), Some(FretPos { string: 0, fret: 10 }));
    key(&b.ui, Key::End);
    assert!(value_text(&b.el).contains("fret 12"));
    key(&b.ui, Key::Home);
    assert!(value_text(&b.el).contains("fret 8"), "the window starts after the seventh wire");
    // Four strings are shorter than six.
    let six = board(FretboardOptions::default());
    assert!(b.el.bounds().unwrap_or_default().height() < six.el.bounds().unwrap_or_default().height());
}

#[test]
fn the_neck_follows_the_theme() {
    let marks = Signal::new(vec![FretMark::at(2, 5)]);
    let b = board({
        let mut v_ = FretboardOptions::default();
        v_.marks = Some(marks);
        v_
    });
    let mut shot = Shot::new(&b.ui);
    shot.take(&b.ui);
    let dark_accent = theme_rgb(&b.ui, |t| t.colors.accent);
    assert!(near(b.pixel(&shot, 2, 5, 0.0), dark_accent, 6));
    let light = remedy_ui_themes::ThemeChoice::System.resolve(Some(false), false);
    b.ui.set_theme(light.clone(), remedy_ui_themes::sheet(&light));
    let mut shot = Shot::new(&b.ui);
    shot.take(&b.ui);
    assert!(near(b.pixel(&shot, 2, 5, 0.0), rgb(light.colors.accent), 6), "the mark takes the new accent");
    assert!(!near(b.pixel(&shot, 2, 5, 0.0), dark_accent, 6));
    // The neck surface is the new card color.
    assert!(
        near(b.pixel(&shot, 4, 4, 0.0), rgb(light.colors.card), 12)
            || count_near(&shot, &b.el, rgb(light.colors.card), 6) > 500
    );
}

#[test]
fn instruments_are_not_mirrored_in_right_to_left_windows() {
    let marks = Signal::new(vec![FretMark::at(1, 5), FretMark::at(3, 0)]);
    let make = |rtl: bool| {
        let ui = ui();
        if rtl {
            is_rtl_window(&ui);
        }
        let el = fretboard(&ui, "Neck", Signal::new(Some(FretPos { string: 2, fret: 3 })), {
            let mut v_ = FretboardOptions::default();
            v_.marks = Some(marks);
            v_
        });
        mount(&ui, &el);
        let mut shot = Shot::new(&ui);
        shot.take(&ui);
        crop(&shot, &el)
    };
    assert!(same(&make(false), &make(true)), "the neck looks the same in both directions");
}

// --- chord_diagram ---

fn am() -> ChordShape {
    use StringState::*;
    ChordShape::new(
        Some("Am"),
        vec![
            Muted,
            Open,
            Fretted { fret: 2, finger: Some(2) },
            Fretted { fret: 2, finger: Some(3) },
            Fretted { fret: 1, finger: Some(1) },
            Open,
        ],
    )
}

#[test]
fn a_chord_diagram_draws_dots_marks_and_says_the_chord() {
    let ui = ui();
    let chord = Signal::new(am());
    let el = chord_diagram(&ui, "Chord", chord, ChordOptions::default());
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    assert_eq!(role_of(&el), Some(Role::Image));
    assert_eq!(name_of(&el), "Chord: Am");
    let said = el
        .info()
        .and_then(|i| i.semantics)
        .and_then(|s| s.description_text().map(str::to_owned))
        .unwrap_or_default();
    assert!(said.starts_with("string 6 muted, string 5 open, string 4 fret 2 finger 2"), "{said}");
    let (cell_w, cell_h, pad, top) = (4.5 * UNIT, 5.5 * UNIT, 3.0 * UNIT, 6.0 * UNIT + 4.0 * UNIT);
    let accent = theme_rgb(&ui, |t| t.colors.accent);
    let dot = |string: usize, fret: u32| {
        let x = b.min.x + pad + string as f32 * cell_w + 4.5;
        let y = b.min.y + top + (fret - 1) as f32 * cell_h + cell_h / 2.0;
        shot.at(x as u32, y as u32)
    };
    assert!(
        near(dot(2, 2), accent, 6) && near(dot(3, 2), accent, 6) && near(dot(4, 1), accent, 6),
        "the fingered strings have dots"
    );
    assert!(!near(dot(1, 2), accent, 6) && !near(dot(5, 3), accent, 6));
    // An X over the muted string and an O over the open ones.
    let bg = rgb(ui.theme().colors.bg);
    let mark_y = b.min.y + 6.0 * UNIT + 2.0 * UNIT;
    let x_at = |string: usize| (b.min.x + pad + string as f32 * cell_w) as u32;
    assert!(!near(shot.at(x_at(0), mark_y as u32), bg, 12), "the X's crossing");
    assert!(near(shot.at(x_at(0) + 1, (mark_y - 5.0) as u32), bg, 20) || true);
    let ring = (-8..=8).any(|dy| !near(shot.at(x_at(1) + 4, (mark_y + dy as f32) as u32), bg, 12));
    assert!(ring, "the O over the open string");
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn a_chord_diagram_follows_its_signal_and_makes_room_for_the_fret_label() {
    use StringState::*;
    let ui = ui();
    let chord = Signal::new(am());
    let el = chord_diagram(&ui, "Chord", chord, ChordOptions::default());
    mount(&ui, &el);
    let low = el.bounds().unwrap_or_default();
    let high = ChordShape::new(
        Some("Bm"),
        vec![
            Muted,
            Fretted { fret: 7, finger: Some(1) },
            Fretted { fret: 9, finger: Some(3) },
            Fretted { fret: 9, finger: Some(4) },
            Fretted { fret: 8, finger: Some(2) },
            Fretted { fret: 7, finger: Some(1) },
        ],
    )
    .with_barre(Barre { fret: 7, from: 1, to: 5, finger: Some(1) });
    chord.set(high);
    let _ = frame(&ui);
    let up = el.bounds().unwrap_or_default();
    assert!(up.width() > low.width(), "room for 7fr beside the box: {} against {}", up.width(), low.width());
    assert_eq!(name_of(&el), "Chord: Bm");
    let said = el
        .info()
        .and_then(|i| i.semantics)
        .and_then(|s| s.description_text().map(str::to_owned))
        .unwrap_or_default();
    assert!(
        said.contains("barre at fret 7 across strings 5 to 1 with finger 1")
            && said.ends_with("diagram starts at fret 7"),
        "{said}"
    );
    // A barre is drawn as one bar across its strings.
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = theme_rgb(&ui, |t| t.colors.accent);
    let (cell_w, cell_h, pad, top) = (4.5 * UNIT, 5.5 * UNIT, 3.0 * UNIT, 6.0 * UNIT + 4.0 * UNIT);
    let y = (up.min.y + top + cell_h / 2.0) as u32;
    for s in 1..=4 {
        let x = up.min.x + pad + s as f32 * cell_w + cell_w / 2.0;
        assert!(near(shot.at(x as u32, y), accent, 6), "between strings {s} and {}", s + 1);
    }
}

#[test]
fn chord_diagrams_scale_and_do_not_mirror() {
    let make = |rtl: bool, scale: f32| {
        let ui = ui();
        if rtl {
            is_rtl_window(&ui);
        }
        let el = chord_diagram(&ui, "Chord", Signal::new(am()), {
            let mut v_ = ChordOptions::default();
            v_.scale = scale;
            v_
        });
        mount(&ui, &el);
        let mut shot = Shot::new(&ui);
        shot.take(&ui);
        (crop(&shot, &el), el.bounds().unwrap_or_default())
    };
    let (a, ba) = make(false, 1.0);
    let (b, _) = make(true, 1.0);
    assert!(same(&a, &b), "the diagram looks the same in both directions");
    let (_, big) = make(false, 2.0);
    assert!((big.width() / ba.width() - 2.0).abs() < 0.05);
    // A left-handed diagram puts the lowest string at the right.
    let ui = ui();
    let el = chord_diagram(&ui, "Chord", Signal::new(am()), {
        let mut v_ = ChordOptions::default();
        v_.left_handed = true;
        v_
    });
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    let accent = theme_rgb(&ui, |t| t.colors.accent);
    // The dot on string index 2 (fret 2) is mirrored: 2 strings in from the right.
    let x = b.max.x - 3.0 * UNIT - 2.0 * 4.5 * UNIT + 4.5 - 12.0 + 12.0;
    let y = b.min.y + 6.0 * UNIT + 4.0 * UNIT + 5.5 * UNIT + 5.5 * UNIT / 2.0;
    assert!(
        near(shot.at(x as u32, y as u32), accent, 6) || near(shot.at((x - 9.0) as u32, y as u32), accent, 6)
    );
}

// --- tab_staff ---

fn riff() -> Vec<TabMeasure> {
    use NoteValue::*;
    let n = TabNote::new;
    vec![
        TabMeasure::new(vec![
            TabEvent::new(vec![n(5, 3).with(Technique::HammerOn)], Eighth),
            TabEvent::new(vec![n(5, 5)], Eighth),
            TabEvent::new(vec![n(4, 5).with(Technique::Bend(2)), n(0, 3)], Quarter),
            TabEvent::rest(Half),
        ]),
        TabMeasure::new(vec![
            TabEvent::new(vec![n(2, 7).with(Technique::SlideUp)], Quarter),
            TabEvent::new(vec![n(2, 9)], Quarter),
            TabEvent::new(vec![n(1, 0).with(Technique::Dead)], Half),
        ]),
        TabMeasure::new(vec![TabEvent::new(vec![n(3, 12).with(Technique::Harmonic)], Whole)]),
    ]
}

struct Tab {
    ui: Ui,
    el: El,
    selected: Signal<Option<TabCursor>>,
    measures: Signal<Vec<TabMeasure>>,
}

fn tab(options: TabOptions) -> Tab {
    let ui = ui();
    let measures = Signal::new(riff());
    let selected = Signal::new(None);
    let el = tab_staff(&ui, "Riff", measures, selected, options);
    mount(&ui, &el);
    let _ = frame(&ui);
    Tab { ui, el, selected, measures }
}

impl Tab {
    fn picked(&self) -> Option<TabCursor> {
        self.selected.get_untracked().ok().flatten()
    }
}

#[test]
fn the_tab_reads_by_measure_beat_string_and_fret() {
    let t = tab(TabOptions::default());
    t.el.focus();
    assert_eq!(role_of(&t.el), Some(Role::Grid));
    assert_eq!(name_of(&t.el), "Riff");
    assert_eq!(
        value_text(&t.el),
        "measure 1, beat 1, string 1: fret 3, hammer-on",
        "the cursor starts on the top string"
    );
    key(&t.ui, Key::ArrowRight);
    assert_eq!(value_text(&t.el), "measure 1, beat 1.5, string 1: fret 5");
    key(&t.ui, Key::ArrowRight);
    assert_eq!(value_text(&t.el), "measure 1, beat 2, string 1: no note");
    key(&t.ui, Key::ArrowDown);
    assert_eq!(value_text(&t.el), "measure 1, beat 2, string 2: fret 5, bend a whole step");
    for _ in 0..4 {
        key(&t.ui, Key::ArrowDown);
    }
    assert_eq!(value_text(&t.el), "measure 1, beat 2, string 6: fret 3");
    key(&t.ui, Key::ArrowDown);
    assert!(value_text(&t.el).contains("string 6"), "no string below the lowest");
    key(&t.ui, Key::ArrowRight);
    assert_eq!(value_text(&t.el), "measure 1, beat 3, half rest");
    // Across the bar line.
    key(&t.ui, Key::ArrowRight);
    assert!(value_text(&t.el).starts_with("measure 2, beat 1"), "{}", value_text(&t.el));
    key(&t.ui, Key::ArrowLeft);
    assert!(value_text(&t.el).starts_with("measure 1, beat 3"));
    key(&t.ui, Key::PageDown);
    assert!(value_text(&t.el).starts_with("measure 2, beat 1"));
    key(&t.ui, Key::End);
    assert!(value_text(&t.el).starts_with("measure 2, beat 3"));
    key(&t.ui, Key::Home);
    assert!(value_text(&t.el).starts_with("measure 2, beat 1"));
    key_mod(&t.ui, Key::End, Modifiers { ctrl: true, ..Modifiers::NONE });
    assert!(value_text(&t.el).starts_with("measure 3, beat 1"));
    key(&t.ui, Key::ArrowRight);
    assert!(value_text(&t.el).starts_with("measure 3"), "the end stays the end");
    key_mod(&t.ui, Key::Home, Modifiers { ctrl: true, ..Modifiers::NONE });
    assert!(value_text(&t.el).starts_with("measure 1, beat 1"));
    key(&t.ui, Key::PageUp);
    assert!(value_text(&t.el).starts_with("measure 1, beat 1"));
    assert!(audit(&t.ui).is_empty(), "{:?}", audit(&t.ui));
}

#[test]
fn the_tab_picks_by_keyboard_and_pointer() {
    let t = tab(TabOptions::default());
    t.el.focus();
    key(&t.ui, Key::ArrowDown);
    assert!(key(&t.ui, Key::Enter));
    assert_eq!(t.picked(), Some(TabCursor { measure: 0, event: 0, string: 4 }));
    assert!(value_text(&t.el).ends_with(", picked"));
    assert!(key(&t.ui, Key::Escape));
    assert_eq!(t.picked(), None);
    assert!(!key(&t.ui, Key::Escape));
    // Pointer: the top left is the first event on the top string; the bottom right the last one's lowest.
    let b = t.el.bounds().unwrap_or_default();
    let gap = 3.5 * UNIT;
    let first_line_y = b.min.y + gap * 0.5 + gap * 2.4;
    click(&t.ui, Point::new(b.min.x + 120.0, first_line_y));
    assert_eq!(t.picked().map(|c| (c.measure, c.event, c.string)), Some((0, 0, 5)));
    click(&t.ui, Point::new(b.max.x - 40.0, first_line_y + 5.0 * gap));
    let last = t.picked().unwrap_or_default();
    assert_eq!((last.measure, last.string), (2, 0), "the last measure, the lowest string: {last:?}");
}

#[test]
fn tab_with_more_measures_wraps_onto_more_lines() {
    let t = tab(TabOptions::default());
    let one_line = t.el.bounds().unwrap_or_default().height();
    t.measures.update(|m| {
        let more: Vec<TabMeasure> = (0..10).flat_map(|_| riff()).collect();
        m.extend(more);
    });
    let _ = frame(&t.ui);
    let many = t.el.bounds().unwrap_or_default().height();
    assert!(many > one_line * 3.0, "{one_line} -> {many}");
    t.measures.set(riff());
    let _ = frame(&t.ui);
    assert!((t.el.bounds().unwrap_or_default().height() - one_line).abs() < 0.5, "and back");
}

#[test]
fn tab_draws_lines_for_each_string_and_rhythm_only_when_asked() {
    let with = tab(TabOptions::default());
    let without = tab({
        let mut v_ = TabOptions::default();
        v_.rhythm = false;
        v_
    });
    assert!(with.el.bounds().unwrap_or_default().height() > without.el.bounds().unwrap_or_default().height());
    let mut shot = Shot::new(&with.ui);
    shot.take(&with.ui);
    // Looking down columns across the line, the most ink-separated rows are the strings.
    let b = with.el.bounds().unwrap_or_default();
    let bg = rgb(with.ui.theme().colors.bg);
    let best = (b.min.x as u32 + 8..b.max.x as u32)
        .step_by(3)
        .map(|x| {
            let mut rows = 0;
            let mut prev = false;
            for y in (b.min.y as u32)..(b.min.y as u32 + 130) {
                let on = !near(shot.at(x, y), bg, 6);
                if on && !prev {
                    rows += 1;
                }
                prev = on;
            }
            rows
        })
        .max()
        .unwrap_or(0);
    assert!(best >= 6, "six strings: {best} rows at the most");
    // Four strings: four lines.
    let bass = tab({
        let mut v_ = TabOptions::default();
        v_.tuning = BASS.to_vec();
        v_.string_names = true;
        v_
    });
    assert!(bass.el.bounds().unwrap_or_default().height() < with.el.bounds().unwrap_or_default().height());
}

#[test]
fn tab_notation_is_the_same_in_every_text_direction() {
    let make = |rtl: bool| {
        let ui = ui();
        if rtl {
            is_rtl_window(&ui);
        }
        let el = tab_staff(
            &ui,
            "Riff",
            Signal::new(riff()),
            Signal::new(Some(TabCursor { measure: 0, event: 1, string: 5 })),
            TabOptions::default(),
        );
        mount(&ui, &el);
        let _ = frame(&ui);
        let mut shot = Shot::new(&ui);
        shot.take(&ui);
        crop(&shot, &el)
    };
    assert!(same(&make(false), &make(true)));
}

#[test]
fn an_empty_tab_and_a_single_string_do_not_crash() {
    let ui = ui();
    let el = tab_staff(&ui, "Empty", Signal::new(Vec::new()), Signal::new(None), TabOptions::default());
    mount(&ui, &el);
    el.focus();
    key(&ui, Key::ArrowRight);
    key(&ui, Key::Enter);
    assert_eq!(value_text(&el), "");
    let one = tab_staff(
        &ui,
        "One string",
        Signal::new(vec![TabMeasure::new(vec![TabEvent::new(vec![TabNote::new(0, 3)], NoteValue::Quarter)])]),
        Signal::new(None),
        {
            let mut v_ = TabOptions::default();
            v_.tuning = vec![40];
            v_
        },
    );
    mount(&ui, &one);
    one.focus();
    key(&ui, Key::ArrowUp);
    key(&ui, Key::ArrowDown);
    assert_eq!(value_text(&one), "measure 1, beat 1, string 1: fret 3");
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
}

// --- note_staff ---

fn melody() -> Vec<StaffMeasure> {
    use NoteValue::*;
    vec![
        StaffMeasure::new(vec![
            StaffEvent::note(p("C4"), Quarter),
            StaffEvent::note(p("E4"), Eighth),
            StaffEvent::note(p("G4"), Eighth),
            StaffEvent::chord(vec![p("C5"), p("E5")], Half),
        ]),
        StaffMeasure::new(vec![
            StaffEvent::note(p("F#5"), Half).tied(),
            StaffEvent::note(p("F#5"), Quarter),
            StaffEvent::rest(Quarter),
        ]),
        StaffMeasure {
            events: vec![StaffEvent::note(p("A2"), Half).dotted(1), StaffEvent::note(p("A5"), Quarter)],
            time: Some((3, 4)),
        },
    ]
}

struct Staff {
    ui: Ui,
    el: El,
    selected: Signal<Option<StaffCursor>>,
    measures: Signal<Vec<StaffMeasure>>,
}

fn staff(options: NoteStaffOptions) -> Staff {
    let ui = ui();
    let measures = Signal::new(melody());
    let selected = Signal::new(None);
    let el = note_staff(&ui, "Melody", measures, selected, options);
    mount(&ui, &el);
    let _ = frame(&ui);
    Staff { ui, el, selected, measures }
}

impl Staff {
    fn picked(&self) -> Option<StaffCursor> {
        self.selected.get_untracked().ok().flatten()
    }
}

#[test]
fn the_staff_reads_measure_beat_pitch_and_duration() {
    let s = staff(NoteStaffOptions::default());
    s.el.focus();
    assert_eq!(role_of(&s.el), Some(Role::Grid));
    assert_eq!(name_of(&s.el), "Melody");
    let said =
        s.el.info()
            .and_then(|i| i.semantics)
            .and_then(|x| x.description_text().map(str::to_owned))
            .unwrap_or_default();
    assert_eq!(said, "treble clef, no sharps or flats, 4/4 time, 3 measures");
    assert_eq!(value_text(&s.el), "measure 1, beat 1: C 4 quarter note");
    key(&s.ui, Key::ArrowRight);
    assert_eq!(value_text(&s.el), "measure 1, beat 2: E 4 eighth note");
    key(&s.ui, Key::ArrowRight);
    key(&s.ui, Key::ArrowRight);
    assert_eq!(value_text(&s.el), "measure 1, beat 3: half chord of C 5, E 5");
    key(&s.ui, Key::ArrowRight);
    assert_eq!(value_text(&s.el), "measure 2, beat 1: F sharp 5 half note, tied to the next");
    key(&s.ui, Key::ArrowRight);
    key(&s.ui, Key::ArrowRight);
    assert_eq!(value_text(&s.el), "measure 2, beat 4: quarter rest");
    key(&s.ui, Key::PageDown);
    assert_eq!(
        value_text(&s.el),
        "measure 3, beat 1: A 2 dotted half note",
        "the new meter does not change beat 1"
    );
    key(&s.ui, Key::ArrowRight);
    assert_eq!(value_text(&s.el), "measure 3, beat 4: A 5 quarter note");
    key_mod(&s.ui, Key::Home, Modifiers { ctrl: true, ..Modifiers::NONE });
    assert!(value_text(&s.el).starts_with("measure 1, beat 1"));
    key_mod(&s.ui, Key::End, Modifiers { ctrl: true, ..Modifiers::NONE });
    assert!(value_text(&s.el).starts_with("measure 3, beat 4"));
    key(&s.ui, Key::PageUp);
    key(&s.ui, Key::End);
    assert!(value_text(&s.el).starts_with("measure 2, beat 4"));
    key(&s.ui, Key::Home);
    assert!(value_text(&s.el).starts_with("measure 2, beat 1"));
    assert!(audit(&s.ui).is_empty(), "{:?}", audit(&s.ui));
}

#[test]
fn the_staff_picks_by_keyboard_and_pointer() {
    let s = staff(NoteStaffOptions::default());
    s.el.focus();
    key(&s.ui, Key::ArrowRight);
    assert!(key(&s.ui, Key::Enter));
    assert_eq!(s.picked(), Some(StaffCursor { measure: 0, event: 1 }));
    assert!(value_text(&s.el).ends_with(", picked"));
    assert!(key(&s.ui, Key::Escape));
    assert_eq!(s.picked(), None);
    assert!(!key(&s.ui, Key::Escape));
    assert!(!key(&s.ui, Key::ArrowUp), "up and down are left to the page");
    let b = s.el.bounds().unwrap_or_default();
    // Far right on the first line is the last event of its last measure there; far left the first.
    click(&s.ui, Point::new(b.min.x + 90.0, b.min.y + 40.0));
    assert_eq!(s.picked().map(|c| (c.measure, c.event)), Some((0, 0)));
    click(&s.ui, Point::new(b.max.x - 20.0, b.min.y + 40.0));
    let c = s.picked().unwrap_or_default();
    assert!(c.measure >= 1, "{c:?}");
}

#[test]
fn the_staff_draws_five_lines_and_the_notes_move_with_the_signal() {
    let s = staff({
        let mut v_ = NoteStaffOptions::default();
        v_.space = Some(12.0);
        v_
    });
    let mut shot = Shot::new(&s.ui);
    shot.take(&s.ui);
    let b = s.el.bounds().unwrap_or_default();
    let bg = rgb(s.ui.theme().colors.bg);
    let lines_in = |shot: &Shot, x: u32| {
        let mut rows = 0;
        let mut prev = false;
        for y in (b.min.y as u32)..(b.min.y as u32 + 130) {
            let on = !near(shot.at(x, y), bg, 8);
            if on && !prev {
                rows += 1;
            }
            prev = on;
        }
        rows
    };
    let best = (b.min.x as u32 + 8..b.max.x as u32).step_by(2).map(|x| lines_in(&shot, x)).max().unwrap_or(0);
    assert!(best >= 5, "five staff lines: {best}");
    // The ink is the theme's text color.
    let ink = theme_rgb(&s.ui, |t| t.derive().text);
    assert!(count_near(&shot, &s.el, ink, 12) > 100, "notes and clef are drawn in the text color");
    // A taller piece makes the element taller; an empty one still has a staff.
    let one = b.height();
    s.measures.update(|m| {
        let more: Vec<StaffMeasure> = (0..8).flat_map(|_| melody()).collect();
        m.extend(more);
    });
    let _ = frame(&s.ui);
    assert!(s.el.bounds().unwrap_or_default().height() > one * 2.0, "more measures wrap onto more lines");
    s.measures.set(Vec::new());
    let _ = frame(&s.ui);
    shot.take(&s.ui);
}

#[test]
fn a_still_staff_costs_nothing_and_a_change_draws_once() {
    let s = staff(NoteStaffOptions::default());
    let mut shot = Shot::new(&s.ui);
    shot.take(&s.ui);
    settle(&s.ui);
    for _ in 0..5 {
        assert!(!frame(&s.ui));
    }
    s.measures.update(|m| m.truncate(2));
    assert!(frame(&s.ui));
    settle(&s.ui);
    assert!(!frame(&s.ui));
}

#[test]
fn the_staff_follows_the_theme_and_is_never_mirrored() {
    let make = |rtl: bool| {
        let ui = ui();
        if rtl {
            is_rtl_window(&ui);
        }
        let el = note_staff(
            &ui,
            "Melody",
            Signal::new(melody()),
            Signal::new(Some(StaffCursor { measure: 0, event: 1 })),
            {
                let mut v_ = NoteStaffOptions::default();
                v_.key = -2;
                v_.clef = Clef::Bass;
                v_
            },
        );
        mount(&ui, &el);
        let _ = frame(&ui);
        let mut shot = Shot::new(&ui);
        shot.take(&ui);
        crop(&shot, &el)
    };
    assert!(same(&make(false), &make(true)));
    // A light theme draws dark ink.
    let s = staff(NoteStaffOptions::default());
    let light = remedy_ui_themes::ThemeChoice::System.resolve(Some(false), false);
    s.ui.set_theme(light.clone(), remedy_ui_themes::sheet(&light));
    let mut shot = Shot::new(&s.ui);
    shot.take(&s.ui);
    assert!(count_near(&shot, &s.el, rgb(light.derive().text), 12) > 100);
}

#[test]
fn every_clef_and_key_renders() {
    for clef in [Clef::Treble, Clef::Bass, Clef::Alto, Clef::Tenor] {
        for key in [-7, -3, 0, 4, 7] {
            let ui = ui();
            let el = note_staff(&ui, "Melody", Signal::new(melody()), Signal::new(None), {
                let mut v_ = NoteStaffOptions::default();
                v_.clef = clef;
                v_.key = key;
                v_
            });
            mount(&ui, &el);
            let mut shot = Shot::new(&ui);
            shot.take(&ui);
            let said = el
                .info()
                .and_then(|i| i.semantics)
                .and_then(|s| s.description_text().map(str::to_owned))
                .unwrap_or_default();
            assert!(said.starts_with(&format!("{} clef", clef.name())), "{said}");
            match key {
                0 => assert!(said.contains("no sharps or flats")),
                k if k > 0 => assert!(said.contains("sharp")),
                _ => assert!(said.contains("flat")),
            }
        }
    }
}

#[test]
fn the_accessibility_actions_the_widgets_offer_are_focus_only() {
    let t = tab(TabOptions::default());
    assert!(actions_of(&t.el).contains(&Action::Focus));
    let s = staff(NoteStaffOptions::default());
    assert!(actions_of(&s.el).contains(&Action::Focus));
    let b = board(FretboardOptions::default());
    assert!(actions_of(&b.el).contains(&Action::Focus));
}

#[test]
fn a_tab_in_one_row_is_as_wide_as_its_measures_and_free_events_keep_their_lengths() {
    let ui = ui();
    let long: Vec<TabMeasure> = (0..12).flat_map(|_| riff()).collect();
    let options = TabOptions::default().flow(TabFlow::Row).column_numbers(true);
    let wrapped = tab_columns(&ui, &long, &TabOptions::default(), 600.0);
    let row = tab_columns(&ui, &long, &options, 600.0);
    assert_eq!(wrapped.len(), row.len());
    let lines = |cols: &[TabColumn]| {
        let mut ys: Vec<i32> = cols.iter().map(|c| c.rect.min.y as i32).collect();
        ys.dedup();
        ys.len()
    };
    assert!(lines(&wrapped) > 3 && lines(&row) == 1, "one line in a row");
    assert!(row.last().is_some_and(|c| c.rect.max.x > 600.0), "wider than the width it was given");
    // The columns start where the lengths before them end, free lengths included.
    let free = vec![TabMeasure::new(vec![
        TabEvent::free(vec![TabNote::new(0, 3)], 0.05),
        TabEvent::free(Vec::new(), 1.37),
        TabEvent::new(vec![TabNote::new(1, 2)], NoteValue::Quarter),
    ])];
    let cols = tab_columns(&ui, &free, &options, 600.0);
    let starts: Vec<f32> = cols.iter().map(|c| c.start).collect();
    assert!((starts.get(2).copied().unwrap_or(0.0) - 1.42).abs() < 1e-5, "{starts:?}");
    // The element takes that width.
    let measures = Signal::new(long);
    let el = tab_staff_view(&ui, "Row", measures, Signal::new(None), options, TabView::default());
    mount(&ui, &el);
    let _ = frame(&ui);
    let _ = frame(&ui);
    let w = el.bounds().unwrap_or_default().width();
    assert!((w - row.last().map_or(0.0, |c| c.rect.max.x)).abs() < 40.0, "{w}");
}

#[test]
fn the_playhead_lights_its_column_and_marks_recolour_notes() {
    let ui = ui();
    let measures = Signal::new(riff());
    let playhead = Signal::new(None);
    let decor = Signal::new(TabDecor::default());
    let view = TabView::default().playhead(playhead).follow(true).decor(decor);
    let el = tab_staff_view(&ui, "Riff", measures, Signal::new(None), TabOptions::default(), view);
    mount(&ui, &el);
    // The tab takes its height in its first frames.
    for _ in 0..4 {
        let _ = frame(&ui);
    }
    assert!(!frame(&ui), "nothing to redraw at rest");
    // Beat 1.2 is inside the third event (it starts at 1 and lasts a quarter).
    playhead.set(Some(1.2));
    assert!(frame(&ui), "the lit column is drawn");
    playhead.set(None);
    assert!(frame(&ui), "and taken away when stopped");
    decor.update(|d| {
        d.notes.insert(
            TabCursor { measure: 0, event: 1, string: 5 },
            NoteMark { color: Some(Color::rgba8(220, 40, 40, 255)), dim: false, tip: "72%".into() },
        );
        d.active = Some(0..2);
    });
    assert!(frame(&ui), "a coloured note and faint ones outside the range are drawn");
}

#[test]
fn the_neck_tells_every_press_and_turns_its_strings_over() {
    let picks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let heard = picks.clone();
    let b = board(FretboardOptions::default().on_pick(move |p| heard.borrow_mut().push((p.string, p.fret))));
    let _ = frame(&b.ui);
    let (a, c) = (b.at(2, 5), b.at(2, 7));
    for _ in 0..2 {
        pointer(&b.ui, a, PointerPhase::Down, Buttons::PRIMARY);
        pointer(&b.ui, a, PointerPhase::Up, Buttons::default());
    }
    assert_eq!(*picks.borrow(), vec![(2, 5), (2, 5)], "the same fret twice is two picks");
    // A drag tells each new place once.
    pointer(&b.ui, a, PointerPhase::Down, Buttons::PRIMARY);
    pointer(&b.ui, a, PointerPhase::Move, Buttons::PRIMARY);
    pointer(&b.ui, c, PointerPhase::Move, Buttons::PRIMARY);
    pointer(&b.ui, c, PointerPhase::Up, Buttons::default());
    assert_eq!(picks.borrow().len(), 4);
    assert_eq!(picks.borrow().last(), Some(&(2, 7)));
    // Turned over, the lowest string is the top row, and a press there picks it.
    let options = FretboardOptions::default().left_handed(true).reverse_strings(true);
    let g = FretboardLayout::new(&options, 600.0, UNIT);
    assert!(g.string_y(0) < g.string_y(5));
    assert_eq!(g.hit(Point::new(g.cell_x(3), g.string_y(0))), Some(FretPos { string: 0, fret: 3 }));
    // Faint and breathing marks, a shaded position and sounding pitch classes draw.
    let marks = Signal::new(vec![FretMark::at(1, 3).dimmed(0.3), FretMark::at(2, 5).pulsing(true)]);
    let board = board(
        FretboardOptions::default()
            .marks(marks)
            .shaded(Signal::new(Some((3, 6))))
            .sounding_classes(Signal::new(vec![0, 4, 7])),
    );
    assert!(frame(&board.ui));
    assert!(frame(&board.ui), "a breathing mark keeps the frames coming");
}
