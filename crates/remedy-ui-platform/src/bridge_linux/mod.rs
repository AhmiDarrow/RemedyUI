//! RemedyUI's desktop Linux bridge, for Wayland and X11 alike (winit picks the session's
//! display server at run time).
//!
//! The shared winit core (`bridge_winit`) provides windows, input, frames, input
//! methods, CPU presents and GPU surface handles. This module adds:
//! - the clipboard (Wayland data-control or X11 selections), with sensitive copies marked
//!   with the password-manager hint so clipboard managers keep them out of history;
//! - system settings from the XDG desktop portal (color scheme, accent, contrast, reduced
//!   motion) and the desktop's interface settings it exposes (text scale, double-click time,
//!   caret blink), the keyboard repeat (from the Wayland compositor, the X server, GNOME or
//!   KDE), plus the locale from the environment.
//!
//! - drag and drop with other apps: out on X11 through XDND, both ways on Wayland through
//!   the data device (winit takes file drops in on X11);
//! - the app's badge on the dock (the launcher entry API KDE, Dash to Dock and Plank read);
//! - window icons: `_NET_WM_ICON` on X11, `xdg-toplevel-icon-v1` on Wayland compositors that
//!   have it (the others show the app's desktop-entry icon for every window).
//!
//! Capture protection is not available: no Linux display server lets a window exclude itself
//! from capture, so it reports `Unsupported` rather than pretending. On other platforms this
//! module is empty.

#[cfg(any(test, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
mod env_locale;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
#[allow(unsafe_code)]
mod os;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod services;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod devices;
#[cfg_attr(
    not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))),
    allow(dead_code)
)]
mod evdev;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod gamepad;
#[cfg_attr(
    not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))),
    allow(dead_code)
)]
mod pointers;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod notify;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod repeat;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod badge;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod xtray;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod dnd;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod dnd_wayland;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod dnd_x11;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod icon_wayland;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod x11_hints;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod linux {
    use std::sync::{Arc, Mutex, PoisonError, Weak};
    use std::time::{Duration, Instant};

    use crate::bridge_winit::winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    use crate::bridge_winit::winit::window::Window;
    use crate::bridge_winit::{PlatformHooks, RunError};
    use arboard::SetExtLinux;
    use remedy_ui_core::Color;
    use remedy_ui_core::host::{
        App, DragData, DragEvent, DragStarted, DropEffect, FileDialog, HostEvent, SystemSettings, TrayIcon,
        Unsupported, Waker, WindowIcon, WindowOptions,
    };
    use remedy_ui_core::input::Platform;
    use zbus::{OwnedVariant, Variant as Value};

    /// Linux's side of the host contract.
    #[derive(Default)]
    pub struct LinuxHooks {
        // The clipboard owner must outlive what it copied: on Wayland and X11 the copying
        // process serves the data to whoever pastes.
        clipboard: Option<arboard::Clipboard>,
        tray: crate::bridge_linux::services::Tray,
        /// The app's badge on the dock's launcher entry.
        badge: crate::bridge_linux::badge::LauncherBadge,
        /// System notifications on the desktop's notification server.
        notifier: crate::bridge_linux::notify::Notifier,
        /// Drag and drop on Wayland, once a Wayland window exists.
        wayland: Option<crate::bridge_linux::dnd_wayland::Wayland>,
        /// Window icons on Wayland, once one is set: `Err` says why the compositor shows none.
        icons: Option<Result<crate::bridge_linux::icon_wayland::Icons, Unsupported>>,
        /// Pointing devices coming and going.
        devices: crate::bridge_linux::devices::DeviceWatch,
        /// The desktop's settings as last read, and the session bus they came over.
        desktop: Desktop,
    }

    impl std::fmt::Debug for LinuxHooks {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("LinuxHooks")
        }
    }

    impl LinuxHooks {
        fn clipboard(&mut self) -> Result<&mut arboard::Clipboard, Unsupported> {
            if self.clipboard.is_none() {
                self.clipboard = Some(
                    arboard::Clipboard::new()
                        .map_err(|e| Unsupported::new("the clipboard", e.to_string()))?,
                );
            }
            self.clipboard.as_mut().ok_or_else(|| Unsupported::new("the clipboard", "unavailable"))
        }
    }

    fn unwrap(v: &Value<'static>) -> Value<'static> {
        match v {
            Value::Variant(inner) => unwrap(inner),
            other => other.clone(),
        }
    }

    /// Portal settings, read in one session-bus round trip set.
    #[derive(Default)]
    struct Portal(std::collections::HashMap<Key, Value<'static>>);

    /// A portal setting's namespace and key.
    type Key = (&'static str, &'static str);

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    const KEYBOARD: &str = "org.gnome.desktop.peripherals.keyboard";

    const KEYS: [Key; 11] = [
        ("org.freedesktop.appearance", "color-scheme"),
        ("org.freedesktop.appearance", "accent-color"),
        ("org.freedesktop.appearance", "contrast"),
        ("org.freedesktop.appearance", "reduced-motion"),
        ("org.gnome.desktop.interface", "enable-animations"),
        ("org.gnome.desktop.interface", "text-scaling-factor"),
        ("org.gnome.desktop.interface", "cursor-blink"),
        ("org.gnome.desktop.interface", "cursor-blink-time"),
        ("org.gnome.desktop.peripherals.mouse", "double-click"),
        ("org.gnome.desktop.peripherals.keyboard", "delay"),
        ("org.gnome.desktop.peripherals.keyboard", "repeat-interval"),
    ];

    impl Portal {
        /// Reads every setting RemedyUI follows over `conn`. Without a portal, every value is
        /// absent and the defaults apply.
        fn read_all(conn: &zbus::Connection) -> Portal {
            let values = zbus::block_on(async {
                let mut out = std::collections::HashMap::new();
                let Ok(p) =
                    zbus::Proxy::new(conn, PORTAL, PORTAL_PATH, "org.freedesktop.portal.Settings").await
                else {
                    return out;
                };
                for key in KEYS {
                    // ReadOne is the current method; Read (wrapped one variant deeper) is what
                    // older portals have.
                    let v: Option<OwnedVariant> = match p.call("ReadOne", &key).await {
                        Ok(v) => Some(v),
                        Err(_) => p.call("Read", &key).await.ok(),
                    };
                    if let Some(v) = v {
                        out.insert(key, unwrap(&v));
                    }
                }
                out
            });
            Portal(values)
        }

        fn read(&self, namespace: &str, key: &str) -> Option<Value<'static>> {
            self.0.iter().find(|((n, k), _)| *n == namespace && *k == key).map(|(_, v)| v.clone())
        }

        fn u32(&self, ns: &str, key: &str) -> Option<u32> {
            match self.read(ns, key)? {
                Value::U32(n) => Some(n),
                _ => None,
            }
        }

        fn i32(&self, ns: &str, key: &str) -> Option<i32> {
            match self.read(ns, key)? {
                Value::I32(n) => Some(n),
                _ => None,
            }
        }

        fn bool(&self, ns: &str, key: &str) -> Option<bool> {
            match self.read(ns, key)? {
                Value::Bool(b) => Some(b),
                _ => None,
            }
        }

        fn f64(&self, ns: &str, key: &str) -> Option<f64> {
            match self.read(ns, key)? {
                Value::F64(f) => Some(f),
                _ => None,
            }
        }

        fn accent(&self) -> Option<Color> {
            let Value::Structure(s) = self.read("org.freedesktop.appearance", "accent-color")? else {
                return None;
            };
            let c: Vec<f64> = s
                .fields()
                .iter()
                .filter_map(|f| if let Value::F64(x) = f { Some(*x) } else { None })
                .collect();
            // Out-of-range components mean the user has not chosen an accent.
            match c.as_slice() {
                [r, g, b] if [r, g, b].iter().all(|v| (0.0..=1.0).contains(*v)) => {
                    Some(Color::new(*r as f32, *g as f32, *b as f32, 1.0))
                }
                _ => None,
            }
        }
    }

    /// What the portal's `SettingChanged` brought since the settings were last read.
    #[derive(Default)]
    struct Changes {
        values: Mutex<Vec<(Key, Value<'static>)>>,
        waker: Mutex<Option<Waker>>,
    }

    /// Follows the portal's `SettingChanged` over `conn` on a thread of its own, noting the
    /// settings RemedyUI follows in `changes` and waking the event loop for each. False when
    /// the thread could not start.
    fn watch(conn: zbus::Connection, changes: Weak<Changes>) -> bool {
        std::thread::Builder::new()
            .name("remedy-ui settings".into())
            .spawn(move || {
                zbus::block_on(async {
                    let Ok(p) =
                        zbus::Proxy::new(&conn, PORTAL, PORTAL_PATH, "org.freedesktop.portal.Settings").await
                    else {
                        return;
                    };
                    let Ok(mut signals) = p.receive_signal("SettingChanged").await else { return };
                    // shortcut: the thread waits for the next change after the hooks are gone,
                    // then ends; a process quitting does not wait for it.
                    while let Some(msg) = std::future::poll_fn(|cx| {
                        futures_core::Stream::poll_next(std::pin::Pin::new(&mut signals), cx)
                    })
                    .await
                    {
                        let Some(changes) = changes.upgrade() else { return };
                        let body = msg.body();
                        let Ok((namespace, key, value)) =
                            body.deserialize::<(String, String, OwnedVariant)>()
                        else {
                            continue;
                        };
                        let Some(known) = KEYS.into_iter().find(|(n, k)| *n == namespace && *k == key) else {
                            continue;
                        };
                        changes
                            .values
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push((known, unwrap(&value)));
                        let waker = changes.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
                        if let Some(w) = waker {
                            w();
                        }
                    }
                });
            })
            .is_ok()
    }

    /// How long the settings read whole stand before a window coming to the front reads them
    /// again. Changes the portal announces arrive at once; this is for what it does not
    /// announce (a desktop without the signal, the compositor's keyboard repeat).
    const REREAD: Duration = Duration::from_secs(60);

    /// The desktop's settings as last read: one session-bus connection for the life of the
    /// app, every value read once and then kept current by the portal's `SettingChanged`,
    /// so a window coming to the front costs no round trips.
    #[derive(Default)]
    struct Desktop {
        conn: Option<zbus::Connection>,
        portal: Portal,
        /// When `portal` and `repeat` were last read whole.
        read_at: Option<Instant>,
        /// The session's keyboard repeat (its own connection to the display server to ask).
        repeat: Option<(Duration, Duration)>,
        changes: Arc<Changes>,
        watching: bool,
    }

    impl Desktop {
        /// The session bus, connected once.
        fn session(&mut self) -> Option<zbus::Connection> {
            if self.conn.is_none() {
                self.conn = zbus::block_on(zbus::Connection::session()).ok();
            }
            self.conn.clone()
        }

        /// True when the portal announced a change that [`Desktop::refresh`] has not taken.
        fn changed(&self) -> bool {
            !self.changes.values.lock().unwrap_or_else(PoisonError::into_inner).is_empty()
        }

        /// Puts announced changes into the values read; true when the keyboard's changed.
        fn apply(&mut self, changed: Vec<(Key, Value<'static>)>) -> bool {
            let mut keyboard = false;
            for (key, value) in changed {
                keyboard |= key.0 == KEYBOARD;
                self.portal.0.insert(key, value);
            }
            keyboard
        }

        /// Brings the values up to date: read whole the first time and when they are older
        /// than [`REREAD`], else only what the portal announced.
        fn refresh(&mut self) {
            let changed =
                std::mem::take(&mut *self.changes.values.lock().unwrap_or_else(PoisonError::into_inner));
            let whole = self.read_at.is_none_or(|at| at.elapsed() >= REREAD);
            let keyboard = if whole {
                let conn = self.session();
                self.portal = conn.as_ref().map(Portal::read_all).unwrap_or_default();
                if let Some(conn) = conn
                    && !self.watching
                {
                    self.watching = watch(conn, Arc::downgrade(&self.changes));
                }
                self.read_at = Some(Instant::now());
                true
            } else {
                self.apply(changed)
            };
            if keyboard {
                let ms = |key| self.portal.u32(KEYBOARD, key).map(|ms| Duration::from_millis(u64::from(ms)));
                self.repeat = crate::bridge_linux::repeat::system(ms("delay").zip(ms("repeat-interval")));
            }
        }
    }

    /// Running on a battery that is discharging, or the firmware's low-power profile is on.
    fn power_saving() -> bool {
        let read = |p: std::path::PathBuf| {
            std::fs::read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_default()
        };
        let discharging = std::fs::read_dir("/sys/class/power_supply").is_ok_and(|dir| {
            dir.flatten().any(|e| {
                read(e.path().join("type")) == "Battery" && read(e.path().join("status")) == "Discharging"
            })
        });
        discharging || read("/sys/firmware/acpi/platform_profile".into()) == "low-power"
    }

    fn env(k: &str) -> Option<String> {
        std::env::var(k).ok()
    }

    impl PlatformHooks for LinuxHooks {
        fn platform(&self) -> Platform {
            Platform::Standard
        }

        fn settings(&mut self) -> SystemSettings {
            const APPEARANCE: &str = "org.freedesktop.appearance";
            const INTERFACE: &str = "org.gnome.desktop.interface";
            self.desktop.refresh();
            let p = &self.desktop.portal;
            let base = SystemSettings::default();
            let reduced_motion = p
                .u32(APPEARANCE, "reduced-motion")
                .map(|v| v == 1)
                .or_else(|| p.bool(INTERFACE, "enable-animations").map(|on| !on))
                .unwrap_or(false);
            let blink = p.bool(INTERFACE, "cursor-blink").unwrap_or(true);
            // cursor-blink-time is the whole on-off cycle.
            let cycle = p
                .i32(INTERFACE, "cursor-blink-time")
                .and_then(|ms| u64::try_from(ms).ok())
                .filter(|ms| *ms > 0);
            {
                let mut v_ = SystemSettings::default();
                v_.platform = Platform::Standard;
                v_.dark = p.u32(APPEARANCE, "color-scheme").and_then(|v| match v {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                });
                v_.accent = p.accent();
                v_.high_contrast = p.u32(APPEARANCE, "contrast") == Some(1);
                v_.text_scale = p.f64(INTERFACE, "text-scaling-factor").map_or(base.text_scale, |f| f as f32);
                v_.reduced_motion = reduced_motion;
                v_.reduced_transparency = false;
                v_.locale = crate::bridge_linux::env_locale::locale(env).unwrap_or(base.locale);
                v_.locales = crate::bridge_linux::env_locale::locales(env);
                if v_.locales.is_empty() {
                    v_.locales = vec![v_.locale.clone()];
                }
                v_.double_click = p
                    .i32("org.gnome.desktop.peripherals.mouse", "double-click")
                    .and_then(|ms| u64::try_from(ms).ok())
                    .map_or(base.double_click, Duration::from_millis);
                v_.caret_blink = if blink {
                    Some(cycle.map_or(Duration::from_millis(600), |c| Duration::from_millis(c / 2)))
                } else {
                    None
                };
                v_.power_saving = power_saving();
                v_.pointer_devices = crate::bridge_linux::devices::caps();
                (v_.key_repeat_delay, v_.key_repeat_interval) =
                    self.desktop.repeat.unwrap_or((base.key_repeat_delay, base.key_repeat_interval));
                v_
            }
        }

        fn window_options(&mut self, window: &Window, options: &WindowOptions) {
            if options.activate && options.taskbar {
                return;
            }
            let Ok(w) = window.window_handle() else { return };
            let id = match w.as_raw() {
                RawWindowHandle::Xlib(x) => u32::try_from(x.window).ok(),
                RawWindowHandle::Xcb(x) => Some(x.window.get()),
                // Wayland has no such requests: the compositor decides focus and its own lists.
                _ => None,
            };
            if let Some(id) = id {
                // Best effort, as the window manager may ignore the hints.
                let _ = crate::bridge_linux::x11_hints::quiet(id, options.activate, options.taskbar);
            }
        }

        fn window_created(&mut self, window: &Window) {
            let (Ok(d), Ok(w)) = (window.display_handle(), window.window_handle()) else { return };
            let (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) = (d.as_raw(), w.as_raw()) else {
                return;
            };
            if self.wayland.is_none() {
                // Without it, drags simply stay inside the app.
                self.wayland = crate::bridge_linux::dnd_wayland::Wayland::new(
                    crate::bridge_linux::os::wayland_connection(d.display),
                )
                .ok();
            }
            if let Some(wl) = self.wayland.as_mut()
                && let Some(surface) = crate::bridge_linux::os::wayland_surface(wl.connection(), w.surface)
            {
                wl.add_window(&surface, window.id());
            }
        }

        fn window_closed(&mut self, window: &Window) {
            if let Some(wl) = self.wayland.as_mut() {
                wl.remove_window(window.id());
            }
        }

        fn start_drag(
            &mut self,
            window: &Window,
            data: &DragData,
            allowed: &[DropEffect],
        ) -> Result<DragStarted, Unsupported> {
            let handle =
                window.window_handle().map_err(|e| Unsupported::new("dragging out", e.to_string()))?;
            match handle.as_raw() {
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => {
                    crate::bridge_linux::dnd_x11::drag(data, allowed).map(DragStarted::Finished)
                }
                RawWindowHandle::Wayland(h) => {
                    let wl = self
                        .wayland
                        .as_mut()
                        .ok_or_else(|| Unsupported::new("dragging out", "no Wayland data device"))?;
                    let surface = crate::bridge_linux::os::wayland_surface(wl.connection(), h.surface)
                        .ok_or_else(|| Unsupported::new("dragging out", "the window's surface is gone"))?;
                    wl.start(&surface, window.id(), data, allowed).map(|()| DragStarted::Pending)
                }
                _ => Err(Unsupported::new("dragging out", "not an X11 or Wayland window")),
            }
        }

        fn set_drop_effect(&mut self, window: &Window, effect: DropEffect) {
            if let Some(wl) = self.wayland.as_mut() {
                wl.set_effect(window.id(), effect);
            }
        }

        fn take_drag_events(&mut self) -> Vec<(crate::bridge_winit::winit::window::WindowId, DragEvent)> {
            self.wayland
                .as_mut()
                .map(crate::bridge_linux::dnd_wayland::Wayland::take_events)
                .unwrap_or_default()
        }

        fn take_drag_ends(&mut self) -> Vec<(crate::bridge_winit::winit::window::WindowId, DropEffect)> {
            self.wayland
                .as_mut()
                .map(crate::bridge_linux::dnd_wayland::Wayland::take_ended)
                .unwrap_or_default()
        }

        fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
            match self.clipboard()?.get_text() {
                Ok(t) => Ok(Some(t)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(e) => Err(Unsupported::new("the clipboard", e.to_string())),
            }
        }

        fn primary_selection_text(&mut self) -> Result<Option<String>, Unsupported> {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            match self.clipboard()?.get().clipboard(LinuxClipboardKind::Primary).text() {
                Ok(t) => Ok(Some(t)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(e) => Err(Unsupported::new("the primary selection", e.to_string())),
            }
        }

        fn set_primary_selection(&mut self, text: &str) -> Result<(), Unsupported> {
            use arboard::LinuxClipboardKind;
            self.clipboard()?
                .set()
                .clipboard(LinuxClipboardKind::Primary)
                .text(text.to_owned())
                .map_err(|e| Unsupported::new("the primary selection", e.to_string()))
        }

        fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
            let set = self.clipboard()?.set();
            let set = if sensitive { set.exclude_from_history() } else { set };
            set.text(text.to_owned()).map_err(|e| Unsupported::new("the clipboard", e.to_string()))
        }

        fn total_memory(&self) -> Option<u64> {
            // "MemTotal:  16303612 kB" in /proc/meminfo.
            let info = std::fs::read_to_string("/proc/meminfo").ok()?;
            let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
            let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            kb.checked_mul(1024)
        }

        fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
            match self.clipboard()?.get().html() {
                Ok(h) => Ok(Some(h)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(e) => Err(Unsupported::new("the clipboard", e.to_string())),
            }
        }

        fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
            // text/html for apps that paste formatting, and the plain text for the rest.
            self.clipboard()?
                .set()
                .html(html, Some(text))
                .map_err(|e| Unsupported::new("the clipboard", e.to_string()))
        }

        fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
            self.tray.set(tray)
        }

        fn gamepads(&mut self) -> Option<Box<dyn crate::bridge_winit::gamepad::PadSource>> {
            Some(Box::new(crate::bridge_linux::gamepad::Evdev::default()))
        }

        fn set_waker(&mut self, waker: Waker) {
            self.notifier.set_waker(waker.clone());
            self.tray.set_waker(waker.clone());
            *self.desktop.changes.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
            self.devices.set_waker(waker);
        }

        fn show_notification(
            &mut self,
            notification: &remedy_ui_core::host::Notification,
        ) -> Result<(), Unsupported> {
            self.notifier.show(notification)
        }

        fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
            self.notifier.withdraw(id)
        }

        fn set_badge(
            &mut self,
            _windows: &[&Window],
            app_id: &str,
            badge: Option<remedy_ui_core::host::Badge>,
        ) -> Result<(), Unsupported> {
            self.badge.set(app_id, badge)
        }

        fn share(
            &mut self,
            window: Option<&Window>,
            share: &remedy_ui_core::host::ShareOptions,
        ) -> Result<remedy_ui_core::host::ShareStarted, Unsupported> {
            let _ = (window, share);
            Err(Unsupported::new(
                "sharing",
                "Linux desktops have no share sheet: the desktop portals offer none, and GNOME and KDE each keep their own",
            ))
        }

        fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
            // The desktop portal opens it the user's way (and works inside sandboxes).
            let portal = match self.desktop.session() {
                Some(conn) => zbus::block_on(async {
                    let p = zbus::Proxy::new(&conn, PORTAL, PORTAL_PATH, "org.freedesktop.portal.OpenURI")
                        .await?;
                    let options: std::collections::HashMap<&str, Value<'_>> =
                        std::collections::HashMap::new();
                    let _: zbus::OwnedObjectPath = p.call("OpenURI", &("", url, options)).await?;
                    Ok::<(), zbus::Error>(())
                })
                .map_err(|e| e.to_string()),
                None => Err("no session bus".to_owned()),
            };
            let Err(e) = portal else { return Ok(()) };
            // No portal (a bare X session): the freedesktop opener.
            let mut opener = std::process::Command::new("xdg-open").arg(url).spawn().map_err(|x| {
                Unsupported::new("opening links", format!("no portal ({e}) and no xdg-open ({x})"))
            })?;
            // Waited for off this thread, so the opener leaves no zombie when it ends.
            let _ = std::thread::Builder::new().name("remedy-ui xdg-open".into()).spawn(move || {
                let _ = opener.wait();
            });
            Ok(())
        }

        fn take_events(&mut self) -> Vec<HostEvent> {
            let mut events: Vec<HostEvent> =
                self.tray.take_events().into_iter().map(HostEvent::Tray).collect();
            events.extend(self.notifier.take_events());
            // Both are asked: each takes its own note.
            if self.devices.take_changed() | self.desktop.changed() {
                events.push(HostEvent::Settings(self.settings()));
            }
            events
        }

        fn utc_offsets(&self) -> fn(i64) -> Option<i32> {
            crate::bridge_linux::os::utc_offset
        }

        fn file_dialog(
            &mut self,
            window: Option<&Window>,
            dialog: &FileDialog,
        ) -> Result<Vec<std::path::PathBuf>, Unsupported> {
            // The portal makes the dialog modal to an X11 parent; Wayland parents need an
            // exported handle (xdg-foreign), which winit does not provide, so they get none.
            let parent = window
                .and_then(|w| w.window_handle().ok())
                .and_then(|h| match h.as_raw() {
                    RawWindowHandle::Xlib(x) => Some(format!("x11:{:x}", x.window)),
                    RawWindowHandle::Xcb(x) => Some(format!("x11:{:x}", x.window.get())),
                    _ => None,
                })
                .unwrap_or_default();
            crate::bridge_linux::services::file_dialog(&parent, dialog)
        }

        fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
            self.clipboard()?.clear().map_err(|e| Unsupported::new("the clipboard", e.to_string()))
        }

        fn set_window_icon(&mut self, window: &Window, icon: Option<&WindowIcon>) -> Result<(), Unsupported> {
            use crate::bridge_winit::winit::platform::wayland::WindowExtWayland;
            let (Ok(d), Ok(w)) = (window.display_handle(), window.window_handle()) else {
                return Err(Unsupported::new("window icons", "the window has no handle"));
            };
            match (d.as_raw(), w.as_raw()) {
                (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) => {
                    let icons = self.icons.get_or_insert_with(|| {
                        crate::bridge_linux::icon_wayland::Icons::new(
                            crate::bridge_linux::os::wayland_connection(d.display),
                        )
                    });
                    let icons = icons.as_mut().map_err(|e| e.clone())?;
                    let toplevel = window
                        .xdg_toplevel()
                        .ok_or_else(|| Unsupported::new("window icons", "the window has no toplevel"))?;
                    let surface = crate::bridge_linux::os::wayland_surface(icons.connection(), w.surface)
                        .ok_or_else(|| Unsupported::new("window icons", "the window's surface is gone"))?;
                    icons.set(toplevel, &surface, icon)
                }
                // X11: `_NET_WM_ICON`, which winit sets from straight RGBA.
                _ => {
                    use crate::bridge_winit::winit::window::Icon;
                    let made = match icon {
                        Some(i) => Some(
                            Icon::from_rgba(i.straight_rgba(), i.size(), i.size())
                                .map_err(|e| Unsupported::new("window icons", e.to_string()))?,
                        ),
                        None => None,
                    };
                    window.set_window_icon(made);
                    Ok(())
                }
            }
        }

        fn set_capture_protected(&mut self, _window: &Window, _on: bool) -> Result<(), Unsupported> {
            Err(Unsupported::new(
                "capture protection",
                "Wayland and X11 offer no way for a window to exclude itself from screen capture",
            ))
        }
    }

    /// Runs `app` on the main thread's event loop until it exits.
    pub fn run(app: impl App) -> Result<(), RunError> {
        crate::bridge_winit::run(app, LinuxHooks::default())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn announced_changes_replace_the_values_read() {
            let mut d = Desktop::default();
            let scheme = ("org.freedesktop.appearance", "color-scheme");
            assert!(!d.apply(vec![(scheme, Value::U32(1))]));
            assert_eq!(d.portal.u32(scheme.0, scheme.1), Some(1));
            assert!(d.apply(vec![((KEYBOARD, "delay"), Value::U32(250))]), "the keyboard's own changed");
            assert_eq!(d.portal.u32(KEYBOARD, "delay"), Some(250));
            assert!(!d.changed(), "nothing announced is left waiting");
        }
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub use dnd::{DROP_FINISH_WAIT, DROP_READ_WAIT, set_drop_waits};
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub use linux::{LinuxHooks, run};
