//! System notifications on Linux: the desktop's notification server
//! (`org.freedesktop.Notifications` on the session bus), which GNOME, KDE, XFCE, Cinnamon,
//! mako, dunst and the rest provide.
//!
//! What the person does comes back as the server's `ActionInvoked` (a button, or `default`
//! for the notification itself) and `NotificationClosed` signals, heard on a thread of their
//! own and passed to the event loop.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_host::{HostEvent, Notification, NotificationResponse, Unsupported, Waker};
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
        for member in ["ActionInvoked", "NotificationClosed"] {
            let (conn, shared) = (conn.clone(), self.shared.clone());
            std::thread::Builder::new()
                .name(format!("remedy-notify-{member}"))
                .spawn(move || {
                    let heard = zbus::block_on(async {
                        let proxy = zbus::Proxy::new(&conn, DEST, PATH, IFACE).await?;
                        let mut signals = proxy.receive_signal(member).await?;
                        loop {
                            let next = std::future::poll_fn(|cx| {
                                futures_core::Stream::poll_next(Pin::new(&mut signals), cx)
                            })
                            .await;
                            let Some(msg) = next else { break };
                            let body = msg.body();
                            let event = if member == "ActionInvoked" {
                                let Ok((server, key)) = body.deserialize::<(u32, String)>() else { continue };
                                (
                                    server,
                                    if key == DEFAULT_ACTION {
                                        NotificationResponse::Activated
                                    } else {
                                        NotificationResponse::Action(key)
                                    },
                                )
                            } else {
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
                        Ok::<(), zbus::Error>(())
                    });
                    if let Err(e) = heard {
                        eprintln!("RemedyUI: notification answers stopped: {e}");
                    }
                })
                .map_err(unsupported)?;
        }
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
        // The server's default timeout (-1).
        let server: u32 = zbus::block_on(async {
            let proxy = zbus::Proxy::new(&conn, DEST, PATH, IFACE).await?;
            proxy
                .call(
                    "Notify",
                    &(n.app.as_str(), replaces, "", n.title.as_str(), n.body.as_str(), actions, hints, -1i32),
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
