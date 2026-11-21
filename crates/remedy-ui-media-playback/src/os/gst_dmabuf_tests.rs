//! Pictures in DMA-BUFs, imported by the GPU renderer on its Vulkan device, show the same
//! colours at the same times as pictures copied through CPU memory, through a seek, and are
//! never mapped on the CPU on the way.
//!
//! Without a hardware decoder, a stand-in hands the decoded pictures over as a VA-API or V4L2
//! decoder does: a probe after the software decoder puts each picture into a buffer of its
//! own made with GStreamer's DMA-BUF allocator (from a pool, as a decoder's are), both planes in it with rows padded to 64
//! bytes and the chroma plane page-aligned, described by a `GstVideoMeta`, and says so in the
//! caps (`video/x-raw(memory:DMABuf)`). The buffer is a memfd: this machine has no DMA-BUF
//! exporter (no dma-heap, udmabuf or DRM device), and Mesa's lavapipe imports a DMA-BUF by
//! mapping it, which a memfd also allows. Everything after the stand-in is the real path.
//!
//! The test runs in a child process of its own: which GPU takes pictures is process-wide.

use std::ffi::{c_char, c_int, c_uint};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use libloading::Library;
use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_paint::{DisplayList, Item, VideoFrame};
use remedy_ui_render_gpu::{GpuContext, GpuRenderer, LadderOptions};

use super::{MAPPED, fns};
use crate::host::{Decoder, MediaSource, Picture, Sample};
use crate::os::gstreamer::{GST_MAP_READ, GstBuffer, GstDecoder, GstMapInfo, GstMiniObject, Ptr, gst};

const CHILD: &str = "REMEDY_DMABUF_TEST_CHILD";
const GST_PAD_PROBE_TYPE_BUFFER: c_uint = 1 << 4;
const GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM: c_uint = 1 << 6;
const GST_PAD_PROBE_OK: c_int = 1;
/// `GST_EVENT_CAPS`: number 50, downstream, serialized, sticky.
const GST_EVENT_CAPS: c_uint = (50 << 8) | (1 << 1) | (1 << 2) | (1 << 3);
const GST_VIDEO_FORMAT_NV12: c_uint = 23;
/// Row padding and plane alignment of the stand-in's buffers.
const ROW_ALIGN: usize = 64;
const PAGE: usize = 4096;
/// `GST_FD_MEMORY_FLAG_DONT_CLOSE`: the pool keeps its descriptors.
const GST_FD_MEMORY_FLAG_DONT_CLOSE: c_uint = 1 << 2;

#[repr(C)]
struct GstPadProbeInfo {
    type_: c_uint,
    id: std::ffi::c_ulong,
    data: Ptr,
    offset: u64,
    size: c_uint,
}

#[repr(C)]
struct GstEvent {
    mini_object: GstMiniObject,
    type_: c_uint,
}

/// What the stand-in needs from GStreamer.
struct Producer {
    element_get_static_pad: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    pad_add_probe: unsafe extern "C" fn(Ptr, c_uint, Ptr, Ptr, Ptr) -> std::ffi::c_ulong,
    event_parse_caps: unsafe extern "C" fn(Ptr, *mut Ptr),
    event_new_caps: unsafe extern "C" fn(Ptr) -> Ptr,
    caps_copy: unsafe extern "C" fn(Ptr) -> Ptr,
    caps_features_from_string: unsafe extern "C" fn(*const c_char) -> Ptr,
    caps_set_features: unsafe extern "C" fn(Ptr, c_uint, Ptr),
    buffer_new: unsafe extern "C" fn() -> *mut GstBuffer,
    buffer_append_memory: unsafe extern "C" fn(*mut GstBuffer, Ptr),
    dmabuf_allocator_alloc_with_flags: unsafe extern "C" fn(Ptr, c_int, usize, c_uint) -> Ptr,
    mini_object_weak_ref: unsafe extern "C" fn(Ptr, unsafe extern "C" fn(Ptr, Ptr), Ptr),
    add_video_meta_full: unsafe extern "C" fn(
        *mut GstBuffer,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        *const usize,
        *const c_int,
    ) -> Ptr,
    allocator: usize,
    _libs: Vec<Library>,
}

// SAFETY: function pointers and a GstAllocator (thread-safe) for the process's life.
unsafe impl Send for Producer {}
// SAFETY: as above.
unsafe impl Sync for Producer {}

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: i64,
}

// SAFETY: the C library's `memfd_create` and `clock_gettime`, with their documented signatures.
unsafe extern "C" {
    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn clock_gettime(clock: c_int, ts: *mut Timespec) -> c_int;
}

/// The calling thread's CPU time (`CLOCK_THREAD_CPUTIME_ID`), in nanoseconds.
fn thread_cpu_ns() -> u64 {
    let mut ts = Timespec { sec: 0, nsec: 0 };
    // SAFETY: writing the clock into a local.
    unsafe { clock_gettime(3, &mut ts) };
    (ts.sec as u64).saturating_mul(1_000_000_000).saturating_add(ts.nsec as u64)
}

/// The process's CPU time so far (all threads, user and system), in milliseconds.
fn process_cpu_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // Fields after the command's closing parenthesis: utime and stime are the 12th and 13th,
    // in clock ticks of 10 ms.
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = rest.split_whitespace().collect();
    let at = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (at(11) + at(12)) * 10.0
}

fn producer() -> &'static Producer {
    static P: OnceLock<Producer> = OnceLock::new();
    P.get_or_init(|| {
        // SAFETY: GStreamer's own libraries; each symbol has the documented C signature.
        unsafe {
            let lib = |n: &str| Library::new(n).unwrap_or_else(|e| unreachable!("{n}: {e}"));
            let (core, alloc, video) =
                (lib("libgstreamer-1.0.so.0"), lib("libgstallocators-1.0.so.0"), lib("libgstvideo-1.0.so.0"));
            macro_rules! f {
                ($l:expr, $n:literal) => {
                    *$l.get(concat!($n, "\0").as_bytes()).unwrap_or_else(|e| unreachable!("{}: {e}", $n))
                };
            }
            let dmabuf_allocator_new: unsafe extern "C" fn() -> Ptr = f!(alloc, "gst_dmabuf_allocator_new");
            Producer {
                element_get_static_pad: f!(core, "gst_element_get_static_pad"),
                pad_add_probe: f!(core, "gst_pad_add_probe"),
                event_parse_caps: f!(core, "gst_event_parse_caps"),
                event_new_caps: f!(core, "gst_event_new_caps"),
                caps_copy: f!(core, "gst_caps_copy"),
                caps_features_from_string: f!(core, "gst_caps_features_from_string"),
                caps_set_features: f!(core, "gst_caps_set_features"),
                buffer_new: f!(core, "gst_buffer_new"),
                buffer_append_memory: f!(core, "gst_buffer_append_memory"),
                dmabuf_allocator_alloc_with_flags: f!(alloc, "gst_dmabuf_allocator_alloc_with_flags"),
                mini_object_weak_ref: f!(core, "gst_mini_object_weak_ref"),
                add_video_meta_full: f!(video, "gst_buffer_add_video_meta_full"),
                allocator: dmabuf_allocator_new() as usize,
                _libs: vec![core, alloc, video],
            }
        }
    })
}

/// CPU time the stand-in spends copying pictures into its buffers, in nanoseconds: the
/// measurement takes it off, as a hardware decoder writes its own.
static STAND_IN_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The stand-in's buffer pool: memfds no picture holds, as a decoder's pool hands the same few
/// buffers out again (so the renderer's per-buffer imports are used again too).
static POOL: Mutex<Vec<OwnedFd>> = Mutex::new(Vec::new());
static ALL: Mutex<Vec<OwnedFd>> = Mutex::new(Vec::new());

/// A picture's buffer was freed: its memfd goes back to the pool.
unsafe extern "C" fn back_to_pool(fd: Ptr, _buffer: Ptr) {
    let fd = fd as usize as c_int;
    let mut all = ALL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = all.iter().position(|f| f.as_raw_fd() == fd) {
        POOL.lock().unwrap_or_else(|e| e.into_inner()).push(all.swap_remove(i));
    }
}

static WIDTH: AtomicU32 = AtomicU32::new(0);
static HEIGHT: AtomicU32 = AtomicU32::new(0);

/// The stand-in decoder's hand-over: caps say DMA-BUF, and each picture moves into one.
unsafe extern "C" fn probe(_pad: Ptr, info: *mut GstPadProbeInfo, _data: Ptr) -> c_int {
    let (p, Ok(g)) = (producer(), gst()) else { return GST_PAD_PROBE_OK };
    // SAFETY: the probe's info and the event or buffer in it, ours to replace while the probe
    // runs; replaced ones are released.
    unsafe {
        let info = &mut *info;
        if info.type_ & GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM != 0 {
            let event = info.data;
            if (*event.cast::<GstEvent>()).type_ != GST_EVENT_CAPS {
                return GST_PAD_PROBE_OK;
            }
            let mut caps: Ptr = std::ptr::null_mut();
            (p.event_parse_caps)(event, &mut caps);
            let caps = (p.caps_copy)(caps);
            let st = (g.caps_get_structure)(caps, 0);
            let (mut w, mut h) = (0, 0);
            (g.structure_get_int)(st, c"width".as_ptr(), &mut w);
            (g.structure_get_int)(st, c"height".as_ptr(), &mut h);
            WIDTH.store(w as u32, Ordering::Relaxed);
            HEIGHT.store(h as u32, Ordering::Relaxed);
            (p.caps_set_features)(caps, 0, (p.caps_features_from_string)(c"memory:DMABuf".as_ptr()));
            info.data = (p.event_new_caps)(caps);
            (g.mini_object_unref)(caps);
            (g.mini_object_unref)(event);
            return GST_PAD_PROBE_OK;
        }
        let started = thread_cpu_ns();
        let src = info.data.cast::<GstBuffer>();
        let (w, h) = (WIDTH.load(Ordering::Relaxed) as usize, HEIGHT.load(Ordering::Relaxed) as usize);
        // Where the software decoder's planes are.
        let meta = fns().map_or(std::ptr::null_mut(), |f| (f.buffer_get_video_meta)(src));
        let [(y0, ys0), (uv0, uvs0)] = if meta.is_null() {
            let s = w.div_ceil(4) * 4;
            [(0, s), (s * h.div_ceil(2) * 2, s)]
        } else {
            let m = &*meta;
            let [o0, o1, ..] = m.offset;
            let [s0, s1, ..] = m.stride;
            [(o0, s0 as usize), (o1, s1 as usize)]
        };
        let mut map = GstMapInfo::default();
        if (g.buffer_map)(src, &mut map, GST_MAP_READ) == 0 {
            return GST_PAD_PROBE_OK;
        }
        let bytes = std::slice::from_raw_parts(map.data, map.size);
        // The stand-in's layout: padded rows, the chroma plane on a page of its own.
        let stride = w.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        let uv_at = (stride * h).div_ceil(PAGE) * PAGE;
        let size = (uv_at + stride * h.div_ceil(2)).div_ceil(PAGE) * PAGE;
        // Reused, as the pool's buffers are: a decoder does not allocate per picture either.
        static SCRATCH: Mutex<Vec<u8>> = Mutex::new(Vec::new());
        let mut out = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
        out.resize(size, 0);
        let planes = [(h, w, (y0, ys0), 0), (h.div_ceil(2), w.div_ceil(2) * 2, (uv0, uvs0), uv_at)];
        for (rows, len, (from, from_stride), to) in planes {
            for r in 0..rows {
                let (a, b) = (from + r * from_stride, to + r * stride);
                if let (Some(d), Some(s)) = (out.get_mut(b..b + len), bytes.get(a..a + len)) {
                    d.copy_from_slice(s);
                }
            }
        }
        (g.buffer_unmap)(src, &mut map);
        let pooled = {
            let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
            let fit = pool.iter().position(|f| {
                let len = f.try_clone().ok().and_then(|f| std::fs::File::from(f).metadata().ok());
                len.is_some_and(|m| m.len() == size as u64)
            });
            fit.map(|i| pool.swap_remove(i))
        };
        let fd = match pooled {
            Some(fd) => fd,
            None => {
                let fd = memfd_create(c"remedy-test-picture".as_ptr(), 0);
                if fd < 0 {
                    return GST_PAD_PROBE_OK;
                }
                OwnedFd::from_raw_fd(fd)
            }
        };
        let file = std::fs::File::from(fd);
        if file.write_all_at(&out, 0).is_err() {
            return GST_PAD_PROBE_OK;
        }
        let fd = OwnedFd::from(file);
        let raw = fd.as_raw_fd();
        let mem = (p.dmabuf_allocator_alloc_with_flags)(
            p.allocator as Ptr,
            raw,
            size,
            GST_FD_MEMORY_FLAG_DONT_CLOSE,
        );
        ALL.lock().unwrap_or_else(|e| e.into_inner()).push(fd);
        let dst = (p.buffer_new)();
        (p.buffer_append_memory)(dst, mem);
        (p.mini_object_weak_ref)(dst.cast(), back_to_pool, raw as usize as Ptr);
        let offsets = [0usize, uv_at, 0, 0];
        let strides = [stride as c_int, stride as c_int, 0, 0];
        (p.add_video_meta_full)(
            dst,
            0,
            GST_VIDEO_FORMAT_NV12,
            w as c_uint,
            h as c_uint,
            2,
            offsets.as_ptr(),
            strides.as_ptr(),
        );
        (*dst).pts = (*src).pts;
        (*dst).dts = (*src).dts;
        (*dst).duration = (*src).duration;
        (g.mini_object_unref)(src.cast());
        info.data = dst.cast();
        STAND_IN_NS.fetch_add(thread_cpu_ns().saturating_sub(started), Ordering::Relaxed);
        GST_PAD_PROBE_OK
    }
}

/// The software decoder's pictures, made NV12, then handed over in DMA-BUFs.
fn stand_in() -> Ptr {
    let (g, f, p) =
        (gst().unwrap_or_else(|e| unreachable!("{e}")), fns().unwrap_or_else(|| unreachable!()), producer());
    let bin = super::filter_bin(f, g, "videoconvert ! video/x-raw,format=NV12 ! identity name=handover")
        .unwrap_or_else(|| unreachable!("the stand-in bin"));
    // SAFETY: looking up the identity in the bin just made and probing its source pad; the
    // references taken by the lookups are released.
    unsafe {
        let id = (g.bin_get_by_name)(bin, c"handover".as_ptr());
        let pad = (p.element_get_static_pad)(id, c"src".as_ptr());
        let probe: unsafe extern "C" fn(Ptr, *mut GstPadProbeInfo, Ptr) -> c_int = probe;
        (p.pad_add_probe)(
            pad,
            GST_PAD_PROBE_TYPE_BUFFER | GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM,
            probe as Ptr,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        (g.object_unref)(pad);
        (g.object_unref)(id);
    }
    bin
}

fn clip() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mp4")
}

fn frame(id: u64, p: &Picture) -> VideoFrame {
    let mut f = VideoFrame::packed(1, id, p.width, p.height, p.format, p.data.clone());
    f.planes = p.planes;
    f.matrix = p.matrix;
    f.full_range = p.full_range;
    f.gpu = p.gpu.clone();
    f
}

/// Every picture of `source`, then twelve from a seek to `seek`, with their times.
fn pictures(source: &MediaSource, dma_bufs: bool, seek: Duration) -> Vec<(Duration, VideoFrame)> {
    let g = gst().unwrap_or_else(|e| unreachable!("GStreamer: {e}"));
    let mut dec = GstDecoder::open_with(g, source, dma_bufs.then(stand_in))
        .unwrap_or_else(|e| unreachable!("open: {e}"));
    let mut out = Vec::new();
    let read = |dec: &mut GstDecoder, out: &mut Vec<(Duration, VideoFrame)>, most: usize| {
        let mut n = 0;
        while n < most {
            match dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
                Some(Sample::Video { at, picture, .. }) => {
                    out.push((at, frame(out.len() as u64, &picture)));
                    n += 1;
                }
                Some(_) => {}
                None => break,
            }
        }
    };
    read(&mut dec, &mut out, usize::MAX);
    dec.seek(seek).unwrap_or_else(|e| unreachable!("seek: {e}"));
    read(&mut dec, &mut out, 12);
    out
}

fn draw(g: &mut GpuRenderer, f: &VideoFrame) -> Vec<u8> {
    let mut list = DisplayList::new();
    let rect = Rect::new(0.0, 0.0, f.width as f32, f.height as f32);
    list.push(Item::Video { frame: f.clone(), rect, radii: CornerRadii::ZERO });
    match g.render(&list, f.width, f.height, 1.0, Color::BLACK) {
        Ok(out) => out.pixels,
        Err(e) => unreachable!("render: {e}"),
    }
}

fn worst(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// Runs `name` again in a child process (with `env` set); true in that child.
fn in_child(name: &str, env: &[(&str, &str)]) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| unreachable!("{e}"));
    let status = std::process::Command::new(exe)
        .args([name, "--exact", "--nocapture", "--test-threads=1", "--include-ignored"])
        .env(CHILD, "1")
        .envs(env.iter().copied())
        .status()
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(status.success(), "the test failed in its own process");
    false
}

/// True when this system can run the test: GStreamer with an H.264 decoder.
fn can_run() -> bool {
    let Ok(g) = gst() else { return false };
    fns().is_some()
        && ["avdec_h264", "videoconvert", "identity"].iter().all(|e| match g.make(e) {
            Ok(el) => {
                // SAFETY: releasing the element just made.
                unsafe { (g.object_unref)(el) };
                true
            }
            Err(_) => false,
        })
}

/// A GPU renderer that imports DMA-BUFs, or `None` (with why) when this machine has none.
fn importing_renderer() -> Option<GpuRenderer> {
    let ctx = GpuContext::new(&LadderOptions::default()).ok()?;
    eprintln!("renderer on {:?}", ctx.adapter.get_info().name);
    let g = GpuRenderer::new(ctx, None);
    let Some(remedy_ui_paint::GpuAdapter::Uuid(_)) = remedy_ui_paint::gpu_video_adapter() else {
        eprintln!("this GPU does not import DMA-BUFs, skipping");
        return None;
    };
    eprintln!("it imports NV12 planes in modifiers {:x?}", remedy_ui_paint::gpu_video_dma_buf_modifiers());
    Some(g)
}

#[test]
fn pictures_in_dma_bufs_match_pictures_in_cpu_memory() {
    if !in_child("os::gst_dmabuf::tests::pictures_in_dma_bufs_match_pictures_in_cpu_memory", &[]) {
        return;
    }
    if !can_run() {
        eprintln!("no GStreamer with avdec_h264 here, skipping");
        return;
    }
    let source = MediaSource::file(clip());
    let seek = Duration::from_millis(1500);
    // No renderer has said it takes pictures on the GPU yet: they come in CPU memory.
    assert_eq!(remedy_ui_paint::gpu_video_adapter(), None);
    let cpu = pictures(&source, false, seek);
    assert_eq!(cpu.len(), 75 + 12);
    assert!(cpu.iter().all(|(_, f)| f.gpu.is_none() && !f.data.is_empty()));

    let Some(mut g) = importing_renderer() else { return };
    // A decoder that cannot hand over DMA-BUFs (the software one, alone) negotiates CPU
    // memory, quietly, with the same pictures.
    let fallback = pictures(&source, false, seek);
    assert_eq!(fallback.len(), cpu.len());
    for ((at_c, c), (at_f, f)) in cpu.iter().zip(&fallback) {
        assert_eq!(at_c, at_f);
        assert!(f.gpu.is_none() && !f.data.is_empty(), "at {at_f:?}");
        assert_eq!(c.to_rgba(), f.to_rgba(), "at {at_f:?}");
    }
    let mapped = MAPPED.load(Ordering::Relaxed);
    let gpu = pictures(&source, true, seek);
    assert_eq!(gpu.len(), cpu.len(), "as many pictures");
    for ((at_c, c), (at_g, f)) in cpu.iter().zip(&gpu) {
        assert_eq!(at_c, at_g, "at the same times");
        assert_eq!((c.width, c.height), (f.width, f.height));
        assert!(f.gpu.is_some() && f.data.is_empty(), "the picture at {at_g:?} came through CPU memory");
        let Some(remedy_ui_paint::GpuTexture::DmaBuf { planes, plane_count: 2, .. }) =
            f.gpu.as_ref().map(|g| g.0.texture())
        else {
            unreachable!("at {at_g:?}: not two planes in DMA-BUFs")
        };
        assert!(planes.iter().take(2).all(|p| p.fd >= 0 && (p.stride as usize).is_multiple_of(ROW_ALIGN)));
    }
    drawn_in_place_like_the_copies(&mut g, &cpu, &gpu, mapped);
}

/// Each of `gpu`'s pictures, drawn by `g` (the renderer that imports them), shows what the
/// copied picture at the same place in `cpu` shows (drawn by a renderer of its own), sampled
/// in place, with the clip's colours at its times, no GPU errors and no picture mapped since
/// `mapped`; read back, the bytes match too.
fn drawn_in_place_like_the_copies(
    g: &mut GpuRenderer,
    cpu: &[(Duration, VideoFrame)],
    gpu: &[(Duration, VideoFrame)],
    mapped: u64,
) {
    let mut w = GpuRenderer::new(
        GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("GPU: {e}")),
        None,
    );
    for (i, ((at, c), (_, f))) in cpu.iter().zip(gpu).enumerate() {
        let shown = draw(g, f);
        // In place: only the converted picture is held, no plane textures to upload into.
        assert_eq!(g.image_bytes(), u64::from(f.width * f.height * 4), "at {at:?}");
        let copied = draw(&mut w, c);
        assert!(worst(&shown, &copied) <= 2, "picture {i} at {at:?}: the GPU shows the same colours");
        // Red for the first second, green for the second, blue for the third.
        let mid = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
        let rgb = shown.get(mid..mid + 3).unwrap_or_default();
        let strongest = (0..3).max_by_key(|&i| rgb.get(i).copied().unwrap_or(0));
        assert_eq!(strongest, Some((at.as_secs() as usize).min(2)), "at {at:?}: {rgb:?}");
    }
    let errors = g.context().take_errors();
    assert!(errors.is_empty(), "the renderer recorded errors: {errors:?}");
    // Decoded, imported and drawn without a single picture mapped on the CPU.
    assert_eq!(MAPPED.load(Ordering::Relaxed), mapped, "a decoded picture was mapped on the CPU");
    // Read back (as the CPU renderer would), the bytes are the copied picture's; only a
    // linear layout can be read back.
    for ((at, c), (_, f)) in cpu.iter().zip(gpu).step_by(10) {
        let linear = matches!(
            f.gpu.as_ref().map(|g| g.0.texture()),
            Some(remedy_ui_paint::GpuTexture::DmaBuf {
                modifier: remedy_ui_paint::DRM_FORMAT_MOD_LINEAR,
                ..
            })
        );
        if linear {
            let (a, b) = (c.to_rgba().unwrap_or_default(), f.to_rgba().unwrap_or_default());
            assert!(worst(&a, &b) <= 2, "read back at {at:?}, the colours match");
        }
    }
}

/// On a machine with a hardware decoder that hands over DMA-BUFs (VA-API's `vah264dec`, a
/// V4L2 decoder, `nvidia-vaapi-driver` behind VA-API) and a Vulkan driver that imports them,
/// the decoder's own buffers (no stand-in) are imported and drawn like the copied pictures,
/// through a seek, never mapped. Run by hand: `tools/dmabuf-hardware-check.sh` does it.
#[test]
#[ignore = "needs a hardware decoder that hands over DMA-BUFs and a Vulkan driver that imports them"]
fn hardware_decoder_pictures_match_pictures_in_cpu_memory() {
    if !in_child("os::gst_dmabuf::tests::hardware_decoder_pictures_match_pictures_in_cpu_memory", &[]) {
        return;
    }
    let Ok(_) = gst() else { unreachable!("GStreamer did not load") };
    let source = MediaSource::file(clip());
    let seek = Duration::from_millis(1500);
    assert_eq!(remedy_ui_paint::gpu_video_adapter(), None);
    let cpu = pictures(&source, false, seek);
    assert_eq!(cpu.len(), 75 + 12);
    let Some(mut g) = importing_renderer() else {
        unreachable!("this Vulkan device does not import DMA-BUFs (see the log above)")
    };
    let mapped = MAPPED.load(Ordering::Relaxed);
    let gpu = pictures(&source, false, seek);
    assert_eq!(gpu.len(), cpu.len(), "as many pictures");
    let shared = gpu.iter().filter(|(_, f)| f.gpu.is_some()).count();
    assert!(
        shared == gpu.len(),
        "{shared} of {} pictures came in DMA-BUFs: the decoder GStreamer chose hands over CPU memory \
         (see the line logged above, and which decoder `GST_DEBUG=decodebin:4` picks)",
        gpu.len()
    );
    for ((at_c, c), (at_g, f)) in cpu.iter().zip(&gpu) {
        assert_eq!(at_c, at_g, "at the same times");
        assert_eq!((c.width, c.height), (f.width, f.height));
        assert!(f.data.is_empty());
    }
    if let Some(remedy_ui_paint::GpuTexture::DmaBuf { planes, plane_count, modifier, .. }) =
        gpu.first().and_then(|(_, f)| f.gpu.as_ref()).map(|g| g.0.texture())
    {
        eprintln!(
            "the decoder's pictures: {plane_count} planes, modifier {modifier:#018x}, strides {:?}, \
             {} buffer(s)",
            planes.iter().take(plane_count as usize).map(|p| p.stride).collect::<Vec<_>>(),
            if planes.first().map(|p| p.fd) == planes.get(1).map(|p| p.fd) { "one" } else { "separate" },
        );
    }
    drawn_in_place_like_the_copies(&mut g, &cpu, &gpu, mapped);
}

/// Per picture: process CPU time and wall time to decode it and draw it on the GPU (waiting
/// for the GPU), on the copy path or the DMA-BUF path. `REMEDY_BENCH_CLIP` names the clip and
/// `REMEDY_BENCH_PATH` the path: `copy`, `dmabuf` (DMA-BUFs from the stand-in decoder, which
/// copies every picture into its buffer on the CPU as a hardware decoder does not; that time
/// is measured and reported apart) or `hardware` (DMA-BUFs from whichever decoder GStreamer
/// picks, no stand-in). Each runs in a process of its own.
#[test]
#[ignore = "a measurement: run by hand with a long clip"]
fn measure_a_clip() {
    let (Ok(clip), Ok(path)) = (std::env::var("REMEDY_BENCH_CLIP"), std::env::var("REMEDY_BENCH_PATH"))
    else {
        eprintln!("set REMEDY_BENCH_CLIP and REMEDY_BENCH_PATH");
        return;
    };
    if !in_child("os::gst_dmabuf::tests::measure_a_clip", &[]) || !can_run() {
        return;
    }
    let g_ = gst().unwrap_or_else(|e| unreachable!("{e}"));
    let Some(mut g) = importing_renderer() else { return };
    let dma = path == "dmabuf" || path == "hardware";
    if !dma {
        remedy_ui_paint::set_gpu_video_adapter(None);
    }
    let producer = (path == "dmabuf").then(stand_in);
    let mut dec = GstDecoder::open_with(g_, &MediaSource::file(&clip), producer)
        .unwrap_or_else(|e| unreachable!("open: {e}"));
    let v = dec.info().video.clone().unwrap_or_else(|| unreachable!("a picture"));
    let target = g.create_target(v.width, v.height).unwrap_or_else(|e| unreachable!("target: {e}"));
    let rect = Rect::new(0.0, 0.0, v.width as f32, v.height as f32);
    let (cpu0, stand0, mapped0) =
        (process_cpu_ms(), STAND_IN_NS.load(Ordering::Relaxed), MAPPED.load(Ordering::Relaxed));
    let start = std::time::Instant::now();
    let (mut n, mut on_gpu) = (0usize, 0usize);
    // This thread's CPU time reading pictures and drawing them (the hand-over itself: mapping
    // and copying, or holding the buffer; uploading, or importing), apart from GStreamer's and
    // the driver's threads.
    let (mut read_ns, mut draw_ns) = (0u64, 0u64);
    let mut shown: Option<VideoFrame> = None;
    loop {
        let t0 = thread_cpu_ns();
        let Some(s) = dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) else { break };
        read_ns += thread_cpu_ns() - t0;
        let Sample::Video { picture, .. } = s else { continue };
        let t1 = thread_cpu_ns();
        on_gpu += usize::from(picture.gpu.is_some());
        let mut list = DisplayList::new();
        let f = frame(n as u64, &picture);
        list.push(Item::Video { frame: f.clone(), rect, radii: CornerRadii::ZERO });
        g.draw(&list, remedy_ui_paint::Affine::IDENTITY, Color::BLACK, &target)
            .unwrap_or_else(|e| unreachable!("draw {n}: {e}"));
        draw_ns += thread_cpu_ns() - t1;
        // Held until the next one shows, as a video surface holds its current frame.
        shown = Some(f);
        let _ = g.context().device.poll(remedy_ui_render_gpu::wgpu::PollType::wait_indefinitely());
        n += 1;
    }
    drop(shown);
    let wall = start.elapsed();
    let cpu = process_cpu_ms() - cpu0;
    let stand = (STAND_IN_NS.load(Ordering::Relaxed) - stand0) as f64 / 1e6;
    let k = n.max(1) as f64;
    eprintln!(
        "{path} {}x{}: {n} pictures ({on_gpu} in DMA-BUFs, {} mapped); per picture: CPU {:.2} ms \
         (stand-in copy {:.2} ms, so {:.2} ms without it); on this thread: read {:.3} ms, draw {:.3} ms; \
         frame time {:.2} ms",
        v.width,
        v.height,
        MAPPED.load(Ordering::Relaxed) - mapped0,
        cpu / k,
        stand / k,
        (cpu - stand) / k,
        read_ns as f64 / 1e6 / k,
        draw_ns as f64 / 1e6 / k,
        wall.as_secs_f64() * 1e3 / k,
    );
}
