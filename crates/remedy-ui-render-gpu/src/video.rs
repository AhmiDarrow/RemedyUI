//! Video frames on the GPU: each frame's planes are uploaded as they are (Y, UV or U and V,
//! or RGBA) and a small render pass converts them into one RGBA texture per stream, which the
//! scene samples like any image. The conversion pipeline is made with the renderer, never
//! mid-frame.
//!
//! Pictures already on the GPU are sampled where they are: a decoder's shared texture
//! (Windows), or a browser's `VideoFrame`, which is put into the stream's texture by the
//! browser itself (`ffi::video::Web`).

use std::collections::HashMap;

use remedy_ui_paint::{Coefficients, PixelFormat, VideoFrame};
use vello_gpu::TextureId;

use crate::renderer::FORMAT;

const SHADER: &str = r"
struct Params {
    kind: u32,
    y_offset: f32,
    y_scale: f32,
    c_scale: f32,
    rv: f32,
    gu: f32,
    gv: f32,
    bu: f32,
};

@group(0) @binding(0) var p0: texture_2d<f32>;
@group(0) @binding(1) var p1: texture_2d<f32>;
@group(0) @binding(2) var p2: texture_2d<f32>;
@group(0) @binding(3) var<uniform> k: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the target.
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    if (k.kind == 0u) {
        let c = textureLoad(p0, at, 0);
        return vec4<f32>(c.rgb * c.a, c.a);
    }
    let half = at / 2;
    let y = textureLoad(p0, at, 0).r * 255.0;
    var u: f32;
    var v: f32;
    if (k.kind == 1u) {
        let uv = textureLoad(p1, half, 0).rg * 255.0;
        u = uv.x;
        v = uv.y;
    } else {
        u = textureLoad(p1, half, 0).r * 255.0;
        v = textureLoad(p2, half, 0).r * 255.0;
    }
    let l = (y - k.y_offset) * k.y_scale;
    let cb = (u - 128.0) * k.c_scale;
    let cr = (v - 128.0) * k.c_scale;
    let rgb = vec3<f32>(l + k.rv * cr, l + k.gu * cb + k.gv * cr, l + k.bu * cb) / 255.0;
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
";

/// A stream's textures: its planes as uploaded and the converted picture.
struct Stream {
    frame: u64,
    size: (u32, u32),
    format: PixelFormat,
    planes: Vec<wgpu::Texture>,
    target: wgpu::Texture,
    id: TextureId,
    params: wgpu::Buffer,
    /// Pictures on the GPU opened on this device, and those still being read.
    shared: crate::ffi::video::Shared,
}

/// A stream's picture this frame: its texture id, a view to bind when the texture is new, and
/// the id of a texture it replaced (the frame changed size), to unbind.
pub(crate) struct Shown {
    pub(crate) id: TextureId,
    pub(crate) new_view: Option<wgpu::TextureView>,
    pub(crate) replaced: Option<TextureId>,
}

/// The conversion pipeline and each stream's textures.
pub(crate) struct GpuVideos {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    streams: HashMap<u64, Stream>,
    /// A browser's frames put into a stream's texture where they are.
    web: crate::ffi::video::Web,
}

fn plane_format(format: PixelFormat, i: usize) -> wgpu::TextureFormat {
    match (format, i) {
        (PixelFormat::Rgba, _) => wgpu::TextureFormat::Rgba8Unorm,
        (PixelFormat::Nv12, 1) => wgpu::TextureFormat::Rg8Unorm,
        _ => wgpu::TextureFormat::R8Unorm,
    }
}

fn mk_texture(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Clears `target` through wgpu, so wgpu counts it as written and never clears it itself
/// after something outside wgpu has written it.
fn clear(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI video clear") });
    drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("RemedyUI video clear"),
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
    }));
    queue.submit([enc.finish()]);
}

/// Uploads each plane straight from the frame's bytes, its stride as is.
fn upload(queue: &wgpu::Queue, frame: &VideoFrame, planes: &[wgpu::Texture]) -> Option<()> {
    let sizes = frame.format.plane_sizes(frame.width, frame.height);
    for (i, ((w, h, _), t)) in sizes.into_iter().zip(planes).enumerate() {
        let p = frame.planes.get(i).copied().unwrap_or_default();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.data,
            wgpu::TexelCopyBufferLayout {
                offset: p.offset as u64,
                bytes_per_row: Some(u32::try_from(p.stride).ok()?),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
    Some(())
}

impl GpuVideos {
    pub(crate) fn new(device: &wgpu::Device, cache: Option<&wgpu::PipelineCache>) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RemedyUI video"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RemedyUI video"),
            entries: &[
                tex(0),
                tex(1),
                tex(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RemedyUI video"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RemedyUI video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache,
        });
        let web = crate::ffi::video::Web::new(device);
        GpuVideos { pipeline, layout, streams: HashMap::new(), web }
    }

    /// The texture showing `frame`, converted on the GPU (once per frame), and the id to bind
    /// it under. `fresh` makes a new texture id; `None` when the device cannot hold the frame.
    pub(crate) fn texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &VideoFrame,
        fresh: &mut dyn FnMut() -> TextureId,
    ) -> Option<Shown> {
        if !frame.is_valid() {
            return None;
        }
        let max = device.limits().max_texture_dimension_2d;
        if frame.width > max || frame.height > max {
            return None;
        }
        let size = (frame.width, frame.height);
        let reuse =
            self.streams.get(&frame.stream).is_some_and(|s| s.size == size && s.format == frame.format);
        let mut new_view = None;
        let mut replaced = None;
        if !reuse {
            replaced = self.streams.get(&frame.stream).map(|s| s.id);
            let target = mk_texture(
                device,
                frame.width,
                frame.height,
                FORMAT,
                // A browser's frames may be copied in (`ffi::video::Web`).
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST,
                "RemedyUI video frame",
            );
            let params = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RemedyUI video"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let id = fresh();
            new_view = Some(target.create_view(&wgpu::TextureViewDescriptor::default()));
            if frame.gpu.is_some() {
                clear(device, queue, &target);
            }
            self.streams.insert(
                frame.stream,
                Stream {
                    frame: u64::MAX,
                    size,
                    format: frame.format,
                    planes: Vec::new(),
                    target,
                    id,
                    params,
                    shared: crate::ffi::video::Shared::default(),
                },
            );
        }
        let s = self.streams.get_mut(&frame.stream)?;
        if s.frame != frame.id {
            s.frame = frame.id;
            // A browser's frame goes into the texture where it is, submitted at once (it is
            // good only until the decoder closes it; work already submitted is not affected).
            if let Some(g) = &frame.gpu
                && self.web.draw(device, &s.target, g)
            {
                return Some(Shown { id: s.id, new_view, replaced });
            }
            // Android: a picture on the GPU is drawn into the target by a pass of the platform's.
            #[cfg(target_os = "android")]
            if frame.gpu.as_ref().is_some_and(|g| {
                s.shared.convert(device, queue, g, frame.matrix, frame.full_range, &s.target)
            }) {
                return Some(Shown { id: s.id, new_view, replaced });
            }
            // A picture on the GPU is sampled where it is when this device can open it.
            let opened = frame.gpu.as_ref().and_then(|g| s.shared.open(device, g));
            let in_place = opened.is_some();
            let views: Vec<wgpu::TextureView> = match opened {
                Some(views) => views,
                None => {
                    // Otherwise its bytes are uploaded, read back first when it is on the GPU.
                    let cpu = frame.in_cpu_memory()?;
                    if s.planes.is_empty() {
                        s.planes = cpu
                            .format
                            .plane_sizes(cpu.width, cpu.height)
                            .into_iter()
                            .enumerate()
                            .map(|(i, (w, h, _))| {
                                mk_texture(
                                    device,
                                    w,
                                    h,
                                    plane_format(cpu.format, i),
                                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                                    "RemedyUI video plane",
                                )
                            })
                            .collect();
                    }
                    upload(queue, &cpu, &s.planes)?;

                    s.planes.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect()
                }
            };
            let k = Coefficients::new(frame.matrix, frame.full_range);
            let kind: u32 = match frame.format {
                PixelFormat::Rgba => 0,
                PixelFormat::Nv12 => 1,
                PixelFormat::I420 => 2,
            };
            let mut params = Vec::with_capacity(32);
            params.extend(kind.to_le_bytes());
            for v in [k.y_offset, k.y_scale, k.c_scale, k.rv, k.gu, k.gv, k.bu] {
                params.extend(v.to_le_bytes());
            }
            queue.write_buffer(&s.params, 0, &params);
            // Unused bindings take the first plane (the shader never reads them).
            let view = |i: usize| views.get(i).or(views.first());
            let (v0, v1, v2) = (view(0)?, view(1)?, view(2)?);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RemedyUI video"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v0) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(v1) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(v2) },
                    wgpu::BindGroupEntry { binding: 3, resource: s.params.as_entire_binding() },
                ],
            });
            let target = s.target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut enc = device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI video") });
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("RemedyUI video"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
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
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            // Submitted now: the queue runs it before the frame that samples the result. A
            // shared picture's submission also waits for the decoder and hands it back after.
            match &frame.gpu {
                Some(g) if in_place => s.shared.submit(queue, enc.finish(), g),
                _ => {
                    queue.submit([enc.finish()]);
                }
            }
        }
        Some(Shown { id: s.id, new_view, replaced })
    }

    /// Frees streams whose frames are all gone; returns their texture ids to unbind.
    pub(crate) fn evict(&mut self, streams: &[u64]) -> Vec<TextureId> {
        streams.iter().filter_map(|s| self.streams.remove(s)).map(|s| s.id).collect()
    }

    /// Drops every stream's textures (memory pressure).
    pub(crate) fn clear(&mut self) -> Vec<TextureId> {
        self.streams.drain().map(|(_, s)| s.id).collect()
    }

    /// Bytes of GPU memory held.
    pub(crate) fn bytes(&self) -> u64 {
        self.streams
            .values()
            .map(|s| {
                let (w, h) = (u64::from(s.size.0), u64::from(s.size.1));
                // The converted picture plus planes that average 1.5 bytes a pixel (4 for RGBA),
                // made only when pictures are uploaded rather than sampled where they are.
                let planes = if s.planes.is_empty() {
                    0
                } else if s.format == PixelFormat::Rgba {
                    w * h * 4
                } else {
                    w * h * 3 / 2
                };
                w * h * 4 + planes
            })
            .sum()
    }
}
