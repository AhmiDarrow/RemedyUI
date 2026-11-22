//! The platform's decoders and sound outputs: the only code here that calls the operating
//! system, and the only place unsafe code is allowed.

use std::sync::Arc;

use crate::host::{MediaError, MediaHost};

#[cfg(target_os = "android")]
mod android;
#[cfg(windows)]
mod d3d11;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gst_dmabuf;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gst_leaks;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gstreamer;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gstreamer_hls;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gstreamer_stream;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod pulse_capture;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) mod web;
#[cfg(windows)]
mod windows;

/// Platform objects alive (Media Foundation starts, D3D11 devices, pictures on the GPU): debug
/// builds count them, so tests can tell a drop or an error path left none behind.
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One counted platform object, held as a field of what it counts.
#[cfg_attr(not(any(windows, feature = "keep-awake")), allow(dead_code))]
pub(crate) struct Live;

#[cfg_attr(not(any(windows, feature = "keep-awake")), allow(dead_code))]
impl Live {
    pub(crate) fn new() -> Live {
        if cfg!(debug_assertions) {
            LIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Live
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if cfg!(debug_assertions) {
            LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

pub(crate) fn live_objects() -> usize {
    let live = LIVE.load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    let live = live + gst_leaks::traced_elements();
    live
}

/// The D3D11 debug layer's lines for objects alive on decoder devices beyond the devices
/// themselves (Windows, `REMEDY_D3D_DEBUG=1`); empty elsewhere.
pub(crate) fn live_d3d_objects() -> (usize, Vec<String>) {
    #[cfg(windows)]
    {
        d3d11::live_objects_report()
    }
    #[cfg(not(windows))]
    {
        (0, Vec::new())
    }
}

/// The threads this process runs: each one's name, and where the platform says, the module
/// it started in.
pub(crate) fn process_threads() -> Vec<(String, String)> {
    #[cfg(windows)]
    {
        windows::process_threads()
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let name = |t: std::fs::DirEntry| std::fs::read_to_string(t.path().join("comm")).unwrap_or_default();
        std::fs::read_dir("/proc/self/task")
            .map(|d| d.flatten().map(|t| (name(t).trim_end().to_owned(), String::new())).collect())
            .unwrap_or_default()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    {
        Vec::new()
    }
}

pub(crate) fn platform_host() -> Result<Arc<dyn MediaHost>, MediaError> {
    #[cfg(windows)]
    {
        windows::host()
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        gstreamer::host()
    }
    #[cfg(target_os = "macos")]
    {
        macos::host()
    }
    #[cfg(target_os = "android")]
    {
        android::host()
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        web::host()
    }
    #[cfg(any(
        target_os = "ios",
        not(any(windows, unix, all(target_arch = "wasm32", target_os = "unknown")))
    ))]
    {
        Err(MediaError::Unsupported(
            "this platform has no media backend yet (iOS arrives with its bridge)".into(),
        ))
    }
}
