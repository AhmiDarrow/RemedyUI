//! The app's badge on Linux: the launcher entry API (`com.canonical.Unity.LauncherEntry`), a
//! signal on the session bus that KDE Plasma's task manager, GNOME's Dash to Dock and Dash to
//! Panel, Plank, Latte and elementary's dock read. It names the app by its desktop entry
//! (`application://<id>.desktop`), so the badge shows on the icon that entry gives the app.

use std::collections::HashMap;

use remedy_ui_host::{Badge, Unsupported};
use zbus::Variant as Value;

const IFACE: &str = "com.canonical.Unity.LauncherEntry";

fn unsupported(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("app badges", format!("the session bus: {e}"))
}

/// The desktop entry the badge is for: the app's id, else the entry the desktop launched the
/// app from (GLib launchers say so), else the program's own name.
pub(crate) fn desktop_entry(app_id: &str) -> String {
    if !app_id.is_empty() {
        return format!("{}.desktop", app_id.trim_end_matches(".desktop"));
    }
    if let Some(file) = std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE")
        && let Some(name) = std::path::Path::new(&file).file_name()
    {
        return name.to_string_lossy().into_owned();
    }
    let program =
        std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()));
    format!("{}.desktop", program.unwrap_or_else(|| "remedy-app".into()))
}

/// The `Update` signal's properties for `badge`.
pub(crate) fn properties(badge: Option<Badge>) -> HashMap<&'static str, Value<'static>> {
    let (count, visible, urgent) = match badge {
        Some(Badge::Count(n)) => (i64::from(n), true, false),
        // No number: the dock's attention state.
        Some(Badge::Dot) => (0, false, true),
        Some(_) | None => (0, false, false),
    };
    HashMap::from([
        ("count", Value::from(count)),
        ("count-visible", Value::from(visible)),
        ("urgent", Value::from(urgent)),
    ])
}

/// Sends the launcher entry's updates.
#[derive(Default)]
pub(crate) struct LauncherBadge {
    conn: Option<zbus::Connection>,
}

impl LauncherBadge {
    #[cfg(test)]
    pub(crate) fn with_connection(conn: zbus::Connection) -> Self {
        LauncherBadge { conn: Some(conn) }
    }

    pub(crate) fn set(&mut self, app_id: &str, badge: Option<Badge>) -> Result<(), Unsupported> {
        let conn = match &self.conn {
            Some(c) => c.clone(),
            None => {
                let c = zbus::block_on(zbus::Connection::session()).map_err(unsupported)?;
                self.conn = Some(c.clone());
                c
            }
        };
        let uri = format!("application://{}", desktop_entry(app_id));
        // Any object path of the app's own; the dock matches on the URI.
        let path = format!("/com/canonical/unity/launcherentry/{}", std::process::id());
        zbus::block_on(conn.emit_signal(
            None::<&str>,
            path.as_str(),
            IFACE,
            "Update",
            &(uri.as_str(), properties(badge)),
        ))
        .map_err(unsupported)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::pin::Pin;

    use remedy_ui_host::Badge;
    use zbus::OwnedVariant;

    use super::LauncherBadge;

    pub(crate) struct Bus {
        daemon: std::process::Child,
        pub(crate) address: String,
    }

    impl Bus {
        /// A private session bus, or `None` (and a note) where `dbus-daemon` is not installed.
        pub(crate) fn start() -> Option<Bus> {
            let mut daemon = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|_| eprintln!("skipped: dbus-daemon is not installed"))
                .ok()?;
            let mut line = String::new();
            std::io::BufReader::new(daemon.stdout.as_mut()?).read_line(&mut line).ok()?;
            Some(Bus { daemon, address: line.trim().to_owned() })
        }

        pub(crate) fn connect(&self) -> zbus::Connection {
            match zbus::block_on(zbus::conn::Builder::address(self.address.as_str()).build()) {
                Ok(c) => c,
                Err(e) => unreachable!("the private bus refused: {e}"),
            }
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    pub(crate) async fn next(stream: &mut zbus::proxy::SignalStream<'_>) -> Option<zbus::Message> {
        std::future::poll_fn(|cx| futures_core::Stream::poll_next(Pin::new(&mut *stream), cx)).await
    }

    #[test]
    fn badges_reach_the_dock_as_launcher_entry_updates() {
        let Some(bus) = Bus::start() else { return };
        let app = bus.connect();
        let dock = bus.connect();
        let Some(sender) = app.unique_name().map(|n| n.to_string()) else { unreachable!("a bus name") };
        let path = format!("/com/canonical/unity/launcherentry/{}", std::process::id());
        let mut updates = match zbus::block_on(async {
            zbus::Proxy::new(&dock, sender.as_str(), path.as_str(), super::IFACE)
                .await?
                .receive_signal("Update")
                .await
        }) {
            Ok(s) => s,
            Err(e) => unreachable!("listening: {e}"),
        };
        let mut badge = LauncherBadge::with_connection(app);
        let mut heard = |b: Option<Badge>| {
            assert!(badge.set("com.example.hearth", b).is_ok());
            let Some(msg) = zbus::block_on(next(&mut updates)) else { unreachable!("an update") };
            let body = msg.body();
            let Ok((uri, props)) = body.deserialize::<(String, HashMap<String, OwnedVariant>)>() else {
                unreachable!("the body is (s, a{{sv}})")
            };
            assert_eq!(uri, "application://com.example.hearth.desktop");
            let get = |k: &str| props.get(k).map(|v| format!("{:?}", **v)).unwrap_or_default();
            (get("count"), get("count-visible"), get("urgent"))
        };
        let (count, visible, urgent) = heard(Some(Badge::Count(4)));
        assert!(
            count.contains('4') && visible.contains("true") && urgent.contains("false"),
            "{count} {visible} {urgent}"
        );
        let (_, visible, urgent) = heard(Some(Badge::Dot));
        assert!(visible.contains("false") && urgent.contains("true"));
        let (_, visible, urgent) = heard(None);
        assert!(visible.contains("false") && urgent.contains("false"));
    }

    #[test]
    fn the_entry_falls_back_to_the_launcher_or_the_program() {
        assert_eq!(super::desktop_entry("com.example.hearth"), "com.example.hearth.desktop");
        assert_eq!(super::desktop_entry("hearth.desktop"), "hearth.desktop");
        assert!(super::desktop_entry("").ends_with(".desktop"));
    }
}
