//! What the GPU embed tests share: a triangle the app renders with its own pipeline, and a
//! window's frame pipeline composed by the GPU or CPU renderer.
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::Compositor;
use remedy_ui_core::{Color, Size};
use remedy_ui_embed_gpu::*;
use remedy_ui_input::ScrollPhysics;
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_render_gpu::{GpuCompositor, GpuContext, GpuRenderer, LadderOptions, wgpu};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};

const SHADER: &str = r"
struct U { angle: f32, a: f32, b: f32, c: f32 };
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(0.0, 0.8), vec2<f32>(-0.7, -0.6), vec2<f32>(0.7, -0.6));
    let v = p[i];
    let c = cos(u.angle);
    let s = sin(u.angle);
    return vec4<f32>(v.x * c - v.y * s, v.x * s + v.y * c, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
";

/// What the test sees of the content.
#[derive(Default)]
pub struct Seen {
    pub prepared: Cell<u32>,
    pub sizes: RefCell<Vec<(u32, u32)>>,
    pub inputs: RefCell<Vec<EmbedInput>>,
}

/// A red triangle on blue, turned by `angle` (radians).
pub struct Triangle {
    pub angle: Rc<Cell<f32>>,
    pub seen: Rc<Seen>,
    pub pipeline: Option<(wgpu::RenderPipeline, wgpu::Buffer, wgpu::BindGroup)>,
}

impl GpuContent for Triangle {
    fn prepare(&mut self, gpu: &Gpu<'_>) -> Result<(), remedy_ui_embed_gpu::PrepareError> {
        self.seen.prepared.set(self.seen.prepared.get() + 1);
        let d = gpu.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("triangle"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let buffer = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("angle"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("triangle"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipeline = Some((pipeline, buffer, group));
        Ok(())
    }

    fn render(&mut self, gpu: &Gpu<'_>, target: &Target<'_>, encoder: &mut wgpu::CommandEncoder) {
        self.seen.sizes.borrow_mut().push((target.width, target.height));
        let Some((pipeline, buffer, group)) = &self.pipeline else { return };
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&self.angle.get().to_le_bytes());
        gpu.queue.write_buffer(buffer, 0, &bytes);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("triangle"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn input(&mut self, input: &EmbedInput) -> bool {
        self.seen.inputs.borrow_mut().push(input.clone());
        true
    }
}

pub fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

pub enum Renderer {
    Gpu(Box<GpuCompositor>),
    Cpu(Box<CpuCompositor>),
}

pub struct Screen {
    pub ui: Ui,
    pub comp: Compositor,
    pub renderer: Renderer,
    pub size: Size,
    pub t: u64,
    pub last: Option<Frame>,
}

impl Screen {
    pub fn new(ui: Ui, gpu: bool) -> Self {
        let renderer = if gpu {
            let ladder = LadderOptions::default();
            let ctx = GpuContext::shared(&ladder).unwrap_or_else(|e| unreachable!("a GPU device: {e}"));
            Renderer::Gpu(Box::new(GpuCompositor::new(GpuRenderer::new(ctx, None), ladder)))
        } else {
            Renderer::Cpu(Box::new(CpuCompositor::new()))
        };
        ui.set_gpu_rendering(gpu);
        Screen {
            ui,
            comp: Compositor::new(200, 200, 1.0, ScrollPhysics::CLAMPED),
            renderer,
            size: Size::new(200.0, 200.0),
            t: 0,
            last: None,
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = Size::new(w, h);
        self.comp.resize(w as u32, h as u32, 1.0);
    }

    pub fn frame(&mut self) {
        let f = self.ui.frame(self.size, 1.0);
        if let Some(l) = f.layers {
            self.comp.commit(l);
        }
        self.t += 16;
        let plan = self.comp.frame(Duration::from_millis(self.t));
        let (w, h) = (self.size.width as u32, self.size.height as u32);
        let frame = match &mut self.renderer {
            Renderer::Gpu(g) => g.present_to_frame(&plan, w, h, Color::WHITE).map_err(|e| e.to_string()),
            Renderer::Cpu(c) => c.present(&plan, w, h, Color::WHITE).map_err(|e| e.to_string()),
        };
        self.last = Some(frame.unwrap_or_else(|e| unreachable!("a frame: {e}")));
    }

    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        self.last.as_ref().and_then(|f| f.pixel(x, y)).unwrap_or([0; 4])
    }
}

pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];

pub fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 8)
}

pub fn triangle(gpu: bool) -> (Screen, El, GpuView, Rc<Cell<f32>>, Rc<Seen>) {
    let ui = ui();
    let angle = Rc::new(Cell::new(0.0f32));
    let seen = Rc::new(Seen::default());
    let content = Triangle { angle: angle.clone(), seen: seen.clone(), pipeline: None };
    let (el, view) = gpu_view(&ui, "Spinning triangle", content);
    ui.root().child(el.clone());
    let mut s = Screen::new(ui, gpu);
    s.frame();
    (s, el, view, angle, seen)
}
