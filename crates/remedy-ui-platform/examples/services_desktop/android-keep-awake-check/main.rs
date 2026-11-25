//! Keeping the screen on on Android for real, as a library an instrumentation
//! (`tools/KeepAwakeCheck.java`) loads inside an app that carries the Android bridge's dex:
//! `hold` takes two `power::inhibit(Inhibit::Display, ..)` holds on the activity (the player's
//! keep-awake hold), `release` lets go of them one by one.
//! `tools/android-keep-awake-check.sh` builds, installs and drives it with adb, reading the
//! window's flags back from `dumpsys window`. Built for any other platform it is empty.

// JNI entry points.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod os;
