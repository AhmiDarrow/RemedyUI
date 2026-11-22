//! Guides, porting guides and recipes.
//!
//! These pages hold no code of their own. Every Rust sample in them is a doctest, so
//! `cargo test --doc -p remedy-ui` compiles and runs it, and a guide cannot drift from the
//! API it describes. Most samples run headless and assert what they show.
//!
//! Start with [`getting_started`], then [`signals`]: everything else builds on them.
//!
//! | Guide | What it covers |
//! |---|---|
//! | [`getting_started`] | A first app with `cargo remedy new`, and what each file is for |
//! | [`stability`] | What does not break, types that grow, editions, `cargo remedy migrate` |
//! | [`signals`] | Signals, memos, effects, properties, context, persisted state, batching and lifetimes |
//! | [`layout`] | Rows, columns, grids, sizes, alignment, scrolling and right to left |
//! | [`theming`] | Themes, typed tokens, style sheets and variants |
//! | [`text`] | Labels, rich text, editable fields, secrets and input methods |
//! | [`control_flow`] | `show`, error boundaries, loading boundaries and resources |
//! | [`lists`] | Keyed lists, `SignalVec`, transitions, selectable and virtual lists |
//! | [`motion`] | Animating elements, transitions, the frame clock and reduced motion |
//! | [`accessibility`] | Semantics, names, the reader view and the accessibility check |
//! | [`testing`] | `TestApp`, finding elements by what they are, `#[preview]` and golden images |
//! | [`network`] | Requests to a server, natively and in a page: JSON, errors, uploads, keepalive, resources |
//! | [`packaging`] | `cargo remedy package`, signing and signed updates |
//! | [`performance`] | Budgets, the performance overlay and Perfetto traces |
//! | [`customization`] | `.rui` markup and layouts people change themselves |
//! | [`agents`] | Assistants operating a window with the person's consent |
//! | [`media`] | Images, animated images, video and playback |
//! | [`documents`] | Paged documents, annotations, code and rich text |
//! | [`terminal`] | The terminal element and the app's PTY |
//! | [`web`] | Running in a browser page or as a PWA, and what a browser cannot do |
//! | [`porting`] | Moving from egui, React, Qt or PyQt, and pywebview or HTML apps |
//! | [`recipes`] | Whole small apps, each with a headless test |

#[doc = include_str!("getting_started.md")]
pub mod getting_started {}

#[cfg(all(feature = "viz", feature = "headless", feature = "widgets"))]
#[doc = include_str!("stability.md")]
pub mod stability {}

#[doc = include_str!("signals.md")]
pub mod signals {}

#[doc = include_str!("layout.md")]
pub mod layout {}

#[doc = include_str!("theming.md")]
pub mod theming {}

#[doc = include_str!("text.md")]
pub mod text {}

#[doc = include_str!("control_flow.md")]
pub mod control_flow {}

#[doc = include_str!("lists.md")]
pub mod lists {}

#[doc = include_str!("motion.md")]
pub mod motion {}

#[doc = include_str!("accessibility.md")]
pub mod accessibility {}

#[cfg(all(feature = "headless", feature = "devtools"))]
#[doc = include_str!("testing.md")]
pub mod testing {}

#[cfg(feature = "net")]
#[doc = include_str!("network.md")]
pub mod network {}

#[cfg(feature = "update")]
#[doc = include_str!("packaging.md")]
pub mod packaging {}

#[cfg(feature = "devtools")]
#[doc = include_str!("performance.md")]
pub mod performance {}

#[cfg(feature = "markup")]
#[doc = include_str!("customization.md")]
pub mod customization {}

#[cfg(feature = "agents")]
#[doc = include_str!("automation.md")]
pub mod agents {}

#[cfg(all(feature = "image", feature = "anim", feature = "playback"))]
#[doc = include_str!("media.md")]
pub mod media {}

#[cfg(all(feature = "pages", feature = "rich", feature = "code"))]
#[doc = include_str!("documents.md")]
pub mod documents {}

#[cfg(feature = "term")]
#[doc = include_str!("terminal.md")]
pub mod terminal {}

#[cfg(all(
    feature = "playback",
    feature = "update",
    feature = "headless",
    feature = "widgets",
    feature = "web"
))]
#[doc = include_str!("web.md")]
pub mod web {}

#[doc = include_str!("porting/mod.md")]
pub mod porting {
    #[doc = include_str!("porting/egui.md")]
    pub mod egui {}

    #[doc = include_str!("porting/react.md")]
    pub mod react {}

    #[doc = include_str!("porting/qt.md")]
    pub mod qt {}

    #[doc = include_str!("porting/web.md")]
    pub mod web {}
}

#[cfg(all(feature = "data", feature = "viz", feature = "pages"))]
#[doc = include_str!("recipes.md")]
pub mod recipes {}
