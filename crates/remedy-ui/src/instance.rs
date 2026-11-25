//! Single instance: the first launch of an app keeps running; later launches hand their
//! arguments to it and exit, or, with [`AppConfig::single_instance_reply`](crate::AppConfig),
//! wait for its reply code (and its exit) and exit with that.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::host::Waker;

/// A later launch: its arguments, and where its reply code goes when it waits for one.
pub(crate) struct Launch {
    pub(crate) args: Vec<String>,
    pub(crate) reply: Option<Sender<i32>>,
}

/// Launch arguments waiting for the runner, and the waker that tells it they arrived.
#[derive(Default)]
pub(crate) struct Inbox {
    pub(crate) launches: Vec<Launch>,
    pub(crate) waker: Option<Waker>,
}

pub(crate) type SharedInbox = Arc<Mutex<Inbox>>;

/// What a second launch waits for once it handed over its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReplyWait {
    pub(crate) reply: Duration,
    pub(crate) exit: Option<Duration>,
}

/// How claiming the instance ended.
// A browser page claims through the web bridge instead, so there only `run`'s match names
// these; the desktop bridges make them.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Claimed {
    /// This is the first instance: run.
    First,
    /// Another instance has the arguments; with a [`ReplyWait`], this is its reply code.
    HandedOver(Option<i32>),
}

#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown", not(feature = "web")), allow(dead_code))]
pub(crate) fn deliver(inbox: &SharedInbox, args: Vec<String>, reply: Option<Sender<i32>>) {
    let waker = {
        let mut i = inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        i.launches.push(Launch { args, reply });
        i.waker.clone()
    };
    if let Some(w) = waker {
        w();
    }
}

/// Claims the instance, or hands `args` to the one that has it (and, with `wait`, waits for
/// its reply).
pub(crate) fn claim(
    app: &str,
    args: &[String],
    wait: Option<ReplyWait>,
    inbox: SharedInbox,
) -> Result<Claimed, String> {
    #[cfg(windows)]
    {
        let wait =
            wait.map(|w| remedy_ui_platform::bridge_windows::ReplyWait { reply: w.reply, exit: w.exit });
        remedy_ui_platform::bridge_windows::claim_single_instance_replying(app, args, wait, move |a, r| {
            deliver(&inbox, a, r);
        })
        .map(|c| match c {
            remedy_ui_platform::bridge_windows::Claim::First => Claimed::First,
            remedy_ui_platform::bridge_windows::Claim::HandedOver(code) => Claimed::HandedOver(code),
        })
        .map_err(|e| e.to_string())
    }
    #[cfg(unix)]
    {
        unix::claim(app, args, wait, inbox)
    }
    // Browser pages claim it across tabs before starting (`run` does it, asynchronously).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let _ = (app, args, wait, inbox);
        Err("a browser page claims a single instance with the `web` feature".into())
    }
    #[cfg(not(any(windows, unix, all(target_arch = "wasm32", target_os = "unknown"))))]
    {
        let _ = (app, args, wait, inbox);
        Err("single instance is not available on this platform yet".into())
    }
}

/// The bytes of a launch that waits for a reply: a mark (a plain launch's arguments never
/// start with it), flags, the arguments' length (u32, little endian), the arguments. The
/// reply is the code as an i32, little endian; with [`FLAG_EXIT`] the first instance then
/// holds the connection until its process exits.
#[cfg(any(unix, test))]
mod frame {
    use super::ReplyWait;

    pub(super) const MARK: u8 = 0x01;
    pub(super) const FLAG_EXIT: u8 = 1;

    pub(super) fn encode(args: &[String], wait: Option<ReplyWait>) -> Vec<u8> {
        let payload = args.join("\0").into_bytes();
        let Some(w) = wait else { return payload };
        let mut out = vec![MARK, if w.exit.is_some() { FLAG_EXIT } else { 0 }];
        out.extend_from_slice(&u32::try_from(payload.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    pub(super) fn decode(bytes: &[u8]) -> Vec<String> {
        let text = String::from_utf8_lossy(bytes);
        if text.is_empty() { Vec::new() } else { text.split('\0').map(str::to_owned).collect() }
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::mpsc::channel;

    use super::frame::{FLAG_EXIT, MARK, decode, encode};
    use super::{Claimed, ReplyWait, SharedInbox, deliver};

    /// A per-user socket: in the runtime directory where there is one, else the per-user
    /// temporary directory with the user's name in the file name.
    fn socket(app: &str) -> PathBuf {
        let id: String = app.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir).join(format!("remedyui-{id}.sock")),
            None => {
                let user = std::env::var("USER").unwrap_or_default();
                std::env::temp_dir().join(format!("remedyui-{id}-{user}.sock"))
            }
        }
    }

    /// Hands `args` over; `Err` when nobody listens. With `wait`, the reply code (and, with
    /// `wait.exit`, once the first instance exited).
    fn send(
        path: &PathBuf,
        args: &[String],
        wait: Option<ReplyWait>,
    ) -> std::io::Result<Result<Option<i32>, String>> {
        let mut s = UnixStream::connect(path)?;
        s.write_all(&encode(args, wait))?;
        let Some(w) = wait else { return Ok(Ok(None)) };
        Ok(await_reply(s, w).map(Some))
    }

    fn await_reply(mut s: UnixStream, w: ReplyWait) -> Result<i32, String> {
        let timed_out = |e: &std::io::Error| {
            matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
        };
        let _ = s.set_read_timeout(Some(w.reply).filter(|d| !d.is_zero()));
        let mut code = [0u8; 4];
        match s.read_exact(&mut code) {
            Ok(()) => {}
            Err(e) if timed_out(&e) => {
                return Err(format!("the running instance did not reply within {:?}", w.reply));
            }
            Err(_) => return Err("the running instance ended without replying".into()),
        }
        if let Some(exit) = w.exit {
            let _ = s.set_read_timeout(Some(exit).filter(|d| !d.is_zero()));
            let mut rest = Vec::new();
            if let Err(e) = s.read_to_end(&mut rest)
                && timed_out(&e)
            {
                return Err(format!("the running instance did not exit within {exit:?}"));
            }
        }
        Ok(i32::from_le_bytes(code))
    }

    /// How long the first instance waits for a launch's bytes once it connected.
    const RECEIVE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    /// Reads one launch: its arguments and, when it waits for a reply, whether to hold the
    /// connection until this process exits.
    fn receive(s: &mut UnixStream) -> Option<(Vec<String>, Option<bool>)> {
        let mut first = [0u8; 1];
        match s.read(&mut first) {
            Ok(0) => return Some((Vec::new(), None)),
            Ok(_) => {}
            Err(_) => return None,
        }
        if first[0] != MARK {
            let mut rest = first.to_vec();
            s.read_to_end(&mut rest).ok()?;
            return Some((decode(&rest), None));
        }
        let mut head = [0u8; 5];
        s.read_exact(&mut head).ok()?;
        let [flags, len @ ..] = head;
        // Read as it arrives: the length is the sender's word, and nothing is set aside for it.
        let len = u64::from(u32::from_le_bytes(len));
        let mut payload = Vec::new();
        if s.by_ref().take(len).read_to_end(&mut payload).ok()? as u64 != len {
            return None;
        }
        Some((decode(&payload), Some(flags & FLAG_EXIT != 0)))
    }

    /// Hands `args` to the launch that is becoming the first instance, which may be a moment
    /// from listening.
    fn hand_to_winner(path: &PathBuf, args: &[String], wait: Option<ReplyWait>) -> Result<Claimed, String> {
        let mut last = String::new();
        for _ in 0..25 {
            match send(path, args, wait) {
                Ok(sent) => return sent.map(Claimed::HandedOver),
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err(format!("the running instance does not listen: {last}"))
    }

    pub(super) fn claim(
        app: &str,
        args: &[String],
        wait: Option<ReplyWait>,
        inbox: SharedInbox,
    ) -> Result<Claimed, String> {
        let path = socket(app);
        if let Ok(sent) = send(&path, args, wait) {
            return sent.map(Claimed::HandedOver);
        }
        // Nobody listens: a stale socket from a crashed run, or none at all. Only the launch
        // holding the lock may clear it and bind, so two first launches at once cannot each
        // remove the other's socket and both run. The system lets go of the lock when the
        // process ends, however it ends.
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("lock"))
            .ok();
        // Where files cannot be locked the claim goes on without one, as bind decides.
        let taken = lock.as_ref().map(std::fs::File::try_lock);
        if matches!(taken, Some(Err(std::fs::TryLockError::WouldBlock))) {
            return hand_to_winner(&path, args, wait);
        }
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            // Another first launch won the race; it is the instance now.
            Err(_) => return hand_to_winner(&path, args, wait),
        };
        std::thread::Builder::new()
            .name("remedy-ui single instance".into())
            .spawn(move || {
                // Held for as long as the instance serves.
                let _lock = lock;
                for stream in listener.incoming().flatten() {
                    let mut s = stream;
                    // A launch that connects and never sends must not keep the others out.
                    let _ = s.set_read_timeout(Some(RECEIVE_WAIT));
                    match receive(&mut s) {
                        Some((launch, None)) => deliver(&inbox, launch, None),
                        Some((launch, Some(hold))) => {
                            let (tx, rx) = channel::<i32>();
                            deliver(&inbox, launch, Some(tx));
                            // The reply may take as long as the app's work: the socket keeps serving.
                            let _ = std::thread::Builder::new()
                                .name("remedy-ui single instance reply".into())
                                .spawn(move || {
                                    let Ok(code) = rx.recv() else { return };
                                    if s.write_all(&code.to_le_bytes()).is_ok() && hold {
                                        // Never closed here: the system closes it as this process
                                        // exits (std opens it close-on-exec, so no child keeps it).
                                        std::mem::forget(s);
                                    }
                                });
                        }
                        None => {}
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Claimed::First)
    }
}

/// What the app answers a later launch with ([`App::on_second_instance`](crate::App)): a code
/// now, or one it sends later from any thread ([`InstanceReply::later`]). A handler that
/// returns nothing answers 0.
#[derive(Debug)]
pub struct InstanceReply(ReplyKind);

#[derive(Debug)]
enum ReplyKind {
    Now(i32),
    Later(Arc<Mutex<LaterSlot>>),
}

#[derive(Debug, Default)]
struct LaterSlot {
    code: Option<i32>,
    to: Option<Sender<i32>>,
    /// The [`ReplyLater`] went away without a code: none will come.
    abandoned: bool,
}

impl InstanceReply {
    /// Answers `code` now.
    pub fn code(code: i32) -> Self {
        InstanceReply(ReplyKind::Now(code))
    }

    /// Answers later: return the first half from the handler and call [`ReplyLater::send`]
    /// on the second once the work the launch asked for is done (stopping a service, say).
    /// Dropping it unsent sends no code; the waiting launch then fails as for a running
    /// instance that never replied.
    pub fn later() -> (InstanceReply, ReplyLater) {
        let slot = Arc::new(Mutex::new(LaterSlot::default()));
        (InstanceReply(ReplyKind::Later(slot.clone())), ReplyLater(Some(slot)))
    }

    /// Sends the code (now or once it is known) to `to`, a launch waiting for one.
    pub(crate) fn deliver(self, to: Sender<i32>) {
        match self.0 {
            ReplyKind::Now(code) => {
                let _ = to.send(code);
            }
            ReplyKind::Later(slot) => {
                let mut s = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match s.code.take() {
                    Some(code) => {
                        let _ = to.send(code);
                    }
                    None if !s.abandoned => s.to = Some(to),
                    None => {}
                }
            }
        }
    }
}

impl From<()> for InstanceReply {
    fn from((): ()) -> Self {
        InstanceReply::code(0)
    }
}

impl From<i32> for InstanceReply {
    fn from(code: i32) -> Self {
        InstanceReply::code(code)
    }
}

/// The code an [`InstanceReply::later`] answers with, sent from any thread.
#[derive(Debug)]
pub struct ReplyLater(Option<Arc<Mutex<LaterSlot>>>);

impl ReplyLater {
    /// Sends `code` to the launch waiting for it (a launch that does not wait ignores it).
    pub fn send(mut self, code: i32) {
        if let Some(slot) = self.0.take() {
            let mut s = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match s.to.take() {
                Some(to) => {
                    let _ = to.send(code);
                }
                None => s.code = Some(code),
            }
        }
    }
}

impl Drop for ReplyLater {
    fn drop(&mut self) {
        if let Some(slot) = self.0.take() {
            let mut s = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            s.abandoned = true;
            s.to = None;
        }
    }
}

#[cfg(test)]
impl ReplyKind {
    fn code_now(&self) -> Option<i32> {
        match self {
            ReplyKind::Now(c) => Some(*c),
            ReplyKind::Later(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_reaches_the_launch_whichever_comes_first() {
        let (tx, rx) = std::sync::mpsc::channel();
        InstanceReply::code(3).deliver(tx);
        assert_eq!(rx.try_recv(), Ok(3));
        // Sent before the launch's channel is wired, and after.
        let (reply, later) = InstanceReply::later();
        later.send(4);
        let (tx, rx) = std::sync::mpsc::channel();
        reply.deliver(tx);
        assert_eq!(rx.try_recv(), Ok(4));
        let (reply, later) = InstanceReply::later();
        let (tx, rx) = std::sync::mpsc::channel();
        reply.deliver(tx);
        assert!(rx.try_recv().is_err(), "nothing yet");
        std::thread::spawn(move || later.send(5)).join().ok();
        assert_eq!(rx.try_recv(), Ok(5));
        // Dropped unsent: the launch's channel closes without a code.
        let (reply, later) = InstanceReply::later();
        let (tx, rx) = std::sync::mpsc::channel();
        reply.deliver(tx);
        drop(later);
        assert_eq!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected));
        assert_eq!(InstanceReply::from(()).0.code_now(), Some(0));
    }

    #[test]
    fn framing_keeps_the_arguments() {
        let wait = ReplyWait { reply: Duration::from_secs(1), exit: None };
        let bytes = frame::encode(&["--stop".into(), "a b".into()], Some(wait));
        assert_eq!(bytes.first(), Some(&frame::MARK));
        assert_eq!(bytes.get(1), Some(&0));
        assert_eq!(
            frame::decode(bytes.get(6..).unwrap_or_default()),
            vec!["--stop".to_owned(), "a b".to_owned()]
        );
        assert_eq!(frame::decode(&frame::encode(&[], None)), Vec::<String>::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_launch_cannot_take_the_place_of_a_running_instance() {
        let name = format!("remedy-ui-lock-test-{}", std::process::id());
        assert_eq!(claim(&name, &[], None, SharedInbox::default()), Ok(Claimed::First));
        // Its socket is gone (cleared by a launch racing it, or by hand) while it runs.
        let id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty());
        let path = match dir {
            Some(dir) => std::path::PathBuf::from(dir).join(format!("remedyui-{id}.sock")),
            None => {
                let user = std::env::var("USER").unwrap_or_default();
                std::env::temp_dir().join(format!("remedyui-{id}-{user}.sock"))
            }
        };
        assert!(std::fs::remove_file(&path).is_ok(), "the first instance bound {}", path.display());
        let second = claim(&name, &[], None, SharedInbox::default());
        assert!(second.is_err(), "a second first instance: {second:?}");
        let _ = std::fs::remove_file(path.with_extension("lock"));
    }

    /// The first instance for [`a_second_launch_gets_the_reply_and_waits_for_the_exit`]: run
    /// as a child process. It answers 7 to `--stop`, waits a moment, and exits.
    #[test]
    #[ignore = "run as a child process by the test below"]
    fn first_instance_child() {
        let Ok(name) = std::env::var("REMEDY_UI_INSTANCE_TEST") else { return };
        let inbox = SharedInbox::default();
        assert_eq!(claim(&name, &[], None, inbox.clone()), Ok(Claimed::First));
        println!("ready");
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            let launch = inbox.lock().ok().and_then(|mut i| i.launches.pop());
            if let Some(l) = launch {
                if let Some(to) = l.reply {
                    let code = if l.args == ["--stop"] { 7 } else { 1 };
                    let _ = to.send(code);
                }
                std::thread::sleep(Duration::from_millis(400));
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_second_launch_gets_the_reply_and_waits_for_the_exit() {
        use std::io::BufRead;
        let name = format!("remedy-ui-reply-test-{}", std::process::id());
        let Ok(exe) = std::env::current_exe() else { return };
        let Ok(mut child) = std::process::Command::new(exe)
            .args(["--ignored", "--exact", "instance::tests::first_instance_child", "--nocapture"])
            .env("REMEDY_UI_INSTANCE_TEST", &name)
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            unreachable!("the child starts")
        };
        let Some(out) = child.stdout.take() else { unreachable!("piped") };
        let mut lines = std::io::BufReader::new(out).lines();
        assert!(lines.any(|l| l.is_ok_and(|l| l.contains("ready"))), "the child claimed first");
        let wait = ReplyWait { reply: Duration::from_secs(10), exit: Some(Duration::from_secs(10)) };
        let got = claim(&name, &["--stop".into()], Some(wait), SharedInbox::default());
        assert_eq!(got, Ok(Claimed::HandedOver(Some(7))));
        // The claim returned only once the first instance's process was gone.
        let mut exited = false;
        for _ in 0..50 {
            if child.try_wait().ok().flatten().is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(exited, "the first instance exited before the reply wait ended");
        let _ = child.kill();
    }
}
