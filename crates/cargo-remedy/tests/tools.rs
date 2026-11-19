//! The developer-tool commands (`cargo remedy docs`, `widgets`, `rui`, `preview`, `audit`,
//! `test`, `budgets`, `app`, `devtools`), run as the person would run them: the program in a
//! subprocess, against temporary folders, the fixture app, and a window on this thread with an
//! agents listener and a devtools server.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::rc::Rc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use remedy_ui::agents::{Listener, Scope};
use remedy_ui::devtools::{RemoteOptions, RemoteServer};
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use serde_json::Value;

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

include!("../../remedy-tools/tests/fixture/setup.rs");

fn checkout() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")).must("the checkout")
}

fn remedy(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cargo-remedy"));
    c.arg("remedy").args(args);
    // The builds the commands start (previews, tests) carry no debug info: smaller targets.
    c.env("CARGO_PROFILE_DEV_DEBUG", "0").env("CARGO_PROFILE_TEST_DEBUG", "0");
    c
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn passes(c: &mut Command) -> String {
    let o = c.output().must("run cargo-remedy");
    assert!(o.status.success(), "{c:?} failed:\n{}\n{}", out(&o), err(&o));
    out(&o)
}

fn fails(c: &mut Command) -> String {
    let o = c.output().must("run cargo-remedy");
    assert!(!o.status.success(), "{c:?} should fail:\n{}", out(&o));
    err(&o)
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("remedy-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).must("temp folder");
    d
}

#[test]
fn docs_widgets_and_markup() {
    let hits = passes(&mut remedy(&["docs", "virtual", "list", "--limit", "3"]));
    assert!(hits.contains(" — "), "{hits}");
    let json: Value = serde_json::from_str(&passes(&mut remedy(&["docs", "agents", "consent", "--json"])))
        .must("JSON hits");
    assert!(json.as_array().is_some_and(|a| !a.is_empty()));
    let read = passes(&mut remedy(&[
        "docs",
        "read",
        "docs/design/11-accessibility-and-automation.md",
        "--heading",
        "Automation by agents",
    ]));
    assert!(read.starts_with("## Automation by agents") && read.contains("Consent"), "{read}");
    assert!(passes(&mut remedy(&["docs", "list"])).contains("guide\tdocs/DESIGN.md"));
    assert!(fails(&mut remedy(&["docs", "read", "nope.md"])).contains("no document"));

    let ws: Value =
        serde_json::from_str(&passes(&mut remedy(&["widgets", "switch", "--feature", "widgets", "--json"])))
            .must("JSON");
    let switch = ws
        .as_array()
        .and_then(|a| a.iter().find(|w| w.get("name").and_then(Value::as_str) == Some("switch")))
        .must("switch");
    assert_eq!(switch.get("path").and_then(Value::as_str), Some("remedy_ui::widgets::switch"));
    assert!(
        passes(&mut remedy(&["widgets", "button"]))
            .contains("remedy_ui::widgets::button (feature `widgets`)")
    );

    let dir = temp("rui");
    std::fs::write(dir.join("good.rui"), "column { switch(label: \"Noise\", checked: $noise) }\n")
        .must("write");
    std::fs::write(dir.join("voice.rui"), "column {\n  slider(label: \"Gain\", value: $gain)\n}\n")
        .must("write");
    let good = dir.join("good.rui").display().to_string();
    assert!(passes(&mut remedy(&["rui", "check", &good])).contains("signal $noise: flag (Signal<bool>)"));
    let bad = fails(&mut remedy(&["rui", "check", &dir.join("voice.rui").display().to_string()]));
    assert!(bad.contains("voice.rui:2:3:") && bad.contains("range"), "{bad}");
    let rs = dir.join("good.rs").display().to_string();
    passes(&mut remedy(&["rui", "compile", &good, "--out", &rs]));
    assert!(std::fs::read_to_string(&rs).must("compiled").contains("pub struct Bindings"));
    std::fs::write(dir.join("fret.rui"), "row { fretboard(strings: 6) }\n").must("write");
    std::fs::write(
        dir.join("els.json"),
        r#"[{"name": "fretboard", "params": [{"name": "strings", "kind": "count"}]}]"#,
    )
    .must("write");
    let fret = dir.join("fret.rui").display().to_string();
    assert!(fails(&mut remedy(&["rui", "check", &fret])).contains("fretboard"));
    passes(&mut remedy(&["rui", "check", &fret, "--elements", &dir.join("els.json").display().to_string()]));
    assert!(
        passes(&mut remedy(&["rui", "elements"]))
            .contains("slider(label: label, value: number, range: range, step: count?)")
    );
    assert!(passes(&mut remedy(&["help"])).contains("DEVELOPER TOOLS"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn previews_audits_tests_and_budgets() {
    let ws = fixture_workspace(&checkout());
    let app = ws.join("fixture-app").display().to_string();
    let png = ws.join("out").join("cli-home.png");
    let _ = std::fs::remove_file(&png);
    let shown = passes(&mut remedy(&[
        "preview",
        "home",
        "--app",
        &app,
        "--size",
        "240x180",
        "--theme",
        "light",
        "--png",
        &png.display().to_string(),
    ]));
    assert!(
        shown.starts_with("240x180 px, theme light (light), locale en-US") && shown.contains("Save"),
        "{shown}"
    );
    let reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&png).must("the PNG")))
        .read_info()
        .must("a PNG");
    assert_eq!((reader.info().width, reader.info().height), (240, 180));
    let json: Value = serde_json::from_str(&passes(&mut remedy(&[
        "preview",
        "screens::about",
        "--app",
        &app,
        "--locale",
        "ar-XB",
        "--json",
    ])))
    .must("JSON");
    assert_eq!(json.get("rtl").and_then(Value::as_bool), Some(true));
    let listing = ws.join("cli-listing");
    std::fs::create_dir_all(listing.join("src")).must("listing app");
    std::fs::write(listing.join("Cargo.toml"), "[package]\nname = \"cli-listing\"\nversion = \"0.1.0\"\n")
        .must("manifest");
    std::fs::write(
        listing.join("src").join("lib.rs"),
        "mod p {\n    #[preview]\n    pub fn hidden(ui: &Ui) -> El { x(ui) }\n}\n",
    )
    .must("source");
    let listed = passes(&mut remedy(&["previews", "--app", &listing.display().to_string()]));
    assert!(listed.contains("p::hidden (private"), "{listed}");

    let audit = fails(&mut remedy(&["audit", "home", "--app", &app]));
    assert!(audit.contains("1 accessibility problem(s)"), "{audit}");

    let failing = remedy(&["test", "--app", &app]).output().must("test");
    assert!(!failing.status.success());
    assert!(
        out(&failing).contains("1 passed, 1 failed") && out(&failing).contains("this one fails on purpose"),
        "{}",
        out(&failing)
    );
    assert!(passes(&mut remedy(&["test", "adds", "--app", &app])).starts_with("ok: 1 passed, 0 failed"));
    assert!(
        passes(&mut remedy(&["budgets", "--list", "--remedy-root", &plain_path(&checkout())]))
            .contains("zoom_budget")
    );
}

/// Runs `cmd` while the app runs frames, as its loop would.
fn while_running(app: &mut TestApp, mut cmd: Command) -> Output {
    let (tx, rx) = channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });
    let start = Instant::now();
    loop {
        app.frames(1);
        if let Ok(o) = rx.try_recv() {
            let _ = worker.join();
            return o.must("the command ran");
        }
        assert!(start.elapsed() < Duration::from_secs(120), "the command finished in time");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_running_app_and_its_devtools_from_the_command_line() {
    // An app that opened its window to agents and devtools, on the in-memory host.
    let noise = Signal::new(false);
    type Opened = (String, String, Listener, RemoteServer);
    let opened: Rc<RefCell<Option<Opened>>> = Rc::default();
    let o2 = opened.clone();
    let mut app = TestApp::window(move |ui, window| {
        ui.root().child(w::column(ui).child(w::switch(ui, "Noise suppression", noise)));
        let agents = window.attach_agents(None).must("the window is open");
        let act = agents.grant("cli", Scope::Act, None);
        let read = agents.grant("viewer", Scope::Read, None);
        let listener = agents.listen().must("listen");
        let devtools = window.serve_devtools(RemoteOptions::default()).must("devtools");
        *o2.borrow_mut() = Some((act, read, listener, devtools));
    });
    let (act, read, listener, devtools) = opened.borrow_mut().take().must("built");
    let addr = listener.addr().to_string();
    let on = || noise.get_untracked().ok();

    let snap =
        while_running(&mut app, remedy(&["app", "snapshot", "--address", &addr, "--token", &act, "--json"]));
    assert!(snap.status.success(), "{}", err(&snap));
    let json: Value = serde_json::from_str(&out(&snap)).must("JSON snapshot");
    let tree = json.get("tree").and_then(Value::as_array).must("a tree");
    let id = tree
        .iter()
        .find(|n| n.get("name").and_then(Value::as_str) == Some("Noise suppression"))
        .and_then(|n| n.get("id"))
        .and_then(Value::as_u64)
        .must("the switch");

    let wrong = while_running(
        &mut app,
        remedy(&["app", "connect", "--address", &addr, "--token", "0123456789abcdef0123456789abcdef"]),
    );
    assert!(!wrong.status.success() && err(&wrong).contains("no access"), "{}", err(&wrong));
    let mut ro = remedy(&["app", "act", &id.to_string(), "click", "--address", &addr]);
    ro.env("REMEDY_APP_TOKEN", &read);
    let refused = while_running(&mut app, ro);
    assert!(!refused.status.success() && err(&refused).contains("may only read"), "{}", err(&refused));
    assert_eq!(on(), Some(false));
    let done = while_running(
        &mut app,
        remedy(&["app", "act", &format!("#{id}"), "click", "--address", &addr, "--token", &act]),
    );
    assert!(done.status.success() && out(&done).trim() == "done", "{}{}", out(&done), err(&done));
    assert_eq!(on(), Some(true), "the switch turned on");
    let undone = while_running(&mut app, remedy(&["app", "undo", "--address", &addr, "--token", &act]));
    assert!(undone.status.success(), "{}", err(&undone));
    app.frames(1);
    assert_eq!(on(), Some(false), "and back off");

    let (daddr, dtoken) = (devtools.addr().to_string(), devtools.token().to_owned());
    let tree =
        while_running(&mut app, remedy(&["devtools", "tree", "--address", &daddr, "--token", &dtoken]));
    assert!(
        tree.status.success() && out(&tree).contains("Noise suppression"),
        "{}{}",
        out(&tree),
        err(&tree)
    );
    let sem =
        while_running(&mut app, remedy(&["devtools", "semantics", "--address", &daddr, "--token", &dtoken]));
    assert!(out(&sem).contains("problem(s)"));
    let timings = while_running(
        &mut app,
        remedy(&["devtools", "timings", "--address", &daddr, "--token", &dtoken, "--json"]),
    );
    let t: Value = serde_json::from_str(&out(&timings)).must("JSON timings");
    assert_eq!(t.get("answer").and_then(Value::as_str), Some("frames"));
    let bad = while_running(&mut app, remedy(&["devtools", "tree", "--address", &daddr, "--token", "nope"]));
    assert!(err(&bad).contains("refused"));
    drop(listener);
    drop(devtools);
}
