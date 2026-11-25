//! Android: a MediaCodec picture's `AHardwareBuffer` imported into wgpu's Vulkan device
//! (`VK_ANDROID_external_memory_android_hardware_buffer`) and drawn into the stream's converted
//! picture through a `VkSamplerYcbcrConversion` for the buffer's format, which may be one only
//! the driver knows (an external format). wgpu cannot bind such a sampler, so the draw is a
//! render pass of ours in a command buffer of its own, submitted right after a wgpu pass that
//! clears the same texture: wgpu then knows the texture as a written render target, which our
//! pass leaves it as.
//!
//! The decoder hands over complete pictures (it waits on the image's acquire fence), so nothing
//! here waits. Each picture, with what imports it, is held until wgpu's queue has run the draw.

use std::collections::HashMap;

use crate::paint::{GpuFrame, GpuTexture, YuvMatrix};
use ash::vk;

type Vk = wgpu::hal::api::Vulkan;

const AHB: &std::ffi::CStr = ash::android::external_memory_android_hardware_buffer::NAME;

/// Device extensions importing pictures needs (Y'CbCr conversion and external memory are
/// Vulkan 1.1, which [`crate::render_gpu::ffi::vulkan::open_device`] asks for too).
pub(crate) fn wanted_extensions() -> &'static [&'static std::ffi::CStr] {
    &[AHB, ash::ext::queue_family_foreign::NAME]
}

/// `device` was opened with what importing pictures needs.
pub(crate) fn can_import(device: &wgpu::Device) -> bool {
    // SAFETY: reading wgpu's own device's extension list.
    unsafe { device.as_hal::<Vk>() }.is_some_and(|d| d.enabled_device_extensions().contains(&AHB))
}

/// The objects one kind of picture is drawn with: its conversion is part of the sampler, the
/// sampler part of the set layout, and the layout part of the pipeline.
struct Pipe {
    conversion: vk::SamplerYcbcrConversion,
    sampler: vk::Sampler,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

/// What a kind of picture is drawn with depends on: its format (or external format), matrix
/// and range.
type Key = (vk::Format, u64, vk::SamplerYcbcrModelConversion, vk::SamplerYcbcrRange);

/// The stream's converted picture as our render pass's target, and the pipelines.
struct Pass {
    device: ash::Device,
    ahb: ash::android::external_memory_android_hardware_buffer::Device,
    family: u32,
    render_pass: vk::RenderPass,
    view: vk::ImageView,
    framebuffer: vk::Framebuffer,
    size: (u32, u32),
    vert: vk::ShaderModule,
    frag: vk::ShaderModule,
    pipes: HashMap<Key, Pipe>,
    /// Keep the texture and the device alive while the objects above use them.
    _target: wgpu::Texture,
    _wgpu: wgpu::Device,
}

/// One picture imported for one draw; let go once the draw has run.
struct Imported {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    _picture: GpuFrame,
    _wgpu: wgpu::Device,
}

impl Drop for Imported {
    fn drop(&mut self) {
        // SAFETY: made by `Shared::convert` on this device; the GPU has finished with them.
        unsafe {
            self.device.destroy_descriptor_pool(self.pool, None);
            self.device.destroy_image_view(self.view, None);
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

/// A stream's imported pictures.
#[derive(Default)]
pub(crate) struct Shared {
    pass: Option<Pass>,
}

impl Shared {
    /// Pictures here are not opened as wgpu textures ([`Shared::convert`] draws them).
    pub(crate) fn open(
        &mut self,
        device: &wgpu::Device,
        picture: &GpuFrame,
    ) -> Option<Vec<wgpu::TextureView>> {
        let _ = (device, picture);
        None
    }

    /// Submits `commands` (never after [`Shared::open`], which opens nothing).
    pub(crate) fn submit(&mut self, queue: &wgpu::Queue, commands: wgpu::CommandBuffer, picture: &GpuFrame) {
        let _ = picture;
        queue.submit([commands]);
    }

    /// Draws `picture` into `target` (the stream's converted picture, the frame's size) and
    /// submits it; false when this device cannot import it (the caller reads it back).
    pub(crate) fn convert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picture: &GpuFrame,
        matrix: YuvMatrix,
        full_range: bool,
        target: &wgpu::Texture,
    ) -> bool {
        let GpuTexture::HardwareBuffer { buffer, width, height, left, top } = picture.0.texture() else {
            return false;
        };
        if buffer == 0 || !can_import(device) {
            return false;
        }
        if self.pass.is_none() {
            self.pass = Pass::new(device, target);
        }
        let Some(pass) = self.pass.as_mut() else { return false };
        let model = match matrix {
            YuvMatrix::Bt601 => vk::SamplerYcbcrModelConversion::YCBCR_601,
            YuvMatrix::Bt709 => vk::SamplerYcbcrModelConversion::YCBCR_709,
        };
        let range =
            if full_range { vk::SamplerYcbcrRange::ITU_FULL } else { vk::SamplerYcbcrRange::ITU_NARROW };
        let Some((imported, key)) =
            pass.import(device, picture, buffer as *mut _, (width, height), model, range)
        else {
            return false;
        };
        // wgpu's pass: the texture becomes a render target wgpu knows is written.
        let mut clear =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI video") });
        {
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let _ = clear.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RemedyUI video"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let mut draw =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI video") });
        let origin = [left as f32, top as f32, 1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32];
        // SAFETY: recording into wgpu's open command buffer of an encoder used only raw, which
        // wgpu ends and submits after the clear above. Every object is alive until the draw has
        // run (the pass's in `self`, the picture's in `imported`).
        let recorded = unsafe {
            draw.as_hal_mut::<Vk, _, _>(|e| {
                let Some(e) = e else { return false };
                pass.record(e.raw_handle(), &imported, key, origin);
                true
            })
        };
        if !recorded {
            return false;
        }
        queue.submit([clear.finish(), draw.finish()]);
        queue.on_submitted_work_done(move || drop(imported));
        true
    }
}

impl Pass {
    fn new(device: &wgpu::Device, target: &wgpu::Texture) -> Option<Pass> {
        // SAFETY: objects made on wgpu's own device from its handles; the target's image is
        // wgpu's, of `FORMAT` and its size, viewed (never destroyed) here.
        unsafe {
            let hal = device.as_hal::<Vk>()?;
            let d = hal.raw_device().clone();
            let ahb = ash::android::external_memory_android_hardware_buffer::Device::new(
                hal.shared_instance().raw_instance(),
                &d,
            );
            let family = hal.queue_family_index();
            drop(hal);
            let image = target.as_hal::<Vk>()?.raw_handle();
            let attachment = [vk::AttachmentDescription::default()
                .format(vk::Format::R8G8B8A8_UNORM)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                // Where wgpu's clear leaves it, and where wgpu expects it after.
                .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let color = [vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let subpass = [vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color)];
            // After wgpu's clear of the same texture.
            let after = [vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
            let render_pass = d
                .create_render_pass(
                    &vk::RenderPassCreateInfo::default()
                        .attachments(&attachment)
                        .subpasses(&subpass)
                        .dependencies(&after),
                    None,
                )
                .ok()?;
            let size = (target.width(), target.height());
            let view = d
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(vk::Format::R8G8B8A8_UNORM)
                        .subresource_range(color_range()),
                    None,
                )
                .ok()?;
            let views = [view];
            let framebuffer = d
                .create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(&views)
                        .width(size.0)
                        .height(size.1)
                        .layers(1),
                    None,
                )
                .ok()?;
            let module = |bytes: &[u8]| {
                let code = ash::util::read_spv(&mut std::io::Cursor::new(bytes)).ok()?;
                d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None).ok()
            };
            let vert = module(include_bytes!("../shaders/ycbcr.vert.spv"))?;
            let frag = module(include_bytes!("../shaders/ycbcr.frag.spv"))?;
            Some(Pass {
                device: d,
                ahb,
                family,
                render_pass,
                view,
                framebuffer,
                size,
                vert,
                frag,
                pipes: HashMap::new(),
                _target: target.clone(),
                _wgpu: device.clone(),
            })
        }
    }

    /// Imports `buffer` (`size`, the picture's) and makes, or finds, what draws it.
    fn import(
        &mut self,
        wgpu_device: &wgpu::Device,
        picture: &GpuFrame,
        buffer: *mut vk::AHardwareBuffer,
        size: (u32, u32),
        model: vk::SamplerYcbcrModelConversion,
        range: vk::SamplerYcbcrRange,
    ) -> Option<(Imported, Key)> {
        let d = self.device.clone();
        // SAFETY: the buffer is the picture's, valid while it lives (it is borrowed here and
        // held by `Imported`); importing takes a reference of its own. Every object made is
        // destroyed on failure or by `Imported` / `Pass`.
        unsafe {
            let mut format = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
            let mut props = vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format);
            self.ahb.get_android_hardware_buffer_properties(buffer, &mut props).ok()?;
            let (allocation, types) = (props.allocation_size, props.memory_type_bits);
            let external = if format.format == vk::Format::UNDEFINED { format.external_format } else { 0 };
            let key = (format.format, external, model, range);
            if !self.pipes.contains_key(&key) {
                let pipe = self.pipe(&format, external, model, range)?;
                self.pipes.insert(key, pipe);
            }
            let pipe = self.pipes.get(&key)?;
            let mut memory_info = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
            let mut external_info = vk::ExternalFormatANDROID::default().external_format(external);
            let image = d
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format.format)
                        .extent(vk::Extent3D { width: size.0, height: size.1, depth: 1 })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::SAMPLED)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .push_next(&mut memory_info)
                        .push_next(&mut external_info),
                    None,
                )
                .ok()?;
            let mut import = vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(buffer);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let memory = match d.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(allocation)
                    .memory_type_index(types.trailing_zeros())
                    .push_next(&mut import)
                    .push_next(&mut dedicated),
                None,
            ) {
                Ok(m) => m,
                Err(_) => {
                    d.destroy_image(image, None);
                    return None;
                }
            };
            let mut imported = Imported {
                device: d.clone(),
                image,
                memory,
                view: vk::ImageView::null(),
                pool: vk::DescriptorPool::null(),
                set: vk::DescriptorSet::null(),
                _picture: picture.clone(),
                _wgpu: wgpu_device.clone(),
            };
            d.bind_image_memory(image, memory, 0).ok()?;
            let mut conversion = vk::SamplerYcbcrConversionInfo::default().conversion(pipe.conversion);
            imported.view = d
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format.format)
                        .subresource_range(color_range())
                        .push_next(&mut conversion),
                    None,
                )
                .ok()?;
            // A Y'CbCr sampler may take a descriptor per plane (three at most).
            let sizes = [vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 3,
            }];
            imported.pool = d
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&sizes),
                    None,
                )
                .ok()?;
            let layouts = [pipe.set_layout];
            imported.set = *d
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(imported.pool)
                        .set_layouts(&layouts),
                )
                .ok()?
                .first()?;
            let image_info = [vk::DescriptorImageInfo::default()
                .image_view(imported.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            d.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(imported.set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&image_info)],
                &[],
            );
            Some((imported, key))
        }
    }

    /// The conversion, sampler, layouts and pipeline for one kind of picture.
    fn pipe(
        &self,
        format: &vk::AndroidHardwareBufferFormatPropertiesANDROID<'_>,
        external: u64,
        model: vk::SamplerYcbcrModelConversion,
        range: vk::SamplerYcbcrRange,
    ) -> Option<Pipe> {
        let d = &self.device;
        // SAFETY: objects made on this device; each is destroyed with the `Pipe` (in `Drop`),
        // or here when a later one fails.
        unsafe {
            let mut external_info = vk::ExternalFormatANDROID::default().external_format(external);
            // The matrix and range are the frame's (as the copy path converts); the rest is
            // what the driver suggests for the buffer. Nearest chroma, as the copy path reads.
            let conversion = d
                .create_sampler_ycbcr_conversion(
                    &vk::SamplerYcbcrConversionCreateInfo::default()
                        .format(format.format)
                        .ycbcr_model(model)
                        .ycbcr_range(range)
                        .components(format.sampler_ycbcr_conversion_components)
                        .x_chroma_offset(format.suggested_x_chroma_offset)
                        .y_chroma_offset(format.suggested_y_chroma_offset)
                        .chroma_filter(vk::Filter::NEAREST)
                        .push_next(&mut external_info),
                    None,
                )
                .ok()?;
            let mut pipe = Pipe {
                conversion,
                sampler: vk::Sampler::null(),
                set_layout: vk::DescriptorSetLayout::null(),
                layout: vk::PipelineLayout::null(),
                pipeline: vk::Pipeline::null(),
            };
            let made = (|| {
                let mut info = vk::SamplerYcbcrConversionInfo::default().conversion(conversion);
                pipe.sampler = d
                    .create_sampler(
                        &vk::SamplerCreateInfo::default()
                            .mag_filter(vk::Filter::NEAREST)
                            .min_filter(vk::Filter::NEAREST)
                            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .push_next(&mut info),
                        None,
                    )
                    .ok()?;
                let samplers = [pipe.sampler];
                let binding = [vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                    .immutable_samplers(&samplers)];
                pipe.set_layout = d
                    .create_descriptor_set_layout(
                        &vk::DescriptorSetLayoutCreateInfo::default().bindings(&binding),
                        None,
                    )
                    .ok()?;
                let sets = [pipe.set_layout];
                let push = [vk::PushConstantRange {
                    stage_flags: vk::ShaderStageFlags::FRAGMENT,
                    offset: 0,
                    size: 16,
                }];
                pipe.layout = d
                    .create_pipeline_layout(
                        &vk::PipelineLayoutCreateInfo::default()
                            .set_layouts(&sets)
                            .push_constant_ranges(&push),
                        None,
                    )
                    .ok()?;
                let stages = [
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::VERTEX)
                        .module(self.vert)
                        .name(c"main"),
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::FRAGMENT)
                        .module(self.frag)
                        .name(c"main"),
                ];
                let vertex = vk::PipelineVertexInputStateCreateInfo::default();
                let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                    .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
                let viewport =
                    vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
                let raster = vk::PipelineRasterizationStateCreateInfo::default()
                    .polygon_mode(vk::PolygonMode::FILL)
                    .cull_mode(vk::CullModeFlags::NONE)
                    .line_width(1.0);
                let multisample = vk::PipelineMultisampleStateCreateInfo::default()
                    .rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let blend_attachment = [vk::PipelineColorBlendAttachmentState::default()
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachment);
                let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
                let info = [vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vertex)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(pipe.layout)
                    .render_pass(self.render_pass)
                    .subpass(0)];
                pipe.pipeline =
                    *d.create_graphics_pipelines(vk::PipelineCache::null(), &info, None).ok()?.first()?;
                Some(())
            })();
            if made.is_none() {
                pipe.destroy(d);
                return None;
            }
            Some(pipe)
        }
    }

    /// Records the draw of `imported` (drawn with `key`'s pipe) into `cmd`. `crop` is the
    /// picture's origin in its buffer and one over the buffer's size.
    ///
    /// # Safety
    /// `cmd` is recording, outside a render pass, on this device's queue family.
    unsafe fn record(&self, cmd: vk::CommandBuffer, imported: &Imported, key: Key, crop: [f32; 4]) {
        let Some(pipe) = self.pipes.get(&key) else { return };
        let d = &self.device;
        let (w, h) = self.size;
        // SAFETY: as the caller promises; every handle is alive (see `Shared::convert`).
        unsafe {
            // The buffer comes from outside Vulkan (the decoder), whole: its contents are kept.
            let acquire = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                .dst_queue_family_index(self.family)
                .image(imported.image)
                .subresource_range(color_range());
            d.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[acquire],
            );
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width: w, height: h },
            };
            d.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(self.framebuffer)
                    .render_area(area),
                vk::SubpassContents::INLINE,
            );
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipe.pipeline);
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: w as f32,
                    height: h as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            d.cmd_set_scissor(cmd, 0, &[area]);
            d.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                pipe.layout,
                0,
                &[imported.set],
                &[],
            );
            let bytes: Vec<u8> = crop.iter().flat_map(|v| v.to_ne_bytes()).collect();
            d.cmd_push_constants(cmd, pipe.layout, vk::ShaderStageFlags::FRAGMENT, 0, &bytes);
            d.cmd_draw(cmd, 3, 1, 0, 0);
            d.cmd_end_render_pass(cmd);
        }
    }
}

impl Pipe {
    /// # Safety
    /// The GPU has finished with these objects, all made on `d` (or null).
    unsafe fn destroy(&self, d: &ash::Device) {
        // SAFETY: as the caller promises; destroying null handles is allowed.
        unsafe {
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_sampler(self.sampler, None);
            d.destroy_sampler_ycbcr_conversion(self.conversion, None);
        }
    }
}

impl Drop for Pass {
    fn drop(&mut self) {
        // SAFETY: once the device is idle nothing uses these objects, all made here.
        unsafe {
            let _ = self.device.device_wait_idle();
            for pipe in self.pipes.values() {
                pipe.destroy(&self.device);
            }
            self.device.destroy_shader_module(self.vert, None);
            self.device.destroy_shader_module(self.frag, None);
            self.device.destroy_framebuffer(self.framebuffer, None);
            self.device.destroy_image_view(self.view, None);
            self.device.destroy_render_pass(self.render_pass, None);
        }
    }
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
