//! Video frames: pictures that change many times a second, in the pixel layouts decoders and
//! capture produce (NV12, I420, RGBA). Renderers convert them to the screen's colors: the GPU
//! renderer on the GPU, the CPU renderer here ([`VideoFrame::to_rgba`]).
//!
//! A frame's pixels are in CPU memory, or already on the GPU ([`GpuPicture`]): a hardware
//! decoder's picture, shared with the renderer's device so it never passes through the CPU.

use std::sync::{Arc, Mutex, PoisonError};

/// How a frame's pixels are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum PixelFormat {
    /// One plane of 8-bit RGBA, straight (not premultiplied) alpha.
    Rgba,
    /// A full-size 8-bit luma plane, then one half-size plane of interleaved U and V.
    Nv12,
    /// A full-size 8-bit luma plane, then half-size U and V planes.
    I420,
}

/// Which matrix turns a frame's Y, U and V into R, G and B.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum YuvMatrix {
    /// Standard definition (BT.601).
    Bt601,
    /// High definition (BT.709), what most encoders use from 720p up.
    #[default]
    Bt709,
}

/// Where one plane starts in the frame's bytes and how far apart its rows are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Plane {
    /// The plane's first byte.
    pub offset: usize,
    /// Bytes from one row to the next (at least the row's width in bytes).
    pub stride: usize,
}

/// One frame of a video stream.
#[derive(Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// The stream it belongs to: renderers keep one texture per stream and refill it.
    pub stream: u64,
    /// This frame, unique within its stream (a new number for every new picture).
    pub id: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// How the bytes are laid out.
    pub format: PixelFormat,
    /// The YUV matrix (ignored for RGBA).
    pub matrix: YuvMatrix,
    /// Full-range YUV (0-255) rather than the video range (16-235 luma, 16-240 chroma).
    pub full_range: bool,
    /// The planes, in order: RGBA has one, NV12 two (Y, UV), I420 three (Y, U, V).
    pub planes: [Plane; 3],
    /// All of the frame's bytes (empty for a frame that lives on the GPU).
    pub data: Arc<[u8]>,
    /// The picture on the GPU, when the decoder handed it over there: `data` is then empty
    /// and `planes` unused, and renderers that cannot open the texture read it back
    /// ([`GpuPicture::read_back`]).
    pub gpu: Option<GpuFrame>,
}

/// A picture in GPU memory, shared by a decoder with whichever renderer draws it.
///
/// A renderer that samples the texture keeps its clone of the [`GpuFrame`] until the GPU work
/// that reads it has finished: the decoder reuses the texture only once every clone is gone.
pub trait GpuPicture: Send + Sync {
    /// The texture, as the platform shares it between devices.
    fn texture(&self) -> GpuTexture;
    /// Its pixels copied into CPU memory, for a renderer that cannot open the texture (the
    /// CPU renderer, or a GPU on another adapter); `None` when it cannot be read.
    fn read_back(&self) -> Option<CpuPixels>;
}

/// The pixels [`GpuPicture::read_back`] gives, laid out as [`VideoFrame`]'s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuPixels {
    /// How the bytes are laid out.
    pub format: PixelFormat,
    /// The planes, as [`VideoFrame::planes`].
    pub planes: [Plane; 3],
    /// The bytes.
    pub data: Arc<[u8]>,
}

/// How a platform shares a texture between the decoder's device and the renderer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GpuTexture {
    /// Windows: an NV12 texture made by a D3D11 device as a shared NT handle, opened on a
    /// D3D12 device with `ID3D12Device::OpenSharedHandle`, and a shared fence the decoder
    /// signals once the picture is written.
    D3dNv12 {
        /// The adapter both devices are on ([`GpuAdapter::Luid`]).
        adapter: u64,
        /// The texture's NT handle, open while the picture lives.
        texture: usize,
        /// Unique to this texture in the process: the same key is the same texture, so a
        /// renderer opens it once.
        texture_key: u64,
        /// The fence's NT handle, open while the picture lives.
        fence: usize,
        /// Unique to this fence in the process, as `texture_key`.
        fence_key: u64,
        /// The fence value at which the picture is written: GPU work reading it waits for it.
        value: u64,
        /// The texture's size (at least the frame's, even).
        width: u32,
        /// The texture's height.
        height: u32,
    },
    /// A browser page: a WebCodecs `VideoFrame`, which the page's GPU device imports where it
    /// is (`GPUDevice.importExternalTexture` on WebGPU, `texImage2D` from the frame on
    /// WebGL2). The picture holds the frame open and closes it when it is dropped.
    WebVideoFrame {
        /// wasm-bindgen's handle to the picture's own reference to the frame (the `u32` a
        /// `&JsValue` passes as), valid while the picture lives. A renderer clones the
        /// reference for as long as it uses the frame; it never drops this one.
        frame: u32,
        /// The frame's size as shown (`displayWidth`), which is what an import gives.
        width: u32,
        /// Its `displayHeight`.
        height: u32,
    },
    /// Android: an `AHardwareBuffer` a MediaCodec decoder wrote (through an `AImageReader`),
    /// already complete, imported into Vulkan with
    /// `VK_ANDROID_external_memory_android_hardware_buffer` and sampled through a
    /// `VkSamplerYcbcrConversion` for its format.
    HardwareBuffer {
        /// The `AHardwareBuffer*`, valid while the picture lives.
        buffer: usize,
        /// The buffer's size in pixels (at least the frame's).
        width: u32,
        /// The buffer's height.
        height: u32,
        /// Where the frame's pixels start in the buffer (its crop).
        left: u32,
        /// As `left`, downwards.
        top: u32,
    },
}

/// A frame's [`GpuPicture`], cheap to clone; equal when it is the same picture.
#[derive(Clone)]
pub struct GpuFrame(pub Arc<dyn GpuPicture>);

impl PartialEq for GpuFrame {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for GpuFrame {}

impl std::fmt::Debug for GpuFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuFrame({:?})", self.0.texture())
    }
}

/// A GPU, as the platform names it, so a decoder makes its pictures on the GPU the renderer
/// draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GpuAdapter {
    /// A DXGI adapter's LUID (Windows), low part in the low 32 bits.
    Luid(u64),
    /// A browser page's GPU (WebGPU or WebGL2): a page's `VideoFrame`s import into any of its
    /// devices, so there is nothing to tell them apart by.
    Browser,
    /// The device's only GPU (Android: decoders have no say in which).
    Only,
}

static GPU_VIDEO: Mutex<Option<GpuAdapter>> = Mutex::new(None);

/// Says a renderer on `adapter` takes [`GpuPicture`]s, or (`None`) that none does. GPU
/// renderers call it when their device can open shared pictures; decoders ask
/// [`gpu_video_adapter`] when they open, and hand over pictures in CPU memory when it is `None`.
pub fn set_gpu_video_adapter(adapter: Option<GpuAdapter>) {
    *GPU_VIDEO.lock().unwrap_or_else(PoisonError::into_inner) = adapter;
}

/// The adapter a renderer takes [`GpuPicture`]s on ([`set_gpu_video_adapter`]), if any.
pub fn gpu_video_adapter() -> Option<GpuAdapter> {
    *GPU_VIDEO.lock().unwrap_or_else(PoisonError::into_inner)
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VideoFrame({} #{} {:?} {}x{})", self.stream, self.id, self.format, self.width, self.height)
    }
}

impl PixelFormat {
    /// Each plane's size in samples (width, height) for a `w` × `h` frame, and bytes per sample.
    pub fn plane_sizes(self, w: u32, h: u32) -> Vec<(u32, u32, u32)> {
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        match self {
            PixelFormat::Rgba => vec![(w, h, 4)],
            PixelFormat::Nv12 => vec![(w, h, 1), (cw, ch, 2)],
            PixelFormat::I420 => vec![(w, h, 1), (cw, ch, 1), (cw, ch, 1)],
        }
    }
}

impl VideoFrame {
    /// A tightly packed frame: planes one after another, rows without padding.
    pub fn packed(
        stream: u64,
        id: u64,
        width: u32,
        height: u32,
        format: PixelFormat,
        data: Arc<[u8]>,
    ) -> Self {
        let mut planes = [Plane::default(); 3];
        let mut at = 0;
        for (i, (w, h, b)) in format.plane_sizes(width, height).into_iter().enumerate() {
            if let Some(p) = planes.get_mut(i) {
                *p = Plane { offset: at, stride: (w * b) as usize };
            }
            at += (w * b * h) as usize;
        }
        VideoFrame {
            stream,
            id,
            width,
            height,
            format,
            matrix: YuvMatrix::default(),
            full_range: false,
            planes,
            data,
            gpu: None,
        }
    }

    /// A frame of `picture`, which lives on the GPU.
    pub fn on_gpu(stream: u64, id: u64, width: u32, height: u32, picture: Arc<dyn GpuPicture>) -> Self {
        let mut f = VideoFrame::packed(stream, id, width, height, PixelFormat::Nv12, Arc::from([]));
        f.gpu = Some(GpuFrame(picture));
        f
    }

    /// True when every plane lies inside the bytes and its rows are wide enough (or the frame
    /// is on the GPU). Renderers skip frames that are not.
    pub fn is_valid(&self) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        if self.gpu.is_some() {
            return true;
        }
        self.format.plane_sizes(self.width, self.height).iter().zip(self.planes.iter()).all(
            |(&(w, h, b), p)| {
                let row = (w * b) as usize;
                p.stride >= row
                    && p.offset
                        .checked_add(p.stride.saturating_mul(h.saturating_sub(1) as usize))
                        .and_then(|v| v.checked_add(row))
                        .is_some_and(|end| end <= self.data.len())
            },
        )
    }

    /// Row `y` of plane `i`, `len` bytes long.
    fn row(&self, i: usize, y: u32, len: usize) -> &[u8] {
        let p = self.planes.get(i).copied().unwrap_or_default();
        let start = p.offset + p.stride * y as usize;
        self.data.get(start..start + len).unwrap_or_default()
    }

    /// The frame with its pixels in CPU memory: itself, or read back from the GPU.
    pub fn in_cpu_memory(&self) -> Option<std::borrow::Cow<'_, VideoFrame>> {
        let Some(gpu) = &self.gpu else { return Some(std::borrow::Cow::Borrowed(self)) };
        let px = gpu.0.read_back()?;
        let f = VideoFrame { format: px.format, planes: px.planes, data: px.data, gpu: None, ..self.clone() };
        f.is_valid().then_some(std::borrow::Cow::Owned(f))
    }

    /// The frame as premultiplied RGBA8, row by row (what images hold), or `None` when its
    /// planes do not fit its bytes. A frame on the GPU is read back first.
    pub fn to_rgba(&self) -> Option<Vec<u8>> {
        if self.gpu.is_some() {
            return self.in_cpu_memory()?.to_rgba();
        }
        if !self.is_valid() {
            return None;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = vec![0u8; w * h * 4];
        if self.format == PixelFormat::Rgba {
            for (y, dst) in out.chunks_exact_mut(w * 4).enumerate() {
                let src = self.row(0, y as u32, w * 4);
                for (d, [r, g, b, a]) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                    // Premultiply, rounding.
                    let pm = |c: u8| ((u16::from(c) * u16::from(*a) + 127) / 255) as u8;
                    *d = [pm(*r), pm(*g), pm(*b), *a];
                }
            }
            return Some(out);
        }
        let k = Coefficients::new(self.matrix, self.full_range);
        let cw = w.div_ceil(2);
        for (y, dst) in out.chunks_exact_mut(w * 4).enumerate() {
            let luma = self.row(0, y as u32, w);
            let cy = (y / 2) as u32;
            let (u_row, v_row, interleaved) = match self.format {
                PixelFormat::Nv12 => (self.row(1, cy, cw * 2), &[][..], true),
                _ => (self.row(1, cy, cw), self.row(2, cy, cw), false),
            };
            for (x, d) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let c = x / 2;
                let (u, v) = if interleaved {
                    (u_row.get(c * 2).copied().unwrap_or(128), u_row.get(c * 2 + 1).copied().unwrap_or(128))
                } else {
                    (u_row.get(c).copied().unwrap_or(128), v_row.get(c).copied().unwrap_or(128))
                };
                let [r, g, b] = k.rgb(luma.get(x).copied().unwrap_or(0), u, v);
                *d = [r, g, b, 255];
            }
        }
        Some(out)
    }
}

/// The numbers of a YUV to RGB conversion, scaled for the range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefficients {
    /// Luma offset (16 for video range, 0 for full).
    pub y_offset: f32,
    /// Luma scale.
    pub y_scale: f32,
    /// Chroma scale.
    pub c_scale: f32,
    /// R from V.
    pub rv: f32,
    /// G from U.
    pub gu: f32,
    /// G from V.
    pub gv: f32,
    /// B from U.
    pub bu: f32,
}

impl Coefficients {
    /// The conversion for `matrix` in the given range.
    pub fn new(matrix: YuvMatrix, full_range: bool) -> Self {
        // Kr and Kb of each standard.
        let (kr, kb) = match matrix {
            YuvMatrix::Bt601 => (0.299, 0.114),
            YuvMatrix::Bt709 => (0.2126, 0.0722),
        };
        let kg = 1.0 - kr - kb;
        let (y_offset, y_scale, c_scale) =
            if full_range { (0.0, 1.0, 1.0) } else { (16.0, 255.0 / 219.0, 255.0 / 224.0) };
        Coefficients {
            y_offset,
            y_scale,
            c_scale,
            rv: 2.0 * (1.0 - kr),
            gu: -2.0 * (1.0 - kb) * kb / kg,
            gv: -2.0 * (1.0 - kr) * kr / kg,
            bu: 2.0 * (1.0 - kb),
        }
    }

    /// One pixel.
    pub fn rgb(&self, y: u8, u: u8, v: u8) -> [u8; 3] {
        let l = (f32::from(y) - self.y_offset) * self.y_scale;
        let cb = (f32::from(u) - 128.0) * self.c_scale;
        let cr = (f32::from(v) - 128.0) * self.c_scale;
        let to = |f: f32| f.round().clamp(0.0, 255.0) as u8;
        [to(l + self.rv * cr), to(l + self.gu * cb + self.gv * cr), to(l + self.bu * cb)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12(w: u32, h: u32, y: u8, u: u8, v: u8) -> VideoFrame {
        let mut data = vec![y; (w * h) as usize];
        for _ in 0..(w.div_ceil(2) * h.div_ceil(2)) {
            data.extend([u, v]);
        }
        VideoFrame::packed(1, 1, w, h, PixelFormat::Nv12, data.into())
    }

    #[test]
    fn video_range_black_white_and_primaries_come_out_right() {
        let k = Coefficients::new(YuvMatrix::Bt709, false);
        assert_eq!(k.rgb(16, 128, 128), [0, 0, 0]);
        assert_eq!(k.rgb(235, 128, 128), [255, 255, 255]);
        // BT.709 video-range red is Y 63, U 102, V 240.
        let [r, g, b] = k.rgb(63, 102, 240);
        assert!(r >= 250 && g <= 5 && b <= 5, "{r} {g} {b}");
        let full = Coefficients::new(YuvMatrix::Bt601, true);
        assert_eq!(full.rgb(0, 128, 128), [0, 0, 0]);
        assert_eq!(full.rgb(255, 128, 128), [255, 255, 255]);
    }

    #[test]
    fn layouts_convert_alike_and_bad_frames_are_refused() {
        let a = nv12(3, 3, 81, 90, 240).to_rgba().unwrap_or_default();
        let mut i420 = vec![81u8; 9];
        i420.extend([90u8; 4]);
        i420.extend([240u8; 4]);
        let b = VideoFrame::packed(1, 2, 3, 3, PixelFormat::I420, i420.into()).to_rgba().unwrap_or_default();
        assert_eq!(a.len(), 36);
        assert_eq!(a, b, "NV12 and I420 of the same picture match");
        // Padded rows: the stride skips the padding.
        let mut f = nv12(2, 2, 200, 128, 128);
        let mut padded = vec![200, 200, 0, 0, 200, 200, 0, 0];
        padded.extend([128, 128, 9, 9]);
        f.planes = [Plane { offset: 0, stride: 4 }, Plane { offset: 8, stride: 4 }, Plane::default()];
        f.data = padded.into();
        assert!(f.is_valid());
        assert_eq!(f.to_rgba().and_then(|p| p.get(..4).map(<[u8]>::to_vec)), Some(vec![214, 214, 214, 255]));
        // Planes past the end of the bytes.
        f.data = vec![0u8; 5].into();
        assert!(!f.is_valid());
        assert_eq!(f.to_rgba(), None);
        // RGBA is premultiplied on the way.
        let rgba = VideoFrame::packed(1, 3, 1, 1, PixelFormat::Rgba, vec![200, 100, 0, 128].into());
        assert_eq!(rgba.to_rgba(), Some(vec![100, 50, 0, 128]));
    }

    struct Fake(Vec<u8>);

    impl GpuPicture for Fake {
        fn texture(&self) -> GpuTexture {
            GpuTexture::D3dNv12 {
                adapter: 0,
                texture: 0,
                texture_key: 1,
                fence: 0,
                fence_key: 1,
                value: 0,
                width: 2,
                height: 2,
            }
        }
        fn read_back(&self) -> Option<CpuPixels> {
            let f = VideoFrame::packed(0, 0, 2, 2, PixelFormat::Nv12, self.0.clone().into());
            Some(CpuPixels { format: f.format, planes: f.planes, data: f.data })
        }
    }

    #[test]
    fn a_frame_on_the_gpu_converts_like_its_bytes_once_read_back() {
        let cpu = nv12(2, 2, 81, 90, 240);
        let gpu = VideoFrame::on_gpu(1, 1, 2, 2, Arc::new(Fake(cpu.data.to_vec())));
        assert!(gpu.is_valid() && gpu.data.is_empty());
        assert_eq!(gpu.to_rgba(), cpu.to_rgba());
        assert_eq!(gpu.clone(), gpu, "the same picture");
        assert_ne!(VideoFrame::on_gpu(1, 1, 2, 2, Arc::new(Fake(vec![]))).to_rgba(), cpu.to_rgba());
    }
}
