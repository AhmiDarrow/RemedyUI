//! The router on the in-memory host: pages per route, back keys, titles, scroll restore,
//! links and deep links.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::a11y::Role;
use remedy_ui::core::Vec2;
use remedy_ui::input::{Key, KeyEvent, Modifiers};
use remedy_ui::router::Router;
use remedy_ui::testing::TestApp;
use remedy_ui::view::El;
use remedy_ui::widgets as w;

type Slot<T> = Rc<RefCell<Option<T>>>;

/// An app whose window shows `router.view`: a scrolling page per route with a link home.
fn app() -> (TestApp, Slot<Router>, Slot<El>) {
    let (router, scroller): (Slot<Router>, Slot<El>) = Default::default();
    let (r2, s2) = (router.clone(), scroller.clone());
    let app = TestApp::window(move |ui, window| {
        let router = Router::for_window(window, "/home");
        router.set_title(|r| format!("Portal: {}", r.segments().first().map_or("home", String::as_str)));
        let (me, ui2) = (router.clone(), ui.clone());
        let view = router.view(ui, move |route| {
            let tall = w::column(&ui2).child(w::text(&ui2, format!("Page {}", route.path())));
            let tall = (0..40).fold(tall, |c, i| c.child(w::text(&ui2, format!("row {i}"))));
            let scroll = w::scroll(&ui2).child(tall).grow(1.0);
            me.remember_scroll(&scroll);
            *s2.borrow_mut() = Some(scroll.clone());
            w::column(&ui2).grow(1.0).child(me.link_to(&ui2, "/settings/audio", "Settings")).child(scroll)
        });
        ui.root().child(view);
        *r2.borrow_mut() = Some(router);
    });
    (app, router, scroller)
}

fn title(app: &TestApp) -> String {
    app.runner().app().windows().first().map(|w| w.title()).unwrap_or_default()
}

#[test]
fn pages_follow_the_route_and_back_keys_go_back() {
    let (mut app, router, _) = app();
    let router = router.borrow().clone().unwrap();
    assert!(app.node(Role::Label, "Page /home").is_some() || app.find(Role::Label, "Page /home").is_some());
    assert_eq!(title(&app), "Portal: home");

    assert!(app.press(Role::Link, "Settings"));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/settings/audio");
    assert_eq!(title(&app), "Portal: settings");
    assert!(app.find(Role::Label, "Page /settings/audio").is_some());

    // The system back key (Android's back, a keyboard's browser-back key).
    app.key(KeyEvent::press(Key::Back));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/home");
    assert_eq!(title(&app), "Portal: home");

    // Alt+Right goes forward again, Alt+Left back.
    app.key(KeyEvent::press(Key::ArrowRight).with_modifiers(Modifiers { alt: true, ..Modifiers::NONE }));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/settings/audio");
    app.key(KeyEvent::press(Key::ArrowLeft).with_modifiers(Modifiers { alt: true, ..Modifiers::NONE }));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/home");
}

#[test]
fn links_carry_their_address_for_screen_readers() {
    let (app, _, _) = app();
    let link = app.node(Role::Link, "Settings").expect("the link");
    assert_eq!(link.url.as_deref(), Some("#/settings/audio"));
}

#[test]
fn going_back_restores_the_scroll_position() {
    let (mut app, router, scroller) = app();
    let router = router.borrow().clone().unwrap();
    let ui = app.ui().unwrap();
    let home = scroller.borrow().clone().unwrap();
    ui.set_scroll_offset(home.id(), Vec2::new(0.0, 120.0));
    router.push("/browse");
    app.frames(1);
    assert_eq!(router.saved_scroll(), Vec2::ZERO, "a new entry starts at the top");
    assert!(router.back());
    app.frames(1);
    assert_eq!(router.saved_scroll(), Vec2::new(0.0, 120.0));
    let rebuilt = scroller.borrow().clone().unwrap();
    assert_ne!(rebuilt.id(), home.id(), "the page was built again");
}

#[test]
fn reload_rebuilds_the_page_at_its_route() {
    let (mut app, router, scroller) = app();
    let router = router.borrow().clone().unwrap();
    router.push("/server");
    app.frames(1);
    let before = scroller.borrow().clone().unwrap().id();
    app.runner().app().reload();
    app.frames(1);
    assert_ne!(scroller.borrow().clone().unwrap().id(), before);
    assert_eq!(router.route_untracked().path(), "/server");
}

#[test]
#[cfg(feature = "desktop")]
fn deep_links_open_their_page() {
    use remedy_ui::host::HostEvent;
    let (mut app, router, _) = app();
    let router = router.borrow().clone().unwrap();
    router.follow_links(app.runner().app(), &["portal"]);
    // A tray item's "Open settings", or a link opened while the app runs: a second launch's
    // arguments.
    app.host().send(HostEvent::SecondInstance(vec!["portal://settings/7".into()]));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/settings/7");
    assert_eq!(title(&app), "Portal: settings");
    assert!(router.back());
    assert_eq!(router.route_untracked().path(), "/home");
}
