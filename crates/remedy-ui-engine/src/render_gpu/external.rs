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

use crate::render_gpu::renderer::FORMAT;
use crate::render_gpu::{GpuContext, GpuError, LadderOptions};

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

/// Whether the process's shared device was made (and so is alive at exit, by design).
#[cfg(windows)]
pub(crate) fn shared_made() -> bool {
    SHARED.try_lock().map_or(true, |s| s.is_some())
}

impl GpuContext {
    /// Registers `texture` so display lists can place it (`Item::Texture` with the returned
    /// id). It must be a 2D premultiplied RGBA8 texture ([`FORMAT`](crate::render_gpu::FORMAT)) made on
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
    /// to the app instead of being lost among the device's errors. A browser answers error
    /// scopes only after returning to the event loop: there the error is returned when the
    /// browser already knows it, and otherwise arrives later in [`GpuContext::take_errors`].
    pub fn catch_errors<T>(&self, f: impl FnOnce() -> T) -> (T, Option<String>) {
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let out = f();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let first = crate::render_gpu::block::block_on(validation.pop());
            let second = crate::render_gpu::block::block_on(oom.pop());
            (out, first.or(second).map(|e| e.to_string()))
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let (validation, oom) = (validation.pop(), oom.pop());
            let ctx = self.clone();
            let mut both = Box::pin(async move { (validation.await, oom.await) });
            let waker = std::task::Waker::noop();
            let mut cx = std::task::Context::from_waker(waker);
            match both.as_mut().poll(&mut cx) {
                std::task::Poll::Ready((a, b)) => (out, a.or(b).map(|e| e.to_string())),
                std::task::Poll::Pending => {
                    wasm_bindgen_futures::spawn_local(async move {
                        let (a, b) = both.await;
                        if let Some(e) = a.or(b) {
                            ctx.record_error(e.to_string());
                        }
                    });
                    (out, None)
                }
            }
        }
    }

    /// The device RemedyUI draws with in this process: the one windows render with and app
    /// content embedded in them shares. Made from `ladder` on first use, and made again when
    /// the current one is lost, so every user of it moves to the replacement together. In a
    /// browser it is made asynchronously by [`GpuContext::prepare_shared`]; until then this
    /// reports [`GpuError::Pending`] (while it is being made) or why it could not be made.
    pub fn shared(ladder: &LadderOptions) -> Result<GpuContext, GpuError> {
        let slot = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ctx) = slot.as_ref().filter(|c| !c.is_lost()) {
            return Ok(ctx.clone());
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = ladder;
            drop(slot);
            web::state()
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut slot = slot;
            let ctx = GpuContext::new(ladder)?;
            *slot = Some(ctx.clone());
            Ok(ctx)
        }
    }

    /// Starts making the shared device for `window` without blocking (how a browser makes
    /// one), unless it exists or is already being made; `ready` runs on this thread once it
    /// is made or has failed ([`GpuContext::shared`] then answers). Natively the device is
    /// made here, before `ready` runs.
    pub fn prepare_shared(
        ladder: &LadderOptions,
        window: Arc<dyn remedy_ui_core::host::WindowHandles>,
        ready: impl FnOnce(Result<(), GpuError>) + 'static,
    ) {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        web::prepare(ladder.clone(), window, Box::new(ready));
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let _ = window;
            ready(Self::shared(ladder).map(drop));
        }
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

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web {
    //! Making the shared device in a browser, where adapters and devices come from promises.

    use std::cell::RefCell;
    use std::sync::{Arc, PoisonError};

    use super::SHARED;
    use crate::render_gpu::{GpuContext, GpuError, LadderOptions};

    type Ready = Box<dyn FnOnce(Result<(), GpuError>)>;

    /// Where making the device stands.
    enum State {
        /// Never asked, or the last device was lost.
        Idle,
        /// Being made; these run when it is.
        Making(Vec<Ready>),
        /// It could not be made, and why.
        Failed(GpuError),
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Idle) };
    }

    pub(super) fn state() -> Result<GpuContext, GpuError> {
        STATE.with_borrow(|s| match s {
            State::Idle => Err(GpuError::NoAdapter),
            State::Making(_) => Err(GpuError::Pending),
            State::Failed(e) => Err(e.clone()),
        })
    }

    pub(super) fn prepare(
        ladder: LadderOptions,
        window: Arc<dyn remedy_ui_core::host::WindowHandles>,
        ready: Ready,
    ) {
        let live =
            SHARED.lock().unwrap_or_else(PoisonError::into_inner).as_ref().is_some_and(|c| !c.is_lost());
        if live {
            ready(Ok(()));
            return;
        }
        let start = STATE.with_borrow_mut(|s| match s {
            State::Making(waiting) => {
                waiting.push(ready);
                None
            }
            // A failure is not final: a lost device, or a later window, may succeed.
            State::Idle | State::Failed(_) => {
                *s = State::Making(Vec::new());
                Some(ready)
            }
        });
        let Some(first) = start else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let made = GpuContext::new_async(&ladder, Some(window)).await;
            let result = match made {
                Ok(ctx) => {
                    *SHARED.lock().unwrap_or_else(PoisonError::into_inner) = Some(ctx);
                    Ok(())
                }
                Err(e) => Err(e),
            };
            let waiting = STATE.with_borrow_mut(|s| {
                let next = match &result {
                    Ok(()) => State::Idle,
                    Err(e) => State::Failed(e.clone()),
                };
                match std::mem::replace(s, next) {
                    State::Making(w) => w,
                    State::Idle | State::Failed(_) => Vec::new(),
                }
            });
            first(result.clone());
            for w in waiting {
                w(result.clone());
            }
        });
    }
}
