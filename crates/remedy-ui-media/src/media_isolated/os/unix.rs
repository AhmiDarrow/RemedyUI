//! Linux and macOS: the helper is the app started again with a Unix socket as its standard
//! input; shared memory regions are file descriptors passed over it (`SCM_RIGHTS`), mapped
//! for one request. The helper shuts itself in (seccomp-bpf and Landlock on Linux, Seatbelt
//! on macOS) before it reads anything. On Android the helper is an isolated service
//! process instead (`android.rs` starts it), over the same socket and regions.
//!
//! Invariants: every descriptor is an `OwnedFd` from creation, so it is closed once. A
//! mapping lives exactly as long as the `Shm` or `Mapped` holding it, and its length is the
//! region's, which cannot shrink under it (Linux seals the memfd; a macOS shared memory
//! object cannot be resized once sized). The app reads what the helper wrote only by copying.
//! A helper's process id is signalled only while it is unreaped (`Child` reaps it last).

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
#[cfg(not(target_os = "android"))]
use std::os::unix::process::ExitStatusExt;
#[cfg(not(target_os = "android"))]
use std::path::Path;
#[cfg(not(target_os = "android"))]
use std::process::{ChildStderr, Command, Stdio};
use std::ptr::null_mut;
#[cfg(not(target_os = "android"))]
use std::time::{Duration, Instant};

#[cfg(target_os = "android")]
use super::android as platform;
use super::frame::{MAX_REGIONS, Region, header, lead, regions, regions_len};
#[cfg(target_os = "linux")]
use super::linux as platform;
#[cfg(target_os = "macos")]
use super::macos as platform;
use crate::media_isolated::Layer;

pub(crate) use platform::LIMITS_MEMORY;
#[cfg(target_os = "android")]
pub(crate) use platform::{Child, Killer};

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

/// Takes ownership of a descriptor a call just returned.
pub(super) fn owned_fd(fd: libc::c_int) -> io::Result<OwnedFd> {
    check(fd)?;
    // SAFETY: `fd` is a valid descriptor the caller just received and nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

// ---------- shared memory ----------

/// A shared memory object, mapped whole into the app.
pub(crate) struct Shm {
    fd: OwnedFd,
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the mapping is plain memory owned by this value; methods copy bytes in and out.
unsafe impl Send for Shm {}
// SAFETY: as above; `&self` methods only read.
unsafe impl Sync for Shm {}

fn map(fd: RawFd, len: usize, writable: bool) -> io::Result<*mut u8> {
    let prot = if writable { libc::PROT_READ | libc::PROT_WRITE } else { libc::PROT_READ };
    // SAFETY: maps `len` bytes of a shared memory object at least that long, at an address
    // the system picks; unmapped by the owner's Drop.
    let p = unsafe { libc::mmap(null_mut(), len, prot, libc::MAP_SHARED, fd, 0) };
    if p == libc::MAP_FAILED { Err(io::Error::last_os_error()) } else { Ok(p.cast()) }
}

impl Shm {
    /// A region of `len` bytes (at least one, so it can be mapped).
    pub(crate) fn new(len: usize) -> io::Result<Shm> {
        let fd = platform::shared_fd(len.max(1))?;
        let ptr = map(fd.as_raw_fd(), len.max(1), true)?;
        Ok(Shm { fd, ptr, len })
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if data.len() > self.len {
            return Err(io::Error::other("data larger than the region"));
        }
        // SAFETY: the mapping holds `len` bytes and `data` fits; the helper has not been
        // handed this region yet.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr, data.len()) };
        Ok(())
    }

    pub(crate) fn read(&self, len: usize) -> io::Result<Vec<u8>> {
        if len > self.len {
            return Err(io::Error::other("read past the region"));
        }
        let mut out: Vec<u8> = Vec::new();
        out.try_reserve_exact(len).map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        // SAFETY: the mapping holds at least `len` bytes and cannot shrink; `out` has room.
        // The helper may still be writing; bytes are copied, never referenced.
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr, out.as_mut_ptr(), len);
            out.set_len(len);
        }
        Ok(out)
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        // SAFETY: mapped in `new` with this length; unmapped once.
        unsafe { libc::munmap(self.ptr.cast(), self.len.max(1)) };
    }
}

// ---------- the channel ----------

pub(super) fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let kind = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let kind = libc::SOCK_STREAM;
    // SAFETY: `fds` has room for the two descriptors.
    check(unsafe { libc::socketpair(libc::AF_UNIX, kind, 0, fds.as_mut_ptr()) })?;
    let [a, b] = fds;
    let (a, b) = (owned_fd(a)?, owned_fd(b)?);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    for fd in [&a, &b] {
        // SAFETY: plain descriptor flag and socket option changes on our own sockets.
        unsafe {
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            let on: libc::c_int = 1;
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                (&raw const on).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
    Ok((a, b))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const SEND_FLAGS: libc::c_int = libc::MSG_NOSIGNAL;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const SEND_FLAGS: libc::c_int = 0;

/// Sends `bytes` with `fds` attached to the first of them.
fn send_with_fds(sock: &UnixStream, bytes: &[u8], fds: &[RawFd]) -> io::Result<()> {
    if fds.is_empty() {
        return (&*sock).write_all(bytes);
    }
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE(size_of_val(fds) as u32) } as usize;
    let mut control = vec![0u64; space.div_ceil(8)];
    let mut iov = libc::iovec { iov_base: bytes.as_ptr().cast_mut().cast(), iov_len: bytes.len() };
    // SAFETY: an all-zero msghdr is valid; the fields used are set below.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = space as _;
    // SAFETY: `msg` describes `control`, which has room for one header and the descriptors
    // (CMSG_SPACE); CMSG_FIRSTHDR/CMSG_DATA point inside it.
    let sent = unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(size_of_val(fds) as u32) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(c).cast::<RawFd>(), fds.len());
        libc::sendmsg(sock.as_raw_fd(), &msg, SEND_FLAGS)
    };
    let sent = usize::try_from(sent).map_err(|_| io::Error::last_os_error())?;
    (&*sock).write_all(bytes.get(sent..).unwrap_or_default())
}

/// Reads a frame's three lead bytes and the descriptors that came with them.
fn recv_lead(sock: &UnixStream) -> io::Result<([u8; 3], Vec<OwnedFd>)> {
    let mut lead_bytes = [0u8; 3];
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE((MAX_REGIONS * size_of::<RawFd>()) as u32) } as usize;
    let mut control = vec![0u64; space.div_ceil(8)];
    let mut iov = libc::iovec { iov_base: lead_bytes.as_mut_ptr().cast(), iov_len: lead_bytes.len() };
    // SAFETY: an all-zero msghdr is valid; the fields used are set below.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = space as _;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let flags = 0;
    let got = loop {
        // SAFETY: `msg` points at `lead_bytes` and `control`, both alive for the call.
        let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, flags) };
        if n >= 0 {
            break n as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut fds = Vec::new();
    // SAFETY: walks the control headers the kernel filled in, within `msg_controllen`;
    // every SCM_RIGHTS descriptor is taken into an OwnedFd at once, so none leaks.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(c).cast::<RawFd>();
                let n = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / size_of::<RawFd>();
                for i in 0..n {
                    fds.push(OwnedFd::from_raw_fd(std::ptr::read_unaligned(data.add(i))));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::other("descriptors were cut off"));
    }
    if got == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    (&*sock).read_exact(lead_bytes.get_mut(got..).unwrap_or_default())?;
    Ok((lead_bytes, fds))
}

/// Sends one request frame with `shared` regions' descriptors beside it.
pub(super) fn send_frame(sock: &UnixStream, payload: &[u8], shared: &[(&Shm, bool)]) -> io::Result<()> {
    let described: Vec<Region> = shared
        .iter()
        .map(|(s, writable)| Region { handle: 0, len: s.len() as u64, writable: *writable })
        .collect();
    let fds: Vec<RawFd> = shared.iter().map(|(s, _)| s.fd.as_raw_fd()).collect();
    let mut frame = header(payload.len(), &described)?;
    frame.extend_from_slice(payload);
    send_with_fds(sock, &frame, &fds)
}

/// Reads one reply frame (which carries no regions).
pub(super) fn recv_frame(sock: &UnixStream) -> io::Result<Vec<u8>> {
    let mut lead_bytes = [0u8; 3];
    (&*sock).read_exact(&mut lead_bytes)?;
    let (len, n) = lead(lead_bytes)?;
    if n != 0 {
        return Err(io::Error::other("the helper sent regions"));
    }
    let mut payload = vec![0u8; len];
    (&*sock).read_exact(&mut payload)?;
    Ok(payload)
}

// ---------- the helper, seen from the app ----------

/// Stops a helper from any thread (only while it is unreaped; see the watchdog).
#[cfg(not(target_os = "android"))]
#[derive(Clone)]
pub(crate) struct Killer(libc::pid_t);

#[cfg(not(target_os = "android"))]
impl Killer {
    pub(crate) fn kill(&self) {
        // SAFETY: signals our own unreaped child.
        unsafe { libc::kill(self.0, libc::SIGKILL) };
    }

    pub(crate) fn memory(&self) -> Option<u64> {
        platform::memory_of(self.0)
    }

    /// The processor time it has used, user and system.
    pub(crate) fn cpu_time(&self) -> Option<Duration> {
        platform::cpu_time_of(self.0)
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) struct Child {
    proc: std::process::Child,
    sock: UnixStream,
    err: ChildStderr,
    err_tail: Vec<u8>,
    reaped: bool,
}

#[cfg(not(target_os = "android"))]
impl Child {
    pub(crate) fn spawn(exe: &Path) -> io::Result<Child> {
        let (host, child) = socketpair()?;
        let mut proc = Command::new(exe)
            .arg(crate::media_isolated::HELPER_ARG)
            .stdin(Stdio::from(child))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let err = proc.stderr.take().ok_or_else(|| io::Error::other("no stderr pipe"))?;
        // SAFETY: makes our end of the stderr pipe non-blocking, so draining never waits.
        unsafe {
            let fl = libc::fcntl(err.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(err.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        Ok(Child { proc, sock: UnixStream::from(host), err, err_tail: Vec::new(), reaped: false })
    }

    pub(crate) fn send(&mut self, payload: &[u8], shared: &[(&Shm, bool)]) -> io::Result<()> {
        send_frame(&self.sock, payload, shared)
    }

    pub(crate) fn recv(&mut self) -> io::Result<Vec<u8>> {
        recv_frame(&self.sock)
    }

    /// Nothing to take back: the helper closes the descriptors it was handed.
    pub(crate) fn end_request(&mut self) {}

    pub(crate) fn killer(&self) -> Killer {
        Killer(self.proc.id() as libc::pid_t)
    }

    pub(crate) fn memory_in_use(&self) -> Option<u64> {
        self.killer().memory()
    }

    pub(crate) fn limit_memory(&self, bytes: u64) -> io::Result<()> {
        platform::limit_memory(self.proc.id() as libc::pid_t, bytes)
    }

    fn keep_err(&mut self, b: &[u8]) {
        self.err_tail.extend_from_slice(b);
        let excess = self.err_tail.len().saturating_sub(crate::media_isolated::protocol::MAX_TEXT);
        self.err_tail.drain(..excess);
    }

    pub(crate) fn drain_stderr(&mut self) {
        let mut buf = [0u8; 4096];
        while let Ok(n) = self.err.read(&mut buf) {
            if n == 0 {
                break;
            }
            let chunk = buf.get(..n).unwrap_or_default().to_vec();
            self.keep_err(&chunk);
        }
    }

    pub(crate) fn wait_exit(&mut self, grace: Duration) -> (String, String) {
        // A grace too long for the clock has no end.
        let until = Instant::now().checked_add(grace);
        let status = loop {
            match self.proc.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if until.is_none_or(|u| Instant::now() < u) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                _ => {
                    self.killer().kill();
                    break self.proc.wait().ok();
                }
            }
        };
        self.reaped = true;
        self.drain_stderr();
        let how = match status {
            Some(s) if s.code() == Some(101) => "it panicked".to_owned(),
            Some(s) => match s.signal() {
                Some(libc::SIGABRT) => "it aborted".to_owned(),
                Some(libc::SIGKILL) => "it was stopped".to_owned(),
                Some(libc::SIGSYS) => "it made a system call its sandbox forbids".to_owned(),
                Some(libc::SIGSEGV | libc::SIGBUS) => "it made a memory fault".to_owned(),
                Some(sig) => format!("it ended on signal {sig}"),
                None => format!("it exited with code {}", s.code().unwrap_or(-1)),
            },
            None => "it could not be waited for".to_owned(),
        };
        (how, String::from_utf8_lossy(&self.err_tail).into_owned())
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for Child {
    fn drop(&mut self) {
        if !self.reaped {
            self.killer().kill();
            let _ = self.proc.wait();
        }
    }
}

// ---------- the app, seen from the helper ----------

pub(crate) struct Parent {
    sock: UnixStream,
}

pub(crate) struct Mapped {
    ptr: *mut u8,
    len: usize,
    writable: bool,
}

impl Mapped {
    /// Its bytes; the app does not write the region while the request is under way.
    pub(crate) fn bytes(&self) -> &[u8] {
        // SAFETY: the mapping is `len` readable bytes while `self` lives, and nobody writes
        // it during the request.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub(crate) fn write(&mut self, data: &[u8]) -> bool {
        if !self.writable || data.len() > self.len {
            return false;
        }
        // SAFETY: the mapping is `len` writable bytes and `data` fits.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr, data.len()) };
        true
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: mapped in `Parent::recv` with this length; unmapped once.
        unsafe { libc::munmap(self.ptr.cast(), self.len.max(1)) };
    }
}

impl Parent {
    pub(crate) fn open() -> io::Result<Parent> {
        // SAFETY: standard input is the socket the app gave this process; it is taken over
        // here and used for nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(0) };
        Ok(Parent { sock: UnixStream::from(fd) })
    }

    pub(crate) fn recv(&mut self) -> io::Result<(Vec<u8>, Vec<Mapped>)> {
        #[cfg(target_os = "android")]
        platform::limit_self(&self.sock)?;
        let (lead_bytes, fds) = recv_lead(&self.sock)?;
        let (len, n) = lead(lead_bytes)?;
        if fds.len() != n {
            return Err(io::Error::other("descriptors and regions differ"));
        }
        let mut described = vec![0u8; regions_len(n)];
        (&self.sock).read_exact(&mut described)?;
        let mut payload = vec![0u8; len];
        (&self.sock).read_exact(&mut payload)?;
        let mut mapped = Vec::with_capacity(n);
        for (r, fd) in regions(&described)?.into_iter().zip(fds) {
            let len = usize::try_from(r.len).map_err(|_| io::Error::other("region too large"))?;
            let ptr = map(fd.as_raw_fd(), len.max(1), r.writable)?;
            // The mapping keeps the memory; the descriptor is closed now.
            drop(fd);
            mapped.push(Mapped { ptr, len, writable: r.writable });
        }
        Ok((payload, mapped))
    }

    pub(crate) fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        let mut frame = header(payload.len(), &[])?;
        frame.extend_from_slice(payload);
        (&self.sock).write_all(&frame)
    }
}

/// Limits the helper shares on Linux and macOS: no core dumps (they would hold images), and
/// no more open files than standard input, output and error plus one request's regions, so
/// regions cannot be hoarded.
#[cfg(not(target_os = "android"))]
pub(super) fn common_limits() -> Result<(), String> {
    let set = |res, n: libc::rlim_t| {
        let lim = libc::rlimit { rlim_cur: n, rlim_max: n };
        // SAFETY: lowers this process's own limit.
        if unsafe { libc::setrlimit(res, &lim) } != 0 {
            return Err(format!("setrlimit: {}", io::Error::last_os_error()));
        }
        Ok(())
    };
    set(libc::RLIMIT_CORE, 0)?;
    set(libc::RLIMIT_NOFILE, (3 + MAX_REGIONS) as libc::rlim_t)
}

/// Closes every descriptor but standard input, output and error.
#[cfg(not(target_os = "android"))]
pub(super) fn close_inherited() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: close_range on descriptors this process owns and no longer needs; the
        // standard ones are kept.
        let r = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) };
        if r == 0 {
            return;
        }
    }
    // SAFETY: as above, one by one, up to the descriptor table's size.
    let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) }.max(256);
    for fd in 3..max as libc::c_int {
        // SAFETY: closing a descriptor that may not exist is harmless.
        unsafe { libc::close(fd) };
    }
}

/// Nothing is registered with the system on Unix.
pub(crate) fn forget() {}

/// On Android the helper shares its process with the Java runtime, whose descriptors stay.
pub(crate) fn lock_down() -> Result<Vec<Layer>, String> {
    #[cfg(not(target_os = "android"))]
    close_inherited();
    platform::lock_down()
}
