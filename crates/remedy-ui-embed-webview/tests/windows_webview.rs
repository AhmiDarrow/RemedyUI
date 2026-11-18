//! The web view scenario on Windows, with WebView2. One test per process (one event loop
//! per process); needs the WebView2 runtime (Windows 11 has it).
#![cfg(windows)]

mod scenario;

use remedy_ui_bridge_windows::WindowsHooks;
use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_bridge_winit::{UserEvent, run_on};

#[test]
fn a_web_view_loads_talks_blocks_and_follows_its_element() {
    scenario::scenario(|runner| {
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_any_thread(true)
            .build()
            .map_err(|e| e.to_string())?;
        run_on(event_loop, runner, WindowsHooks::default()).map_err(|e| e.to_string())
    });
}
