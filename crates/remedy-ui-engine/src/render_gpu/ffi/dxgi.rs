//! A window's swapchain of RemedyUI's own on wgpu's D3D12 device: flip-sequential, so each back
//! buffer keeps what it showed and a present copies in only what changed since, then tells
//! the system only those rectangles (`Present1` dirty rectangles) so it composites only them.
//!
//! wgpu draws and copies into the back buffers as textures wrapped from them each frame; the
//! swapchain is created on wgpu's own command queue, so D3D12 orders those copies before each
//! present. The one thing wgpu cannot do for a texture it did not make is hand it back in the
//! present state: a one-barrier command list of ours does that.

use windows::Win32::Foundation::{HANDLE, HWND, RECT, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0,
    D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE,
    D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE,
    D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATES, D3D12_RESOURCE_TRANSITION_BARRIER,
    ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue, ID3D12Device, ID3D12Fence,
    ID3D12GraphicsCommandList, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_FEATURE_PRESENT_ALLOW_TEARING, DXGI_MWA_NO_ALT_ENTER, DXGI_MWA_NO_WINDOW_CHANGES, DXGI_PRESENT,
    DXGI_PRESENT_ALLOW_TEARING, DXGI_PRESENT_PARAMETERS, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING,
    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory2, IDXGIFactory5, IDXGISwapChain3,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::Interface;

type Dx12 = wgpu::hal::api::Dx12;

/// Frames the CPU may run ahead of the display (wgpu's default), and the back buffers that
/// takes: one more than the latency.
const LATENCY: u32 = 2;
const BUFFERS: u32 = LATENCY + 1;

/// A present whose window is wholly covered or minimized (a success code).
const DXGI_STATUS_OCCLUDED: windows::core::HRESULT = windows::core::HRESULT(0x087A_0001);

/// Why a swapchain call failed.
#[derive(Debug)]
pub(crate) enum ChainError {
    /// The device was removed or reset: recover the device, then build again.
    Lost,
    /// Anything else, with what was being done.
    Failed(String),
}

fn failed(what: &str) -> impl Fn(windows::core::Error) -> ChainError + '_ {
    move |e| {
        let code = e.code();
        if code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_RESET {
            ChainError::Lost
        } else {
            ChainError::Failed(format!("{what}: {e}"))
        }
    }
}

/// A back buffer handed out for one frame.
pub(crate) struct Acquired {
    /// The back buffer as a wgpu texture (copy source and destination, render target).
    pub texture: wgpu::Texture,
    /// Presents ago that this buffer was last shown, `0` when it holds nothing known.
    pub age: u32,
}

pub(crate) use super::LastUse;

/// The swapchain, its back buffers and the small command lists that return them to the
/// present state.
pub(crate) struct DxgiChain {
    raw: IDXGISwapChain3,
    queue: ID3D12CommandQueue,
    buffers: Vec<ID3D12Resource>,
    /// One allocator and list per back buffer, reused when that buffer comes round again.
    lists: Vec<(ID3D12CommandAllocator, ID3D12GraphicsCommandList, u64)>,
    fence: ID3D12Fence,
    fence_value: u64,
    waitable: HANDLE,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    vsync: bool,
    tearing: bool,
    /// Presents so far, and the present each buffer was last shown at (`0`: never).
    presents: u64,
    last_presented: Vec<u64>,
    /// The buffer handed out by `acquire`, until it is presented.
    current: Option<usize>,
    /// wgpu's device, whose textures wrap the back buffers: the swapchain is released only
    /// once wgpu let go of them (a window takes a new swapchain only after the old one is gone).
    device: wgpu::Device,
}

// SAFETY: D3D12 queues, fences and resources are free-threaded; the command allocators and
// lists are used from one thread at a time (`&mut self`). Unlike the Vulkan chain, nothing
// here needs wgpu's other submissions on the same thread: a D3D12 queue orders them.
unsafe impl Send for DxgiChain {}

fn dxgi_format(format: wgpu::TextureFormat) -> Option<DXGI_FORMAT> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Some(DXGI_FORMAT_R8G8B8A8_UNORM),
        wgpu::TextureFormat::Bgra8Unorm => Some(DXGI_FORMAT_B8G8R8A8_UNORM),
        _ => None,
    }
}

fn flags(tearing: bool) -> DXGI_SWAP_CHAIN_FLAG {
    let mut f = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;
    if tearing {
        // Set whenever the system allows it: ResizeBuffers cannot change it later.
        f |= DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING;
    }
    f
}

fn hwnd(handles: &dyn remedy_ui_core::host::WindowHandles) -> Option<HWND> {
    use raw_window_handle::RawWindowHandle;
    match handles.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

impl DxgiChain {
    /// A swapchain for the window behind `handles` on `device` (a D3D12 device), `width` ×
    /// `height` pixels in `format`. `None` when the device is not D3D12 or the window is not
    /// a Win32 window: the caller presents through wgpu's own surface instead.
    pub(crate) fn new(
        device: &wgpu::Device,
        handles: &dyn remedy_ui_core::host::WindowHandles,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        vsync: bool,
    ) -> Option<Result<DxgiChain, ChainError>> {
        let hwnd = hwnd(handles)?;
        let dxgi = dxgi_format(format)?;
        // SAFETY: reading wgpu's own D3D12 device and queue; both outlive this chain, which
        // holds its own references.
        let (raw_device, queue): (ID3D12Device, ID3D12CommandQueue) = {
            let hal = unsafe { device.as_hal::<Dx12>() }?;
            (hal.raw_device().clone(), hal.raw_queue().clone())
        };
        Some(Self::build(device, &raw_device, queue, hwnd, width.max(1), height.max(1), format, dxgi, vsync))
    }

    #[allow(clippy::too_many_arguments, reason = "the swapchain's whole description")]
    fn build(
        wgpu_device: &wgpu::Device,
        device: &ID3D12Device,
        queue: ID3D12CommandQueue,
        hwnd: HWND,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        dxgi: DXGI_FORMAT,
        vsync: bool,
    ) -> Result<DxgiChain, ChainError> {
        // SAFETY: DXGI and D3D12 calls on objects made here or held by reference; every
        // pointer passed lives across its call.
        unsafe {
            let factory: IDXGIFactory2 =
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(failed("CreateDXGIFactory2"))?;
            let mut allow = windows::core::BOOL(0);
            let tearing = factory.cast::<IDXGIFactory5>().is_ok_and(|f5| {
                f5.CheckFeatureSupport(
                    DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                    (&raw mut allow).cast(),
                    std::mem::size_of_val(&allow) as u32,
                )
                .is_ok()
                    && allow.as_bool()
            });
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: dxgi,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: BUFFERS,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: flags(tearing).0 as u32,
            };
            let raw = factory
                .CreateSwapChainForHwnd(&queue, hwnd, &desc, None, None)
                .map_err(failed("CreateSwapChainForHwnd"))?
                .cast::<IDXGISwapChain3>()
                .map_err(failed("IDXGISwapChain3"))?;
            // The window, not DXGI, decides on full screen (Alt+Enter is the app's).
            factory
                .MakeWindowAssociation(hwnd, DXGI_MWA_NO_WINDOW_CHANGES | DXGI_MWA_NO_ALT_ENTER)
                .map_err(failed("MakeWindowAssociation"))?;
            raw.SetMaximumFrameLatency(LATENCY).map_err(failed("SetMaximumFrameLatency"))?;
            let waitable = raw.GetFrameLatencyWaitableObject();
            let fence: ID3D12Fence =
                device.CreateFence(0, D3D12_FENCE_FLAG_NONE).map_err(failed("CreateFence"))?;
            let mut lists = Vec::new();
            for _ in 0..BUFFERS {
                let allocator: ID3D12CommandAllocator = device
                    .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                    .map_err(failed("CreateCommandAllocator"))?;
                let list: ID3D12GraphicsCommandList = device
                    .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                    .map_err(failed("CreateCommandList"))?;
                list.Close().map_err(failed("Close"))?;
                lists.push((allocator, list, 0));
            }
            let mut chain = DxgiChain {
                raw,
                queue,
                buffers: Vec::new(),
                lists,
                fence,
                fence_value: 0,
                waitable,
                format,
                width,
                height,
                vsync,
                tearing,
                presents: 0,
                last_presented: vec![0; BUFFERS as usize],
                current: None,
                device: wgpu_device.clone(),
            };
            chain.take_buffers()?;
            Ok(chain)
        }
    }

    fn take_buffers(&mut self) -> Result<(), ChainError> {
        self.buffers.clear();
        for i in 0..BUFFERS {
            // SAFETY: the swapchain has BUFFERS buffers.
            let b: ID3D12Resource = unsafe { self.raw.GetBuffer(i) }.map_err(failed("GetBuffer"))?;
            self.buffers.push(b);
        }
        Ok(())
    }

    /// The size the back buffers have.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Gives the back buffers a new size. Waits for `device` to finish with them first (the
    /// swapchain cannot resize buffers the GPU still uses).
    pub(crate) fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<(), ChainError> {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        self.current = None;
        // wgpu lets go of the textures it wrapped once the GPU is done with them.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        self.wait_idle();
        self.buffers.clear();
        let dxgi = dxgi_format(self.format).ok_or_else(|| ChainError::Failed("no DXGI format".into()))?;
        // SAFETY: no reference to the old buffers is left (above).
        unsafe { self.raw.ResizeBuffers(BUFFERS, width, height, dxgi, flags(self.tearing)) }
            .map_err(failed("ResizeBuffers"))?;
        (self.width, self.height) = (width, height);
        // New buffers hold nothing known.
        self.last_presented.fill(0);
        self.take_buffers()
    }

    /// Waits until the queue has run everything sent to it so far: the barrier lists, and
    /// the presents, which use the back buffers after the lists' own fence values (the debug
    /// layer reports their release while a present is in flight as corruption).
    fn wait_idle(&mut self) {
        // SAFETY: a signal on the queue the chain presents on; a null event makes the wait
        // return once the fence reaches the value.
        unsafe {
            self.fence_value += 1;
            if self.queue.Signal(&self.fence, self.fence_value).is_err() {
                return;
            }
            if self.fence.GetCompletedValue() < self.fence_value {
                let _ = self.fence.SetEventOnCompletion(self.fence_value, HANDLE::default());
            }
        }
    }

    /// The next back buffer, once the swapchain takes another frame. `Ok(None)` when it has
    /// not within [`GpuLimits::swapchain_wait`](crate::render_gpu::GpuLimits::swapchain_wait) (the window
    /// is hidden, or the system is busy): draw again later.
    pub(crate) fn acquire(&mut self, device: &wgpu::Device) -> Result<Option<Acquired>, ChainError> {
        // A wait longer than a u32 of milliseconds holds is INFINITE (u32::MAX): no end.
        let wait = u32::try_from(crate::render_gpu::gpu_limits().swapchain_wait.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: waiting on the swapchain's own waitable object.
        match unsafe { WaitForSingleObject(self.waitable, wait) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(None),
            // A failed wait says nothing about the device: build the swapchain again.
            other => return Err(ChainError::Failed(format!("waiting for the swapchain: {other:?}"))),
        }
        // SAFETY: a query on a live swapchain.
        let index = unsafe { self.raw.GetCurrentBackBufferIndex() } as usize;
        let resource =
            self.buffers.get(index).cloned().ok_or_else(|| ChainError::Failed("no back buffer".into()))?;
        let size = wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 };
        // SAFETY: the resource is a live back buffer of this format and size, in the present
        // state (where every present leaves it), as the initial state says.
        let texture = unsafe {
            let hal = wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                self.format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            );
            device.create_texture_from_hal::<Dx12>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("RemedyUI back buffer"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::PRESENT,
            )
        };
        let age = match self.last_presented.get(index) {
            Some(&last) if last > 0 => u32::try_from(self.presents - last + 1).unwrap_or(0),
            _ => 0,
        };
        self.current = Some(index);
        Ok(Some(Acquired { texture, age }))
    }

    /// Presents the buffer `acquire` handed out, after the work wgpu submitted for it.
    /// `last` is what that work did to it; `damage` lists the rectangles that changed since
    /// it was last shown (`None`: all of it). Returns whether the window is occluded.
    pub(crate) fn present(
        &mut self,
        last: LastUse,
        damage: Option<&[(u16, u16, u16, u16)]>,
    ) -> Result<bool, ChainError> {
        let index = self.current.take().ok_or_else(|| ChainError::Failed("nothing acquired".into()))?;
        let before: Option<D3D12_RESOURCE_STATES> = match last {
            LastUse::Untouched => None,
            LastUse::Written => Some(D3D12_RESOURCE_STATE_COPY_DEST),
            LastUse::Read => Some(D3D12_RESOURCE_STATE_COPY_SOURCE),
        };
        if let Some(before) = before {
            self.back_to_present(index, before)?;
        }
        let interval = u32::from(self.vsync);
        let flags = if !self.vsync && self.tearing { DXGI_PRESENT_ALLOW_TEARING } else { DXGI_PRESENT(0) };
        let mut rects: Vec<RECT> = damage
            .unwrap_or_default()
            .iter()
            .filter_map(|&(x0, y0, x1, y1)| {
                let (r, b) = (u32::from(x1).min(self.width), u32::from(y1).min(self.height));
                (u32::from(x0) < r && u32::from(y0) < b).then(|| RECT {
                    left: i32::from(x0),
                    top: i32::from(y0),
                    right: r as i32,
                    bottom: b as i32,
                })
            })
            .collect();
        // A present that changed nothing still shows a frame (it paces to the display). The
        // buffer equals the window's frame, so one pixel of it stands for "nothing": no
        // rectangle at all would mean all of it.
        if damage.is_some() && rects.is_empty() {
            rects.push(RECT { left: 0, top: 0, right: 1, bottom: 1 });
        }
        let params = DXGI_PRESENT_PARAMETERS {
            DirtyRectsCount: if damage.is_some() { rects.len() as u32 } else { 0 },
            pDirtyRects: if damage.is_some() { rects.as_mut_ptr() } else { std::ptr::null_mut() },
            pScrollRect: std::ptr::null_mut(),
            pScrollOffset: std::ptr::null_mut(),
        };
        // SAFETY: the rectangles outlive the call; the buffer is in the present state.
        let hr = unsafe { self.raw.Present1(interval, flags, &params) };
        if let Err(e) = hr.ok() {
            return Err(failed("Present1")(e));
        }
        self.presents += 1;
        if let Some(l) = self.last_presented.get_mut(index) {
            *l = self.presents;
        }
        Ok(hr == DXGI_STATUS_OCCLUDED)
    }

    /// Returns back buffer `index` to the present state on wgpu's queue, after what wgpu
    /// submitted for it.
    fn back_to_present(&mut self, index: usize, before: D3D12_RESOURCE_STATES) -> Result<(), ChainError> {
        let resource = self.buffers.get(index).ok_or_else(|| ChainError::Failed("no back buffer".into()))?;
        let (allocator, list, used) =
            self.lists.get_mut(index).ok_or_else(|| ChainError::Failed("no barrier list".into()))?;
        // SAFETY: the allocator is reset only after the GPU ran what it last recorded (its
        // fence value); the barrier borrows the resource for the call (no reference taken),
        // and the list is closed before it is executed.
        unsafe {
            if self.fence.GetCompletedValue() < *used {
                let _ = self.fence.SetEventOnCompletion(*used, HANDLE::default());
            }
            allocator.Reset().map_err(failed("Reset allocator"))?;
            list.Reset(&*allocator, None).map_err(failed("Reset list"))?;
            let barrier = D3D12_RESOURCE_BARRIER {
                Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                Anonymous: D3D12_RESOURCE_BARRIER_0 {
                    Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                        pResource: std::mem::transmute_copy(resource),
                        Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                        StateBefore: before,
                        StateAfter: D3D12_RESOURCE_STATE_PRESENT,
                    }),
                },
            };
            list.ResourceBarrier(&[barrier]);
            list.Close().map_err(failed("Close"))?;
            let run: ID3D12CommandList = list.cast().map_err(failed("ID3D12CommandList"))?;
            self.queue.ExecuteCommandLists(&[Some(run)]);
            self.fence_value += 1;
            self.queue.Signal(&self.fence, self.fence_value).map_err(failed("Signal"))?;
            *used = self.fence_value;
        }
        Ok(())
    }
}

impl Drop for DxgiChain {
    fn drop(&mut self) {
        // wgpu lets go of the back buffers it wrapped once the GPU is done with them, and the
        // GPU may still run a barrier list or a present: the lists, buffers and swapchain go
        // only after.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.wait_idle();
        // SAFETY: the waitable object is the swapchain's, closed once.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.waitable);
        }
    }
}
