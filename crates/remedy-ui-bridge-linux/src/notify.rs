//! System notifications on Linux: the desktop's notification server
//! (`org.freedesktop.Notifications` on the session bus), which GNOME, KDE, XFCE, Cinnamon,
//! mako, dunst and the rest provide.
//!
//! What the person does comes back as the server's `ActionInvoked` (a button, or `default`
//! for the notification itself) and `NotificationClosed` signals, heard in order on a thread
//! of their own and passed to the event loop.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_host::{HostEvent, Notification, NotificationResponse, Unsupported, Urgency, Waker};
use zbus::{Structure, Variant as Value};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
/// The action key servers report when the notification itself is pressed.
const DEFAULT_ACTION: &str = "default";
/// `NotificationClosed`'s reason for the person dismissing it.
const DISMISSED_BY_USER: u32 = 2;

#[derive(Default)]
struct Shared {
    /// The server's number for each of the app's notifications, and back.
    by_app: HashMap<u64, u32>,
    by_server: HashMap<u32, u64>,
    events: Vec<HostEvent>,
    waker: Option<Waker>,
}

fn lock(s: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The app's notifications on the desktop's notification server.
#[derive(Default)]
pub(crate) struct Notifier {
    shared: Arc<Mutex<Shared>>,
    conn: Option<zbus::Connection>,
}

fn unsupported(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("system notifications", format!("the desktop's notification server: {e}"))
}

/// The `image-data` hint: width, height, row stride, alpha, bits per sample, channels and
/// straight (not premultiplied) RGBA bytes.
fn image_data(icon: &remedy_ui_host::TrayImage) -> Option<Value<'static>> {
    let size = i32::try_from(icon.size).ok()?;
    if icon.rgba.len() != (icon.size as usize).checked_mul(icon.size as usize)?.checked_mul(4)? {
        return None;
    }
    let straight: Vec<u8> = icon
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8
                }
            };
            [un(r), un(g), un(b), a]
        })
        .collect();
    Some(Value::Structure(Structure::from((size, size, size * 4, true, 8i32, 4i32, straight))))
}

impl Notifier {
    pub(crate) fn set_waker(&mut self, waker: Waker) {
        lock(&self.shared).waker = Some(waker);
    }

    pub(crate) fn take_events(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut lock(&self.shared).events)
    }

    /// The session bus, and the threads hearing the server's signals, on first use.
    fn connection(&mut self) -> Result<zbus::Connection, Unsupported> {
        if let Some(c) = &self.conn {
            return Ok(c.clone());
        }
        let conn = zbus::block_on(zbus::Connection::session()).map_err(unsupported)?;
        self.listen(conn)
    }

    /// Uses `conn` (a test's private bus).
    #[cfg(test)]
    pub(crate) fn with_connection(conn: zbus::Connection) -> Result<Self, Unsupported> {
        let mut n = Notifier::default();
        let _conn = n.listen(conn)?;
        Ok(n)
    }

    /// Hears the server's answers on `conn` from now on.
    fn listen(&mut self, conn: zbus::Connection) -> Result<zbus::Connection, Unsupported> {
        // One stream for both signals keeps them in the order the server sent them: a press
        // and the close that follows it arrive as press, then close.
        let rule = format!("type='signal',interface='{IFACE}',path='{PATH}'");
        let mut signals = zbus::block_on(zbus::MessageStream::for_match_rule(rule.as_str(), &conn, None))
            .map_err(unsupported)?;
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("remedy-notify".into())
            .spawn(move || {
                zbus::block_on(async {
                    loop {
                        let next = std::future::poll_fn(|cx| {
                            futures_core::Stream::poll_next(Pin::new(&mut signals), cx)
                        })
                        .await;
                        let Some(Ok(msg)) = next else { break };
                        let header = msg.header();
                        let member = header.member().map(|m| m.as_str().to_owned()).unwrap_or_default();
                        let body = msg.body();
                        let event = match member.as_str() {
                            "ActionInvoked" => {
                                let Ok((server, key)) = body.deserialize::<(u32, String)>() else { continue };
                                (
                                    server,
                                    if key == DEFAULT_ACTION {
                                        NotificationResponse::Activated
                                    } else {
                                        NotificationResponse::Action(key)
                                    },
                                )
                            }
                            "NotificationClosed" => {
                                let Ok((server, reason)) = body.deserialize::<(u32, u32)>() else { continue };
                                if reason != DISMISSED_BY_USER {
                                    // Expired (still in the desktop's history) or withdrawn by the
                                    // app: nothing the person did.
                                    let mut g = lock(&shared);
                                    if reason != 1
                                        && let Some(app) = g.by_server.remove(&server)
                                    {
                                        g.by_app.remove(&app);
                                    }
                                    continue;
                                }
                                (server, NotificationResponse::Dismissed)
                            }
                            _ => continue,
                        };
                        let waker = {
                            let mut g = lock(&shared);
                            // Signals are broadcast: only this app's notifications count.
                            let Some(&app) = g.by_server.get(&event.0) else { continue };
                            if event.1 == NotificationResponse::Dismissed {
                                g.by_server.remove(&event.0);
                                g.by_app.remove(&app);
                            }
                            g.events.push(HostEvent::Notification(app, event.1));
                            g.waker.clone()
                        };
                        if let Some(w) = waker {
                            w();
                        }
                    }
                });
            })
            .map_err(unsupported)?;
        self.conn = Some(conn.clone());
        Ok(conn)
    }

    pub(crate) fn show(&mut self, n: &Notification) -> Result<(), Unsupported> {
        let conn = self.connection()?;
        let replaces = lock(&self.shared).by_app.get(&n.id).copied().unwrap_or(0);
        let mut actions: Vec<&str> = vec![DEFAULT_ACTION, ""];
        for a in &n.actions {
            if a.id == DEFAULT_ACTION {
                continue;
            }
            actions.push(&a.id);
            actions.push(&a.label);
        }
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        if n.silent {
            hints.insert("suppress-sound", Value::from(true));
        }
        if let Some(img) = n.icon.as_ref().and_then(image_data) {
            hints.insert("image-data", img);
        }
        let (urgency, app_icon) = urgency(n.urgency);
        hints.insert("urgency", Value::from(urgency));
        // The server's default timeout (-1).
        let server: u32 = zbus::block_on(async {
            let proxy = zbus::Proxy::new(&conn, DEST, PATH, IFACE).await?;
            proxy
                .call(
                    "Notify",
                    &(
                        n.app.as_str(),
                        replaces,
                        app_icon,
                        n.title.as_str(),
                        n.body.as_str(),
                        actions,
                        hints,
                        -1i32,
                    ),
                )
                .await
        })
        .map_err(unsupported)?;
        let mut g = lock(&self.shared);
        if let Some(old) = g.by_app.insert(n.id, server)
            && old != server
        {
            g.by_server.remove(&old);
        }
        g.by_server.insert(server, n.id);
        Ok(())
    }

    pub(crate) fn withdraw(&mut self, id: u64) -> Result<(), Unsupported> {
        let Some(server) = lock(&self.shared).by_app.remove(&id) else { return Ok(()) };
        lock(&self.shared).by_server.remove(&server);
        let conn = self.connection()?;
        zbus::block_on(async {
            let proxy = zbus::Proxy::new(&conn, DEST, PATH, IFACE).await?;
            proxy.call::<_, _, ()>("CloseNotification", &(server,)).await
        })
        .map_err(unsupported)
    }
}

/// The spec's urgency byte (1 normal, 2 critical) and the stock icon a warning or error shows
/// when the app gives no picture of its own.
fn urgency(u: Urgency) -> (u8, &'static str) {
    match u {
        Urgency::Warning => (1, "dialog-warning"),
        Urgency::Error => (2, "dialog-error"),
        _ => (1, ""),
    }
}
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use remedy_ui_host::{HostEvent, Notification, NotificationResponse, TrayImage};
    use zbus::object_server::SignalEmitter;
    use zbus::{OwnedVariant, interface};

    use super::{DEST, Notifier, PATH, urgency};
    use crate::badge::tests::Bus;

    /// What the stand-in server was asked: (replaces, summary, body, actions, hint names).
    type Shown = (u32, String, String, Vec<String>, Vec<String>);

    #[derive(Clone, Default)]
    struct Server {
        shown: Arc<Mutex<Vec<Shown>>>,
        closed: Arc<Mutex<Vec<u32>>>,
    }

    #[interface(name = "org.freedesktop.Notifications")]
    impl Server {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            _app: String,
            replaces: u32,
            _icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedVariant>,
            _timeout: i32,
        ) -> u32 {
            let mut names: Vec<String> = hints.into_keys().collect();
            names.sort();
            let mut s = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
            s.push((replaces, summary, body, actions, names));
            if replaces != 0 { replaces } else { 40 + s.len() as u32 }
        }

        fn close_notification(&self, id: u32) {
            self.closed.lock().unwrap_or_else(PoisonError::into_inner).push(id);
        }

        #[zbus(signal)]
        async fn action_invoked(e: &SignalEmitter<'_>, id: u32, key: &str) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn notification_closed(e: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;
    }

    fn events(n: &mut Notifier, want: usize) -> Vec<HostEvent> {
        let start = Instant::now();
        let mut out = Vec::new();
        while out.len() < want && start.elapsed() < Duration::from_secs(5) {
            out.extend(n.take_events());
            std::thread::sleep(Duration::from_millis(10));
        }
        out
    }

    #[test]
    fn warnings_and_errors_carry_the_spec_urgency_and_a_stock_icon() {
        use remedy_ui_host::Urgency;
        assert_eq!(urgency(Urgency::Info), (1, ""));
        assert_eq!(urgency(Urgency::Warning), (1, "dialog-warning"));
        assert_eq!(urgency(Urgency::Error), (2, "dialog-error"), "critical");
    }

    #[test]
    fn notifications_go_to_the_server_and_answers_come_back() {
        let Some(bus) = Bus::start() else { return };
        let server = Server::default();
        let Ok(served) = zbus::block_on(
            zbus::conn::Builder::address(bus.address.as_str())
                .name(DEST)
                .serve_at(PATH, server.clone())
                .build(),
        ) else {
            unreachable!("serving the stand-in")
        };
        let Ok(mut n) = Notifier::with_connection(bus.connect()) else { unreachable!("a notifier") };
        let icon =
            TrayImage { size: 2, rgba: vec![128, 0, 0, 128, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255] };
        let note = Notification::new(7, "Break time", "Stand up and stretch")
            .action("snooze", "Snooze")
            .icon(icon)
            .silent(true);
        assert!(n.show(&note).is_ok());
        {
            let shown = server.shown.lock().unwrap_or_else(PoisonError::into_inner);
            let Some((replaces, summary, body, actions, hints)) = shown.first() else {
                unreachable!("shown")
            };
            assert_eq!(
                (*replaces, summary.as_str(), body.as_str()),
                (0, "Break time", "Stand up and stretch")
            );
            assert_eq!(
                actions,
                &["default", "", "snooze", "Snooze"],
                "pressing it reports back as `default`"
            );
            assert_eq!(hints, &["image-data", "suppress-sound", "urgency"]);
        }
        // Showing the same id again replaces it on the server.
        assert!(n.show(&Notification::new(7, "Break time", "Now")).is_ok());
        assert_eq!(server.shown.lock().unwrap_or_else(PoisonError::into_inner).get(1).map(|s| s.0), Some(41));

        let emit = |key: Option<&str>, id: u32| {
            zbus::block_on(async {
                let i = served.object_server().interface::<_, Server>(PATH).await?;
                match key {
                    Some(k) => Server::action_invoked(i.signal_emitter(), id, k).await,
                    None => Server::notification_closed(i.signal_emitter(), id, 2).await,
                }
            })
        };
        // Another app's notification is not this app's business.
        assert!(emit(Some("default"), 99).is_ok());
        assert!(emit(Some("default"), 41).is_ok());
        assert!(emit(Some("snooze"), 41).is_ok());
        assert!(emit(None, 41).is_ok());
        assert_eq!(
            events(&mut n, 3),
            vec![
                HostEvent::Notification(7, NotificationResponse::Activated),
                HostEvent::Notification(7, NotificationResponse::Action("snooze".into())),
                HostEvent::Notification(7, NotificationResponse::Dismissed),
            ]
        );

        assert!(n.show(&Notification::new(8, "Saved", "")).is_ok());
        assert!(n.withdraw(8).is_ok());
        assert_eq!(*server.closed.lock().unwrap_or_else(PoisonError::into_inner), vec![43]);
        assert!(n.withdraw(8).is_ok(), "withdrawing twice is not an error");
    }
}
