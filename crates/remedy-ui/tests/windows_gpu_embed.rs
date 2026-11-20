//! GPU content embedded in a real Windows window: the element asks for the GPU, an `Auto`
//! window moves there at once (not after slow frames), and the content renders on the same
//! device the window draws with, shown straight from its texture, every frame while it
//! animates. One test per process.
//!
//! Optimized builds only, with the feature
//! (`cargo test --release -p remedy-ui --features gpu-embed --test windows_gpu_embed`).
#![cfg(all(windows, feature = "gpu-embed"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::gpu_embed::{EmbedStatus, Gpu, GpuContent, GpuView, Target, gpu_view, wgpu};
use remedy_ui::{App, AppConfig, GeometryStore, RendererKind, Runner, WindowSpec};
use remedy_ui_bridge_windows::WindowsHooks;
use remedy_ui_bridge_winit::winit::event_loop::EventLoop;
use remedy_ui_bridge_winit::winit::platform::windows::EventLoopBuilderExtWindows;
use remedy_ui_bridge_winit::{UserEvent, run_on};
use remedy_ui_render_gpu::GpuContext;

/// Clears to a color that cycles with the frame number, and remembers which device it was
/// given.
struct Pulse {
    device: Rc<Cell<u64>>,
}

impl GpuContent for Pulse {
    fn prepare(&mut self, gpu: &Gpu<'_>) -> Result<(), remedy_ui::gpu_embed::PrepareError> {
        self.device.set(gpu.serial);
        Ok(())
    }

    fn render(&mut self, _gpu: &Gpu<'_>, target: &Target<'_>, encoder: &mut wgpu::CommandEncoder) {
        let t = (target.frame % 60) as f64 / 60.0;
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pulse"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: t, g: 0.3, b: 1.0 - t, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
}

#[derive(Default)]
struct Seen {
    renderer: Option<RendererKind>,
    status: Option<EmbedStatus>,
    rendered: u64,
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "optimized builds only: cargo test --release -p remedy-ui --features gpu-embed --test windows_gpu_embed"
)]
fn an_embed_moves_its_window_to_the_gpu_and_renders_on_its_device() {
    let device = Rc::new(Cell::new(0u64));
    let seen = Rc::new(RefCell::new(Seen::default()));
    let (d2, s2) = (device.clone(), seen.clone());
    let runner = Runner::with_geometry(
        AppConfig::new("RemedyUI GPU embed test"),
        GeometryStore::in_memory(),
        move |app: &App| {
            let view: Rc<RefCell<Option<GpuView>>> = Rc::new(RefCell::new(None));
            let v2 = view.clone();
            let d3 = d2.clone();
            let win =
                app.open(WindowSpec::new("RemedyUI GPU embed test").size(320.0, 240.0), move |ui, _| {
                    let (el, v) = gpu_view(ui, "Pulse", Pulse { device: d3 });
                    v.set_animating(true);
                    ui.root().child(el);
                    *v2.borrow_mut() = Some(v);
                });
            let a = app.clone();
            app.after(Duration::from_millis(2500), move || {
                let mut s = s2.borrow_mut();
                s.renderer = win.renderer();
                if let Some(v) = view.borrow().as_ref() {
                    s.status = Some(v.status());
                    s.rendered = v.stats().rendered;
                }
                a.quit();
            });
        },
    );
    let event_loop = match EventLoop::<UserEvent>::with_user_event().with_any_thread(true).build() {
        Ok(e) => e,
        Err(e) => unreachable!("an event loop: {e}"),
    };
    let result = run_on(event_loop, runner, WindowsHooks::default());
    assert!(result.is_ok(), "{result:?}");
    let s = seen.borrow();
    assert_eq!(s.renderer, Some(RendererKind::Gpu), "the window moved to the GPU for its embed");
    assert_eq!(s.status, Some(EmbedStatus::Gpu), "shown straight from the texture");
    assert!(s.rendered > 30, "a new picture each frame while animating: {}", s.rendered);
    let shared = GpuContext::current_shared().map(|c| c.device_serial());
    assert_eq!(Some(device.get()), shared, "the content rendered on the window's own device");
}
