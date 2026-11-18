//! Remote devtools: an app serves its window's element tree, element details, semantics,
//! signal graph, "why did this update" log and frame timings over a local, authenticated
//! connection, and a devtools window elsewhere (on the desktop, for an app on an Android
//! emulator through `adb forward`, or another process) connects to it.
//!
//! - **Local.** The server listens on the loopback interface only; reaching a device goes
//!   through a forward the developer sets up (`adb forward tcp:PORT tcp:PORT`).
//! - **Authenticated.** A connection opens by sending the server's token on its own line;
//!   anything else is refused before a byte reaches the window. Tokens are 128 random bits,
//!   compared in constant time.
//! - **Debug builds only by default.** [`RemoteServer::listen`] refuses in a release build
//!   unless [`RemoteOptions::allow_release`] says otherwise.
//!
//! After the token, each line is one [`RemoteRequest`] in JSON, answered by one line holding
//! a [`RemoteAnswer`]. Requests are answered on the window's thread: connection threads post
//! them through the reactive runtime's waker, so the app's loop wakes for them like any other
//! work.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::token::{TOKEN_LEN, random_token, same_secret};
use remedy_ui_reactive::debug::NodeInfo;
use remedy_ui_view::Ui;
use serde::{Deserialize, Serialize};

use crate::recorder::{FrameFeed, FrameSample, Recorder};
use crate::signals::{UpdateLog, element_signals};

/// A request to a remote app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RemoteRequest {
    /// Every element of the window, depth first.
    Tree,
    /// One element's details.
    Element {
        /// Its id ([`RemoteElement::id`]).
        id: u64,
    },
    /// The window as a screen reader meets it, and the accessibility problems found.
    Semantics,
    /// An element's bindings, what they read and what depends on that; or, with no id,
    /// every node of the window thread's reactive graph.
    Signals {
        /// The element ([`RemoteElement::id`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
    },
    /// The recent "why did this update" entries.
    Updates,
    /// The recent frames' timings.
    Frames,
}

/// An element in the remote tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteElement {
    /// Its id (its accessibility node id).
    pub id: u64,
    /// Its parent's id.
    pub parent: Option<u64>,
    /// How deep it is (layer roots are 0).
    pub depth: usize,
    /// Its widget name.
    pub widget: String,
    /// A one-line description ([`crate::describe`]).
    pub label: String,
    /// Hidden.
    pub hidden: bool,
    /// Its box in window logical pixels: x, y, width, height.
    pub bounds: Option<[f32; 4]>,
}

/// A node of the reactive graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteNode {
    /// Its key.
    pub key: u64,
    /// Signal, memo, effect, reaction or scope.
    pub kind: String,
    /// Its name, or its kind and creation site.
    pub label: String,
    /// Where it was created.
    pub created_at: String,
    /// What it read.
    pub sources: Vec<u64>,
    /// What reads it.
    pub subscribers: Vec<u64>,
    /// The element whose binding or painter it is, in the served window.
    pub element: Option<u64>,
}

/// A signal or memo a binding read, with what depends on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteRead {
    /// The signal or memo.
    pub node: RemoteNode,
    /// What depends on it, nearest first.
    pub dependents: Vec<RemoteNode>,
}

/// One binding of an element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteBinding {
    /// The binding or painter.
    pub node: RemoteNode,
    /// What it read.
    pub reads: Vec<RemoteRead>,
}

/// A node as a screen reader meets it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteReaderNode {
    /// The element's id.
    pub id: u64,
    /// Its depth.
    pub depth: usize,
    /// What a screen reader says on reaching it.
    pub announcement: String,
    /// The problems found on it.
    pub problems: Vec<String>,
}

/// An accessibility problem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteProblem {
    /// The element's id.
    pub element: u64,
    /// What is wrong, in a sentence.
    pub message: String,
}

/// An element marked for work, with why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteUpdate {
    /// The element's id.
    pub element: u64,
    /// The whole sentence: what updated and why.
    pub sentence: String,
    /// When, on the app's trace clock.
    pub at_ns: u64,
}

/// One frame's timings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RemoteFrame {
    /// When it started, on the app's trace clock.
    pub start_ns: u64,
    /// How long it took.
    pub frame_ns: u64,
    /// Each phase's time, by name, in pipeline order.
    pub phases: Vec<(String, u64)>,
    /// Engine time outside every phase.
    pub other_ns: u64,
}

/// An answer from a remote app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RemoteAnswer {
    /// The token was accepted; requests may follow.
    Welcome {
        /// The protocol's version.
        version: u32,
    },
    /// The token was wrong: the connection closes.
    Refused,
    /// The request could not be answered.
    Error {
        /// Why.
        message: String,
    },
    /// [`RemoteRequest::Tree`].
    Tree {
        /// The elements, depth first.
        elements: Vec<RemoteElement>,
    },
    /// [`RemoteRequest::Element`].
    Element {
        /// The element's id.
        id: u64,
        /// Its details as name and value, as the inspector shows them.
        details: Vec<(String, String)>,
    },
    /// [`RemoteRequest::Semantics`].
    Semantics {
        /// The reader's view, in reading order.
        nodes: Vec<RemoteReaderNode>,
        /// Every problem found, those on unreachable elements included.
        problems: Vec<RemoteProblem>,
    },
    /// [`RemoteRequest::Signals`] with an element.
    Bindings {
        /// Its bindings.
        bindings: Vec<RemoteBinding>,
    },
    /// [`RemoteRequest::Signals`] with no element.
    Graph {
        /// Every node.
        nodes: Vec<RemoteNode>,
    },
    /// [`RemoteRequest::Updates`].
    Updates {
        /// Oldest first.
        entries: Vec<RemoteUpdate>,
    },
    /// [`RemoteRequest::Frames`].
    Frames {
        /// Oldest first.
        frames: Vec<RemoteFrame>,
        /// Frame timings are recorded (false when the app's own tracing subscriber took the
        /// place devtools record from).
        recording: bool,
    },
}

/// The protocol's version, sent in [`RemoteAnswer::Welcome`].
pub const PROTOCOL_VERSION: u32 = 1;

/// How a [`RemoteServer`] listens.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RemoteOptions {
    /// The loopback port; 0 lets the system pick one (read it from [`RemoteServer::addr`]).
    /// A fixed port suits `adb forward`.
    pub port: u16,
    /// Serve in a release build too. Off by default: remote devtools are for debug builds.
    pub allow_release: bool,
    /// The token clients must send; `None` draws a fresh random one.
    pub token: Option<String>,
    /// How many "why did this update" entries are kept for clients between requests.
    pub keep_updates: usize,
    /// How many frames' timings are kept for clients between requests.
    pub keep_frames: usize,
}

impl RemoteOptions {
    /// Sets `port`: the loopback port; 0 lets the system pick one (read it from [`RemoteServer::addr`]).
    pub fn port(mut self, value: u16) -> Self {
        self.port = value;
        self
    }

    /// Sets `allow_release`: serve in a release build too.
    pub fn allow_release(mut self, value: bool) -> Self {
        self.allow_release = value;
        self
    }

    /// Sets `token`: the token clients must send; `None` draws a fresh random one.
    pub fn token(mut self, value: String) -> Self {
        self.token = Some(value);
        self
    }

    /// Sets `keep_updates`: how many "why did this update" entries are kept for clients between requests.
    pub fn keep_updates(mut self, value: usize) -> Self {
        self.keep_updates = value;
        self
    }

    /// Sets `keep_frames`: how many frames' timings are kept for clients between requests.
    pub fn keep_frames(mut self, value: usize) -> Self {
        self.keep_frames = value;
        self
    }
}

impl Default for RemoteOptions {
    fn default() -> Self {
        RemoteOptions {
            port: 0,
            allow_release: false,
            token: None,
            keep_updates: 500,
            keep_frames: crate::overlay::graph_frames(),
        }
    }
}

thread_local! {
    /// The windows served from this thread, by server.
    static SERVING: RefCell<HashMap<u64, Served>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

struct Served {
    ui: Ui,
    log: UpdateLog,
    feed: Option<FrameFeed>,
    frames: VecDeque<FrameSample>,
    keep_frames: usize,
}

/// A window served to remote devtools. Dropping it stops serving and closes connections.
#[derive(Debug)]
pub struct RemoteServer {
    id: u64,
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<TcpStream>>>,
}

impl RemoteServer {
    /// Serves `ui` on a loopback port. Must be called on the window's thread, which must run
    /// the reactive runtime's posted work (every bridge does). Fails in a release build
    /// unless [`RemoteOptions::allow_release`] is set.
    pub fn listen(ui: &Ui, options: RemoteOptions) -> std::io::Result<RemoteServer> {
        if !cfg!(debug_assertions) && !options.allow_release {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "remote devtools serve debug builds only (set allow_release to serve this one)",
            ));
        }
        let token = match options.token {
            Some(t) if !t.is_empty() && t.len() <= TOKEN_LEN && !t.contains(['\n', '\r']) => t,
            Some(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "a token is one line of at most 32 characters",
                ));
            }
            None => random_token(),
        };
        let waker = remedy_ui_reactive::waker().map_err(std::io::Error::other)?;
        let socket = TcpListener::bind(("127.0.0.1", options.port))?;
        let addr = socket.local_addr()?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let feed = Recorder::install().ok().map(|r| r.frames(options.keep_frames.max(1)));
        let served = Served {
            ui: ui.clone(),
            log: UpdateLog::attach(ui, options.keep_updates),
            feed,
            frames: VecDeque::new(),
            keep_frames: options.keep_frames.max(1),
        };
        SERVING.with(|s| s.borrow_mut().insert(id, served));
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let (secret, stop2, conns) = (token.clone(), stop.clone(), connections.clone());
        std::thread::Builder::new().name("remedy-devtools".into()).spawn(move || {
            for stream in socket.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                if let Ok(c) = stream.try_clone() {
                    conns.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(c);
                }
                let (secret, waker, stop) = (secret.clone(), waker.clone(), stop2.clone());
                let _ = std::thread::Builder::new().name("remedy-devtools-client".into()).spawn(move || {
                    // Strangers are turned away here, before reaching the window.
                    let known = read_token(&mut stream).is_some_and(|t| same_secret(&t, &secret));
                    if !known {
                        let _ = writeln!(stream, "{}", json(&RemoteAnswer::Refused));
                        let _ = stream.shutdown(Shutdown::Both);
                        return;
                    }
                    if writeln!(stream, "{}", json(&RemoteAnswer::Welcome { version: PROTOCOL_VERSION }))
                        .is_err()
                    {
                        return;
                    }
                    serve(stream, id, &waker, &stop);
                });
            }
        })?;
        Ok(RemoteServer { id, addr, token, stop, connections })
    }

    /// Where clients connect.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The token clients must send.
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accepting thread so it sees the stop.
        let _ = TcpStream::connect(self.addr);
        for c in self.connections.lock().unwrap_or_else(std::sync::PoisonError::into_inner).drain(..) {
            let _ = c.shutdown(Shutdown::Both);
        }
        let id = self.id;
        let _ = SERVING.try_with(|s| s.borrow_mut().remove(&id));
    }
}

fn json(a: &RemoteAnswer) -> String {
    serde_json::to_string(a).unwrap_or_else(|e| {
        format!("{{\"answer\":\"error\",\"message\":{}}}", serde_json::Value::String(e.to_string()))
    })
}

/// Reads the opening token line, never more than a token and its line break.
fn read_token(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(TOKEN_LEN + 2);
    let mut byte = [0u8; 1];
    while buf.len() <= TOKEN_LEN + 1 {
        match stream.read(&mut byte) {
            Ok(1) => {}
            _ => return None,
        }
        if byte[0] == b'\n' {
            let text = String::from_utf8(buf).ok()?;
            return Some(text.trim_end_matches('\r').to_owned());
        }
        buf.push(byte[0]);
    }
    None
}

/// Answers one connection's requests until it closes.
fn serve(stream: TcpStream, id: u64, waker: &remedy_ui_reactive::Waker, stop: &AtomicBool) {
    let Ok(mut out) = stream.try_clone() else { return };
    let (tx, rx) = channel::<String>();
    // Answers are written in the order they come, from the window's thread.
    let writer = std::thread::spawn(move || {
        for line in rx {
            if writeln!(out, "{line}").is_err() {
                break;
            }
        }
    });
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let tx: Sender<String> = tx.clone();
        if waker.post(move || answer(id, &line, &tx)).is_err() {
            break;
        }
    }
    drop(tx);
    let _ = writer.join();
}

/// Runs on the window's thread: answers `line` and sends the answer back.
fn answer(id: u64, line: &str, tx: &Sender<String>) {
    let reply = match serde_json::from_str::<RemoteRequest>(line) {
        Ok(req) => SERVING.with(|s| match s.borrow_mut().get_mut(&id) {
            Some(served) => respond(served, &req),
            None => RemoteAnswer::Error { message: "the window is closed".into() },
        }),
        Err(e) => RemoteAnswer::Error { message: format!("not a request: {e}") },
    };
    let _ = tx.send(json(&reply));
}

fn remote_node(ui: &Ui, n: &NodeInfo) -> RemoteNode {
    RemoteNode {
        key: n.key.0,
        kind: n.kind.as_str().to_owned(),
        label: n.label(),
        created_at: format!("{}:{}", n.created_at.file(), n.created_at.line()),
        sources: n.sources.iter().map(|k| k.0).collect(),
        subscribers: n.subscribers.iter().map(|k| k.0).collect(),
        element: ui.element_of_binding(n.key).map(|e| ui.element(e).node_id()),
    }
}

fn frame(s: &FrameSample) -> RemoteFrame {
    RemoteFrame {
        start_ns: s.start_ns,
        frame_ns: s.frame_ns,
        phases: remedy_ui_trace::phases::ALL.iter().map(|p| ((*p).to_owned(), s.phase(p))).collect(),
        other_ns: s.other_ns,
    }
}

/// Answers one request about a served window.
fn respond(served: &mut Served, req: &RemoteRequest) -> RemoteAnswer {
    let ui = served.ui.clone();
    match req {
        RemoteRequest::Tree => RemoteAnswer::Tree { elements: tree(&ui) },
        RemoteRequest::Element { id } => match ui.element_by_node_id(*id).and_then(|e| e.info()) {
            Some(info) => RemoteAnswer::Element {
                id: *id,
                details: crate::inspect::detail_rows(&info)
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect(),
            },
            None => RemoteAnswer::Error { message: format!("no element {id}") },
        },
        RemoteRequest::Semantics => {
            let nodes = crate::inspect::reader_lines(&ui)
                .into_iter()
                .map(|l| RemoteReaderNode {
                    id: l.node.id,
                    depth: l.node.depth,
                    announcement: l.node.announcement(),
                    problems: l.problems.iter().map(crate::Problem::message).collect(),
                })
                .collect();
            let problems = crate::audit(&ui)
                .iter()
                .map(|p| RemoteProblem { element: ui.element(p.element).node_id(), message: p.message() })
                .collect();
            RemoteAnswer::Semantics { nodes, problems }
        }
        RemoteRequest::Signals { id: Some(id) } => match ui.element_by_node_id(*id) {
            Some(el) => RemoteAnswer::Bindings {
                bindings: element_signals(&ui, el.id())
                    .into_iter()
                    .map(|b| RemoteBinding {
                        node: remote_node(&ui, &b.node),
                        reads: b
                            .reads
                            .into_iter()
                            .map(|r| RemoteRead {
                                node: remote_node(&ui, &r.node),
                                dependents: r.dependents.iter().map(|d| remote_node(&ui, &d.node)).collect(),
                            })
                            .collect(),
                    })
                    .collect(),
            },
            None => RemoteAnswer::Error { message: format!("no element {id}") },
        },
        RemoteRequest::Signals { id: None } => RemoteAnswer::Graph {
            nodes: remedy_ui_reactive::debug::graph().iter().map(|n| remote_node(&ui, n)).collect(),
        },
        RemoteRequest::Updates => RemoteAnswer::Updates {
            entries: served
                .log
                .entries()
                .into_iter()
                .map(|e| RemoteUpdate {
                    element: ui.element(e.record.element).node_id(),
                    sentence: e.sentence(),
                    at_ns: e.record.at_ns,
                })
                .collect(),
        },
        RemoteRequest::Frames => {
            let me = ui.id();
            if let Some(feed) = &served.feed {
                served.frames.extend(feed.take().into_iter().filter(|s| s.ui == Some(me)));
                while served.frames.len() > served.keep_frames {
                    served.frames.pop_front();
                }
            }
            RemoteAnswer::Frames {
                frames: served.frames.iter().map(frame).collect(),
                recording: served.feed.is_some(),
            }
        }
    }
}

/// The served window's elements, depth first.
fn tree(ui: &Ui) -> Vec<RemoteElement> {
    crate::walk(ui)
        .into_iter()
        .filter_map(|(depth, el)| {
            let info = el.info()?;
            Some(RemoteElement {
                id: el.node_id(),
                parent: el.parent().map(|p| p.node_id()),
                depth,
                widget: info.widget.to_owned(),
                label: crate::describe(&info),
                hidden: info.hidden,
                bounds: info.bounds.map(|b| [b.min.x, b.min.y, b.width(), b.height()]),
            })
        })
        .collect()
}

/// A connection to a remote app's devtools server. Blocking: use it off the UI thread.
#[derive(Debug)]
pub struct RemoteClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl RemoteClient {
    /// Connects to `addr` and presents `token`. Fails with `PermissionDenied` when the
    /// server refuses the token, and when nothing answers within `timeout`.
    pub fn connect(addr: SocketAddr, token: &str, timeout: Duration) -> std::io::Result<RemoteClient> {
        let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        writeln!(stream, "{token}")?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = RemoteClient { stream, reader };
        match client.read_answer()? {
            RemoteAnswer::Welcome { .. } => Ok(client),
            RemoteAnswer::Refused => {
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "the app refused the token"))
            }
            other => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unexpected greeting: {other:?}"),
            )),
        }
    }

    fn read_answer(&mut self) -> std::io::Result<RemoteAnswer> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the app closed the connection",
            ));
        }
        serde_json::from_str(line.trim_end())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Sends `req` and waits for its answer.
    pub fn request(&mut self, req: &RemoteRequest) -> std::io::Result<RemoteAnswer> {
        let line = serde_json::to_string(req).map_err(std::io::Error::other)?;
        writeln!(self.stream, "{line}")?;
        self.read_answer()
    }
}
