//! The web view scenario on Linux, with WebKitGTK under X11. One test per process. It needs
//! an X11 display (or XWayland) and WebKitGTK 4.1 or 4.0, so it runs only when asked:
//! `cargo test -p remedy-ui-embed-webview --test linux_webview -- --ignored`.
#![cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]

mod scenario;

use remedy_ui_platform::bridge_linux::LinuxHooks;
use remedy_ui_platform::bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_platform::bridge_winit::winit::platform::x11::EventLoopBuilderExtX11;
use remedy_ui_platform::bridge_winit::{UserEvent, run_on};

#[test]
#[ignore = "needs an X11 display and WebKitGTK"]
fn a_web_view_loads_talks_blocks_and_follows_its_element() {
    scenario::scenario(|runner| {
        // X11 even where a Wayland compositor runs (its XWayland): WebKitGTK can only be
        // placed inside an X11 window.
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_x11()
            .with_any_thread(true)
            .build()
            .map_err(|e| e.to_string())?;
        run_on(event_loop, runner, LinuxHooks::default()).map_err(|e| e.to_string())
    });
}
