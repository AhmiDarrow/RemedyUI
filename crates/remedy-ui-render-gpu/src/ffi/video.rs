//! Decoded pictures that are already on the GPU ([`remedy_ui_paint::GpuPicture`]), opened on
//! the renderer's device where the platform shares them with it. Where it does not (yet), a
//! picture is read back and uploaded like any other frame.

use remedy_ui_paint::GpuFrame;

/// A stream's pictures opened on this device, and those the GPU is still reading.
#[derive(Default)]
pub(crate) struct Shared(());

impl Shared {
    /// `picture`'s texture opened on `device`; `None` when this device cannot open it.
    pub(crate) fn open(&mut self, device: &wgpu::Device, picture: &GpuFrame) -> Option<wgpu::Texture> {
        let _ = (device, picture);
        None
    }

    /// Submits `commands`, which read `texture` (from [`Shared::open`]).
    pub(crate) fn submit(
        &mut self,
        queue: &wgpu::Queue,
        commands: wgpu::CommandBuffer,
        picture: &GpuFrame,
        texture: wgpu::Texture,
    ) {
        let _ = (picture, texture);
        queue.submit([commands]);
    }
}
