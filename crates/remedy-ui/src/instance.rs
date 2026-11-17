//! Single instance: the first launch of an app keeps running; later launches hand their
//! arguments to it and exit.

use std::sync::{Arc, Mutex};

use remedy_ui_host::Waker;

/// Launch arguments waiting for the runner, and the waker that tells it they arrived.
#[derive(Default)]
pub(crate) struct Inbox {
    pub(crate) launches: Vec<Vec<String>>,
    pub(crate) waker: Option<Waker>,
}

pub(crate) type SharedInbox = Arc<Mutex<Inbox>>;

pub(crate) fn deliver(inbox: &SharedInbox, args: Vec<String>) {
    let waker = {
        let mut i = inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        i.launches.push(args);
        i.waker.clone()
    };
    if let Some(w) = waker {
        w();
    }
}

/// Claims the instance. `Ok(true)`: run; `Ok(false)`: another instance has these arguments.
pub(crate) fn claim(app: &str, args: &[String], inbox: SharedInbox) -> Result<bool, String> {
    #[cfg(windows)]
    {
        remedy_ui_bridge_windows::claim_single_instance(app, args, move |a| deliver(&inbox, a))
            .map_err(|e| e.to_string())
    }
    #[cfg(unix)]
    {
        unix::claim(app, args, inbox)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (app, args, inbox);
        Err("single instance is not available on this platform yet".into())
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    use super::{SharedInbox, deliver};

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

    fn send(path: &PathBuf, args: &[String]) -> std::io::Result<()> {
        let mut s = UnixStream::connect(path)?;
        s.write_all(args.join("\0").as_bytes())
    }

    pub(super) fn claim(app: &str, args: &[String], inbox: SharedInbox) -> Result<bool, String> {
        let path = socket(app);
        if send(&path, args).is_ok() {
            return Ok(false);
        }
        // Nobody listens: a stale socket from a crashed run, or none at all.
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            // Another first launch won the race; it is the instance now.
            Err(_) => return send(&path, args).map(|()| false).map_err(|e| e.to_string()),
        };
        std::thread::Builder::new()
            .name("remedy-ui single instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut s = stream;
                    let mut buf = Vec::new();
                    if s.read_to_end(&mut buf).is_ok() {
                        let text = String::from_utf8_lossy(&buf);
                        let launch = if text.is_empty() {
                            Vec::new()
                        } else {
                            text.split('\0').map(str::to_owned).collect()
                        };
                        deliver(&inbox, launch);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(true)
    }
}
