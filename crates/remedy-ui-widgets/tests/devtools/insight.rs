//! Devtools: the signal graph and why elements update, frame timings and the
//! performance overlay, Perfetto traces, the screen-reader view, and remote devtools.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use remedy_ui_core::Size;
use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_text::FontCollection;
use remedy_ui_widgets::base as w;
use remedy_ui_widgets::devtools::*;

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

fn ui() -> Ui {
    let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts)
}

/// One frame of `ui`, inside the frame span the app runner opens around each frame.
fn frame(ui: &Ui) {
    let _frame = remedy_ui_core::phase!("frame", ui = ui.id()).entered();
    let _ = ui.frame(Size::new(480.0, 360.0), 1.0);
    let _ = ui.take_requests();
}

/// Runs the window's posted work until `done` says so.
fn pump_until(mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        let _ = remedy_ui_core::reactive::run_posted();
        assert!(start.elapsed() < Duration::from_secs(20), "the window answered in time");
        std::thread::yield_now();
    }
}

/// A chat row whose text follows a named signal, and a counter elsewhere reading the same
/// signal.
fn chat(ui: &Ui) -> (Signal<String>, El, El) {
    let text = Signal::new(String::from("hello")).named("message.text");
    let row = w::text(ui, "").text(move || text.get());
    let count = w::text(ui, "").variant("muted").text(move || format!("{} letters", text.get().len()));
    let _ = ui.root().child(w::column(ui).child(row.clone()).child(count.clone()));
    (text, row, count)
}

#[test]
fn an_elements_signals_and_their_dependents() {
    let ui = ui();
    let (text, row, count) = chat(&ui);
    frame(&ui);
    let bindings = element_signals(&ui, row.id());
    let b = bindings.first().must("the row's text binding");
    assert_eq!(bindings.len(), 1);
    assert_eq!(b.node.name.as_deref(), Some("text"), "bindings are named by what they bind");
    assert!(
        b.node.created_at.file().ends_with("insight.rs"),
        "the binding's site is the app's: {:?}",
        b.node.created_at
    );
    let read = b.reads.first().must("it reads the message");
    assert_eq!(read.node.key, text.key());
    assert_eq!(read.node.label(), "message.text");
    let users: Vec<_> = read.dependents.iter().filter_map(|d| d.element).collect();
    assert!(
        users.contains(&row.id()) && users.contains(&count.id()),
        "both elements depend on it: {users:?}"
    );
    let counter = read.dependents.iter().find(|d| d.element == Some(count.id())).must("the counter");
    assert!(counter.element_label.as_deref().is_some_and(|l| l.contains("letters")), "{counter:?}");
    // The element's info lists its bindings by key.
    assert_eq!(row.info().must("info").binding_keys, vec![b.node.key]);
    assert_eq!(ui.element_of_binding(b.node.key), Some(row.id()));
}

#[test]
fn the_log_says_why_each_element_updated() {
    let ui = ui();
    let (text, row, count) = chat(&ui);
    frame(&ui);
    let log = UpdateLog::attach(&ui, 50);
    text.set("hello again".into());
    let line = line!() - 1;
    frame(&ui);
    let mine = log.entries_for(row.id());
    let e = mine.first().must("the row updated");
    assert_eq!(e.record.verb(), "relaid out", "new text lays out again");
    let cause = e.record.cause.as_ref().must("a binding marked it");
    assert_eq!(cause.name.as_deref(), Some("text"));
    let why = cause.causes.first().must("the write");
    assert_eq!(why.signal, text.key());
    assert_eq!(why.written_at.map(|l| l.line()), Some(line), "the write's own line");
    let sentence = e.sentence();
    assert!(sentence.contains("because its text binding re-ran: `message.text` changed"), "{sentence}");
    assert!(sentence.contains("insight.rs"), "{sentence}");
    assert!(!log.entries_for(count.id()).is_empty(), "the counter updated for the same write");

    // A change made directly (not by a binding) is told apart.
    row.state(remedy_ui_core::style::States::HOVER, true);
    let last = log.entries().last().cloned().must("the hover mark");
    assert!(last.record.cause.is_none());
    assert!(last.sentence().contains("changed directly"), "{}", last.sentence());

    // What devtools draw into the window (the inspector's outline here) is not the app's.
    let insp = Inspector::attach(&ui, remedy_ui_widgets::themes::sheet);
    frame(&ui);
    let before = log.entries().len();
    insp.select(Some(row.id()));
    frame(&ui);
    let outline = walk(&ui)
        .into_iter()
        .find(|(_, e)| e.info().is_some_and(|i| i.widget == "inspector_highlight"))
        .map(|(_, e)| e.id())
        .must("the outline is drawn");
    assert!(
        log.entries().iter().skip(before).all(|e| e.record.element != outline),
        "{:?}",
        log.entries().iter().skip(before).collect::<Vec<_>>()
    );
    insp.detach();
    drop(insp);

    // Entries are kept up to what was asked, and the log can be cleared.
    log.set_keep(2);
    assert!(log.entries().len() <= 2);
    log.clear();
    assert!(log.entries().is_empty());
    drop(log);
    assert!(!remedy_ui_core::reactive::debug::is_tracing(), "dropping the log stops tracing");
}

#[test]
fn a_frame_reports_what_it_redid() {
    let ui = ui();
    let (text, row, _) = chat(&ui);
    frame(&ui);
    frame(&ui);
    assert_eq!(
        ui.last_frame_stats(),
        remedy_ui_engine::view::FrameStats::default(),
        "an idle frame does nothing"
    );
    text.set("changed".into());
    frame(&ui);
    let stats = ui.last_frame_stats();
    assert!(stats.repainted.contains(&row.id()));
    assert!(stats.laid_out >= 1);
    assert!(stats.layers && stats.semantics);
}

#[test]
fn frame_timings_come_from_the_phase_spans() {
    let rec = Recorder::install().must("no other subscriber in this test binary");
    let feed = rec.frames(16);
    let ui = ui();
    let (text, _, _) = chat(&ui);
    // Input before the frame counts in it.
    {
        let _events = remedy_ui_core::phase!("events", ui = ui.id()).entered();
        text.set("typed".into());
    }
    frame(&ui);
    let mine: Vec<FrameSample> = feed.take().into_iter().filter(|s| s.ui == Some(ui.id())).collect();
    let s = mine.last().must("a sample for this window");
    for p in ["events", "binds", "style", "layout", "paint", "layers", "semantics"] {
        assert!(s.phase(p) > 0, "{p} was timed: {s:?}");
    }
    assert_eq!(s.phase("raster"), 0, "nothing rastered in a view-only frame");
    assert!(s.frame_ns >= s.phase("layout"));
    assert!(s.total_ns() >= s.phase("events") + s.phase("paint"));
    assert_eq!(s.phase("no such phase"), 0);
}

#[test]
fn the_overlay_shows_frames_without_keeping_the_window_busy() {
    let rec = Recorder::install().must("installed");
    let target = ui();
    let (text, _, _) = chat(&target);
    frame(&target);
    let overlay = PerfOverlay::show(&target, &rec, Some(Duration::from_micros(16_667)));
    let el = overlay.element().must("the overlay is in the window");
    // The overlay is over the content, out of the accessibility tree and click-through.
    assert!(el.info().must("info").semantics.is_some_and(|s| s.is_hidden()));
    frame(&target);
    text.set("something new".into());
    frame(&target);
    pump_until(|| !overlay.samples().is_empty());
    // It redraws for the app's frames…
    frame(&target);
    assert!(!target.last_frame_stats().repainted.is_empty(), "the overlay redrew");
    // …and that redraw's own frame is no reason to draw again: the window goes idle.
    let _ = remedy_ui_core::reactive::run_posted();
    let _ = target.frame(Size::new(480.0, 360.0), 1.0);
    let idle = target.last_frame_stats();
    assert!(idle.repainted.is_empty(), "nothing left to draw: {idle:?}");
    assert!(overlay.samples().iter().all(|s| s.ui == Some(target.id())));
    let texts: Vec<String> =
        walk(&target).iter().filter_map(|(_, e)| e.info()).filter_map(|i| i.text).collect();
    assert!(texts.iter().any(|t| t.starts_with("Frame ") && t.contains(" ms")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("frames over the 16.7 ms budget")), "{texts:?}");
    drop(overlay);
    frame(&target);
    assert!(el.info().is_none(), "dropping the overlay removes it");
}

#[test]
fn the_overlay_follows_the_display_and_shows_work_damage_and_caches() {
    let rec = Recorder::install().must("installed");
    let target = ui();
    let (text, label, _) = chat(&target);
    frame(&target);
    // A 120 Hz display: frames have 8.3 ms before they miss a refresh.
    target.set_refresh_interval(Some(Duration::from_nanos(8_333_333)));
    let overlay = PerfOverlay::show(&target, &rec, None);
    frame(&target);
    assert_eq!(overlay.budget(), Duration::from_nanos(8_333_333));
    // Moved to a 60 Hz display, it follows.
    target.set_refresh_interval(Some(Duration::from_nanos(16_666_667)));
    frame(&target);
    assert_eq!(overlay.budget(), Duration::from_nanos(16_666_667));
    // A budget the developer sets holds until they let go of it.
    overlay.set_budget(Some(Duration::from_millis(5)));
    target.set_refresh_interval(Some(Duration::from_nanos(6_944_444)));
    frame(&target);
    assert_eq!(overlay.budget(), Duration::from_millis(5));
    overlay.set_budget(None);
    assert_eq!(overlay.budget(), Duration::from_nanos(6_944_444));

    // What the app's frame did, without the overlay's own drawing; where it repainted.
    let _held = remedy_ui_core::trace::gauges::Gauge::new("test tiles");
    _held.set(3 * 1024 * 1024);
    overlay.show_damage(true);
    frame(&target);
    text.set("a longer message than before".into());
    frame(&target);
    pump_until(|| !overlay.work().repainted.is_empty());
    let work = overlay.work();
    assert!(work.repainted.contains(&label.id()), "the label repainted: {work:?}");
    let ov = overlay.element().must("overlay");
    for id in &work.repainted {
        let mut cur = Some(target.element(*id));
        while let Some(c) = cur {
            assert_ne!(c.id(), ov.id(), "the overlay's own drawing is not the app's work");
            cur = c.parent();
        }
    }
    let damage = overlay.damage();
    let b = label.bounds().must("laid out");
    assert!(damage.contains(&b), "the label's area is outlined: {damage:?}");
    frame(&target);
    let texts: Vec<String> =
        walk(&target).iter().filter_map(|(_, e)| e.info()).filter_map(|i| i.text).collect();
    assert!(texts.iter().any(|t| t.contains("frames missed a refresh (144 Hz display)")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Restyled ") && t.contains("repainted")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Caches: ") && t.contains("test tiles 3.0 MB")), "{texts:?}");
    overlay.show_damage(false);
}

#[test]
fn a_trace_exports_every_thread_and_the_gpu_for_perfetto() {
    let rec = Recorder::install().must("installed");
    rec.start_capture();
    let ui = ui();
    let (text, _, _) = chat(&ui);
    text.set("traced".into());
    frame(&ui);
    let worker = std::thread::Builder::new()
        .name("remedy-test-worker".into())
        .spawn(|| {
            let _span = remedy_ui_core::phase!("page raster").entered();
            std::thread::sleep(Duration::from_millis(1));
        })
        .must("a worker");
    let _ = worker.join();
    let now = remedy_ui_core::trace::now_ns();
    remedy_ui_core::trace::gpu_slice("frame", now, now + 2_000_000);
    let trace = rec.stop_capture();
    assert!(!rec.is_capturing());

    let json: serde_json::Value = serde_json::from_str(&trace.to_chrome_json()).must("the export is JSON");
    let events = json.get("traceEvents").and_then(|e| e.as_array()).must("a trace event list");
    let named = |name: &str| {
        events
            .iter()
            .filter(move |e| e.get("name").and_then(|n| n.as_str()) == Some(name))
            .collect::<Vec<_>>()
    };
    let threads: Vec<String> = named("thread_name")
        .iter()
        .filter_map(|e| e.pointer("/args/name").and_then(|n| n.as_str()).map(str::to_owned))
        .collect();
    assert!(threads.iter().any(|t| t == "remedy-test-worker"), "the worker has a track: {threads:?}");
    assert!(threads.iter().any(|t| t == "GPU"), "the GPU has a track: {threads:?}");
    assert!(
        threads.iter().any(|t| t.contains("a_trace_exports")),
        "the test's own thread (the UI thread here) has a track: {threads:?}"
    );
    for phase in ["frame", "style", "layout", "paint", "semantics", "binds", "page raster"] {
        let e = named(phase);
        let first = e.first().must(phase);
        assert_eq!(first.get("ph").and_then(|p| p.as_str()), Some("X"), "{phase} is a complete event");
        assert!(first.get("dur").and_then(serde_json::Value::as_f64).is_some_and(|d| d >= 0.0));
        assert!(first.get("ts").and_then(serde_json::Value::as_f64).is_some());
    }
    let gpu = named("frame")
        .into_iter()
        .find(|e| e.get("tid").and_then(serde_json::Value::as_u64) == Some(GPU_TRACK))
        .must("the GPU slice is on the GPU track");
    assert_eq!(gpu.get("dur").and_then(serde_json::Value::as_f64), Some(2000.0), "microseconds");
    let mine = ui.id().to_string();
    let frame_ev = named("frame")
        .into_iter()
        .find(|e| e.pointer("/args/ui").and_then(|v| v.as_str()) == Some(mine.as_str()))
        .must("the frame names its window");
    assert_ne!(frame_ev.get("tid").and_then(serde_json::Value::as_u64), Some(GPU_TRACK));
    // The file is written as is.
    let dir = std::env::temp_dir().join(format!("remedy-trace-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    trace.write(&path).must("written");
    let back: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).must("read back")).must("still JSON");
    assert_eq!(back, json);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_reader_view_is_what_a_screen_reader_says_with_problems_flagged() {
    let ui = ui();
    let fine = w::button(&ui, "Send", || {});
    let hidden_group = ui.el("group").semantics(Semantics::new(Role::Group).hidden(true));
    let lost = w::button(&ui, "Delete", || {});
    let merged = ui
        .el("row")
        .row()
        .semantics(Semantics::new(Role::ListItem).label("Message from Ada").merge_descendants());
    let folded = w::button(&ui, "Reply", || {});
    let mute = ui
        .el("custom")
        .width(40.0)
        .height(40.0)
        .on_click(|| {})
        .semantics(Semantics::new(Role::Button).label("Mute"));
    let _ = ui.root().child(
        w::column(&ui)
            .child(fine.clone())
            .child(hidden_group.child(lost.clone()))
            .child(merged.clone().child(folded.clone()))
            .child(mute.clone()),
    );
    frame(&ui);
    let lines = reader_lines(&ui);
    let send = lines.iter().find(|l| l.element == Some(fine.id())).must("the send button is read");
    assert_eq!(send.node.announcement(), "button, Send");
    assert!(send.problems.is_empty());
    assert!(lines.first().is_some_and(|l| l.node.role == Role::Window && l.node.depth == 0));
    assert!(lines.iter().all(|l| l.element != Some(lost.id())), "hidden content is not read");
    assert!(lines.iter().all(|l| l.element != Some(folded.id())), "merged content is read in its row");
    let row = lines.iter().find(|l| l.element == Some(merged.id())).must("the row");
    assert!(row.node.announcement().contains("Message from Ada, Reply"), "{}", row.node.announcement());
    let muted = lines.iter().find(|l| l.element == Some(mute.id())).must("the mute control is read");
    assert!(
        muted.problems.iter().any(|p| p.kind == ProblemKind::Unreachable { why: Unreachable::NoAction }),
        "{:?}",
        muted.problems
    );
    let out = unreachable_problems(&ui);
    let why = |el: &El| out.iter().find(|p| p.element == el.id()).map(|p| p.kind.clone());
    assert_eq!(why(&lost), Some(ProblemKind::Unreachable { why: Unreachable::Hidden }));
    assert_eq!(why(&folded), Some(ProblemKind::Unreachable { why: Unreachable::Merged }));
    assert!(out.iter().all(|p| p.element != fine.id()));
    let msg = out.first().map(Problem::message).unwrap_or_default();
    assert!(msg.starts_with("Control "), "{msg}");
}

/// Sends `line` on a fresh connection and reads the first answer line.
fn first_answer(addr: std::net::SocketAddr, line: &str) -> (String, BufReader<TcpStream>) {
    let mut s = TcpStream::connect(addr).must("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).must("timeout");
    writeln!(s, "{line}").must("write");
    let mut r = BufReader::new(s);
    let mut answer = String::new();
    let _ = r.read_line(&mut answer);
    (answer, r)
}

#[test]
fn remote_devtools_refuse_strangers_and_serve_the_window() {
    let ui = ui();
    let (text, row, _) = chat(&ui);
    let _ = ui.root().child(w::button(&ui, "Send", || {}));
    frame(&ui);
    let bad = RemoteServer::listen(&ui, {
        let mut v_ = RemoteOptions::default();
        v_.token = Some("two\nlines".into());
        v_
    });
    assert_eq!(bad.map(|_| ()).map_err(|e| e.kind()), Err(std::io::ErrorKind::InvalidInput));
    let server = RemoteServer::listen(&ui, RemoteOptions::default()).must("serving");
    let addr = server.addr();
    assert!(addr.ip().is_loopback(), "only this machine can connect");
    let token = server.token().to_owned();
    assert_eq!(token.len(), 32);
    // A write after serving started shows in the update log it serves.
    text.set("remote".into());
    frame(&ui);
    let row_id = row.node_id();

    let (tx, rx) = channel();
    let client = std::thread::spawn(move || {
        // A wrong token is refused before reaching the window, and the connection closes.
        let (answer, mut r) = first_answer(addr, "0123456789abcdef0123456789abcdef");
        assert_eq!(answer.trim(), r#"{"answer":"refused"}"#);
        let mut more = String::new();
        assert_eq!(r.read_line(&mut more).ok(), Some(0), "closed");
        let (answer, _) = first_answer(addr, "not a token at all, and far too long to be one");
        assert!(answer.trim().is_empty() || answer.contains("refused"), "{answer}");
        let refused = RemoteClient::connect(addr, "nope", Duration::from_secs(20)).map(|_| ());
        assert_eq!(refused.map_err(|e| e.kind()), Err(std::io::ErrorKind::PermissionDenied));

        let mut c =
            RemoteClient::connect(addr, &token, Duration::from_secs(20)).must("the token is accepted");
        let mut ask = |r: RemoteRequest| c.request(&r).must("an answer");
        let tree = ask(RemoteRequest::Tree);
        let element = ask(RemoteRequest::Element { id: row_id });
        let semantics = ask(RemoteRequest::Semantics);
        let signals = ask(RemoteRequest::Signals { id: Some(row_id) });
        let graph = ask(RemoteRequest::Signals { id: None });
        let updates = ask(RemoteRequest::Updates);
        let frames = ask(RemoteRequest::Frames);
        let missing = ask(RemoteRequest::Element { id: u64::MAX });
        let _ = tx.send((tree, element, semantics, signals, graph, updates, frames, missing));
    });
    let mut got = None;
    pump_until(|| {
        got = got.take().or_else(|| rx.try_recv().ok());
        got.is_some()
    });
    let _ = client.join();
    let (tree, element, semantics, signals, graph, updates, frames, missing) = got.must("answers");

    let RemoteAnswer::Tree { elements } = tree else { unreachable!("a tree: {tree:?}") };
    let me = elements.iter().find(|e| e.id == row_id).must("the row is in the tree");
    assert!(me.label.contains("remote"), "{me:?}");
    assert!(me.parent.is_some() && me.depth > 0);
    assert!(elements.iter().any(|e| e.label.contains("Send")));

    let RemoteAnswer::Element { id, details } = element else { unreachable!("details: {element:?}") };
    assert_eq!(id, row_id);
    assert!(details.iter().any(|(k, v)| k == "Built at" && v.contains("insight.rs")), "{details:?}");

    let RemoteAnswer::Semantics { nodes, .. } = semantics else { unreachable!("semantics: {semantics:?}") };
    assert!(nodes.iter().any(|n| n.announcement == "button, Send"), "{nodes:?}");

    let RemoteAnswer::Bindings { bindings } = signals else { unreachable!("bindings: {signals:?}") };
    let read = bindings.first().and_then(|b| b.reads.first()).must("the row reads the message");
    assert_eq!(read.node.label, "message.text");
    assert!(read.dependents.iter().any(|d| d.element == Some(row_id)));

    let RemoteAnswer::Graph { nodes } = graph else { unreachable!("graph: {graph:?}") };
    assert!(nodes.iter().any(|n| n.label == "message.text" && n.kind == "signal"));

    let RemoteAnswer::Updates { entries } = updates else { unreachable!("updates: {updates:?}") };
    assert!(
        entries.iter().any(|e| e.element == row_id && e.sentence.contains("`message.text` changed")),
        "{entries:?}"
    );

    let RemoteAnswer::Frames { frames, recording } = frames else { unreachable!("frames: {frames:?}") };
    assert!(recording, "nothing else took the tracing subscriber");
    let last = frames.last().must("the frame after serving started");
    assert!(last.phases.iter().any(|(p, ns)| p == "layout" && *ns > 0), "{last:?}");

    assert!(matches!(missing, RemoteAnswer::Error { .. }), "{missing:?}");
    drop(server);
    // Stopped: a new connection is no longer answered.
    let after = RemoteClient::connect(addr, server_token_after_drop(), Duration::from_secs(2));
    assert!(after.is_err());
}

/// Any token: after the server stops, nothing greets a connection.
fn server_token_after_drop() -> &'static str {
    "0123456789abcdef0123456789abcdef"
}

#[test]
fn the_inspector_panel_has_every_page() {
    let target = ui();
    let (text, row, _) = chat(&target);
    // A boundary whose content fails the first time it builds.
    let attempts = std::rc::Rc::new(std::cell::Cell::new(0));
    let (a2, t2) = (attempts.clone(), target.clone());
    let boundary = remedy_ui_engine::view::catch_errors(
        &target,
        {
            let t3 = target.clone();
            move |e, _| w::text(&t3, format!("Fallback: {}", e.message))
        },
        move || {
            a2.set(a2.get() + 1);
            if a2.get() == 1 {
                std::panic::resume_unwind(Box::new("profile missing"));
            }
            w::text(&t2, "Profile")
        },
    );
    let _ = target.root().child(boundary);
    frame(&target);
    let insp = Inspector::attach(&target, remedy_ui_widgets::themes::sheet);
    let panel = ui();
    let _ = panel.root().child(inspector_panel(&panel, &insp));
    frame(&panel);
    insp.select(Some(row.id()));
    text.set("edited".into());
    frame(&target);
    let shown = |ui: &Ui| -> String {
        walk(ui)
            .iter()
            .filter(|(_, e)| {
                let mut cur = Some(e.clone());
                while let Some(c) = cur {
                    if c.info().is_some_and(|i| i.hidden) {
                        return false;
                    }
                    cur = c.parent();
                }
                true
            })
            .filter_map(|(_, e)| e.info())
            .filter_map(|i| i.text)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tab = |i: usize| {
        let seg = walk(&panel)
            .into_iter()
            .filter_map(|(_, e)| e.info().map(|i| (e, i)))
            .filter(|(_, i)| i.widget == "segment")
            .nth(i)
            .map(|(e, _)| e)
            .must("a tab");
        assert!(panel.accessibility_action(seg.node_id(), remedy_ui_core::a11y::Action::Click));
        frame(&panel);
    };
    assert!(shown(&panel).contains("Pick an element"));
    tab(1);
    let page = shown(&panel);
    assert!(page.contains("message.text"), "{page}");
    assert!(page.contains("because its text binding re-ran"), "{page}");
    tab(2);
    let page = shown(&panel);
    assert!(page.contains("Screen reader view") && page.contains("window"), "{page}");
    tab(3);
    let page = shown(&panel);
    // Captures are process-wide (one tracing subscriber), and the trace test in this binary
    // may be recording at this moment: the button offers whichever applies.
    assert!(
        page.contains("Perfetto trace")
            && (page.contains("Start recording") || page.contains("Stop and save")),
        "{page}"
    );
    assert!(insp.show_overlay(true));
    assert!(insp.overlay().is_some());
    assert!(insp.show_overlay(false));
    assert!(insp.overlay().is_none());
    tab(4);
    let page = shown(&panel);
    assert!(page.contains("Not connected."), "{page}");
    // Failures: what broke, where in the tree and the source, with a retry and a clear.
    tab(5);
    let page = shown(&panel);
    assert!(page.contains("⚠ profile missing"), "{page}");
    assert!(page.contains("Failed while building at") && page.contains("error_boundary"), "{page}");
    assert!(page.contains("insight.rs:"), "the source location: {page}");
    let button = |label: &str| {
        walk(&panel)
            .into_iter()
            .map(|(_, e)| e)
            .find(|e| {
                e.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)).as_deref()
                    == Some(label)
            })
            .must("the button")
    };
    let retry = button("Retry");
    assert!(panel.accessibility_action(retry.node_id(), remedy_ui_core::a11y::Action::Click));
    frame(&target);
    frame(&panel);
    assert_eq!(attempts.get(), 2, "the boundary built its content again");
    assert!(shown(&target).contains("Profile"));
    assert!(shown(&panel).contains("Its boundary has built its content again."));
    let clear = button("Clear failures");
    assert!(panel.accessibility_action(clear.node_id(), remedy_ui_core::a11y::Action::Click));
    frame(&panel);
    assert!(shown(&panel).contains("Nothing has failed in this window."));
    assert_eq!(TABS.len(), 6);
}
