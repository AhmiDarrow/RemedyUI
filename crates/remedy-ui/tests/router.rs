//! The router on the in-memory host: pages per route, back keys, titles, scroll restore,
//! links and deep links.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::a11y::Role;
use remedy_ui::core::Vec2;
use remedy_ui::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui::layout::Length;
use remedy_ui::router::Router;
use remedy_ui::testing::TestApp;
use remedy_ui::view::El;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, Window, WindowSpec};

type Slot<T> = Rc<RefCell<Option<T>>>;

trait Must<T> {
    fn must(self) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self) -> T {
        self.unwrap_or_else(|| unreachable!("set by the window's build"))
    }
}

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
            let scroll = w::scroll(&ui2).child(tall).layout(|l| l.height = Length::Px(120.0));
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
    let router = router.borrow().clone().must();
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

/// Presses and releases the mouse's `button` (back or forward) over the page.
fn mouse_button(app: &mut TestApp, button: Buttons) {
    let window = app.runner().app().windows().first().cloned().must();
    let p = remedy_ui::core::Point::new(200.0, 200.0);
    window.send_pointer(PointerEvent::new(PointerPhase::Move, p));
    window.send_pointer(PointerEvent::new(PointerPhase::Down, p).with_buttons(button));
    window.send_pointer(PointerEvent::new(PointerPhase::Up, p).with_buttons(Buttons::default()));
    app.frames(2);
}

#[test]
fn the_mouse_back_and_forward_buttons_move_one_entry_each_press() {
    let (mut app, router, _) = app();
    let router = router.borrow().clone().must();
    router.push("/browse");
    router.push("/settings/audio");
    app.frames(1);
    mouse_button(&mut app, Buttons::BACK);
    assert_eq!(router.route_untracked().path(), "/browse");
    mouse_button(&mut app, Buttons::BACK);
    assert_eq!(router.route_untracked().path(), "/home");
    mouse_button(&mut app, Buttons::FORWARD);
    assert_eq!(router.route_untracked().path(), "/browse");
}

#[test]
fn with_spatial_navigation_escape_goes_back_once() {
    let (mut app, router, _) = app();
    let router = router.borrow().clone().must();
    router.push("/a");
    router.push("/b");
    app.frames(1);
    // Off (a desktop's default): Escape is only Escape.
    app.key(KeyEvent::press(Key::Escape));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/b");
    // On, as on a TV: an Escape nothing took is the remote's back, one step per press.
    app.ui().must().set_spatial_navigation(true);
    app.key(KeyEvent::press(Key::Escape));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/a");
    // The remote's own back key goes the same way, once.
    app.key(KeyEvent::press(Key::Back));
    app.frames(1);
    assert_eq!(router.route_untracked().path(), "/home");
    assert_eq!(app.runner().app().windows().len(), 1, "a back the router took keeps the window");
}

#[test]
fn links_carry_their_address_for_screen_readers() {
    let (app, _, _) = app();
    let link = app.node(Role::Link, "Settings").must();
    assert_eq!(link.url.as_deref(), Some("#/settings/audio"));
}

#[test]
fn going_back_restores_the_scroll_position() {
    let (mut app, router, scroller) = app();
    let router = router.borrow().clone().must();
    let ui = app.ui().must();
    let home = scroller.borrow().clone().must();
    // Scrolled as the person would: through the compositor, which reports it back.
    home.scroll_to(Vec2::new(0.0, 120.0));
    app.frames(2);
    assert_eq!(ui.scroll_offset(home.id()), Vec2::new(0.0, 120.0), "the list scrolled");
    router.push("/browse");
    app.frames(2);
    assert_eq!(router.saved_scroll(), Vec2::ZERO, "a new entry starts at the top");
    let browse = scroller.borrow().clone().must();
    assert_eq!(ui.scroll_offset(browse.id()), Vec2::ZERO);
    assert!(router.back());
    app.frames(2);
    assert_eq!(router.saved_scroll(), Vec2::new(0.0, 120.0));
    let rebuilt = scroller.borrow().clone().must();
    assert_ne!(rebuilt.id(), home.id(), "the page was built again");
    // The rebuilt list is drawn where the old one was, though it had no layer when asked.
    assert_eq!(ui.scroll_offset(rebuilt.id()), Vec2::new(0.0, 120.0), "shown at its position");
}

#[test]
fn reload_rebuilds_the_page_at_its_route() {
    let (mut app, router, scroller) = app();
    let router = router.borrow().clone().must();
    router.push("/server");
    app.frames(1);
    let before = scroller.borrow().clone().must().id();
    app.runner().app().reload();
    app.frames(1);
    assert_ne!(scroller.borrow().clone().must().id(), before);
    assert_eq!(router.route_untracked().path(), "/server");
}

#[test]
#[cfg(feature = "desktop")]
fn deep_links_open_their_page() {
    use remedy_ui::host::HostEvent;
    let (mut app, router, _) = app();
    let router = router.borrow().clone().must();
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

/// Presses the mouse on `name`'s link in window `at` (a press with `buttons` and
/// `modifiers`, then the release).
fn press_link(app: &mut TestApp, at: usize, name: &str, buttons: Buttons, modifiers: Modifiers) {
    let window = app.runner().app().windows().get(at).cloned().must();
    let ui = window.ui().must();
    let node = ui.reader_view().into_iter().find(|n| n.role == Role::Link && n.name.as_deref() == Some(name));
    let link = node.and_then(|n| ui.element_by_node_id(n.id)).must();
    let b = link.bounds().must();
    let p = b.center();
    window.send_pointer(PointerEvent::new(PointerPhase::Move, p));
    window.send_pointer(
        PointerEvent::new(PointerPhase::Down, p).with_buttons(buttons).with_modifiers(modifiers),
    );
    window.send_pointer(
        PointerEvent::new(PointerPhase::Up, p).with_buttons(Buttons::default()).with_modifiers(modifiers),
    );
    app.frames(2);
}

const CTRL: Modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };

#[test]
fn without_new_windows_a_ctrl_or_middle_click_navigates_in_place() {
    let (mut app, router, _) = app();
    let router = router.borrow().clone().must();
    press_link(&mut app, 0, "Settings", Buttons::PRIMARY, CTRL);
    assert_eq!(router.route_untracked().path(), "/settings/audio");
    assert_eq!(app.runner().app().windows().len(), 1);
    assert!(router.back());
    app.frames(1);
    press_link(&mut app, 0, "Settings", Buttons::MIDDLE, Modifiers::NONE);
    assert_eq!(router.route_untracked().path(), "/settings/audio");
    assert_eq!(app.runner().app().windows().len(), 1);
}

/// A window with a router that opens links elsewhere in windows of their own.
fn build(ui: &remedy_ui::view::Ui, router: &Router, seen: &Rc<RefCell<Vec<Router>>>) {
    seen.borrow_mut().push(router.clone());
    let (me, ui2) = (router.clone(), ui.clone());
    let view = router.view(ui, move |route| {
        w::column(&ui2).child(w::text(&ui2, format!("Page {}", route.path()))).child(me.link_to(
            &ui2,
            "/settings/audio",
            "Settings",
        ))
    });
    let _ = ui.root().child(view);
}

#[test]
fn with_new_windows_a_ctrl_or_middle_click_opens_the_route_in_a_window() {
    let seen: Rc<RefCell<Vec<Router>>> = Rc::default();
    let s = seen.clone();
    let mut app = TestApp::new(AppConfig::new("Test"), move |app: &App| {
        let s2 = s.clone();
        app.open(WindowSpec::new("Main").size(480.0, 360.0), move |ui, window: &Window| {
            let router = Router::for_window(window, "/home");
            let s3 = s2.clone();
            router.open_windows(WindowSpec::new("Another").size(480.0, 360.0), move |ui, _, r| {
                build(ui, r, &s3)
            });
            build(ui, &router, &s2);
        });
    });
    let main = seen.borrow().first().cloned().must();

    // A plain click still navigates.
    press_link(&mut app, 0, "Settings", Buttons::PRIMARY, Modifiers::NONE);
    assert_eq!(main.route_untracked().path(), "/settings/audio");
    assert!(main.back());
    app.frames(1);

    // Ctrl-click: a second window at the route, this one staying where it was.
    press_link(&mut app, 0, "Settings", Buttons::PRIMARY, CTRL);
    assert_eq!(app.runner().app().windows().len(), 2);
    assert_eq!(main.route_untracked().path(), "/home");
    let second = seen.borrow().get(1).cloned().must();
    assert_eq!(second.route_untracked().path(), "/settings/audio");
    assert!(app.runner().app().windows().get(1).and_then(Window::ui).is_some_and(|ui| {
        ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Page /settings/audio"))
    }));

    // Middle click: another, and the new window's own links open windows too.
    press_link(&mut app, 0, "Settings", Buttons::MIDDLE, Modifiers::NONE);
    assert_eq!(app.runner().app().windows().len(), 3);
    press_link(&mut app, 1, "Settings", Buttons::MIDDLE, Modifiers::NONE);
    assert_eq!(app.runner().app().windows().len(), 4);
    assert_eq!(main.route_untracked().path(), "/home");
}
