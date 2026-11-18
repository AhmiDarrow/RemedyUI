//! Textures the app renders on RemedyUI's own device (an embedded 3D view), and the device
//! itself, shared by the process's windows and the app.
//!
//! The app renders into a texture it registers with [`GpuContext::register_texture`]; display
//! lists then place it with an `Item::Texture` naming the registration id, and the renderer
//! samples it like any image. Registrations belong to the device: a replacement device after
//! a loss starts with none, and the app registers its new textures there.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::block::block_on;
use crate::renderer::FORMAT;
use crate::{GpuContext, GpuError, LadderOptions};

/// A device's registered textures, by registration id.
#[derive(Clone, Debug, Default)]
pub(crate) struct Registry(Arc<Mutex<HashMap<u64, Registered>>>);

#[derive(Clone, Debug)]
pub(crate) struct Registered {
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Registry {
    pub(crate) fn get(&self, id: u64) -> Option<Registered> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).get(&id).cloned()
    }

    pub(crate) fn contains(&self, id: u64) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).contains_key(&id)
    }
}

/// The device RemedyUI draws with in this process, once one is made. It lives until the
/// process ends, as the GPU device of an app does (and a static is never torn down while
/// wgpu's own thread-local state is going away).
static SHARED: Mutex<Option<GpuContext>> = Mutex::new(None);

impl GpuContext {
    /// Registers `texture` so display lists can place it (`Item::Texture` with the returned
    /// id). It must be a 2D premultiplied RGBA8 texture ([`FORMAT`](crate::FORMAT)) made on
    /// this device with `TEXTURE_BINDING` usage; anything else is refused, since the renderer
    /// samples it as such. Each registration gets a new id.
    pub fn register_texture(&self, texture: &wgpu::Texture) -> Result<u64, GpuError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if texture.format() != FORMAT
            || texture.dimension() != wgpu::TextureDimension::D2
            || !texture.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)
            || texture.sample_count() != 1
        {
            return Err(GpuError::Render(format!(
                "an embedded texture must be a single-sampled 2D {FORMAT:?} texture with TEXTURE_BINDING usage"
            )));
        }
        if self.is_lost() {
            return Err(GpuError::Lost);
        }
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let entry = Registered { view, width: texture.width(), height: texture.height() };
        self.textures.0.lock().unwrap_or_else(PoisonError::into_inner).insert(id, entry);
        Ok(id)
    }

    /// Forgets a registration: items naming `id` draw nothing from now on, and the renderer
    /// lets go of the texture.
    pub fn unregister_texture(&self, id: u64) {
        self.textures.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
    }

    /// Runs `f` (the app's own GPU work) and returns what it returned with the first GPU
    /// validation or out-of-memory error it caused, so a mistake in app rendering is reported
    /// to the app instead of being lost among the device's errors.
    pub fn catch_errors<T>(&self, f: impl FnOnce() -> T) -> (T, Option<String>) {
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let out = f();
        let first = block_on(validation.pop());
        let second = block_on(oom.pop());
        (out, first.or(second).map(|e| e.to_string()))
    }

    /// The device RemedyUI draws with in this process: the one windows render with and app
    /// content embedded in them shares. Made from `ladder` on first use, and made again when
    /// the current one is lost, so every user of it moves to the replacement together.
    pub fn shared(ladder: &LadderOptions) -> Result<GpuContext, GpuError> {
        let mut slot = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ctx) = slot.as_ref().filter(|c| !c.is_lost()) {
            return Ok(ctx.clone());
        }
        let ctx = GpuContext::new(ladder)?;
        *slot = Some(ctx.clone());
        Ok(ctx)
    }

    /// The shared device, if one was made (it may since have been lost).
    pub fn current_shared() -> Option<GpuContext> {
        SHARED.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// A number that differs between devices: content on the GPU compares it to know its
    /// resources belong to a device that has been replaced.
    pub fn device_serial(&self) -> u64 {
        self.serial
    }
}
