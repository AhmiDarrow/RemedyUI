//! RemedyUI isolated decoding: images decoded in a separate, sandboxed helper process that
//! gets bytes and returns pixels over shared memory (the Chromium model). A decoder that
//! crashes, loops or allocates without end, or is exploited, takes down only the helper: the
//! image shows its error state, the helper is started again, and the app carries on.
//!
//! - [`helper_main`]: call it first thing in `main`. The helper is the app's own executable
//!   started again in helper mode, so an app ships one binary.
//! - [`enable`]: routes the image loader (`remedy-ui-media-image`: [`image()`], `load`,
//!   `probe`, `decode_for_box`) and, with the `anim` feature (on by default), animations
//!   (`remedy-ui-media-anim`) through helpers from then on, and says how they are
//!   sandboxed; [`disable`] decodes in the app again.
//! - [`stats`]: how many helpers started, and how many were stopped and why.
//!
//! The sandboxes:
//!
//! - **Windows**: a less privileged AppContainer with no capabilities (no access to the
//!   user's files, the network, devices or other apps' named objects, nor to what every
//!   AppContainer may use: Windows Sockets cannot even start), entered with a restricted token
//!   (every privilege removed, administrator groups deny-only), in a job object (no child
//!   processes, no desktop, clipboard or UI handles, a memory limit for each image, killed with
//!   the app). The helper checks it is in an AppContainer before it reads anything, and quits
//!   if not. Before that it gives up bypass-traverse checking, the last privilege, as
//!   Chromium's lockdown token does, so the socket driver cannot be opened either.
//! - **Linux**: a seccomp-bpf allow-list (no opening files, no sockets, no new processes or
//!   programs, no executable memory, no changing its own limits), Landlock with no file
//!   system access (and no TCP from Landlock ABI 4), a fresh network namespace where the
//!   system lets unprivileged programs have one, and an address-space limit for each image.
//! - **macOS**: a deny-by-default Seatbelt profile (the mechanism the App Sandbox is built
//!   on) and a memory watch for each image.
//! - **Android** (10 and later): an isolated service process (`android:isolatedProcess`, the
//!   Android bridge's `dev.remedyui.android.Decoder`, which `cargo remedy package` declares)
//!   for each helper: a fresh user id with no permissions (no network, none of the app's files,
//!   no other apps' services) in the `isolated_app` SELinux domain, and an address-space limit
//!   for each image. The helper runs no `main` there: an app that registers its own formats
//!   exports `#[unsafe(no_mangle)] extern "C" fn remedy_ui_media_isolated_formats()` doing so,
//!   which the helper calls before it reads anything.
//! - **RemedyOS** (a kernel isolation domain) needs its bridge, and other platforms have no
//!   helper: [`enable`] says [`EnableError::Unsupported`] with the reason, and decoding stays
//!   in the app.
//!
//! Nothing is a fixed number. The time a helper gets for an image comes from the image's
//! pixel and byte count and this machine's measured speed (timed when the first helper
//! starts); the memory it may use comes from the image's size and the helper's own size.
//!
//! Files are read by the app, never the helper: the helper sees only the bytes it is given.
//!
//! An animation is a session on one helper, which keeps its decoder; each frame is asked for
//! with a budget sized from the frame. When a helper is stopped, the animation whose request
//! it was fails and every other animation it held opens again on a fresh helper and reads
//! forward to where it was. Memory policy, shared playheads and pausing stay
//! `remedy-ui-media-anim`'s.
//!
//! [`image()`]: remedy_ui_media_image::image

#[allow(unsafe_code)]
mod os;

#[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
mod engine;
#[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
mod helper;
#[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
mod protocol;
#[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
mod watchdog;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The argument the helper is started with. An app that parses its own arguments should
/// leave this one to [`helper_main`] (which runs before anything else does).
pub const HELPER_ARG: &str = "--remedy-ui-media-helper";

static WIRED: AtomicBool = AtomicBool::new(false);

/// Runs the decoding helper when this process was started as one, and never returns then;
/// otherwise returns at once. Call it first thing in `main`, before any threads are started
/// or windows opened (after registering any image formats with
/// `remedy_ui_media_image::add_format`, so the helper reads them too):
///
/// ```no_run
/// fn main() {
///     remedy_ui_media_isolated::helper_main();
///     let _ = remedy_ui_media_isolated::enable();
///     // ... the app ...
/// }
/// ```
// The example is about `main` itself.
#[allow(clippy::needless_doctest_main)]
pub fn helper_main() {
    WIRED.store(true, Ordering::Relaxed);
    if std::env::args_os().nth(1).is_some_and(|a| a == HELPER_ARG) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
        std::process::exit(helper::run());
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos", target_os = "android")))]
        std::process::exit(2);
    }
}

/// One way the helper is shut in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Layer {
    /// Windows: a less privileged AppContainer with no capabilities: no access to the user's
    /// files, the network, devices or other programs' named objects, nor to the objects every
    /// AppContainer may use (Windows Sockets cannot start).
    AppContainer,
    /// Windows: a restricted token: every privilege removed (bypass-traverse checking too, so
    /// devices such as the socket driver check every open), administrator groups deny-only.
    RestrictedToken,
    /// Windows: a job object: no child processes, no desktop, clipboard or UI handles, a
    /// memory limit for each image, and killed when the app goes.
    JobObject,
    /// Linux: a seccomp-bpf allow-list of system calls: no opening files, no sockets, no
    /// new processes or programs, no executable memory, no raising its own limits.
    Seccomp,
    /// Linux: Landlock at this ABI version: no file system access at all, and from ABI 4 no
    /// TCP binding or connecting.
    Landlock {
        /// The kernel's Landlock ABI version.
        abi: u32,
    },
    /// Linux: a new, empty network namespace (where unprivileged user namespaces are allowed).
    NetworkNamespace,
    /// Linux, macOS and Android: limits set for each image (address space on Linux and
    /// Android, watched memory on macOS) and, on Linux and macOS, no more open files than a
    /// request carries.
    ResourceLimits,
    /// macOS: a deny-by-default Seatbelt profile, the sandbox the App Sandbox is built on.
    AppSandbox,
    /// Android: an isolated service process: a user id of its own with no permissions (no
    /// network, none of the app's files, no other apps' services), in the `isolated_app`
    /// SELinux domain.
    IsolatedProcess,
}

/// How decoding is isolated, once [`enable`] has succeeded.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Isolation {
    /// The sandbox layers each helper runs under.
    pub layers: Vec<Layer>,
}

/// Why decoding could not be isolated. Images keep decoding in the app's own process.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnableError {
    /// This platform has no sandbox this crate can start, and why.
    Unsupported(String),
    /// [`helper_main`] was not called at the top of `main`: started as a helper, the app
    /// would run itself again instead.
    NotWired,
    /// The helper could not be started, or could not shut itself in, and why.
    Failed(String),
}

impl std::fmt::Display for EnableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnableError::Unsupported(why) => write!(f, "isolated decoding is not available here: {why}"),
            EnableError::NotWired => {
                f.write_str("remedy_ui_media_isolated::helper_main() must be called first thing in main")
            }
            EnableError::Failed(why) => write!(f, "the decoding helper could not start: {why}"),
        }
    }
}

impl std::error::Error for EnableError {}

/// What the helpers have done since the app started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stats {
    /// Helpers started (the first, those started again after one was stopped, and extra ones
    /// for images decoded at the same time).
    pub started: u64,
    /// Images (and headers) decoded in a helper.
    pub requests: u64,
    /// Helpers that crashed.
    pub crashed: u64,
    /// Helpers stopped for running far longer than their image takes.
    pub timed_out: u64,
    /// Helpers stopped for using more memory than their image needs.
    pub over_memory: u64,
    /// Helpers let go after an image because they had grown (a fresh one takes over).
    pub recycled: u64,
    /// Animations opened again on a fresh helper after another animation's request stopped
    /// the one they were on.
    pub reopened: u64,
}

pub(crate) struct Counters {
    pub started: AtomicU64,
    pub requests: AtomicU64,
    pub crashed: AtomicU64,
    pub timed_out: AtomicU64,
    pub over_memory: AtomicU64,
    pub recycled: AtomicU64,
    pub reopened: AtomicU64,
}

pub(crate) static COUNTERS: Counters = Counters {
    started: AtomicU64::new(0),
    requests: AtomicU64::new(0),
    crashed: AtomicU64::new(0),
    timed_out: AtomicU64::new(0),
    over_memory: AtomicU64::new(0),
    recycled: AtomicU64::new(0),
    reopened: AtomicU64::new(0),
};

/// What the helpers have done so far.
pub fn stats() -> Stats {
    let c = &COUNTERS;
    Stats {
        started: c.started.load(Ordering::Relaxed),
        requests: c.requests.load(Ordering::Relaxed),
        crashed: c.crashed.load(Ordering::Relaxed),
        timed_out: c.timed_out.load(Ordering::Relaxed),
        over_memory: c.over_memory.load(Ordering::Relaxed),
        recycled: c.recycled.load(Ordering::Relaxed),
        reopened: c.reopened.load(Ordering::Relaxed),
    }
}

/// Starts a sandboxed helper and routes the image loader's decoding through helpers from now
/// on. Returns how they are shut in. Calling it again while enabled returns the same answer.
///
/// It starts the first helper and times it on a picture of its own, which takes as long as
/// starting a small program does: call it at start-up, off the UI thread if the first frame
/// must not wait. When it fails, nothing changes and images decode in the app.
pub fn enable() -> Result<Isolation, EnableError> {
    if !WIRED.load(Ordering::Relaxed) {
        return Err(EnableError::NotWired);
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
    {
        engine::enable()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos", target_os = "android")))]
    {
        Err(EnableError::Unsupported(os::unsupported_reason()))
    }
}

/// Decodes in the app's own process again, and stops the helpers. Decodes under way finish in
/// their helpers. On Windows it also removes the AppContainer profile the helpers ran under,
/// so nothing stays registered on the machine (enabling again registers it again).
pub fn disable() {
    #[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
    engine::disable();
}

/// How decoding is isolated now, or `None` while it is not.
pub fn isolation() -> Option<Isolation> {
    #[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
    {
        engine::isolation()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos", target_os = "android")))]
    {
        None
    }
}
