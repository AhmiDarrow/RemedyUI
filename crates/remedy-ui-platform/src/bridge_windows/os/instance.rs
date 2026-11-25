//! Single instance: a session-local named mutex says whether the app already runs, and a
//! named pipe carries a second launch's arguments to it (and, when the second launch waits
//! for one, the first's reply back).

use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use remedy_ui_core::host::Unsupported;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE,
    GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe, WaitNamedPipeW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};

use super::wide;

/// How long a second launch keeps trying to reach the first (which may be starting up),
/// unless the app says otherwise ([`set_instance_wait`]).
pub const INSTANCE_WAIT: Duration = Duration::from_secs(5);

static WAIT: std::sync::Mutex<Duration> = std::sync::Mutex::new(INSTANCE_WAIT);

/// Sets how long a second launch keeps trying to hand its arguments to the first before it
/// reports that the running instance did not answer (default [`INSTANCE_WAIT`], 5 s).
/// `remedy_ui::run` sets it from `AppConfig::single_instance_wait`.
pub fn set_instance_wait(wait: Duration) {
    *WAIT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = wait;
}

fn names(app_id: &str) -> (String, String) {
    let id: String = app_id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let mut session = 0u32;
    // SAFETY: a valid out-parameter for this process's session.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &raw mut session) };
    (format!("Local\\RemedyUI.{id}"), format!("\\\\.\\pipe\\RemedyUI.{id}.{session}"))
}

/// What a second launch waits for after handing over its arguments
/// ([`claim_single_instance_replying`](crate::bridge_windows::claim_single_instance_replying)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyWait {
    /// How long to wait for the running instance's reply code.
    pub reply: Duration,
    /// Also wait, this long, for the running instance's process to exit after it replied.
    pub exit: Option<Duration>,
}

/// How a claim ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// This is the first instance: run.
    First,
    /// Another instance runs and has the arguments; with a [`ReplyWait`], its reply code.
    HandedOver(Option<i32>),
}

/// Starts a launch that waits for a reply (an old-style launch's arguments never start with
/// it): the mark, the flags, the arguments' length (u32, little endian), the arguments.
const REPLY_MARK: u8 = 0x01;
/// The flag asking the first instance to hold the connection until its process exits.
const FLAG_EXIT: u8 = 1;

/// How long the first instance waits for a connected launch's bytes: one that connects and
/// never sends, or never closes, must not keep the others out.
const RECEIVE_WAIT: Duration = Duration::from_secs(5);
/// The most bytes one launch may carry (a Windows command line holds 32 767 characters).
const MAX_LAUNCH: usize = 1024 * 1024;

/// Hands the running instance's reply code to a launch that waits for one.
pub type Replier = Sender<i32>;

/// Claims the single instance for `app_id` in this user session. Returns `true` when this is
/// the first instance (`on_launch` then receives each later launch's arguments, on a
/// background thread), or `false` after handing `args` to the running instance.
pub(crate) fn claim(
    app_id: &str,
    args: &[String],
    on_launch: impl Fn(Vec<String>) + Send + 'static,
) -> Result<bool, Unsupported> {
    claim_replying(app_id, args, None, move |a, _| on_launch(a)).map(|c| c == Claim::First)
}

/// [`claim`], where a later launch may wait for a reply: `on_launch` then gets a
/// [`Replier`], and the code sent on it goes back to that launch (dropping it sends none).
/// With `wait`, this launch, when it is not the first, waits for the running instance's
/// reply (and, with [`ReplyWait::exit`], for its process to exit).
pub(crate) fn claim_replying(
    app_id: &str,
    args: &[String],
    wait: Option<ReplyWait>,
    on_launch: impl Fn(Vec<String>, Option<Replier>) + Send + 'static,
) -> Result<Claim, Unsupported> {
    let (mutex_name, pipe_name) = names(app_id);
    let m = wide(&mutex_name);
    // SAFETY: creating or opening a named mutex; the result and last error are checked.
    let (mutex, existed) = unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, m.as_ptr());
        (h, GetLastError() == ERROR_ALREADY_EXISTS)
    };
    if mutex.is_null() {
        return Err(Unsupported::new("single instance", std::io::Error::last_os_error().to_string()));
    }
    if existed {
        // SAFETY: our handle to the other instance's mutex is not needed.
        unsafe { CloseHandle(mutex) };
        return send(&pipe_name, args, wait).map(Claim::HandedOver);
    }
    // The mutex handle stays open for the life of the process: it is the claim.
    let pipe = wide(&pipe_name);
    std::thread::Builder::new()
        .name("remedy-ui single instance".into())
        .spawn(move || {
            loop {
                match receive(&pipe, RECEIVE_WAIT) {
                    Some((launch, None)) => on_launch(launch, None),
                    Some((launch, Some((conn, hold)))) => {
                        let (tx, rx) = channel();
                        on_launch(launch, Some(tx));
                        // The reply may take as long as the app's work: the pipe keeps serving.
                        let _ = std::thread::Builder::new()
                            .name("remedy-ui single instance reply".into())
                            .spawn(move || conn.reply(rx.recv().ok(), hold));
                    }
                    None => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        })
        .map_err(|e| Unsupported::new("single instance", e.to_string()))?;
    Ok(Claim::First)
}

/// A connected pipe end: the first instance's to a launch waiting for its reply, or that
/// launch's own.
struct Conn(HANDLE);

// SAFETY: a pipe handle may be used from any thread; one thread at a time holds this one.
unsafe impl Send for Conn {}

impl Conn {
    /// Writes `code` (`None`: the app sent no reply) and, with `hold`, keeps the connection
    /// open until this process exits: that is how the waiting launch learns of the exit.
    fn reply(self, code: Option<i32>, hold: bool) {
        // SAFETY: writing four bytes to our connected handle and waiting until they are read.
        unsafe {
            if let Some(code) = code {
                let bytes = code.to_le_bytes();
                let mut n = 0u32;
                WriteFile(self.0, bytes.as_ptr(), 4, &raw mut n, std::ptr::null_mut());
                FlushFileBuffers(self.0);
                if hold {
                    // Never closed here: the system closes it as this process exits. The
                    // handle is not inheritable, so no child process keeps it open.
                    return;
                }
            }
            DisconnectNamedPipe(self.0);
            CloseHandle(self.0);
        }
    }
}

/// One `ReadFile` into `bytes`; false at the end or on an error.
///
/// # Safety
/// `h` is an open pipe handle.
unsafe fn read_some(h: HANDLE, bytes: &mut Vec<u8>) -> bool {
    let mut buf = [0u8; 4096];
    let mut n = 0u32;
    // SAFETY: reading into our buffer from the caller's open handle.
    let ok =
        unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &raw mut n, std::ptr::null_mut()) } != 0;
    if !ok || n == 0 {
        return false;
    }
    bytes.extend_from_slice(buf.get(..n as usize).unwrap_or_default());
    true
}

/// [`read_some`] that waits for bytes only until `deadline`, and takes no more than
/// [`MAX_LAUNCH`] in all; false also when either ran out.
///
/// # Safety
/// `h` is an open pipe handle.
unsafe fn read_by(h: HANDLE, bytes: &mut Vec<u8>, deadline: Instant) -> bool {
    loop {
        let mut waiting = 0u32;
        // SAFETY: asking how many bytes wait in the caller's open pipe; nothing is read.
        let open = unsafe {
            PeekNamedPipe(
                h,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &raw mut waiting,
                std::ptr::null_mut(),
            )
        } != 0;
        if !open || bytes.len() > MAX_LAUNCH {
            return false;
        }
        if waiting > 0 {
            // SAFETY: the caller's open handle; bytes wait, so this read returns at once.
            return unsafe { read_some(h, bytes) };
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A launch that waits for a reply, once all of it arrived (`None` while more is to come):
/// its arguments, and whether it waits for the exit too.
fn framed(bytes: &[u8]) -> Option<(Vec<String>, bool)> {
    let flags = *bytes.get(1)?;
    let len = u32::from_le_bytes(bytes.get(2..6)?.try_into().ok()?) as usize;
    let payload = bytes.get(6..6usize.checked_add(len)?)?;
    Some((decode(payload), flags & FLAG_EXIT != 0))
}

/// A launch's arguments and, when it waits for a reply, its connection and whether to hold
/// that until this process exits.
type Received = (Vec<String>, Option<(Conn, bool)>);

/// Waits for one launch on the pipe; once it connected, its bytes have `wait` to arrive.
fn receive(pipe: &[u16], wait: Duration) -> Option<Received> {
    // SAFETY: one duplex, local-only pipe instance; connected, read, then closed or handed on.
    unsafe {
        let h: HANDLE = CreateNamedPipeW(
            pipe.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            // Launches waiting for the exit hold theirs, so later ones need instances of their own.
            PIPE_UNLIMITED_INSTANCES,
            4096,
            64 * 1024,
            0,
            std::ptr::null(),
        );
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let connected =
            ConnectNamedPipe(h, std::ptr::null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED;
        let deadline = Instant::now().checked_add(wait);
        let read = |bytes: &mut Vec<u8>| deadline.is_some_and(|d| read_by(h, bytes, d));
        let mut bytes = Vec::new();
        if connected && read(&mut bytes) && bytes.first() == Some(&REPLY_MARK) {
            loop {
                if let Some((launch, hold)) = framed(&bytes) {
                    return Some((launch, Some((Conn(h), hold))));
                }
                if !read(&mut bytes) {
                    break;
                }
            }
            // Cut short: nothing to deliver.
            DisconnectNamedPipe(h);
            CloseHandle(h);
            return None;
        }
        while connected && read(&mut bytes) {}
        // Whole only when the launch closed its end: one cut off by the wait or the limit
        // would be delivered with arguments missing.
        let mut waiting = 0u32;
        let whole = connected
            && bytes.len() <= MAX_LAUNCH
            && PeekNamedPipe(
                h,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &raw mut waiting,
                std::ptr::null_mut(),
            ) == 0;
        DisconnectNamedPipe(h);
        CloseHandle(h);
        whole.then(|| (decode(&bytes), None))
    }
}

fn decode(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() { Vec::new() } else { text.split('\0').map(str::to_owned).collect() }
}

/// A launch's bytes: its arguments, framed when it waits for a reply.
fn encode(args: &[String], wait: Option<ReplyWait>) -> Vec<u8> {
    let payload = args.join("\0").into_bytes();
    let Some(w) = wait else { return payload };
    let mut out = vec![REPLY_MARK, if w.exit.is_some() { FLAG_EXIT } else { 0 }];
    out.extend_from_slice(&u32::try_from(payload.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// What the reading thread heard from the running instance.
enum Heard {
    Code(i32),
    /// The connection closed: the instance exited, or ended it without a reply.
    Closed,
}

fn send(pipe_name: &str, args: &[String], wait: Option<ReplyWait>) -> Result<Option<i32>, Unsupported> {
    let pipe = wide(pipe_name);
    let payload = encode(args, wait);
    let start = Instant::now();
    let fail = |why: String| Unsupported::new("single instance", why);
    if payload.len() > MAX_LAUNCH {
        return Err(fail(format!("the arguments are longer than {MAX_LAUNCH} bytes")));
    }
    loop {
        // SAFETY: opening the pipe; the handle is checked, and closed or handed on.
        let h = unsafe {
            CreateFileW(
                pipe.as_ptr(),
                if wait.is_some() { GENERIC_READ | GENERIC_WRITE } else { GENERIC_WRITE },
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            let mut written = 0u32;
            // SAFETY: writing our bytes to the open handle.
            let ok = unsafe {
                WriteFile(h, payload.as_ptr(), payload.len() as u32, &raw mut written, std::ptr::null_mut())
                    != 0
            };
            let e = std::io::Error::last_os_error().to_string();
            let Some(w) = wait.filter(|_| ok) else {
                // SAFETY: closing our handle once.
                unsafe { CloseHandle(h) };
                return if ok { Ok(None) } else { Err(fail(e)) };
            };
            return await_reply(Conn(h), w).map(Some).map_err(fail);
        }
        // SAFETY: the failed open's error, read before any other call can replace it.
        let busy = unsafe { GetLastError() } == ERROR_PIPE_BUSY;
        if start.elapsed() > *WAIT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            return Err(fail("the running instance did not answer".into()));
        }
        if busy {
            // SAFETY: waiting for a free pipe instance; the name is a NUL-terminated string.
            unsafe { WaitNamedPipeW(pipe.as_ptr(), 1000) };
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Reads the reply code, then (with [`ReplyWait::exit`]) waits for the connection to close.
/// The reads block, so a thread does them while this waits on it with the timeouts.
fn await_reply(conn: Conn, w: ReplyWait) -> Result<i32, String> {
    let (tx, rx) = channel();
    let hold = w.exit.is_some();
    std::thread::Builder::new()
        .name("remedy-ui single instance wait".into())
        .spawn(move || {
            let conn = conn;
            let mut bytes = Vec::new();
            // SAFETY: reading from our open handle, closed once at the end.
            unsafe {
                while bytes.len() < 4 && read_some(conn.0, &mut bytes) {}
                if let Some(code) = bytes.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()) {
                    let _ = tx.send(Heard::Code(i32::from_le_bytes(code)));
                    while hold && read_some(conn.0, &mut bytes) {}
                }
                let _ = tx.send(Heard::Closed);
                CloseHandle(conn.0);
            }
        })
        .map_err(|e| e.to_string())?;
    let code = match rx.recv_timeout(w.reply) {
        Ok(Heard::Code(c)) => c,
        Ok(Heard::Closed) => return Err("the running instance ended without replying".into()),
        Err(_) => return Err(format!("the running instance did not reply within {:?}", w.reply)),
    };
    if let Some(exit) = w.exit
        && rx.recv_timeout(exit).is_err()
    {
        return Err(format!("the running instance did not exit within {exit:?}"));
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_waits_as_long_as_the_app_says() {
        set_instance_wait(Duration::from_millis(100));
        let start = Instant::now();
        let sent = send(r"\\.\pipe\remedy-ui-test-no-such-instance", &["a".into()], None);
        set_instance_wait(INSTANCE_WAIT);
        assert!(sent.is_err());
        assert!(start.elapsed() < INSTANCE_WAIT, "gave up after the app's wait: {:?}", start.elapsed());
    }

    #[test]
    fn a_launch_that_connects_and_never_sends_does_not_hold_the_pipe() {
        let name = format!(r"\\.\pipe\remedy-ui-test-silent-{}", std::process::id());
        let pipe = wide(&name);
        let server = pipe.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(receive(&server, Duration::from_millis(50)).is_none());
        });
        let mut client = INVALID_HANDLE_VALUE;
        for _ in 0..200 {
            // SAFETY: opening the test's own pipe; the handle is checked and closed below.
            client = unsafe {
                CreateFileW(
                    pipe.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if client != INVALID_HANDLE_VALUE {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(client, INVALID_HANDLE_VALUE, "the pipe was created");
        let gave_up = rx.recv_timeout(Duration::from_secs(3));
        // SAFETY: closing the test's handle once.
        unsafe { CloseHandle(client) };
        assert_eq!(gave_up, Ok(true), "the silent launch is dropped after the wait");
    }

    #[test]
    fn a_framed_launch_is_read_whole_and_only_whole() {
        let wait = ReplyWait { reply: Duration::from_secs(1), exit: Some(Duration::from_secs(1)) };
        let bytes = encode(&["--stop".into(), "--quiet".into()], Some(wait));
        assert_eq!(bytes.first(), Some(&REPLY_MARK));
        assert_eq!(framed(&bytes), Some((vec!["--stop".to_owned(), "--quiet".to_owned()], true)));
        assert_eq!(framed(bytes.get(..bytes.len() - 1).unwrap_or_default()), None, "more is to come");
        let plain = encode(&["--show".into()], None);
        assert_eq!(decode(&plain), vec!["--show".to_owned()]);
        let no_exit = encode(&[], Some(ReplyWait { exit: None, ..wait }));
        assert_eq!(framed(&no_exit), Some((Vec::new(), false)));
    }
}
