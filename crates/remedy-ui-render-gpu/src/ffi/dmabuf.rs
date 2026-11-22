//! Linux: decoded pictures in DMA-BUFs ([`GpuTexture::DmaBuf`]) imported on wgpu's Vulkan
//! device, so they are sampled where the decoder wrote them.
//!
//! Each plane is imported as an image of its own (`VK_EXT_external_memory_dma_buf`): NV12's
//! luma as R8 and its chroma as RG8, which the conversion shader samples as it samples
//! uploaded planes. The plane's layout is given with `VK_EXT_image_drm_format_modifier`; a
//! device without it takes linear planes whose row pitch, as the driver lays out a linear
//! image, is the buffer's. Imports are kept per buffer (a decoder's pool hands the same few
//! out again), keyed by the DMA-BUF's inode, which cannot be reused while the import holds it.
//!
//! Per picture: the writes still in flight in the buffer (its implicit fences) are taken as a
//! sync file and waited on by a submission of ours (`VK_KHR_external_semaphore_fd`), or
//! waited for on the CPU with `poll` where the device cannot import one. That submission takes
//! the images from the foreign queue family into the shader-read layout; wgpu's commands then
//! sample them, and a last submission of ours hands them back. The picture (and so its buffer)
//! is held until that one's fence has signalled.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_int, c_ulong};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use ash::vk;
use remedy_ui_paint::{DRM_FORMAT_MOD_LINEAR, DRM_FORMAT_NV12, GpuFrame, GpuTexture};

type Vk = wgpu::hal::api::Vulkan;

// `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` (Linux 6.0): `_IOWR('b', 2, struct dma_buf_export_sync_file)`.
const DMA_BUF_IOCTL_EXPORT_SYNC_FILE: c_ulong = 0xC008_6202;
/// Waits for the buffer's writers, as a reader must.
const DMA_BUF_SYNC_READ: u32 = 1;
const POLLIN: i16 = 1;

#[repr(C)]
struct ExportSyncFile {
    flags: u32,
    fd: i32,
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

// SAFETY: the C library's own `ioctl` and `poll`, with their documented signatures.
unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn poll(fds: *mut PollFd, n: c_ulong, timeout: c_int) -> c_int;
}

/// What wgpu's device can do with DMA-BUFs.
struct Device {
    raw: ash::Device,
    memory_fd: ash::khr::external_memory_fd::Device,
    /// Sync files imported as semaphores, where the device takes them.
    semaphore_fd: Option<ash::khr::external_semaphore_fd::Device>,
    /// Plane layouts given explicitly (`VK_EXT_image_drm_format_modifier`).
    modifiers: bool,
    /// The queue family pictures come from and go back to.
    foreign: u32,
    family: u32,
}

/// `device` as wgpu opened it, when it has the DMA-BUF import extensions.
fn device(device: &wgpu::Device) -> Option<Device> {
    // SAFETY: reading wgpu's own device; the clones of its tables hold no reference to wgpu.
    unsafe {
        let hal = device.as_hal::<Vk>()?;
        let enabled = hal.enabled_device_extensions();
        let on = |n: &std::ffi::CStr| enabled.contains(&n);
        if !(on(ash::khr::external_memory_fd::NAME) && on(ash::ext::external_memory_dma_buf::NAME)) {
            return None;
        }
        let instance = hal.shared_instance().raw_instance();
        let raw = hal.raw_device().clone();
        let semaphore_fd = on(ash::khr::external_semaphore_fd::NAME)
            .then(|| {
                // Only when sync files can be imported into a binary semaphore.
                let mut props = vk::ExternalSemaphoreProperties::default();
                instance.get_physical_device_external_semaphore_properties(
                    hal.raw_physical_device(),
                    &vk::PhysicalDeviceExternalSemaphoreInfo::default()
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                    &mut props,
                );
                props
                    .external_semaphore_features
                    .contains(vk::ExternalSemaphoreFeatureFlags::IMPORTABLE)
                    .then(|| ash::khr::external_semaphore_fd::Device::new(instance, &raw))
            })
            .flatten();
        Some(Device {
            memory_fd: ash::khr::external_memory_fd::Device::new(instance, &raw),
            semaphore_fd,
            modifiers: on(ash::ext::image_drm_format_modifier::NAME),
            foreign: if on(ash::ext::queue_family_foreign::NAME) {
                vk::QUEUE_FAMILY_FOREIGN_EXT
            } else {
                vk::QUEUE_FAMILY_EXTERNAL
            },
            family: hal.queue_family_index(),
            raw,
        })
    }
}

/// The device's UUID and the DRM format modifiers it imports both NV12 planes in (R8 and
/// RG8, one memory plane each, sampled), when it imports DMA-BUFs at all.
pub(crate) fn adapter(wgpu_device: &wgpu::Device) -> Option<([u8; 16], Vec<u64>)> {
    let d = device(wgpu_device)?;
    // SAFETY: queries on wgpu's own physical device into locals.
    unsafe {
        let hal = wgpu_device.as_hal::<Vk>()?;
        let instance = hal.shared_instance().raw_instance();
        let physical = hal.raw_physical_device();
        let mut id = vk::PhysicalDeviceIDProperties::default();
        let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut id);
        instance.get_physical_device_properties2(physical, &mut props);
        let uuid = id.device_uuid;
        let planes = [vk::Format::R8_UNORM, vk::Format::R8G8_UNORM];
        let modifiers: Vec<u64> = if d.modifiers {
            let each: Vec<Vec<u64>> = planes
                .iter()
                .map(|&format| {
                    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
                    let mut p = vk::FormatProperties2::default().push_next(&mut list);
                    instance.get_physical_device_format_properties2(physical, format, &mut p);
                    let n = list.drm_format_modifier_count as usize;
                    let mut entries = vec![vk::DrmFormatModifierPropertiesEXT::default(); n];
                    let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
                        .drm_format_modifier_properties(&mut entries);
                    let mut p = vk::FormatProperties2::default().push_next(&mut list);
                    instance.get_physical_device_format_properties2(physical, format, &mut p);
                    entries
                        .iter()
                        .filter(|m| {
                            m.drm_format_modifier_plane_count == 1
                                && m.drm_format_modifier_tiling_features
                                    .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
                        })
                        .map(|m| m.drm_format_modifier)
                        .collect()
                })
                .collect();
            let (first, rest) = each.split_first()?;
            first.iter().copied().filter(|m| rest.iter().all(|r| r.contains(m))).collect()
        } else {
            // Linear planes only, laid out by the driver (`import` checks the row pitch).
            let sampled = planes.iter().all(|&f| {
                instance
                    .get_physical_device_format_properties(physical, f)
                    .linear_tiling_features
                    .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
            });
            if sampled { vec![DRM_FORMAT_MOD_LINEAR] } else { Vec::new() }
        };
        (!modifiers.is_empty()).then_some((uuid, modifiers))
    }
}

/// One import: the same buffer, plane and layout is the same image.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    dev: u64,
    ino: u64,
    offset: u64,
    stride: u32,
    modifier: u64,
    size: (u32, u32),
    format: vk::Format,
}

/// A plane imported as an image; wgpu's texture owns the image and its memory.
struct Imported {
    texture: wgpu::Texture,
    image: vk::Image,
}

/// One picture's images, between `open` and `submit`.
struct Opening {
    images: Vec<vk::Image>,
    textures: Vec<wgpu::Texture>,
    /// Sync files of the picture's writes, waited on by the hand-over submission.
    waits: Vec<vk::Semaphore>,
}

/// A picture the GPU may still read, until `fence` signals.
struct InFlight {
    fence: vk::Fence,
    buffers: [vk::CommandBuffer; 2],
    waits: Vec<vk::Semaphore>,
    _picture: GpuFrame,
    _textures: Vec<wgpu::Texture>,
}

/// A stream's DMA-BUF pictures imported on wgpu's Vulkan device.
#[derive(Default)]
pub(crate) struct Shared {
    /// Read on first use; `None` inside when the device cannot import.
    device: Option<Option<Device>>,
    imported: HashMap<Key, Imported>,
    pool: vk::CommandPool,
    /// Command buffer pairs and fences whose work is done, to use again.
    free: Vec<([vk::CommandBuffer; 2], vk::Fence)>,
    in_flight: VecDeque<InFlight>,
    opening: Option<Opening>,
}

// SAFETY: Vulkan handles are plain values used from one thread at a time (the renderer's).
unsafe impl Send for Shared {}

/// Says once, in the log, why a picture in DMA-BUFs is read back rather than imported.
fn read_back(why: &str) {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!("remedy-ui-render-gpu: a video picture in DMA-BUFs is copied through CPU memory: {why}");
    }
}

impl Shared {
    /// Lets go of pictures the GPU has finished with.
    fn release(&mut self) {
        let Some(Some(d)) = &self.device else { return };
        // SAFETY: fences and semaphores made here; a picture's are let go once its last
        // submission's fence has signalled, so nothing on the GPU uses them any more.
        unsafe {
            while let Some(f) = self.in_flight.front() {
                if !d.raw.get_fence_status(f.fence).unwrap_or(true) {
                    break;
                }
                let Some(f) = self.in_flight.pop_front() else { break };
                for s in &f.waits {
                    d.raw.destroy_semaphore(*s, None);
                }
                if d.raw.reset_fences(&[f.fence]).is_ok() {
                    self.free.push((f.buffers, f.fence));
                } else {
                    d.raw.destroy_fence(f.fence, None);
                    d.raw.free_command_buffers(self.pool, &f.buffers);
                }
            }
        }
    }

    pub(crate) fn open(
        &mut self,
        device: &wgpu::Device,
        picture: &GpuFrame,
    ) -> Option<Vec<wgpu::TextureView>> {
        let GpuTexture::DmaBuf { planes, plane_count, drm_fourcc, modifier, width, height } =
            picture.0.texture()
        else {
            return None;
        };
        if self.device.is_none() {
            self.device = Some(self::device(device));
        }
        self.release();
        if drm_fourcc != DRM_FORMAT_NV12 || plane_count != 2 || width == 0 || height == 0 {
            read_back("only NV12 in two planes is imported");
            return None;
        }
        // Imports kept per stream before they are let go (a stream that opens another file
        // brings new ones): `GpuLimits::video_imports`.
        if self.imported.len() >= crate::gpu_limits().video_imports {
            self.imported.clear();
        }
        let layout = [
            (vk::Format::R8_UNORM, wgpu::TextureFormat::R8Unorm, (width, height)),
            (vk::Format::R8G8_UNORM, wgpu::TextureFormat::Rg8Unorm, (width.div_ceil(2), height.div_ceil(2))),
        ];
        let mut images = Vec::new();
        let mut textures = Vec::new();
        for (plane, (format, wgpu_format, size)) in planes.iter().zip(layout) {
            let Some(Some(d)) = &self.device else { return None };
            // SAFETY: the picture's descriptor, open while it lives (it is borrowed here).
            let file =
                std::fs::File::from(unsafe { BorrowedFd::borrow_raw(plane.fd) }.try_clone_to_owned().ok()?);
            let meta = file.metadata().ok()?;
            let key = Key {
                dev: meta.dev(),
                ino: meta.ino(),
                offset: plane.offset,
                stride: plane.stride,
                modifier,
                size,
                format,
            };
            let i = match self.imported.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    // SAFETY: the plane's layout is the decoder's; `import` checks it against
                    // what the device takes before binding.
                    let Some((image, memory)) =
                        (unsafe { import(d, file.into(), format, size, plane, modifier) })
                    else {
                        read_back("the Vulkan driver would not import its planes");
                        return None;
                    };
                    // SAFETY: an image of this format and size bound to imported memory; wgpu
                    // destroys both when the texture goes. Its layout is the shader-read one
                    // whenever wgpu records (the hand-over submissions put it there and back).
                    let texture = unsafe { wrap(device, image, memory, wgpu_format, size) }?;
                    e.insert(Imported { texture, image })
                }
            };
            images.push(i.image);
            textures.push(i.texture.clone());
        }
        let waits = self.wait_for_writes(&planes[..2]);
        let views = textures.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect();
        self.opening = Some(Opening { images, textures, waits });
        Some(views)
    }

    /// The picture's writes still in flight: as semaphores to wait on where the device imports
    /// sync files, otherwise waited for here.
    fn wait_for_writes(&self, planes: &[remedy_ui_paint::DmaBufPlane]) -> Vec<vk::Semaphore> {
        let Some(Some(d)) = &self.device else { return Vec::new() };
        let mut fds: Vec<i32> = planes.iter().map(|p| p.fd).collect();
        fds.dedup();
        let mut waits = Vec::new();
        for fd in fds {
            let sync = d.semaphore_fd.as_ref().and_then(|s| {
                let mut arg = ExportSyncFile { flags: DMA_BUF_SYNC_READ, fd: -1 };
                // SAFETY: the documented ioctl on the picture's descriptor, writing into `arg`.
                let ok =
                    unsafe { ioctl(fd, DMA_BUF_IOCTL_EXPORT_SYNC_FILE, &mut arg as *mut ExportSyncFile) };
                if ok != 0 || arg.fd < 0 {
                    return None;
                }
                // SAFETY: a semaphore of ours; on success the import owns the sync file,
                // otherwise it is closed here.
                unsafe {
                    let sem = d.raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).ok()?;
                    let info = vk::ImportSemaphoreFdInfoKHR::default()
                        .semaphore(sem)
                        .flags(vk::SemaphoreImportFlags::TEMPORARY)
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
                        .fd(arg.fd);
                    if s.import_semaphore_fd(&info).is_err() {
                        drop(OwnedFd::from_raw_fd(arg.fd));
                        d.raw.destroy_semaphore(sem, None);
                        return None;
                    }
                    Some(sem)
                }
            });
            match sync {
                Some(sem) => waits.push(sem),
                None => {
                    // A DMA-BUF is readable once its writes are done.
                    let mut p = PollFd { fd, events: POLLIN, revents: 0 };
                    // How long the writes are waited for on the CPU (`GpuLimits::video_fence_wait`;
                    // past `c_int::MAX` ms, -1: no end).
                    let wait =
                        c_int::try_from(crate::gpu_limits().video_fence_wait.as_millis()).unwrap_or(-1);
                    // SAFETY: polling one descriptor of the picture's.
                    unsafe { poll(&mut p, 1, wait) };
                }
            }
        }
        waits
    }

    pub(crate) fn submit(&mut self, queue: &wgpu::Queue, commands: wgpu::CommandBuffer, picture: &GpuFrame) {
        let Some(o) = self.opening.take() else {
            queue.submit([commands]);
            return;
        };
        let Some((buffers, fence)) = self.slot() else {
            // shortcut: without our hand-over the images stay in the foreign family's hands as
            // wgpu found them; the picture is only held until wgpu's work is done. Upgrade if a
            // command buffer ever fails to be made in practice.
            queue.submit([commands]);
            let held = (picture.clone(), o.textures);
            queue.on_submitted_work_done(move || drop(held));
            return;
        };
        let Some(Some(d)) = &self.device else { return };
        // SAFETY: wgpu's queue, used from the renderer's thread as all its submissions are.
        let raw_queue = unsafe { queue.as_hal::<Vk>() }.map(|q| q.as_raw());
        let barrier = |image, take: bool| {
            let (src, dst) = if take { (d.foreign, d.family) } else { (d.family, d.foreign) };
            let (old, new) = if take {
                (vk::ImageLayout::GENERAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            } else {
                (vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::GENERAL)
            };
            vk::ImageMemoryBarrier::default()
                .src_access_mask(if take { vk::AccessFlags::empty() } else { vk::AccessFlags::SHADER_READ })
                .dst_access_mask(if take { vk::AccessFlags::SHADER_READ } else { vk::AccessFlags::empty() })
                .old_layout(old)
                .new_layout(new)
                .src_queue_family_index(src)
                .dst_queue_family_index(dst)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
        };
        // SAFETY: command buffers of ours, free (their last fence signalled), recorded with
        // barriers on images that live while `o` does, and submitted around wgpu's commands on
        // wgpu's queue: the barriers order them before and after it in submission order.
        let done = unsafe {
            raw_queue.and_then(|q| {
                for (cb, take) in buffers.iter().zip([true, false]) {
                    let barriers: Vec<_> = o.images.iter().map(|&i| barrier(i, take)).collect();
                    d.raw
                        .begin_command_buffer(
                            *cb,
                            &vk::CommandBufferBeginInfo::default()
                                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                        )
                        .ok()?;
                    d.raw.cmd_pipeline_barrier(
                        *cb,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::PipelineStageFlags::ALL_COMMANDS,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &barriers,
                    );
                    d.raw.end_command_buffer(*cb).ok()?;
                }
                let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; o.waits.len()];
                let take = [buffers[0]];
                let give = [buffers[1]];
                d.raw
                    .queue_submit(
                        q,
                        &[vk::SubmitInfo::default()
                            .wait_semaphores(&o.waits)
                            .wait_dst_stage_mask(&stages)
                            .command_buffers(&take)],
                        vk::Fence::null(),
                    )
                    .ok()?;
                queue.submit([commands]);
                d.raw.queue_submit(q, &[vk::SubmitInfo::default().command_buffers(&give)], fence).ok()
            })
        };
        if done.is_none() {
            // The hand-over could not run: wait for the GPU, then let everything go.
            // SAFETY: waiting on wgpu's device, then destroying what was made here.
            unsafe {
                let _ = d.raw.device_wait_idle();
                for s in &o.waits {
                    d.raw.destroy_semaphore(*s, None);
                }
                d.raw.destroy_fence(fence, None);
                d.raw.free_command_buffers(self.pool, &buffers);
            }
            return;
        }
        self.in_flight.push_back(InFlight {
            fence,
            buffers,
            waits: o.waits,
            _picture: picture.clone(),
            _textures: o.textures,
        });
    }

    /// A free pair of command buffers and their fence, made when none is.
    fn slot(&mut self) -> Option<([vk::CommandBuffer; 2], vk::Fence)> {
        if let Some(s) = self.free.pop() {
            return Some(s);
        }
        let Some(Some(d)) = &self.device else { return None };
        // SAFETY: objects made on wgpu's device for this stream, destroyed in `drop`.
        unsafe {
            if self.pool == vk::CommandPool::null() {
                self.pool = d
                    .raw
                    .create_command_pool(
                        &vk::CommandPoolCreateInfo::default()
                            .queue_family_index(d.family)
                            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                        None,
                    )
                    .ok()?;
            }
            let cbs = d
                .raw
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(2),
                )
                .ok()?;
            let fence = d.raw.create_fence(&vk::FenceCreateInfo::default(), None).ok()?;
            Some(([*cbs.first()?, *cbs.get(1)?], fence))
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let Some(Some(d)) = &self.device else { return };
        // SAFETY: waiting for our submissions before destroying what they use; the pool frees
        // its command buffers.
        unsafe {
            let fences: Vec<vk::Fence> = self.in_flight.iter().map(|f| f.fence).collect();
            if !fences.is_empty() {
                let _ = d.raw.wait_for_fences(&fences, true, 5_000_000_000);
            }
            for f in self.in_flight.drain(..) {
                for s in &f.waits {
                    d.raw.destroy_semaphore(*s, None);
                }
                d.raw.destroy_fence(f.fence, None);
            }
            for (_, fence) in self.free.drain(..) {
                d.raw.destroy_fence(fence, None);
            }
            if self.pool != vk::CommandPool::null() {
                d.raw.destroy_command_pool(self.pool, None);
            }
        }
    }
}

/// Imports one plane of a DMA-BUF as an image of `format`, bound to the buffer's memory.
///
/// # Safety
/// `fd` is a DMA-BUF holding the plane at `plane`'s offset and stride, in `modifier`'s layout.
unsafe fn import(
    d: &Device,
    fd: OwnedFd,
    format: vk::Format,
    (w, h): (u32, u32),
    plane: &remedy_ui_paint::DmaBufPlane,
    modifier: u64,
) -> Option<(vk::Image, vk::DeviceMemory)> {
    if !d.modifiers && modifier != DRM_FORMAT_MOD_LINEAR {
        return None;
    }
    // The buffer's size: a DMA-BUF reports it as its end.
    let size = {
        use std::io::Seek;
        let mut f = std::fs::File::from(fd.try_clone().ok()?);
        let end = f.seek(std::io::SeekFrom::End(0)).ok()?;
        let _ = f.seek(std::io::SeekFrom::Start(0));
        end
    };
    let layouts = [vk::SubresourceLayout {
        offset: plane.offset,
        size: 0,
        row_pitch: u64::from(plane.stride),
        array_pitch: 0,
        depth_pitch: 0,
    }];
    let mut explicit = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(modifier)
        .plane_layouts(&layouts);
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width: w, height: h, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(if d.modifiers { vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT } else { vk::ImageTiling::LINEAR })
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external);
    if d.modifiers {
        info = info.push_next(&mut explicit);
    }
    // SAFETY: Vulkan calls on wgpu's device with structures built above. On failure the image
    // is destroyed and the descriptor closed; on success the memory owns the descriptor.
    unsafe {
        let image = d.raw.create_image(&info, None).ok()?;
        let fail = |image| {
            d.raw.destroy_image(image, None);
            None
        };
        // With explicit layouts the plane's offset is in the layout and the memory binds at
        // its start; a linear image laid out by the driver binds at the plane's offset, and
        // its rows must be the buffer's.
        let bind_at = if d.modifiers { 0 } else { plane.offset };
        if !d.modifiers {
            let l = d.raw.get_image_subresource_layout(
                image,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    array_layer: 0,
                },
            );
            if l.row_pitch != u64::from(plane.stride) || l.offset != 0 {
                return fail(image);
            }
        }
        let mut dedicated = vk::MemoryDedicatedRequirements::default();
        let mut req = vk::MemoryRequirements2::default().push_next(&mut dedicated);
        d.raw.get_image_memory_requirements2(
            &vk::ImageMemoryRequirementsInfo2::default().image(image),
            &mut req,
        );
        let req = req.memory_requirements;
        let mut fd_props = vk::MemoryFdPropertiesKHR::default();
        if d.memory_fd
            .get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                fd.as_raw_fd(),
                &mut fd_props,
            )
            .is_err()
        {
            return fail(image);
        }
        let types = req.memory_type_bits & fd_props.memory_type_bits;
        let use_dedicated =
            dedicated.requires_dedicated_allocation != 0 || dedicated.prefers_dedicated_allocation != 0;
        if types == 0
            || !bind_at.is_multiple_of(req.alignment.max(1))
            || bind_at + req.size > size
            || (use_dedicated && bind_at != 0 && dedicated.requires_dedicated_allocation != 0)
        {
            return fail(image);
        }
        let dedicated_too = use_dedicated && bind_at == 0;
        let raw_fd = fd.into_raw_fd();
        let mut import_info = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(raw_fd);
        let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let mut alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(if dedicated_too { req.size } else { size })
            .memory_type_index(types.trailing_zeros())
            .push_next(&mut import_info);
        if dedicated_too {
            alloc = alloc.push_next(&mut dedicated_info);
        }
        let memory = match d.raw.allocate_memory(&alloc, None) {
            Ok(m) => m,
            Err(_) => {
                drop(OwnedFd::from_raw_fd(raw_fd));
                return fail(image);
            }
        };
        if d.raw.bind_image_memory(image, memory, bind_at).is_err() {
            d.raw.free_memory(memory, None);
            return fail(image);
        }
        Some((image, memory))
    }
}

/// `image` (bound to `memory`) as a wgpu texture that owns both.
///
/// # Safety
/// `image` is a sampled 2D image of `format`'s Vulkan format and `size`, made on `device`.
unsafe fn wrap(
    device: &wgpu::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
) -> Option<wgpu::Texture> {
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    // SAFETY: as the function says; wgpu takes the image and its memory and frees them when
    // the texture goes, after the GPU work using it.
    unsafe {
        let hal = device.as_hal::<Vk>()?;
        let raw = hal.texture_from_raw(
            image,
            &wgpu::hal::TextureDescriptor {
                label: Some("RemedyUI decoded plane"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUses::RESOURCE,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            },
            None,
            wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
        );
        drop(hal);
        Some(device.create_texture_from_hal::<Vk>(
            raw,
            &wgpu::TextureDescriptor {
                label: Some("RemedyUI decoded plane"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::TextureUses::RESOURCE,
        ))
    }
}
