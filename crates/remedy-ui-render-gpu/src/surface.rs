//! Presenting to a window: a swapchain surface with its own pipelines for its pixel format.

use std::sync::Arc;

use remedy_ui_compositor::FramePlan;
use remedy_ui_core::Color;
use remedy_ui_host::WindowHandles;

use crate::renderer::submit_scene;
use crate::{GpuCompositor, GpuContext, GpuError};

/// Formats RemedyUI composites into, best first. Both are plain 8-bit (blending happens in
/// sRGB space, as browsers and design tools do); the hardware swizzles BGRA.
const PREFERRED: [wgpu::TextureFormat; 2] =
    [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Bgra8Unorm];

/// What presenting a frame did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presented {
    /// The frame is on its way to the screen.
    Shown,
    /// The window is hidden or minimized; nothing was drawn. Try again when it is visible.
    Occluded,
    /// The swapchain was stale and has been rebuilt; draw the frame again.
    Retry,
}

/// A window's GPU surface.
pub struct GpuSurface {
    handles: Arc<dyn WindowHandles>,
    /// `None` only between dropping a dead swapchain and building its replacement.
    surface: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    vello: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    generation: u64,
    tiles: crate::composite::TileSet,
}

impl std::fmt::Debug for GpuSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuSurface({}x{} {:?})", self.config.width, self.config.height, self.config.format)
    }
}

fn build(
    ctx: &GpuContext,
    handles: &Arc<dyn WindowHandles>,
    width: u32,
    height: u32,
    transparent: bool,
) -> Result<
    (wgpu::Surface<'static>, wgpu::SurfaceConfiguration, vello_gpu::Renderer, vello_gpu::Resources),
    GpuError,
> {
    if let Some(bound) = ctx.bound_canvas
        && crate::context::canvas_id(&**handles) != Some(bound)
    {
        return Err(GpuError::Surface(
            "a WebGL2 device draws only on the canvas it was made for; this window stays on the CPU renderer"
                .into(),
        ));
    }
    let surface =
        ctx.instance.create_surface(handles.clone()).map_err(|e| GpuError::Surface(e.to_string()))?;
    let caps = surface.get_capabilities(&ctx.adapter);
    let format = PREFERRED
        .into_iter()
        .find(|f| caps.formats.contains(f))
        .ok_or_else(|| GpuError::Surface(format!("no 8-bit RGBA/BGRA format among {:?}", caps.formats)))?;
    let mut config = surface
        .get_default_config(&ctx.adapter, width.max(1), height.max(1))
        .ok_or_else(|| GpuError::Surface("the adapter cannot present to this window".into()))?;
    config.format = format;
    config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    // Vsync: frames pace to the display, the compositor's begin-frame source.
    config.present_mode = wgpu::PresentMode::AutoVsync;
    if transparent && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
        config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
    }
    surface.configure(&ctx.device, &config);
    let target = vello_gpu::RenderTargetConfig {
        format,
        width: u16::try_from(config.width).map_err(|_| GpuError::BadSize)?,
        height: u16::try_from(config.height).map_err(|_| GpuError::BadSize)?,
    };
    let (vello, resources) = vello_gpu::Renderer::new(&ctx.device, &ctx.queue, &target);
    Ok((surface, config, vello, resources))
}

impl GpuCompositor {
    /// A surface for a window from its native handles (from the bridge's
    /// `Host::window_handles`), `width` × `height` device pixels. A transparent window gets a
    /// premultiplied-alpha surface where the platform supports one.
    pub fn create_surface(
        &self,
        handles: Arc<dyn WindowHandles>,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<GpuSurface, GpuError> {
        let (surface, config, vello, resources) =
            build(self.renderer().context(), &handles, width, height, transparent)?;
        Ok(GpuSurface {
            handles,
            surface: Some(surface),
            config,
            vello,
            resources,
            generation: self.generation(),
            tiles: Default::default(),
        })
    }

    /// Resizes a surface after the window changed size.
    pub fn resize_surface(&self, surface: &mut GpuSurface, width: u32, height: u32) {
        surface.config.width = width.max(1);
        surface.config.height = height.max(1);
        if let Some(s) = &surface.surface {
            s.configure(&self.renderer().context().device, &surface.config);
        }
    }

    /// Rasters, composites and presents `plan` to a window. A surface made before a device
    /// loss is rebuilt on the new device here, with no app code.
    pub fn present_to_surface(
        &mut self,
        plan: &FramePlan,
        surface: &mut GpuSurface,
        background: Color,
    ) -> Result<Presented, GpuError> {
        if surface.generation != self.generation() || surface.surface.is_none() {
            // A window has one swapchain: the old one goes before the new one is made.
            surface.surface = None;
            let transparent = surface.config.alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;
            let (s, c, v, r) = build(
                self.renderer().context(),
                &surface.handles,
                surface.config.width,
                surface.config.height,
                transparent,
            )?;
            (surface.surface, surface.config, surface.vello, surface.resources) = (Some(s), c, v, r);
            surface.tiles.0.clear();
            surface.generation = self.generation();
        }
        let Some(swapchain) = surface.surface.as_ref() else { return Ok(Presented::Retry) };
        // Waiting for the swapchain to hand out an image is part of presenting.
        let acquire = remedy_ui_trace::phase!("present").entered();
        let frame = match swapchain.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => {
                return Ok(Presented::Occluded);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize_surface(surface, surface.config.width, surface.config.height);
                return Ok(Presented::Retry);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                surface.surface = None;
                return Ok(Presented::Retry);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(GpuError::Surface("the swapchain rejected the frame".into()));
            }
        };
        drop(acquire);
        let (w, h) = (frame.texture.width(), frame.texture.height());
        let raster = remedy_ui_trace::phase!("raster").entered();
        let (scene, bindings) = self.prepare(&mut surface.tiles, plan, w, h)?;
        drop(raster);
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        submit_scene(
            self.renderer().context(),
            &mut surface.vello,
            &mut surface.resources,
            &scene,
            background,
            &view,
            &bindings,
        )?;
        let _present = remedy_ui_trace::phase!("present").entered();
        self.renderer().context().queue.present(frame);
        Ok(Presented::Shown)
    }
}
