//! The isolated-decoding tests (`tests/isolated.rs`) as an Android library. An instrumentation
//! (`tools/IsolatedCheck.java`) loads it and calls `run` with the app's Context; the tests'
//! report comes back as text. `tools/android-check.sh` builds, installs and runs it. Built for
//! any other platform it is empty.

#[cfg(target_os = "android")]
#[path = "../../tests/isolated.rs"]
mod suite;

// JNI entry points and descriptor redirection.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod os;
