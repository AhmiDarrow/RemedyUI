//! The trace recorder: a `tracing` subscriber that turns the engine's phase spans into
//! per-frame timings (the performance overlay) and, while capturing, into a whole-session
//! trace for Perfetto, with every thread and the GPU on one timeline.
//!
//! It records only while someone listens: a [`FrameFeed`] is alive (frame timings, engine
//! spans only) or a capture runs (everything). Otherwise each span costs the check of two
//! atomics, and nothing is allocated.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use remedy_ui_core::trace::phases;
use remedy_ui_core::trace::tracing::field::{Field, Visit};
use remedy_ui_core::trace::tracing::span::{Attributes, Id, Record};
use remedy_ui_core::trace::tracing::subscriber::Interest;
use remedy_ui_core::trace::tracing::{Dispatch, Event, Metadata, Subscriber};

/// The thread id the GPU's track gets in exported traces.
pub const GPU_TRACK: u64 = 0;

/// Why the recorder could not be installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallError;

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another global tracing subscriber is installed; devtools cannot record traces")
    }
}

impl std::error::Error for InstallError {}

/// One frame of one window, by phase ([`remedy_ui_core::trace::phases`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameSample {
    /// The window's view ([`remedy_ui_view::Ui::id`]), when the frame span named it.
    pub ui: Option<u64>,
    /// When the frame started, on the trace clock ([`remedy_ui_core::trace::now_ns`]).
    pub start_ns: u64,
    /// How long the frame took, from its start to its end.
    pub frame_ns: u64,
    /// Time in each phase, in [`phases::ALL`] order, since the window's previous frame
    /// ended: input and bindings run between frames are counted in the frame they lead to.
    /// Each phase's own time: nested phases are not counted twice.
    pub phases: [u64; 10],
    /// Engine time outside every named phase.
    pub other_ns: u64,
}

impl FrameSample {
    /// The time in the phase named `name` (0 for names that are not phases).
    pub fn phase(&self, name: &str) -> u64 {
        phases::ALL.iter().position(|p| *p == name).and_then(|i| self.phases.get(i)).copied().unwrap_or(0)
    }

    /// Everything the engine spent on the frame: every phase, and the rest.
    pub fn total_ns(&self) -> u64 {
        self.phases.iter().sum::<u64>() + self.other_ns
    }
}

/// A span or event as a trace holds it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Slice {
    /// The track (thread) it ran on; [`GPU_TRACK`] for GPU work.
    pub track: u64,
    /// Its name.
    pub name: String,
    /// Its target (category).
    pub target: String,
    /// Its start on the trace clock.
    pub start_ns: u64,
    /// How long it lasted; `None` for an instant event.
    pub dur_ns: Option<u64>,
    /// Its fields, formatted.
    pub args: Vec<(String, String)>,
}

/// A captured session ([`Recorder::stop_capture`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    /// Each track's id and name (threads, then the GPU when it has slices).
    pub tracks: Vec<(u64, String)>,
    /// Every span that ended and every event, in the order they were recorded.
    pub slices: Vec<Slice>,
}

impl Trace {
    /// The trace in the Chrome trace event format (JSON), which Perfetto UI
    /// (ui.perfetto.dev) and `chrome://tracing` open: one process, a track per thread and
    /// one for the GPU, complete events for spans and instant events for events.
    pub fn to_chrome_json(&self) -> String {
        let pid = u64::from(std::process::id());
        let mut events: Vec<serde_json::Value> =
            Vec::with_capacity(self.slices.len() + self.tracks.len() + 1);
        events.push(serde_json::json!({
            "ph": "M", "name": "process_name", "pid": pid, "tid": 0,
            "args": { "name": "RemedyUI app" },
        }));
        for (tid, name) in &self.tracks {
            events.push(serde_json::json!({
                "ph": "M", "name": "thread_name", "pid": pid, "tid": tid, "args": { "name": name },
            }));
            events.push(serde_json::json!({
                "ph": "M", "name": "thread_sort_index", "pid": pid, "tid": tid,
                "args": { "sort_index": if *tid == GPU_TRACK { u64::MAX >> 12 } else { *tid } },
            }));
        }
        for s in &self.slices {
            let args: serde_json::Map<String, serde_json::Value> =
                s.args.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
            // Microseconds, with the nanoseconds kept as fractions.
            let ts = s.start_ns as f64 / 1000.0;
            let mut e = serde_json::json!({
                "name": s.name, "cat": s.target, "pid": pid, "tid": s.track, "ts": ts, "args": args,
            });
            if let Some(obj) = e.as_object_mut() {
                match s.dur_ns {
                    Some(d) => {
                        obj.insert("ph".into(), "X".into());
                        obj.insert("dur".into(), serde_json::json!(d as f64 / 1000.0));
                    }
                    None => {
                        obj.insert("ph".into(), "i".into());
                        obj.insert("s".into(), "t".into());
                    }
                }
            }
            events.push(e);
        }
        serde_json::json!({ "traceEvents": events, "displayTimeUnit": "ms" }).to_string()
    }

    /// Writes [`Trace::to_chrome_json`] to `path`.
    pub fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_chrome_json())
    }
}

/// Frame timings as they happen ([`Recorder::frames`]). Keeps the newest `keep` samples
/// until taken; dropping it stops sampling when no other feed is left.
pub struct FrameFeed {
    inner: Arc<FeedInner>,
    shared: Arc<Shared>,
}

/// Told about each frame sample a feed receives.
type SampleHook = Box<dyn Fn(&FrameSample) + Send + Sync>;

struct FeedInner {
    keep: AtomicUsize,
    queue: Mutex<VecDeque<FrameSample>>,
    notify: Mutex<Option<SampleHook>>,
}

impl std::fmt::Debug for FrameFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FrameFeed(keep {})", self.inner.keep.load(Ordering::Relaxed))
    }
}

impl FrameFeed {
    /// The samples since the last call, oldest first.
    pub fn take(&self) -> Vec<FrameSample> {
        self.inner.queue.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default()
    }

    /// How many samples it holds between takes; older ones are dropped first.
    pub fn set_keep(&self, keep: usize) {
        self.inner.keep.store(keep.max(1), Ordering::Relaxed);
    }

    /// Calls `f` (on the thread that finished the frame, right after it) with each sample
    /// that arrives.
    pub fn on_sample(&self, f: impl Fn(&FrameSample) + Send + Sync + 'static) {
        if let Ok(mut n) = self.inner.notify.lock() {
            *n = Some(Box::new(f));
        }
    }
}

impl Drop for FrameFeed {
    fn drop(&mut self) {
        self.shared.sampling.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The process's trace recorder ([`Recorder::install`]).
#[derive(Clone)]
pub struct Recorder {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Recorder(capturing: {})", self.is_capturing())
    }
}

#[derive(Default)]
struct Shared {
    capturing: AtomicBool,
    sampling: AtomicUsize,
    /// Counts the times sampling started from nothing: phase times a thread kept from an
    /// earlier sampling session are dropped instead of landing in a much later frame.
    sessions: AtomicU64,
    next_span: AtomicU64,
    spans: Mutex<HashMap<u64, SpanMeta>>,
    capture: Mutex<Vec<Slice>>,
    tracks: Mutex<Vec<(u64, String)>>,
    feeds: Mutex<Vec<Weak<FeedInner>>>,
}

struct SpanMeta {
    name: &'static str,
    target: &'static str,
    engine: bool,
    fields: Vec<(&'static str, String)>,
    ui: Option<u64>,
    refs: usize,
}

/// A span entered on this thread.
struct Open {
    id: u64,
    start_ns: u64,
    child_ns: u64,
    /// The phase its own time counts in (`None`: outside every phase).
    phase: Option<usize>,
    ui: Option<u64>,
    engine: bool,
}

#[derive(Default)]
struct ThreadState {
    track: u64,
    stack: Vec<Open>,
    /// Phase times since each window's last frame (by view id; `None` for work that named
    /// no window, counted in the next frame on this thread).
    pending: HashMap<Option<u64>, ([u64; 10], u64)>,
    /// The sampling session `pending` belongs to.
    session: u64,
}

static NEXT_TRACK: AtomicU64 = AtomicU64::new(GPU_TRACK + 1);

thread_local! {
    static THREAD: RefCell<ThreadState> = RefCell::new(ThreadState::default());
}

static GLOBAL: OnceLock<Result<Recorder, InstallError>> = OnceLock::new();

impl Recorder {
    /// Installs the recorder as the process's global `tracing` subscriber (once; later calls
    /// return the same recorder). Fails when the app installed a subscriber of its own first:
    /// traces then go to the app's subscriber, and devtools say so.
    pub fn install() -> Result<Recorder, InstallError> {
        GLOBAL
            .get_or_init(|| {
                let shared = Arc::new(Shared { next_span: AtomicU64::new(1), ..Shared::default() });
                let sub = Sub { shared: shared.clone() };
                match remedy_ui_core::trace::tracing::dispatcher::set_global_default(Dispatch::new(sub)) {
                    Ok(()) => Ok(Recorder { shared }),
                    Err(_) => Err(InstallError),
                }
            })
            .clone()
    }

    /// The installed recorder, if [`Recorder::install`] succeeded.
    pub fn get() -> Option<Recorder> {
        GLOBAL.get().and_then(|r| r.as_ref().ok()).cloned()
    }

    /// Starts sampling frame timings into a new feed holding up to `keep` samples.
    pub fn frames(&self, keep: usize) -> FrameFeed {
        let inner = Arc::new(FeedInner {
            keep: AtomicUsize::new(keep.max(1)),
            queue: Mutex::new(VecDeque::new()),
            notify: Mutex::new(None),
        });
        if let Ok(mut f) = self.shared.feeds.lock() {
            f.retain(|w| w.strong_count() > 0);
            f.push(Arc::downgrade(&inner));
        }
        if self.shared.sampling.fetch_add(1, Ordering::SeqCst) == 0 {
            self.shared.sessions.fetch_add(1, Ordering::SeqCst);
        }
        FrameFeed { inner, shared: self.shared.clone() }
    }

    /// Starts capturing every span and event of every thread (and GPU work) until
    /// [`Recorder::stop_capture`]. A capture already running goes on.
    pub fn start_capture(&self) {
        if let Ok(mut c) = self.shared.capture.lock()
            && !self.shared.capturing.load(Ordering::SeqCst)
        {
            c.clear();
        }
        self.shared.capturing.store(true, Ordering::SeqCst);
    }

    /// True while capturing.
    pub fn is_capturing(&self) -> bool {
        self.shared.capturing.load(Ordering::SeqCst)
    }

    /// Stops capturing and returns the session.
    pub fn stop_capture(&self) -> Trace {
        self.shared.capturing.store(false, Ordering::SeqCst);
        let slices = self.shared.capture.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default();
        let mut tracks: Vec<(u64, String)> = self.shared.tracks.lock().map(|t| t.clone()).unwrap_or_default();
        tracks.retain(|(t, _)| slices.iter().any(|s| s.track == *t));
        if slices.iter().any(|s| s.track == GPU_TRACK) {
            tracks.push((GPU_TRACK, "GPU".to_owned()));
        }
        Trace { tracks, slices }
    }
}

struct Sub {
    shared: Arc<Shared>,
}

/// Collects a span's or event's fields as text, and a `ui` field as the view it names.
#[derive(Default)]
struct Fields {
    list: Vec<(&'static str, String)>,
    ui: Option<u64>,
    begin_ns: Option<u64>,
    end_ns: Option<u64>,
    what: Option<String>,
}

impl Visit for Fields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "ui" => self.ui = Some(value),
            "begin_ns" => self.begin_ns = Some(value),
            "end_ns" => self.end_ns = Some(value),
            _ => {}
        }
        self.list.push((field.name(), value.to_string()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "what" {
            self.what = Some(value.to_owned());
        }
        self.list.push((field.name(), value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut s = String::new();
        let _ = write!(s, "{value:?}");
        self.list.push((field.name(), s));
    }
}

fn is_engine(meta: &Metadata<'_>) -> bool {
    meta.target() == remedy_ui_core::trace::TARGET
}

fn phase_index(name: &str) -> Option<usize> {
    phases::ALL.iter().position(|p| *p == name)
}

impl Sub {
    fn wants(&self, meta: &Metadata<'_>) -> bool {
        if meta.target() == remedy_ui_core::trace::GPU_TARGET {
            return self.shared.capturing.load(Ordering::Relaxed);
        }
        self.shared.capturing.load(Ordering::Relaxed)
            || (is_engine(meta) && self.shared.sampling.load(Ordering::Relaxed) > 0)
    }

    /// This thread's track, registered with its name on first use.
    fn track(&self, t: &mut ThreadState) -> u64 {
        if t.track == 0 {
            t.track = NEXT_TRACK.fetch_add(1, Ordering::Relaxed);
            let cur = std::thread::current();
            let name = cur.name().map_or_else(|| format!("thread {}", t.track), str::to_owned);
            if let Ok(mut tr) = self.shared.tracks.lock() {
                tr.push((t.track, name));
            }
        }
        t.track
    }

    fn push_capture(&self, s: Slice) {
        if let Ok(mut c) = self.shared.capture.lock() {
            c.push(s);
        }
    }

    fn deliver(&self, sample: FrameSample) {
        let feeds: Vec<Arc<FeedInner>> = self
            .shared
            .feeds
            .lock()
            .map(|f| f.iter().filter_map(Weak::upgrade).collect())
            .unwrap_or_default();
        for f in feeds {
            if let Ok(mut q) = f.queue.lock() {
                q.push_back(sample.clone());
                let keep = f.keep.load(Ordering::Relaxed);
                while q.len() > keep {
                    q.pop_front();
                }
            }
            if let Ok(n) = f.notify.lock()
                && let Some(n) = n.as_ref()
            {
                n(&sample);
            }
        }
    }
}

impl Subscriber for Sub {
    fn register_callsite(&self, _meta: &'static Metadata<'static>) -> Interest {
        // What the recorder wants changes as feeds and captures come and go: every callsite
        // asks `enabled` each time (two atomic loads) instead of caching an answer.
        Interest::sometimes()
    }

    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        self.wants(meta)
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let id = self.shared.next_span.fetch_add(1, Ordering::Relaxed).max(1);
        let mut fields = Fields::default();
        span.record(&mut fields);
        let meta = span.metadata();
        let s = SpanMeta {
            name: meta.name(),
            target: meta.target(),
            engine: is_engine(meta),
            fields: fields.list,
            ui: fields.ui,
            refs: 1,
        };
        if let Ok(mut m) = self.shared.spans.lock() {
            m.insert(id, s);
        }
        Id::from_u64(id)
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        let mut fields = Fields::default();
        values.record(&mut fields);
        if let Ok(mut m) = self.shared.spans.lock()
            && let Some(s) = m.get_mut(&span.into_u64())
        {
            if fields.ui.is_some() {
                s.ui = fields.ui;
            }
            s.fields.extend(fields.list);
        }
    }

    fn record_follows_from(&self, span: &Id, follows: &Id) {
        if let Ok(mut m) = self.shared.spans.lock() {
            let other = m.get(&follows.into_u64()).map(|s| s.name);
            if let (Some(other), Some(s)) = (other, m.get_mut(&span.into_u64())) {
                s.fields.push(("follows", other.to_owned()));
            }
        }
    }

    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);
        if meta.target() == remedy_ui_core::trace::GPU_TARGET {
            if let (Some(b), Some(e)) = (fields.begin_ns, fields.end_ns) {
                self.push_capture(Slice {
                    track: GPU_TRACK,
                    name: fields.what.unwrap_or_else(|| "gpu".to_owned()),
                    target: meta.target().to_owned(),
                    start_ns: b,
                    dur_ns: Some(e.saturating_sub(b)),
                    args: vec![("measured".to_owned(), "GPU timestamps, placed at submit".to_owned())],
                });
            }
            return;
        }
        if !self.shared.capturing.load(Ordering::Relaxed) {
            return;
        }
        let track = THREAD.with(|t| self.track(&mut t.borrow_mut()));
        self.push_capture(Slice {
            track,
            name: meta.name().to_owned(),
            target: meta.target().to_owned(),
            start_ns: remedy_ui_core::trace::now_ns(),
            dur_ns: None,
            args: fields.list.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        });
    }

    fn enter(&self, span: &Id) {
        let id = span.into_u64();
        let (name, ui, engine) = match self.shared.spans.lock() {
            Ok(m) => match m.get(&id) {
                Some(s) => (s.name, s.ui, s.engine),
                None => return,
            },
            Err(_) => return,
        };
        let start_ns = remedy_ui_core::trace::now_ns();
        THREAD.with(|t| {
            let mut t = t.borrow_mut();
            let _ = self.track(&mut t);
            let parent = t.stack.last();
            let phase =
                if engine { phase_index(name).or_else(|| parent.and_then(|p| p.phase)) } else { None };
            let ui = ui.or_else(|| parent.and_then(|p| p.ui));
            t.stack.push(Open { id, start_ns, child_ns: 0, phase, ui, engine });
        });
    }

    fn exit(&self, span: &Id) {
        let id = span.into_u64();
        let end = remedy_ui_core::trace::now_ns();
        let Some((open, track)) = THREAD.with(|t| {
            let mut t = t.borrow_mut();
            let at = t.stack.iter().rposition(|o| o.id == id)?;
            let open = t.stack.remove(at);
            let dur = end.saturating_sub(open.start_ns);
            // The parent's own time excludes this span's.
            if let Some(p) = at.checked_sub(1).and_then(|i| t.stack.get_mut(i)) {
                p.child_ns += dur;
            }
            Some((open, t.track))
        }) else {
            return;
        };
        let dur = end.saturating_sub(open.start_ns);
        let own = dur.saturating_sub(open.child_ns);
        let meta = match self.shared.spans.lock() {
            Ok(m) => m.get(&id).map(|s| (s.name, s.target, s.fields.clone())),
            Err(_) => None,
        };
        let Some((name, target, fields)) = meta else { return };
        if self.shared.capturing.load(Ordering::Relaxed) {
            self.push_capture(Slice {
                track,
                name: name.to_owned(),
                target: target.to_owned(),
                start_ns: open.start_ns,
                dur_ns: Some(dur),
                args: fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            });
        }
        if !open.engine || self.shared.sampling.load(Ordering::Relaxed) == 0 {
            return;
        }
        let sample = THREAD.with(|t| {
            let mut t = t.borrow_mut();
            let session = self.shared.sessions.load(Ordering::Relaxed);
            if t.session != session {
                t.pending.clear();
                t.session = session;
            }
            let entry = t.pending.entry(open.ui).or_insert(([0; 10], 0));
            match open.phase.and_then(|i| entry.0.get_mut(i)) {
                Some(slot) => *slot += own,
                None => entry.1 += own,
            }
            if name != phases::FRAME {
                return None;
            }
            let (mut ph, mut other) = t.pending.remove(&open.ui).unwrap_or(([0; 10], 0));
            if open.ui.is_some()
                && let Some((p, o)) = t.pending.remove(&None)
            {
                for (a, b) in ph.iter_mut().zip(p) {
                    *a += b;
                }
                other += o;
            }
            Some(FrameSample {
                ui: open.ui,
                start_ns: open.start_ns,
                frame_ns: dur,
                phases: ph,
                other_ns: other,
            })
        });
        if let Some(s) = sample {
            self.deliver(s);
        }
    }

    fn clone_span(&self, id: &Id) -> Id {
        if let Ok(mut m) = self.shared.spans.lock()
            && let Some(s) = m.get_mut(&id.into_u64())
        {
            s.refs += 1;
        }
        id.clone()
    }

    fn try_close(&self, id: Id) -> bool {
        let Ok(mut m) = self.shared.spans.lock() else { return false };
        let key = id.into_u64();
        let Some(s) = m.get_mut(&key) else { return false };
        s.refs = s.refs.saturating_sub(1);
        if s.refs == 0 {
            m.remove(&key);
            true
        } else {
            false
        }
    }
}
