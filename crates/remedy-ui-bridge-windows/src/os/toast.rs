//! System notifications on Windows: toast notifications (Windows 10 and 11), shown in the
//! corner and kept in the notification center, with the app's name and icon, buttons, and
//! replacing or taking away by the app's own id.
//!
//! An app that is not installed from a package has no identity of its own for toasts, so
//! the engine gives it one: an application user model id made from its name, registered
//! for the current user (`HKCU\Software\Classes\AppUserModelId`, the way the Windows App
//! SDK registers unpackaged apps) with its display name and icon. The process's own id is
//! left alone, so taskbar pins and grouping are not disturbed.
//!
//! Presses and dismissals come back on the toast's events, on a system thread, and reach
//! the event loop through the waker.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use remedy_ui_host::{HostEvent, Notification, NotificationResponse, Unsupported, Urgency, Waker};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    NotificationSetting, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
    ToastNotification, ToastNotificationManager, ToastNotificationPriority, ToastNotifier,
};
use windows::core::{HSTRING, IInspectable, Interface};

/// The group every toast of the app is in (tags are the app's ids).
const GROUP: &str = "remedy";
/// What the toast's own press reports, as against a button's `action:` arguments.
const ACTIVATE: &str = "activate";

#[derive(Default)]
struct Shared {
    events: Vec<HostEvent>,
    waker: Option<Waker>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(Mutex::default)
}

fn post(e: HostEvent) {
    let waker = {
        let mut g = shared().lock().unwrap_or_else(PoisonError::into_inner);
        g.events.push(e);
        g.waker.clone()
    };
    if let Some(w) = waker {
        w();
    }
}

pub(crate) fn set_waker(waker: Waker) {
    shared().lock().unwrap_or_else(PoisonError::into_inner).waker = Some(waker);
}

pub(crate) fn take_events() -> Vec<HostEvent> {
    std::mem::take(&mut shared().lock().unwrap_or_else(PoisonError::into_inner).events)
}

/// The app's toasts: its notifier, and the toasts shown, kept so their events stay wired.
#[derive(Default)]
pub(crate) struct Toasts {
    notifier: Option<(String, ToastNotifier)>,
    shown: HashMap<u64, ToastNotification>,
}

fn unsupported(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("system notifications", e.to_string())
}

/// An application user model id for an app called `name`: letters, digits and dots only,
/// at most 128 characters.
pub(crate) fn app_id(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => Some(c),
            ' ' | '-' | '_' | '.' => Some('.'),
            _ => None,
        })
        .collect();
    let clean: Vec<&str> = clean.split('.').filter(|p| !p.is_empty()).collect();
    let mut id =
        format!("RemedyUI.App.{}", if clean.is_empty() { "Unnamed".to_owned() } else { clean.join(".") });
    id.truncate(128);
    id
}

/// Text made safe inside XML.
fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 cannot carry at all.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// The toast's XML: the generic template with title and body, the icon in the app's logo
/// spot, up to five buttons, and quiet audio when asked.
pub(crate) fn toast_xml(n: &Notification, icon: Option<&std::path::Path>) -> String {
    let mut x = format!(
        r#"<toast launch="{ACTIVATE}"{}><visual><binding template="ToastGeneric"><text>{}</text>"#,
        // Windows 11's "important" toast, which also breaks through do-not-disturb.
        if n.urgency == Urgency::Error { r#" scenario="urgent""# } else { "" },
        xml(&n.title)
    );
    if !n.body.is_empty() {
        x.push_str(&format!("<text>{}</text>", xml(&n.body)));
    }
    if let Some(p) = icon {
        let url = format!("file:///{}", p.to_string_lossy().replace('\\', "/"));
        x.push_str(&format!(r#"<image placement="appLogoOverride" src="{}"/>"#, xml(&url)));
    }
    x.push_str("</binding></visual>");
    if !n.actions.is_empty() {
        x.push_str("<actions>");
        for a in n.actions.iter().take(5) {
            x.push_str(&format!(
                r#"<action content="{}" arguments="action:{}" activationType="foreground"/>"#,
                xml(&a.label),
                xml(&a.id)
            ));
        }
        x.push_str("</actions>");
    }
    if n.silent {
        x.push_str(r#"<audio silent="true"/>"#);
    }
    x.push_str("</toast>");
    x
}

/// Writes the icon as a PNG in the app's local cache (toasts take images by file), named by
/// its content so a changed icon is a new file.
fn icon_file(id: &str, icon: &remedy_ui_host::TrayImage) -> Option<std::path::PathBuf> {
    let n = icon.size as usize;
    if n == 0 || icon.rgba.len() != n.checked_mul(n)?.checked_mul(4)? {
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
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in &straight {
        hash = (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    let dir = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("RemedyUI")
        .join("notification-icons");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{id}-{hash:016x}.png"));
    if path.exists() {
        return Some(path);
    }
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, icon.size, icon.size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(&straight).ok()?;
    }
    // Written beside and moved into place, so a reader never sees half a file.
    let tmp = path.with_extension("png.tmp");
    std::fs::write(&tmp, &bytes).ok()?;
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

impl Toasts {
    /// The notifier for the app's identity, registering it for the current user on first use.
    fn notifier(&mut self, app: &str, icon: Option<&std::path::Path>) -> Result<ToastNotifier, Unsupported> {
        let id = app_id(app);
        if let Some((have, n)) = &self.notifier
            && *have == id
        {
            return Ok(n.clone());
        }
        crate::os::register_app_id(&id, app, icon).map_err(unsupported)?;
        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&id)).map_err(unsupported)?;
        self.notifier = Some((id, notifier.clone()));
        Ok(notifier)
    }

    pub(crate) fn show(&mut self, n: &Notification) -> Result<(), Unsupported> {
        crate::os::com_for_winrt();
        let id = app_id(&n.app);
        let icon = n.icon.as_ref().and_then(|i| icon_file(&id, i));
        let notifier = self.notifier(&n.app, icon.as_deref())?;
        // An identity registered only for the current user (no Start menu entry) has no
        // setting Windows can report until its first toast: an error here means "not known
        // yet", not "off".
        match notifier.Setting().unwrap_or(NotificationSetting::Enabled) {
            NotificationSetting::Enabled => {}
            NotificationSetting::DisabledForApplication => {
                return Err(unsupported("notifications are turned off for this app in Windows' settings"));
            }
            NotificationSetting::DisabledForUser => {
                return Err(unsupported("notifications are turned off in Windows' settings"));
            }
            NotificationSetting::DisabledByGroupPolicy => {
                return Err(unsupported("an administrator's policy turned notifications off"));
            }
            other => return Err(unsupported(format!("Windows will not show notifications ({})", other.0))),
        }
        let doc = XmlDocument::new().map_err(unsupported)?;
        doc.LoadXml(&HSTRING::from(toast_xml(n, icon.as_deref()))).map_err(unsupported)?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(unsupported)?;
        toast.SetTag(&HSTRING::from(n.id.to_string())).map_err(unsupported)?;
        toast.SetGroup(&HSTRING::from(GROUP)).map_err(unsupported)?;
        if n.urgency != Urgency::Info {
            // Shown above other toasts and in front of full-screen apps' suppression.
            toast.SetPriority(ToastNotificationPriority::High).map_err(unsupported)?;
        }
        let app_id = n.id;
        toast
            .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
                let arguments = args
                    .as_ref()
                    .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
                    .and_then(|a| a.Arguments().ok())
                    .map(|h| h.to_string())
                    .unwrap_or_default();
                post(HostEvent::Notification(
                    app_id,
                    match arguments.strip_prefix("action:") {
                        Some(action) => NotificationResponse::Action(action.to_owned()),
                        None => NotificationResponse::Activated,
                    },
                ));
                Ok(())
            }))
            .map_err(unsupported)?;
        toast
            .Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(
                move |_, args| {
                    // Only the person closing it counts: a toast that slid into the notification
                    // center (timed out) or that the app took away was not dismissed by anyone.
                    if args.as_ref().and_then(|a| a.Reason().ok()) == Some(ToastDismissalReason::UserCanceled)
                    {
                        post(HostEvent::Notification(app_id, NotificationResponse::Dismissed));
                    }
                    Ok(())
                },
            ))
            .map_err(unsupported)?;
        notifier.Show(&toast).map_err(unsupported)?;
        self.shown.insert(n.id, toast);
        Ok(())
    }

    pub(crate) fn withdraw(&mut self, id: u64) -> Result<(), Unsupported> {
        crate::os::com_for_winrt();
        self.shown.remove(&id);
        let Some((app, _)) = &self.notifier else { return Ok(()) };
        ToastNotificationManager::History()
            .and_then(|h| {
                h.RemoveGroupedTagWithId(
                    &HSTRING::from(id.to_string()),
                    &HSTRING::from(GROUP),
                    &HSTRING::from(app),
                )
            })
            .map_err(unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_are_plain_and_stable() {
        assert_eq!(app_id("Secret Sticky"), "RemedyUI.App.Secret.Sticky");
        assert_eq!(app_id("Alex's Craft-Calc"), "RemedyUI.App.Alexs.Craft.Calc");
        assert_eq!(app_id("  "), "RemedyUI.App.Unnamed");
        assert!(app_id(&"x".repeat(500)).len() <= 128);
    }

    #[test]
    fn toast_xml_escapes_text_and_carries_buttons_and_quiet() {
        let n = Notification::new(4, "Ada & <Bob>", "\"hi\"\u{1}").action("mute", "Mute").silent(true);
        let x = toast_xml(&n, None);
        assert!(x.contains("<text>Ada &amp; &lt;Bob&gt;</text><text>&quot;hi&quot;</text>"), "{x}");
        assert!(
            x.contains(r#"<action content="Mute" arguments="action:mute" activationType="foreground"/>"#)
        );
        assert!(x.contains(r#"<audio silent="true"/>"#));
        assert!(!x.contains("scenario"), "news is a plain toast");
        let urgent = toast_xml(&Notification::new(5, "Disk full", "").urgency(Urgency::Error), None);
        assert!(urgent.starts_with(r#"<toast launch="activate" scenario="urgent">"#), "{urgent}");
        let doc = XmlDocument::new();
        assert!(doc.is_ok_and(|d| d.LoadXml(&HSTRING::from(x)).is_ok()), "Windows reads it as XML");
    }
}
