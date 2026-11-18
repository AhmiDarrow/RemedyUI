//! RemedyUI devtools, version 1.
//!
//! - **Inspector** ([`Inspector`], [`inspector_panel`]): pick an element in a window to see
//!   its widget, the source line that built it, its box, resolved style, states, semantics
//!   and bindings; browse the element tree; run the accessibility check ([`audit`]: unnamed
//!   controls, low-contrast text, small targets); edit theme colors live.
//! - **Signal graph** ([`element_signals`], [`UpdateLog`]): an element's bindings, the
//!   signals and memos they read and everything else depending on those; and why each element
//!   updated ("row “42” repainted because its text binding re-ran: `message.edited` changed").
//! - **Performance** ([`Recorder`], [`PerfOverlay`]): the engine's phase spans as per-frame
//!   timings drawn over a window, and whole sessions exported as Perfetto traces with the UI
//!   thread, the compositor, workers and the GPU on one timeline ([`Trace`]).
//! - **Accessibility view** ([`reader_lines`]): the window as a screen reader meets it, with
//!   problems flagged, unreachable controls included.
//! - **Remote devtools** ([`RemoteServer`], [`RemoteClient`]): a local, token-authenticated
//!   connection (debug builds only by default) serving all of the above to a devtools window
//!   elsewhere, such as an app on an Android emulator.
//! - **Previews** ([`Preview`]): render a piece of UI in every theme, locale (right-to-left
//!   ones mirrored), window size and text scale asked for, show them side by side
//!   ([`gallery`]), and check them against golden images ([`check_golden`]).
//!
//! The `remedy-ui` facade opens these as windows with its `devtools` feature (F12 opens
//! the inspector). See `docs/design/12-developer-experience.md`.

mod inspect;
mod overlay;
mod panels;
mod preview;
mod recorder;
mod remote;
mod signals;

/// The theme and style sheet devtools windows draw with (the first Remedy palette and the
/// widget sheet), whatever the inspected app uses.
pub fn tool_style() -> (remedy_ui_style::Theme, remedy_ui_style::StyleSheet) {
    let theme = remedy_ui_themes::PALETTES
        .first()
        .map_or_else(remedy_ui_style::Theme::dark_forest, remedy_ui_themes::theme);
    let sheet = remedy_ui_themes::sheet(&theme);
    (theme, sheet)
}

pub use inspect::{
    COLOR_ROLES, DEFAULT_LOG_KEEP, Inspector, Problem, ProblemKind, ReaderLine, SheetFn, Unreachable, audit,
    describe, detail_rows, inspector_panel, reader_lines, unreachable_problems, walk,
};
pub use overlay::{PHASE_COLORS, PerfOverlay, graph_frames};
pub use panels::TABS;
pub use preview::{
    GOLDEN_TOLERANCE, GoldenError, Preview, PreviewBuilder, PreviewSpec, Snapshot, Variation, background,
    check_golden, gallery, golden_test, is_rtl,
};
pub use recorder::{FrameFeed, FrameSample, GPU_TRACK, InstallError, Recorder, Slice, Trace};
/// `#[preview]` declares a preview beside the UI it shows; `#[previews]` lists a module's
/// previews for the gallery.
pub use remedy_ui_macros::{preview, previews};
pub use remote::{
    PROTOCOL_VERSION, RemoteAnswer, RemoteBinding, RemoteClient, RemoteElement, RemoteFrame, RemoteNode,
    RemoteOptions, RemoteProblem, RemoteRead, RemoteReaderNode, RemoteRequest, RemoteServer, RemoteUpdate,
};
pub use signals::{Binding, DEVTOOLS_WIDGETS, Dependent, Read, UpdateEntry, UpdateLog, element_signals};
