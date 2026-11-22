//! Windows: pictures decoded on the graphics card stay there. Media Foundation decodes on a
//! D3D11 device of the renderer's adapter (through an `IMFDXGIDeviceManager`); each picture
//! is copied on the GPU into an NV12 texture shared as an NT handle, which the renderer opens
//! on its D3D12 device, and a shared fence says when the copy is done.
//!
//! Textures come from a pool: one goes back when the renderer and the player have let go of
//! its picture ([`remedy_ui_paint::GpuPicture`]'s contract), so the pool holds as many as
//! there are pictures decoded ahead, as the copy path holds as many buffers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_paint::{CpuPixels, GpuPicture, GpuTexture, Plane};
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_DEBUG, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_FENCE_FLAG_SHARED, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MESSAGE,
    D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_RLDO_DETAIL,
    D3D11_RLDO_IGNORE_INTERNAL, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Debug, ID3D11Device, ID3D11Device5, ID3D11DeviceContext,
    ID3D11DeviceContext4, ID3D11Fence, ID3D11InfoQueue, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4, IDXGIResource1};
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIBuffer, IMFDXGIDeviceManager, IMFSample, MFCreateDXGIDeviceManager,
};
use windows::core::Interface;

use crate::host::{Picture, YuvMatrix};

/// Under the debug layer, a reference to each decoder device made since the last
/// [`live_objects_report`], so the report can still ask it what is alive once the decoder
/// has let go of everything else.
static WATCHED: Mutex<Vec<ID3D11Debug>> = Mutex::new(Vec::new());

/// For leak tests (`REMEDY_D3D_DEBUG=1`): asks each decoder device made since the last call
/// what is alive on it (`ReportLiveDeviceObjects`), then lets go of the devices. Returns the
/// layer's lines for every object beyond the device itself; empty when nothing leaked or
/// the debug layer is off.
pub(crate) fn live_objects_report() -> (usize, Vec<String>) {
    report_on(std::mem::take(&mut *WATCHED.lock().unwrap_or_else(PoisonError::into_inner)))
}

/// [`live_objects_report`] on `devices`, which are let go after.
fn report_on(devices: Vec<ID3D11Debug>) -> (usize, Vec<String>) {
    let (checked, mut leaked) = (devices.len(), Vec::new());
    for debug in devices {
        // SAFETY: plain debug-layer calls on a live device; each message is read into a buffer
        // of the size the queue says, aligned for the message it holds.
        unsafe {
            let Ok(queue) = debug.cast::<ID3D11InfoQueue>() else { continue };
            // Kept off the debugger output: the GPU renderer's handler ends the process on
            // any warning, and each live object is reported as one.
            queue.SetMuteDebugOutput(true);
            queue.ClearStoredMessages();
            let _ = debug.ReportLiveDeviceObjects(D3D11_RLDO_DETAIL | D3D11_RLDO_IGNORE_INTERNAL);
            for i in 0..queue.GetNumStoredMessages() {
                let mut len = 0;
                if queue.GetMessage(i, None, &mut len).is_err() || len == 0 {
                    continue;
                }
                let mut buf = vec![0u64; len.div_ceil(8)];
                let m = buf.as_mut_ptr().cast::<D3D11_MESSAGE>();
                if queue.GetMessage(i, Some(m), &mut len).is_err() || (*m).pDescription.is_null() {
                    continue;
                }
                let text = std::slice::from_raw_parts((*m).pDescription, (*m).DescriptionByteLength);
                let text = String::from_utf8_lossy(text).trim_matches(['\0', '\t', ' ']).to_owned();
                // The device itself (held here) and the summary line are not leaks.
                if text.starts_with("Live ") && !text.starts_with("Live ID3D11Device ") {
                    leaked.push(text);
                }
            }
            queue.ClearStoredMessages();
            queue.SetMuteDebugOutput(false);
        }
    }
    (checked, leaked)
}

/// Process-unique keys for shared textures and fences: a renderer opens each key once.
static KEYS: AtomicU64 = AtomicU64::new(1);

fn key() -> u64 {
    KEYS.fetch_add(1, Ordering::Relaxed)
}

fn err(what: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |e| format!("{what}: {} ({:#010x})", e.message(), e.code().0)
}

/// An NT handle, closed with its owner.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle was made by CreateSharedHandle and is closed once, here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A pooled NV12 texture and its shared handle.
struct Slot {
    texture: ID3D11Texture2D,
    handle: Owned,
    key: u64,
    size: (u32, u32),
}

struct Fence {
    fence: ID3D11Fence,
    handle: Owned,
    key: u64,
}

/// The D3D11 device and immediate context, used from the decoder's thread and (to read a
/// picture back) the renderer's: the device is multithread-protected.
#[derive(Clone)]
struct Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext4,
}

// SAFETY: textures, fences and the context of a multithread-protected D3D11 device are
// free-threaded, and NT handles may be used from any thread; `&self` methods only read
// handles or call the protected context.
unsafe impl Send for Slot {}
// SAFETY: as above.
unsafe impl Sync for Slot {}
// SAFETY: as above.
unsafe impl Send for Fence {}
// SAFETY: as above.
unsafe impl Sync for Fence {}
// SAFETY: as above.
unsafe impl Send for Device {}
// SAFETY: as above.
unsafe impl Sync for Device {}

/// A decoder's hardware decoding and its pool of shared textures.
pub(super) struct Gpu {
    dev: Device,
    /// Handed to the source reader so it decodes on `dev`.
    pub(super) manager: IMFDXGIDeviceManager,
    fence: Arc<Fence>,
    value: u64,
    adapter: u64,
    pool: Arc<Mutex<Vec<Slot>>>,
    _live: super::Live,
}

// SAFETY: D3D11 objects of a multithread-protected device and the DXGI device manager are
// free-threaded; the decoder that owns this uses it from one thread at a time.
unsafe impl Send for Gpu {}

/// The D3D11 debug layer, for test runs with `REMEDY_D3D_DEBUG=1` (docs/d3d-debug-layers.md);
/// the GPU renderer, made first, reports its messages.
fn debug_layer() -> D3D11_CREATE_DEVICE_FLAG {
    match std::env::var_os("REMEDY_D3D_DEBUG") {
        Some(v) if v != "0" => D3D11_CREATE_DEVICE_DEBUG,
        _ => D3D11_CREATE_DEVICE_FLAG(0),
    }
}

impl Gpu {
    /// The LUID of the adapter it decodes on.
    pub(super) fn adapter(&self) -> u64 {
        self.adapter
    }

    /// Hardware decoding on the adapter with LUID `adapter`; why not, when it cannot.
    pub(super) fn new(adapter: u64) -> Result<Gpu, String> {
        let luid = LUID { LowPart: adapter as u32, HighPart: (adapter >> 32) as i32 };
        // SAFETY: plain D3D11, DXGI and Media Foundation calls on objects made here; every
        // out-pointer is a local that outlives its call.
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory1().map_err(err("CreateDXGIFactory1"))?;
            let a: IDXGIAdapter = factory.EnumAdapterByLuid(luid).map_err(err("the renderer's adapter"))?;
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                &a,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT | debug_layer(),
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(err("D3D11CreateDevice with video support"))?;
            let device: ID3D11Device = device.ok_or("D3D11CreateDevice gave no device")?;
            if debug_layer() == D3D11_CREATE_DEVICE_DEBUG
                && let Ok(debug) = device.cast::<ID3D11Debug>()
            {
                WATCHED.lock().unwrap_or_else(PoisonError::into_inner).push(debug);
            }
            let context: ID3D11DeviceContext = context.ok_or("D3D11CreateDevice gave no context")?;
            // Media Foundation's threads and the renderer's read-backs share the context.
            let mt: ID3D11Multithread = context.cast().map_err(err("ID3D11Multithread"))?;
            let _ = mt.SetMultithreadProtected(true);
            let context: ID3D11DeviceContext4 =
                context.cast().map_err(err("ID3D11DeviceContext4 (fences)"))?;
            let device5: ID3D11Device5 = device.cast().map_err(err("ID3D11Device5 (fences)"))?;
            let mut fence: Option<ID3D11Fence> = None;
            device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence).map_err(err("CreateFence"))?;
            let fence = fence.ok_or("CreateFence gave no fence")?;
            let handle =
                fence.CreateSharedHandle(None, GENERIC_ALL.0, None).map_err(err("sharing the fence"))?;
            let (mut token, mut manager) = (0u32, None);
            MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(err("MFCreateDXGIDeviceManager"))?;
            let manager: IMFDXGIDeviceManager = manager.ok_or("no DXGI device manager")?;
            manager.ResetDevice(&device, token).map_err(err("IMFDXGIDeviceManager::ResetDevice"))?;
            Ok(Gpu {
                dev: Device { device, context },
                manager,
                fence: Arc::new(Fence { fence, handle: Owned(handle), key: key() }),
                value: 0,
                adapter,
                pool: Arc::new(Mutex::new(Vec::new())),
                _live: super::Live::new(),
            })
        }
    }

    /// A free pooled texture of `size`, or a new one.
    fn slot(&self, size: (u32, u32)) -> Result<Slot, String> {
        let mut pool = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        // Textures of another size (the stream changed size) are no use again.
        pool.retain(|s| s.size == size);
        if let Some(s) = pool.pop() {
            return Ok(s);
        }
        drop(pool);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
        };
        // SAFETY: a texture made from a filled description, then a handle to share it.
        unsafe {
            let mut texture = None;
            self.dev
                .device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .map_err(err("a shared NV12 texture"))?;
            let texture: ID3D11Texture2D = texture.ok_or("CreateTexture2D gave no texture")?;
            let r: IDXGIResource1 = texture.cast().map_err(err("IDXGIResource1"))?;
            let handle = r.CreateSharedHandle(None, GENERIC_ALL.0, None).map_err(err("sharing a texture"))?;
            Ok(Slot { texture, handle: Owned(handle), key: key(), size })
        }
    }

    /// The decoded `sample` as a picture on the GPU: its shown part (`x`, `y`, `width` ×
    /// `height` of the decoder's texture) copied on the GPU into a shared texture. Never
    /// locks the sample's buffer.
    #[allow(clippy::too_many_arguments, reason = "the picture's whole layout")]
    pub(super) fn picture(
        &mut self,
        sample: &IMFSample,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        matrix: YuvMatrix,
        full_range: bool,
    ) -> Result<Picture, String> {
        // NV12 copies take even boxes.
        let size = (width.next_multiple_of(2), height.next_multiple_of(2));
        // SAFETY: reading the decoded texture behind a live sample, copying from it on the
        // GPU and signalling the fence; the out-pointer is a local.
        unsafe {
            let buffer = sample.GetBufferByIndex(0).map_err(err("the picture's buffer"))?;
            let dxgi: IMFDXGIBuffer =
                buffer.cast().map_err(|_| "the decoder gave a picture in CPU memory".to_string())?;
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw).map_err(err("the picture's texture"))?;
            let source = ID3D11Texture2D::from_raw(raw);
            let index = dxgi.GetSubresourceIndex().map_err(err("the picture's subresource"))?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_NV12 {
                return Err(format!("the decoder's pictures are not NV12 ({:?})", desc.Format));
            }
            if x + size.0 > desc.Width || y + size.1 > desc.Height {
                return Err("the picture's shown part lies outside its texture".into());
            }
            let slot = self.slot(size)?;
            let area =
                D3D11_BOX { left: x, top: y, front: 0, right: x + size.0, bottom: y + size.1, back: 1 };
            self.dev.context.CopySubresourceRegion(&slot.texture, 0, 0, 0, 0, &source, index, Some(&area));
            self.value += 1;
            self.dev.context.Signal(&self.fence.fence, self.value).map_err(err("signalling the fence"))?;
            // Sent now, so the renderer's wait on the fence is met without waiting for more work.
            self.dev.context.Flush();
            let shown = Shown {
                slot: Some(slot),
                pool: self.pool.clone(),
                fence: self.fence.clone(),
                value: self.value,
                adapter: self.adapter,
                dev: self.dev.clone(),
                _live: super::Live::new(),
            };
            Ok(Picture::on_gpu(width, height, matrix, full_range, Arc::new(shown)))
        }
    }
}

/// One picture on the GPU; its texture goes back to the pool when it is dropped.
struct Shown {
    slot: Option<Slot>,
    pool: Arc<Mutex<Vec<Slot>>>,
    fence: Arc<Fence>,
    value: u64,
    adapter: u64,
    dev: Device,
    _live: super::Live,
}

impl Drop for Shown {
    fn drop(&mut self) {
        if let Some(s) = self.slot.take() {
            self.pool.lock().unwrap_or_else(PoisonError::into_inner).push(s);
        }
    }
}

impl GpuPicture for Shown {
    fn texture(&self) -> GpuTexture {
        let (texture, texture_key, (width, height)) =
            self.slot.as_ref().map_or((0, 0, (0, 0)), |s| (s.handle.0.0 as usize, s.key, s.size));
        GpuTexture::D3dNv12 {
            adapter: self.adapter,
            texture,
            texture_key,
            fence: self.fence.handle.0.0 as usize,
            fence_key: self.fence.key,
            value: self.value,
            width,
            height,
        }
    }

    fn read_back(&self) -> Option<CpuPixels> {
        let slot = self.slot.as_ref()?;
        let (w, h) = slot.size;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        // SAFETY: a staging copy on the same context as the copy that wrote the texture (so
        // after it), mapped while its `pitch * h * 3 / 2` bytes are copied out, then unmapped.
        unsafe {
            let mut staging = None;
            self.dev.device.CreateTexture2D(&desc, None, Some(&mut staging)).ok()?;
            let staging: ID3D11Texture2D = staging?;
            let ctx = &self.dev.context;
            ctx.CopyResource(&staging, &slot.texture);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).ok()?;
            let pitch = map.RowPitch as usize;
            let len = pitch * h as usize * 3 / 2;
            let data: Option<Arc<[u8]>> = (!map.pData.is_null())
                .then(|| Arc::from(std::slice::from_raw_parts(map.pData.cast::<u8>(), len)));
            ctx.Unmap(&staging, 0);
            Some(CpuPixels {
                format: remedy_ui_paint::PixelFormat::Nv12,
                planes: [
                    Plane { offset: 0, stride: pitch },
                    Plane { offset: pitch * h as usize, stride: pitch },
                    Plane::default(),
                ],
                data: data?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report finds a texture kept past its decoder's device, and nothing once it goes.
    /// Needs the debug layer: run with `REMEDY_D3D_DEBUG=1`.
    #[test]
    fn the_live_object_report_names_a_kept_texture() {
        if debug_layer() != D3D11_CREATE_DEVICE_DEBUG {
            return eprintln!("skipped: set REMEDY_D3D_DEBUG=1");
        }
        // SAFETY: plain DXGI calls into locals.
        let luid = unsafe {
            let f: windows::Win32::Graphics::Dxgi::IDXGIFactory1 =
                CreateDXGIFactory1().unwrap_or_else(|e| unreachable!("{e}"));
            let d = f.EnumAdapters1(0).and_then(|a| a.GetDesc1()).unwrap_or_else(|e| unreachable!("{e}"));
            (u64::from(d.AdapterLuid.HighPart as u32) << 32) | u64::from(d.AdapterLuid.LowPart)
        };
        // Its own devices' reports: other tests here make decoder devices at the same time.
        let debug = |g: &Gpu| g.dev.device.cast::<ID3D11Debug>().unwrap_or_else(|e| unreachable!("{e}"));
        let gpu = match Gpu::new(luid) {
            Ok(g) => g,
            Err(why) => return eprintln!("skipped: no D3D11 video device ({why})"),
        };
        let watch = debug(&gpu);
        let kept = gpu.slot((64, 64)).unwrap_or_else(|e| unreachable!("{e}"));
        drop(gpu);
        let (devices, leaked) = report_on(vec![watch]);
        assert_eq!(devices, 1);
        assert!(leaked.iter().any(|l| l.contains("ID3D11Texture2D")), "{leaked:#?}");
        drop(kept);
        // A second device, let go whole: nothing beyond it.
        let gpu = Gpu::new(luid).unwrap_or_else(|e| unreachable!("{e}"));
        let watch = debug(&gpu);
        drop(gpu);
        assert_eq!(report_on(vec![watch]), (1, Vec::new()));
    }
}
