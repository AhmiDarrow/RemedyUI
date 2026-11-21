//! A window's swapchain of RemedyUI's own on wgpu's Vulkan device: its images keep what they
//! showed, so a present copies in only what changed since that image was last shown, and tells
//! the presentation engine only those rectangles (`VK_KHR_incremental_present`, where the
//! device has it).
//!
//! wgpu copies into the images as textures wrapped from them each frame. The image's acquire
//! semaphore gates wgpu's submission and a semaphore wgpu signals gates the present; a command
//! buffer of ours, submitted with wgpu's, returns the image to the present layout.
//!
//! One thread: Vulkan needs a queue's users kept apart, and wgpu does that only among its
//! own calls. This chain presents on wgpu's queue and waits for the device idle outside
//! wgpu's lock, and the acquire's semaphore is waited on by wgpu's *next* submission,
//! whichever thread makes it. So every submission to the device's queue must come from the
//! thread that presents windows. In this workspace it does: the runner, embedded GPU
//! content (`remedy-ui-embed-gpu`), effects (`remedy-ui-media-effects`) and video all submit
//! on the UI thread, and no other thread touches the GPU ([`GpuContext::queue`] says so to
//! apps). wgpu's own swapchain, which RemedyUI uses everywhere else, has no such rule.
//!
//! [`GpuContext::queue`]: crate::GpuContext::queue

use ash::vk;

type Vk = wgpu::hal::api::Vulkan;

/// How long to wait for the next image before reporting the swapchain busy.
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// Why a swapchain call failed.
#[derive(Debug)]
pub(crate) enum ChainError {
    /// The device was lost: recover it, then build again.
    Lost,
    /// The window's surface was lost: build again on a new one.
    SurfaceLost,
    /// The swapchain no longer fits the surface: build it again at the next present.
    OutOfDate,
    /// Anything else, with what was being done.
    Failed(String),
}

fn failed(what: &'static str) -> impl Fn(vk::Result) -> ChainError {
    move |r| match r {
        vk::Result::ERROR_DEVICE_LOST => ChainError::Lost,
        vk::Result::ERROR_SURFACE_LOST_KHR => ChainError::SurfaceLost,
        vk::Result::ERROR_OUT_OF_DATE_KHR => ChainError::OutOfDate,
        other => ChainError::Failed(format!("{what}: {other}")),
    }
}

/// The device extensions that let decoded pictures in DMA-BUFs be imported (`ffi::dmabuf`):
/// the first two are needed, the others used where the device has them (layouts other than
/// linear, the foreign queue family for the hand-over, and fences as sync files).
pub(crate) const DMA_BUF_EXTENSIONS: [&std::ffi::CStr; 5] = [
    ash::khr::external_memory_fd::NAME,
    ash::ext::external_memory_dma_buf::NAME,
    ash::ext::image_drm_format_modifier::NAME,
    ash::ext::queue_family_foreign::NAME,
    ash::khr::external_semaphore_fd::NAME,
];

/// Opens `adapter`'s Vulkan device as wgpu would, with `VK_KHR_incremental_present` added
/// where the device has it, plus what importing decoded pictures needs where the device has
/// it: on Linux the DMA-BUF extensions ([`DMA_BUF_EXTENSIONS`]), on Android
/// `VK_ANDROID_external_memory_android_hardware_buffer` with the Y'CbCr sampler feature
/// ([`super::video`]). Returns the device and whether presents can report what changed. `None`
/// when the adapter is not Vulkan or has none of these (the caller opens the device the usual
/// way). An error says why a device that has them would not open with them.
pub(crate) fn open_device(
    adapter: &wgpu::Adapter,
    desc: &wgpu::DeviceDescriptor<'_>,
) -> Option<Result<(wgpu::Device, wgpu::Queue, bool), String>> {
    // SAFETY: wgpu's own adapter. The device is made as `open_with_callback` makes it, from
    // wgpu-hal's own extension and feature lists, plus extensions the device reports and the
    // one feature bit (in wgpu's own feature struct) that the device reports too.
    unsafe {
        let hal = adapter.as_hal::<Vk>()?;
        let instance = hal.shared_instance().raw_instance();
        let physical = hal.raw_physical_device();
        let extensions = instance.enumerate_device_extension_properties(physical).ok()?;
        let has = |name: &std::ffi::CStr| extensions.iter().any(|e| e.extension_name_as_c_str() == Ok(name));
        let incremental = has(ash::khr::incremental_present::NAME);
        let api = instance.get_physical_device_properties(physical).api_version;
        // DMA-BUF import needs Vulkan 1.1 (external memory, dedicated allocations, bind
        // memory 2) and both of the first two extensions; only Linux imports DMA-BUFs.
        let mut dma_buf: Vec<&'static std::ffi::CStr> = Vec::new();
        if cfg!(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))
            && vk::api_version_minor(api) >= 1
            && DMA_BUF_EXTENSIONS.iter().take(2).all(|n| has(n))
        {
            dma_buf.extend(DMA_BUF_EXTENSIONS.iter().copied().filter(|n| has(n)));
            // The modifier extension needs `VK_KHR_image_format_list`, core from 1.2.
            if dma_buf.contains(&ash::ext::image_drm_format_modifier::NAME) && vk::api_version_minor(api) < 2
            {
                if has(ash::khr::image_format_list::NAME) {
                    dma_buf.push(ash::khr::image_format_list::NAME);
                } else {
                    dma_buf.retain(|n| *n != ash::ext::image_drm_format_modifier::NAME);
                }
            }
        }
        let wanted = super::video::wanted_extensions();
        let video = !wanted.is_empty() && wanted.iter().all(|n| has(n)) && {
            let mut ycbcr = vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default();
            let mut f = vk::PhysicalDeviceFeatures2::default().push_next(&mut ycbcr);
            instance.get_physical_device_features2(physical, &mut f);
            ycbcr.sampler_ycbcr_conversion == vk::TRUE && api >= vk::API_VERSION_1_1
        };
        if !incremental && !video && dma_buf.is_empty() {
            return None;
        }
        let open = |dma_buf: &[&'static std::ffi::CStr]| {
            let mut names = hal.required_device_extensions(desc.required_features);
            let mut add = |n: &'static std::ffi::CStr| {
                if !names.contains(&n) {
                    names.push(n);
                }
            };
            if incremental {
                add(ash::khr::incremental_present::NAME);
            }
            if video {
                wanted.iter().for_each(|n| add(n));
            }
            dma_buf.iter().for_each(|n| add(n));
            let mut features = hal.physical_device_features(&names, desc.required_features);
            let pointers: Vec<_> = names.iter().map(|n| n.as_ptr()).collect();
            let queues =
                [vk::DeviceQueueCreateInfo::default().queue_family_index(0).queue_priorities(&[1.0])];
            let info = features.add_to_device_create(
                vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&pointers),
            );
            if video {
                // wgpu chains the Y'CbCr conversion feature but leaves it off; it is turned on.
                let mut at = info.p_next as *mut vk::BaseOutStructure<'_>;
                while let Some(s) = at.as_mut() {
                    if s.s_type == vk::StructureType::PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES {
                        (*(at as *mut vk::PhysicalDeviceSamplerYcbcrConversionFeatures<'_>))
                            .sampler_ycbcr_conversion = vk::TRUE;
                    }
                    at = s.p_next;
                }
            }
            let raw = instance.create_device(physical, &info, None).map_err(|e| e.to_string())?;
            hal.device_from_raw(
                raw,
                None,
                &names,
                desc.required_features,
                &desc.required_limits,
                &desc.memory_hints,
                0,
                0,
            )
            .map_err(|e| e.to_string())
        };
        let open = match open(&dma_buf) {
            Ok(open) => open,
            // A device that will not take the DMA-BUF extensions still gets the rest; pictures
            // then come through CPU memory.
            Err(_) if (incremental || video) && !dma_buf.is_empty() => match open(&[]) {
                Ok(open) => open,
                Err(e) => return Some(Err(e)),
            },
            Err(e) => return Some(Err(e)),
        };
        drop(hal);
        Some(
            adapter
                .create_device_from_hal::<Vk>(open, desc)
                .map(|(d, q)| (d, q, incremental))
                .map_err(|e| e.to_string()),
        )
    }
}

/// A back buffer handed out for one frame.
pub(crate) struct Acquired {
    /// The image as a wgpu texture (copy source and destination, render target).
    pub texture: wgpu::Texture,
    /// Presents ago that this image was last shown, `0` when it holds nothing known.
    pub age: u32,
}

/// A swapchain image's per-image state.
struct Image {
    raw: vk::Image,
    /// Signalled by the frame's submission, waited for by its present.
    rendered: vk::Semaphore,
    /// The present it was last shown at (`0`: never).
    last_presented: u64,
}

/// The swapchain on wgpu's device for one window's surface.
pub(crate) struct VkChain {
    device: ash::Device,
    queue: vk::Queue,
    surface_fns: ash::khr::surface::Instance,
    swapchain_fns: ash::khr::swapchain::Device,
    physical: vk::PhysicalDevice,
    raw_surface: vk::SurfaceKHR,
    raw: vk::SwapchainKHR,
    images: Vec<Image>,
    /// Acquire semaphores, used in turn, each with the submission that last waited on it.
    acquires: Vec<(vk::Semaphore, Option<wgpu::SubmissionIndex>)>,
    next_acquire: usize,
    format: wgpu::TextureFormat,
    vk_format: vk::Format,
    /// The size asked for (the window's), and the size the images have (the surface's,
    /// which can differ while the window is being resized).
    want: (u32, u32),
    width: u32,
    height: u32,
    /// The swapchain no longer fits the surface (or there is none, while the surface has no
    /// size): it is built again before the next image, whatever size is asked for.
    stale: bool,
    vsync: bool,
    incremental: bool,
    presents: u64,
    /// The image and acquire semaphore slot handed out by `acquire`, until presented.
    current: Option<(u32, usize)>,
    /// wgpu's surface, never configured: it owns the `VkSurfaceKHR` and outlives the swapchain.
    _surface: wgpu::Surface<'static>,
    /// wgpu's device, kept so the `VkDevice` outlives the swapchain made on it.
    wgpu_device: wgpu::Device,
    /// Images wrapped as wgpu textures that wgpu has not let go of yet: an image must stay
    /// valid until then.
    wrapped: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

// SAFETY: Vulkan handles are plain values, used from one thread at a time (`&mut self`). That
// thread must also be the one that submits to wgpu's queue (see the module's docs): moving
// the chain to another thread moves the window's presents with it.
unsafe impl Send for VkChain {}

fn vk_format(format: wgpu::TextureFormat) -> Option<vk::Format> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Some(vk::Format::R8G8B8A8_UNORM),
        wgpu::TextureFormat::Bgra8Unorm => Some(vk::Format::B8G8R8A8_UNORM),
        _ => None,
    }
}

impl VkChain {
    /// A swapchain on `device` (a Vulkan device) for `surface`, `width` × `height` pixels in
    /// `format`. `None` when the device is not Vulkan or the surface cannot take copies: the
    /// caller configures wgpu's surface instead (and gets `surface` back).
    pub(crate) fn new(
        device: &wgpu::Device,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        vsync: bool,
        incremental: bool,
    ) -> Result<Result<VkChain, ChainError>, wgpu::Surface<'static>> {
        let Some(vk_format) = vk_format(format) else { return Err(surface) };
        // SAFETY: reading wgpu's own device and surface handles; the chain keeps the surface
        // (which owns the VkSurfaceKHR) and its own clones of the device's function tables.
        let parts = unsafe {
            let Some(raw_surface) = surface.as_hal::<Vk>().and_then(|s| s.raw_native_handle()) else {
                return Err(surface);
            };
            let Some(hal) = device.as_hal::<Vk>() else { return Err(surface) };
            let shared = hal.shared_instance();
            let surface_fns = ash::khr::surface::Instance::new(shared.entry(), shared.raw_instance());
            let swapchain_fns = ash::khr::swapchain::Device::new(shared.raw_instance(), hal.raw_device());
            let physical = hal.raw_physical_device();
            let supported = surface_fns
                .get_physical_device_surface_support(physical, hal.queue_family_index(), raw_surface)
                .unwrap_or(false);
            let caps = surface_fns.get_physical_device_surface_capabilities(physical, raw_surface).ok();
            let formats =
                surface_fns.get_physical_device_surface_formats(physical, raw_surface).unwrap_or_default();
            let usable = supported
                && caps.is_some_and(|c| c.supported_usage_flags.contains(vk::ImageUsageFlags::TRANSFER_DST))
                && formats
                    .iter()
                    .any(|f| f.format == vk_format && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR);
            if !usable {
                drop(hal);
                return Err(surface);
            }
            (hal.raw_device().clone(), hal.raw_queue(), surface_fns, swapchain_fns, physical, raw_surface)
        };
        let (device_fns, queue, surface_fns, swapchain_fns, physical, raw_surface) = parts;
        let mut chain = VkChain {
            device: device_fns,
            queue,
            surface_fns,
            swapchain_fns,
            physical,
            raw_surface,
            raw: vk::SwapchainKHR::null(),
            images: Vec::new(),
            acquires: Vec::new(),
            next_acquire: 0,
            format,
            vk_format,
            want: (width.max(1), height.max(1)),
            width: 0,
            height: 0,
            stale: true,
            vsync,
            incremental,
            presents: 0,
            current: None,
            _surface: surface,
            wgpu_device: device.clone(),
            wrapped: Default::default(),
        };
        // A surface with no size yet (a minimized window) gets its swapchain once it has one.
        Ok(match chain.rebuild() {
            Ok(()) | Err(ChainError::OutOfDate) => Ok(chain),
            Err(e) => Err(e),
        })
    }

    /// Makes the swapchain (again) at the size asked for, replacing the old one.
    /// `OutOfDate` while the surface has no size (the old swapchain, if any, stays).
    fn rebuild(&mut self) -> Result<(), ChainError> {
        // SAFETY: Vulkan calls on handles this chain owns; the device is idle before anything
        // it may still use is destroyed or replaced.
        unsafe {
            let caps = self
                .surface_fns
                .get_physical_device_surface_capabilities(self.physical, self.raw_surface)
                .map_err(failed("surface capabilities"))?;
            let modes = self
                .surface_fns
                .get_physical_device_surface_present_modes(self.physical, self.raw_surface)
                .unwrap_or_default();
            let mode = if self.vsync {
                vk::PresentModeKHR::FIFO
            } else if modes.contains(&vk::PresentModeKHR::MAILBOX) {
                vk::PresentModeKHR::MAILBOX
            } else if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
                vk::PresentModeKHR::IMMEDIATE
            } else {
                vk::PresentModeKHR::FIFO
            };
            let extent = if caps.current_extent.width == u32::MAX {
                vk::Extent2D {
                    width: self.want.0.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: self.want.1.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            } else {
                caps.current_extent
            };
            if extent.width == 0 || extent.height == 0 {
                // A minimized window: nothing to build until it has a size again.
                self.stale = true;
                return Err(ChainError::OutOfDate);
            }
            let mut count = caps.min_image_count.max(3);
            if caps.max_image_count > 0 {
                count = count.min(caps.max_image_count);
            }
            let alpha = [vk::CompositeAlphaFlagsKHR::OPAQUE, vk::CompositeAlphaFlagsKHR::INHERIT]
                .into_iter()
                .find(|a| caps.supported_composite_alpha.contains(*a))
                .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
            let mut usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT;
            if caps.supported_usage_flags.contains(vk::ImageUsageFlags::TRANSFER_SRC) {
                usage |= vk::ImageUsageFlags::TRANSFER_SRC;
            }
            // Frames are drawn upright: the presentation engine rotates them for a rotated
            // display (as with wgpu's swapchain). The surface's current transform would say
            // they are already rotated.
            let transform = if caps.supported_transforms.contains(vk::SurfaceTransformFlagsKHR::IDENTITY) {
                vk::SurfaceTransformFlagsKHR::IDENTITY
            } else {
                caps.current_transform
            };
            let old = self.raw;
            let info = vk::SwapchainCreateInfoKHR::default()
                .surface(self.raw_surface)
                .min_image_count(count)
                .image_format(self.vk_format)
                .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                .image_extent(extent)
                .image_array_layers(1)
                .image_usage(usage)
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(transform)
                .composite_alpha(alpha)
                .present_mode(mode)
                .clipped(true)
                .old_swapchain(old);
            self.let_go();
            // The old swapchain is retired by this call even when it fails.
            self.stale = true;
            let raw = self.swapchain_fns.create_swapchain(&info, None).map_err(failed("create swapchain"))?;
            for image in self.images.drain(..) {
                self.device.destroy_semaphore(image.rendered, None);
            }
            if old != vk::SwapchainKHR::null() {
                self.swapchain_fns.destroy_swapchain(old, None);
            }
            self.raw = raw;
            (self.width, self.height) = (extent.width, extent.height);
            let images = self.swapchain_fns.get_swapchain_images(raw).map_err(failed("swapchain images"))?;
            let semaphore = || self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
            let mut made = Vec::with_capacity(images.len());
            for raw in images {
                made.push(Image {
                    raw,
                    rendered: semaphore().map_err(failed("semaphore"))?,
                    last_presented: 0,
                });
            }
            while self.acquires.len() < made.len() + 1 {
                self.acquires.push((semaphore().map_err(failed("semaphore"))?, None));
            }
            self.images = made;
            self.current = None;
            self.stale = false;
        }
        Ok(())
    }

    /// Waits until the device has finished everything submitted and wgpu has let go of every
    /// texture wrapped from an image, so the images, and what waited on the semaphores, may go.
    fn let_go(&self) {
        // wgpu frees a texture once the submissions using it are done and it polls.
        let _ = self.wgpu_device.poll(wgpu::PollType::wait_indefinitely());
        // SAFETY: a call on wgpu's device, from the only thread that submits to its queue
        // (the module's docs).
        let _ = unsafe { self.device.device_wait_idle() };
        debug_assert_eq!(
            self.wrapped.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "wgpu still holds a swapchain image",
        );
    }

    /// Whether presents tell the presentation engine which rectangles changed.
    pub(crate) fn reports_damage(&self) -> bool {
        self.incremental
    }

    /// The size asked for last (the images have the surface's, which may differ).
    pub(crate) fn size(&self) -> (u32, u32) {
        self.want
    }

    /// Gives the swapchain a new size, or builds it again at the same size when it no longer
    /// fits the surface. `OutOfDate` while the surface has no size.
    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<(), ChainError> {
        let want = (width.max(1), height.max(1));
        if want == self.want && !self.stale {
            return Ok(());
        }
        self.want = want;
        self.rebuild()
    }

    /// The next image, once the swapchain hands one out. `Ok(None)` when none came within a
    /// second (the window is hidden): draw again later.
    pub(crate) fn acquire(&mut self, device: &wgpu::Device) -> Result<Option<Acquired>, ChainError> {
        if self.stale {
            self.rebuild()?;
        }
        let slot = self.next_acquire;
        let (semaphore, used) =
            self.acquires.get(slot).map(|(s, u)| (*s, u.clone())).ok_or(ChainError::OutOfDate)?;
        if let Some(index) = used {
            // The submission that waited on this semaphore has to be done before it is reused.
            let _ = device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None });
        }
        // SAFETY: an acquire on this chain's swapchain with an unsignalled semaphore of its own.
        let index = match unsafe {
            self.swapchain_fns.acquire_next_image(self.raw, ACQUIRE_TIMEOUT_NS, semaphore, vk::Fence::null())
        } {
            Ok((index, _suboptimal)) => index,
            Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => return Ok(None),
            Err(e) => {
                self.stale |= e == vk::Result::ERROR_OUT_OF_DATE_KHR;
                return Err(failed("acquire")(e));
            }
        };
        self.next_acquire = (slot + 1) % self.acquires.len();
        let image = self.images.get(index as usize).ok_or(ChainError::OutOfDate)?;
        let shown = image.last_presented > 0;
        let size = wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 };
        let usage =
            wgpu::TextureUses::COPY_DST | wgpu::TextureUses::COPY_SRC | wgpu::TextureUses::COLOR_TARGET;
        let wrapped = self.wrapped.clone();
        wrapped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // SAFETY: the image is this swapchain's, of this format and size; the drop callback
        // leaves it to the swapchain, which is destroyed only once wgpu let go of every
        // texture wrapped from it (`let_go`). It is in the present layout once it has been presented,
        // and holds nothing (undefined) before.
        let texture = unsafe {
            let hal = device.as_hal::<Vk>().ok_or(ChainError::Lost)?;
            let raw = hal.texture_from_raw(
                image.raw,
                &wgpu::hal::TextureDescriptor {
                    label: Some("RemedyUI swapchain image"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage,
                    memory_flags: wgpu::hal::MemoryFlags::empty(),
                    view_formats: Vec::new(),
                },
                Some(Box::new(move || {
                    wrapped.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                })),
                wgpu::hal::vulkan::TextureMemory::External,
            );
            drop(hal);
            device.create_texture_from_hal::<Vk>(
                raw,
                &wgpu::TextureDescriptor {
                    label: Some("RemedyUI swapchain image"),
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
                if shown { wgpu::TextureUses::PRESENT } else { wgpu::TextureUses::UNINITIALIZED },
            )
        };
        let age =
            if shown { u32::try_from(self.presents - image.last_presented + 1).unwrap_or(0) } else { 0 };
        self.current = Some((index, slot));
        Ok(Some(Acquired { texture, age }))
    }

    /// Makes wgpu's next submission wait for the acquired image (call before the frame's
    /// first submission that uses it).
    pub(crate) fn wait_for_image(&self, queue: &wgpu::Queue) {
        let Some((_, slot)) = self.current else { return };
        let Some(&(semaphore, _)) = self.acquires.get(slot) else { return };
        // SAFETY: the semaphore is the one the acquire signals; wgpu's next submission waits on
        // it once, and that is this frame's (only this thread submits: the module's docs).
        if let Some(hal) = unsafe { queue.as_hal::<Vk>() } {
            hal.add_wait_semaphore(semaphore, None, vk::PipelineStageFlags::ALL_COMMANDS);
        }
    }

    /// Submits `copies` (the frame's copies into the image, if not submitted already) with
    /// the image's return to the present layout in one submission, which signals that the
    /// frame is drawn, then presents it. `last` is what was last done to the image;
    /// `damage` lists what changed since it was last shown (`None`: all of it). Returns
    /// whether the swapchain should be built again (it no longer fits the surface exactly).
    pub(crate) fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        copies: Option<wgpu::CommandBuffer>,
        last: super::LastUse,
        damage: Option<&[(u16, u16, u16, u16)]>,
    ) -> Result<bool, ChainError> {
        let (index, slot) =
            self.current.take().ok_or_else(|| ChainError::Failed("nothing acquired".into()))?;
        let image = self.images.get(index as usize).ok_or(ChainError::OutOfDate)?;
        let (raw_image, rendered, shown) = (image.raw, image.rendered, image.last_presented > 0);
        let from = match last {
            super::LastUse::Written => {
                Some((vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::TRANSFER_WRITE))
            }
            super::LastUse::Read => {
                Some((vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::AccessFlags::TRANSFER_READ))
            }
            super::LastUse::Untouched if !shown => {
                Some((vk::ImageLayout::UNDEFINED, vk::AccessFlags::empty()))
            }
            super::LastUse::Untouched => None,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RemedyUI back to present"),
        });
        if let Some((old, access)) = from {
            let device_fns = &self.device;
            // SAFETY: recording one barrier into wgpu's open command buffer, which wgpu ends;
            // the image is this swapchain's.
            unsafe {
                encoder.as_hal_mut::<Vk, _, _>(|e| {
                    let Some(e) = e else { return };
                    let barrier = vk::ImageMemoryBarrier::default()
                        .src_access_mask(access)
                        .dst_access_mask(vk::AccessFlags::empty())
                        .old_layout(old)
                        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(raw_image)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        });
                    device_fns.cmd_pipeline_barrier(
                        e.raw_handle(),
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier],
                    );
                });
            }
        }
        // SAFETY: the semaphore is this image's; the present below waits on it once.
        if let Some(hal) = unsafe { queue.as_hal::<Vk>() } {
            hal.add_signal_semaphore(rendered, None);
        }
        let submitted = queue.submit(copies.into_iter().chain([encoder.finish()]));
        if let Some(a) = self.acquires.get_mut(slot) {
            a.1 = Some(submitted);
        }
        // A submission wgpu rejected (an invalid command buffer) never reached the queue, and
        // the semaphores staged for it would go with the next one, whoever makes it; the
        // present would wait for a signal that never comes. The image cannot be shown.
        // SAFETY: the semaphores are this chain's, staged above.
        if let Some(hal) = unsafe { queue.as_hal::<Vk>() }
            && hal.remove_signal_semaphore(rendered)
        {
            if let Some(&(acquired, _)) = self.acquires.get(slot) {
                hal.remove_wait_semaphore(acquired);
            }
            self.stale = true;
            return Err(ChainError::Failed("wgpu rejected the frame's submission".into()));
        }
        let rects: Vec<vk::RectLayerKHR> = damage
            .unwrap_or_default()
            .iter()
            .filter_map(|&(x0, y0, x1, y1)| {
                let (r, b) = (u32::from(x1).min(self.width), u32::from(y1).min(self.height));
                let (x0, y0) = (u32::from(x0), u32::from(y0));
                (x0 < r && y0 < b).then(|| vk::RectLayerKHR {
                    offset: vk::Offset2D { x: x0 as i32, y: y0 as i32 },
                    extent: vk::Extent2D { width: r - x0, height: b - y0 },
                    layer: 0,
                })
            })
            .collect();
        // Nothing changed: one pixel of an image equal to the window's frame says so (no
        // rectangle at all would mean all of it).
        let one = [vk::RectLayerKHR {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: 1, height: 1 },
            layer: 0,
        }];
        let rects: &[vk::RectLayerKHR] = if rects.is_empty() { &one } else { &rects };
        let regions = [vk::PresentRegionKHR::default().rectangles(rects)];
        let mut region_info = vk::PresentRegionsKHR::default().regions(&regions);
        let (waits, chains, indices) = ([rendered], [self.raw], [index]);
        let mut info =
            vk::PresentInfoKHR::default().wait_semaphores(&waits).swapchains(&chains).image_indices(&indices);
        if self.incremental && damage.is_some() {
            info = info.push_next(&mut region_info);
        }
        // SAFETY: presenting this chain's acquired image on wgpu's queue, outside wgpu's lock:
        // this is the only thread that submits to it (the module's docs).
        let result = unsafe { self.swapchain_fns.queue_present(self.queue, &info) };
        self.presents += 1;
        if let Some(image) = self.images.get_mut(index as usize) {
            image.last_presented = self.presents;
        }
        let rebuild = match result {
            // Android reports every present suboptimal while the display is rotated away from
            // the swapchain's transform; the compositor rotates (as with wgpu's swapchain).
            Ok(suboptimal) => suboptimal && cfg!(not(target_os = "android")),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => true,
            Err(e) => return Err(failed("present")(e)),
        };
        self.stale |= rebuild;
        Ok(rebuild)
    }
}

impl Drop for VkChain {
    fn drop(&mut self) {
        self.let_go();
        // SAFETY: the device is idle and wgpu holds no image before the swapchain and
        // semaphores go; wgpu's surface, which owns the VkSurfaceKHR, and wgpu's device are
        // dropped after (fields).
        unsafe {
            for image in self.images.drain(..) {
                self.device.destroy_semaphore(image.rendered, None);
            }
            for (s, _) in self.acquires.drain(..) {
                self.device.destroy_semaphore(s, None);
            }
            if self.raw != vk::SwapchainKHR::null() {
                self.swapchain_fns.destroy_swapchain(self.raw, None);
            }
        }
    }
}
