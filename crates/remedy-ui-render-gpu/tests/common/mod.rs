//! What the tests that present to a real window share: the event loop (Windows, or X11 on
//! Linux, where Xvfb is enough) and the validation layer's reports.

#[cfg(target_os = "linux")]
pub use remedy_ui_bridge_linux::LinuxHooks as Hooks;
#[cfg(windows)]
pub use remedy_ui_bridge_windows::WindowsHooks as Hooks;
use remedy_ui_bridge_winit::UserEvent;
use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
#[cfg(windows)]
use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
#[cfg(target_os = "linux")]
use remedy_ui_bridge_winit::winit::platform::x11::EventLoopBuilderExtX11;

/// An event loop for this test thread, with the validation layer's reports collected from
/// now on. `None` on Linux without an X11 display or a Vulkan device (said on stderr): the
/// test has nothing to present with and passes.
pub fn event_loop() -> Option<EventLoop<UserEvent>> {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("no X11 display (run under xvfb-run): nothing presented");
            return None;
        }
        builder.with_x11();
        // What these tests check is presenting through RemedyUI's own Vulkan swapchain;
        // wgpu's GL surface presents nothing under Xvfb.
        match remedy_ui_render_gpu::GpuContext::new(&remedy_ui_render_gpu::LadderOptions::default()) {
            Ok(ctx) if ctx.adapter.get_info().backend == wgpu::Backend::Vulkan => {}
            Ok(ctx) => {
                eprintln!("no Vulkan device ({}): nothing presented", ctx.describe());
                return None;
            }
            Err(e) => {
                eprintln!("no GPU ({e}): nothing presented");
                return None;
            }
        }
    }
    if log::set_logger(&VALIDATION).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    match builder.with_any_thread(true).build() {
        Ok(e) => Some(e),
        Err(e) => unreachable!("an event loop: {e}"),
    }
}

/// What the Vulkan validation layer reported through wgpu so far: errors, and
/// synchronization hazards. Empty without the layer; it is turned on with
/// `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`, and synchronization validation with
/// `VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true`.
pub fn validation() -> Vec<String> {
    VALIDATION.0.lock().map(|v| v.clone()).unwrap_or_default()
}

struct Validation(std::sync::Mutex<Vec<String>>);

static VALIDATION: Validation = Validation(std::sync::Mutex::new(Vec::new()));

impl log::Log for Validation {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Warn && m.target().starts_with("wgpu_hal")
    }

    fn log(&self, r: &log::Record<'_>) {
        let text = r.args().to_string();
        // The layer's errors, and its synchronization hazards (warnings in some versions);
        // wgpu starts each message with its type (the loader's are GENERAL).
        if self.enabled(r.metadata())
            && text.starts_with("VALIDATION")
            && (r.level() == log::Level::Error || text.contains("SYNC-"))
            && let Ok(mut v) = self.0.lock()
        {
            v.push(text);
        }
    }

    fn flush(&self) {}
}
