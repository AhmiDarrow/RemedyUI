//! The player full screen on a real window: the mouse pointer hides with its resting
//! controls and comes back when the mouse moves. On Windows the pointer is read with
//! `GetCursorInfo`; on Linux on X11 (XWayland under WSLg too: unset `WAYLAND_DISPLAY`) with
//! XFixes' cursor image, moved with XTest. It takes the screen and moves the mouse for a few
//! seconds (putting it back after), so it runs on request:
//! `cargo test -p remedy-ui --test real_cursor -- --ignored`.
#![cfg(any(windows, target_os = "linux"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui::playback::{
    AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Player, PlayerOptions,
    Sample, VideoInfo, player,
};
use remedy_ui::{App, AppConfig, GeometryStore, Runner, WindowSpec};
use remedy_ui_bridge_winit::run_on;

#[cfg(windows)]
mod os {
    pub use remedy_ui_bridge_windows::pointer::{click, move_to, position, screen_size, showing};
    use remedy_ui_bridge_winit::UserEvent;
    use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
    use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;

    pub fn event_loop() -> EventLoop<UserEvent> {
        match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
            Ok(e) => e,
            Err(e) => unreachable!("an event loop: {e}"),
        }
    }

    pub fn hooks() -> remedy_ui_bridge_windows::WindowsHooks {
        remedy_ui_bridge_windows::WindowsHooks::default()
    }
}

#[cfg(target_os = "linux")]
mod os {
    use remedy_ui_bridge_winit::UserEvent;
    use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
    use remedy_ui_bridge_winit::winit::platform::x11::EventLoopBuilderExtX11;
    use x11rb::connection::Connection;
    use x11rb::protocol::xfixes::ConnectionExt as _;
    use x11rb::protocol::xproto::{
        BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, MOTION_NOTIFY_EVENT,
    };
    use x11rb::protocol::xtest::ConnectionExt as _;

    fn x() -> (x11rb::rust_connection::RustConnection, u32, (i32, i32)) {
        let (c, n) = x11rb::connect(None).unwrap_or_else(|e| unreachable!("an X server: {e}"));
        let s = &c.setup().roots[n];
        let (root, size) = (s.root, (i32::from(s.width_in_pixels), i32::from(s.height_in_pixels)));
        (c, root, size)
    }

    /// Whether the pointer's image has any pixel that shows (a hidden pointer is blank).
    pub fn showing() -> bool {
        let (c, _, _) = x();
        let _ = c.xfixes_query_version(5, 0).ok().and_then(|r| r.reply().ok());
        let image = c.xfixes_get_cursor_image().ok().and_then(|r| r.reply().ok());
        image.is_some_and(|i| i.cursor_image.iter().any(|p| p >> 24 != 0))
    }

    pub fn position() -> (i32, i32) {
        let (c, root, _) = x();
        let at = c.query_pointer(root).ok().and_then(|r| r.reply().ok());
        at.map_or((0, 0), |p| (i32::from(p.root_x), i32::from(p.root_y)))
    }

    pub fn move_to(x_: i32, y: i32) {
        let (c, root, _) = x();
        let (x16, y16) = (i16::try_from(x_).unwrap_or(0), i16::try_from(y).unwrap_or(0));
        let _ = c.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x16, y16, 0);
        let _ = c.flush();
        // A round trip, so the motion has reached the server.
        let _ = c.get_input_focus().ok().and_then(|r| r.reply().ok());
    }

    /// A left click where the pointer is.
    pub fn click() {
        let (c, root, _) = x();
        for kind in [BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT] {
            let _ = c.xtest_fake_input(kind, 1, 0, root, 0, 0, 0);
        }
        let _ = c.flush();
        let _ = c.get_input_focus().ok().and_then(|r| r.reply().ok());
    }

    pub fn screen_size() -> (i32, i32) {
        x().2
    }

    pub fn event_loop() -> EventLoop<UserEvent> {
        match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
            Ok(e) => e,
            Err(e) => unreachable!("an event loop: {e}"),
        }
    }

    pub fn hooks() -> remedy_ui_bridge_linux::LinuxHooks {
        remedy_ui_bridge_linux::LinuxHooks::default()
    }
}

use os as pointer;

const RATE: u32 = 8000;

/// A minute of silence in tenths of a second, so the player plays by the clock.
struct Silence {
    info: MediaInfo,
    next: u32,
}

impl Decoder for Silence {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        if self.next == 600 {
            return Ok(None);
        }
        let at = Duration::from_millis(u64::from(self.next) * 100);
        self.next += 1;
        Ok(Some(Sample::Audio { at, samples: vec![0.0; (RATE / 10) as usize] }))
    }
    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.next = u32::try_from(to.as_millis() / 100).unwrap_or(600).min(600);
        Ok(())
    }
}

struct SilenceHost;

impl MediaHost for SilenceHost {
    fn name(&self) -> &str {
        "silence"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let mut info = MediaInfo::default();
        info.duration = Some(Duration::from_secs(60));
        info.seekable = true;
        info.audio = Some(AudioFormat::new(RATE, 1));
        // A picture, so the stage fills the screen (no frames come: it stays black).
        info.video = Some(VideoInfo::new(640, 360, None));
        Ok(Box::new(Silence { info, next: 0 }))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("no sound device here".into()))
    }
}

#[test]
#[ignore = "takes the screen and moves the mouse for a few seconds"]
fn full_screen_hides_the_pointer_with_the_resting_controls() {
    let was = pointer::position();
    let (w, h) = pointer::screen_size();
    let (cx, cy) = (w / 2, h / 2);
    // (pointer showing, controls shown): moved over the picture, rested, moved again.
    let seen: Rc<RefCell<Vec<(bool, bool)>>> = Rc::default();
    let s = seen.clone();
    let runner = Runner::with_geometry(
        AppConfig::new("RemedyUI cursor test").cpu_only(),
        GeometryStore::in_memory(),
        move |app: &App| {
            let s = s.clone();
            let app2 = app.clone();
            let _ = app.open(WindowSpec::new("Player").size(480.0, 320.0), move |ui, window| {
                let p = Player::open(
                    ui,
                    Arc::new(SilenceHost),
                    MediaSource::url("silence:"),
                    PlayerOptions::default(),
                );
                let w = window.clone();
                p.on_fullscreen(move |on| w.set_fullscreen(on));
                // Over whatever window has the screen, so the pointer is over this one.
                window.set_always_on_top(true);
                window.focus();
                ui.root().child(player(ui, &p, "Silence"));
                p.play();
                p.request_fullscreen(true);
                // Steps checked every 100 ms: frames of a full screen drawn on the CPU can
                // hold timers back, so nothing waits on a fixed time.
                let step = Rc::new(std::cell::Cell::new(0u32));
                let since = Rc::new(std::cell::Cell::new(0u32));
                fn tick(
                    app: App,
                    p: Player,
                    s: Rc<RefCell<Vec<(bool, bool)>>>,
                    step: Rc<std::cell::Cell<u32>>,
                    since: Rc<std::cell::Cell<u32>>,
                    at: (i32, i32),
                    n: u32,
                ) {
                    let note = || s.borrow_mut().push((pointer::showing(), p.controls_shown()));
                    since.set(since.get() + 1);
                    let playing = p.state() == remedy_ui::playback::PlaybackState::Playing;
                    match step.get() {
                        // Full screen and playing: the pointer comes over the picture.
                        0 if p.is_fullscreen() && playing && since.get() > 8 => {
                            pointer::move_to(at.0, at.1);
                            // A click brings the window to the front, as a person would: Windows
                            // hides the pointer only over the window in front.
                            pointer::click();
                            step.set(1);
                            since.set(0);
                        }
                        // Until the window has the pointer (a full-screen switch can take a
                        // moment): the controls show for movement over it.
                        1 if since.get() > 2 && p.controls_shown() => {
                            note();
                            step.set(2);
                            since.set(0);
                        }
                        1 => {
                            let d = 4 * i32::from(since.get() % 2 == 1);
                            pointer::move_to(at.0 + d, at.1 + d);
                        }
                        // Rested: a moment for the platform, then a move brings it all back.
                        2 if !p.controls_shown() => {
                            step.set(3);
                            since.set(0);
                        }
                        3 if since.get() == 2 => {
                            note();
                            pointer::move_to(at.0 + 40, at.1 + 40);
                        }
                        // Checked as soon as they show: a full screen drawn on the CPU can take
                        // long enough between ticks for them to rest again.
                        3 if since.get() > 2 && p.controls_shown() => {
                            note();
                            app.quit();
                            return;
                        }
                        _ => {}
                    }
                    if n > 150 {
                        app.quit();
                        return;
                    }
                    let a = app.clone();
                    app.after(Duration::from_millis(100), move || tick(a, p, s, step, since, at, n + 1));
                }
                let (a, s) = (app2.clone(), s.clone());
                app2.after(Duration::from_millis(100), move || tick(a, p, s, step, since, (cx, cy), 0));
            });
        },
    );
    let result = run_on(os::event_loop(), runner, os::hooks());
    pointer::move_to(was.0, was.1);
    assert!(result.is_ok(), "{result:?}");
    let seen = seen.borrow().clone();
    assert_eq!(seen, [(true, true), (false, false), (true, true)], "(pointer showing, controls shown)");
}
