//! Android: each helper is an isolated service process (`dev.remedyui.android.Decoder` in the
//! Android bridge's Java, declared with `android:isolatedProcess="true"`), one per helper
//! (`Context.bindIsolatedService` with an instance name of its own). Through the binder the
//! app hands it one end of a Unix socket and the write end of a pipe for its standard error;
//! the service loads the app's native library and runs the helper on them. From then on it
//! is as on Linux: frames over the socket, `ASharedMemory` regions passed beside them.
//!
//! The app cannot signal or look into a process of another user id, so: a helper is stopped
//! by shutting its socket and unbinding it (the system kills an isolated process nobody is
//! bound to); its time is on the clock; and its memory is limited by the helper itself, which
//! before each request reads the allowance the app sends ahead of the frame and sets its own
//! address-space limit (RLIMIT_AS) to its size now, its starting size and that.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

use super::unix::{Shm, owned_fd, recv_frame, send_frame, socketpair};
use crate::media_isolated::Layer;

/// The helper limits its own address space for each request, from the app's allowance.
pub(crate) const LIMITS_MEMORY: bool = true;

/// Isolated processes run under these user ids (within each Android user's range).
const ISOLATED_UIDS: std::ops::RangeInclusive<u32> = 90_000..=99_999;
const PER_USER_RANGE: u32 = 100_000;

/// How long the system may take to start a helper's process.
// shortcut: a fixed bound on starting a process, only reached when the system is stuck;
// measure it like the engine's spawn cost if a slow device ever needs more.
const START_WAIT_MS: i64 = 30_000;

/// The symbol an app exports to register its own image formats in the helper: on Android
/// the helper runs no `main` of the app's.
const FORMATS_SYMBOL: &CStr = c"remedy_ui_media_isolated_formats";

#[link(name = "android")]
unsafe extern "C" {
    fn ASharedMemory_create(name: *const c_char, size: usize) -> c_int;
}

pub(super) fn shared_fd(len: usize) -> io::Result<OwnedFd> {
    // SAFETY: the name is a NUL-terminated literal.
    let fd = owned_fd(unsafe { ASharedMemory_create(c"remedy-image".as_ptr(), len) })?;
    // SAFETY: descriptor flag and seal changes on our own region. Where ASharedMemory is a
    // memfd the seals keep its size fixed as on Linux (failing harmlessly elsewhere: an
    // ashmem region's size cannot change once mapped, which `Shm::new` does before the helper
    // gets it).
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(
            fd.as_raw_fd(),
            libc::F_ADD_SEALS,
            libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL,
        );
    }
    Ok(fd)
}

// ---------- Java ----------

struct Java {
    vm: JavaVM,
    context: GlobalRef,
    class: GlobalRef,
}

/// The pending Java exception as text (cleared), or the JNI error.
fn describe(env: &mut JNIEnv<'_>, e: jni::errors::Error) -> String {
    if !env.exception_check().unwrap_or(false) {
        return e.to_string();
    }
    let thrown = env.exception_occurred();
    let _ = env.exception_clear();
    thrown
        .and_then(|t| env.call_method(&t, "toString", "()Ljava/lang/String;", &[]))
        .and_then(|s| s.l())
        .and_then(|s| env.get_string(&JString::from(s)).map(String::from))
        .unwrap_or_else(|_| e.to_string())
}

/// Runs `f` on this thread's JNI environment, in a frame of its own.
fn with_env<T>(vm: &JavaVM, f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<T>) -> Result<T, String> {
    let mut env = vm.attach_current_thread_permanently().map_err(|e| e.to_string())?;
    let r = env.with_local_frame(8, |env| f(env));
    r.map_err(|e| {
        let mut described = String::new();
        let _ = env.with_local_frame(8, |env| -> jni::errors::Result<()> {
            described = describe(env, e);
            Ok(())
        });
        described
    })
}

fn java() -> Result<&'static Java, String> {
    static J: OnceLock<Result<Java, String>> = OnceLock::new();
    J.get_or_init(|| {
        let ctx = std::panic::catch_unwind(ndk_context::android_context)
            .map_err(|_| "no Android context (the app is not running in an activity)".to_owned())?;
        // SAFETY: ndk-context holds the process's JavaVM, valid for the process's life.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
        let (context, class) = with_env(&vm, |env| {
            // SAFETY: ndk-context's context is a global reference to the app's Context (its
            // activity), alive while the app runs; it is only read here.
            let context = unsafe { JObject::from_raw(ctx.context().cast()) };
            let loader =
                env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string("dev.remedyui.android.Decoder")?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            Ok((env.new_global_ref(&context)?, env.new_global_ref(class)?))
        })
        .map_err(|e| {
            format!(
                "the APK has no decoding service ({e}): it must carry the Android bridge's dex and \
                 declare dev.remedyui.android.Decoder with android:isolatedProcess=\"true\" \
                 (cargo remedy package does)"
            )
        })?;
        Ok(Java { vm, context, class })
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The file name of the native library this code is in, as `System.loadLibrary` takes it.
fn own_library() -> io::Result<String> {
    // SAFETY: an all-zero Dl_info is valid; dladdr fills it for an address in this library,
    // and its file name is a C string the loader keeps for the library's life.
    let path = unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(own_library as *const c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return Err(io::Error::other("this library's file could not be found"));
        }
        CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned()
    };
    let file = path.rsplit('/').next().unwrap_or(&path);
    file.strip_prefix("lib")
        .and_then(|f| f.strip_suffix(".so"))
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other(format!("{path} is not a library Android can load by name")))
}

// ---------- the helper, seen from the app ----------

struct Link {
    sock: UnixStream,
    /// The service connection (`Decoder.Link`).
    conn: GlobalRef,
}

/// Stops a helper from any thread.
#[derive(Clone)]
pub(crate) struct Killer(Arc<Link>);

impl Killer {
    pub(crate) fn kill(&self) {
        // Both sides' reads end at once; the process goes once unbound.
        let _ = self.0.sock.shutdown(std::net::Shutdown::Both);
        if let Ok(j) = java() {
            let _ = with_env(&j.vm, |env| {
                let class: &JClass<'_> = j.class.as_obj().into();
                env.call_static_method(
                    class,
                    "stop",
                    "(Landroid/content/ServiceConnection;)V",
                    &[JValue::Object(self.0.conn.as_obj())],
                )
                .map(|_| ())
            });
        }
    }

    /// Another user id's process is out of the app's sight.
    pub(crate) fn memory(&self) -> Option<u64> {
        None
    }

    /// As above: its time is on the clock.
    pub(crate) fn cpu_time(&self) -> Option<Duration> {
        None
    }
}

pub(crate) struct Child {
    link: Arc<Link>,
    err: File,
    err_tail: Vec<u8>,
    /// What the next request may use beyond the helper's size (set by `limit_memory`).
    allowance: AtomicU64,
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as c_int; 2];
    // SAFETY: `fds` has room for the two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let [r, w] = fds;
    Ok((owned_fd(r)?, owned_fd(w)?))
}

impl Child {
    pub(crate) fn spawn(_exe: &Path) -> io::Result<Child> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let j = java().map_err(io::Error::other)?;
        let library = own_library()?;
        let (host, theirs) = socketpair()?;
        let (err, err_theirs) = pipe()?;
        // SAFETY: makes our end of the stderr pipe non-blocking, so draining never waits.
        unsafe {
            let fl = libc::fcntl(err.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(err.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        let instance = format!("remedy_decoder_{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let conn = with_env(&j.vm, |env| {
            let class: &JClass<'_> = j.class.as_obj().into();
            let instance = env.new_string(&instance)?;
            let library = env.new_string(&library)?;
            let conn = env
                .call_static_method(
                    class,
                    "start",
                    "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;IIJ)Landroid/content/ServiceConnection;",
                    &[
                        JValue::Object(j.context.as_obj()),
                        JValue::Object(&instance),
                        JValue::Object(&library),
                        JValue::Int(theirs.as_raw_fd()),
                        JValue::Int(err_theirs.as_raw_fd()),
                        JValue::Long(START_WAIT_MS),
                    ],
                )?
                .l()?;
            env.new_global_ref(conn)
        })
        .map_err(io::Error::other)?;
        // The helper has its own copies now.
        drop((theirs, err_theirs));
        Ok(Child {
            link: Arc::new(Link { sock: UnixStream::from(host), conn }),
            err: File::from(err),
            err_tail: Vec::new(),
            allowance: AtomicU64::new(u64::MAX),
        })
    }

    pub(crate) fn send(&mut self, payload: &[u8], shared: &[(&Shm, bool)]) -> io::Result<()> {
        // Its own write: the frame's descriptors must not come with it (see `limit_self`).
        (&self.link.sock).write_all(&self.allowance.load(Ordering::Relaxed).to_le_bytes())?;
        send_frame(&self.link.sock, payload, shared)
    }

    pub(crate) fn recv(&mut self) -> io::Result<Vec<u8>> {
        recv_frame(&self.link.sock)
    }

    pub(crate) fn end_request(&mut self) {}

    pub(crate) fn killer(&self) -> Killer {
        Killer(self.link.clone())
    }

    pub(crate) fn memory_in_use(&self) -> Option<u64> {
        None
    }

    /// The helper's size is not known here (the engine then asks for its need alone): the
    /// helper adds its own size when it sets the limit.
    pub(crate) fn limit_memory(&self, bytes: u64) -> io::Result<()> {
        self.allowance.store(bytes, Ordering::Relaxed);
        Ok(())
    }

    fn keep_err(&mut self, b: &[u8]) {
        self.err_tail.extend_from_slice(b);
        let excess = self.err_tail.len().saturating_sub(crate::media_isolated::protocol::MAX_TEXT);
        self.err_tail.drain(..excess);
    }

    /// Reads what it wrote to standard error; `true` once it has gone (the pipe closed).
    fn read_err(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        loop {
            match self.err.read(&mut buf) {
                Ok(0) => return true,
                Ok(n) => {
                    let chunk = buf.get(..n).unwrap_or_default().to_vec();
                    self.keep_err(&chunk);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return e.kind() != io::ErrorKind::WouldBlock,
            }
        }
    }

    pub(crate) fn drain_stderr(&mut self) {
        self.read_err();
    }

    /// Waits up to `grace` for its process to end (its standard error closing: no other
    /// process holds it), stopping it after that.
    pub(crate) fn wait_exit(&mut self, grace: Duration) -> (String, String) {
        let until = Instant::now() + grace;
        let ended = loop {
            if self.read_err() {
                break true;
            }
            if Instant::now() >= until {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        if !ended {
            self.killer().kill();
        }
        let err = String::from_utf8_lossy(&self.err_tail).into_owned();
        let how = if !ended {
            "it was stopped"
        } else if err.contains("panicked at") {
            "it panicked"
        } else if err.contains("memory allocation of") {
            "it ran out of memory"
        } else {
            "its process ended"
        };
        (how.to_owned(), err)
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.killer().kill();
    }
}

// ---------- the helper's side ----------

/// Its resident size when it shut itself in: the room its limits allow beyond each image.
static START_RESIDENT: AtomicU64 = AtomicU64::new(0);

/// This process's address space and resident size, in bytes.
fn statm() -> io::Result<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/self/statm")?;
    let mut f = s.split_whitespace().map(|n| n.parse::<u64>().ok());
    let bad = || io::Error::other("unreadable /proc/self/statm");
    let (size, resident) = (f.next().flatten().ok_or_else(bad)?, f.next().flatten().ok_or_else(bad)?);
    // SAFETY: a pure query.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).map_err(|_| bad())?;
    Ok((size.saturating_mul(page), resident.saturating_mul(page)))
}

/// Before each request: reads the app's allowance and limits this process's address space
/// to its size now, its starting resident size and that. Only the soft limit is set, so the
/// next request can raise it again.
pub(super) fn limit_self(sock: &UnixStream) -> io::Result<()> {
    let mut b = [0u8; 8];
    (&*sock).read_exact(&mut b)?;
    let (size, _) = statm()?;
    let limit =
        size.saturating_add(START_RESIDENT.load(Ordering::Relaxed)).saturating_add(u64::from_le_bytes(b));
    let lim = libc::rlimit { rlim_cur: limit as libc::rlim_t, rlim_max: libc::RLIM_INFINITY };
    // SAFETY: sets this process's own soft limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &lim) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn lock_down() -> Result<Vec<Layer>, String> {
    // SAFETY: a pure query.
    let uid = unsafe { libc::getuid() };
    if !ISOLATED_UIDS.contains(&(uid % PER_USER_RANGE)) {
        return Err(format!("the helper runs as user {uid}, not in an isolated process"));
    }
    let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: lowers this process's own limit: no core dumps (they would hold images).
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(format!("setrlimit: {}", io::Error::last_os_error()));
    }
    let (_, resident) = statm().map_err(|e| e.to_string())?;
    START_RESIDENT.store(resident, Ordering::Relaxed);
    Ok(vec![Layer::IsolatedProcess, Layer::ResourceLimits])
}

/// Calls the app's `remedy_ui_media_isolated_formats`, when its library exports one.
fn app_formats() {
    // SAFETY: dladdr/dlopen(RTLD_NOLOAD) find the library this code is in, already loaded;
    // the symbol, when there, is the app's `extern "C" fn()` by its documented contract.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(app_formats as *const c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return;
        }
        let lib = libc::dlopen(info.dli_fname, libc::RTLD_NOW | libc::RTLD_NOLOAD);
        if lib.is_null() {
            return;
        }
        let f = libc::dlsym(lib, FORMATS_SYMBOL.as_ptr());
        if !f.is_null() {
            let f: extern "C" fn() = std::mem::transmute(f);
            f();
        }
    }
}

/// `Decoder.run`, on the isolated service's own thread: `sock` and `err` are the descriptors
/// the app handed over the binder. Never returns: the process ends with the helper.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_remedyui_android_Decoder_run(
    _env: *mut c_void,
    _class: *mut c_void,
    sock: c_int,
    err: c_int,
) {
    // SAFETY: the helper reads its socket as standard input and writes errors to standard
    // error, as on Linux; this process is the helper's alone, and Java gave up both
    // descriptors to this call.
    unsafe {
        if sock != 0 {
            libc::dup2(sock, 0);
            libc::close(sock);
        }
        if err != 2 {
            libc::dup2(err, 2);
            libc::close(err);
        }
    }
    app_formats();
    let code = crate::media_isolated::helper::run();
    // SAFETY: ends this process at once, as a helper's exit does elsewhere.
    unsafe { libc::_exit(code) }
}
