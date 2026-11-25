//! Linux: sealed memfds for shared memory, an address-space limit set by the app for each
//! request, and the helper's sandbox: a fresh network namespace where allowed, Landlock with
//! no file system access, and a seccomp-bpf allow-list.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

use super::unix::{common_limits, owned_fd};
use crate::media_isolated::Layer;

/// RLIMIT_AS, set from the app, makes allocations past the limit fail in the helper.
pub(crate) const LIMITS_MEMORY: bool = true;

pub(super) fn shared_fd(len: usize) -> io::Result<OwnedFd> {
    // SAFETY: the name is a NUL-terminated literal.
    let fd = owned_fd(unsafe {
        libc::memfd_create(c"remedy-image".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
    })?;
    let len = libc::off_t::try_from(len).map_err(|_| io::Error::other("region too large"))?;
    // SAFETY: sizes our own memfd.
    if unsafe { libc::ftruncate(fd.as_raw_fd(), len) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Neither side can shrink (a mapping past the end would fault in the app) or grow it.
    let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    // SAFETY: adds seals to our own memfd.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, seals) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Its address space, in bytes (what RLIMIT_AS limits).
pub(super) fn memory_of(pid: libc::pid_t) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = statm.split_whitespace().next()?.parse().ok()?;
    // SAFETY: a pure query.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages.saturating_mul(u64::try_from(page).ok()?))
}

/// Its user and system time, from `/proc/<pid>/stat` (fields 14 and 15, in clock ticks).
pub(super) fn cpu_time_of(pid: libc::pid_t) -> Option<std::time::Duration> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) may hold spaces; the fields after it follow its ')'.
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    // SAFETY: a pure query.
    let hz = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).ok().filter(|&h| h > 0)?;
    let ticks = user.saturating_add(system);
    Some(std::time::Duration::from_nanos(ticks.saturating_mul(1_000_000_000) / hz))
}

pub(super) fn limit_memory(pid: libc::pid_t, bytes: u64) -> io::Result<()> {
    let mut old = libc::rlimit64 { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: reads our child's limit.
    if unsafe { libc::prlimit64(pid, libc::RLIMIT_AS, std::ptr::null(), &mut old) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Only the soft limit moves (the helper cannot raise it: seccomp stops setrlimit).
    let new = libc::rlimit64 { rlim_cur: bytes.min(old.rlim_max), rlim_max: old.rlim_max };
    // SAFETY: sets our child's limit.
    if unsafe { libc::prlimit64(pid, libc::RLIMIT_AS, &new, std::ptr::null_mut()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn lock_down() -> Result<Vec<Layer>, String> {
    common_limits()?;
    let mut layers = Vec::new();
    // A new user namespace with an empty network namespace: no interfaces at all. Some
    // systems forbid unprivileged user namespaces; seccomp still allows no sockets then.
    // SAFETY: the helper is single-threaded here (helper_main runs first in main).
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } == 0 {
        layers.push(Layer::NetworkNamespace);
    }
    // SAFETY: required before Landlock and seccomp without privileges; it only ever
    // removes abilities.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!("no_new_privs: {}", io::Error::last_os_error()));
    }
    if let Some(abi) = landlock()? {
        layers.push(Layer::Landlock { abi });
    }
    seccomp()?;
    layers.push(Layer::Seccomp);
    layers.push(Layer::ResourceLimits);
    Ok(layers)
}

// ---------- Landlock ----------

const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;
const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

/// Every file system access the kernel's Landlock knows, with no rule allowing any; from ABI
/// 4 TCP bind and connect too, from ABI 6 abstract Unix sockets and signals outside the
/// helper. `None` when the kernel has no Landlock.
fn landlock() -> Result<Option<u32>, String> {
    // SAFETY: the version query takes no attribute.
    let abi = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < 1 {
        return Ok(None);
    }
    let abi = u32::try_from(abi).unwrap_or(u32::MAX);
    let fs_bits = match abi {
        1 => 13,
        2 => 14,
        3 | 4 => 15,
        _ => 16,
    };
    let attr = RulesetAttr {
        handled_access_fs: (1u64 << fs_bits) - 1,
        handled_access_net: if abi >= 4 { 0b11 } else { 0 },
        scoped: if abi >= 6 { 0b11 } else { 0 },
    };
    let size: usize = if abi >= 6 {
        24
    } else if abi >= 4 {
        16
    } else {
        8
    };
    // SAFETY: `attr` is valid for `size` bytes (a prefix of the struct the ABI knows).
    let fd = unsafe { libc::syscall(SYS_LANDLOCK_CREATE_RULESET, &raw const attr, size, 0u32) };
    let fd = owned_fd(fd as libc::c_int).map_err(|e| format!("landlock ruleset: {e}"))?;
    // SAFETY: restricts this (single-threaded) process with the ruleset just made.
    if unsafe { libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, fd.as_raw_fd(), 0u32) } != 0 {
        return Err(format!("landlock: {}", io::Error::last_os_error()));
    }
    Ok(Some(abi))
}

// ---------- seccomp-bpf ----------

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xC000_003E;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xC000_00B7;

const RET_KILL: u32 = 0x8000_0000; // SECCOMP_RET_KILL_PROCESS
const RET_ALLOW: u32 = 0x7FFF_0000;
const RET_EPERM: u32 = 0x0005_0000 | libc::EPERM as u32; // SECCOMP_RET_ERRNO

const LD_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
const JEQ: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
const JGE: u16 = 0x35; // BPF_JMP | BPF_JGE | BPF_K
const JSET: u16 = 0x45; // BPF_JMP | BPF_JSET | BPF_K
const RET: u16 = 0x06; // BPF_RET | BPF_K

// Offsets in struct seccomp_data.
const NR: u32 = 0;
const ARCH: u32 = 4;
const fn arg(i: u32) -> u32 {
    16 + 8 * i
}

fn op(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt: 0, jf: 0, k }
}
fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

/// What a decoder needs: reading and writing the channel and stderr, memory (never
/// executable), futexes, time, randomness, signals to itself (abort), and exiting.
fn allowed() -> Vec<libc::c_long> {
    use libc::*;
    let mut v = vec![
        SYS_read,
        SYS_write,
        SYS_readv,
        SYS_writev,
        SYS_recvfrom,
        SYS_recvmsg,
        SYS_sendto,
        SYS_sendmsg,
        SYS_close,
        SYS_munmap,
        SYS_mremap,
        SYS_madvise,
        SYS_brk,
        SYS_futex,
        SYS_exit,
        SYS_exit_group,
        SYS_rt_sigreturn,
        SYS_rt_sigprocmask,
        SYS_rt_sigaction,
        SYS_sigaltstack,
        SYS_getpid,
        SYS_gettid,
        SYS_clock_gettime,
        SYS_clock_nanosleep,
        SYS_nanosleep,
        SYS_sched_yield,
        SYS_getrandom,
        SYS_restart_syscall,
    ];
    #[cfg(target_arch = "x86_64")]
    v.push(SYS_arch_prctl);
    v
}

/// Calls that only an attack makes: the process is killed on the spot.
fn fatal() -> Vec<libc::c_long> {
    use libc::*;
    let mut v = vec![
        SYS_execve,
        SYS_execveat,
        SYS_clone,
        SYS_clone3,
        SYS_ptrace,
        SYS_process_vm_readv,
        SYS_process_vm_writev,
        SYS_setrlimit,
        SYS_prlimit64,
        SYS_unshare,
        SYS_setns,
        SYS_mount,
        SYS_kill,
        SYS_tkill,
        SYS_bpf,
        SYS_perf_event_open,
        SYS_keyctl,
        SYS_add_key,
        SYS_userfaultfd,
        SYS_io_uring_setup,
        SYS_seccomp,
        SYS_prctl,
        SYS_pidfd_open,
        SYS_pidfd_getfd,
    ];
    #[cfg(target_arch = "x86_64")]
    v.extend([SYS_fork, SYS_vfork]);
    v
}

/// The filter: allowed calls allowed, `mmap`/`mprotect` only without PROT_EXEC, `tgkill`
/// only to this process, attack calls fatal, everything else (opening files, sockets,
/// any call not listed) failing with EPERM.
fn program() -> Vec<libc::sock_filter> {
    let mut p = vec![op(LD_ABS, ARCH), jump(JEQ, AUDIT_ARCH, 1, 0), op(RET, RET_KILL), op(LD_ABS, NR)];
    #[cfg(target_arch = "x86_64")]
    {
        // The x32 ABI's numbers would slip past the lists.
        p.push(jump(JGE, 0x4000_0000, 0, 1));
        p.push(op(RET, RET_KILL));
    }
    for nr in [libc::SYS_mmap, libc::SYS_mprotect] {
        p.extend([
            jump(JEQ, nr as u32, 0, 4),
            op(LD_ABS, arg(2)),
            jump(JSET, libc::PROT_EXEC as u32, 0, 1),
            op(RET, RET_EPERM),
            op(RET, RET_ALLOW),
        ]);
    }
    // SAFETY: a pure query.
    let me = unsafe { libc::getpid() } as u32;
    p.extend([
        jump(JEQ, libc::SYS_tgkill as u32, 0, 4),
        op(LD_ABS, arg(0)),
        jump(JEQ, me, 1, 0),
        op(RET, RET_KILL),
        op(RET, RET_ALLOW),
    ]);
    for nr in allowed() {
        p.extend([jump(JEQ, nr as u32, 0, 1), op(RET, RET_ALLOW)]);
    }
    for nr in fatal() {
        p.extend([jump(JEQ, nr as u32, 0, 1), op(RET, RET_KILL)]);
    }
    p.push(op(RET, RET_EPERM));
    p
}

fn seccomp() -> Result<(), String> {
    let mut filter = program();
    let prog = libc::sock_fprog {
        len: u16::try_from(filter.len()).map_err(|_| "seccomp program too long".to_owned())?,
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: `prog` points at `filter`, alive for the call; no_new_privs is set.
    let r = unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &raw const prog, 0, 0) };
    if r != 0 {
        return Err(format!("seccomp-bpf is not available: {}", io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Each layer on its own. Landlock, no_new_privs and a seccomp filter without TSYNC bind
    //! only the thread that applies them, so each is applied alone in a thread of its own
    //! and the rest of the test run is untouched.

    use super::*;

    fn in_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        std::thread::spawn(f).join().ok()
    }

    fn no_new_privs() {
        // SAFETY: sets the attribute on this thread only.
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    }

    fn file() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "remedy-layer-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::write(&p, "private");
        p
    }

    #[test]
    fn landlock_alone_denies_reading_writing_and_listing() {
        let p = file();
        let (p2, dir) = (p.clone(), std::env::temp_dir());
        let r = in_thread(move || {
            no_new_privs();
            let abi = landlock().ok().flatten();
            let read = std::fs::read(&p2).map_err(|e| e.kind());
            let write = std::fs::write(dir.join("landlock-escape"), "x").map_err(|e| e.kind());
            let list = std::fs::read_dir(&dir).map(|_| ()).map_err(|e| e.kind());
            (abi, read.map(|_| ()), write, list)
        });
        let _ = std::fs::remove_file(&p);
        let Some((abi, read, write, list)) = r else { unreachable!("the thread failed") };
        let Some(abi) = abi else {
            println!("this kernel has no Landlock; nothing to prove");
            return;
        };
        println!("Landlock ABI {abi}: read {read:?}, write {write:?}, list {list:?}");
        for r in [read, write, list] {
            assert_eq!(r, Err(io::ErrorKind::PermissionDenied));
        }
        assert!(!std::env::temp_dir().join("landlock-escape").exists());
        // The rest of the process is untouched.
        assert!(std::fs::read_dir(std::env::temp_dir()).is_ok());
    }

    #[test]
    fn seccomp_alone_denies_files_sockets_and_executable_memory() {
        let p = file();
        let p2 = p.clone();
        let r = in_thread(move || {
            no_new_privs();
            let installed = seccomp();
            let read = std::fs::read(&p2).map(|_| ()).map_err(|e| e.raw_os_error());
            let tcp = std::net::TcpStream::connect("127.0.0.1:9").map(|_| ()).map_err(|e| e.raw_os_error());
            let udp = std::net::UdpSocket::bind("127.0.0.1:0").map(|_| ()).map_err(|e| e.raw_os_error());
            // SAFETY: asks for one executable anonymous page (and unmaps it if granted).
            let exec = unsafe {
                let p = libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_EXEC,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                );
                if p == libc::MAP_FAILED {
                    Err(io::Error::last_os_error().raw_os_error())
                } else {
                    libc::munmap(p, 4096);
                    Ok(())
                }
            };
            (installed, read, tcp, udp, exec)
        });
        let _ = std::fs::remove_file(&p);
        let Some((installed, read, tcp, udp, exec)) = r else { unreachable!("the thread failed") };
        assert_eq!(installed, Ok(()));
        println!("seccomp: read {read:?}, tcp {tcp:?}, udp {udp:?}, exec memory {exec:?}");
        for r in [read, tcp, udp, exec] {
            assert_eq!(r, Err(Some(libc::EPERM)));
        }
    }
}
