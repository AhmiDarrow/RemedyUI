//! The platform's decoders and sound outputs: the only code here that calls the operating
//! system, and the only place unsafe code is allowed.

use std::sync::Arc;

use crate::host::{MediaError, MediaHost};

#[cfg(target_os = "android")]
mod android;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gstreamer;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(windows)]
mod windows;

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
