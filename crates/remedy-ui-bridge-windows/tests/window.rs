//! A real Windows window through the whole host contract. One test, because a process may
//! create only one event loop. It restores the user's clipboard text afterwards.
#![cfg(windows)]

use std::time::Duration;

use remedy_ui_bridge_windows::WindowsHooks;
use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_bridge_winit::{UserEvent, run_on};
use remedy_ui_core::{Rect, Size};
use remedy_ui_host::{App, Host, HostEvent, Pixels, SystemSettings, TextInputState, WindowId, WindowOptions};

#[derive(Default)]
struct Probe {
    window: Option<WindowId>,
    settings: Option<SystemSettings>,
    presents: Vec<Result<(), String>>,
    timer_fired: bool,
    woke: bool,
    clipboard: Option<(Option<String>, Option<String>)>,
    capture: Option<Result<(), String>>,
    handles: bool,
    text_input: bool,
    destroyed: bool,
    deadline_hit: bool,
}

impl App for Probe {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        match event {
            HostEvent::Settings(s) => self.settings = Some(s),
            HostEvent::Resumed => {
                let deadline = host.now() + Duration::from_secs(20);
                host.set_timer(deadline);
                let opts = WindowOptions {
                    title: "RemedyUI bridge test".into(),
                    size: Size::new(200.0, 150.0),
                    ..Default::default()
                };
                self.window = host.create_window(&opts).ok();
            }
            HostEvent::Resized { window, .. } => host.request_frame(window),
            HostEvent::Frame { window, .. } => {
                let Some((w, h, _)) = host.window_size(window) else { return };
                let n = self.presents.len();
                let shade = [200u8, 40, 40, 255];
                let pixels: Vec<u8> =
                    shade.iter().copied().cycle().take(w as usize * h as usize * 4).collect();
                let damage = if n == 0 {
                    Rect::new(0.0, 0.0, w as f32, h as f32)
                } else {
                    Rect::new(0.0, 0.0, 10.0, 10.0)
                };
                let r = host.present_pixels(window, Pixels { width: w, height: h, data: &pixels }, &[damage]);
                self.presents.push(r.map_err(|e| e.to_string()));
                if self.presents.len() < 3 {
                    host.request_frame(window);
                } else {
                    // Then a timer and a wake from another thread.
                    let soon = host.now() + Duration::from_millis(30);
                    host.set_timer(soon);
                }
            }
            HostEvent::Timer(_) if !self.timer_fired && !self.presents.is_empty() => {
                self.timer_fired = true;
                let waker = host.waker();
                std::thread::spawn(move || waker());
            }
            HostEvent::Timer(_) => {
                self.deadline_hit = true;
                host.exit();
            }
            HostEvent::Wake => {
                self.woke = true;
                let Some(window) = self.window else { return };
                let saved = host.clipboard_text().ok().flatten();
                let wrote = host.set_clipboard_text("RemedyUI clipboard check", true);
                let read = host.clipboard_text().ok().flatten();
                if let Some(text) = &saved {
                    let _ = host.set_clipboard_text(text, false);
                }
                self.clipboard = Some((wrote.ok().map(|()| "written".to_owned()), read));
                self.capture = Some(host.set_capture_protected(window, true).map_err(|e| e.to_string()));
                self.handles = host.window_handles(window).is_ok();
                // A text session through TSF, a password field (no input method), then none.
                let field = TextInputState {
                    surrounding: "Hello".into(),
                    selection: 5..5,
                    caret: Rect::new(40.0, 4.0, 41.0, 22.0),
                    private: false,
                    multiline: false,
                };
                host.set_text_input(window, Some(field.clone()));
                host.set_text_input(window, Some(TextInputState { private: true, ..field }));
                host.set_text_input(window, None);
                self.text_input = true;
                host.close_window(window);
            }
            HostEvent::Destroyed(_) => {
                self.destroyed = true;
                host.exit();
            }
            _ => {}
        }
    }
}

#[test]
fn a_real_window_runs_the_whole_host_contract() {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
        Ok(e) => e,
        Err(e) => unreachable!("an event loop: {e}"),
    };
    let mut probe = Probe::default();
    let result = run_on(event_loop, &mut probe, WindowsHooks::default());
    assert!(result.is_ok(), "{result:?}");
    assert!(!probe.deadline_hit, "the scenario finished in time");
    let s = probe.settings.unwrap_or_default();
    assert!(!s.locale.is_empty() && s.double_click > Duration::ZERO, "settings come from Windows: {s:?}");
    assert_eq!(probe.presents.len(), 3);
    assert!(probe.presents.iter().all(Result::is_ok), "{:?}", probe.presents);
    assert!(probe.timer_fired && probe.woke);
    assert_eq!(
        probe.clipboard,
        Some((Some("written".to_owned()), Some("RemedyUI clipboard check".to_owned()))),
        "the sensitive copy reads back"
    );
    assert_eq!(probe.capture, Some(Ok(())));
    assert!(probe.handles, "native handles for GPU surfaces");
    assert!(probe.text_input, "text sessions open and close through TSF");
    assert!(probe.destroyed);
}
