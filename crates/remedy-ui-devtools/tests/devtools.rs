//! Previews, golden images, the accessibility check and the inspector.

use std::time::Duration;

use remedy_ui_core::{Color, Point, Size};
use remedy_ui_devtools::*;
use remedy_ui_input::{Buttons, PointerEvent, PointerKind, PointerPhase};
use remedy_ui_style::StylePatch;
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;
use remedy_ui_widgets_core as w;

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
    let _ = ui.take_requests();
}

fn ptr(phase: PointerPhase, p: Point) -> PointerEvent {
    PointerEvent {
        id: 0,
        kind: PointerKind::Mouse,
        phase,
        position: p,
        buttons: if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() },
        pressure: 0.5,
        time: Duration::ZERO,
        coalesced: Vec::new(),
        modifiers: remedy_ui_input::Modifiers::NONE,
    }
}

/// A preview's first variation.
fn v0(p: &Preview) -> Variation {
    p.variations().into_iter().next().unwrap_or_else(|| unreachable!("a variation"))
}

/// A red box at the start of a row.
fn marker() -> Preview {
    Preview::new("marker", |ui| {
        w::row(ui).fill_width().child(
            ui.el("box").width(40.0).height(40.0).style(StylePatch {
                background: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
                ..StylePatch::default()
            }),
        )
    })
    .sizes(&[(200.0, 60.0)])
}

#[test]
fn previews_render_every_variation_and_mirror_right_to_left() {
    let p = marker().themes(&["forest", "light"]).locales(&["en-US", "ar-EG"]).text_scales(&[1.0, 1.5]);
    assert_eq!(p.variations().len(), 8);
    let ltr = p.render(&v0(&p));
    assert_eq!((ltr.width, ltr.height), (200, 60));
    let red = |s: &Snapshot, x| s.pixel(x, 20).is_some_and(|px| px[0] > 200 && px[1] < 40);
    assert!(red(&ltr, 10) && !red(&ltr, 190), "starts at the left");
    let rtl = p.render(&Variation { locale: "ar-EG".into(), ..v0(&p).clone() });
    assert!(red(&rtl, 190) && !red(&rtl, 10), "starts at the right in Arabic");
    assert!(is_rtl("he-IL") && !is_rtl("en-GB"));
    let light = p.render(&Variation { theme: "light".into(), ..v0(&p).clone() });
    assert_ne!(light.pixel(150, 50), ltr.pixel(150, 50), "the light palette has another background");
}

#[test]
fn golden_images_catch_changes_and_round_trip_through_png() {
    let p = marker();
    let v = &v0(&p);
    let shot = p.render(v);
    let png = shot.to_png().unwrap_or_default();
    let back = Snapshot::from_png(&png).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(shot.diff(&back, 1), 0, "PNG keeps the pixels");
    let dir = std::env::temp_dir().join(format!("remedy-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(check_golden(&p, &dir), Err(GoldenError::Missing(_))), "first run writes a baseline");
    assert_eq!(check_golden(&p, &dir), Ok(()), "unchanged UI passes");
    let moved =
        Preview::new("marker", |ui| {
            w::row(ui).fill_width().child(ui.el("box").width(80.0).height(40.0).style(StylePatch {
                background: Some(Color::new(1.0, 0.0, 0.0, 1.0)),
                ..StylePatch::default()
            }))
        })
        .sizes(&[(200.0, 60.0)]);
    match check_golden(&moved, &dir) {
        Err(GoldenError::Differs { pixels, actual, .. }) => {
            assert!(pixels >= 40 * 40, "{pixels}");
            assert!(actual.exists(), "the new rendering is written for review");
        }
        other => unreachable!("expected a difference: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_audit_finds_unnamed_controls_low_contrast_and_small_targets() {
    let ui = ui();
    let bg = ui.theme().colors.bg;
    let good = w::button(&ui, "Save", || {});
    let unnamed = ui.el("box").width(40.0).height(40.0).on_click(|| {});
    let tiny = w::icon_button(&ui, w::Icon::Close, "Close", || {}).width(16.0).height(16.0);
    let faint = w::label(&ui, "Barely there", "")
        .style(StylePatch { foreground: Some(bg.with_alpha(0.9)), ..StylePatch::default() });
    let gone = w::label(&ui, "Hidden", "")
        .style(StylePatch { foreground: Some(bg), ..StylePatch::default() })
        .hidden(true);
    ui.root().child(
        w::column(&ui)
            .child(good.clone())
            .child(unnamed.clone())
            .child(tiny.clone())
            .child(faint.clone())
            .child(gone),
    );
    frame(&ui);
    let found = audit(&ui);
    let of = |id| found.iter().filter(move |p| p.element == id).map(|p| p.kind.clone()).collect::<Vec<_>>();
    assert!(of(good.id()).is_empty(), "{found:?}");
    assert_eq!(of(unnamed.id()), vec![ProblemKind::Unnamed]);
    assert!(matches!(of(tiny.id()).as_slice(), [ProblemKind::SmallTarget { .. }]), "{found:?}");
    let faint_text = faint.id();
    assert!(
        found.iter().any(|p| p.element == faint_text && matches!(p.kind, ProblemKind::LowContrast { .. })),
        "{found:?}"
    );
    assert_eq!(found.len(), 3, "the hidden label is skipped: {found:?}");
    let msg = found.iter().find(|p| p.kind == ProblemKind::Unnamed).map(Problem::message).unwrap_or_default();
    assert!(msg.contains("devtools.rs"), "points at the source line: {msg}");
}

#[test]
fn the_inspector_picks_outlines_and_edits_theme_colors() {
    let target = ui();
    let b = w::button(&target, "Unlock", || {});
    target.root().child(w::column(&target).padding(remedy_ui_core::Insets::all(20.0)).child(b.clone()));
    frame(&target);
    let insp = Inspector::attach(&target, remedy_ui_themes::sheet);
    insp.pick(true);
    let c = b
        .bounds()
        .map(|r| Point::new((r.min.x + r.max.x) / 2.0, (r.min.y + r.max.y) / 2.0))
        .unwrap_or_default();
    assert!(target.pointer(&ptr(PointerPhase::Move, c)), "picking takes the pointer");
    frame(&target);
    let outlines = target.layers()[1].child_elements().len();
    assert!(outlines >= 1, "an outline follows the pointer");
    target.pointer(&ptr(PointerPhase::Down, c));
    let picked = insp.selected().map(|id| target.element(id).info().map(|i| i.widget));
    assert!(matches!(picked, Some(Some("button" | "button_label"))), "{picked:?}");
    assert!(!insp.is_picking(), "a press ends picking");
    let n = std::rc::Rc::new(std::cell::Cell::new(0));
    let n2 = n.clone();
    let b2 = w::button(&target, "After", move || n2.set(n2.get() + 1));
    target.root().child(b2.clone());
    frame(&target);
    let c2 = b2
        .bounds()
        .map(|r| Point::new((r.min.x + r.max.x) / 2.0, (r.min.y + r.max.y) / 2.0))
        .unwrap_or_default();
    target.pointer(&ptr(PointerPhase::Down, c2));
    target.pointer(&ptr(PointerPhase::Up, c2));
    assert_eq!(n.get(), 1, "input works again after picking");

    insp.set_color("accent", Color::new(1.0, 0.0, 1.0, 1.0));
    assert_eq!(target.theme().colors.accent, Color::new(1.0, 0.0, 1.0, 1.0));

    // The panel builds in its own window's Ui and lists what it found.
    let panel_ui = ui();
    panel_ui.root().child(inspector_panel(&panel_ui, &insp));
    panel_ui.connect_accessibility();
    frame(&panel_ui);
    let tree = format!("{:?}", panel_ui.take_accessibility_update());
    assert!(
        tree.contains("Pick an element") && tree.contains("Built at") && tree.contains("button"),
        "{tree}"
    );
    insp.detach();
    frame(&target);
    assert_eq!(target.layers()[1].child_elements().len(), outlines - 1, "the outline is gone");
}

#[test]
fn the_gallery_shows_each_variation_with_a_caption() {
    let ui = ui();
    ui.connect_accessibility();
    let p = marker().themes(&["forest", "light"]);
    ui.root().child(gallery(&ui, &[p]));
    frame(&ui);
    let tree = format!("{:?}", ui.take_accessibility_update());
    assert!(tree.contains("forest · en-US · 200×60 · 1.0×") && tree.contains("light · en-US"), "{tree}");
}

/// Writes PNGs of the form widgets and the inspector to `REMEDY_SHOTS` (run with
/// `--ignored`), for a person to look at.
#[test]
#[ignore = "writes images for review"]
fn shots() {
    use remedy_ui_reactive::Signal;
    use remedy_ui_widgets_forms as f;
    let Some(dir) = std::env::var_os("REMEDY_SHOTS").map(std::path::PathBuf::from) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let forms = Preview::new("forms", |ui| {
        let d = |s: &str| f::Decimal::parse(s).unwrap_or_default();
        let unit = ui.theme().spacing.unit;
        let date = f::Date::new(2026, 11, 17).unwrap_or(f::Date { year: 2026, month: 11, day: 17 });
        w::column(ui)
            .gap(unit * 3.0)
            .padding(remedy_ui_core::Insets::all(unit * 4.0))
            .fill_width()
            .child(f::number_input(
                ui,
                "Lye",
                Signal::new(d("123.45")),
                f::NumberOptions::range(d("0"), d("1000"), d("0.5"), 2).unit("g"),
            ))
            .child(f::time_picker(ui, "Remind at", Signal::new(f::Time::new(14, 30).unwrap_or_default()), 15))
            .child(f::key_capture(
                ui,
                "Shortcut",
                Signal::new(Some(remedy_ui_input::Chord::new(
                    remedy_ui_input::Modifiers {
                        ctrl: true,
                        shift: true,
                        ..remedy_ui_input::Modifiers::NONE
                    },
                    remedy_ui_input::Key::Character("k".into()),
                ))),
            ))
            .child(f::date_picker(ui, "Due", Signal::new(date), None, None))
            .child(f::calendar(ui, "Calendar", Signal::new(date), None, None))
            .child(f::color_picker(
                ui,
                "Accent",
                Signal::new(Color::from_hex("#3d8b5f").unwrap_or(Color::BLACK)),
                Some(Color::WHITE),
            ))
    })
    .sizes(&[(420.0, 1100.0)])
    .locales(&["en-US", "de-DE", "ar-EG", "en-XA", "ar-XB"]);
    for v in forms.variations() {
        let s = forms.render(&v);
        let _ = std::fs::write(dir.join(format!("forms-{}.png", v.locale)), s.to_png().unwrap_or_default());
    }
    let insp = Preview::new("inspector", |ui| {
        let target = self::ui();
        let b = w::button(&target, "Unlock", || {});
        target.root().child(
            w::column(&target)
                .child(b.clone())
                .child(target.el("box").width(30.0).height(30.0).on_click(|| {})),
        );
        frame(&target);
        let i = Inspector::attach(&target, remedy_ui_themes::sheet);
        i.select(Some(b.id()));
        // Keep the target alive with the panel.
        let keep = target.clone();
        inspector_panel(ui, &i).on_removed(move || drop(keep))
    })
    .sizes(&[(480.0, 900.0)]);
    let s = insp.render(&v0(&insp));
    let _ = std::fs::write(dir.join("inspector.png"), s.to_png().unwrap_or_default());
}

#[test]
fn start_aligned_text_follows_the_layout_direction() {
    let p =
        Preview::new("label", |ui| w::column(ui).fill_width().child(w::label(ui, "Start", "").fill_width()))
            .sizes(&[(300.0, 40.0)])
            .locales(&["en-US", "ar-EG"]);
    let ink = |s: &Snapshot, x0: u32, x1: u32| {
        let bg = s.pixel(299, 39).unwrap_or_default();
        (x0..x1).any(|x| {
            (0..40)
                .any(|y| s.pixel(x, y).is_some_and(|px| px.iter().zip(bg).any(|(a, b)| a.abs_diff(b) > 60)))
        })
    };
    let vs = p.variations();
    let (Some(ltr), Some(rtl)) = (vs.first(), vs.get(1)) else { unreachable!() };
    let (l, r) = (p.render(ltr), p.render(rtl));
    assert!(ink(&l, 0, 100) && !ink(&l, 200, 300), "left to right: text at the left");
    assert!(ink(&r, 200, 300) && !ink(&r, 0, 100), "right to left: text at the right");
}

#[test]
fn wrapped_text_and_fields_stay_inside_their_boxes_right_to_left() {
    // Ink columns of a snapshot: (leftmost, rightmost) x that differs from the background.
    let extent = |s: &Snapshot, y0: u32, y1: u32| {
        let bg = s.pixel(s.width - 1, s.height - 1).unwrap_or_default();
        let mut cols = (0..s.width).filter(|x| {
            (y0..y1)
                .any(|y| s.pixel(*x, y).is_some_and(|px| px.iter().zip(bg).any(|(a, b)| a.abs_diff(b) > 60)))
        });
        let first = cols.next();
        (first, cols.next_back().or(first))
    };
    let long = "The quick brown fox jumps over the lazy dog while the boxing wizards jump quickly";
    let p = Preview::new("wrapped", move |ui| {
        w::column(ui).fill_width().child(w::label(ui, long, "").fill_width())
    })
    .sizes(&[(240.0, 120.0)])
    .locales(&["ar-EG"]);
    let s = p.render(&v0(&p));
    if let Some(d) = std::env::var_os("REMEDY_SHOTS") {
        let _ =
            std::fs::write(std::path::Path::new(&d).join("rtl-wrapped.png"), s.to_png().unwrap_or_default());
    }
    let (l, r) = extent(&s, 0, 120);
    assert!(
        r.is_some_and(|r| r <= 239) && r.is_some_and(|r| r >= 225),
        "lines end at the right edge: {l:?}..{r:?}"
    );
    assert!(l.is_some_and(|l| l < 60), "wrapped lines fill the width: {l:?}");
    let fields = Preview::new("fields", |ui| {
        let one = w::text_input(ui, "Name");
        one.set_value("Ada");
        let many = w::text_area(ui, "Notes");
        many.set_value("Short");
        w::column(ui)
            .fill_width()
            .gap(10.0)
            .child(one.el().fill_width())
            .child(many.el().fill_width().height(60.0))
    })
    .sizes(&[(300.0, 120.0)])
    .locales(&["ar-EG"]);
    let s = fields.render(&v0(&fields));
    // Text sits in the right half of both fields (borders aside: look inside them).
    let (l1, _) = extent(&Snapshot { rgba: s.rgba.clone(), ..s.clone() }, 8, 28);
    assert!(l1.is_some_and(|x| x > 150), "single-line field text at the right: {l1:?}");
    let (l2, _) = extent(&s, 50, 70);
    assert!(l2.is_some_and(|x| x > 150), "multi-line field text at the right: {l2:?}");
}

#[test]
fn every_widget_passes_the_accessibility_check() {
    use remedy_ui_reactive::Signal;
    use remedy_ui_widgets_forms as f;
    let ui = ui();
    let d = |s: &str| f::Decimal::parse(s).unwrap_or_default();
    let date = f::Date::new(2026, 11, 17).unwrap_or(f::Date { year: 2026, month: 11, day: 17 });
    let col = w::column(&ui)
        .gap(8.0)
        .child(w::button(&ui, "Save", || {}))
        .child(w::icon_button(&ui, w::Icon::Close, "Close", || {}))
        .child(w::toggle_button(&ui, "Bold", Signal::new(false)))
        .child(w::menu_button(&ui, "More", vec![w::MenuItem::new("Copy", || {})]))
        .child(w::check(&ui, "Remember me", Signal::new(true)))
        .child(w::switch(&ui, "Dark mode", Signal::new(false)))
        .child(w::radio_group(&ui, "Size", &["Small", "Large"], Signal::new(0)))
        .child(w::segmented(&ui, "View", &["List", "Grid"], Signal::new(0)))
        .child(w::select(&ui, "Theme", vec!["Forest".into(), "Light".into()], Signal::new(0)))
        .child(w::chip(&ui, "Unread", Signal::new(false)))
        .child(w::slider(&ui, "Volume", Signal::new(0.5), 0.0, 1.0, 0.1))
        .child(w::stepper(&ui, "Copies", Signal::new(1.0), 1.0, 9.0, 1.0))
        .child(w::text_input(&ui, "Name").el())
        .child(w::text_area(&ui, "Notes").el())
        .child(w::search(&ui, "Search", |_| {}))
        .child(w::secret_input(&ui, "Password").el())
        .child(w::link(&ui, "Help", || {}))
        .child(f::number_input(
            &ui,
            "Grams",
            Signal::new(d("12.5")),
            f::NumberOptions::range(d("0"), d("100"), d("0.5"), 1),
        ))
        .child(f::time_picker(&ui, "At", Signal::new(f::Time::default()), 15))
        .child(f::key_capture(&ui, "Shortcut", Signal::new(None)))
        .child(f::date_picker(&ui, "Due", Signal::new(date), None, None))
        .child(f::calendar(&ui, "Calendar", Signal::new(date), None, None))
        .child(f::color_picker(
            &ui,
            "Accent",
            Signal::new(Color::from_hex("#3d8b5f").unwrap_or(Color::BLACK)),
            Some(Color::WHITE),
        ));
    ui.root().child(w::scroll(&ui).grow(1.0).child(col));
    let _ = ui.frame(Size::new(480.0, 2400.0), 1.0);
    let _ = ui.take_requests();
    let found: Vec<String> = audit(&ui)
        .iter()
        .map(|p| {
            format!(
                "{} [{}]",
                p.message(),
                ui.element(p.element).info().map(|i| describe(&i)).unwrap_or_default()
            )
        })
        .collect();
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn the_audit_finds_pointer_only_and_invisible_focus() {
    let ui = ui();
    let grip = ui.el("box").width(30.0).height(30.0).on_pointer(|_| true);
    let excused = ui
        .el("box")
        .width(30.0)
        .height(30.0)
        .on_pointer(|_| true)
        .keyboard_alternative("The window menu's Size command");
    let ghost = ui.el("box").width(0.0).height(0.0).focusable(true);
    ui.root().child(w::column(&ui).child(grip.clone()).child(excused.clone()).child(ghost.clone()));
    frame(&ui);
    let found = audit(&ui);
    let kinds =
        |id| found.iter().filter(move |p| p.element == id).map(|p| p.kind.clone()).collect::<Vec<_>>();
    assert_eq!(kinds(grip.id()), vec![ProblemKind::PointerOnly]);
    assert!(kinds(excused.id()).is_empty(), "{found:?}");
    assert_eq!(kinds(ghost.id()), vec![ProblemKind::InvisibleFocus]);
}

#[test]
fn the_audit_finds_focus_that_does_not_show() {
    let ui = ui();
    // The default ring shows focus; so does a rule that changes the background instead.
    let ringed = ui
        .el("box")
        .width(40.0)
        .height(40.0)
        .focusable(true)
        .semantics(remedy_ui_a11y::Semantics::new(remedy_ui_a11y::Role::Button).label("Ringed"));
    let unmarked = ui
        .el("box")
        .width(40.0)
        .height(40.0)
        .focusable(true)
        .semantics(remedy_ui_a11y::Semantics::new(remedy_ui_a11y::Role::Button).label("Unmarked"))
        .style(StylePatch { outline_width: Some(0.0), ..StylePatch::default() });
    ui.root().child(w::column(&ui).child(ringed.clone()).child(unmarked.clone()));
    frame(&ui);
    let found = audit(&ui);
    let kinds =
        |id| found.iter().filter(move |p| p.element == id).map(|p| p.kind.clone()).collect::<Vec<_>>();
    assert!(kinds(ringed.id()).is_empty(), "{found:?}");
    assert_eq!(kinds(unmarked.id()), vec![ProblemKind::NoFocusIndicator]);
}
