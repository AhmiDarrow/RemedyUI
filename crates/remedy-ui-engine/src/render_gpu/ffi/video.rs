//! Decoded pictures that are already on the GPU ([`crate::paint::GpuPicture`]), opened on
//! the renderer's device where the platform shares them with it. Where it does not (yet), a
//! picture is read back and uploaded like any other frame.
//!
//! Windows: a decoder's NV12 texture, shared from D3D11 as an NT handle, is opened on wgpu's
//! D3D12 device and wrapped as a wgpu texture for each picture. Its submission waits on the
//! decoder's shared fence first, and is followed by a one-barrier list of ours that hands the
//! texture back in the common state (wgpu leaves it in the shader-resource state), so D3D11
//! may write the next picture into it. The picture is held until that list has run.
//!
//! Linux: a picture in DMA-BUFs is imported on wgpu's Vulkan device, one image per plane
//! (`ffi::dmabuf`).
//!
//! Android: a MediaCodec picture's `AHardwareBuffer` is imported into wgpu's Vulkan device and
//! drawn into the stream's converted picture by a pass of ours ([`android`]).
//!
//! [`Shared::open`] gives the views the conversion shader samples (luma, then chroma);
//! [`Shared::submit`] submits the commands that read them.

#[cfg(not(any(
    windows,
    target_os = "android",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
use crate::paint::GpuFrame;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub(crate) use android::{Shared, wanted_extensions};
#[cfg(windows)]
pub(crate) use windows_shared::Shared;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub(crate) use super::dmabuf::Shared;

/// Device extensions importing pictures needs, which [`super::vulkan::open_device`] adds
/// where the device has them all (none: nothing is imported on this platform).
#[cfg(not(target_os = "android"))]
#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios"))),
    all(windows, feature = "render-gpu-windows-vulkan")
))]
pub(crate) fn wanted_extensions() -> &'static [&'static std::ffi::CStr] {
    &[]
}

/// A stream's pictures opened on this device, and those the GPU is still reading.
#[cfg(not(any(
    windows,
    target_os = "android",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
#[derive(Default)]
pub(crate) struct Shared(());

#[cfg(not(any(
    windows,
    target_os = "android",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
impl Shared {
    /// The views of `picture`'s planes opened on `device`; `None` when it cannot open them.
    pub(crate) fn open(
        &mut self,
        device: &wgpu::Device,
        picture: &GpuFrame,
    ) -> Option<Vec<wgpu::TextureView>> {
        let _ = (device, picture);
        None
    }

    /// Submits `commands`, which read the views [`Shared::open`] gave.
    pub(crate) fn submit(&mut self, queue: &wgpu::Queue, commands: wgpu::CommandBuffer, picture: &GpuFrame) {
        let _ = picture;
        queue.submit([commands]);
    }
}

/// Says which GPU takes pictures on the GPU: `device`'s adapter, when it can open them.
pub(crate) fn announce(device: &wgpu::Device) {
    #[cfg(windows)]
    {
        if let Some(luid) = windows_shared::adapter(device) {
            crate::paint::set_gpu_video_adapter(Some(crate::paint::GpuAdapter::Luid(luid)));
        }
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        if let Some((uuid, modifiers)) = super::dmabuf::adapter(device) {
            crate::paint::set_gpu_video_dma_buf_modifiers(modifiers);
            crate::paint::set_gpu_video_adapter(Some(crate::paint::GpuAdapter::Uuid(uuid)));
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        if web::takes_frames(device) {
            crate::paint::set_gpu_video_adapter(Some(crate::paint::GpuAdapter::Browser));
        }
    }
    #[cfg(target_os = "android")]
    {
        if android::can_import(device) {
            crate::paint::set_gpu_video_adapter(Some(crate::paint::GpuAdapter::Only));
        }
    }
    #[cfg(not(any(
        windows,
        target_os = "android",
        target_arch = "wasm32",
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    )))]
    let _ = device;
}

#[cfg(target_arch = "wasm32")]
pub(crate) use web::Web;

/// Pictures that are a browser's `VideoFrame`s; none come anywhere else.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Web(());

#[cfg(not(target_arch = "wasm32"))]
impl Web {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let _ = device;
        Web(())
    }

    /// Puts `picture` into `target` where it is; false when it is not a frame this can.
    pub(crate) fn draw(
        &self,
        device: &wgpu::Device,
        target: &wgpu::Texture,
        picture: &crate::paint::GpuFrame,
    ) -> bool {
        let _ = (device, target, picture);
        false
    }
}

/// A browser page: WebCodecs' `VideoFrame`s put into a stream's converted texture without
/// leaving the GPU.
///
/// wgpu 30's WebGPU backend has no external textures (`create_external_texture` and an
/// `ExternalTexture` binding are `unimplemented!` there), so on WebGPU the frame is imported
/// with `GPUDevice.importExternalTexture` on wgpu's own device and drawn into the texture by a
/// small pipeline made on that device directly, in the same task and submitted at once: an
/// external texture is good only until its frame is closed. A browser whose WebGPU has no
/// external textures (Firefox 160's, with `dom.webgpu.external-texture.enabled` off) gets the
/// frame copied into the texture with `GPUQueue.copyExternalImageToTexture` instead, also on
/// the GPU: through a 2D canvas where the copy takes no `VideoFrame` (Firefox 160's takes only
/// images and canvases). Which one is known once the pipeline's error scope answers; frames
/// before that are read back. wgpu's WebGL2 backend copies a `VideoFrame` only when built with `--cfg=web_sys_unstable_apis` (an app's build flag, not
/// a library's), so on WebGL2 the frame is given to `texSubImage2D` on wgpu's own WebGL2
/// context, into the texture wgpu made. Either way the browser converts YUV to RGB with the
/// frame's own color space, on the GPU.
#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::{Cell, RefCell};
    use std::mem::ManuallyDrop;
    use std::rc::Rc;

    use crate::paint::{GpuFrame, GpuTexture};
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::convert::FromWasmAbi;
    use wasm_bindgen::{JsCast, JsValue};

    use crate::render_gpu::renderer::FORMAT;

    /// Draws the imported frame over the whole target, one texel to one pixel.
    const SHADER: &str = r"
@group(0) @binding(0) var frame: texture_external;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(frame, vec2<u32>(pos.xy));
    return vec4<f32>(c.rgb, 1.0);
}
";

    fn get(o: &JsValue, k: &str) -> JsValue {
        Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
    }

    fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
        let o = Object::new();
        for (k, v) in pairs {
            let _ = Reflect::set(&o, &JsValue::from_str(k), v);
        }
        o.into()
    }

    fn call(o: &JsValue, method: &str, args: &[&JsValue]) -> Result<JsValue, JsValue> {
        let f: Function = get(o, method).dyn_into()?;
        f.apply(o, &args.iter().copied().collect::<Array>())
    }

    /// Whether `device` puts `VideoFrame`s into textures: WebGPU, or wgpu's WebGL2.
    pub(crate) fn takes_frames(device: &wgpu::Device) -> bool {
        // SAFETY: the hal device is only looked at, not used.
        device.as_webgpu().is_some() || unsafe { device.as_hal::<wgpu::hal::api::Gles>() }.is_some()
    }

    /// The pipeline that draws an imported frame, on wgpu's own `GPUDevice`.
    struct WebGpu {
        device: JsValue,
        queue: JsValue,
        layout: JsValue,
        pipeline: JsValue,
        /// The pipeline was made without a validation error (`None` until the browser says).
        valid: Rc<Cell<Option<bool>>>,
        /// `copyExternalImageToTexture` takes a `VideoFrame` (Firefox 160's takes only images
        /// and canvases): `false` once it has refused one.
        copies_frames: Cell<bool>,
        /// Where a frame is drawn for that copy otherwise: an `OffscreenCanvas` and its 2D
        /// context, made on first use.
        canvas: RefCell<Option<(JsValue, JsValue)>>,
    }

    impl WebGpu {
        fn new(device: &JsValue) -> Result<WebGpu, JsValue> {
            let valid: Rc<Cell<Option<bool>>> = Rc::default();
            if !get(device, "importExternalTexture").is_function() {
                valid.set(Some(false));
            }
            call(device, "pushErrorScope", &[&"validation".into()])?;
            let made = Self::pipeline(device);
            let scope: js_sys::Promise = call(device, "popErrorScope", &[])?.dyn_into()?;
            let (layout, pipeline) = made?;
            let answer = valid.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let error = wasm_bindgen_futures::JsFuture::from(scope).await;
                let ok = matches!(&error, Ok(e) if e.is_null());
                answer.set(Some(answer.get().unwrap_or(true) && ok));
            });
            Ok(WebGpu {
                device: device.clone(),
                queue: get(device, "queue"),
                layout,
                pipeline,
                valid,
                copies_frames: Cell::new(true),
                canvas: RefCell::default(),
            })
        }

        /// The bind group layout and the pipeline.
        fn pipeline(device: &JsValue) -> Result<(JsValue, JsValue), JsValue> {
            let module = call(device, "createShaderModule", &[&obj(&[("code", SHADER.into())])])?;
            // GPUShaderStage.FRAGMENT
            let entry = obj(&[
                ("binding", 0.into()),
                ("visibility", 2.into()),
                ("externalTexture", Object::new().into()),
            ]);
            let layout =
                call(device, "createBindGroupLayout", &[&obj(&[("entries", Array::of1(&entry).into())])])?;
            let pipeline_layout = call(
                device,
                "createPipelineLayout",
                &[&obj(&[("bindGroupLayouts", Array::of1(&layout).into())])],
            )?;
            let format = match FORMAT {
                wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
                _ => return Err("the renderer's format".into()),
            };
            let target = obj(&[("format", format.into())]);
            let pipeline = call(
                device,
                "createRenderPipeline",
                &[&obj(&[
                    ("label", "RemedyUI web video".into()),
                    ("layout", pipeline_layout),
                    ("vertex", obj(&[("module", module.clone()), ("entryPoint", "vs".into())])),
                    (
                        "fragment",
                        obj(&[
                            ("module", module),
                            ("entryPoint", "fs".into()),
                            ("targets", Array::of1(&target).into()),
                        ]),
                    ),
                    ("primitive", obj(&[("topology", "triangle-list".into())])),
                ])],
            )?;
            Ok((layout, pipeline))
        }

        /// Puts `frame` into `target`, submitted before this returns: imported and drawn, or
        /// copied on the GPU where the browser cannot import it.
        fn draw(&self, target: &JsValue, frame: &JsValue) -> Result<(), JsValue> {
            if self.valid.get().is_none() {
                return Err("the pipeline is not known to be valid yet".into());
            }
            if self.valid.get() != Some(true) {
                return self.copy(target, frame);
            }
            let external =
                call(&self.device, "importExternalTexture", &[&obj(&[("source", frame.clone())])])?;
            let entry = obj(&[("binding", 0.into()), ("resource", external)]);
            let group = call(
                &self.device,
                "createBindGroup",
                &[&obj(&[("layout", self.layout.clone()), ("entries", Array::of1(&entry).into())])],
            )?;
            let view = call(target, "createView", &[])?;
            let encoder = call(&self.device, "createCommandEncoder", &[])?;
            let attachment = obj(&[
                ("view", view),
                ("loadOp", "clear".into()),
                ("storeOp", "store".into()),
                ("clearValue", obj(&[("r", 0.into()), ("g", 0.into()), ("b", 0.into()), ("a", 0.into())])),
            ]);
            let pass = call(
                &encoder,
                "beginRenderPass",
                &[&obj(&[("colorAttachments", Array::of1(&attachment).into())])],
            )?;
            call(&pass, "setPipeline", &[&self.pipeline])?;
            call(&pass, "setBindGroup", &[&0.into(), &group])?;
            call(&pass, "draw", &[&3.into()])?;
            call(&pass, "end", &[])?;
            let commands = call(&encoder, "finish", &[])?;
            call(&self.queue, "submit", &[&Array::of1(&commands).into()])?;
            Ok(())
        }
    }

    impl WebGpu {
        /// `frame` copied into `target` with `copyExternalImageToTexture`: as it is, or drawn
        /// into a canvas by the browser first where the copy takes no frames (never read back
        /// to memory either way).
        fn copy(&self, target: &JsValue, frame: &JsValue) -> Result<(), JsValue> {
            let (width, height) = (get(target, "width"), get(target, "height"));
            let copy = |source: &JsValue| {
                let source = obj(&[("source", source.clone())]);
                let dest = obj(&[("texture", target.clone())]);
                let size = Array::of2(&width, &height);
                call(&self.queue, "copyExternalImageToTexture", &[&source, &dest, &size.into()])
            };
            if self.copies_frames.get() {
                if copy(frame).is_ok() {
                    return Ok(());
                }
                self.copies_frames.set(false);
            }
            let mut slot = self.canvas.borrow_mut();
            if slot.is_none() {
                let class: Function = get(&js_sys::global(), "OffscreenCanvas").dyn_into()?;
                let canvas = Reflect::construct(&class, &Array::of2(&width, &height))?;
                let context = call(&canvas, "getContext", &[&"2d".into()])?;
                *slot = Some((canvas, context));
            }
            let Some((canvas, context)) = slot.as_ref() else { return Err("no canvas".into()) };
            for (k, v) in [("width", &width), ("height", &height)] {
                if get(canvas, k) != *v {
                    Reflect::set(canvas, &JsValue::from_str(k), v)?;
                }
            }
            call(context, "drawImage", &[frame, &0.into(), &0.into()])?;
            copy(canvas).map(drop)
        }
    }

    /// What puts a browser's frames into textures on this renderer's device.
    pub(crate) struct Web {
        webgpu: Option<WebGpu>,
    }

    impl Web {
        pub(crate) fn new(device: &wgpu::Device) -> Self {
            Web { webgpu: device.as_webgpu().and_then(|d| WebGpu::new(d.as_ref()).ok()) }
        }

        /// Puts `picture` into `target` (the stream's converted texture, already written once
        /// by wgpu so it never clears it again) on the GPU; false when it is not a frame or
        /// this device cannot, and the caller reads it back instead.
        pub(crate) fn draw(&self, device: &wgpu::Device, target: &wgpu::Texture, picture: &GpuFrame) -> bool {
            let GpuTexture::WebVideoFrame { frame, width, height } = picture.0.texture() else {
                return false;
            };
            if (width, height) != (target.width(), target.height()) {
                return false;
            }
            // SAFETY: `frame` is the handle of the picture's own reference to the frame, which
            // lives as long as `picture` (borrowed for this call); wrapped so it is never
            // dropped here, and only borrowed (a clone is a reference of its own).
            let frame = ManuallyDrop::new(unsafe { JsValue::from_abi(frame) });
            let frame: &JsValue = &frame;
            match (&self.webgpu, target.as_webgpu()) {
                (Some(w), Some(t)) => w.draw(t.as_ref(), frame).is_ok(),
                _ => gl_upload(device, target, frame),
            }
        }
    }

    /// `frame` given to WebGL2's `texSubImage2D` into `target`'s GL texture.
    fn gl_upload(device: &wgpu::Device, target: &wgpu::Texture, frame: &JsValue) -> bool {
        use glow::HasContext;
        use wgpu::hal::api::Gles;
        // SAFETY: wgpu's GL device and texture are used on the one thread a page has, between
        // wgpu's own submissions (wgpu runs a submission's GL calls when it is submitted), and
        // the GL state touched (the bound 2D texture, two unpack flags) is put back.
        let Some(dev) = (unsafe { device.as_hal::<Gles>() }) else { return false };
        // SAFETY: as above; the texture outlives this call (the caller holds it).
        let Some(tex) = (unsafe { target.as_hal::<Gles>() }) else { return false };
        let wgpu::hal::gles::TextureInner::Texture { raw, target: bind } = tex.inner else { return false };
        if bind != glow::TEXTURE_2D {
            return false;
        }
        let ctx = dev.context();
        let gl = ctx.lock();
        let webgl: &JsValue = ctx.webgl2_context.as_ref();
        // SAFETY: as above.
        unsafe {
            // Earlier errors are not this upload's (a lost context answers forever: a few will do).
            for _ in 0..8 {
                if gl.get_error() == glow::NO_ERROR {
                    break;
                }
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(raw));
            gl.pixel_store_bool(web_sys::WebGl2RenderingContext::UNPACK_FLIP_Y_WEBGL, false);
            gl.pixel_store_bool(web_sys::WebGl2RenderingContext::UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
        }
        let n = |v: u32| JsValue::from(v);
        let called = call(
            webgl,
            "texSubImage2D",
            &[&n(glow::TEXTURE_2D), &n(0), &n(0), &n(0), &n(glow::RGBA), &n(glow::UNSIGNED_BYTE), frame],
        );
        // SAFETY: as above.
        let error = unsafe {
            let e = gl.get_error();
            gl.bind_texture(glow::TEXTURE_2D, None);
            e
        };
        called.is_ok() && error == glow::NO_ERROR
    }
}

#[cfg(windows)]
mod windows_shared {
    use std::collections::{HashMap, VecDeque};

    use crate::paint::{GpuFrame, GpuTexture};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER,
        D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE,
        D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_STATE_COMMON,
        D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        D3D12_RESOURCE_TRANSITION_BARRIER, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
        ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
    };
    use windows::core::Interface;

    type Dx12 = wgpu::hal::api::Dx12;

    /// wgpu's D3D12 device and queue, when the device is D3D12 and can sample NV12.
    fn raw(device: &wgpu::Device) -> Option<(ID3D12Device, ID3D12CommandQueue)> {
        if !device.features().contains(wgpu::Features::TEXTURE_FORMAT_NV12) {
            return None;
        }
        // SAFETY: reading wgpu's own device and queue; the clones hold their own references.
        let hal = unsafe { device.as_hal::<Dx12>() }?;
        Some((hal.raw_device().clone(), hal.raw_queue().clone()))
    }

    /// The LUID of `device`'s adapter, when it can open shared NV12 pictures.
    pub(super) fn adapter(device: &wgpu::Device) -> Option<u64> {
        let (d, _) = raw(device)?;
        // SAFETY: a query on a live device.
        let l = unsafe { d.GetAdapterLuid() };
        Some(u64::from(l.LowPart) | (u64::from(l.HighPart as u32) << 32))
    }

    /// One picture's texture on this device, between `open` and `submit`.
    struct Opening {
        texture: wgpu::Texture,
        resource: ID3D12Resource,
        fence: ID3D12Fence,
        value: u64,
    }

    /// A stream's shared pictures opened on wgpu's D3D12 device.
    #[derive(Default)]
    pub(crate) struct Shared {
        textures: HashMap<u64, ID3D12Resource>,
        fences: HashMap<u64, ID3D12Fence>,
        opening: Option<Opening>,
        /// Our barrier lists and the `done` value each last ran to.
        lists: Vec<(ID3D12CommandAllocator, ID3D12GraphicsCommandList, u64)>,
        done: Option<ID3D12Fence>,
        done_value: u64,
        /// Pictures the GPU may still read, with the `done` value that says it has finished.
        held: VecDeque<(u64, GpuFrame)>,
    }

    // SAFETY: D3D12 objects are free-threaded; a renderer uses its streams from one thread.
    unsafe impl Send for Shared {}

    impl Shared {
        /// Lets go of pictures the GPU has finished with.
        fn release(&mut self) {
            let Some(done) = &self.done else { return };
            // SAFETY: a query on a live fence.
            let reached = unsafe { done.GetCompletedValue() };
            while self.held.front().is_some_and(|(v, _)| *v <= reached) {
                self.held.pop_front();
            }
        }

        pub(crate) fn open(
            &mut self,
            device: &wgpu::Device,
            picture: &GpuFrame,
        ) -> Option<Vec<wgpu::TextureView>> {
            self.release();
            let GpuTexture::D3dNv12 {
                adapter: luid,
                texture,
                texture_key,
                fence,
                fence_key,
                value,
                width,
                height,
            } = picture.0.texture()
            else {
                return None;
            };
            let (d, _) = raw(device)?;
            if adapter(device)? != luid || width == 0 || height == 0 {
                return None;
            }
            // Opened textures kept per stream before they are let go: `GpuLimits::video_imports`.
            if self.textures.len() >= crate::render_gpu::gpu_limits().video_imports {
                self.textures.clear();
            }
            // SAFETY: the handles are the picture's, open while it lives (it is borrowed here);
            // OpenSharedHandle takes its own reference to each object.
            let resource = match self.textures.get(&texture_key) {
                Some(r) => r.clone(),
                None => unsafe {
                    let mut r: Option<ID3D12Resource> = None;
                    d.OpenSharedHandle(HANDLE(texture as *mut _), &mut r).ok()?;
                    let r = r?;
                    self.textures.insert(texture_key, r.clone());
                    r
                },
            };
            let fence = match self.fences.get(&fence_key) {
                Some(f) => f.clone(),
                None => unsafe {
                    let mut f: Option<ID3D12Fence> = None;
                    d.OpenSharedHandle(HANDLE(fence as *mut _), &mut f).ok()?;
                    let f = f?;
                    self.fences.insert(fence_key, f.clone());
                    f
                },
            };
            let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
            // SAFETY: the resource is an NV12 texture of this size made by the decoder's device
            // on this adapter, in the common state (D3D11 leaves it there and our barrier list
            // returns it there), as the initial state says.
            let t = unsafe {
                let hal = wgpu::hal::dx12::Device::texture_from_raw(
                    resource.clone(),
                    wgpu::TextureFormat::NV12,
                    wgpu::TextureDimension::D2,
                    size,
                    1,
                    1,
                );
                device.create_texture_from_hal::<Dx12>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("RemedyUI decoded picture"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::NV12,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::PRESENT,
                )
            };
            // The Y and UV planes, sampled in place.
            let views = [wgpu::TextureAspect::Plane0, wgpu::TextureAspect::Plane1]
                .into_iter()
                .zip([wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm])
                .map(|(aspect, format)| {
                    t.create_view(&wgpu::TextureViewDescriptor {
                        format: Some(format),
                        aspect,
                        ..Default::default()
                    })
                })
                .collect();
            self.opening = Some(Opening { texture: t, resource, fence, value });
            Some(views)
        }

        pub(crate) fn submit(
            &mut self,
            queue: &wgpu::Queue,
            commands: wgpu::CommandBuffer,
            picture: &GpuFrame,
        ) {
            let Some(o) = self.opening.take() else {
                queue.submit([commands]);
                return;
            };
            let raw_queue = {
                // SAFETY: reading wgpu's own queue; the clone holds its own reference. The
                // submission below waits for the decoder's copy to signal before it reads.
                let hal = unsafe { queue.as_hal::<Dx12>() };
                hal.map(|q| {
                    q.add_wait_fence(o.fence.clone(), o.value);
                    q.as_raw().clone()
                })
            };
            queue.submit([commands]);
            drop(o.texture);
            match raw_queue.and_then(|q| self.give_back(&q, &o.resource)) {
                Some(()) => self.held.push_back((self.done_value, picture.clone())),
                None => {
                    // shortcut: without our barrier list the texture stays in the
                    // shader-resource state; the picture is only held until wgpu's work is done.
                    // Upgrade if a list ever fails to be made in practice.
                    let held = picture.clone();
                    queue.on_submitted_work_done(move || drop(held));
                }
            }
        }

        /// Runs a barrier returning `resource` to the common state after what is queued, and
        /// signals `done` after it.
        fn give_back(&mut self, queue: &ID3D12CommandQueue, resource: &ID3D12Resource) -> Option<()> {
            // SAFETY: D3D12 calls on objects made here or held alive by `self`; a list is reset
            // only once the GPU has run it (its `done` value is reached), and closed before it
            // is executed. The barrier borrows `resource`, which outlives the call.
            unsafe {
                let mut device: Option<ID3D12Device> = None;
                queue.GetDevice(&mut device).ok()?;
                let device = device?;
                if self.done.is_none() {
                    self.done = Some(device.CreateFence(0, D3D12_FENCE_FLAG_NONE).ok()?);
                }
                let done = self.done.clone()?;
                let reached = done.GetCompletedValue();
                let free = self.lists.iter().position(|(_, _, v)| *v <= reached);
                let i = match free {
                    Some(i) => i,
                    None => {
                        let a: ID3D12CommandAllocator =
                            device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).ok()?;
                        let l: ID3D12GraphicsCommandList =
                            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &a, None).ok()?;
                        l.Close().ok()?;
                        self.lists.push((a, l, 0));
                        self.lists.len() - 1
                    }
                };
                let (allocator, list, used) = self.lists.get_mut(i)?;
                allocator.Reset().ok()?;
                list.Reset(&*allocator, None).ok()?;
                let barrier = D3D12_RESOURCE_BARRIER {
                    Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                    Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                    Anonymous: D3D12_RESOURCE_BARRIER_0 {
                        Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                            pResource: std::mem::transmute_copy(resource),
                            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                            // Where wgpu leaves a texture it sampled.
                            StateBefore: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
                                | D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                            StateAfter: D3D12_RESOURCE_STATE_COMMON,
                        }),
                    },
                };
                list.ResourceBarrier(&[barrier]);
                list.Close().ok()?;
                let run: ID3D12CommandList = list.cast().ok()?;
                queue.ExecuteCommandLists(&[Some(run)]);
                self.done_value += 1;
                queue.Signal(&done, self.done_value).ok()?;
                *used = self.done_value;
                Some(())
            }
        }
    }

    impl Drop for Shared {
        fn drop(&mut self) {
            // Pictures and lists go only once the GPU is done with them.
            if let Some(done) = &self.done {
                // SAFETY: a null event makes the call return once the fence reaches the value.
                unsafe {
                    if done.GetCompletedValue() < self.done_value {
                        let _ = done.SetEventOnCompletion(self.done_value, HANDLE::default());
                    }
                }
            }
        }
    }
}
