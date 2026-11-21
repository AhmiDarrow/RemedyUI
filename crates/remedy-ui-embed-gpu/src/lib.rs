//! The app's own GPU rendering as an element: a 3D preview, a game view, a shader
//! canvas. The app renders with wgpu on RemedyUI's own device (the one its windows draw
//! with), into textures the element owns, and the GPU renderer places the picture like any
//! image, with the element's clips, corners, opacity and scrolling.
//!
//! ```no_run
//! use remedy_ui_embed_gpu::{Gpu, GpuContent, PrepareError, Target, gpu_view, wgpu};
//! # fn build(ui: &remedy_ui_view::Ui) {
//! struct Clear;
//! impl GpuContent for Clear {
//!     fn prepare(&mut self, _gpu: &Gpu<'_>) -> Result<(), PrepareError> {
//!         Ok(())
//!     }
//!     fn render(&mut self, _gpu: &Gpu<'_>, target: &Target<'_>, encoder: &mut wgpu::CommandEncoder) {
//!         let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
//!             color_attachments: &[Some(wgpu::RenderPassColorAttachment {
//!                 view: target.view,
//!                 depth_slice: None,
//!                 resolve_target: None,
//!                 ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLUE), store: wgpu::StoreOp::Store },
//!             })],
//!             ..Default::default()
//!         });
//!     }
//! }
//! let (el, view) = gpu_view(ui, "Preview", Clear);
//! ui.root().child(el);
//! view.set_animating(true);
//! # }
//! ```
//!
//! - **The device.** [`GpuContent::prepare`] gets the device and queue (only those: the
//!   adapter, the instance and the renderer's own resources stay RemedyUI's) once at the
//!   start and again after a device loss, when every resource must be made anew. The app's
//!   GPU work runs inside error scopes: a mistake is reported in [`GpuView::status`] and the
//!   last good picture stays, instead of the window failing.
//! - **The textures.** Two, sized from the element and the window's scale, made again when
//!   either changes. The app draws into one while the other is shown.
//! - **On the CPU renderer.** A window the app keeps on the CPU renderer shows the picture
//!   read back from the GPU after each render (slower: the element asks the runner to move
//!   its window to the GPU where the app lets it choose). With no GPU at all, or when reading
//!   back fails, the element draws nothing and [`GpuView::status`] says why.
//! - **Input.** Pointer presses, moves and releases (in element coordinates), keys and focus
//!   reach [`GpuContent::input`]. The element takes focus when pressed. Wheel input scrolls
//!   the element's scroll container, as everywhere in RemedyUI, and does not reach content.
//! - **Accessibility.** The element reads as an image named by its label.
//!
//! Layer 5: built only from public engine APIs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_input::{KeyEvent, PointerEvent};
use remedy_ui_paint::{ExternalTexture, Image, Item, Sampling};
use remedy_ui_reactive::Signal;
/// The wgpu RemedyUI renders with: app content uses these types, at this version.
pub use remedy_ui_render_gpu::wgpu;
use remedy_ui_render_gpu::{FORMAT, GpuContext, LadderOptions};
use remedy_ui_view::{El, ElementId, Ui, WeakUi};

/// The device and queue the app renders with: RemedyUI's own, shared with its windows.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Gpu<'a> {
    /// The device.
    pub device: &'a wgpu::Device,
    /// Its queue.
    pub queue: &'a wgpu::Queue,
    /// The format of the textures the app draws into: premultiplied RGBA8.
    pub format: wgpu::TextureFormat,
    /// Which device this is: it changes when a lost device is replaced.
    pub serial: u64,
}

/// Where one picture goes.
#[derive(Debug)]
#[non_exhaustive]
pub struct Target<'a> {
    /// The texture to draw into ([`Gpu::format`], premultiplied alpha). Its earlier content is
    /// a picture from two renders ago, so a render starts by clearing or covering all of it.
    pub view: &'a wgpu::TextureView,
    /// The texture itself (to copy into, or to size passes by).
    pub texture: &'a wgpu::Texture,
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// This picture's number, from 1.
    pub frame: u64,
    /// Time since the element was made: what animations step by.
    pub time: Duration,
}

/// Input that reached the element.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EmbedInput {
    /// A pointer event, with its position moved into the element's own space.
    Pointer {
        /// The event as the element got it (its position in window coordinates).
        event: PointerEvent,
        /// The position in the element, in logical pixels from its top-left corner.
        local: Point,
        /// The same in the texture's device pixels.
        pixel: Point,
    },
    /// A key, while the element has focus.
    Key(KeyEvent),
    /// The element took (`true`) or lost keyboard focus.
    Focus(bool),
}

/// What the app renders in the element.
pub trait GpuContent: 'static {
    /// Makes what rendering needs (pipelines, buffers) on `gpu`: once before the first
    /// render, and again on a replacement device after a loss. An error is shown in
    /// [`GpuView::status`] and rendering waits for the next device.
    fn prepare(&mut self, gpu: &Gpu<'_>) -> Result<(), PrepareError>;

    /// Records one picture into `target` with `encoder`, which RemedyUI submits.
    fn render(&mut self, gpu: &Gpu<'_>, target: &Target<'_>, encoder: &mut wgpu::CommandEncoder);

    /// Input reached the element. Returns true when the content used it (it then goes no
    /// further). Ask for a new picture with [`GpuView::request_redraw`] when it changes it.
    fn input(&mut self, input: &EmbedInput) -> bool {
        let _ = input;
        false
    }
}

/// Why content could not get ready to render, in the app's own words.
///
/// ```
/// use remedy_ui_embed_gpu::PrepareError;
///
/// fn load_shader(found: bool) -> Result<(), PrepareError> {
///     if !found {
///         Err("the preview shader is missing")?;
///     }
///     Ok(())
/// }
/// assert_eq!(load_shader(false).map_err(|e| e.to_string()), Err("the preview shader is missing".to_owned()));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrepareError {
    reason: String,
}

impl PrepareError {
    /// Preparing failed because of `reason`.
    pub fn new(reason: impl Into<String>) -> Self {
        PrepareError { reason: reason.into() }
    }

    /// What failed.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for PrepareError {}

impl From<String> for PrepareError {
    fn from(reason: String) -> Self {
        PrepareError { reason }
    }
}

impl From<&str> for PrepareError {
    fn from(reason: &str) -> Self {
        PrepareError { reason: reason.to_owned() }
    }
}

/// How the element is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmbedStatus {
    /// Not drawn yet.
    Starting,
    /// Shown by the GPU renderer straight from the texture.
    Gpu,
    /// Shown by the CPU renderer from pictures read back from the GPU.
    ReadBack,
    /// Cannot be shown here, and why.
    Unsupported(String),
    /// The app's GPU work failed, and how; the last good picture stays.
    Failed(String),
}

/// How many pictures the element has made and shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EmbedStats {
    /// Pictures rendered by the app.
    pub rendered: u64,
    /// Of those, read back for the CPU renderer.
    pub read_back: u64,
    /// Times the textures were made (the first time, then on each resize or new device).
    pub allocations: u64,
}

/// One of the two textures.
struct Buffer {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Its registration with the device.
    id: u64,
}

/// The element's GPU state.
struct Surface {
    ctx: GpuContext,
    buffers: [Buffer; 2],
    /// Which buffer is shown.
    front: usize,
    width: u32,
    height: u32,
    /// The picture in the front buffer (0: none yet).
    frame: u64,
}

impl Surface {
    fn release(&self) {
        for b in &self.buffers {
            self.ctx.unregister_texture(b.id);
        }
    }
}

struct State {
    content: Box<dyn GpuContent>,
    /// The device content was prepared on, by serial.
    prepared: Option<u64>,
    surface: Option<Surface>,
    status: EmbedStatus,
    stats: EmbedStats,
    /// The newest picture, read back for the CPU renderer: (frame, image).
    cpu: Option<(u64, Image)>,
    frames: u64,
}

struct Shared {
    state: RefCell<State>,
    /// Bumped to ask for a new picture.
    redraw: Signal<u64>,
    animating: Cell<bool>,
    /// Some of the element is seen.
    seen: Cell<bool>,
    /// The element is gone.
    closed: Cell<bool>,
    started: Instant,
    ladder: LadderOptions,
    ui: WeakUi,
    id: Cell<Option<ElementId>>,
}

/// The app's handle to its embedded GPU content. Cheap to clone; UI thread only.
#[derive(Clone)]
pub struct GpuView(Rc<Shared>);

impl std::fmt::Debug for GpuView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuView({:?}, {:?})", self.status(), self.stats())
    }
}

impl GpuView {
    /// Renders a new picture in the next frame.
    pub fn request_redraw(&self) {
        if !self.0.closed.get() {
            self.0.redraw.update(|n| *n = n.wrapping_add(1));
        }
    }

    /// Renders a new picture every frame while `on` and the element is seen (an animation, a
    /// live preview), at the display's rate; off, only when asked or when the element's size
    /// changes.
    pub fn set_animating(&self, on: bool) {
        self.0.animating.set(on);
        if on {
            self.request_redraw();
        }
    }

    /// True while rendering every frame.
    pub fn is_animating(&self) -> bool {
        self.0.animating.get()
    }

    /// How the element is doing.
    pub fn status(&self) -> EmbedStatus {
        self.0.state.borrow().status.clone()
    }

    /// Pictures made and shown so far.
    pub fn stats(&self) -> EmbedStats {
        self.0.state.borrow().stats
    }

    /// False once the element is gone.
    pub fn is_open(&self) -> bool {
        !self.0.closed.get()
    }

    /// Hands `input` to the content as the element would (tests and automation drive the
    /// content through it). Returns whether the content used it.
    pub fn send_input(&self, input: &EmbedInput) -> bool {
        deliver(&self.0, input)
    }
}

fn deliver(shared: &Shared, input: &EmbedInput) -> bool {
    if shared.closed.get() {
        return false;
    }
    match shared.state.try_borrow_mut() {
        Ok(mut s) => s.content.input(input),
        // The content is busy (input sent from inside its own render): drop it.
        Err(_) => false,
    }
}

/// The texture size for `logical` at `scale`: whole device pixels, at least one.
fn pixels(logical: f32, scale: f32) -> u32 {
    let px = (logical * scale).round();
    if px.is_finite() && px >= 1.0 { px as u32 } else { 0 }
}

fn make_buffer(ctx: &GpuContext, width: u32, height: u32, label: &str) -> Result<Buffer, String> {
    let max = ctx.device.limits().max_texture_dimension_2d;
    if width > max || height > max {
        return Err(format!("{width} × {height} pixels is larger than this GPU's {max} pixel textures"));
    }
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let id = ctx.register_texture(&texture).map_err(|e| e.to_string())?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Buffer { texture, view, id })
}

impl State {
    /// Renders a new picture at `width` × `height` into the back buffer and shows it.
    fn render(&mut self, ladder: &LadderOptions, width: u32, height: u32, scale: f32, time: Duration) {
        let ctx = match GpuContext::shared(ladder) {
            Ok(c) => c,
            Err(e) => {
                self.status = EmbedStatus::Unsupported(format!("no GPU device to render on: {e}"));
                return;
            }
        };
        let serial = ctx.device_serial();
        let gpu = Gpu { device: &ctx.device, queue: &ctx.queue, format: FORMAT, serial };
        if self.prepared != Some(serial) {
            // A first or a replacement device: everything made on the old one is gone.
            if let Some(old) = self.surface.take() {
                old.release();
            }
            self.cpu = None;
            let (prepared, error) = ctx.catch_errors(|| self.content.prepare(&gpu));
            match prepared.map_err(|e| e.to_string()).and_then(|()| error.map_or(Ok(()), Err)) {
                Ok(()) => self.prepared = Some(serial),
                Err(e) => {
                    self.status = EmbedStatus::Failed(format!("preparing failed: {e}"));
                    return;
                }
            }
        }
        if self.surface.as_ref().is_none_or(|s| s.width != width || s.height != height) {
            if let Some(old) = self.surface.take() {
                old.release();
            }
            let made = make_buffer(&ctx, width, height, "RemedyUI embed A").and_then(|a| {
                match make_buffer(&ctx, width, height, "RemedyUI embed B") {
                    Ok(b) => Ok([a, b]),
                    Err(e) => {
                        ctx.unregister_texture(a.id);
                        Err(e)
                    }
                }
            });
            match made {
                Ok(buffers) => {
                    self.stats.allocations += 1;
                    self.surface =
                        Some(Surface { ctx: ctx.clone(), buffers, front: 0, width, height, frame: 0 });
                }
                Err(e) => {
                    self.status = EmbedStatus::Unsupported(e);
                    return;
                }
            }
        }
        let Some(surface) = self.surface.as_mut() else { return };
        let back = 1 - surface.front;
        let Some(buffer) = surface.buffers.get(back) else { return };
        self.frames += 1;
        let target = Target {
            view: &buffer.view,
            texture: &buffer.texture,
            width,
            height,
            scale,
            frame: self.frames,
            time,
        };
        let content = &mut self.content;
        let ((), error) = ctx.catch_errors(|| {
            let mut encoder = ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI embed") });
            content.render(&gpu, &target, &mut encoder);
            ctx.queue.submit([encoder.finish()]);
        });
        if let Some(e) = error {
            // The back buffer holds a broken picture: keep showing the front one.
            self.status = EmbedStatus::Failed(format!("rendering failed: {e}"));
            return;
        }
        surface.front = back;
        surface.frame = self.frames;
        self.stats.rendered += 1;
        // A good picture again after a failure: the painter says how it is shown.
        if matches!(self.status, EmbedStatus::Failed(_) | EmbedStatus::Unsupported(_)) {
            self.status = EmbedStatus::Starting;
        }
    }

    /// The front picture read back for the CPU renderer (once per picture).
    fn read_back(&mut self) -> Option<Image> {
        let surface = self.surface.as_ref()?;
        if surface.frame == 0 {
            return None;
        }
        if let Some((frame, image)) = &self.cpu
            && *frame == surface.frame
        {
            return Some(image.clone());
        }
        let buffer = surface.buffers.get(surface.front)?;
        match surface.ctx.read_back(&buffer.texture) {
            Ok(frame) => {
                let image = Image::new(frame.width, frame.height, frame.pixels.into());
                self.cpu = Some((surface.frame, image.clone()));
                self.stats.read_back += 1;
                Some(image)
            }
            Err(e) => {
                self.status = EmbedStatus::Unsupported(format!(
                    "the CPU renderer draws this window and the picture cannot be read back from the GPU: {e}"
                ));
                None
            }
        }
    }
}

/// An element showing what `content` renders on the GPU, and the app's handle to it. `label`
/// says what it shows for screen readers ("Model preview").
#[track_caller]
pub fn gpu_view(ui: &Ui, label: impl Into<String>, content: impl GpuContent) -> (El, GpuView) {
    let shared = Rc::new(Shared {
        state: RefCell::new(State {
            content: Box::new(content),
            prepared: None,
            surface: None,
            status: EmbedStatus::Starting,
            stats: EmbedStats::default(),
            cpu: None,
            frames: 0,
        }),
        redraw: Signal::new(0),
        animating: Cell::new(false),
        seen: Cell::new(true),
        closed: Cell::new(false),
        started: Instant::now(),
        ladder: LadderOptions::default(),
        ui: ui.downgrade(),
        id: Cell::new(None),
    });
    let gpu_window = ui.gpu_rendering();
    // Shown straight from the texture only on the GPU renderer: ask for it.
    ui.prefer_gpu();
    let painting = shared.clone();
    // What the last paint rendered for: (width, height, redraw count). A repaint for any
    // other reason (a theme change) shows the same picture without rendering again.
    let last = Cell::new((0u32, 0u32, u64::MAX));
    let el = ui
        .el("gpu_view")
        .grow(1.0)
        .focusable(true)
        .semantics(Semantics::new(Role::Image).label(label.into()))
        .paint(move |cx| {
            let s = &painting;
            if s.closed.get() {
                return;
            }
            let asked = s.redraw.get();
            let on_gpu = gpu_window.get();
            let (width, height) = (pixels(cx.size.width, cx.scale), pixels(cx.size.height, cx.scale));
            if width == 0 || height == 0 {
                return;
            }
            let Ok(mut state) = s.state.try_borrow_mut() else { return };
            let fresh = last.get() != (width, height, asked);
            let animate = s.animating.get() && s.seen.get();
            let lost = state.surface.as_ref().is_some_and(|f| f.ctx.is_lost());
            if fresh || animate || lost || state.surface.is_none() {
                last.set((width, height, asked));
                state.render(&s.ladder, width, height, cx.scale, s.started.elapsed());
            }
            let rect = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
            let radii = CornerRadii::all(cx.style.radius);
            if on_gpu {
                if let Some(surface) = state.surface.as_ref().filter(|f| f.frame > 0)
                    && let Some(front) = surface.buffers.get(surface.front)
                {
                    let texture = ExternalTexture { id: front.id, frame: surface.frame, width, height };
                    cx.list.push(Item::Texture { texture, rect, radii });
                    if !matches!(state.status, EmbedStatus::Failed(_)) {
                        state.status = EmbedStatus::Gpu;
                    }
                }
            } else if let Some(image) = state.read_back() {
                cx.list.push(Item::Image { image, rect, radii, sampling: Sampling::Linear });
                if !matches!(state.status, EmbedStatus::Failed(_)) {
                    state.status = EmbedStatus::ReadBack;
                }
            }
            if animate {
                cx.request_next_frame();
            }
        });
    shared.id.set(Some(el.id()));
    let seen = shared.clone();
    let pointer = shared.clone();
    let keys = shared.clone();
    let focus = shared.clone();
    let closing = shared.clone();
    let el = el
        .on_visibility(move |on| {
            seen.seen.set(on);
            // Seen again: an animation picks up where it stopped.
            if on
                && seen.animating.get()
                && let (Some(ui), Some(id)) = (seen.ui.upgrade(), seen.id.get())
            {
                ui.element(id).repaint();
                ui.request_frame();
            }
        })
        .on_pointer(move |e| {
            let (Some(ui), Some(id)) = (pointer.ui.upgrade(), pointer.id.get()) else { return false };
            let el = ui.element(id);
            let origin = el.bounds().map_or(Point::ZERO, |b| b.min);
            if e.phase == remedy_ui_input::PointerPhase::Down && !el.is_focused() {
                el.focus();
            }
            let local = Point::new(e.position.x - origin.x, e.position.y - origin.y);
            let scale = ui.scale();
            let pixel = Point::new(local.x * scale, local.y * scale);
            deliver(&pointer, &EmbedInput::Pointer { event: e.clone(), local, pixel })
        })
        .on_key(move |e| deliver(&keys, &EmbedInput::Key(e.clone())))
        .on_focus_change(move |on| {
            deliver(&focus, &EmbedInput::Focus(on));
        })
        .on_removed(move || {
            closing.closed.set(true);
            if let Ok(mut state) = closing.state.try_borrow_mut() {
                if let Some(surface) = state.surface.take() {
                    surface.release();
                }
                state.cpu = None;
            }
        });
    (el, GpuView(shared))
}

#[cfg(test)]
mod tests {
    use super::pixels;

    #[test]
    fn texture_sizes_are_whole_device_pixels() {
        assert_eq!(pixels(100.0, 1.0), 100);
        assert_eq!(pixels(100.0, 1.5), 150);
        assert_eq!(pixels(33.3, 1.25), 42);
        assert_eq!(pixels(0.2, 1.0), 0, "less than half a pixel shows nothing");
        assert_eq!(pixels(f32::NAN, 1.0), 0);
    }
}
