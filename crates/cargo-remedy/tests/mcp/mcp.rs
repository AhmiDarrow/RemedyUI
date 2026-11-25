//! remedy-mcp end to end: the server runs as a child process over stdio and a scripted MCP
//! client lists and calls its tools: the docs and markup tools, a real preview of a real app
//! (a PNG with the expected pixels), its tests, and a running app operated through its agents
//! listener under grants made here (a read-only grant cannot act, a wrong token is refused,
//! a confirming grant waits for the person). Also the HTTP transport's token, `--tools`
//! selection, and a build with only the docs tools.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use remedy_ui::TextField;
use remedy_ui::a11y::Role;
use remedy_ui::agents::{Listener, Scope};
use remedy_ui::devtools::{RemoteOptions, RemoteServer};
use remedy_ui::nav::Commands;
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, GetPromptRequestParams, ReadResourceRequestParams,
};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

/// Unwraps what a test needs, failing it with `why`.
trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

fn workspace_root() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")).must("the checkout")
}

fn plain(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s).replace('\\', "/")
}

/// A folder for this test's files, persisting between runs (so the fixture app's build is
/// incremental) under the target directory.
fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("remedy-mcp").join(name);
    std::fs::create_dir_all(&d).must("scratch folder");
    // Without Windows' verbatim prefix, which the MSVC linker does not take everywhere.
    PathBuf::from(plain(&std::fs::canonicalize(&d).must("canonical")))
}

struct Mcp {
    rt: tokio::runtime::Runtime,
    client: RunningService<RoleClient, ()>,
}

impl Mcp {
    fn start(args: &[String], env: &[(&str, String)]) -> Mcp {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().must("runtime");
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_remedy-mcp"));
        cmd.args(args);
        // The builds the tools start (previews, tests) carry no debug info: smaller targets.
        cmd.env("CARGO_PROFILE_DEV_DEBUG", "0").env("CARGO_PROFILE_TEST_DEBUG", "0");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let client = rt.block_on(async {
            let transport = TokioChildProcess::new(cmd).must("start the server");
            ().serve(transport).await.must("initialize")
        });
        Mcp { rt, client }
    }

    fn call(&self, name: &str, args: Value) -> CallToolResult {
        let Value::Object(map) = args else { unreachable!("arguments are an object") };
        let params = CallToolRequestParams::new(name.to_owned()).with_arguments(map);
        self.rt.block_on(self.client.call_tool(params)).must(name)
    }

    fn tools(&self) -> Vec<String> {
        self.rt
            .block_on(self.client.list_all_tools())
            .must("tools")
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
    }
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ok(r: &CallToolResult, what: &str) -> String {
    let t = text(r);
    assert_ne!(r.is_error, Some(true), "{what} failed: {t}");
    t
}

fn failed(r: &CallToolResult) -> String {
    assert_eq!(r.is_error, Some(true), "expected an error, got: {}", text(r));
    text(r)
}

fn unbase64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let mut out = Vec::new();
    for chunk in s.as_bytes().chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, c)| n | (val(*c) << (18 - 6 * i as u32)));
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let pad = chunk.iter().filter(|c| **c == b'=').count();
        out.extend(bytes.iter().take(3 - pad));
    }
    out
}

#[test]
fn docs_markup_and_tool_listing_over_stdio() {
    let ws = scratch("docs-ws");
    let mcp = Mcp::start(&["--workspace".into(), plain(&ws)], &[]);
    let tools = mcp.tools();
    for t in [
        "docs_search",
        "docs_read",
        "widget_catalog",
        "rui_check",
        "rui_compile",
        "new_app",
        "render_preview",
        "a11y_audit",
        "run_tests",
        "budget_check",
        "app_connect",
        "app_snapshot",
        "app_act",
        "app_command",
        "app_undo",
        "app_disconnect",
        "devtools_connect",
        "devtools_tree",
        "devtools_element",
        "devtools_signals",
        "devtools_timings",
    ] {
        assert!(tools.iter().any(|x| x == t), "{t} is offered: {tools:?}");
    }

    let hits = mcp.call("docs_search", json!({"query": "agents consent token undo"}));
    let t = ok(&hits, "docs_search");
    assert!(
        t.contains("docs/design/11-accessibility-and-automation.md") || t.contains("remedy-ui-automation"),
        "{t}"
    );
    let structured = hits.structured_content.clone().must("structured hits");
    assert!(structured.get("result").and_then(Value::as_array).is_some_and(|a| !a.is_empty()));
    let read = ok(&mcp.call("docs_read", json!({"path": "docs/design/11-accessibility-and-automation.md", "heading": "Automation by agents"})), "docs_read");
    assert!(read.contains("Consent") && !read.contains("## Semantics"), "{read}");
    assert!(failed(&mcp.call("docs_read", json!({"path": "nope.md"}))).contains("no document"));

    let cat =
        ok(&mcp.call("widget_catalog", json!({"name": "switch", "feature": "widgets"})), "widget_catalog");
    assert!(
        cat.contains("remedy_ui::widgets::switch")
            && cat.contains("value: Signal<bool>")
            && cat.contains("sheet rules: switch"),
        "{cat}"
    );

    // Deprecated builders are left out: their replacements are in the catalog.
    assert!(failed(&mcp.call("widget_catalog", json!({"name": "button_dyn"}))).contains("no widget matches"));
    let button = ok(&mcp.call("widget_catalog", json!({"name": "button", "feature": "widgets"})), "button");
    assert!(button.contains("call: remedy_ui::widgets::button(ui, \"Label\", || {})"), "{button}");

    let good = mcp.call("rui_check", json!({"source": "column(gap: s2) {\n  switch(label: \"Noise\", checked: $noise)\n  button(label: \"Save\", on_press: !save)\n}\n"}));
    let t = ok(&good, "rui_check");
    assert!(t.contains("signal $noise: flag (Signal<bool>)") && t.contains("command !save"), "{t}");
    let bad = ok(
        &mcp.call(
            "rui_check",
            json!({"source": "column {\n  slider(label: \"Gain\", value: $gain)\n}\n", "file": "voice.rui"}),
        ),
        "rui_check",
    );
    assert!(bad.starts_with("error: voice.rui:2:3:") && bad.contains("range"), "{bad}");
    std::fs::write(ws.join("screen.rui"), "row { fretboard(strings: 6) }\n").must("write markup");
    let custom = json!({"path": "screen.rui", "elements": [{"name": "fretboard", "params": [{"name": "strings", "kind": "count"}], "path": "crate::ui::fretboard"}]});
    assert_eq!(ok(&mcp.call("rui_check", custom.clone()), "custom check"), "ok");
    let rust = ok(&mcp.call("rui_compile", custom), "rui_compile");
    assert!(rust.contains("crate::ui::fretboard") && rust.contains("pub struct Bindings"), "{rust}");
    let outside = mcp.call("rui_check", json!({"path": plain(&workspace_root().join("README.md"))}));
    assert!(failed(&outside).contains("outside the workspace"));

    let prompt =
        mcp.rt
            .block_on(mcp.client.get_prompt(
                GetPromptRequestParams::new("build_screen").with_arguments(
                    serde_json::from_value(json!({"screen": "a settings screen"})).must("args"),
                ),
            ))
            .must("prompt");
    assert!(format!("{:?}", prompt.messages).contains("a settings screen"));
    let resources = mcp.rt.block_on(mcp.client.list_all_resources()).must("resources");
    let design = resources
        .iter()
        .find(|r| r.uri == "remedy-docs://docs/DESIGN.md")
        .must("the design overview is a resource");
    let read = mcp
        .rt
        .block_on(mcp.client.read_resource(ReadResourceRequestParams::new(design.uri.clone())))
        .must("read");
    assert!(format!("{:?}", read.contents).contains("RemedyUI"));
}

#[test]
fn run_time_tool_selection_and_a_docs_only_build() {
    let mcp = Mcp::start(&["--tools".into(), "docs,live".into()], &[]);
    let tools = mcp.tools();
    assert!(tools.contains(&"docs_search".to_owned()) && tools.contains(&"app_connect".to_owned()));
    assert!(!tools.iter().any(|t| t.starts_with("rui_") || t == "render_preview"), "{tools:?}");

    // The docs tools alone build without the engine's renderer, previews or networking.
    let target = scratch("minimal-build");
    let built = Command::new(env!("CARGO"))
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .args([
            "build",
            "-p",
            "cargo-remedy",
            "--bin",
            "remedy-mcp",
            "--no-default-features",
            "--features",
            "mcp,mcp-docs",
            "--target-dir",
        ])
        .arg(&target)
        .current_dir(workspace_root())
        .output()
        .must("cargo build");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let exe = target.join("debug").join(format!("remedy-mcp{}", std::env::consts::EXE_SUFFIX));
    let list = Command::new(&exe).arg("--list-tools").output().must("list");
    let listed = String::from_utf8_lossy(&list.stdout).into_owned();
    assert!(
        listed.contains("docs_search") && !listed.contains("app_connect") && !listed.contains("rui_check"),
        "{listed}"
    );
    let refused = Command::new(&exe).args(["--tools", "live", "--list-tools"]).output().must("run");
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not in this build"));
    let tree = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "cargo-remedy",
            "--no-default-features",
            "--features",
            "mcp,mcp-docs",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .current_dir(workspace_root())
        .output()
        .must("cargo tree");
    let deps = String::from_utf8_lossy(&tree.stdout).into_owned();
    for heavy in ["remedy-ui-engine", "remedy-ui-widgets", "hyper ", "remedy-ui "] {
        assert!(!deps.contains(heavy), "the docs-only build does not pull {heavy}");
    }
}

#[test]
fn http_needs_the_token() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_remedy-mcp"))
        .args(["--http", "0", "--tools", "docs"])
        .env("REMEDY_MCP_TOKEN", "test-token-1234")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .must("start");
    let mut err = BufReader::new(child.stderr.take().must("stderr"));
    let mut line = String::new();
    err.read_line(&mut line).must("the server says where it listens");
    let url = line.trim().rsplit(' ').next().must("a URL").to_owned();
    let addr = url.trim_start_matches("http://").trim_end_matches("/mcp").to_owned();
    let post = |token: Option<&str>| -> String {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        let mut s = std::net::TcpStream::connect(&addr).must("connect");
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        write!(
            s,
            "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .must("send");
        s.set_read_timeout(Some(Duration::from_secs(30))).must("timeout");
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        // Read until the answer's JSON shows up or the server closes.
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend(buf.iter().take(n));
            let so_far = String::from_utf8_lossy(&out);
            if so_far.contains("serverInfo")
                || so_far.starts_with("HTTP/1.1 401") && so_far.contains("required")
            {
                break;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    let none = post(None);
    assert!(none.starts_with("HTTP/1.1 401"), "{none}");
    assert!(post(Some("wrong")).starts_with("HTTP/1.1 401"));
    let yes = post(Some("test-token-1234"));
    assert!(yes.starts_with("HTTP/1.1 200") && yes.contains("remedy-mcp"), "{yes}");
    let _ = child.kill();
    let _ = child.wait();
}

include!("../devkit/fixture/setup.rs");

#[test]
fn previews_audits_tests_and_new_apps() {
    let ws = fixture_workspace(&workspace_root());
    let mcp = Mcp::start(&["--workspace".into(), plain(&ws)], &[]);

    let r = mcp.call("render_preview", json!({"app": "fixture-app", "function": "home", "width": 200, "height": 160, "save_png": "out/home.png"}));
    let t = ok(&r, "render_preview");
    assert!(t.starts_with("200×160 px, theme forest (dark), locale en-US"), "{t}");
    assert!(t.contains("Save"), "the screen reader view names the button: {t}");
    let png_b64 = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Image(i) if i.mime_type == "image/png" => Some(i.data.clone()),
            _ => None,
        })
        .must("a PNG image");
    let bytes = unbase64(&png_b64);
    assert_eq!(std::fs::read(ws.join("out").join("home.png")).must("saved PNG"), bytes);
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().must("a PNG");
    let mut buf = vec![0u8; reader.output_buffer_size().must("size")];
    let info = reader.next_frame(&mut buf).must("frame");
    assert_eq!((info.width, info.height), (200, 160));
    let px = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * 200 + x) * 4;
        buf.get(i..i + 4).and_then(|p| p.try_into().ok()).must("pixel")
    };
    // The corner is Dark Forest's background, #0a0e0b; the button is drawn somewhere else.
    let corner = px(1, 158);
    assert!(corner.iter().zip([10u8, 14, 11, 255]).all(|(a, b)| a.abs_diff(b) <= 2), "{corner:?}");
    let drawn =
        (0..160).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|(x, y)| px(*x, *y) != corner).count();
    assert!(drawn > 200, "the screen drew text and a button ({drawn} pixels)");

    let light = ok(&mcp.call("render_preview", json!({"app": "fixture-app", "function": "screens::about", "theme": "light", "locale": "ar-EG", "scale": 2})), "light rtl");
    assert!(
        light.contains("theme light (light), locale ar-EG, right to left") && light.starts_with("800×600 px"),
        "{light}"
    );
    let missing = failed(&mcp.call("render_preview", json!({"app": "fixture-app", "function": "nowhere"})));
    assert!(missing.contains("did not build") && missing.contains("nowhere"), "{missing}");
    assert!(
        failed(
            &mcp.call("render_preview", json!({"app": "fixture-app", "function": "home", "theme": "plaid"}))
        )
        .contains("no theme")
    );
    assert!(
        failed(&mcp.call("render_preview", json!({"app": "fixture-app", "function": "home(ui); evil()"})))
            .contains("not a function path")
    );

    let listing = ws.join("listing");
    std::fs::create_dir_all(listing.join("src")).must("listing app");
    std::fs::write(listing.join("Cargo.toml"), "[package]\nname = \"listing\"\nversion = \"0.1.0\"\n")
        .must("manifest");
    std::fs::write(
        listing.join("src").join("lib.rs"),
        "#[preview(theme = \"forest\")]\npub fn home_preview(ui: &Ui) -> El { home(ui) }\n",
    )
    .must("source");
    let listed = ok(&mcp.call("list_previews", json!({"app": "listing"})), "list_previews");
    assert!(
        listed.contains("home_preview") && listed.contains("#[preview(theme = \"forest\")] at src/lib.rs:2"),
        "{listed}"
    );

    let audit = ok(&mcp.call("a11y_audit", json!({"app": "fixture-app", "function": "home"})), "a11y_audit");
    assert!(audit.contains("Control with no accessible name") && audit.contains("lib.rs:15"), "{audit}");

    let tests = mcp.call("run_tests", json!({"app": "fixture-app"}));
    let summary = tests.structured_content.clone().must("a summary");
    assert_eq!(summary.get("passed").and_then(Value::as_u64), Some(1), "{}", text(&tests));
    assert_eq!(summary.get("failed").and_then(Value::as_u64), Some(1));
    assert!(text(&tests).contains("this one fails on purpose"));
    let only = mcp.call("run_tests", json!({"app": "fixture-app", "filter": "adds"}));
    assert_eq!(only.structured_content.and_then(|s| s.get("success").and_then(Value::as_bool)), Some(true));

    let budgets = ok(&mcp.call("budget_check", json!({"list_only": true})), "budget_check");
    assert!(budgets.contains("zoom_budget"), "{budgets}");

    let made = mcp.call("new_app", json!({"name": "my-notes", "folder": "apps/my-notes", "template": "basic", "remedy_path": plain(&workspace_root())}));
    let t = ok(&made, "new_app");
    assert!(t.contains("src/lib.rs") && t.contains("tests/app.rs") && t.contains("i18n/en-US.ftl"), "{t}");
    assert!(ws.join("apps/my-notes/Cargo.toml").is_file());
    assert!(
        failed(&mcp.call("new_app", json!({"name": "my-notes", "folder": "apps/my-notes"})))
            .contains("not empty")
    );
    assert!(
        failed(&mcp.call("new_app", json!({"name": "x", "folder": "../escape"})))
            .contains("outside the workspace")
    );
    let _ = std::fs::remove_dir_all(ws.join("apps"));
}

/// An app that opened its window to agents and to remote devtools, as an app does: through
/// `Window::attach_agents` and `Window::serve_devtools`, on the in-memory host.
struct LiveApp {
    app: RefCell<TestApp>,
    noise: Signal<bool>,
    name: TextField,
    deleted: Rc<Cell<u32>>,
    act: String,
    read: String,
    address: String,
    devtools: (String, String),
    _serving: (Listener, RemoteServer),
}

fn live_app() -> LiveApp {
    let noise = Signal::new(false);
    let deleted = Rc::new(Cell::new(0));
    type Opened = (TextField, String, String, Listener, RemoteServer);
    let opened: Rc<RefCell<Option<Opened>>> = Rc::default();
    let (o2, d2) = (opened.clone(), deleted.clone());
    let app = TestApp::window(move |ui, window| {
        let commands = Commands::new();
        commands.add("chat.delete", "Delete chat", Vec::new(), None, move || d2.set(d2.get() + 1));
        commands.set_destructive("chat.delete", true);
        let field = w::text_input(ui, "Display name");
        field.set_value("Sam");
        ui.root().child(
            w::column(ui)
                .child(w::switch(ui, "Noise suppression", noise))
                .child(field.el())
                .child(w::secret_input(ui, "Password").el()),
        );
        let agents = window.attach_agents(Some(&commands)).must("the window is open");
        let act = agents.grant("assistant", Scope::Act, None);
        let read = agents.grant("reader", Scope::Read, None);
        let listener = agents.listen().must("listening for agents");
        let devtools = window.serve_devtools(RemoteOptions::default()).must("serving devtools");
        *o2.borrow_mut() = Some((field, act, read, listener, devtools));
    });
    let (name, act, read, listener, devtools) = opened.borrow_mut().take().must("the window was built");
    LiveApp {
        app: RefCell::new(app),
        noise,
        name,
        deleted,
        act,
        read,
        address: listener.addr().to_string(),
        devtools: (devtools.addr().to_string(), devtools.token().to_owned()),
        _serving: (listener, devtools),
    }
}

impl LiveApp {
    /// Runs `call` on another thread while the app runs frames, as its loop would.
    fn during<T: Send + 'static>(&self, call: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = channel();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(call());
        });
        let start = Instant::now();
        loop {
            self.app.borrow_mut().frames(1);
            if let Ok(v) = rx.try_recv() {
                let _ = worker.join();
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(120), "the call finished in time");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The person presses a button of their own (in the trusted layer), once it is there.
    fn person_presses(&self, name: &str) {
        let start = Instant::now();
        while !self.app.borrow_mut().press(Role::Button, name) {
            assert!(start.elapsed() < Duration::from_secs(30), "{name} appeared");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn find_id(snapshot: &Value, name: &str) -> Option<u64> {
    fn walk(nodes: &[Value], name: &str) -> Option<u64> {
        for n in nodes {
            if n.get("name").and_then(Value::as_str) == Some(name) {
                return n.get("id").and_then(Value::as_u64);
            }
            if let Some(found) =
                walk(n.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default(), name)
            {
                return Some(found);
            }
        }
        None
    }
    let tree = snapshot.get("tree").or_else(|| snapshot.get("snapshot").and_then(|s| s.get("tree")))?;
    walk(tree.as_array()?, name)
}

#[test]
fn a_running_app_is_operated_under_consent_and_its_devtools_are_read() {
    let live = live_app();
    let mcp = Arc::new(Mcp::start(&["--tools".into(), "live,devtools".into()], &[]));
    let call = |name: &'static str, args: Value| {
        let m = mcp.clone();
        live.during(move || m.call(name, args))
    };
    let addr = live.address.clone();

    // A wrong token is refused by the app.
    let wrong = call("app_connect", json!({"address": addr, "token": "0123456789abcdef0123456789abcdef"}));
    assert!(failed(&wrong).contains("no access"), "{}", text(&wrong));

    // A read-only grant sees but cannot act.
    let reader = call("app_connect", json!({"address": addr, "token": live.read, "connection": "reader"}));
    let seen = ok(&reader, "read connect");
    assert!(seen.contains("switch \"Noise suppression\" [off] {click"), "{seen}");
    assert!(
        seen.contains("Password") && !seen.contains("password_input \"Password\" ="),
        "a secret field shows, its text never"
    );
    let switch =
        find_id(&reader.structured_content.clone().must("snapshot"), "Noise suppression").must("switch id");
    let refused = call("app_act", json!({"connection": "reader", "node": switch, "action": "click"}));
    assert!(ok(&refused, "act").contains("may only read"));
    assert_eq!(
        refused.structured_content.as_ref().and_then(|s| s.get("refused")).and_then(Value::as_str),
        Some("read_only")
    );
    assert_eq!(live.noise.get_untracked().ok(), Some(false));

    // An acting grant acts, by name in the undo history, and undoes.
    ok(&call("app_connect", json!({"address": addr, "token": live.act})), "connect");
    assert_eq!(ok(&call("app_act", json!({"node": switch, "action": "click"})), "click"), "done");
    assert_eq!(live.noise.get_untracked().ok(), Some(true), "the switch turned on");
    let field =
        find_id(&call("app_snapshot", json!({})).structured_content.clone().must("snapshot"), "Display name")
            .must("field");
    assert_eq!(
        ok(&call("app_act", json!({"node": field, "action": "set_value", "value": "Alex"})), "set"),
        "done"
    );
    assert_eq!(live.name.value(), "Alex");
    let snap = ok(&call("app_snapshot", json!({})), "snapshot");
    assert!(snap.contains("undo: Undo Set Display name to Alex, by assistant"), "{snap}");
    assert_eq!(ok(&call("app_undo", json!({})), "undo"), "done");
    assert_eq!(live.name.value(), "Sam");

    // A destructive command asks the person; the answer comes later.
    let asking = call("app_command", json!({"id": "chat.delete", "wait_seconds": 0}));
    assert!(ok(&asking, "command").contains("asking the person"));
    assert_eq!(live.deleted.get(), 0);
    assert!(
        failed(&call("app_act", json!({"node": switch, "action": "click"}))).contains("has not answered")
    );
    live.person_presses("Allow");
    let decided = call("app_wait", json!({"wait_seconds": 30}));
    assert!(ok(&decided, "wait").contains("the person allowed it"), "{}", text(&decided));
    assert_eq!(live.deleted.get(), 1);
    assert!(ok(&call("app_disconnect", json!({})), "disconnect").contains("disconnected"));
    assert!(failed(&call("app_snapshot", json!({}))).contains("no connection `app`"));

    // Devtools: refused with a wrong token; then the tree, semantics, an element, timings.
    let (dev_addr, dev_token) = live.devtools.clone();
    assert!(
        failed(&call("devtools_connect", json!({"address": dev_addr, "token": "nope"}))).contains("refused")
    );
    ok(&call("devtools_connect", json!({"address": dev_addr, "token": dev_token})), "devtools connect");
    let tree = call("devtools_tree", json!({}));
    let tt = ok(&tree, "tree");
    assert!(tt.contains('#'), "{tt}");
    let first = tree
        .structured_content
        .as_ref()
        .and_then(|s| s.get("elements"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|e| e.get("widget").and_then(Value::as_str) == Some("switch")))
        .and_then(|e| e.get("id"))
        .and_then(Value::as_u64)
        .must("the switch element");
    assert!(
        ok(&call("devtools_element", json!({"id": first})), "element").contains(&format!("element #{first}"))
    );
    assert!(ok(&call("devtools_semantics", json!({})), "semantics").contains("Noise suppression"));
    ok(&call("devtools_signals", json!({"id": first})), "signals");
    ok(&call("devtools_timings", json!({})), "timings");
    assert!(ok(&call("devtools_disconnect", json!({})), "disconnect").contains("disconnected"));
}

#[test]
fn effects_are_checked_and_rendered_for_agents() {
    let ws = scratch("effects-ws");
    let mcp = Mcp::start(&["--workspace".into(), plain(&ws)], &[]);
    let tools = mcp.tools();
    for t in ["rfx_check", "rfx_render", "rfx_library"] {
        assert!(tools.iter().any(|x| x == t), "{t} is offered: {tools:?}");
    }
    let lib = ok(&mcp.call("rfx_library", json!({})), "rfx_library");
    assert!(lib.contains("fire (cost") && lib.contains("shimmer"), "{lib}");
    // A slip of the keyboard: the problem under its line, with a suggestion.
    let bad = ok(
        &mcp.call("rfx_check", json!({"source": "effect e {\n    pass p { out = vec4(fmb(uv, 3)); }\n}\n"})),
        "rfx_check",
    );
    assert!(bad.contains("did you mean `fbm`?") && bad.contains("2 |     pass p"), "{bad}");
    let good = ok(&mcp.call("rfx_check", json!({"name": "fire", "untrusted": true})), "rfx_check");
    assert!(good.starts_with("ok: effect `fire`") && good.contains("pass flame (shade)"), "{good}");
    let r = mcp.call("rfx_render", json!({"name": "plasma", "width": 160, "height": 100, "time": 1.0}));
    let t = ok(&r, "rfx_render");
    assert!(t.contains("at t = 1 s"), "{t}");
    let png_b64 = r
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Image(i) if i.mime_type == "image/png" => Some(i.data.clone()),
            _ => None,
        })
        .must("a PNG image");
    let reader = png::Decoder::new(std::io::Cursor::new(unbase64(&png_b64))).read_info().must("a PNG");
    // The element's size (plasma draws at a lower resolution, stretched over it).
    assert_eq!((reader.info().width, reader.info().height), (160, 100));
    assert!(t.contains("the effect drew at") && !t.contains("drew at 160 x 100"), "{t}");
    // A file outside the workspace stays out of reach.
    let outside = mcp.call("rfx_check", json!({"path": plain(&workspace_root().join("README.md"))}));
    assert!(failed(&outside).contains("outside the workspace"));
}
