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
    #[cfg(any(target_os = "ios", not(any(windows, unix))))]
    {
        Err(MediaError::Unsupported(
            "this platform has no media backend yet (the web and iOS arrive with their bridges)".into(),
        ))
    }
}
