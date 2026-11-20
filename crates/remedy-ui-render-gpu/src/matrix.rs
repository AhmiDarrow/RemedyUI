//! Colour-matrix groups on the GPU. vello runs no colour matrices, so a group with one is
//! rendered to a texture of its own, run through the matrix by the small pass below (the
//! same arithmetic as `ColorMatrix::apply_premultiplied`, which the CPU renderer runs), and
//! sampled back pixel for pixel by the scene it belongs to.

use remedy_ui_core::ColorMatrix;
use vello_gpu::{ImageSource, Scene, TextureId, TextureRegion};

use crate::renderer::FORMAT;
use crate::{GpuContext, GpuError};

const SHADER: &str = r"
struct Matrix {
    r: vec4<f32>,
    g: vec4<f32>,
    b: vec4<f32>,
    a: vec4<f32>,
    offset: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> m: Matrix;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(floor(p.xy)), 0);
    // Where the group drew nothing stays transparent.
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let u = vec4<f32>(min(c.rgb / c.a, vec3<f32>(1.0)), c.a);
    let o = clamp(
        vec4<f32>(dot(m.r, u), dot(m.g, u), dot(m.b, u), dot(m.a, u)) + m.offset,
        vec4<f32>(0.0),
        vec4<f32>(1.0),
    );
    return vec4<f32>(o.rgb * o.a, o.a);
}
";

/// The colour-matrix pass: made once per device, with the renderer's other pipelines.
pub(crate) struct MatrixPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl MatrixPass {
    pub(crate) fn new(device: &wgpu::Device, cache: Option<&wgpu::PipelineCache>) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RemedyUI colour matrix"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RemedyUI colour matrix"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
            label: Some("RemedyUI colour matrix"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RemedyUI colour matrix"),
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
        MatrixPass { pipeline, layout }
    }

    /// Renders `scene` with `vello` (whose target format is `format`) to a texture of the
    /// scene's size, and returns that picture run through `matrix` (premultiplied RGBA8).
    #[allow(clippy::too_many_arguments, reason = "a scene, the renderer drawing it, and the matrix")]
    pub(crate) fn picture(
        &self,
        ctx: &GpuContext,
        vello: &mut vello_gpu::Renderer,
        resources: &mut vello_gpu::Resources,
        scene: &Scene,
        format: wgpu::TextureFormat,
        bindings: &vello_gpu::TextureBindings,
        matrix: &ColorMatrix,
    ) -> Result<wgpu::Texture, GpuError> {
        let size = wgpu::Extent3d {
            width: u32::from(scene.width()),
            height: u32::from(scene.height()),
            depth_or_array_layers: 1,
        };
        let texture = |format, label| {
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let drawn = texture(format, "RemedyUI colour-matrix group");
        let view = drawn.create_view(&wgpu::TextureViewDescriptor::default());
        crate::renderer::submit_scene(
            ctx,
            vello,
            resources,
            scene,
            remedy_ui_core::Color::TRANSPARENT,
            &view,
            bindings,
        )?;
        let out = texture(FORMAT, "RemedyUI colour-matrix picture");
        // Four rows of weights, then the four offsets (the fifth column).
        let m = matrix.limited().0;
        let rows = m.as_chunks::<5>().0;
        let mut params = Vec::with_capacity(80);
        for row in rows {
            for v in row.iter().take(4) {
                params.extend(v.to_le_bytes());
            }
        }
        for row in rows {
            params.extend(row.get(4).copied().unwrap_or(0.0).to_le_bytes());
        }
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RemedyUI colour matrix"),
            size: params.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(&buffer, 0, &params);
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RemedyUI colour matrix"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: buffer.as_entire_binding() },
            ],
        });
        let target = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RemedyUI colour matrix"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RemedyUI colour matrix"),
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
        // Submitted now: the queue runs it before the scene that samples the picture.
        ctx.queue.submit([enc.finish()]);
        Ok(out)
    }
}

/// Paints `[x, y, w, h]` of `scene` (its own pixels, nothing transformed) with the texture
/// bound as `id`, one pixel for one.
pub(crate) fn place(scene: &mut Scene, id: TextureId, [x, y, w, h]: [u16; 4]) {
    use vello_gpu::kurbo;
    use vello_gpu::peniko::{ImageQuality, ImageSampler};
    let (x, y) = (f64::from(x), f64::from(y));
    scene.set_transform(kurbo::Affine::IDENTITY);
    scene.set_paint(vello_gpu::Image {
        image: ImageSource::external_texture(id, TextureRegion::Full, true),
        sampler: ImageSampler { quality: ImageQuality::Low, ..Default::default() },
    });
    scene.set_paint_transform(kurbo::Affine::translate((x, y)));
    scene.fill_rect(&kurbo::Rect::new(x, y, x + f64::from(w), y + f64::from(h)));
    scene.reset_paint_transform();
}
