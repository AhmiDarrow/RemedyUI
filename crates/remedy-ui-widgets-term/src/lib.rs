//! Terminals for RemedyUI apps: VT and xterm emulation on a screen of cells, scrollback on a
//! virtual list, selection and copying, links, input methods and true color. The app owns
//! the PTY: it feeds the program's output in ([`Terminal::feeder`], from any thread) and
//! writes what the terminal sends ([`Terminal::on_output`]) to the program.
//!
//! - [`Emulator`]: the screen model alone (primary and alternate screens, scroll regions,
//!   SGR, modes, OSC titles, hyperlinks and clipboard requests), for tests and headless use.
//! - [`terminal`]: the element.
//! - [`encode_key`], [`encode_paste`], [`encode_mouse`]: what keys, pastes and the pointer
//!   send, as xterm encodes them.
//! - [`ansi`], [`background`], [`foreground`]: the colors, from the theme's `term-*` tokens.
//!
//! Layer 5: built only from public engine APIs and `widgets-core`.

mod boxdraw;
mod cell;
mod keys;
mod palette;
mod shared;
mod sixel;
mod term;
mod view;

pub use cell::{Attrs, Cell, Line, Picture, PicturePiece, PromptMark, TermColor, Underline};
pub use keys::{MouseAction, MouseButton, encode_focus, encode_key, encode_mouse, encode_paste};
pub use palette::{ansi, background, cursor, foreground};
pub use shared::Feeder;
pub use term::{
    CursorShape, CursorState, Emulator, Modes, MouseEncoding, MouseMode, Scrollback, default_string_limit,
};
pub use view::{GridSize, Terminal, TerminalOptions, terminal};
