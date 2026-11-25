//! RemedyUI's engine: display lists, the view tree and its elements, the compositor, the CPU and GPU renderers, effects and sandboxed scripts.
//!
//! Each part is a module behind a Cargo feature of the same name.

#[cfg(feature = "compositor")]
pub mod compositor;
#[cfg(feature = "effects")]
pub mod effects;
#[cfg(feature = "media-effects")]
pub mod media_effects;
#[cfg(feature = "paint")]
pub mod paint;
#[cfg(feature = "render-cpu")]
pub mod render_cpu;
#[cfg(feature = "render-gpu")]
pub mod render_gpu;
#[cfg(feature = "script")]
pub mod script;
#[cfg(feature = "script-lua")]
pub mod script_lua;
#[cfg(feature = "script-wasm")]
pub mod script_wasm;
#[cfg(feature = "view")]
pub mod view;
