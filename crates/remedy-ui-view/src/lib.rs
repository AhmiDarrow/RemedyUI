//! RemedyUI's view layer.
//!
//! A window's UI is a tree of elements. Each element has a widget name and variant (what the
//! theme styles), states (hover, pressed, focus, disabled, …), local style overrides, a layout
//! style, and optionally text, an editable text field, a custom painter, semantics and
//! handlers. Components build elements once with [`El`]; reactive bindings (`*_dyn`) update
//! one property when its signals change and mark only the work that change needs.
//!
//! Each frame, [`Ui::frame`] re-resolves styles of elements whose state or style changed,
//! lays out what changed, re-records the paint chunks of elements that changed, and assembles
//! them into compositor layers: one per scroll container's content, and segments that keep
//! paint order around them. A frame where nothing changed produces nothing. Input
//! ([`Ui::pointer`], [`Ui::key`], [`Ui::ime`]) is routed by hit testing and focus; what the
//! view needs from its host (an input-method session, the clipboard, a cursor, a new frame)
//! comes out as [`Request`]s. See `docs/design/02-state-and-elements.md`.

mod ambient;
mod client;
mod decoration;
mod deep;
pub mod device;
mod dnd;
mod element;
mod events;
mod field;
mod flow;
mod frame;
mod history;
mod links;
pub mod motion;
mod native;
mod prop;
mod select;
pub mod strings;
mod tokens;

pub use ambient::{
    AmbientLoop, AmbientMode, AmbientPolicy, AmbientRate, AmbientTick, LoopMotion, TickKind, Wave,
};
pub use client::TextInput;
pub use decoration::{Decoration, DecorationKind};
pub use dnd::Drag;
pub use element::{
    DirtyObserver, DirtyRecord, El, ElementInfo, InlineImage, PaintCx, PointerInterceptor, TextAntialias,
    TextPaint, TextRendering, Ui, WeakUi, WindowDirection, clear_paint_cache, rasterize,
};
pub use events::{Click, MenuKey, MenuKeyHandler, PointerInput};
pub use field::{CodeColors, FieldKind, TextField};
pub use flow::{
    CaughtError, ErrorHook, FailurePhase, Retry, SUSPENSE_DELAY, SUSPENSE_MIN_SHOWN, Show, SuspenseBoundary,
    Transition, catch_errors, play_enter, play_exit, show, suspense,
};
pub use frame::{Frame, FrameStats, Placement, gradient_brush};
pub use history::HistoryItem;
pub use prop::{IntoProp, Prop};
pub use remedy_ui_a11y::Action as A11yAction;
pub use remedy_ui_host::{DragData, DragEvent, DropEffect, ImeEvent};
pub use remedy_ui_input::WheelEvent;
pub use remedy_ui_layout::{Align, Justify};
pub use remedy_ui_style::{ColorRole, Dp, Gradient, GradientStop, Pad, Pos, Radius, Space, Style, TextRole};
pub use remedy_ui_text::{InlineObject, RichText, TextStylePatch};
pub use remedy_ui_tree::ElementId;

use remedy_ui_host::TextInputState;

/// Something the view needs its host to do.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: what a view asks of its runner; every runner must carry out every request, so a new kind is a major change.
pub enum Request {
    /// Start, update (`Some`) or end (`None`) an input-method session.
    TextInput(Option<TextInputState>),
    /// Put text on the clipboard; `sensitive` keeps it out of history.
    Copy {
        /// The text.
        text: String,
        /// A secret.
        sensitive: bool,
        /// The same as HTML, for apps that paste formatting (rich text).
        html: Option<String>,
    },
    /// Put a secret on the clipboard: kept out of history and sync, and cleared after `keep`
    /// if the clipboard still holds it (a field that [`TextField::copy_as_secret`]).
    CopySecret {
        /// The text.
        text: String,
        /// How long it may stay.
        keep: std::time::Duration,
    },
    /// Read the clipboard and deliver it with [`Ui::paste`].
    Paste,
    /// Make this text the primary selection (what was just selected; X11 and Wayland paste
    /// it with the middle button). Platforms without one ignore it.
    SetPrimary(String),
    /// Deliver the primary selection with [`Ui::paste`]; on platforms without one, deliver
    /// `fallback` (the element's own last selection) instead.
    PastePrimary {
        /// What to paste where the platform has no primary selection.
        fallback: Option<String>,
    },
    /// Draw a frame.
    Frame,
    /// Call [`Ui::fire_timer`] with `id` after `delay`.
    Timer {
        /// The view's timer id.
        id: u64,
        /// How long from now.
        delay: std::time::Duration,
    },
    /// Forget a timer that has not fired.
    CancelTimer(u64),
    /// Hand a drag that left the window to the platform, for other apps; call
    /// [`Ui::drag_finished`] with the result.
    StartDrag {
        /// What is dragged.
        data: remedy_ui_host::DragData,
        /// What a drop may do.
        allowed: Vec<remedy_ui_host::DropEffect>,
    },
    /// Draw this window with the GPU renderer from now on, where the app lets the runner
    /// choose: it shows content rendered on the GPU (an embedded 3D view), which the CPU
    /// renderer could only show by reading it back every frame.
    PreferGpu,
    /// Move a scroll container to `offset` (clamped to what it can scroll).
    ScrollTo {
        /// The scroll container.
        scroller: ElementId,
        /// Its new offset.
        offset: remedy_ui_core::Vec2,
    },
}
