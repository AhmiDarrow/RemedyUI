//! The media controls on Android for real, as a library an instrumentation
//! (`tools/MediaCheck.java`) loads inside an app that carries the Android bridge's dex:
//! `start` opens a `MediaSession` showing an item, `commands` lists what the system pressed,
//! `stop` ends it. `tools/android-media-check.sh` builds, installs and drives it with adb.
//! Built for any other platform it is empty.

// JNI entry points.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod os;
