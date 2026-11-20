//! Presenting to a window: a swapchain surface with its own pipelines for its pixel format.

use std::sync::Arc;

use remedy_ui_compositor::FramePlan;
use remedy_ui_core::Color;
use remedy_ui_host::WindowHandles;
use remedy_ui_render_cpu::Frame;

use crate::renderer::submit_scene;
use crate::{GpuCompositor, GpuContext, GpuError};

/// Formats RemedyUI composites into, best first. Both are plain 8-bit (blending happens in
/// sRGB space, as browsers and design tools do); the hardware swizzles BGRA.
const PREFERRED: [wgpu::TextureFormat; 2] =
    [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Bgra8Unorm];

/// What presenting a frame did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The window's frame, kept between presents: each present draws only the plan's damage
    /// into it, then shows it. Also what a snapshot of the window reads.
    frame: Option<wgpu::Texture>,
    /// The swapchain takes copies (otherwise the frame is drawn onto it).
    copies: bool,
}

impl GpuSurface {
    /// Drops the swapchain as the platform does when it reports it lost (a display mode
    /// change, a monitor unplugged); the next present builds another. For tests.
    #[doc(hidden)]
    pub fn lose_swapchain(&mut self) {
        self.surface = None;
    }
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
    vsync: bool,
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
    // The frame is kept in a texture of its own and copied to the swapchain image, where the
    // swapchain allows copies.
    if caps.usages.contains(wgpu::TextureUsages::COPY_DST) {
        config.usage |= wgpu::TextureUsages::COPY_DST;
    }
    // Vsync: frames pace to the display, the compositor's begin-frame source. Without it
    // frames show as soon as they are drawn (lower latency; a fast scroll may tear).
    config.present_mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
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
        self.create_surface_paced(handles, width, height, transparent, true)
    }

    /// [`GpuCompositor::create_surface`], presenting in step with the display (`vsync`) or
    /// as soon as each frame is drawn.
    pub fn create_surface_paced(
        &self,
        handles: Arc<dyn WindowHandles>,
        width: u32,
        height: u32,
        transparent: bool,
        vsync: bool,
    ) -> Result<GpuSurface, GpuError> {
        let (surface, config, vello, resources) =
            build(self.renderer().context(), &handles, width, height, transparent, vsync)?;
        let copies = config.usage.contains(wgpu::TextureUsages::COPY_DST);
        Ok(GpuSurface {
            handles,
            surface: Some(surface),
            config,
            vello,
            resources,
            generation: self.generation(),
            tiles: Default::default(),
            frame: None,
            copies,
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
        // Watched: a present that hangs lets the device go (see `health`), and the next frame
        // finds it lost and recovers down the ladder.
        let watchdog = self.watchdog().clone();
        let done = watchdog.watch(|| self.present_to_surface_unwatched(plan, surface, background));
        self.after_present();
        done
    }

    fn present_to_surface_unwatched(
        &mut self,
        plan: &FramePlan,
        surface: &mut GpuSurface,
        background: Color,
    ) -> Result<Presented, GpuError> {
        if surface.generation != self.generation() || surface.surface.is_none() {
            // A window has one swapchain: the old one goes before the new one is made.
            surface.surface = None;
            let transparent = surface.config.alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;
            let vsync = surface.config.present_mode != wgpu::PresentMode::AutoNoVsync;
            let (s, c, v, r) = build(
                self.renderer().context(),
                &surface.handles,
                surface.config.width,
                surface.config.height,
                transparent,
                vsync,
            )?;
            (surface.surface, surface.config, surface.vello, surface.resources) = (Some(s), c, v, r);
            surface.copies = surface.config.usage.contains(wgpu::TextureUsages::COPY_DST);
            // Tiles and the kept frame are device textures: a lost swapchain on a living
            // device keeps them (the runner re-rasters only after a device change, so tiles
            // dropped here would stay missing from the window).
            if surface.generation != self.generation() {
                surface.tiles.0.clear();
                surface.frame = None;
            }
            surface.generation = self.generation();
        }
        // The frame first: drawn even when the swapchain has no image to give now, so no
        // damage is lost and the next present shows it.
        let raster = remedy_ui_trace::phase!("raster").entered();
        let size = (surface.config.width, surface.config.height);
        self.render_damage(
            &mut surface.tiles,
            plan,
            &mut surface.frame,
            surface.config.format,
            size,
            background,
            Some((&mut surface.vello, &mut surface.resources)),
        )?;
        drop(raster);
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
        let _present = remedy_ui_trace::phase!("present").entered();
        let Some(kept) = surface.frame.as_ref() else { return Ok(Presented::Retry) };
        // wgpu presents whole images (it has no present with damage), so the kept frame is
        // shown whole: one copy, or one draw where the swapchain takes no copies.
        let (w, h) = (frame.texture.width().min(kept.width()), frame.texture.height().min(kept.height()));
        let ctx = self.renderer().context();
        if surface.copies && frame.texture.format() == kept.format() {
            let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("RemedyUI show frame"),
            });
            encoder.copy_texture_to_texture(
                kept.as_image_copy(),
                frame.texture.as_image_copy(),
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            ctx.queue.submit([encoder.finish()]);
        } else {
            let (sw, sh) = crate::renderer::size16(w, h)?;
            let mut scene = vello_gpu::Scene::new(sw, sh);
            let id = vello_gpu::TextureId(u64::MAX);
            let mut bindings = vello_gpu::TextureBindings::new();
            bindings.insert(id, kept.create_view(&wgpu::TextureViewDescriptor::default()));
            scene.set_paint(vello_gpu::Image {
                image: vello_gpu::ImageSource::external_texture(id, vello_gpu::TextureRegion::Full, true),
                sampler: vello_gpu::peniko::ImageSampler {
                    quality: vello_gpu::peniko::ImageQuality::Low,
                    ..Default::default()
                },
            });
            scene.fill_rect(&vello_gpu::kurbo::Rect::new(0.0, 0.0, f64::from(sw), f64::from(sh)));
            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            submit_scene(
                ctx,
                &mut surface.vello,
                &mut surface.resources,
                &scene,
                Color::TRANSPARENT,
                &view,
                &bindings,
            )?;
        }
        ctx.queue.present(frame);
        Ok(Presented::Shown)
    }

    /// A picture of what the window shows: the frame the latest present drew, read back from
    /// the GPU, effects, video and app textures included (premultiplied RGBA). `None` before the
    /// first present.
    pub fn read_surface(&self, surface: &GpuSurface) -> Option<Result<Frame, GpuError>> {
        let kept = surface.frame.as_ref()?;
        let mut frame = match self.renderer().context().read_back(kept) {
            Ok(f) => f,
            Err(e) => return Some(Err(e)),
        };
        if matches!(kept.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for px in frame.pixels.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        Some(Ok(frame))
    }
}
