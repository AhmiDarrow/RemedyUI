//! `.rui` markup: screens described as data. Rust builders stay the primary API; markup
//! earns its place for live editing, files designers can change without a Rust toolchain,
//! and people rearranging the apps they use.
//!
//! - [`syntax`]: the language and its parser, with errors at file, line and column.
//! - [`spec`]: what each element takes, shared by everything below.
//! - [`build`] (in build scripts): compiles `.rui` files to Rust, so release builds pay
//!   nothing for markup.
//! - [`Scope`], [`Widgets`], [`build_markup`] (feature `runtime`): build a file at run time
//!   from what the app exposes to it.
//! - [`Layouts`] (feature `runtime`): people's own versions of an app's screens, falling
//!   back to the built-in screen with a notice when theirs does not load.
//!
//! A file can only bind the signals (`$name`) and commands (`!name`) the app exposes for
//! it, tokens and translations. It cannot run code, read files or reach the network, so a
//! stranger's file is as safe to load as any other data.

pub mod codegen;
pub mod spec;
pub mod syntax;

pub use codegen::{Custom, build};
pub use syntax::{MAX_DEPTH, Markup, Pos, RuiError, max_depth, set_max_depth};

/// The words a person's layout notice shows, in every language the engine ships, from
/// `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_engine::view::strings::add_messages`] under the name `rui`.
#[cfg(feature = "rui-runtime")]
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("rui");

#[cfg(feature = "rui-runtime")]
mod interp;
#[cfg(feature = "rui-runtime")]
mod layouts;
#[cfg(feature = "rui-runtime")]
pub mod rt;

#[cfg(feature = "rui-runtime")]
pub use interp::{Binding, Bound, Builder, Call, Scope, Widgets, build as build_markup, check};
#[cfg(feature = "rui-runtime")]
pub use layouts::{Layout, Layouts, SaveError, WeakLayout};
