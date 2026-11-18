//! WebAssembly test widgets written as text: a minimal widget whose functions a test
//! replaces, and the hostile ones built from it. Shared by the engine's tests and the
//! board's browser test.

#![allow(dead_code)]

/// The manifest every test widget describes.
pub const MANIFEST: &str = r#"{"name":"probe"}"#;
/// What a test widget renders.
pub const SHOWN: &str = r#"{"kind":"text","text":"ok"}"#;
/// What a failing test widget says.
pub const BOOM: &str = "boom: the widget gave up";

/// Where the data segments sit.
const AT_MANIFEST: u32 = 0;
const AT_SHOWN: u32 = 256;
const AT_BOOM: u32 = 512;

/// A place in memory, as the ABI returns it.
pub fn place(at: u32, text: &str) -> i64 {
    ((u64::from(at) << 32) | text.len() as u64) as i64
}

fn data(at: u32, text: &str) -> String {
    format!("(data (i32.const {at}) \"{}\")", text.replace('"', "\\\""))
}

/// The function bodies of a test widget.
pub struct Parts<'a> {
    /// `remedy_describe`: `() -> i64`.
    pub describe: &'a str,
    /// `remedy_render`: `(i32, i32) -> i64`.
    pub render: &'a str,
    /// `remedy_act`: `(i32, i32)`.
    pub act: &'a str,
    /// More module fields (functions, imports, globals).
    pub extra: &'a str,
    /// The memory's size in 64 KiB pages.
    pub pages: u32,
}

impl Default for Parts<'_> {
    fn default() -> Self {
        Parts { describe: "", render: "", act: "", extra: "", pages: 1 }
    }
}

/// The body that returns the manifest.
pub fn describe_ok() -> String {
    format!("(i64.const {})", place(AT_MANIFEST, MANIFEST))
}

/// The body that returns [`SHOWN`].
pub fn render_ok() -> String {
    format!("(i64.const {})", place(AT_SHOWN, SHOWN))
}

/// A test widget's text, each missing body the well-behaved one.
pub fn wat(parts: &Parts<'_>) -> String {
    let describe = if parts.describe.is_empty() { describe_ok() } else { parts.describe.to_owned() };
    let render = if parts.render.is_empty() { render_ok() } else { parts.render.to_owned() };
    format!(
        r#"(module
  {extra}
  (memory (export "memory") {pages})
  {manifest}
  {shown}
  {boom}
  (func (export "remedy_alloc") (param i32) (result i32) (i32.const 4096))
  (func (export "remedy_describe") (result i64) {describe})
  (func (export "remedy_render") (param i32 i32) (result i64) {render})
  (func (export "remedy_act") (param i32 i32) {act})
)"#,
        extra = parts.extra,
        pages = parts.pages,
        manifest = data(AT_MANIFEST, MANIFEST),
        shown = data(AT_SHOWN, SHOWN),
        boom = data(AT_BOOM, BOOM),
        act = parts.act,
    )
}

/// A test widget's module.
pub fn module(parts: &Parts<'_>) -> Vec<u8> {
    wat::parse_str(wat(parts)).unwrap_or_else(|e| unreachable!("the test widget is valid: {e}"))
}

/// A widget whose render never stops.
pub fn spin() -> Vec<u8> {
    module(&Parts { render: "(loop $l (br $l)) (i64.const 0)", ..Parts::default() })
}

/// A widget that spins while it describes itself (it never loads).
pub fn spin_on_load() -> Vec<u8> {
    module(&Parts { describe: "(loop $l (br $l)) (i64.const 0)", ..Parts::default() })
}

/// A widget whose render recurses without end.
pub fn recurse() -> Vec<u8> {
    module(&Parts {
        extra: "(func $down (param i32) (result i32) (call $down (i32.add (local.get 0) (i32.const 1))))",
        render: &format!("(drop (call $down (i32.const 0))) {}", render_ok()),
        ..Parts::default()
    })
}

/// A widget whose render grows its memory until it is refused, and then gives up, as an
/// allocator does.
pub fn hog() -> Vec<u8> {
    module(&Parts {
        render: "(loop $l (br_if $l (i32.ne (memory.grow (i32.const 16)) (i32.const -1)))) unreachable",
        ..Parts::default()
    })
}

/// A widget that asks for more memory than it may have before it runs at all.
pub fn huge_at_start() -> Vec<u8> {
    module(&Parts { pages: 4096, ..Parts::default() })
}

/// A widget that reports a failure through `remedy::fail`.
pub fn fails() -> Vec<u8> {
    module(&Parts {
        extra: r#"(import "remedy" "fail" (func $fail (param i32 i32)))"#,
        render: &format!("(call $fail (i32.const {AT_BOOM}) (i32.const {})) (i64.const 0)", BOOM.len()),
        ..Parts::default()
    })
}

/// A widget that logs through `remedy::log` and renders.
pub fn logs() -> Vec<u8> {
    module(&Parts {
        extra: r#"(import "remedy" "log" (func $log (param i32 i32)))"#,
        render: &format!("(call $log (i32.const {AT_BOOM}) (i32.const {})) {}", BOOM.len(), render_ok()),
        ..Parts::default()
    })
}

/// Widgets that import what the sandbox does not give: files and clocks through WASI, a
/// socket, processes, a host function that does not exist, and the host's memory.
pub fn reaching_out() -> Vec<(&'static str, Vec<u8>)> {
    [
        (
            "WASI files",
            r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
        ),
        (
            "WASI clocks",
            r#"(import "wasi_snapshot_preview1" "clock_time_get" (func (param i32 i64 i32) (result i32)))"#,
        ),
        ("a socket", r#"(import "env" "socket_connect" (func (param i32 i32) (result i32)))"#),
        ("a process", r#"(import "env" "system" (func (param i32) (result i32)))"#),
        ("a missing host function", r#"(import "remedy" "open_file" (func (param i32 i32) (result i32)))"#),
        ("the host's memory", r#"(import "remedy" "log" (memory 1))"#),
    ]
    .into_iter()
    .map(|(what, extra)| (what, module(&Parts { extra, ..Parts::default() })))
    .collect()
}

/// A widget with a start function.
pub fn starts() -> Vec<u8> {
    module(&Parts { extra: "(func $s) (start $s)", ..Parts::default() })
}

/// A widget that renders until its button is pressed, and spins in every render after.
pub fn spins_after_press() -> Vec<u8> {
    let shown = r#"{"kind":"button","label":"Spin","on":1}"#;
    module(&Parts {
        extra: &format!("(global $pressed (mut i32) (i32.const 0)) {}", data(1024, shown)),
        render: &format!(
            "(if (global.get $pressed) (then (loop $l (br $l)))) (i64.const {})",
            place(1024, shown)
        ),
        act: "(global.set $pressed (i32.const 1))",
        ..Parts::default()
    })
}

/// A widget whose render returns `text` (placed past the other segments).
pub fn renders(text: &str) -> Vec<u8> {
    module(&Parts {
        extra: &data(8192, text),
        render: &format!("(i64.const {})", place(8192, text)),
        pages: 2,
        ..Parts::default()
    })
}
