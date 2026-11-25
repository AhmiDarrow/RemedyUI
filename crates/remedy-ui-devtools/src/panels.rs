//! The inspector panel's pages beyond the element inspector: signals, accessibility,
//! performance, remote devtools and caught failures.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::{Color, Insets};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_core::style::StylePatch;
use remedy_ui_engine::view::{El, ElementId, Ui};
use remedy_ui_widgets_core as w;

use crate::inspect::{Inspector, line};
use crate::remote::{
    RemoteAnswer, RemoteBinding, RemoteClient, RemoteElement, RemoteFrame, RemoteProblem, RemoteRequest,
    RemoteUpdate,
};

/// The panel's pages, in order.
pub const TABS: [&str; 6] = ["Inspect", "Signals", "Accessibility", "Performance", "Remote", "Failures"];

/// How long a remote request may take before the panel reports the app as unreachable,
/// unless the app says otherwise ([`set_remote_timeout`]).
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);

static TIMEOUT: std::sync::RwLock<Duration> = std::sync::RwLock::new(REMOTE_TIMEOUT);

/// Sets how long the Remote page waits for an app it connects to, for each request, before
/// it reports the app as unreachable (default [`REMOTE_TIMEOUT`], 10 s): raise it for an app
/// on a slow link or under a debugger. Applies to the next connection.
pub fn set_remote_timeout(timeout: Duration) {
    *TIMEOUT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = timeout;
}

/// How long the Remote page waits now ([`set_remote_timeout`]).
pub fn remote_timeout() -> Duration {
    *TIMEOUT.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The frame budget for a display rate in hertz.
pub(crate) fn budget_of(hz: f64) -> Duration {
    Duration::from_secs_f64(1.0 / hz.max(1.0))
}

fn warn_color(ui: &Ui) -> Color {
    let t = ui.theme();
    remedy_ui_core::style::ensure_contrast(t.colors.danger, t.colors.bg, 4.5)
}

fn note(ui: &Ui, s: impl Into<String>) -> El {
    w::text(ui, s.into()).variant("muted").data_text()
}

fn warning(ui: &Ui, s: impl Into<String>) -> El {
    let c = warn_color(ui);
    w::text(ui, s.into()).data_text().style(StylePatch::default().foreground(c))
}

/// The pages other than the inspector's own, shown by the selected tab and rebuilt when it
/// changes.
pub(crate) fn pages(ui: &Ui, insp: &Inspector) -> El {
    let holder = w::column(ui).fill_width();
    let (h2, u2, i2) = (holder.clone(), ui.clone(), insp.clone());
    let tab = insp.0.tab;
    // New log entries arrive inside the target's marks; the page refreshes once per turn of
    // the loop, however many arrive.
    let scheduled = std::rc::Rc::new(std::cell::Cell::new(false));
    let (u3, rev, sch) = (ui.downgrade(), insp.0.log_revision, scheduled);
    insp.0.log.on_entry(move || {
        if sch.replace(true) {
            return;
        }
        let Some(ui) = u3.upgrade() else { return };
        let sch = sch.clone();
        ui.after(Duration::ZERO, move || {
            sch.set(false);
            let _ = rev.try_update(|r| *r += 1);
        });
    });
    holder.bind(move || {
        let t = tab.get();
        untrack(|| {
            h2.clear();
            let page = match t {
                1 => Some(signals_page(&u2, &i2)),
                2 => Some(accessibility_page(&u2, &i2)),
                3 => Some(performance_page(&u2, &i2)),
                4 => Some(remote_page(&u2, &i2)),
                5 => Some(failures_page(&u2, &i2)),
                _ => None,
            };
            if let Some(p) = page {
                let _ = h2.clone().child(p);
            }
        });
    })
}

fn describe_el(target: &Ui, id: ElementId) -> String {
    target.element(id).info().map_or_else(|| "a removed element".to_owned(), |i| crate::describe(&i))
}

fn signals_page(ui: &Ui, insp: &Inspector) -> El {
    let unit = ui.theme().spacing.unit;
    let target = insp.0.target.clone();
    let (sel, rev, log_rev) = (insp.0.selected, insp.0.revision, insp.0.log_revision);

    // The selected element's bindings and what depends on what they read.
    let graph = w::column(ui).gap(unit * 2.0).fill_width();
    let (g2, u2, t2, i2) = (graph.clone(), ui.clone(), target.clone(), insp.clone());
    let graph = graph.bind(move || {
        let _ = rev.get();
        let _ = log_rev.get();
        let picked = sel.get();
        untrack(|| {
            g2.clear();
            let mut col = g2.clone();
            let Some(id) = picked else {
                let _ = col.child(note(
                    &u2,
                    "Pick an element to see the signals it reads and what else depends on them.",
                ));
                return;
            };
            let bindings = crate::signals::element_signals(&t2, id);
            if bindings.is_empty() {
                let _ = col.child(note(
                    &u2,
                    "This element has no bindings or painter: nothing reactive updates it.",
                ));
                return;
            }
            for b in bindings {
                let at = format!("{}:{}", b.node.created_at.file(), b.node.created_at.line());
                col = col.child(
                    w::text(&u2, format!("{} {} ({at})", b.node.label(), b.node.kind.as_str())).data_text(),
                );
                if b.reads.is_empty() {
                    col = col.child(note(&u2, "    reads nothing reactive"));
                }
                for r in b.reads {
                    col = col.child(line(&u2, "  reads", r.node.label()));
                    let deps: Vec<String> = r
                        .dependents
                        .iter()
                        .map(|d| match &d.element_label {
                            Some(e) => format!("{} of {e}", d.node.label()),
                            None => d.node.label(),
                        })
                        .collect();
                    let shown = if deps.is_empty() { "nothing else".to_owned() } else { deps.join("\n") };
                    let mut row = line(&u2, "  used by", shown);
                    // Each dependent element can be picked from here.
                    if let Some(first) = r.dependents.iter().find_map(|d| d.element.filter(|e| *e != id)) {
                        let i3 = i2.clone();
                        row = row.child(w::link(&u2, "Go to", move || i3.select(Some(first))));
                    }
                    col = col.child(row);
                }
            }
        });
    });

    // Why the selected element updated, then the whole window.
    let log = insp.0.log.clone();
    let mine = w::column(ui).gap(unit).fill_width();
    let all = w::column(ui).gap(unit).fill_width();
    let (m2, a2, u3, t3, l2) = (mine.clone(), all.clone(), ui.clone(), target.clone(), log.clone());
    let mine = mine.bind(move || {
        let _ = log_rev.get();
        let picked = sel.get();
        untrack(|| {
            m2.clear();
            a2.clear();
            let entries = l2.entries();
            let mut col = m2.clone();
            match picked {
                Some(id) => {
                    let own: Vec<_> = entries.iter().filter(|e| e.record.element == id).collect();
                    if own.is_empty() {
                        col = col.child(note(
                            &u3,
                            format!("{} has not updated while recording.", describe_el(&t3, id)),
                        ));
                    }
                    for e in own.iter().rev() {
                        col = col.child(w::text(&u3, e.sentence()).data_text());
                    }
                }
                None => col = col.child(note(&u3, "Pick an element to see why it updated.")),
            }
            let _ = col;
            let mut col = a2.clone();
            if entries.is_empty() {
                col = col.child(note(&u3, "Nothing in the window has updated since recording started."));
            }
            for e in entries.iter().rev() {
                col = col.child(w::text(&u3, e.sentence()).data_text());
            }
            let _ = col;
        });
    });
    let keep = insp.0.log_keep;
    let l3 = log.clone();
    let keeper = w::column(ui).bind(move || {
        let k = keep.get();
        l3.set_keep(k.max(1.0) as usize);
    });
    let l4 = log.clone();
    let controls = w::wrap(ui)
        .gap(unit * 2.0)
        .align(remedy_ui_engine::view::Align::Center)
        .child(w::stepper(ui, "Entries kept", keep, 1.0, f64::MAX, 100.0))
        .child(
            w::button(ui, "Clear", move || {
                l4.clear();
                let _ = log_rev.try_update(|r| *r += 1);
            })
            .variant("quiet"),
        );
    w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .child(w::section(ui, "Signals of the selected element").child(graph))
        .child(w::section(ui, "Why it updated").child(mine))
        .child(w::section(ui, "Recent updates in the window").child(controls).child(keeper).child(all))
}

fn accessibility_page(ui: &Ui, insp: &Inspector) -> El {
    let unit = ui.theme().spacing.unit;
    let target = insp.0.target.clone();
    let rev = insp.0.revision;
    let tree = w::column(ui).gap(unit / 2.0).fill_width();
    let lost = w::column(ui).gap(unit).fill_width();
    let (t2, l2, u2, tg, i2) = (tree.clone(), lost.clone(), ui.clone(), target.clone(), insp.clone());
    let tree = tree.bind(move || {
        let _ = rev.get();
        untrack(|| {
            t2.clear();
            l2.clear();
            let mut col = t2.clone();
            let lines = crate::inspect::reader_lines(&tg);
            if lines.is_empty() {
                col = col.child(note(&u2, "The window has not drawn a frame yet."));
            }
            for l in lines {
                let pad = Insets { top: 0.0, bottom: 0.0, start: l.node.depth as f32 * unit * 3.0, end: 0.0 };
                let text = format!("{}{}", if l.node.focused { "▶ " } else { "" }, l.node.announcement());
                let mut row = w::column(&u2).padding(pad).fill_width();
                row = match l.element {
                    Some(e) => {
                        let i3 = i2.clone();
                        row.child(w::link(&u2, text, move || i3.select(Some(e))))
                    }
                    None => row.child(w::text(&u2, text).data_text()),
                };
                for p in &l.problems {
                    row = row.child(warning(&u2, format!("⚠ {}", p.message())));
                }
                col = col.child(row);
            }
            let _ = col;
            let mut col = l2.clone();
            let unreachable = crate::inspect::unreachable_problems(&tg);
            if unreachable.is_empty() {
                col = col.child(note(&u2, "Every control is in the screen reader's reach."));
            }
            for p in unreachable {
                let i3 = i2.clone();
                let id = p.element;
                col = col.child(w::link(
                    &u2,
                    format!("⚠ {}: {}", describe_el(&tg, id), p.message()),
                    move || i3.select(Some(id)),
                ));
            }
            let _ = col;
        });
    });
    w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .child(note(
            ui,
            "What a screen reader announces for each node, in reading order, with problems flagged.",
        ))
        .child(w::section(ui, "Screen reader view").child(tree))
        .child(w::section(ui, "Out of a screen reader's reach").child(lost))
}

/// What the window's error boundaries caught: each failure's message, what was running,
/// the failing element's path and where it was made in the source, newest first, with a
/// retry for boundaries still showing their fallback, and a way to clear the list.
fn failures_page(ui: &Ui, insp: &Inspector) -> El {
    let unit = ui.theme().spacing.unit;
    let target = insp.0.target.clone();
    let list = w::column(ui).gap(unit * 2.0).fill_width();
    let (l2, u2, tg) = (list.clone(), ui.clone(), target.clone());
    let list = list.bind(move || {
        let failures = tg.failures();
        // Whether each boundary still shows its fallback (read here, so a retry refreshes).
        let failed: Vec<bool> =
            failures.iter().map(|f| f.boundary.is_some_and(|b| tg.is_failed(b))).collect();
        untrack(|| {
            l2.clear();
            let mut col = l2.clone();
            if failures.is_empty() {
                col = col.child(note(&u2, "Nothing has failed in this window."));
            }
            for (f, still) in failures.iter().zip(failed).rev() {
                let phase = f.phase.describe();
                let at = f
                    .location
                    .map_or_else(|| "unknown source".to_owned(), |l| format!("{}:{}", l.file(), l.line()));
                let mut card = w::column(&u2)
                    .gap(unit)
                    .fill_width()
                    .child(warning(&u2, format!("⚠ {}", f.message)))
                    .child(note(&u2, format!("Failed {phase} at {}", f.element_path)))
                    .child(note(&u2, format!("Built at {at}")));
                card = match f.boundary {
                    Some(b) if still => {
                        let tg2 = tg.clone();
                        card.child(w::button(&u2, "Retry", move || {
                            let _ = tg2.retry_failure(b);
                        }))
                    }
                    Some(_) => card.child(note(&u2, "Its boundary has built its content again.")),
                    None => {
                        card.child(note(&u2, "No error boundary was above it; it was logged and contained."))
                    }
                };
                col = col.child(card);
            }
            let _ = col;
        });
    });
    let tg3 = target.clone();
    w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .child(note(ui, "Failures this window's error boundaries caught, newest first."))
        .child(w::button(ui, "Clear failures", move || tg3.clear_failures()))
        .child(w::section(ui, "Failures").child(list))
}

fn performance_page(ui: &Ui, insp: &Inspector) -> El {
    let unit = ui.theme().spacing.unit;
    let rec = match &insp.0.recorder {
        Ok(r) => r.clone(),
        Err(e) => return warning(ui, e.to_string()),
    };
    let on = insp.0.overlay_on;
    let i2 = insp.clone();
    let overlay = w::switch(ui, "Show the performance overlay over the window", on).bind(move || {
        let want = on.get();
        untrack(|| {
            let _ = i2.show_overlay(want);
        });
    });
    let hz = insp.0.budget_hz;
    let follow = insp.0.budget_follow;
    let i3 = insp.clone();
    let follow_switch = w::switch(ui, "Measure frames against the display's refresh rate", follow);
    let rate = w::stepper(ui, "Budget rate (Hz) when not following the display", hz, 1.0, f64::MAX, 1.0);
    let r2 = rate.clone();
    let budget =
        w::column(ui).gap(ui.theme().spacing.unit).child(follow_switch).child(rate).bind(move || {
            let v = hz.get();
            let f = follow.get();
            let _ = r2.clone().disabled(f);
            untrack(|| {
                if let Some(o) = i3.overlay() {
                    o.set_budget((!f).then(|| budget_of(v)));
                }
            });
        });
    let i4 = insp.clone();
    let damage_on = Signal::new(false);
    let damage = w::switch(ui, "Outline what each frame repaints", damage_on).bind(move || {
        let on = damage_on.get();
        untrack(|| {
            if let Some(o) = i4.overlay() {
                o.show_damage(on);
            }
        });
    });
    let default_path = std::env::temp_dir().join(format!(
        "remedy-trace-{}.json",
        remedy_ui_core::time::SystemTime::now()
            .duration_since(remedy_ui_core::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    ));
    let path = w::text_input(ui, "Save the trace to");
    path.set_value(&default_path.to_string_lossy());
    let status = insp.0.trace_status;
    let capturing = Signal::new(rec.is_capturing());
    let (r2, p2) = (rec.clone(), path.clone());
    let record = w::button(ui, "Start recording", move || {
        if r2.is_capturing() {
            let trace = r2.stop_capture();
            let target = std::path::PathBuf::from(p2.value());
            status.set(match trace.write(&target) {
                Ok(()) => format!(
                    "Saved {} slices on {} tracks to {}. Open it in ui.perfetto.dev.",
                    trace.slices.len(),
                    trace.tracks.len(),
                    target.display()
                ),
                Err(e) => format!("Could not save to {}: {e}", target.display()),
            });
            capturing.set(false);
        } else {
            r2.start_capture();
            status.set("Recording every thread and the GPU…".to_owned());
            capturing.set(true);
        }
    })
    .variant("primary");
    let label = record.child_elements().into_iter().next();
    let record = record.bind(move || {
        let on = capturing.get();
        if let Some(l) = &label {
            let _ = l.clone().text(remedy_ui_engine::view::RichText::plain(if on {
                "Stop and save"
            } else {
                "Start recording"
            }));
        }
    });
    let shown = note(ui, "").text(move || status.get());
    w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .child(
            w::section(ui, "Overlay")
                .child(note(ui, "Each frame's time by phase, drawn over the inspected window."))
                .child(overlay)
                .child(budget)
                .child(damage),
        )
        .child(
            w::section(ui, "Perfetto trace")
                .child(note(
                    ui,
                    "Records the UI thread, the compositor, workers and the GPU on one timeline (Chrome JSON trace format).",
                ))
                .child(path.el())
                .child(record)
                .child(shown),
        )
}

/// The remote page's state, kept while the inspector lives so switching pages keeps the
/// connection.
#[derive(Clone)]
pub(crate) struct RemoteState {
    client: Arc<Mutex<Option<RemoteClient>>>,
    status: Signal<String>,
    connected: Signal<bool>,
    tree: Signal<Vec<RemoteElement>>,
    selected: Signal<Option<u64>>,
    details: Signal<Vec<(String, String)>>,
    bindings: Signal<Vec<RemoteBinding>>,
    problems: Signal<Vec<RemoteProblem>>,
    updates: Signal<Vec<RemoteUpdate>>,
    frames: Signal<Vec<RemoteFrame>>,
}

impl RemoteState {
    pub(crate) fn new() -> Self {
        RemoteState {
            client: Arc::new(Mutex::new(None)),
            status: Signal::new("Not connected.".to_owned()),
            connected: Signal::new(false),
            tree: Signal::new(Vec::new()),
            selected: Signal::new(None),
            details: Signal::new(Vec::new()),
            bindings: Signal::new(Vec::new()),
            problems: Signal::new(Vec::new()),
            updates: Signal::new(Vec::new()),
            frames: Signal::new(Vec::new()),
        }
    }

    /// Runs `requests` against the connection on a thread of its own and hands each answer
    /// back to the window's thread.
    fn fetch(&self, requests: Vec<RemoteRequest>) {
        let Ok(waker) = remedy_ui_core::reactive::waker() else { return };
        let me = self.clone_send();
        let client = self.client.clone();
        let _ = remedy_ui_core::exec::spawn("remedy-devtools-remote", move || {
            let mut guard = client.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(c) = guard.as_mut() else { return };
            for req in requests {
                let answer = c.request(&req);
                let lost = answer.is_err();

                let _ = waker.post(move || me.take(answer));
                if lost {
                    *guard = None;
                    break;
                }
            }
        });
    }

    fn clone_send(&self) -> RemoteSignals {
        RemoteSignals {
            status: self.status,
            connected: self.connected,
            tree: self.tree,
            details: self.details,
            bindings: self.bindings,
            problems: self.problems,
            updates: self.updates,
            frames: self.frames,
        }
    }
}

/// The signals answers land in: plain handles, so they cross to the connection's thread and
/// come back inside posted work.
#[derive(Clone, Copy)]
struct RemoteSignals {
    status: Signal<String>,
    connected: Signal<bool>,
    tree: Signal<Vec<RemoteElement>>,
    details: Signal<Vec<(String, String)>>,
    bindings: Signal<Vec<RemoteBinding>>,
    problems: Signal<Vec<RemoteProblem>>,
    updates: Signal<Vec<RemoteUpdate>>,
    frames: Signal<Vec<RemoteFrame>>,
}

impl RemoteSignals {
    fn take(&self, answer: std::io::Result<RemoteAnswer>) {
        match answer {
            Ok(RemoteAnswer::Tree { elements }) => {
                let _ = self.status.try_set(format!("Connected: {} elements.", elements.len()));
                let _ = self.tree.try_set(elements);
            }
            Ok(RemoteAnswer::Element { details, .. }) => {
                let _ = self.details.try_set(details);
            }
            Ok(RemoteAnswer::Bindings { bindings }) => {
                let _ = self.bindings.try_set(bindings);
            }
            Ok(RemoteAnswer::Semantics { problems, .. }) => {
                let _ = self.problems.try_set(problems);
            }
            Ok(RemoteAnswer::Updates { entries }) => {
                let _ = self.updates.try_set(entries);
            }
            Ok(RemoteAnswer::Frames { frames, .. }) => {
                let _ = self.frames.try_set(frames);
            }
            Ok(RemoteAnswer::Error { message }) => {
                let _ = self.status.try_set(format!("The app answered: {message}"));
            }
            Ok(RemoteAnswer::Graph { nodes }) => {
                let _ = self.status.try_set(format!("Connected: {} reactive nodes.", nodes.len()));
            }
            Ok(RemoteAnswer::Welcome { .. } | RemoteAnswer::Refused) => {}
            Err(e) => {
                let _ = self.status.try_set(format!("Connection lost: {e}"));
                let _ = self.connected.try_set(false);
            }
        }
    }
}

fn remote_page(ui: &Ui, insp: &Inspector) -> El {
    let unit = ui.theme().spacing.unit;
    let st = insp.0.remote.clone();
    let address = w::text_input(ui, "Address (host:port)");
    address.set_value("127.0.0.1:");
    let token = w::secret_input(ui, "Token");
    let (a2, t2, s2) = (address.clone(), token.clone(), st.clone());
    let connect = w::button(ui, "Connect", move || {
        let addr: Result<SocketAddr, _> = a2.value().trim().parse();
        let Ok(addr) = addr else {
            s2.status.set("Write the address as host:port, such as 127.0.0.1:9230.".to_owned());
            return;
        };
        let secret = t2.take_secret().expose().to_owned();
        let Ok(waker) = remedy_ui_core::reactive::waker() else { return };
        s2.status.set(format!("Connecting to {addr}…"));
        let (client, sig) = (s2.client.clone(), s2.clone_send());
        let _ = remedy_ui_core::exec::spawn("remedy-devtools-connect", move || {
            let result = RemoteClient::connect(addr, &secret, remote_timeout()).and_then(|mut c| {
                let first = c.request(&RemoteRequest::Tree);
                *client.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(c);
                first
            });
            let _ = waker.post(move || {
                let ok = result.is_ok();
                if let Err(e) = &result {
                    let _ = sig.status.try_set(format!("Could not connect: {e}"));
                }
                let _ = sig.connected.try_set(ok);
                if ok {
                    sig.take(result);
                }
            });
        });
    })
    .variant("primary");
    let s3 = st.clone();
    let refresh = w::button(ui, "Refresh", move || {
        let mut reqs = vec![
            RemoteRequest::Tree,
            RemoteRequest::Semantics,
            RemoteRequest::Updates,
            RemoteRequest::Frames,
        ];
        if let Ok(Some(id)) = s3.selected.get_untracked() {
            reqs.push(RemoteRequest::Element { id });
            reqs.push(RemoteRequest::Signals { id: Some(id) });
        }
        s3.fetch(reqs);
    });
    let status = note(ui, "").text({
        let s = st.status;
        move || s.get()
    });

    let tree = w::column(ui).fill_width();
    let (tr2, u2, s4) = (tree.clone(), ui.clone(), st.clone());
    let tree = tree.bind(move || {
        let els = s4.tree.get();
        let picked = s4.selected.get();
        untrack(|| {
            tr2.clear();
            let mut col = tr2.clone();
            for e in els {
                let s5 = s4.clone();
                let id = e.id;
                let row = u2
                    .el("list_item")
                    .row()
                    .fill_width()
                    .padding(Insets {
                        top: unit / 2.0,
                        bottom: unit / 2.0,
                        start: unit + e.depth as f32 * unit * 3.0,
                        end: unit,
                    })
                    .child(
                        w::text(&u2, e.label.clone())
                            .variant(if e.hidden { "muted" } else { "" })
                            .data_text()
                            .nowrap(),
                    )
                    .on_click(move || {
                        s5.selected.set(Some(id));
                        s5.fetch(vec![
                            RemoteRequest::Element { id },
                            RemoteRequest::Signals { id: Some(id) },
                        ]);
                    })
                    .semantics(
                        remedy_ui_core::a11y::Semantics::new(remedy_ui_core::a11y::Role::ListItem)
                            .label(e.label.clone())
                            .selected(picked == Some(id)),
                    );
                row.state(remedy_ui_core::style::States::SELECTED, picked == Some(id));
                col = col.child(row);
            }
        });
    });

    let detail = w::column(ui).gap(unit).fill_width();
    let (d2, u3, s6) = (detail.clone(), ui.clone(), st.clone());
    let detail = detail.bind(move || {
        let rows = s6.details.get();
        let bindings = s6.bindings.get();
        untrack(|| {
            d2.clear();
            let mut col = d2.clone();
            if rows.is_empty() {
                col = col.child(note(&u3, "Choose an element of the remote window."));
            }
            for (k, v) in rows {
                col = col.child(line(&u3, &k, v));
            }
            for b in bindings {
                col = col.child(line(&u3, "Binding", format!("{} ({})", b.node.label, b.node.created_at)));
                for r in b.reads {
                    let users: Vec<String> = r.dependents.iter().map(|d| d.label.clone()).collect();
                    col = col.child(line(
                        &u3,
                        "  reads",
                        format!(
                            "{} → used by {}",
                            r.node.label,
                            if users.is_empty() { "nothing else".into() } else { users.join(", ") }
                        ),
                    ));
                }
            }
            let _ = col;
        });
    });

    let more = w::column(ui).gap(unit).fill_width();
    let (m2, u4, s7) = (more.clone(), ui.clone(), st.clone());
    let more = more.bind(move || {
        let problems = s7.problems.get();
        let updates = s7.updates.get();
        let frames = s7.frames.get();
        untrack(|| {
            m2.clear();
            let mut col = m2.clone();
            col = col.child(w::text(&u4, format!("{} accessibility problems", problems.len())).data_text());
            for p in problems {
                col = col.child(warning(&u4, format!("⚠ {}", p.message)));
            }
            if let Some(f) = frames.last() {
                let parts: Vec<String> = f
                    .phases
                    .iter()
                    .filter(|(_, ns)| *ns > 0)
                    .map(|(n, ns)| format!("{n} {:.1}", *ns as f64 / 1e6))
                    .collect();
                col = col.child(line(
                    &u4,
                    "Last frame",
                    format!("{:.1} ms: {}", f.frame_ns as f64 / 1e6, parts.join(" · ")),
                ));
            }
            for u in updates.iter().rev() {
                col = col.child(w::text(&u4, u.sentence.clone()).data_text());
            }
            let _ = col;
        });
    });

    let connected = st.connected;
    let live = w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .child(refresh)
        .child(w::section(ui, "Remote elements").child(tree))
        .child(w::section(ui, "Selected remote element").child(detail))
        .child(w::section(ui, "Accessibility, updates and frames").child(more));
    let l2 = live.clone();
    let live = live.bind(move || {
        let on = connected.get();
        let _ = l2.clone().hidden(!on);
    });
    w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .child(note(
            ui,
            "Inspect an app running elsewhere: in the app, serve its window with RemoteServer::listen and read its address and token; for an Android emulator, forward the port with adb forward tcp:PORT tcp:PORT.",
        ))
        .child(w::section(ui, "Connect").child(address.el()).child(token.el()).child(connect).child(status))
        .child(live)
}
