//! RemedyUI's Android bridge.
//!
//! An app runs in a native activity (`dev.remedyui.android.RemedyActivity`, the bridge's own
//! `NativeActivity`, in APKs `cargo remedy package` makes; a plain `android.app.NativeActivity`
//! also works, with the limits below) on the shared winit core (`remedy-ui-platform::bridge_winit`), which provides the window, touch and
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
//!   kept up to date ([`remedy_ui_core::host::SafeArea`]); the engine keeps content inside it;
//! - back navigation: the back button and gesture arrive as [`remedy_ui_core::input::Key::Back`];
//! - memory: `onTrimMemory` levels and low-memory calls become memory pressure, and the
//!   device's memory comes from `/proc/meminfo`;
//! - system settings, followed as they change: dark mode, font scale, locale, reduced motion
//!   (animations off), high-contrast text, the Material You accent and the double-tap time;
//! - screen readers (TalkBack) through AccessKit's Android adapter on the activity's view,
//!   plus "set text" on text fields, which the adapter lacks;
//! - system notifications (`Host::show_notification`): an alerting and a quiet channel, the
//!   app's icon made monochrome for the status bar, the notification's picture, up to three
//!   buttons, replacing and withdrawing by id; pressing one opens the app and reports
//!   `Activated` (through a not-exported alias of the activity, so only the app's own
//!   notifications can), buttons and dismissals report through a receiver the running app
//!   registers (not exported, immutable intents, a token only the process knows).
//!   `Permission::Notifications` asks on Android 13 and later and answers whether they are on
//!   before;
//! - foreground work (`Host::start_foreground`): the `dev.remedyui.android.Foreground`
//!   service with the task's types and notification, after checking what Android would
//!   refuse (the manifest's declaration and permissions, microphone and camera permissions,
//!   the app on screen for microphone and camera work); what Android ends later arrives as
//!   `HostEvent::ForegroundStopped`;
//! - keeping the process running out of sight (`hold_process`): the
//!   `dev.remedyui.android.Keepalive` started service, so keepalive HTTP requests (an app's
//!   last saves) finish when the app goes to the background instead of being frozen with it,
//!   and the app's way out waits for them (up to 10 seconds), as on the desktop;
//! - launch options from the starting intent (`launch_extra`);
//! - the app's private data folder (`data_dir`), where `remedy_ui`'s `App::data_dir` and
//!   persisted state live;
//! - the system log (`log`), since Android discards a native app's standard output.
//!
//! What Android cannot do reports [`remedy_ui_core::host::Unsupported`]: the tray (Android has
//! none), choosing folders (a chosen folder is a document tree reachable only through content
//! URIs, never a path), dragging files out (other apps read shared files only through a
//! content provider the app declares, not paths), and emptying the clipboard before Android 9
//! (`clearPrimaryClip` arrived in API 28; earlier releases have no call that empties it).
//!
//! When the app's loop ends the activity finishes and the process exits (winit makes one
//! event loop per process; see `run`).
//!
//! The Java listeners Android reports through live in `java/`, compiled to `dex/classes.dex`
//! by `tools/build-dex.sh`. `cargo remedy package` puts that dex in the APK as the app's code
//! (the activity and the service must be classes the system can start), and the bridge uses
//! those classes; on a plain `NativeActivity` with no code in the APK, it loads the same dex
//! from memory. There pressing a notification opens the app without reporting `Activated`
//! (`NativeActivity` drops the intent naming it, except for one that starts the app with no
//! task of the app left), and foreground work reports `Unsupported`, since the manifest declares
//! no service. `tools/build-apk.sh` builds,
//! packages, signs and installs an app; see the `android-demo` example. `tools/emulator-check.sh`
//! drives the demo on an emulator, `tools/notify-check.sh` its notifications and foreground
//! work, and `tools/a11y-check/run.sh` acts on it as a screen reader does. On other platforms this crate holds only its platform-neutral conversions.
//!
//! An app's entry point is [`main!`](crate::main):
//!
//! ```ignore
//! fn app_main() {
//!     remedy_ui::run(remedy_ui::AppConfig::new("Example"), |app| { /* open windows */ }).ok();
//! }
//! remedy_ui_platform::main!(app_main);
//! ```

pub mod convert;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod os;

#[cfg(target_os = "android")]
pub use crate::bridge_winit::RunError;
#[cfg(target_os = "android")]
pub use crate::bridge_winit::winit::platform::android::activity::AndroidApp;
#[cfg(target_os = "android")]
pub use android::{
    AndroidHooks, ApkInstall, HELD_GRACE, attach, can_request_package_installs, data_dir, hold_process,
    install_apk, launch_extra, log, run, set_held_grace,
};

/// Declares the app's Android entry point: the activity starts `entry` (a `fn()`) with the
/// bridge attached, so `run` (and `remedy_ui::run`) find the activity. Compiles to nothing
/// on other platforms. The activity finishes when `entry` returns.
#[macro_export]
macro_rules! main {
    ($entry:path) => {
        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        fn android_main(app: $crate::bridge_android::AndroidApp) {
            $crate::bridge_android::attach(app);
            $entry();
        }
    };
}
