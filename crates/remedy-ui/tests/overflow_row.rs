//! A one-line overflow bar: children that do not fit move into a More menu,
//! and nothing paints past the bar's box.

use remedy_ui::a11y::Role;
use remedy_ui::layout::Length;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

fn app(width: f32, labels: &'static [&'static str]) -> TestApp {
    TestApp::new(AppConfig::new("overflow-row"), move |app: &App| {
        app.open(WindowSpec::new("overflow-row").size(width, 80.0), move |ui, _| {
            let mut bar = w::overflow_row(ui).gap(4.0);
            for &label in labels {
                bar = bar.child(label, w::button(ui, label, || {}));
            }
            ui.root().child(bar.el().layout(|l| {
                l.width = Length::Px(width);
                l.min_width = Length::Px(0.0);
                l.max_width = Length::Px(width);
            }));
        });
    })
}

fn button_names(app: &TestApp) -> Vec<String> {
    app.reader_view().into_iter().filter(|n| n.role == Role::Button).filter_map(|n| n.name).collect()
}

fn max_x(app: &TestApp) -> f32 {
    app.find_all(Role::Button).into_iter().filter_map(|el| el.bounds()).map(|b| b.max.x).fold(0.0, f32::max)
}

#[test]
fn overflow_row_moves_trailing_buttons_into_a_more_menu() {
    const LABELS: &[&str] = &["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot"];
    let mut narrow = app(220.0, LABELS);
    narrow.frames(4);
    let names = button_names(&narrow);
    assert!(names.iter().any(|n| n == "More"), "narrow bar shows More, got {names:?}");
    assert!(names.iter().any(|n| n == "Alpha"), "the first button stays in the row: {names:?}");
    // Hidden controls stay in a 0-size bucket (so More can still activate them)
    // but More is the visible overflow affordance, and nothing paints past the window.
    assert!(max_x(&narrow) <= 221.0, "nothing may paint past the 220 px bar, max_x={}", max_x(&narrow));

    assert!(narrow.press(Role::Button, "More"), "open the More menu");
    narrow.frames(2);
    let after = narrow.reader_view().into_iter().filter_map(|n| n.name).collect::<Vec<_>>();
    assert!(
        after.iter().any(|n| n == "Foxtrot" || n == "Echo" || n == "Delta"),
        "More menu lists the overflowed buttons: {after:?}"
    );

    let mut wide = app(800.0, LABELS);
    wide.frames(4);
    let names = button_names(&wide);
    assert!(names.iter().all(|n| n != "More"), "a wide bar keeps every button, no More: {names:?}");
    for label in LABELS {
        assert!(names.iter().any(|n| n == label), "{label} should stay in the wide bar: {names:?}");
    }
}

#[test]
fn overflow_row_keeps_a_flex_child_and_clips_to_its_box() {
    let mut app = TestApp::new(AppConfig::new("overflow-keep"), move |app: &App| {
        app.open(WindowSpec::new("overflow-keep").size(240.0, 80.0), move |ui, _| {
            let field = w::text_input(ui, "Address")
                .placeholder("https://example.invalid/a/very/long/path")
                .el()
                .layout(|l| {
                    l.grow = 1.0;
                    l.shrink = 1.0;
                    l.min_width = Length::Px(0.0);
                });
            let bar = w::overflow_row(ui)
                .gap(4.0)
                .keep(w::button(ui, "Back", || {}))
                .keep(w::button(ui, "Forward", || {}))
                .flex(field)
                .child("Bookmark", w::button(ui, "Bookmark", || {}))
                .child("Bookmarks", w::button(ui, "Bookmarks", || {}))
                .child("Desktop site", w::button(ui, "Desktop site", || {}))
                .el();
            ui.root().child(bar.layout(|l| {
                l.width = Length::Px(240.0);
                l.min_width = Length::Px(0.0);
                l.max_width = Length::Px(240.0);
            }));
        });
    });
    app.frames(4);
    let names = button_names(&app);
    assert!(names.iter().any(|n| n == "Back"), "{names:?}");
    assert!(names.iter().any(|n| n == "Forward"), "{names:?}");
    assert!(names.iter().any(|n| n == "More"), "trailing actions overflow at 240 px: {names:?}");
    assert!(max_x(&app) <= 241.0, "nothing may paint past the 240 px bar, max_x={}", max_x(&app));
}

#[test]
fn overflow_row_collapsed_slots_do_not_force_a_more_menu() {
    // Two 80 px buttons fit in 200 px. Two collapsed slots must not reserve
    // MIN_TARGET each and push a button into the More menu.
    let mut app = TestApp::new(AppConfig::new("overflow-maybe"), move |app: &App| {
        app.open(WindowSpec::new("overflow-maybe").size(200.0, 80.0), move |ui, _| {
            let collapsed = ui.el("collapsed").layout(|l| {
                l.width = Length::Px(0.0);
                l.max_width = Length::Px(0.0);
                l.min_width = Length::Px(0.0);
                l.height = Length::Px(0.0);
            });
            let collapsed2 = ui.el("collapsed-2").layout(|l| {
                l.width = Length::Px(0.0);
                l.max_width = Length::Px(0.0);
                l.min_width = Length::Px(0.0);
                l.height = Length::Px(0.0);
            });
            let bar = w::overflow_row(ui)
                .gap(4.0)
                .child("Alpha", w::button(ui, "Alpha", || {}))
                .child_maybe("Hidden", collapsed)
                .child_maybe("Hidden 2", collapsed2)
                .child("Bravo", w::button(ui, "Bravo", || {}))
                .el();
            ui.root().child(bar.layout(|l| {
                l.width = Length::Px(200.0);
                l.min_width = Length::Px(0.0);
                l.max_width = Length::Px(200.0);
            }));
        });
    });
    app.frames(4);
    let names = button_names(&app);
    assert!(names.iter().any(|n| n == "Alpha"), "{names:?}");
    assert!(names.iter().any(|n| n == "Bravo"), "collapsed slots must not hide Bravo: {names:?}");
    assert!(names.iter().all(|n| n != "More"), "collapsed slots must not force More: {names:?}");
    assert!(max_x(&app) <= 201.0, "nothing may paint past the 200 px bar, max_x={}", max_x(&app));
}
