//! Linux: decoded pictures handed over in DMA-BUFs, never mapped on the CPU. When a renderer
//! has announced a Vulkan device that imports them ([`remedy_ui_engine::paint::GpuAdapter::Uuid`]),
//! the video app sink asks for `video/x-raw(memory:DMABuf)` NV12 in the layouts (DRM format
//! modifiers) the renderer takes, before NV12 in CPU memory. A decoder that can (VA-API's,
//! V4L2's) then hands over its own buffers; one that cannot negotiates CPU memory as before.
//!
//! Each picture holds its `GstBuffer` (so the decoder's pool does not reuse it) and gives the
//! renderer each plane's descriptor, offset and stride from the buffer's `GstVideoMeta`
//! ([`GpuTexture::DmaBuf`]). The buffer goes back to the pool when the picture is dropped,
//! which the renderer does only once the GPU has finished reading it.

use std::ffi::{CStr, c_char, c_int, c_uint};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use libloading::Library;
use remedy_ui_engine::paint::{
    CpuPixels, DRM_FORMAT_MOD_LINEAR, DRM_FORMAT_NV12, DmaBufPlane, GpuPicture, GpuTexture,
};

use super::gstreamer::{
    GBoolean, GST_MAP_READ, Gst, GstBuffer, GstMapInfo, GstMiniObject, Ptr, open_lib, sym,
};
use crate::host::{PixelFormat, Plane};

/// Video buffers mapped on the CPU by this crate: the copy path maps every picture, the
/// DMA-BUF path none (tests check it stays so).
pub(crate) static MAPPED: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct GstMemory {
    mini_object: GstMiniObject,
    allocator: Ptr,
    parent: Ptr,
    maxsize: usize,
    align: usize,
    offset: usize,
    size: usize,
}

#[repr(C)]
struct GstVideoMeta {
    meta_flags: c_uint,
    meta_info: Ptr,
    buffer: Ptr,
    flags: c_uint,
    format: c_uint,
    id: c_uint,
    width: c_uint,
    height: c_uint,
    n_planes: c_uint,
    offset: [usize; 4],
    stride: [c_int; 4],
}

/// The functions DMA-BUF pictures need beyond [`Gst`]'s, from GStreamer's core, allocators and
/// video libraries.
pub(crate) struct Fns {
    caps_get_features: unsafe extern "C" fn(Ptr, c_uint) -> Ptr,
    caps_features_contains: unsafe extern "C" fn(Ptr, *const c_char) -> GBoolean,
    mini_object_ref: unsafe extern "C" fn(Ptr) -> Ptr,
    buffer_find_memory:
        unsafe extern "C" fn(*mut GstBuffer, usize, usize, *mut c_uint, *mut c_uint, *mut usize) -> GBoolean,
    buffer_peek_memory: unsafe extern "C" fn(*mut GstBuffer, c_uint) -> *mut GstMemory,
    is_dmabuf_memory: unsafe extern "C" fn(*mut GstMemory) -> GBoolean,
    dmabuf_memory_get_fd: unsafe extern "C" fn(*mut GstMemory) -> c_int,
    buffer_get_video_meta: unsafe extern "C" fn(*mut GstBuffer) -> *mut GstVideoMeta,
    #[cfg(test)]
    pub(crate) parse_bin_from_description:
        unsafe extern "C" fn(*const c_char, GBoolean, *mut *mut super::gstreamer::GError) -> Ptr,
    query_add_allocation_meta: unsafe extern "C" fn(Ptr, usize, Ptr),
    video_meta_api_get_type: unsafe extern "C" fn() -> usize,
    app_sink_set_callbacks: unsafe extern "C" fn(Ptr, *const AppSinkCallbacks, Ptr, Ptr),
    _libs: Vec<Library>,
}

/// `GstAppSinkCallbacks`: five callbacks (the fifth, `propose_allocation`, from 1.24) and
/// padding, seven pointers in every release since 1.20. Room for more is left zeroed.
#[repr(C)]
pub(crate) struct AppSinkCallbacks {
    eos: Ptr,
    new_preroll: Ptr,
    new_sample: Ptr,
    new_event: Ptr,
    propose_allocation: Option<unsafe extern "C" fn(Ptr, Ptr, Ptr) -> GBoolean>,
    reserved: [Ptr; 11],
}

/// Tells the decoder the sink reads `GstVideoMeta`, so it hands over buffers with padded rows
/// or planes where it put them (a decoder or exporter that cannot otherwise copies, or falls
/// back to CPU memory).
unsafe extern "C" fn propose_allocation(_sink: Ptr, query: Ptr, _data: Ptr) -> GBoolean {
    let Some(f) = fns() else { return 0 };
    // SAFETY: the allocation query GStreamer handed to the sink, for this call.
    unsafe { (f.query_add_allocation_meta)(query, (f.video_meta_api_get_type)(), std::ptr::null_mut()) };
    1
}

/// Makes `sink` (an appsink) answer allocation queries with `GstVideoMeta`.
pub(crate) fn accept_video_meta(f: &Fns, sink: Ptr) {
    let callbacks = AppSinkCallbacks {
        eos: std::ptr::null_mut(),
        new_preroll: std::ptr::null_mut(),
        new_sample: std::ptr::null_mut(),
        new_event: std::ptr::null_mut(),
        propose_allocation: Some(propose_allocation),
        reserved: [std::ptr::null_mut(); 11],
    };
    // SAFETY: appsink copies the callbacks; the function lives for the process.
    unsafe { (f.app_sink_set_callbacks)(sink, &callbacks, std::ptr::null_mut(), std::ptr::null_mut()) };
}

// SAFETY: plain C function pointers into libraries kept loaded for the process's life.
unsafe impl Send for Fns {}
// SAFETY: as above.
unsafe impl Sync for Fns {}

/// The table, when the system's GStreamer has the allocators and video libraries.
pub(crate) fn fns() -> Option<&'static Fns> {
    static FNS: OnceLock<Option<Fns>> = OnceLock::new();
    let load = || -> Result<Fns, String> {
        let core = open_lib(&["libgstreamer-1.0.so.0", "libgstreamer-1.0.so"])?;
        let alloc = open_lib(&["libgstallocators-1.0.so.0", "libgstallocators-1.0.so"])?;
        let video = open_lib(&["libgstvideo-1.0.so.0", "libgstvideo-1.0.so"])?;
        let app = open_lib(&["libgstapp-1.0.so.0", "libgstapp-1.0.so"])?;
        Ok(Fns {
            caps_get_features: sym!(core, "gst_caps_get_features"),
            caps_features_contains: sym!(core, "gst_caps_features_contains"),
            mini_object_ref: sym!(core, "gst_mini_object_ref"),
            buffer_find_memory: sym!(core, "gst_buffer_find_memory"),
            buffer_peek_memory: sym!(core, "gst_buffer_peek_memory"),
            is_dmabuf_memory: sym!(alloc, "gst_is_dmabuf_memory"),
            dmabuf_memory_get_fd: sym!(alloc, "gst_dmabuf_memory_get_fd"),
            buffer_get_video_meta: sym!(video, "gst_buffer_get_video_meta"),
            #[cfg(test)]
            parse_bin_from_description: sym!(core, "gst_parse_bin_from_description"),
            query_add_allocation_meta: sym!(core, "gst_query_add_allocation_meta"),
            video_meta_api_get_type: sym!(video, "gst_video_meta_api_get_type"),
            app_sink_set_callbacks: sym!(app, "gst_app_sink_set_callbacks"),
            _libs: vec![core, alloc, video, app],
        })
    };
    FNS.get_or_init(|| load().ok()).as_ref()
}

/// The video sink's caps: NV12 in DMA-BUFs in `modifiers`' layouts, then in CPU memory.
/// GStreamer 1.24 names DMA-BUF layouts in `drm-format` (`NV12`, linear, or
/// `NV12:0x<modifier>`); older releases say `format=NV12` and mean linear.
pub(crate) fn caps(modifiers: &[u64]) -> String {
    let names: Vec<String> = modifiers
        .iter()
        .map(|&m| if m == DRM_FORMAT_MOD_LINEAR { "NV12".to_owned() } else { format!("NV12:0x{m:016x}") })
        .collect();
    let mut caps =
        format!("video/x-raw(memory:DMABuf),format=DMA_DRM,drm-format=(string){{ {} }}; ", names.join(", "));
    if modifiers.contains(&DRM_FORMAT_MOD_LINEAR) {
        caps.push_str("video/x-raw(memory:DMABuf),format=NV12; ");
    }
    caps.push_str("video/x-raw,format=NV12");
    caps
}

/// True when `sample`'s caps say its buffer is in DMA-BUFs.
pub(crate) fn is_dmabuf(g: &Gst, f: &Fns, sample: Ptr) -> bool {
    // SAFETY: reading a held sample's caps.
    unsafe {
        let caps = (g.sample_get_caps)(sample);
        if caps.is_null() {
            return false;
        }
        let features = (f.caps_get_features)(caps, 0);
        !features.is_null() && (f.caps_features_contains)(features, c"memory:DMABuf".as_ptr()) != 0
    }
}

/// The DRM fourcc and modifier `sample`'s caps state.
fn drm_format(g: &Gst, sample: Ptr) -> Option<(u32, u64)> {
    // SAFETY: reading a held sample's caps fields into locals; the strings are the caps'.
    let (format, drm) = unsafe {
        let st = (g.caps_get_structure)((g.sample_get_caps)(sample), 0);
        let s = |k: &CStr| {
            let p = (g.structure_get_string)(st, k.as_ptr());
            (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
        };
        (s(c"format")?, s(c"drm-format"))
    };
    let (fourcc, modifier) = if format == "DMA_DRM" {
        let drm = drm?;
        match drm.split_once(':') {
            Some((f, m)) => (f.to_owned(), u64::from_str_radix(m.trim_start_matches("0x"), 16).ok()?),
            None => (drm, DRM_FORMAT_MOD_LINEAR),
        }
    } else {
        (format, DRM_FORMAT_MOD_LINEAR)
    };
    let fourcc: [u8; 4] = fourcc.as_bytes().try_into().ok()?;
    Some((u32::from_le_bytes(fourcc), modifier))
}

/// The two planes' offsets and strides a buffer's `GstVideoMeta` gives, `Err` with the count
/// when it has other than two, `None` without one.
///
/// # Safety
/// `buffer` is a live `GstBuffer`.
unsafe fn video_meta(f: &Fns, buffer: *mut GstBuffer) -> Option<Result<[(usize, usize); 2], c_uint>> {
    // SAFETY: as the function says; the meta lives with the buffer.
    let m = unsafe { (f.buffer_get_video_meta)(buffer).as_ref() }?;
    if m.n_planes != 2 {
        return Some(Err(m.n_planes));
    }
    let [o0, o1, ..] = m.offset;
    let [s0, s1, ..] = m.stride;
    let s = |v: c_int| usize::try_from(v).unwrap_or(0);
    Some(Ok([(o0, s(s0)), (o1, s(s1))]))
}

/// An NV12 buffer's planes as its `GstVideoMeta` gives them, when it has one.
pub(crate) fn video_layout(f: &Fns, buffer: *mut GstBuffer) -> Option<[(usize, usize); 2]> {
    if buffer.is_null() {
        return None;
    }
    // SAFETY: a buffer held by the sample being read.
    unsafe { video_meta(f, buffer) }?.ok()
}

/// A picture in DMA-BUFs: a reference to its buffer, and where its planes are.
pub(crate) struct DmaPicture {
    g: &'static Gst,
    buffer: *mut GstBuffer,
    planes: [DmaBufPlane; 4],
    /// Each plane's offset and stride in the buffer as a whole (for reading it back).
    layout: [(usize, usize); 2],
    modifier: u64,
    width: u32,
    height: u32,
    /// Counted while it holds its buffer (debug builds).
    _live: super::Live,
}

// SAFETY: a GstBuffer reference may be held and released from any thread; it is only read.
unsafe impl Send for DmaPicture {}
// SAFETY: as above.
unsafe impl Sync for DmaPicture {}

impl DmaPicture {
    /// `sample`'s NV12 picture in DMA-BUFs of `width` × `height`, without mapping it; `Err`
    /// says why it cannot be shared.
    pub(crate) fn new(
        g: &'static Gst,
        f: &Fns,
        sample: Ptr,
        width: u32,
        height: u32,
    ) -> Result<DmaPicture, (String, bool)> {
        let (fourcc, modifier) =
            drm_format(g, sample).ok_or(("its caps name no DRM format".to_owned(), false))?;
        let linear = modifier == DRM_FORMAT_MOD_LINEAR;
        let no = |why: &str| (why.to_owned(), linear);
        if fourcc != DRM_FORMAT_NV12 {
            return Err(no("it is not NV12"));
        }
        // SAFETY: reading a held sample's buffer, its video meta and its memories through
        // GStreamer's accessors; the buffer gets a reference of our own, released on drop.
        unsafe {
            let buffer = (g.sample_get_buffer)(sample);
            if buffer.is_null() {
                return Err(no("the sample has no buffer"));
            }
            let layout = match video_meta(f, buffer) {
                Some(Ok(l)) => l,
                Some(Err(n)) => return Err(no(&format!("it has {n} planes"))),
                // GStreamer's default NV12 layout: rows padded to 4 bytes, chroma after the
                // luma's even rows.
                None => {
                    let stride = (width as usize).div_ceil(4) * 4;
                    [(0, stride), (stride * (height as usize).div_ceil(2) * 2, stride)]
                }
            };
            let mut planes = [DmaBufPlane { fd: -1, offset: 0, stride: 0 }; 4];
            for (p, &(offset, stride)) in planes.iter_mut().zip(&layout) {
                let (mut idx, mut len, mut skip) = (0 as c_uint, 0 as c_uint, 0usize);
                if (f.buffer_find_memory)(buffer, offset, 1, &mut idx, &mut len, &mut skip) == 0 {
                    return Err(no("a plane lies outside its memory"));
                }
                let mem = (f.buffer_peek_memory)(buffer, idx);
                if mem.is_null() || (f.is_dmabuf_memory)(mem) == 0 {
                    return Err(no("a plane is not in a DMA-BUF"));
                }
                let fd = (f.dmabuf_memory_get_fd)(mem);
                if fd < 0 || stride == 0 {
                    return Err(no("a plane has no descriptor or stride"));
                }
                *p = DmaBufPlane {
                    fd,
                    offset: ((*mem).offset + skip) as u64,
                    stride: u32::try_from(stride).map_err(|_| no("a stride too large"))?,
                };
            }
            (f.mini_object_ref)(buffer.cast());
            Ok(DmaPicture { g, buffer, planes, layout, modifier, width, height, _live: super::Live::new() })
        }
    }
}

impl Drop for DmaPicture {
    fn drop(&mut self) {
        // SAFETY: releasing the reference taken in `new`.
        unsafe { (self.g.mini_object_unref)(self.buffer.cast()) };
    }
}

impl GpuPicture for DmaPicture {
    fn texture(&self) -> GpuTexture {
        GpuTexture::DmaBuf {
            planes: self.planes,
            plane_count: 2,
            drm_fourcc: DRM_FORMAT_NV12,
            modifier: self.modifier,
            width: self.width,
            height: self.height,
        }
    }

    fn read_back(&self) -> Option<CpuPixels> {
        // Only a linear layout means anything when mapped.
        if self.modifier != DRM_FORMAT_MOD_LINEAR {
            return None;
        }
        MAPPED.fetch_add(1, Ordering::Relaxed);
        let [(y, ys), (uv, uvs)] = self.layout;
        let rows = self.height as usize;
        let end = (uv + uvs * rows.div_ceil(2)).max(y + ys * rows);
        // SAFETY: mapping the held buffer for reading while its bytes are copied, then
        // unmapping it (GStreamer syncs a DMA-BUF's caches around the map).
        unsafe {
            let mut map = GstMapInfo::default();
            if (self.g.buffer_map)(self.buffer, &mut map, GST_MAP_READ) == 0 {
                return None;
            }
            let bytes: Option<std::sync::Arc<[u8]>> = (!map.data.is_null() && map.size >= end)
                .then(|| std::sync::Arc::from(std::slice::from_raw_parts(map.data, end)));
            (self.g.buffer_unmap)(self.buffer, &mut map);
            Some(CpuPixels {
                format: PixelFormat::Nv12,
                planes: [
                    Plane { offset: y, stride: ys },
                    Plane { offset: uv, stride: uvs },
                    Plane::default(),
                ],
                data: bytes?,
            })
        }
    }
}

/// `description` (a `gst-launch` fragment) made into a bin, for playbin's `video-filter`.
#[cfg(test)]
pub(crate) fn filter_bin(f: &Fns, g: &Gst, description: &str) -> Option<Ptr> {
    let c = std::ffi::CString::new(description).ok()?;
    let mut err: *mut super::gstreamer::GError = std::ptr::null_mut();
    // SAFETY: parsing a description with unlinked pads ghosted; the bin is floating.
    let bin = unsafe { (f.parse_bin_from_description)(c.as_ptr(), 1, &mut err) };
    if bin.is_null() {
        let _ = g.take_error(err);
        return None;
    }
    Some(bin)
}

#[cfg(test)]
#[path = "gst_dmabuf_tests.rs"]
mod tests;
