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

mod element;
mod events;
mod field;
mod frame;

pub use element::{El, PaintCx, Ui};
pub use events::{Click, PointerInput};
pub use field::{FieldKind, TextField};
pub use frame::Frame;
pub use remedy_ui_a11y::Action as A11yAction;
pub use remedy_ui_layout::{Align, Justify};
pub use remedy_ui_text::{RichText, StylePatch as TextPatch};
pub use remedy_ui_tree::ElementId;

use remedy_ui_host::TextInputState;

/// Something the view needs its host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Start, update (`Some`) or end (`None`) an input-method session.
    TextInput(Option<TextInputState>),
    /// Put text on the clipboard; `sensitive` keeps it out of history.
    Copy {
        /// The text.
        text: String,
        /// A secret.
        sensitive: bool,
    },
    /// Read the clipboard and deliver it with [`Ui::paste`].
    Paste,
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
}
