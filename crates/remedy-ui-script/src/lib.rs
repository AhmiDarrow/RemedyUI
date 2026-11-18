//! RemedyUI scripted widgets: small programs people write and share (a FreeSpeak Home's
//! custom widgets), run as untrusted code.
//!
//! A script is Luau, which was made for running untrusted code. In a browser page
//! (`wasm32`) no Luau machine can be built, and [`ScriptWidget::load`] says why with
//! [`ScriptError::Unsupported`]. Each widget gets a virtual
//! machine of its own, sandboxed: no files, network, processes or loading other code; only
//! the standard library's pure parts, a `ui` table that describes elements, its own `state`,
//! its settings, and the read-only data the host grants (`host`). Every call into it runs
//! against a time budget (an interrupt stops a script that runs past it, even in a tight loop)
//! and the machine against a memory limit. Both are sized from the device ([`Limits`]).
//!
//! A script never touches elements: it returns a [`Node`] tree, which the host builds into
//! ordinary RemedyUI elements, so scripted widgets are themed, accessible and fast like
//! built-in ones. Buttons name handlers ([`Action`]); the host runs one with
//! [`ScriptWidget::act`] and renders again.
//!
//! ```lua
//! return {
//!   name = "Claps",
//!   settings = { { key = "title", label = "Title", kind = "text", default = "Claps" } },
//!   render = function(settings, state, ui, host)
//!     state.claps = state.claps or 0
//!     return ui.column({ gap = 8 }, {
//!       ui.heading(settings.title),
//!       ui.text(state.claps .. " claps"),
//!       ui.button("Clap", function() state.claps = state.claps + 1 end),
//!     })
//!   end,
//! }
//! ```

use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
mod luau;
#[cfg(target_arch = "wasm32")]
mod unsupported;

#[cfg(not(target_arch = "wasm32"))]
pub use luau::ScriptWidget;
#[cfg(target_arch = "wasm32")]
pub use unsupported::ScriptWidget;

/// How much a script may use, sized from the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most memory one widget's machine may hold.
    pub memory: usize,
    /// The longest one call (a render or a handler) may run.
    pub time: Duration,
}

impl Limits {
    /// Limits for a device with `total_memory` bytes and `cores` cores: a widget may use
    /// 1/2048 of the memory (between 4 and 64 MiB), and a call a part of one frame that
    /// shrinks on slower devices (between 4 and 16 ms).
    pub fn for_device(total_memory: u64, cores: usize) -> Limits {
        const MIB: u64 = 1024 * 1024;
        let memory = (total_memory / 2048).clamp(4 * MIB, 64 * MIB);
        let ms = (cores as u64 * 2).clamp(4, 16);
        Limits { memory: usize::try_from(memory).unwrap_or(usize::MAX), time: Duration::from_millis(ms) }
    }
}

impl Default for Limits {
    /// 16 MiB and 8 ms: a mid-range device.
    fn default() -> Self {
        Limits { memory: 16 * 1024 * 1024, time: Duration::from_millis(8) }
    }
}

/// Why a script failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptError {
    /// It does not compile, or is not a widget (what is wrong, for its author).
    Invalid(String),
    /// It failed while running.
    Runtime(String),
    /// It ran past its time budget.
    TooSlow,
    /// It used more memory than allowed.
    TooBig,
    /// What it described is not a widget tree (what is wrong).
    Shape(String),
    /// Scripts cannot run on this platform, and why.
    Unsupported(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Invalid(e) => write!(f, "the script is not a widget: {e}"),
            ScriptError::Runtime(e) => write!(f, "the script failed: {e}"),
            ScriptError::TooSlow => f.write_str("the script took too long"),
            ScriptError::TooBig => f.write_str("the script used too much memory"),
            ScriptError::Shape(e) => write!(f, "the script described something that is not a widget: {e}"),
            ScriptError::Unsupported(e) => write!(f, "scripted widgets cannot run here: {e}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// The kind of a setting, which decides its editor.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    /// One line of text.
    Text,
    /// Several lines.
    LongText,
    /// A number, optionally bounded.
    Number {
        /// The least.
        min: Option<f64>,
        /// The most.
        max: Option<f64>,
    },
    /// On or off.
    Bool,
    /// One of these.
    Choice(Vec<String>),
    /// A color (`#rrggbb`).
    Color,
    /// A web link.
    Link,
}

/// A setting a widget declares: the board's property grid edits it.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    /// Its key in the settings.
    pub key: String,
    /// What people see.
    pub label: String,
    /// Its kind.
    pub kind: SettingKind,
    /// Its value until set.
    pub default: serde_json::Value,
}

/// A handler a button names; the host runs it with [`ScriptWidget::act`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Action(pub usize);

/// How text is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// Body text.
    Body,
    /// Secondary.
    Muted,
    /// Emphasized.
    Strong,
    /// A heading of level 1 to 3.
    Heading(u8),
}

/// What a script describes; the host builds elements from it.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// Children top to bottom.
    Column {
        /// Space between them.
        gap: f32,
        /// The children.
        children: Vec<Node>,
    },
    /// Children side by side, wrapping when narrow.
    Row {
        /// Space between them.
        gap: f32,
        /// The children.
        children: Vec<Node>,
    },
    /// Text.
    Text {
        /// The text.
        text: String,
        /// Its style.
        style: TextStyle,
    },
    /// A button.
    Button {
        /// Its label.
        label: String,
        /// What it runs.
        action: Action,
    },
    /// A web link, opened only after the person agrees (the host asks).
    Link {
        /// Its label.
        label: String,
        /// Where it goes (`https:` or `mailto:` only).
        url: String,
    },
    /// A progress bar, 0 to 1.
    Progress {
        /// What it measures.
        label: String,
        /// How far.
        value: f32,
    },
    /// A line between parts.
    Divider,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    const CLAPS: &str = r#"
return {
  name = "Claps",
  settings = { { key = "title", label = "Title", kind = "text", default = "Claps" },
               { key = "goal", kind = "number", min = 1, max = 100, default = 10 } },
  render = function(settings, state, ui, host)
    state.claps = state.claps or 0
    return ui.column({ gap = 8 }, {
      ui.heading(settings.title),
      ui.text(state.claps .. " of " .. settings.goal),
      ui.progress("Claps", state.claps / settings.goal),
      ui.button("Clap", function() state.claps = state.claps + 1 end),
      ui.muted("Hello, " .. (host.name or "guest")),
    })
  end,
}
"#;

    fn host() -> serde_json::Value {
        serde_json::json!({ "name": "Sam" })
    }

    #[test]
    fn a_widget_renders_takes_settings_and_handles_its_buttons() {
        let mut w = ScriptWidget::load(CLAPS, Limits::default()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(w.name(), "Claps");
        assert_eq!(w.settings().len(), 2);
        assert_eq!(
            w.settings().get(1).map(|s| &s.kind),
            Some(&SettingKind::Number { min: Some(1.0), max: Some(100.0) })
        );
        let mut settings = serde_json::Map::new();
        settings.insert("title".into(), "Party".into());
        let n = w.render(&settings, &host()).unwrap_or_else(|e| unreachable!("{e}"));
        let Node::Column { children, .. } = &n else { unreachable!("{n:?}") };
        assert_eq!(
            children.first(),
            Some(&Node::Text { text: "Party".into(), style: TextStyle::Heading(2) })
        );
        assert_eq!(children.get(1), Some(&Node::Text { text: "0 of 10".into(), style: TextStyle::Body }));
        let Some(Node::Button { action, .. }) = children.get(3) else { unreachable!() };
        w.act(*action).unwrap_or_else(|e| unreachable!("{e}"));
        w.act(*action).unwrap_or_else(|e| unreachable!("{e}"));
        let n = w.render(&settings, &host()).unwrap_or_else(|e| unreachable!("{e}"));
        let Node::Column { children, .. } = &n else { unreachable!() };
        assert_eq!(children.get(1), Some(&Node::Text { text: "2 of 10".into(), style: TextStyle::Body }));
        assert_eq!(children.get(2), Some(&Node::Progress { label: "Claps".into(), value: 0.2 }));
        assert_eq!(children.get(4), Some(&Node::Text { text: "Hello, Sam".into(), style: TextStyle::Muted }));
    }

    #[test]
    fn a_hostile_script_cannot_reach_files_the_network_processes_or_loaders() {
        for probe in [
            "io.open('x')",
            "os.execute('x')",
            "os.remove('x')",
            "require('socket')",
            "dofile('x')",
            "loadfile('x')",
            "package.loadlib('x')",
            "debug.getregistry()",
            "ui.text = nil",
            "string.rep = nil",
        ] {
            let src = format!(
                "return {{ name = 'x', render = function(s, st, ui) {probe} return ui.text('ran') end }}"
            );
            let r = ScriptWidget::load(&src, Limits::default())
                .and_then(|mut w| w.render(&serde_json::Map::new(), &host()));
            assert!(r.is_err(), "{probe} must fail, got {r:?}");
        }
    }

    #[test]
    fn a_script_that_never_stops_is_stopped_in_time() {
        let src = "return { name = 'spin', render = function(s, st, ui) while true do end end }";
        let mut w = ScriptWidget::load(src, Limits { time: Duration::from_millis(20), ..Limits::default() })
            .unwrap_or_else(|e| unreachable!("{e}"));
        let start = Instant::now();
        assert_eq!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::TooSlow));
        assert!(start.elapsed() < Duration::from_millis(500), "{:?}", start.elapsed());
        // Loading code that spins at the top level is stopped too.
        let r = ScriptWidget::load(
            "while true do end",
            Limits { time: Duration::from_millis(20), ..Limits::default() },
        );
        assert_eq!(r.err(), Some(ScriptError::TooSlow));
    }

    #[test]
    fn a_script_that_eats_memory_is_stopped_at_its_limit() {
        let src = "return { name = 'hog', render = function(s, st, ui) local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1000) .. i end end }";
        let limits = Limits { memory: 4 * 1024 * 1024, time: Duration::from_secs(10) };
        let mut w = ScriptWidget::load(src, limits).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::TooBig));
        assert!(w.memory_used() <= limits.memory + 1024 * 1024);
    }

    #[test]
    fn what_is_not_a_widget_is_said_plainly() {
        assert!(matches!(ScriptWidget::load("return 5", Limits::default()), Err(ScriptError::Invalid(_))));
        assert!(matches!(ScriptWidget::load("return {", Limits::default()), Err(ScriptError::Invalid(_))));
        assert!(matches!(
            ScriptWidget::load("return { name = 'x' }", Limits::default()),
            Err(ScriptError::Invalid(_))
        ));
        let deep = "return { name='d', render = function(s, st, ui) local n = ui.text('x') for i = 1, 200 do n = ui.column({}, { n }) end return n end }";
        let mut w = ScriptWidget::load(deep, Limits::default()).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(matches!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::Shape(_))));
        let odd = "return { name='o', render = function(s, st, ui) return { kind = 'iframe' } end }";
        let mut w = ScriptWidget::load(odd, Limits::default()).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(matches!(w.render(&serde_json::Map::new(), &host()), Err(ScriptError::Shape(_))));
    }

    #[test]
    fn limits_follow_the_device() {
        let small = Limits::for_device(2 * 1024 * 1024 * 1024, 2);
        let big = Limits::for_device(64 * 1024 * 1024 * 1024, 16);
        assert_eq!(small.memory, 4 * 1024 * 1024);
        assert_eq!(big.memory, 32 * 1024 * 1024);
        assert_eq!((small.time, big.time), (Duration::from_millis(4), Duration::from_millis(16)));
    }
}
