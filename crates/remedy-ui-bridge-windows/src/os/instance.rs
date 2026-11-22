//! Single instance: a session-local named mutex says whether the app already runs, and a
//! named pipe carries a second launch's arguments to it.

use std::time::{Duration, Instant};

use remedy_ui_host::Unsupported;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_WRITE, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, OPEN_EXISTING, PIPE_ACCESS_INBOUND, ReadFile, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_WAIT, WaitNamedPipeW,
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

/// Claims the single instance for `app_id` in this user session. Returns `true` when this is
/// the first instance (`on_launch` then receives each later launch's arguments, on a
/// background thread), or `false` after handing `args` to the running instance.
pub(crate) fn claim(
    app_id: &str,
    args: &[String],
    on_launch: impl Fn(Vec<String>) + Send + 'static,
) -> Result<bool, Unsupported> {
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
        send(&pipe_name, args)?;
        return Ok(false);
    }
    // The mutex handle stays open for the life of the process: it is the claim.
    let pipe = wide(&pipe_name);
    std::thread::Builder::new()
        .name("remedy-ui single instance".into())
        .spawn(move || {
            loop {
                match receive(&pipe) {
                    Some(launch) => on_launch(launch),
                    None => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        })
        .map_err(|e| Unsupported::new("single instance", e.to_string()))?;
    Ok(true)
}

/// Waits for one launch on the pipe and returns its arguments.
fn receive(pipe: &[u16]) -> Option<Vec<String>> {
    // SAFETY: one inbound, local-only pipe instance; connected, read to the end, closed.
    unsafe {
        let h: HANDLE = CreateNamedPipeW(
            pipe.as_ptr(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            64 * 1024,
            0,
            std::ptr::null(),
        );
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let connected =
            ConnectNamedPipe(h, std::ptr::null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED;
        let mut bytes = Vec::new();
        if connected {
            let mut buf = [0u8; 4096];
            loop {
                let mut n = 0u32;
                if ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &raw mut n, std::ptr::null_mut()) == 0
                    || n == 0
                {
                    break;
                }
                bytes.extend_from_slice(buf.get(..n as usize).unwrap_or_default());
            }
        }
        DisconnectNamedPipe(h);
        CloseHandle(h);
        connected.then(|| decode(&bytes))
    }
}

fn decode(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() { Vec::new() } else { text.split('\0').map(str::to_owned).collect() }
}

fn send(pipe_name: &str, args: &[String]) -> Result<(), Unsupported> {
    let pipe = wide(pipe_name);
    let payload = args.join("\0");
    let start = Instant::now();
    loop {
        // SAFETY: opening the pipe for writing; the handle is checked and closed.
        let h = unsafe {
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
        if h != INVALID_HANDLE_VALUE {
            let mut written = 0u32;
            // SAFETY: writing our bytes to the open handle, then closing it.
            let ok = unsafe {
                let ok = WriteFile(
                    h,
                    payload.as_ptr(),
                    payload.len() as u32,
                    &raw mut written,
                    std::ptr::null_mut(),
                ) != 0;
                CloseHandle(h);
                ok
            };
            return if ok {
                Ok(())
            } else {
                Err(Unsupported::new("single instance", std::io::Error::last_os_error().to_string()))
            };
        }
        if start.elapsed() > *WAIT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            return Err(Unsupported::new("single instance", "the running instance did not answer"));
        }
        // SAFETY: waiting for a free pipe instance, or briefly for the first to create one.
        if unsafe { GetLastError() } == ERROR_PIPE_BUSY {
            unsafe { WaitNamedPipeW(pipe.as_ptr(), 1000) };
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_waits_as_long_as_the_app_says() {
        set_instance_wait(Duration::from_millis(100));
        let start = Instant::now();
        let sent = send(r"\\.\pipe\remedy-ui-test-no-such-instance", &["a".into()]);
        set_instance_wait(INSTANCE_WAIT);
        assert!(sent.is_err());
        assert!(start.elapsed() < INSTANCE_WAIT, "gave up after the app's wait: {:?}", start.elapsed());
    }
}
