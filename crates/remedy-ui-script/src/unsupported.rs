//! Scripted widgets in a browser page: unavailable.
//!
//! Luau is C++. Building it for `wasm32-unknown-unknown` needs a C++ standard library for
//! that target, which no toolchain provides (wasi-sdk's targets WASI and brings WASI imports a
//! page does not have), and its error handling (C++ exceptions, or `setjmp`/`longjmp`) needs
//! exception handling that Rust's `wasm32-unknown-unknown` standard library is not built with
//! (it aborts on panic, and stable Rust cannot rebuild it to unwind). mlua, which embeds Luau,
//! supports the web only through Emscripten. No maintained pure-Rust Luau exists (pure-Rust
//! Lua machines run Lua 5.x, whose syntax and sandbox differ from the Luau scripts people
//! share). So here a script never loads, and says why.

use std::convert::Infallible;

use crate::{Action, Limits, Node, ScriptError, SettingSpec};

/// Why scripts cannot run in a browser page.
pub(crate) const REASON: &str = "Luau, the language of scripted widgets, is C++ and cannot be built for a browser page (wasm32-unknown-unknown has no C++ standard library and no exception handling for its errors)";

/// One scripted widget. In a browser page none can be made: [`ScriptWidget::load`] fails
/// with [`ScriptError::Unsupported`].
pub struct ScriptWidget {
    never: Infallible,
}

impl std::fmt::Debug for ScriptWidget {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.never {}
    }
}

impl ScriptWidget {
    /// Fails: scripts cannot run in a browser page.
    pub fn load(source: &str, limits: Limits) -> Result<ScriptWidget, ScriptError> {
        let _ = (source, limits);
        Err(ScriptError::Unsupported(REASON.into()))
    }

    /// The widget's name.
    pub fn name(&self) -> &str {
        match self.never {}
    }

    /// Its settings.
    pub fn settings(&self) -> &[SettingSpec] {
        match self.never {}
    }

    /// Its limits.
    pub fn limits(&self) -> Limits {
        match self.never {}
    }

    /// Describes the widget.
    pub fn render(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        host: &serde_json::Value,
    ) -> Result<Node, ScriptError> {
        let _ = (settings, host);
        match self.never {}
    }

    /// Runs a button's handler.
    pub fn act(&mut self, action: Action) -> Result<(), ScriptError> {
        let _ = action;
        match self.never {}
    }

    /// The memory its machine holds.
    pub fn memory_used(&self) -> usize {
        match self.never {}
    }
}
