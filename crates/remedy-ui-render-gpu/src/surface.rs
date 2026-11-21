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
    /// Rectangles drawn into `frame` since the latest present, `(x0, y0, x1, y1)`.
    pending: Vec<(u16, u16, u16, u16)>,
    /// What each present changed, the latest first: a swapchain image `n` presents old needs
    /// `pending` and the first `n - 1` of these copied in.
    history: std::collections::VecDeque<Vec<(u16, u16, u16, u16)>>,
    /// Tests: what [`GpuSurface::check_presents`] found.
    checks: Option<PresentChecks>,
}

/// What [`GpuSurface::check_presents`] found. For tests.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentChecks {
    /// Presents whose swapchain image was compared with the window's frame.
    pub compared: usize,
    /// Presents that copied in only what changed (the image kept an older frame).
    pub damaged: usize,
    /// Presents whose image held nothing known (age `0`: new to the swapchain, after a
    /// resize, or a backend that does not keep contents), so all of it was drawn.
    pub unknown: usize,
    /// Presents whose image differed from the window's frame anywhere.
    pub mismatched: usize,
}

impl GpuSurface {
    /// Drops the swapchain as the platform does when it reports it lost (a display mode
    /// change, a monitor unplugged); the next present builds another. For tests.
    #[doc(hidden)]
    pub fn lose_swapchain(&mut self) {
        self.surface = None;
    }

    /// From now on reads every swapchain image back before it is presented and compares it
    /// with the window's frame, pixel for pixel. Slow; for tests. Takes effect where the
    /// swapchain can be read (the next time it is built or resized).
    #[doc(hidden)]
    pub fn check_presents(&mut self) {
        self.checks.get_or_insert_default();
        self.config.usage |= wgpu::TextureUsages::COPY_SRC;
    }

    /// What [`check_presents`](Self::check_presents) found so far.
    #[doc(hidden)]
    pub fn present_checks(&self) -> PresentChecks {
        self.checks.unwrap_or_default()
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
    // Swapchain images keep what they showed, so a present copies in and reports only what
    // changed since ([`GpuSurface`]'s history): the compositor redraws only that on screen.
    config.preserve_contents = true;
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
            pending: Vec::new(),
            history: Default::default(),
            checks: None,
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
        let drawn = self.render_damage(
            &mut surface.tiles,
            plan,
            &mut surface.frame,
            surface.config.format,
            size,
            background,
            Some((&mut surface.vello, &mut surface.resources)),
        )?;
        surface.pending.extend(drawn);
        // Frames drawn while nothing is shown (an occluded window) pile up: past a few, one
        // rectangle around them all costs less to copy than the list (a tuning constant).
        if surface.pending.len() > 32
            && let Some(all) = surface
                .pending
                .iter()
                .copied()
                .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        {
            surface.pending = vec![all];
        }
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
        let (w, h) = (frame.texture.width().min(kept.width()), frame.texture.height().min(kept.height()));
        let ctx = self.renderer().context();
        let copies = surface.copies && frame.texture.format() == kept.format();
        // A swapchain image that kept what it showed `age` presents ago needs only what
        // changed since: this frame's rectangles and those of the presents in between. The
        // presentation engine is told the same rectangles, and updates only them on screen.
        let age = frame.buffer_age() as usize;
        let changed: Option<Vec<(u16, u16, u16, u16)>> =
            (copies && age > 0 && surface.history.len() + 1 >= age).then(|| {
                surface
                    .pending
                    .iter()
                    .chain(surface.history.iter().take(age - 1).flatten())
                    .copied()
                    .collect()
            });
        if let Some(changed) = &changed {
            let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("RemedyUI show damage"),
            });
            for &(x0, y0, x1, y1) in changed {
                let (x1, y1) = (u32::from(x1).min(w), u32::from(y1).min(h));
                let (x0, y0) = (u32::from(x0), u32::from(y0));
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let origin = wgpu::Origin3d { x: x0, y: y0, z: 0 };
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { origin, ..kept.as_image_copy() },
                    wgpu::TexelCopyTextureInfo { origin, ..frame.texture.as_image_copy() },
                    wgpu::Extent3d { width: x1 - x0, height: y1 - y0, depth_or_array_layers: 1 },
                );
            }
            ctx.queue.submit([encoder.finish()]);
        } else if copies {
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
        if let Some(checks) = surface.checks.as_mut()
            && frame.texture.usage().contains(wgpu::TextureUsages::COPY_SRC)
        {
            let (shown, want) = (ctx.read_back(&frame.texture)?, ctx.read_back(kept)?);
            checks.compared += 1;
            checks.damaged += usize::from(changed.is_some());
            checks.unknown += usize::from(age == 0);
            checks.mismatched += usize::from(shown.pixels != want.pixels);
        }
        match changed {
            Some(changed) => {
                let damage: Vec<wgpu::DamageRect> = changed
                    .iter()
                    .map(|&(x0, y0, x1, y1)| wgpu::DamageRect {
                        x: u32::from(x0),
                        y: u32::from(y0),
                        width: u32::from(x1 - x0),
                        height: u32::from(y1 - y0),
                    })
                    .collect();
                ctx.queue.present_with_damage(frame, &damage);
            }
            None => ctx.queue.present(frame),
        }
        surface.history.push_front(std::mem::take(&mut surface.pending));
        // A swapchain has at most 16 images (DXGI's limit; presentation engines keep fewer):
        // an image older than that is drawn whole.
        surface.history.truncate(16);
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
