//! Decoded pictures that are already on the GPU ([`remedy_ui_paint::GpuPicture`]), opened on
//! the renderer's device where the platform shares them with it. Where it does not (yet), a
//! picture is read back and uploaded like any other frame.
//!
//! Windows: a decoder's NV12 texture, shared from D3D11 as an NT handle, is opened on wgpu's
//! D3D12 device and wrapped as a wgpu texture for each picture. Its submission waits on the
//! decoder's shared fence first, and is followed by a one-barrier list of ours that hands the
//! texture back in the common state (wgpu leaves it in the shader-resource state), so D3D11
//! may write the next picture into it. The picture is held until that list has run.
//!
//! Android: a MediaCodec picture's `AHardwareBuffer` is imported into wgpu's Vulkan device and
//! drawn into the stream's converted picture by a pass of ours ([`android`]).
//!
//! Linux: a picture in DMA-BUFs is imported on wgpu's Vulkan device, one image per plane
//! (`ffi::dmabuf`).
//!
//! [`Shared::open`] gives the views the conversion shader samples (luma, then chroma);
//! [`Shared::submit`] submits the commands that read them.

#[cfg(not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios"))))))]
use remedy_ui_paint::GpuFrame;

#[cfg(target_os = "android")]
mod android;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub(crate) use super::dmabuf::Shared;
#[cfg(target_os = "android")]
pub(crate) use android::{Shared, wanted_extensions};
#[cfg(windows)]
pub(crate) use windows_shared::Shared;

/// Device extensions importing pictures needs, which [`super::vulkan::open_device`] adds
/// where the device has them all (none here: Linux's DMA-BUF extensions are added by their
/// own rule, [`super::vulkan::DMA_BUF_EXTENSIONS`]).
#[cfg(not(target_os = "android"))]
#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios"))),
    all(windows, feature = "windows-vulkan")
))]
pub(crate) fn wanted_extensions() -> &'static [&'static std::ffi::CStr] {
    &[]
}

/// A stream's pictures opened on this device, and those the GPU is still reading.
#[cfg(not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios"))))))]
#[derive(Default)]
pub(crate) struct Shared(());

#[cfg(not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios"))))))]
impl Shared {
    /// The views of `picture`'s planes opened on `device`; `None` when it cannot open them.
    pub(crate) fn open(
        &mut self,
        device: &wgpu::Device,
        picture: &GpuFrame,
    ) -> Option<Vec<wgpu::TextureView>> {
        let _ = (device, picture);
        None
    }

    /// Submits `commands`, which read the views [`Shared::open`] gave.
    pub(crate) fn submit(&mut self, queue: &wgpu::Queue, commands: wgpu::CommandBuffer, picture: &GpuFrame) {
        let _ = picture;
        queue.submit([commands]);
    }
}

/// Says which GPU takes pictures on the GPU: `device`'s adapter, when it can open them.
pub(crate) fn announce(device: &wgpu::Device) {
    #[cfg(windows)]
    {
        if let Some(luid) = windows_shared::adapter(device) {
            remedy_ui_paint::set_gpu_video_adapter(Some(remedy_ui_paint::GpuAdapter::Luid(luid)));
        }
    }
    #[cfg(target_os = "android")]
    {
        if android::can_import(device) {
            remedy_ui_paint::set_gpu_video_adapter(Some(remedy_ui_paint::GpuAdapter::Only));
        }
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        if let Some((uuid, modifiers)) = super::dmabuf::adapter(device) {
            remedy_ui_paint::set_gpu_video_dma_buf_modifiers(modifiers);
            remedy_ui_paint::set_gpu_video_adapter(Some(remedy_ui_paint::GpuAdapter::Uuid(uuid)));
        }
    }
    #[cfg(not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios"))))))]
    let _ = device;
}

#[cfg(windows)]
mod windows_shared {
    use std::collections::{HashMap, VecDeque};

    use remedy_ui_paint::{GpuFrame, GpuTexture};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER,
        D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE,
        D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_STATE_COMMON,
        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        D3D12_RESOURCE_TRANSITION_BARRIER, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
        ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
    };
    use windows::core::Interface;

    type Dx12 = wgpu::hal::api::Dx12;

    /// Opened textures kept per stream before the oldest are let go (a decoder's pool is a
    /// few dozen at most; a stream that opens another file brings new ones).
    const KEEP: usize = 64;

    /// wgpu's D3D12 device and queue, when the device is D3D12 and can sample NV12.
    fn raw(device: &wgpu::Device) -> Option<(ID3D12Device, ID3D12CommandQueue)> {
        if !device.features().contains(wgpu::Features::TEXTURE_FORMAT_NV12) {
            return None;
        }
        // SAFETY: reading wgpu's own device and queue; the clones hold their own references.
        let hal = unsafe { device.as_hal::<Dx12>() }?;
        Some((hal.raw_device().clone(), hal.raw_queue().clone()))
    }

    /// The LUID of `device`'s adapter, when it can open shared NV12 pictures.
    pub(super) fn adapter(device: &wgpu::Device) -> Option<u64> {
        let (d, _) = raw(device)?;
        // SAFETY: a query on a live device.
        let l = unsafe { d.GetAdapterLuid() };
        Some(u64::from(l.LowPart) | (u64::from(l.HighPart as u32) << 32))
    }

    /// One picture's texture on this device, between `open` and `submit`.
    struct Opening {
        texture: wgpu::Texture,
        resource: ID3D12Resource,
        fence: ID3D12Fence,
        value: u64,
    }

    /// A stream's shared pictures opened on wgpu's D3D12 device.
    #[derive(Default)]
    pub(crate) struct Shared {
        textures: HashMap<u64, ID3D12Resource>,
        fences: HashMap<u64, ID3D12Fence>,
        opening: Option<Opening>,
        /// Our barrier lists and the `done` value each last ran to.
        lists: Vec<(ID3D12CommandAllocator, ID3D12GraphicsCommandList, u64)>,
        done: Option<ID3D12Fence>,
        done_value: u64,
        /// Pictures the GPU may still read, with the `done` value that says it has finished.
        held: VecDeque<(u64, GpuFrame)>,
    }

    // SAFETY: D3D12 objects are free-threaded; a renderer uses its streams from one thread.
    unsafe impl Send for Shared {}

    impl Shared {
        /// Lets go of pictures the GPU has finished with.
        fn release(&mut self) {
            let Some(done) = &self.done else { return };
            // SAFETY: a query on a live fence.
            let reached = unsafe { done.GetCompletedValue() };
            while self.held.front().is_some_and(|(v, _)| *v <= reached) {
                self.held.pop_front();
            }
        }

        pub(crate) fn open(
            &mut self,
            device: &wgpu::Device,
            picture: &GpuFrame,
        ) -> Option<Vec<wgpu::TextureView>> {
            self.release();
            let GpuTexture::D3dNv12 {
                adapter: luid,
                texture,
                texture_key,
                fence,
                fence_key,
                value,
                width,
                height,
            } = picture.0.texture()
            else {
                return None;
            };
            let (d, _) = raw(device)?;
            if adapter(device)? != luid || width == 0 || height == 0 {
                return None;
            }
            if self.textures.len() >= KEEP {
                self.textures.clear();
            }
            // SAFETY: the handles are the picture's, open while it lives (it is borrowed here);
            // OpenSharedHandle takes its own reference to each object.
            let resource = match self.textures.get(&texture_key) {
                Some(r) => r.clone(),
                None => unsafe {
                    let mut r: Option<ID3D12Resource> = None;
                    d.OpenSharedHandle(HANDLE(texture as *mut _), &mut r).ok()?;
                    let r = r?;
                    self.textures.insert(texture_key, r.clone());
                    r
                },
            };
            let fence = match self.fences.get(&fence_key) {
                Some(f) => f.clone(),
                None => unsafe {
                    let mut f: Option<ID3D12Fence> = None;
                    d.OpenSharedHandle(HANDLE(fence as *mut _), &mut f).ok()?;
                    let f = f?;
                    self.fences.insert(fence_key, f.clone());
                    f
                },
            };
            let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
            // SAFETY: the resource is an NV12 texture of this size made by the decoder's device
            // on this adapter, in the common state (D3D11 leaves it there and our barrier list
            // returns it there), as the initial state says.
            let t = unsafe {
                let hal = wgpu::hal::dx12::Device::texture_from_raw(
                    resource.clone(),
                    wgpu::TextureFormat::NV12,
                    wgpu::TextureDimension::D2,
                    size,
                    1,
                    1,
                );
                device.create_texture_from_hal::<Dx12>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("RemedyUI decoded picture"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::NV12,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::PRESENT,
                )
            };
            // The Y and UV planes, sampled in place.
            let views = [wgpu::TextureAspect::Plane0, wgpu::TextureAspect::Plane1]
                .into_iter()
                .zip([wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm])
                .map(|(aspect, format)| {
                    t.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(format),
                        aspect,
                        ..Default::default()
                    })
                })
                .collect();
            self.opening = Some(Opening { texture: t, resource, fence, value });
            Some(views)
        }

        pub(crate) fn submit(
            &mut self,
            queue: &wgpu::Queue,
            commands: wgpu::CommandBuffer,
            picture: &GpuFrame,
        ) {
            let Some(o) = self.opening.take() else {
                queue.submit([commands]);
                return;
            };
            let raw_queue = {
                // SAFETY: reading wgpu's own queue; the clone holds its own reference. The
                // submission below waits for the decoder's copy to signal before it reads.
                let hal = unsafe { queue.as_hal::<Dx12>() };
                hal.map(|q| {
                    q.add_wait_fence(o.fence.clone(), o.value);
                    q.as_raw().clone()
                })
            };
            queue.submit([commands]);
            drop(o.texture);
            match raw_queue.and_then(|q| self.give_back(&q, &o.resource)) {
                Some(()) => self.held.push_back((self.done_value, picture.clone())),
                None => {
                    // shortcut: without our barrier list the texture stays in the
                    // shader-resource state; the picture is only held until wgpu's work is done.
                    // Upgrade if a list ever fails to be made in practice.
                    let held = picture.clone();
                    queue.on_submitted_work_done(move || drop(held));
                }
            }
        }

        /// Runs a barrier returning `resource` to the common state after what is queued, and
        /// signals `done` after it.
        fn give_back(&mut self, queue: &ID3D12CommandQueue, resource: &ID3D12Resource) -> Option<()> {
            // SAFETY: D3D12 calls on objects made here or held alive by `self`; a list is reset
            // only once the GPU has run it (its `done` value is reached), and closed before it
            // is executed. The barrier borrows `resource`, which outlives the call.
            unsafe {
                let mut device: Option<ID3D12Device> = None;
                queue.GetDevice(&mut device).ok()?;
                let device = device?;
                if self.done.is_none() {
                    self.done = Some(device.CreateFence(0, D3D12_FENCE_FLAG_NONE).ok()?);
                }
                let done = self.done.clone()?;
                let reached = done.GetCompletedValue();
                let free = self.lists.iter().position(|(_, _, v)| *v <= reached);
                let i = match free {
                    Some(i) => i,
                    None => {
                        let a: ID3D12CommandAllocator =
                            device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).ok()?;
                        let l: ID3D12GraphicsCommandList =
                            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &a, None).ok()?;
                        l.Close().ok()?;
                        self.lists.push((a, l, 0));
                        self.lists.len() - 1
                    }
                };
                let (allocator, list, used) = self.lists.get_mut(i)?;
                allocator.Reset().ok()?;
                list.Reset(&*allocator, None).ok()?;
                let barrier = D3D12_RESOURCE_BARRIER {
                    Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                    Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                    Anonymous: D3D12_RESOURCE_BARRIER_0 {
                        Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                            pResource: std::mem::transmute_copy(resource),
                            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                            // Where wgpu leaves a texture it sampled.
                            StateBefore: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
                                | D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                            StateAfter: D3D12_RESOURCE_STATE_COMMON,
                        }),
                    },
                };
                list.ResourceBarrier(&[barrier]);
                list.Close().ok()?;
                let run: ID3D12CommandList = list.cast().ok()?;
                queue.ExecuteCommandLists(&[Some(run)]);
                self.done_value += 1;
                queue.Signal(&done, self.done_value).ok()?;
                *used = self.done_value;
                Some(())
            }
        }
    }

    impl Drop for Shared {
        fn drop(&mut self) {
            // Pictures and lists go only once the GPU is done with them.
            if let Some(done) = &self.done {
                // SAFETY: a null event makes the call return once the fence reaches the value.
                unsafe {
                    if done.GetCompletedValue() < self.done_value {
                        let _ = done.SetEventOnCompletion(self.done_value, HANDLE::default());
                    }
                }
            }
        }
    }
}
