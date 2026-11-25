//! A real app loop through the host contract: windows, frames, damage-only presents, wheel
//! scrolling, input methods, timers, wakes, the clipboard and closing.

use std::time::Duration;

use remedy_ui_core::host::{App, Host, HostEvent, ImeEvent, Pixels, TextInputState, WindowId, WindowOptions};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::{Color, Point, Rect, Size, Vec2};
use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_engine::paint::{Brush, DisplayList};
use remedy_ui_engine::render_cpu::{CpuCompositor, Frame};
use remedy_ui_platform::bridge_headless::HeadlessHost;

fn stripes(h: f32) -> DisplayList {
    let mut l = DisplayList::new();
    let mut y = 0.0;
    let mut i = 0u8;
    while y < h {
        l.fill_rect(
            Rect::new(0.0, y, 200.0, y + 20.0),
            Brush::Solid(Color::rgb8(i.wrapping_mul(40), 90, 200)),
        );
        y += 20.0;
        i = i.wrapping_add(1);
    }
    l
}

fn tree() -> LayerTree {
    // A sidebar that never changes and a scrolling list beside it.
    let mut side = DisplayList::new();
    side.fill_rect(Rect::new(0.0, 0.0, 100.0, 300.0), Brush::Solid(Color::rgb8(30, 30, 30)));
    let sidebar = Layer::new(LayerId(10), Rect::new(0.0, 0.0, 100.0, 300.0), side);
    let list = Layer::new(LayerId(2), Rect::new(0.0, 0.0, 200.0, 2000.0), stripes(2000.0));
    let scroller = Layer::new(LayerId(1), Rect::new(100.0, 0.0, 300.0, 300.0), DisplayList::new())
        .scrolling(Size::new(200.0, 2000.0))
        .with_child(list);
    LayerTree::new(vec![sidebar, scroller])
}

#[derive(Default)]
struct Demo {
    window: Option<WindowId>,
    compositor: Option<Compositor>,
    cpu: CpuCompositor,
    last_full: Option<Frame>,
    ime: Vec<ImeEvent>,
    timers: usize,
    wakes: usize,
}

impl App for Demo {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        match event {
            HostEvent::Resumed => {
                let opts = {
                    let mut v_ = WindowOptions::default();
                    v_.title = "Demo".into();
                    v_.size = Size::new(300.0, 300.0);
                    v_
                };
                self.window = host.create_window(&opts).ok();
            }
            HostEvent::Resized { window, width, height, scale } => {
                let mut c = Compositor::new(width, height, scale, ScrollPhysics::CLAMPED);
                c.commit(tree());
                self.compositor = Some(c);
                host.request_frame(window);
            }
            HostEvent::Frame { window, at } => {
                let Some(c) = self.compositor.as_mut() else { return };
                let plan = c.frame(at);
                if !plan.damage.is_empty() {
                    let (w, h) = c.size();
                    if let Ok(frame) = self.cpu.present(&plan, w, h, Color::WHITE) {
                        let pixels = Pixels { width: w, height: h, data: &frame.pixels };
                        let _ = host.present_pixels(window, pixels, plan.damage.rects());
                        self.last_full = Some(frame);
                    }
                }
                if plan.animating {
                    host.request_frame(window);
                }
            }
            HostEvent::Wheel(window, e) => {
                if let Some(c) = self.compositor.as_mut() {
                    let s = c.scale();
                    c.scroll_at(
                        Point::new(e.position.x * s, e.position.y * s),
                        Vec2::new(e.delta.x * s, e.delta.y * s),
                    );
                    host.request_frame(window);
                }
            }
            HostEvent::Ime(_, e) => self.ime.push(e),
            HostEvent::Timer(_) => self.timers += 1,
            HostEvent::Wake => self.wakes += 1,
            HostEvent::CloseRequested(window) => host.close_window(window),
            HostEvent::Destroyed(_) => host.exit(),
            _ => {}
        }
    }
}

fn start(scale: f32) -> (HeadlessHost, Demo, WindowId) {
    let mut host = HeadlessHost::new().with_scale(scale);
    let mut app = Demo::default();
    host.run_frames(&mut app, 1);
    let Some(id) = app.window else { unreachable!("the window opens") };
    (host, app, id)
}

fn presents(host: &HeadlessHost, id: WindowId) -> usize {
    host.window(id).map_or(0, |w| w.presents())
}

#[test]
fn the_first_frame_presents_and_an_unchanged_one_does_not() {
    let (mut host, mut app, id) = start(1.0);
    assert_eq!(presents(&host, id), 1);
    assert_eq!(host.window(id).map(|w| w.size()), Some((300, 300, 1.0)));
    host.request_frame(id);
    host.run_frames(&mut app, 5);
    assert_eq!(presents(&host, id), 1, "nothing changed, nothing presented");
}

#[test]
fn scrolling_presents_only_the_scroller_and_the_surface_stays_exact() {
    for scale in [1.0, 2.0] {
        let (mut host, mut app, id) = start(scale);
        host.wheel(id, Point::new(200.0, 150.0), Vec2::new(0.0, 130.0));
        host.run_frames(&mut app, 1);
        assert_eq!(presents(&host, id), 2);
        let damage = host.window(id).map(|w| w.last_damage().to_vec()).unwrap_or_default();
        assert_eq!(
            damage,
            vec![Rect::new(100.0 * scale, 0.0, 300.0 * scale, 300.0 * scale)],
            "the sidebar is not redrawn"
        );
        let surface = host.window(id).map(|w| w.surface().to_vec());
        assert_eq!(
            surface,
            app.last_full.as_ref().map(|f| f.pixels.clone()),
            "damage covered every change at {scale}x"
        );
        // The wheel over the sidebar scrolls nothing.
        host.wheel(id, Point::new(50.0, 150.0), Vec2::new(0.0, 50.0));
        host.run_frames(&mut app, 1);
        assert_eq!(presents(&host, id), 2);
    }
}

#[test]
fn input_methods_are_scripted_through_the_session() {
    let (mut host, mut app, id) = start(1.0);
    let state = {
        let mut v_ = TextInputState::default();
        v_.surrounding = "Hello ".into();
        v_.selection = 6..6;
        v_.caret = Rect::new(40.0, 10.0, 41.0, 28.0);
        v_.private = false;
        v_.multiline = false;
        v_
    };
    host.set_text_input(id, Some(state.clone()));
    assert_eq!(host.window(id).and_then(|w| w.text_input()), Some(&state));
    host.compose_and_commit(id, "にほん", "日本");
    host.turn(&mut app);
    assert_eq!(app.ime.first(), Some(&ImeEvent::Enabled));
    assert!(app.ime.contains(&ImeEvent::Preedit { text: "にほん".into(), cursor: Some(9..9) }));
    assert!(app.ime.contains(&ImeEvent::Commit("日本".into())));
    assert_eq!(app.ime.last(), Some(&ImeEvent::Disabled));
    host.set_text_input(id, None);
    assert!(host.window(id).is_some_and(|w| w.text_input().is_none()));
}

#[test]
fn timers_fire_when_due_and_wakes_cross_threads() {
    let (mut host, mut app, _) = start(1.0);
    let at = host.now() + Duration::from_millis(100);
    host.set_timer(at);
    let cancelled = host.set_timer(at);
    host.cancel_timer(cancelled);
    host.advance(Duration::from_millis(50));
    host.turn(&mut app);
    assert_eq!(app.timers, 0);
    host.advance(Duration::from_millis(60));
    host.turn(&mut app);
    assert_eq!(app.timers, 1, "due once; the cancelled one never fires");
    let waker = host.waker();
    let t = std::thread::spawn(move || waker());
    assert!(t.join().is_ok());
    host.turn(&mut app);
    assert_eq!(app.wakes, 1);
}

#[test]
fn clipboard_window_state_and_closing() {
    let (mut host, mut app, id) = start(1.0);
    assert!(host.set_clipboard_text("secret", true).is_ok());
    assert_eq!(host.clipboard(), Some(("secret", true)));
    assert_eq!(host.clipboard_text().ok().flatten().as_deref(), Some("secret"));
    host.set_title(id, "Renamed");
    assert!(host.set_capture_protected(id, true).is_ok());
    assert!(host.window(id).is_some_and(|w| w.options().title == "Renamed" && w.options().capture_protected));
    assert!(host.window_handles(id).is_err(), "no GPU surfaces, said plainly");
    host.send(HostEvent::CloseRequested(id));
    host.turn(&mut app);
    assert!(host.window(id).is_none());
    assert!(host.has_exited());
}

#[test]
fn resizing_rebuilds_and_redraws_everything() {
    let (mut host, mut app, id) = start(1.0);
    host.user_resize(id, Size::new(320.0, 240.0));
    host.run_frames(&mut app, 1);
    assert_eq!(host.window(id).map(|w| w.size()), Some((320, 240, 1.0)));
    assert_eq!(presents(&host, id), 2);
    assert_eq!(
        host.window(id).map(|w| w.last_damage().to_vec()),
        Some(vec![Rect::new(0.0, 0.0, 320.0, 240.0)])
    );
}

#[test]
fn tray_menus_checks_submenus_and_urgency_are_recorded() {
    use remedy_ui_core::host::{Notification, TrayIcon, TrayImage, TrayItem, Urgency};
    let mut host = HeadlessHost::new();
    let menu = vec![
        TrayItem::Item { id: 1, label: "Open".into(), enabled: true, default: true },
        TrayItem::check(2, "Start with the session", true),
        TrayItem::submenu("Server", vec![TrayItem::item(3, "Restart"), TrayItem::Separator]),
    ];
    let tray = TrayIcon::new("App", TrayImage { size: 1, rgba: vec![255; 4] }, menu.clone());
    assert!(host.set_tray(Some(&tray)).is_ok());
    let shown = host.tray().map(|t| t.menu.clone());
    assert_eq!(shown.as_deref(), Some(menu.as_slice()));
    assert_eq!(menu.get(1).and_then(TrayItem::chosen_id), Some(2));
    assert_eq!(menu.get(2).and_then(TrayItem::chosen_id), None, "a submenu is not chosen itself");
    assert!(
        host.show_notification(&Notification::new(9, "Disk almost full", "").urgency(Urgency::Warning))
            .is_ok()
    );
    assert_eq!(host.notifications().last().map(|n| n.urgency), Some(Urgency::Warning));
    assert_eq!(Notification::new(1, "", "").urgency, Urgency::Info, "news by default");
}

#[test]
fn a_window_closed_by_an_earlier_frame_gets_no_frame() {
    /// Closes the second window in the first one's frame, and notes every frame.
    struct Closer(WindowId, Vec<WindowId>);
    impl App for Closer {
        fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
            if let HostEvent::Frame { window, .. } = event {
                self.1.push(window);
                host.close_window(self.0);
            }
        }
    }
    let mut host = HeadlessHost::new();
    let (Ok(a), Ok(b)) =
        (host.create_window(&WindowOptions::default()), host.create_window(&WindowOptions::default()))
    else {
        unreachable!("headless windows open")
    };
    host.request_frame(a);
    host.request_frame(b);
    let mut app = Closer(b, Vec::new());
    host.turn(&mut app);
    assert_eq!(app.1, vec![a], "the closed window is not drawn");
}
