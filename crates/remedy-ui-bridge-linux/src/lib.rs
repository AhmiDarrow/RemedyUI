//! RemedyUI's desktop Linux bridge, for Wayland and X11 alike (winit picks the session's
//! display server at run time).
//!
//! The shared winit core (`remedy-ui-bridge-winit`) provides windows, input, frames, input
//! methods, CPU presents and GPU surface handles. This crate adds:
//! - the clipboard (Wayland data-control or X11 selections), with sensitive copies marked
//!   with the password-manager hint so clipboard managers keep them out of history;
//! - system settings from the XDG desktop portal (color scheme, accent, contrast, reduced
//!   motion) and the desktop's interface settings it exposes (text scale, double-click time,
//!   caret blink), plus the locale from the environment.
//!
//! - drag and drop with other apps: out on X11 through XDND, both ways on Wayland through
//!   the data device (winit takes file drops in on X11).
//!
//! Capture protection is not available: no Linux display server lets a window exclude itself
//! from capture, so it reports `Unsupported` rather than pretending. On other platforms this
//! crate is empty.

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
#[allow(unsafe_code)]
mod os;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod services;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod dnd;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod dnd_wayland;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod dnd_x11;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod linux {
    use std::time::Duration;

    use arboard::SetExtLinux;
    use remedy_ui_bridge_winit::winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    use remedy_ui_bridge_winit::winit::window::Window;
    use remedy_ui_bridge_winit::{PlatformHooks, RunError};
    use remedy_ui_core::Color;
    use remedy_ui_host::{
        App, DragData, DragEvent, DragStarted, DropEffect, FileDialog, HostEvent, SystemSettings, TrayIcon,
        Unsupported, Waker,
    };
    use remedy_ui_input::Platform;
    use zbus::{OwnedVariant, Variant as Value};

    /// Linux's side of the host contract.
    #[derive(Default)]
    pub struct LinuxHooks {
        // The clipboard owner must outlive what it copied: on Wayland and X11 the copying
        // process serves the data to whoever pastes.
        clipboard: Option<arboard::Clipboard>,
        tray: crate::services::Tray,
        /// Drag and drop on Wayland, once a Wayland window exists.
        wayland: Option<crate::dnd_wayland::Wayland>,
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
    struct Portal(std::collections::HashMap<(&'static str, &'static str), Value<'static>>);

    const KEYS: [(&str, &str); 9] = [
        ("org.freedesktop.appearance", "color-scheme"),
        ("org.freedesktop.appearance", "accent-color"),
        ("org.freedesktop.appearance", "contrast"),
        ("org.freedesktop.appearance", "reduced-motion"),
        ("org.gnome.desktop.interface", "enable-animations"),
        ("org.gnome.desktop.interface", "text-scaling-factor"),
        ("org.gnome.desktop.interface", "cursor-blink"),
        ("org.gnome.desktop.interface", "cursor-blink-time"),
        ("org.gnome.desktop.peripherals.mouse", "double-click"),
    ];

    impl Portal {
        /// Reads every setting RemedyUI follows. Without a session bus or portal, every value
        /// is absent and the defaults apply.
        fn connect() -> Portal {
            let values = zbus::block_on(async {
                let mut out = std::collections::HashMap::new();
                let Ok(conn) = zbus::Connection::session().await else { return out };
                let Ok(p) = zbus::Proxy::new(
                    &conn,
                    "org.freedesktop.portal.Desktop",
                    "/org/freedesktop/portal/desktop",
                    "org.freedesktop.portal.Settings",
                )
                .await
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

    /// `en_US.UTF-8` (POSIX) to `en-US` (BCP 47).
    fn locale() -> Option<String> {
        let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))?;
        let base = raw.split(['.', '@']).next()?;
        (base != "C" && base != "POSIX").then(|| base.replace('_', "-"))
    }

    impl PlatformHooks for LinuxHooks {
        fn platform(&self) -> Platform {
            Platform::Standard
        }

        fn settings(&mut self) -> SystemSettings {
            const APPEARANCE: &str = "org.freedesktop.appearance";
            const INTERFACE: &str = "org.gnome.desktop.interface";
            let p = Portal::connect();
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
            SystemSettings {
                platform: Platform::Standard,
                dark: p.u32(APPEARANCE, "color-scheme").and_then(|v| match v {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                }),
                accent: p.accent(),
                high_contrast: p.u32(APPEARANCE, "contrast") == Some(1),
                text_scale: p.f64(INTERFACE, "text-scaling-factor").map_or(base.text_scale, |f| f as f32),
                reduced_motion,
                reduced_transparency: false,
                locale: locale().unwrap_or(base.locale),
                double_click: p
                    .i32("org.gnome.desktop.peripherals.mouse", "double-click")
                    .and_then(|ms| u64::try_from(ms).ok())
                    .map_or(base.double_click, Duration::from_millis),
                caret_blink: if blink {
                    Some(cycle.map_or(Duration::from_millis(600), |c| Duration::from_millis(c / 2)))
                } else {
                    None
                },
            }
        }

        fn window_created(&mut self, window: &Window) {
            let (Ok(d), Ok(w)) = (window.display_handle(), window.window_handle()) else { return };
            let (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) = (d.as_raw(), w.as_raw()) else {
                return;
            };
            if self.wayland.is_none() {
                // Without it, drags simply stay inside the app.
                self.wayland =
                    crate::dnd_wayland::Wayland::new(crate::os::wayland_connection(d.display)).ok();
            }
            if let Some(wl) = self.wayland.as_mut()
                && let Some(surface) = crate::os::wayland_surface(wl.connection(), w.surface)
            {
                wl.add_window(&surface, window.id());
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
                    crate::dnd_x11::drag(data, allowed).map(DragStarted::Finished)
                }
                RawWindowHandle::Wayland(h) => {
                    let wl = self
                        .wayland
                        .as_mut()
                        .ok_or_else(|| Unsupported::new("dragging out", "no Wayland data device"))?;
                    let surface = crate::os::wayland_surface(wl.connection(), h.surface)
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

        fn take_drag_events(&mut self) -> Vec<(remedy_ui_bridge_winit::winit::window::WindowId, DragEvent)> {
            self.wayland.as_mut().map(crate::dnd_wayland::Wayland::take_events).unwrap_or_default()
        }

        fn take_drag_ends(&mut self) -> Vec<(remedy_ui_bridge_winit::winit::window::WindowId, DropEffect)> {
            self.wayland.as_mut().map(crate::dnd_wayland::Wayland::take_ended).unwrap_or_default()
        }

        fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
            match self.clipboard()?.get_text() {
                Ok(t) => Ok(Some(t)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(e) => Err(Unsupported::new("the clipboard", e.to_string())),
            }
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

        fn set_waker(&mut self, waker: Waker) {
            self.tray.set_waker(waker);
        }

        fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
            // The desktop portal opens it the user's way (and works inside sandboxes).
            let portal = zbus::block_on(async {
                let conn = zbus::Connection::session().await?;
                let p = zbus::Proxy::new(
                    &conn,
                    "org.freedesktop.portal.Desktop",
                    "/org/freedesktop/portal/desktop",
                    "org.freedesktop.portal.OpenURI",
                )
                .await?;
                let options: std::collections::HashMap<&str, Value<'_>> = std::collections::HashMap::new();
                let _: zbus::OwnedObjectPath = p.call("OpenURI", &("", url, options)).await?;
                Ok::<(), zbus::Error>(())
            });
            match portal {
                Ok(()) => Ok(()),
                // No portal (a bare X session): the freedesktop opener.
                Err(e) => std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(|x| {
                    Unsupported::new("opening links", format!("no portal ({e}) and no xdg-open ({x})"))
                }),
            }
        }

        fn take_events(&mut self) -> Vec<HostEvent> {
            self.tray.take_events().into_iter().map(HostEvent::Tray).collect()
        }

        fn utc_offset(&self, unix_seconds: i64) -> Option<i32> {
            crate::os::utc_offset(unix_seconds)
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
            crate::services::file_dialog(&parent, dialog)
        }

        fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
            self.clipboard()?.clear().map_err(|e| Unsupported::new("the clipboard", e.to_string()))
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
        remedy_ui_bridge_winit::run(app, LinuxHooks::default())
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub use linux::{LinuxHooks, run};
