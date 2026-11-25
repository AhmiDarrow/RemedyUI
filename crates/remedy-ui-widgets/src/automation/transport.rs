//! The local transport: agents in other processes on this machine connect over a loopback
//! socket. A connection opens by sending a grant's token on its own line; anything else closes
//! it before a byte reaches the window. After that, each line is one request of the JSON
//! protocol and each answer comes back as one line (an answer that waits for the person is
//! followed, later, by the outcome on its own line).
//!
//! Requests are answered on the window's thread: connection threads post them through the
//! reactive runtime's waker, so the app's loop wakes for them like any other work.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use crate::automation::{Agents, Answer, Refused};

thread_local! {
    /// The windows listening on this thread, by listener.
    static LISTENING: RefCell<HashMap<u64, Agents>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// The length of a grant's token: 128 bits as hex.
const TOKEN_LEN: usize = remedy_ui_core::token::TOKEN_LEN;

/// How long a new connection has to send its token.
const TOKEN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A window listening for agents. Dropping it stops listening and closes its connections.
#[derive(Debug)]
pub struct Listener {
    id: u64,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<(u64, TcpStream)>>>,
}

impl Listener {
    /// Where agents connect.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accepting thread so it sees the stop.
        let _ = TcpStream::connect(self.addr);
        for (_, c) in self.connections.lock().unwrap_or_else(std::sync::PoisonError::into_inner).drain(..) {
            let _ = c.shutdown(Shutdown::Both);
        }
        let id = self.id;
        let _ = LISTENING.try_with(|l| l.borrow_mut().remove(&id));
    }
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

impl Agents {
    /// Listens for agents on a loopback port (the system picks it; give the address to agents
    /// along with their tokens). Must be called on the window's thread, which must be running
    /// the reactive runtime's posted work (every bridge does).
    pub fn listen(&self) -> std::io::Result<Listener> {
        let waker = remedy_ui_core::reactive::waker().map_err(std::io::Error::other)?;
        let socket = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = socket.local_addr()?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        LISTENING.with(|l| l.borrow_mut().insert(id, self.clone()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Arc<Mutex<Vec<(u64, TcpStream)>>> = Arc::default();
        let (tokens, stop2, conns) = (self.tokens.clone(), stop.clone(), connections.clone());
        std::thread::Builder::new().name("remedy-agents".into()).spawn(move || {
            for stream in socket.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let (tokens, waker, stop) = (tokens.clone(), waker.clone(), stop2.clone());
                let conn = NEXT.fetch_add(1, Ordering::Relaxed);
                if let Ok(c) = stream.try_clone() {
                    conns.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((conn, c));
                }
                let open = conns.clone();
                let spawned = std::thread::Builder::new().name("remedy-agent".into()).spawn(move || {
                    // Strangers are turned away here, before reaching the window; one that
                    // sends no token does not hold its thread for good.
                    let _ = stream.set_read_timeout(Some(TOKEN_WAIT));
                    let token = read_token(&mut stream);
                    let _ = stream.set_read_timeout(None);
                    let known = token.as_ref().map(|t| {
                        tokens.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(t)
                    });
                    match known {
                        Some(true) => serve(stream, id, &waker, &stop),
                        Some(false) => {
                            let _ = writeln!(
                                stream,
                                "{}",
                                crate::automation::to_json(&Answer::Refused(Refused::NoAccess))
                            );
                        }
                        None => {}
                    }
                    // The kept handle would hold the socket open for as long as the window listens.
                    forget(&open, conn);
                });
                if spawned.is_err() {
                    forget(&conns, conn);
                }
            }
        })?;
        Ok(Listener { id, addr, stop, connections })
    }
}

/// Drops the handle kept for closing connection `conn`, once it has ended.
fn forget(connections: &Mutex<Vec<(u64, TcpStream)>>, conn: u64) {
    connections.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|(c, _)| *c != conn);
}

/// Answers one connection's requests until it closes.
fn serve(stream: TcpStream, id: u64, waker: &remedy_ui_core::reactive::Waker, stop: &AtomicBool) {
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
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let tx: Sender<String> = tx.clone();
        let posted = waker.post(move || answer(id, &line, tx));
        if posted.is_err() {
            break;
        }
    }
    drop(tx);
    let _ = writer.join();
}

/// Runs on the window's thread: answers `line` and sends the answer back.
fn answer(id: u64, line: &str, tx: Sender<String>) {
    let agents = LISTENING.with(|l| l.borrow().get(&id).cloned());
    let Some(agents) = agents else {
        let _ = tx.send(crate::automation::to_json(&Answer::Refused(Refused::NoAccess)));
        return;
    };
    let later = tx.clone();
    let now = agents.handle(line, move |a| {
        let _ = later.send(a);
    });
    let _ = tx.send(now);
}
