//! Windows: the helper runs in an AppContainer with no capabilities, entered with a restricted
//! token, inside a job object. Requests go over anonymous pipes (the helper's standard input
//! and output); shared memory is a pagefile-backed section whose handle the app duplicates
//! into the helper for one request and closes again after.
//!
//! Invariants: every raw handle here is owned by an `OwnedHandle` or `File` from the moment it
//! is created, so it is closed exactly once. A mapped view lives exactly as long as the `Shm`
//! or `Mapped` holding it. The helper never forms a reference to memory the app may still be
//! writing (the app writes a region only before sending it), and the app reads what the helper
//! wrote only by copying it out, so a hostile helper can change bytes but not break memory
//! safety in the app.

use std::ffi::{OsString, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    DUPLICATE_CLOSE_SOURCE, DuplicateHandle, FALSE, FILETIME, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, LUID, SetHandleInformation, TRUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CopySid, CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, FreeSid, GetLengthSid,
    GetTokenInformation, LUA_TOKEN, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_REMOVED, SECURITY_CAPABILITIES,
    TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenIsAppContainer, TokenPrivileges,
};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DETACHED_PROCESS, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::frame::{Region, header, lead, regions, regions_len};
use crate::media_isolated::Layer;

/// The job's memory limit is the operating system's: allocations past it fail in the helper.
pub(crate) const LIMITS_MEMORY: bool = true;

fn last() -> io::Error {
    io::Error::last_os_error()
}

/// Names the step an error came from.
fn at(step: &'static str) -> impl Fn(io::Error) -> io::Error {
    move |e| io::Error::new(e.kind(), format!("{step}: {e}"))
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Takes ownership of a handle a call just returned; null or invalid is the call's error.
fn owned(h: HANDLE) -> io::Result<OwnedHandle> {
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(last());
    }
    // SAFETY: `h` is a valid handle the caller just received and nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

// ---------- shared memory ----------

/// A pagefile-backed section, mapped whole into the app.
pub(crate) struct Shm {
    section: OwnedHandle,
    view: *mut u8,
    len: usize,
}

// SAFETY: the view is plain memory owned by this value; access goes through `&self`/`&mut
// self` methods that copy bytes, so moving or sharing it between threads is sound.
unsafe impl Send for Shm {}
// SAFETY: as above; `&self` methods only read.
unsafe impl Sync for Shm {}

impl Shm {
    /// A region of `len` bytes (at least one, so the section can exist).
    pub(crate) fn new(len: usize) -> io::Result<Shm> {
        let size = len.max(1) as u64;
        // SAFETY: a pagefile-backed mapping (INVALID_HANDLE_VALUE) with default security and
        // no name; the returned handle is taken by `owned`.
        let section = owned(unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                null(),
                PAGE_READWRITE,
                (size >> 32) as u32,
                size as u32,
                null(),
            )
        })?;
        // SAFETY: `section` is a valid section of `size` bytes; the view is unmapped in Drop.
        let view = unsafe {
            MapViewOfFile(section.as_raw_handle(), FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, size as usize)
        };
        if view.Value.is_null() {
            return Err(last());
        }
        Ok(Shm { section, view: view.Value.cast(), len })
    }

    /// Its length in bytes.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Copies `data` to its start (before it is handed over).
    pub(crate) fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if data.len() > self.len {
            return Err(io::Error::other("data larger than the region"));
        }
        // SAFETY: the view is `self.len` bytes and `data` fits; the helper has no handle to
        // this region yet, so nothing else writes it.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.view, data.len()) };
        Ok(())
    }

    /// A copy of its first `len` bytes (what the helper wrote).
    pub(crate) fn read(&self, len: usize) -> io::Result<Vec<u8>> {
        if len > self.len {
            return Err(io::Error::other("read past the region"));
        }
        let mut out: Vec<u8> = Vec::new();
        out.try_reserve_exact(len).map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        // SAFETY: the view holds `self.len >= len` bytes and `out` has room for `len`. The
        // helper may still be writing (it is untrusted); the bytes are copied, never
        // referenced, so that can change what is copied but nothing else.
        unsafe {
            std::ptr::copy_nonoverlapping(self.view, out.as_mut_ptr(), len);
            out.set_len(len);
        }
        Ok(out)
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        // SAFETY: the view was mapped in `new` and is unmapped once, here.
        unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view.cast() }) };
    }
}

// ---------- starting the helper ----------

/// The AppContainer's SID.
struct Sid(Vec<u32>);

/// The AppContainer profile this process registered: its name (wide, NUL-terminated) and SID.
type Registered = Mutex<Option<(Vec<u16>, Arc<Sid>)>>;

fn container() -> &'static Registered {
    static C: OnceLock<Registered> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The SID of the AppContainer the helpers run in: one per installed app (named from its
/// executable's path), not shared with other RemedyUI apps. Its profile is registered on
/// first use and removed by [`forget`].
fn container_sid(exe: &Path) -> io::Result<Arc<Sid>> {
    let mut slot = container().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, sid)) = slot.as_ref() {
        return Ok(sid.clone());
    }
    let made = (|| {
        // FNV-1a of the lower-cased path.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in exe.to_string_lossy().to_lowercase().bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        let name = wide(std::ffi::OsStr::new(&format!("RemedyUI.MediaDecoder.{h:016x}")));
        let display = wide(std::ffi::OsStr::new("RemedyUI image decoder"));
        let mut psid: *mut c_void = null_mut();
        // SAFETY: NUL-terminated wide strings and no capabilities; on success `psid` is a SID
        // released with FreeSid below, after it is copied.
        let mut hr = unsafe {
            CreateAppContainerProfile(name.as_ptr(), display.as_ptr(), display.as_ptr(), null(), 0, &mut psid)
        };
        // HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS): registered by an earlier run.
        if hr == 0x8007_00B7_u32 as i32 {
            // SAFETY: as above; the SID is released the same way.
            hr = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut psid) };
        }
        if hr < 0 || psid.is_null() {
            return Err(format!("no AppContainer SID (HRESULT {hr:#x})"));
        }
        // SAFETY: `psid` is a valid SID.
        let len = unsafe { GetLengthSid(psid) };
        let mut buf = vec![0u32; (len as usize).div_ceil(4)];
        // SAFETY: `buf` holds at least `len` bytes, 4-aligned as a SID needs.
        let ok = unsafe { CopySid(len, buf.as_mut_ptr().cast(), psid) };
        // SAFETY: `psid` came from DeriveAppContainerSidFromAppContainerName.
        unsafe { FreeSid(psid) };
        if ok == 0 {
            return Err(format!("could not copy the AppContainer SID: {}", last()));
        }
        Ok((name, Arc::new(Sid(buf))))
    })();
    let (name, sid) = made.map_err(io::Error::other)?;
    *slot = Some((name, sid.clone()));
    Ok(sid)
}

/// Removes the AppContainer profile this process registered (isolation was turned off), so
/// nothing stays behind on the machine. Running helpers are unaffected; the next helper
/// registers it again.
pub(crate) fn forget() {
    let taken = container().lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some((name, _)) = taken {
        // SAFETY: a NUL-terminated profile name this process registered.
        unsafe { DeleteAppContainerProfile(name.as_ptr()) };
    }
}

/// This process's token with every privilege but bypass-traverse removed (the helper drops
/// that one itself, see `drop_traverse`) and administrator groups made deny-only.
fn restricted_token() -> io::Result<OwnedHandle> {
    let mut token: HANDLE = null_mut();
    let access =
        TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID;
    // SAFETY: the current-process pseudo handle; `token` is taken by `owned` on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut token) } == 0 {
        return Err(last());
    }
    let token = owned(token)?;
    let mut restricted: HANDLE = null_mut();
    // SAFETY: `token` is a primary token opened with TOKEN_DUPLICATE; no SID or privilege
    // lists are passed (the flags do the restricting).
    let ok = unsafe {
        CreateRestrictedToken(
            token.as_raw_handle(),
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
            0,
            null(),
            0,
            null(),
            0,
            null(),
            &mut restricted,
        )
    };
    if ok == 0 {
        return Err(last());
    }
    owned(restricted)
}

/// An anonymous pipe: (read end, write end), neither inheritable yet.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: both out-pointers are valid; the handles are taken by `owned`.
    if unsafe { CreatePipe(&mut r, &mut w, null(), 0) } == 0 {
        return Err(last());
    }
    Ok((owned(r)?, owned(w)?))
}

fn inheritable(h: &OwnedHandle) -> io::Result<()> {
    // SAFETY: `h` is a valid handle owned by the caller.
    if unsafe { SetHandleInformation(h.as_raw_handle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(last());
    }
    Ok(())
}

/// A process-creation attribute list.
struct Attributes {
    buf: Vec<u64>,
}

impl Attributes {
    fn new(count: u32) -> io::Result<Attributes> {
        let mut size = 0usize;
        // SAFETY: the documented size query (null list); it fails with the size set.
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        let mut buf = vec![0u64; size.div_ceil(8)];
        // SAFETY: `buf` holds `size` bytes, 8-aligned.
        if unsafe { InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &mut size) } == 0 {
            return Err(last());
        }
        Ok(Attributes { buf })
    }

    fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_mut_ptr().cast()
    }

    /// Sets `attribute` to the `size` bytes at `value`, which must outlive the process
    /// creation that uses this list.
    fn set(&mut self, attribute: u32, value: *const c_void, size: usize) -> io::Result<()> {
        // SAFETY: the list was initialized for enough attributes; `value` points at `size`
        // readable bytes kept alive by the caller until CreateProcessAsUserW returns.
        let ok = unsafe {
            UpdateProcThreadAttribute(self.list(), 0, attribute as usize, value, size, null_mut(), null())
        };
        if ok == 0 { Err(last()) } else { Ok(()) }
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: the list was initialized in `new`.
        unsafe { DeleteProcThreadAttributeList(self.list()) };
    }
}

/// Mitigations that cost a decoder nothing: no dynamic code, no extension points, no
/// non-system fonts, no images from remote or low-integrity locations, system DLLs first,
/// strict handle checks, terminate on heap corruption, full ASLR.
const MITIGATIONS: u64 = (1 << 12) // heap terminate
    | (1 << 16) // bottom-up ASLR
    | (1 << 20) // high-entropy ASLR
    | (1 << 24) // strict handle checks
    | (1 << 32) // extension points disabled
    | (1 << 36) // dynamic code prohibited
    | (1 << 48) // non-system fonts disabled
    | (1 << 52) // no images from remote locations
    | (1 << 56) // no images with a low mandatory label
    | (1 << 60); // prefer System32 images

/// A less privileged AppContainer: it is not a member of ALL APPLICATION PACKAGES, so the
/// objects every AppContainer may use (among them what Windows Sockets needs) are closed to
/// it; only those granted to ALL RESTRICTED APPLICATION PACKAGES (system DLLs) stay open.
const ALL_APPLICATION_PACKAGES_OPT_OUT: u32 = 1;

/// No child processes at all.
const CHILD_PROCESS_RESTRICTED: u32 = 1;

/// Every UI restriction a job can place: no desktop switching, display settings, logoff,
/// global atoms, other processes' USER handles, clipboard, or system parameters.
const UI_LIMITS: u32 = 0x1FF;

/// The job's flags, without the memory limit (added per request).
const JOB_FLAGS: u32 = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
    | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;

fn job_limits(job: &OwnedHandle, memory: Option<u64>) -> io::Result<()> {
    // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a valid value.
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_FLAGS;
    info.BasicLimitInformation.ActiveProcessLimit = 1;
    if let Some(m) = memory {
        info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.ProcessMemoryLimit = usize::try_from(m).unwrap_or(usize::MAX);
    }
    // SAFETY: `job` is a job handle; `info` is the struct the class names, at its size.
    let ok = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 { Err(last()) } else { Ok(()) }
}

fn make_job() -> io::Result<OwnedHandle> {
    // SAFETY: an unnamed job with default security; taken by `owned`.
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
    job_limits(&job, None)?;
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS { UIRestrictionsClass: UI_LIMITS };
    // SAFETY: as in `job_limits`, for the UI restrictions class.
    let ok = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicUIRestrictions,
            (&raw const ui).cast(),
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    };
    if ok == 0 {
        return Err(last());
    }
    Ok(job)
}

/// Stops a helper from any thread.
#[derive(Clone)]
pub(crate) struct Killer(Arc<OwnedHandle>);

impl Killer {
    pub(crate) fn kill(&self) {
        // SAFETY: the process handle stays open while this Arc lives; terminating a process
        // that already ended does nothing.
        unsafe { TerminateProcess(self.0.as_raw_handle(), 0xDEC0_DE00) };
    }

    /// The processor time it has used, user and kernel.
    pub(crate) fn cpu_time(&self) -> Option<Duration> {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the handle has PROCESS_QUERY_LIMITED_INFORMATION; four out-pointers.
        let ok = unsafe {
            GetProcessTimes(self.0.as_raw_handle(), &mut created, &mut exited, &mut kernel, &mut user)
        };
        let ticks = |t: FILETIME| u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime);
        // FILETIME counts 100 ns.
        (ok != 0).then(|| Duration::from_nanos((ticks(kernel) + ticks(user)).saturating_mul(100)))
    }

    /// Its committed private memory, in bytes.
    pub(crate) fn memory(&self) -> Option<u64> {
        // SAFETY: an all-zero counters struct is valid; `cb` says which struct it is.
        let mut c: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        c.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        // SAFETY: the handle has PROCESS_QUERY_LIMITED_INFORMATION (it is the creator's).
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                self.0.as_raw_handle(),
                (&raw mut c).cast::<PROCESS_MEMORY_COUNTERS>(),
                c.cb,
            )
        };
        (ok != 0).then_some(c.PrivateUsage as u64)
    }
}

/// A running helper, seen from the app.
pub(crate) struct Child {
    process: Arc<OwnedHandle>,
    job: OwnedHandle,
    to: File,
    from: File,
    err: File,
    /// Its stderr so far (the tail), to tell how it ended.
    err_tail: Vec<u8>,
    /// Section handles duplicated into it for the request under way.
    remote: Vec<u64>,
}

impl Child {
    /// Starts `exe` in helper mode, sandboxed.
    pub(crate) fn spawn(exe: &Path) -> io::Result<Child> {
        // Process creation wants backslashes (paths built by tools may mix them).
        let exe = std::path::PathBuf::from(exe.as_os_str().to_string_lossy().replace('/', "\\"));
        let exe = exe.as_path();
        match Self::create(exe, &*container_sid(exe)?) {
            // The profile was removed (another instance of the app turned isolation off):
            // register it again.
            Err(e) if e.raw_os_error() == Some(2) => {
                container().lock().unwrap_or_else(PoisonError::into_inner).take();
                Self::create(exe, &*container_sid(exe)?)
            }
            r => r,
        }
        .map_err(at("starting the helper"))
    }

    fn create(exe: &Path, sid: &Sid) -> io::Result<Child> {
        let token = restricted_token().map_err(at("restricted token"))?;
        let (child_in, host_in) = pipe()?;
        let (host_out, child_out) = pipe()?;
        let (host_err, child_err) = pipe()?;
        for h in [&child_in, &child_out, &child_err] {
            inheritable(h)?;
        }

        let caps = SECURITY_CAPABILITIES {
            AppContainerSid: sid.0.as_ptr().cast_mut().cast(),
            Capabilities: null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        let handles = [child_in.as_raw_handle(), child_out.as_raw_handle(), child_err.as_raw_handle()];
        let child_policy = CHILD_PROCESS_RESTRICTED;
        let mitigations = MITIGATIONS;
        // Created inside its job: there is no moment it runs (or waits, suspended, for an app
        // that may die) outside it.
        let job = make_job().map_err(at("job object"))?;
        let jobs = [job.as_raw_handle()];
        let lpac = ALL_APPLICATION_PACKAGES_OPT_OUT;
        let mut attrs = Attributes::new(6)?;
        attrs.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, jobs.as_ptr().cast(), size_of_val(&jobs))?;
        attrs.set(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, (&raw const lpac).cast(), 4)?;
        attrs.set(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            (&raw const caps).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
        )?;
        attrs.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles.as_ptr().cast(), size_of_val(&handles))?;
        attrs.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, (&raw const child_policy).cast(), 4)?;
        attrs.set(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, (&raw const mitigations).cast(), 8)?;

        // SAFETY: an all-zero STARTUPINFOEXW is valid; the fields used are set below.
        let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = child_in.as_raw_handle();
        si.StartupInfo.hStdOutput = child_out.as_raw_handle();
        si.StartupInfo.hStdError = child_err.as_raw_handle();
        si.lpAttributeList = attrs.list();

        let app = wide(exe.as_os_str());
        let mut line = OsString::from("\"");
        line.push(exe.as_os_str());
        line.push("\" ");
        line.push(crate::media_isolated::HELPER_ARG);
        let mut line = wide(&line);
        // It starts in the system folder: the app's working folder is closed to it.
        // SAFETY: the size query (no buffer).
        let need = unsafe { GetSystemDirectoryW(null_mut(), 0) };
        let mut cwd = vec![0u16; need as usize + 1];
        // SAFETY: `cwd` has room for the length passed.
        let n = unsafe { GetSystemDirectoryW(cwd.as_mut_ptr(), cwd.len() as u32) } as usize;
        if n == 0 || n >= cwd.len() {
            return Err(at("system folder")(last()));
        }
        // SAFETY: an all-zero PROCESS_INFORMATION is valid output space.
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: every pointer is valid for the call: the NUL-terminated application name and
        // mutable command line, the startup info whose attribute list, handles, SID and
        // policies all outlive the call, and `pi` for the output. The token is a restricted
        // copy of this process's own primary token, which needs no extra privilege.
        let ok = unsafe {
            CreateProcessAsUserW(
                token.as_raw_handle(),
                app.as_ptr(),
                line.as_mut_ptr(),
                null(),
                null(),
                TRUE,
                // Detached: no console, which an AppContainer process cannot start up with.
                EXTENDED_STARTUPINFO_PRESENT | DETACHED_PROCESS,
                null(),
                cwd.as_ptr(),
                &si.StartupInfo,
                &mut pi,
            )
        };
        if ok == 0 {
            // Raw, so `spawn` can tell a missing profile.
            return Err(last());
        }
        let process = owned(pi.hProcess)?;
        drop(owned(pi.hThread));
        drop((child_in, child_out, child_err));
        Ok(Child {
            process: Arc::new(process),
            job,
            to: File::from(host_in),
            from: File::from(host_out),
            err: File::from(host_err),
            err_tail: Vec::new(),
            remote: Vec::new(),
        })
    }

    /// Sends a frame, handing the helper `regions` (each read-only or writable for it).
    pub(crate) fn send(&mut self, payload: &[u8], shared: &[(&Shm, bool)]) -> io::Result<()> {
        let mut described = Vec::with_capacity(shared.len());
        for (shm, writable) in shared {
            let access = if *writable { FILE_MAP_READ | FILE_MAP_WRITE } else { FILE_MAP_READ };
            let mut remote: HANDLE = null_mut();
            // SAFETY: duplicates our section handle into the helper's handle table with only
            // the access it needs; the helper's copy is closed in `end_request`.
            let ok = unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    shm.section.as_raw_handle(),
                    self.process.as_raw_handle(),
                    &mut remote,
                    access,
                    FALSE,
                    0,
                )
            };
            if ok == 0 {
                return Err(last());
            }
            self.remote.push(remote as u64);
            described.push(Region { handle: remote as u64, len: shm.len() as u64, writable: *writable });
        }
        let mut frame = header(payload.len(), &described)?;
        frame.extend_from_slice(payload);
        self.to.write_all(&frame)
    }

    /// Reads one frame from the helper (which hands no regions back).
    pub(crate) fn recv(&mut self) -> io::Result<Vec<u8>> {
        let mut lead_bytes = [0u8; 3];
        self.from.read_exact(&mut lead_bytes)?;
        let (len, n) = lead(lead_bytes)?;
        if n != 0 {
            return Err(io::Error::other("the helper sent regions"));
        }
        let mut payload = vec![0u8; len];
        self.from.read_exact(&mut payload)?;
        Ok(payload)
    }

    /// Takes back the regions handed over for the request just answered.
    pub(crate) fn end_request(&mut self) {
        for remote in self.remote.drain(..) {
            // SAFETY: closes, in the helper, the handle duplicated into it for this request.
            // The helper does not close these itself, so the value still names our section.
            unsafe {
                DuplicateHandle(
                    self.process.as_raw_handle(),
                    remote as HANDLE,
                    null_mut(),
                    null_mut(),
                    0,
                    FALSE,
                    DUPLICATE_CLOSE_SOURCE,
                )
            };
        }
    }

    pub(crate) fn killer(&self) -> Killer {
        Killer(self.process.clone())
    }

    /// Its committed private memory, in bytes (what the job limits).
    pub(crate) fn memory_in_use(&self) -> Option<u64> {
        self.killer().memory()
    }

    /// Allocations past `bytes` of committed memory fail in the helper from now on.
    pub(crate) fn limit_memory(&self, bytes: u64) -> io::Result<()> {
        job_limits(&self.job, Some(bytes))
    }

    /// Reads what it wrote to stderr since last time, keeping the tail (it must not block on
    /// a full pipe).
    pub(crate) fn drain_stderr(&mut self) {
        let mut avail = 0u32;
        // SAFETY: peeks the pipe's byte count only.
        let ok = unsafe {
            PeekNamedPipe(self.err.as_raw_handle(), null_mut(), 0, null_mut(), &mut avail, null_mut())
        };
        if ok != 0 && avail > 0 {
            let mut buf = vec![0u8; avail as usize];
            if let Ok(n) = self.err.read(&mut buf) {
                self.keep_err(buf.get(..n).unwrap_or_default());
            }
        }
    }

    fn keep_err(&mut self, b: &[u8]) {
        self.err_tail.extend_from_slice(b);
        let excess = self.err_tail.len().saturating_sub(crate::media_isolated::protocol::MAX_TEXT);
        self.err_tail.drain(..excess);
    }

    /// Waits up to `grace` for it to end (killing it then), and says how it ended, with the
    /// tail of what it wrote to stderr.
    pub(crate) fn wait_exit(&mut self, grace: Duration) -> (String, String) {
        let ms = u32::try_from(grace.as_millis()).unwrap_or(INFINITE - 1);
        // SAFETY: waits on our process handle.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), ms) } != WAIT_OBJECT_0 {
            self.killer().kill();
            // SAFETY: as above; it was just terminated.
            unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) };
        }
        let mut code = 0u32;
        // SAFETY: reads the exit code of the process that has ended.
        unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) };
        let mut rest = Vec::new();
        // It has ended: its end of the pipe is closed (it could start no other process to
        // hold it), so this reads to the end.
        let _ = (&self.err).take(crate::media_isolated::protocol::MAX_TEXT as u64 * 16).read_to_end(&mut rest);
        self.keep_err(&rest);
        let how = match code {
            101 => "it panicked".to_owned(),
            0xC000_0409 => "it aborted".to_owned(),
            0xC000_0005 => "it made an access violation".to_owned(),
            0xC000_00FD => "it overflowed its stack".to_owned(),
            0xDEC0_DE00 => "it was stopped".to_owned(),
            c => format!("it exited with code {c:#x}"),
        };
        (how, String::from_utf8_lossy(&self.err_tail).into_owned())
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.killer().kill();
    }
}

// ---------- the helper's side ----------

/// The app, seen from the helper: its standard input and output.
pub(crate) struct Parent {
    from: File,
    to: File,
}

/// A region the app handed over, mapped for one request.
pub(crate) struct Mapped {
    view: *mut u8,
    len: usize,
    writable: bool,
}

impl Mapped {
    /// Its bytes. The app wrote them before handing the region over and does not touch it
    /// again until the reply, so they hold still while borrowed.
    pub(crate) fn bytes(&self) -> &[u8] {
        // SAFETY: the view is `len` readable bytes for as long as `self` lives, and the app
        // does not write it during the request (see above).
        unsafe { std::slice::from_raw_parts(self.view, self.len) }
    }

    /// Copies `data` to its start, if it is writable and `data` fits.
    pub(crate) fn write(&mut self, data: &[u8]) -> bool {
        if !self.writable || data.len() > self.len {
            return false;
        }
        // SAFETY: the view is `len` writable bytes and `data` fits.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.view, data.len()) };
        true
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: mapped in `Parent::recv`, unmapped once here. The handle is the app's to
        // close.
        unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view.cast() }) };
    }
}

impl Parent {
    pub(crate) fn open() -> io::Result<Parent> {
        // SAFETY: standard handles set by the app at creation; this process owns them and
        // nothing else in the helper uses them.
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        // SAFETY: as above.
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        Ok(Parent { from: File::from(owned(input)?), to: File::from(owned(output)?) })
    }

    pub(crate) fn recv(&mut self) -> io::Result<(Vec<u8>, Vec<Mapped>)> {
        let mut lead_bytes = [0u8; 3];
        self.from.read_exact(&mut lead_bytes)?;
        let (len, n) = lead(lead_bytes)?;
        let mut described = vec![0u8; regions_len(n)];
        self.from.read_exact(&mut described)?;
        let mut payload = vec![0u8; len];
        self.from.read_exact(&mut payload)?;
        let mut mapped = Vec::with_capacity(n);
        for r in regions(&described)? {
            let len = usize::try_from(r.len).map_err(|_| io::Error::other("region too large"))?;
            let access = if r.writable { FILE_MAP_READ | FILE_MAP_WRITE } else { FILE_MAP_READ };
            // SAFETY: the handle was duplicated into this process by the app for this
            // request, to a section of at least `len.max(1)` bytes.
            let view = unsafe { MapViewOfFile(r.handle as HANDLE, access, 0, 0, len.max(1)) };
            if view.Value.is_null() {
                return Err(last());
            }
            mapped.push(Mapped { view: view.Value.cast(), len, writable: r.writable });
        }
        Ok((payload, mapped))
    }

    pub(crate) fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        let mut frame = header(payload.len(), &[])?;
        frame.extend_from_slice(payload);
        self.to.write_all(&frame)?;
        self.to.flush()
    }
}

fn token_info(class: i32) -> Option<Vec<u8>> {
    let mut token: HANDLE = null_mut();
    // SAFETY: opens this process's own token for query; closed by `owned`.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = owned(token).ok()?;
    let mut len = 0u32;
    // SAFETY: the size query.
    unsafe { GetTokenInformation(token.as_raw_handle(), class, null_mut(), 0, &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` holds `len` bytes, 8-aligned.
    let ok =
        unsafe { GetTokenInformation(token.as_raw_handle(), class, buf.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 {
        return None;
    }
    Some(buf.iter().flat_map(|w| w.to_ne_bytes()).take(len as usize).collect())
}

/// Removes bypass-traverse checking, the one privilege the restricted token keeps, from the
/// helper's own token for good (as Chromium's lockdown token lacks it). Holding it, opening a
/// path below a device that leaves such opens to its driver, as the socket driver does
/// (`\Device\Afd\Endpoint`, what Windows Sockets opens), skips the device's access check, which
/// refuses the AppContainer. Done here, not in the token the helper starts with: Windows'
/// loader needs it to start a process in an AppContainer.
fn drop_traverse() -> Result<(), String> {
    let mut token: HANDLE = null_mut();
    // SAFETY: opens this process's own token to adjust privileges; closed by `owned`.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token) } == 0 {
        return Err(format!("the helper cannot open its token: {}", last()));
    }
    let token = owned(token).map_err(|e| e.to_string())?;
    let removed = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        // SeChangeNotifyPrivilege's fixed LUID.
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: LUID { LowPart: 23, HighPart: 0 },
            Attributes: SE_PRIVILEGE_REMOVED,
        }],
    };
    // SAFETY: a token opened with TOKEN_ADJUST_PRIVILEGES and one privilege to remove.
    let ok =
        unsafe { AdjustTokenPrivileges(token.as_raw_handle(), FALSE, &removed, 0, null_mut(), null_mut()) };
    // It succeeds without removing a privilege the token lacks (ERROR_NOT_ALL_ASSIGNED): fine.
    if ok == 0 {
        return Err(format!("the helper cannot drop bypass-traverse: {}", last()));
    }
    Ok(())
}

/// The helper was started already shut in; this checks that it was, and says how. A helper
/// not in an AppContainer refuses to run.
pub(crate) fn lock_down() -> Result<Vec<Layer>, String> {
    let in_container = token_info(TokenIsAppContainer)
        .and_then(|b| b.first_chunk::<4>().map(|c| u32::from_ne_bytes(*c)))
        .is_some_and(|v| v != 0);
    if !in_container {
        return Err("the helper is not in an AppContainer".into());
    }
    drop_traverse()?;
    let mut layers = vec![Layer::AppContainer];
    // TOKEN_PRIVILEGES starts with its count: none is left.
    let privileges = token_info(TokenPrivileges)
        .and_then(|b| b.first_chunk::<4>().map(|c| u32::from_ne_bytes(*c)))
        .unwrap_or(u32::MAX);
    if privileges == 0 {
        layers.push(Layer::RestrictedToken);
    }
    let mut in_job = FALSE;
    // SAFETY: asks whether this process is in any job.
    if unsafe { IsProcessInJob(GetCurrentProcess(), null_mut(), &mut in_job) } != 0 && in_job != 0 {
        layers.push(Layer::JobObject);
    }
    Ok(layers)
}
