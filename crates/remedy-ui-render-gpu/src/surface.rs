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

/// What presents a window's frames.
enum Swap {
    /// wgpu's surface.
    Wgpu(wgpu::Surface<'static>),
    /// RemedyUI's own swapchain on wgpu's D3D12 device, which presents only what changed.
    #[cfg(windows)]
    Dxgi(crate::ffi::dxgi::DxgiChain),
    /// RemedyUI's own swapchain on wgpu's Vulkan device, which presents only what changed.
    #[cfg(any(
        all(unix, not(any(target_os = "macos", target_os = "ios"))),
        all(windows, feature = "windows-vulkan")
    ))]
    Vulkan(Box<crate::ffi::vulkan::VkChain>),
}

/// A window's GPU surface.
pub struct GpuSurface {
    handles: Arc<dyn WindowHandles>,
    /// `None` only between dropping a dead swapchain and building its replacement.
    surface: Option<Swap>,
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

    /// Whether the window presents through RemedyUI's own swapchain (D3D12) rather than
    /// wgpu's surface. For tests.
    #[doc(hidden)]
    pub fn own_swapchain(&self) -> bool {
        match &self.surface {
            #[cfg(windows)]
            Some(Swap::Dxgi(_)) => true,
            #[cfg(any(
                all(unix, not(any(target_os = "macos", target_os = "ios"))),
                all(windows, feature = "windows-vulkan")
            ))]
            Some(Swap::Vulkan(_)) => true,
            _ => false,
        }
    }

    /// Whether presents tell the system which rectangles changed (so it composites only
    /// them). For tests.
    #[doc(hidden)]
    pub fn reports_damage(&self) -> bool {
        match &self.surface {
            #[cfg(windows)]
            Some(Swap::Dxgi(_)) => true,
            #[cfg(any(
                all(unix, not(any(target_os = "macos", target_os = "ios"))),
                all(windows, feature = "windows-vulkan")
            ))]
            Some(Swap::Vulkan(chain)) => chain.reports_damage(),
            _ => false,
        }
    }

    /// Stops what [`check_presents`](Self::check_presents) started and returns what it found:
    /// later presents go as an app's do (a read back changes the image's last use). For tests.
    #[doc(hidden)]
    pub fn stop_checking_presents(&mut self) -> PresentChecks {
        self.checks.take().unwrap_or_default()
    }

    /// What [`check_presents`](Self::check_presents) found so far.
    #[doc(hidden)]
    pub fn present_checks(&self) -> PresentChecks {
        self.checks.unwrap_or_default()
    }
}

impl GpuSurface {
    /// What a swapchain image that showed a frame `age` presents ago needs copied in: this
    /// frame's rectangles and those of the presents in between. `None` when the image holds
    /// nothing known (age `0`, or older than the history kept): all of it. Only this window's
    /// own swapchains keep what they showed.
    #[cfg(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios")))))]
    fn changed_since(&self, age: usize) -> Option<Vec<(u16, u16, u16, u16)>> {
        (age > 0 && self.history.len() + 1 >= age).then(|| {
            self.pending.iter().chain(self.history.iter().take(age - 1).flatten()).copied().collect()
        })
    }
}

/// Records copying `changed` (or, for `None`, all of it) from the window's frame into a
/// swapchain image. `None` when there is nothing to copy.
fn copy_in(
    ctx: &GpuContext,
    kept: &wgpu::Texture,
    image: &wgpu::Texture,
    changed: Option<&[(u16, u16, u16, u16)]>,
    (w, h): (u32, u32),
) -> Option<wgpu::CommandBuffer> {
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI show frame") });
    let mut copied = false;
    match changed {
        Some(changed) => {
            for &(x0, y0, x1, y1) in changed {
                let (x1, y1) = (u32::from(x1).min(w), u32::from(y1).min(h));
                let (x0, y0) = (u32::from(x0), u32::from(y0));
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let origin = wgpu::Origin3d { x: x0, y: y0, z: 0 };
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { origin, ..kept.as_image_copy() },
                    wgpu::TexelCopyTextureInfo { origin, ..image.as_image_copy() },
                    wgpu::Extent3d { width: x1 - x0, height: y1 - y0, depth_or_array_layers: 1 },
                );
                copied = true;
            }
        }
        None => {
            encoder.copy_texture_to_texture(
                kept.as_image_copy(),
                image.as_image_copy(),
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            copied = true;
        }
    }
    copied.then(|| encoder.finish())
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
) -> Result<(Swap, wgpu::SurfaceConfiguration, vello_gpu::Renderer, vello_gpu::Resources), GpuError> {
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
    let premultiplied = transparent && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied);
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
    // On D3D12 a window opaque to the system gets a swapchain of RemedyUI's own, which
    // presents only what changed; wgpu's surface (never configured, so it made no swapchain)
    // goes first, as a window has one swapchain.
    #[cfg(windows)]
    let swap = match (!premultiplied)
        .then(|| {
            crate::ffi::dxgi::DxgiChain::new(
                &ctx.device,
                &**handles,
                config.width,
                config.height,
                format,
                vsync,
            )
        })
        .flatten()
    {
        Some(made) => {
            drop(surface);
            let chain = made.map_err(chain_error)?;
            config.usage |= wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
            Swap::Dxgi(chain)
        }
        None => own_vulkan_or_wgpu(ctx, surface, &mut config, premultiplied, format, vsync)?,
    };
    #[cfg(not(windows))]
    let swap = own_vulkan_or_wgpu(ctx, surface, &mut config, premultiplied, format, vsync)?;
    let target = vello_gpu::RenderTargetConfig {
        format,
        width: u16::try_from(config.width).map_err(|_| GpuError::BadSize)?,
        height: u16::try_from(config.height).map_err(|_| GpuError::BadSize)?,
    };
    let (vello, resources) = vello_gpu::Renderer::new(&ctx.device, &ctx.queue, &target);
    Ok((swap, config, vello, resources))
}

/// On Vulkan, a window opaque to the system gets a swapchain of RemedyUI's own on wgpu's
/// surface; otherwise wgpu's surface is configured.
fn own_vulkan_or_wgpu(
    ctx: &GpuContext,
    surface: wgpu::Surface<'static>,
    config: &mut wgpu::SurfaceConfiguration,
    premultiplied: bool,
    format: wgpu::TextureFormat,
    vsync: bool,
) -> Result<Swap, GpuError> {
    #[cfg(any(
        all(unix, not(any(target_os = "macos", target_os = "ios"))),
        all(windows, feature = "windows-vulkan")
    ))]
    let surface = if premultiplied {
        surface
    } else {
        match crate::ffi::vulkan::VkChain::new(
            &ctx.device,
            surface,
            config.width,
            config.height,
            format,
            vsync,
            ctx.incremental_present,
        ) {
            Ok(made) => {
                let chain = made.map_err(vk_error)?;
                config.usage |= wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
                return Ok(Swap::Vulkan(Box::new(chain)));
            }
            Err(surface) => surface,
        }
    };
    let _ = (premultiplied, format, vsync);
    surface.configure(&ctx.device, config);
    Ok(Swap::Wgpu(surface))
}

#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios"))),
    all(windows, feature = "windows-vulkan")
))]
fn vk_error(e: crate::ffi::vulkan::ChainError) -> GpuError {
    match e {
        crate::ffi::vulkan::ChainError::Lost => GpuError::Lost,
        crate::ffi::vulkan::ChainError::SurfaceLost => {
            GpuError::Surface("the window's surface was lost".into())
        }
        crate::ffi::vulkan::ChainError::OutOfDate => GpuError::Surface("the swapchain no longer fits".into()),
        crate::ffi::vulkan::ChainError::Failed(why) => GpuError::Surface(why),
    }
}

/// How a window's own swapchain failed.
#[cfg(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios")))))]
enum Failure {
    /// The device is lost.
    Device,
    /// The window's surface is lost, or the swapchain no longer fits it (Vulkan).
    #[cfg_attr(all(windows, not(feature = "windows-vulkan")), allow(dead_code))]
    Surface,
    /// Anything else.
    Other(GpuError),
}

/// A window's own swapchain failed presenting: it is dropped, and the next present builds
/// another. A lost device is marked lost, so the next frame recovers it as when wgpu finds a
/// loss; only other failures are errors, which move the window to the CPU renderer.
#[cfg(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios")))))]
fn chain_failed(ctx: &GpuContext, surface: &mut GpuSurface, failure: Failure) -> Result<Presented, GpuError> {
    surface.surface = None;
    match failure {
        Failure::Device => {
            ctx.mark_lost();
            Ok(Presented::Retry)
        }
        Failure::Surface => Ok(Presented::Retry),
        Failure::Other(e) => Err(e),
    }
}

#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios"))),
    all(windows, feature = "windows-vulkan")
))]
fn vk_failed(
    ctx: &GpuContext,
    surface: &mut GpuSurface,
    e: crate::ffi::vulkan::ChainError,
) -> Result<Presented, GpuError> {
    use crate::ffi::vulkan::ChainError;
    let failure = match e {
        ChainError::Lost => Failure::Device,
        ChainError::SurfaceLost | ChainError::OutOfDate => Failure::Surface,
        e => Failure::Other(vk_error(e)),
    };
    chain_failed(ctx, surface, failure)
}

#[cfg(windows)]
fn dxgi_failed(
    ctx: &GpuContext,
    surface: &mut GpuSurface,
    e: crate::ffi::dxgi::ChainError,
) -> Result<Presented, GpuError> {
    let failure = match e {
        crate::ffi::dxgi::ChainError::Lost => Failure::Device,
        e => Failure::Other(chain_error(e)),
    };
    chain_failed(ctx, surface, failure)
}

#[cfg(windows)]
fn chain_error(e: crate::ffi::dxgi::ChainError) -> GpuError {
    match e {
        crate::ffi::dxgi::ChainError::Lost => GpuError::Lost,
        crate::ffi::dxgi::ChainError::Failed(why) => GpuError::Surface(why),
    }
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
        let device = &self.renderer().context().device;
        match &mut surface.surface {
            Some(Swap::Wgpu(s)) => s.configure(device, &surface.config),
            #[cfg(windows)]
            Some(Swap::Dxgi(chain)) => {
                if chain.resize(device, surface.config.width, surface.config.height).is_err() {
                    // Built again at the next present.
                    surface.surface = None;
                }
            }
            #[cfg(any(
                all(unix, not(any(target_os = "macos", target_os = "ios"))),
                all(windows, feature = "windows-vulkan")
            ))]
            Some(Swap::Vulkan(chain)) => match chain.resize(surface.config.width, surface.config.height) {
                // No size to build at (minimized): built when presents find it has one.
                Ok(()) | Err(crate::ffi::vulkan::ChainError::OutOfDate) => {}
                // Built again at the next present.
                Err(_) => surface.surface = None,
            },
            None => {}
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
            let (s, c, v, r) = match build(
                self.renderer().context(),
                &surface.handles,
                surface.config.width,
                surface.config.height,
                transparent,
                vsync,
            ) {
                Ok(built) => built,
                // A device lost under a window's own swapchain: the next frame recovers it.
                Err(GpuError::Lost) => {
                    self.renderer().context().mark_lost();
                    return Ok(Presented::Retry);
                }
                Err(e) => return Err(e),
            };
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
        #[cfg(windows)]
        if let Some(Swap::Dxgi(_)) = &surface.surface {
            return self.present_dxgi(surface);
        }
        #[cfg(any(
            all(unix, not(any(target_os = "macos", target_os = "ios"))),
            all(windows, feature = "windows-vulkan")
        ))]
        if let Some(Swap::Vulkan(_)) = &surface.surface {
            return self.present_vulkan(surface);
        }
        let Some(Swap::Wgpu(swapchain)) = surface.surface.as_ref() else { return Ok(Presented::Retry) };
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
        // wgpu's images hold nothing known: the whole frame goes in (only this window's own
        // swapchains, on D3D12 and Vulkan, keep what they showed).
        let changed: Option<Vec<(u16, u16, u16, u16)>> = None;
        if copies {
            if let Some(copies) = copy_in(ctx, kept, &frame.texture, None, (w, h)) {
                ctx.queue.submit([copies]);
            }
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
            checks.unknown += 1;
            checks.mismatched += usize::from(shown.pixels != want.pixels);
        }
        ctx.queue.present(frame);
        surface.history.push_front(std::mem::take(&mut surface.pending));
        // A swapchain has at most 16 images (DXGI's limit; presentation engines keep fewer):
        // an image older than that is drawn whole.
        surface.history.truncate(16);
        Ok(Presented::Shown)
    }

    /// Presents through RemedyUI's own swapchain: the back buffer keeps what it showed, so
    /// only what changed since is copied in and reported to the system.
    #[cfg(windows)]
    fn present_dxgi(&mut self, surface: &mut GpuSurface) -> Result<Presented, GpuError> {
        use crate::ffi::dxgi::LastUse;
        let ctx = self.renderer().context();
        let Some(Swap::Dxgi(chain)) = surface.surface.as_mut() else { return Ok(Presented::Retry) };
        if chain.size() != (surface.config.width, surface.config.height)
            && let Err(e) = chain.resize(&ctx.device, surface.config.width, surface.config.height)
        {
            // Built again at the next present.
            return match e {
                crate::ffi::dxgi::ChainError::Failed(_) => {
                    surface.surface = None;
                    Ok(Presented::Retry)
                }
                e => dxgi_failed(ctx, surface, e),
            };
        }
        let acquire = remedy_ui_trace::phase!("present").entered();
        let acquired = match chain.acquire(&ctx.device) {
            Ok(Some(a)) => a,
            Ok(None) => return Ok(Presented::Occluded),
            Err(e) => return dxgi_failed(ctx, surface, e),
        };
        drop(acquire);
        let _present = remedy_ui_trace::phase!("present").entered();
        let Some(kept) = surface.frame.as_ref() else {
            // Nothing drawn yet: the buffer goes back as it came.
            drop(acquired);
            if let Some(Swap::Dxgi(chain)) = surface.surface.as_mut()
                && let Err(e) = chain.present(LastUse::Untouched, Some(&[]))
            {
                return dxgi_failed(ctx, surface, e);
            }
            return Ok(Presented::Retry);
        };
        let back = &acquired.texture;
        let (w, h) = (back.width().min(kept.width()), back.height().min(kept.height()));
        let changed = surface.changed_since(acquired.age as usize);
        let (copied, rejected) = ctx.catch_errors(|| {
            let copies = copy_in(ctx, kept, back, changed.as_deref(), (w, h));
            let copied = copies.is_some();
            if let Some(copies) = copies {
                ctx.queue.submit([copies]);
            }
            copied
        });
        if let Some(why) = rejected {
            // wgpu ran none of it: the buffer goes back as it came (a barrier from the copy
            // state would be wrong), and the window leaves the GPU.
            drop(acquired);
            if let Some(Swap::Dxgi(chain)) = surface.surface.as_mut() {
                let _ = chain.present(LastUse::Untouched, None);
            }
            surface.surface = None;
            return Err(GpuError::Surface(format!("the frame's copies were rejected: {why}")));
        }
        let mut last = if copied { LastUse::Written } else { LastUse::Untouched };
        if let Some(checks) = surface.checks.as_mut() {
            let (shown, want) = (ctx.read_back(back)?, ctx.read_back(kept)?);
            checks.compared += 1;
            checks.damaged += usize::from(changed.is_some());
            checks.unknown += usize::from(acquired.age == 0);
            checks.mismatched += usize::from(shown.pixels != want.pixels);
            last = LastUse::Read;
        }
        drop(acquired);
        let Some(Swap::Dxgi(chain)) = surface.surface.as_mut() else { return Ok(Presented::Retry) };
        if let Err(e) = chain.present(last, changed.as_deref()) {
            return dxgi_failed(ctx, surface, e);
        }
        surface.history.push_front(std::mem::take(&mut surface.pending));
        surface.history.truncate(16);
        Ok(Presented::Shown)
    }

    /// Presents through RemedyUI's own Vulkan swapchain: the image keeps what it showed, so
    /// only what changed since is copied in and reported to the presentation engine.
    #[cfg(any(
        all(unix, not(any(target_os = "macos", target_os = "ios"))),
        all(windows, feature = "windows-vulkan")
    ))]
    fn present_vulkan(&mut self, surface: &mut GpuSurface) -> Result<Presented, GpuError> {
        use crate::ffi::LastUse;
        let ctx = self.renderer().context();
        let Some(Swap::Vulkan(chain)) = surface.surface.as_mut() else { return Ok(Presented::Retry) };
        if chain.size() != (surface.config.width, surface.config.height)
            && let Err(e) = chain.resize(surface.config.width, surface.config.height)
        {
            return match e {
                crate::ffi::vulkan::ChainError::OutOfDate => Ok(Presented::Occluded),
                e => vk_failed(ctx, surface, e),
            };
        }
        let acquire = remedy_ui_trace::phase!("present").entered();
        let acquired = match chain.acquire(&ctx.device) {
            Ok(Some(a)) => a,
            Ok(None) => return Ok(Presented::Occluded),
            Err(crate::ffi::vulkan::ChainError::OutOfDate) => {
                let (w, h) = (surface.config.width, surface.config.height);
                return Ok(if chain.resize(w, h).is_ok() { Presented::Retry } else { Presented::Occluded });
            }
            Err(e) => return vk_failed(ctx, surface, e),
        };
        chain.wait_for_image(&ctx.queue);
        drop(acquire);
        let _present = remedy_ui_trace::phase!("present").entered();
        let back = &acquired.texture;
        let (mut last, changed, mut copies) = match surface.frame.as_ref() {
            Some(kept) => {
                let (w, h) = (back.width().min(kept.width()), back.height().min(kept.height()));
                let changed = surface.changed_since(acquired.age as usize);
                let copies = copy_in(ctx, kept, back, changed.as_deref(), (w, h));
                (if copies.is_some() { LastUse::Written } else { LastUse::Untouched }, changed, copies)
            }
            None => (LastUse::Untouched, Some(Vec::new()), None),
        };
        if let (Some(checks), Some(kept)) = (surface.checks.as_mut(), surface.frame.as_ref()) {
            // Read back only once the copies ran.
            if let Some(c) = copies.take() {
                ctx.queue.submit([c]);
            }
            let (shown, want) = (ctx.read_back(back)?, ctx.read_back(kept)?);
            checks.compared += 1;
            checks.damaged += usize::from(changed.is_some());
            checks.unknown += usize::from(acquired.age == 0);
            checks.mismatched += usize::from(shown.pixels != want.pixels);
            last = LastUse::Read;
        }
        drop(acquired);
        let drawn = surface.frame.is_some();
        let Some(Swap::Vulkan(chain)) = surface.surface.as_mut() else { return Ok(Presented::Retry) };
        match chain.present(&ctx.device, &ctx.queue, copies, last, changed.as_deref()) {
            Ok(rebuild) => {
                let (w, h) = (surface.config.width, surface.config.height);
                if rebuild
                    && let Err(e) = chain.resize(w, h)
                    && !matches!(e, crate::ffi::vulkan::ChainError::OutOfDate)
                {
                    surface.surface = None;
                }
            }
            Err(e) => return vk_failed(ctx, surface, e),
        }
        if !drawn {
            return Ok(Presented::Retry);
        }
        surface.history.push_front(std::mem::take(&mut surface.pending));
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
