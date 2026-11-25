//! A mistake in the app's GPU work is caught and reported; the last good picture stays, and
//! the next good render recovers.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::*;
use remedy_ui_platform::embed_gpu::*;

/// Clears to blue; while `broken`, also writes past the end of a buffer (a validation
/// error) after clearing to red.
struct Clumsy {
    broken: Rc<Cell<bool>>,
    small: Option<wgpu::Buffer>,
}

impl GpuContent for Clumsy {
    fn prepare(&mut self, gpu: &Gpu<'_>) -> Result<(), remedy_ui_platform::embed_gpu::PrepareError> {
        self.small = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("small"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        Ok(())
    }

    fn render(&mut self, gpu: &Gpu<'_>, target: &Target<'_>, encoder: &mut wgpu::CommandEncoder) {
        let color = if self.broken.get() { wgpu::Color::RED } else { wgpu::Color::BLUE };
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(color), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        }));
        if self.broken.get()
            && let Some(b) = &self.small
        {
            gpu.queue.write_buffer(b, 0, &[0u8; 64]);
        }
    }
}

#[test]
fn a_broken_render_is_reported_and_the_last_good_picture_stays() {
    let ui = ui();
    let broken = Rc::new(Cell::new(false));
    let (el, view) = gpu_view(&ui, "Clumsy", Clumsy { broken: broken.clone(), small: None });
    ui.root().child(el);
    let mut s = Screen::new(ui, true);
    s.frame();
    assert!(near(s.px(50, 50), BLUE), "{:?}", s.px(50, 50));
    assert_eq!(view.status(), EmbedStatus::Gpu);
    broken.set(true);
    view.request_redraw();
    s.frame();
    match view.status() {
        EmbedStatus::Failed(e) => assert!(e.starts_with("rendering failed"), "{e}"),
        other => unreachable!("a reported failure: {other:?}"),
    }
    assert!(near(s.px(50, 50), BLUE), "the last good picture stays: {:?}", s.px(50, 50));
    assert_eq!(view.stats().rendered, 1);
    broken.set(false);
    view.request_redraw();
    s.frame();
    assert_eq!(view.status(), EmbedStatus::Gpu, "recovered");
    assert_eq!(view.stats().rendered, 2);
}
