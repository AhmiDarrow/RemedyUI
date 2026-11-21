//! HLS over https on Android, inside `app_process` (a real Java VM, which the https fetch goes
//! through): `tools/HlsCheck.java` serves `tests/fixtures/hls/aac` over https with a
//! self-signed certificate it trusts, loads this library and calls `run` with its port.
//! `tools/android-hls-check.sh` builds, pushes and runs it. Built for any other platform it is
//! empty.

// The JNI entry point.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod os;
