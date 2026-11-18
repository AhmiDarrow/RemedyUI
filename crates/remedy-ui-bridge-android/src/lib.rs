//! RemedyUI's Android bridge.
//!
//! An app runs in a native activity (`android.app.NativeActivity`, so the app ships no Java)
//! on the shared winit core (`remedy-ui-bridge-winit`), which provides the window, touch and
//! key input, frames, timers, wakes, CPU presents into the `ANativeWindow`'s buffers, the
//! native handles wgpu draws on with Vulkan or GLES, and the lifecycle: surfaces are dropped
//! when the activity goes to the background and the window is announced again, with its new
//! size, when it returns. A surface keeps the renderer it was first presented with, so
//! windows start on the GPU; one that started on the CPU moves with its next surface. This
//! crate adds:
//!
//! - input methods: an invisible view over the activity's own takes focus while a field
//!   edits and gives the input method a real `InputConnection`, so composition (underlined
//!   preedit, suggestions, reconversion), commits and delete-surrounding reach the field, with
//!   the caret's place for the input method's popups; keys it sends still reach the app as
//!   keys;
//! - open and save dialogs through the Storage Access Framework: an opened document is
//!   copied into the app's cache and that path returned; a document to save is a cache file
//!   whose contents go into the chosen document each time the app closes it after writing;
//! - drags out to other apps (split screen and freeform windows): text and links, through
//!   `View.startDragAndDrop`, with how the drop went reported back;
//! - the clipboard through `ClipboardManager` (text and HTML; sensitive copies carry
//!   `EXTRA_IS_SENSITIVE`, so Android hides them in clipboard previews);
//! - links through an `Intent` (web links open in the browser, mail links in the mail app);
//! - capture protection with `FLAG_SECURE`;
//! - the safe area: the window is laid out edge to edge (the surface already is), and the
//!   system bars, display cutouts and on-screen keyboard it leaves covered are reported and
//!   kept up to date ([`remedy_ui_host::SafeArea`]); the engine keeps content inside it;
//! - back navigation: the back button and gesture arrive as [`remedy_ui_input::Key::Back`];
//! - memory: `onTrimMemory` levels and low-memory calls become memory pressure, and the
//!   device's memory comes from `/proc/meminfo`;
//! - system settings, followed as they change: dark mode, font scale, locale, reduced motion
//!   (animations off), high-contrast text, the Material You accent and the double-tap time;
//! - screen readers (TalkBack) through AccessKit's Android adapter on the activity's view,
//!   plus "set text" on text fields, which the adapter lacks;
//! - launch options from the starting intent (`launch_extra`);
//! - the system log (`log`), since Android discards a native app's standard output.
//!
//! What Android cannot do reports [`remedy_ui_host::Unsupported`]: the tray (Android has
//! none), choosing folders (a chosen folder is a document tree reachable only through content
//! URIs, never a path), dragging files out (other apps read shared files only through a
//! content provider the app declares, not paths), and emptying the clipboard before Android 9
//! (`clearPrimaryClip` arrived in API 28; earlier releases have no call that empties it).
//!
//! When the app's loop ends the activity finishes and the process exits (winit makes one
//! event loop per process; see `run`).
//!
//! The Java listeners Android reports through live in `java/`, compiled to `dex/classes.dex`
//! by `tools/build-dex.sh` and loaded from memory at start. `tools/build-apk.sh` builds,
//! packages, signs and installs an app; see the `android-demo` example. `tools/emulator-check.sh`
//! drives the demo on an emulator, and `tools/a11y-check/run.sh` acts on it as a screen
//! reader does. On other platforms this crate holds only its platform-neutral conversions.
//!
//! An app's entry point is [`main!`]:
//!
//! ```ignore
//! fn app_main() {
//!     remedy_ui::run(remedy_ui::AppConfig::new("Example"), |app| { /* open windows */ }).ok();
//! }
//! remedy_ui_bridge_android::main!(app_main);
//! ```

pub mod convert;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod os;

#[cfg(target_os = "android")]
pub use android::{AndroidHooks, attach, launch_extra, log, run};
#[cfg(target_os = "android")]
pub use remedy_ui_bridge_winit::RunError;
#[cfg(target_os = "android")]
pub use remedy_ui_bridge_winit::winit::platform::android::activity::AndroidApp;

/// Declares the app's Android entry point: the activity starts `entry` (a `fn()`) with the
/// bridge attached, so `run` (and `remedy_ui::run`) find the activity. Compiles to nothing
/// on other platforms. The activity finishes when `entry` returns.
#[macro_export]
macro_rules! main {
    ($entry:path) => {
        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        fn android_main(app: $crate::AndroidApp) {
            $crate::attach(app);
            $entry();
        }
    };
}
