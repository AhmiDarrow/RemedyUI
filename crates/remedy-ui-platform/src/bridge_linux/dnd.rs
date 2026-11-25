//! What a drag out of the app offers other apps on Linux, whichever display server carries
//! it: files as `text/uri-list` (and their paths as text), links as `text/uri-list`, text as
//! UTF-8 under the names X11 and Wayland apps ask for.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use remedy_ui_core::host::{DragData, DropEffect};

/// How long reading a dropped offer from another app may take (Wayland) unless the app says
/// otherwise ([`set_drop_waits`]).
pub const DROP_READ_WAIT: Duration = Duration::from_secs(2);
/// How long a drag out of the app serves its data once released (X11) unless the app says
/// otherwise ([`set_drop_waits`]).
pub const DROP_FINISH_WAIT: Duration = Duration::from_secs(10);

static WAITS: Mutex<(Duration, Duration)> = Mutex::new((DROP_READ_WAIT, DROP_FINISH_WAIT));

/// Sets how long a drop from another app may take to read (Wayland; default
/// [`DROP_READ_WAIT`], 2 s) and how long a drag out of the app serves its data once released
/// (X11; default [`DROP_FINISH_WAIT`], 10 s). Each guards against an app that never answers:
/// longer lets a slow app hand over a large drop, at a drop that waits that long when the
/// other app hangs. `remedy_ui::run` sets them from `AppConfig::drop_waits`.
pub fn set_drop_waits(read: Duration, finish: Duration) {
    *WAITS.lock().unwrap_or_else(PoisonError::into_inner) = (read, finish);
}

/// The waits [`set_drop_waits`] set: (read, finish).
pub(crate) fn drop_waits() -> (Duration, Duration) {
    *WAITS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One way of reading the dragged data: a MIME type (or X11 target name) and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) mime: &'static str,
    pub(crate) bytes: Vec<u8>,
}

/// A `file://` URI for an absolute path, every byte outside the unreserved set and `/`
/// percent-encoded.
pub(crate) fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The offers for `data`, most specific first.
pub(crate) fn offers(data: &DragData) -> Vec<Offer> {
    let mut uris: Vec<String> = data.files.iter().map(|p| file_uri(p)).collect();
    uris.extend(data.uris.iter().cloned());
    let mut out = Vec::new();
    if !uris.is_empty() {
        // RFC 2483: one URI a line, CRLF after each.
        let list: String = uris.iter().map(|u| format!("{u}\r\n")).collect();
        out.push(Offer { mime: "text/uri-list", bytes: list.into_bytes() });
    }
    let text = match &data.text {
        Some(t) => Some(t.clone()),
        None if !data.files.is_empty() => {
            Some(data.files.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n"))
        }
        None if !data.uris.is_empty() => Some(data.uris.join("\n")),
        None => None,
    };
    if let Some(t) = text {
        for mime in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING"] {
            out.push(Offer { mime, bytes: t.clone().into_bytes() });
        }
    }
    out
}

/// The action to ask for: copy when allowed, else move, else link.
pub(crate) fn preferred(allowed: &[DropEffect]) -> DropEffect {
    [DropEffect::Copy, DropEffect::Move, DropEffect::Link]
        .into_iter()
        .find(|e| allowed.contains(e))
        .unwrap_or(DropEffect::None)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use remedy_ui_core::host::{DragData, DropEffect};

    use super::{file_uri, offers, preferred};

    #[test]
    fn files_are_offered_as_a_uri_list_and_as_their_paths() {
        let data =
            DragData { files: vec![PathBuf::from("/home/sam/Photos/beach day.jpg")], ..DragData::default() };
        let o = offers(&data);
        assert_eq!(o.first().map(|o| o.mime), Some("text/uri-list"));
        assert_eq!(
            o.first().map(|o| String::from_utf8_lossy(&o.bytes).into_owned()).as_deref(),
            Some("file:///home/sam/Photos/beach%20day.jpg\r\n")
        );
        let plain = o.iter().find(|o| o.mime == "UTF8_STRING").map(|o| o.bytes.clone());
        assert_eq!(plain.as_deref(), Some("/home/sam/Photos/beach day.jpg".as_bytes()));
    }

    #[test]
    fn text_alone_is_offered_as_text() {
        let o = offers(&DragData { text: Some("héllo".into()), ..DragData::default() });
        assert!(o.iter().all(|o| o.mime != "text/uri-list"));
        assert!(o.iter().any(|o| o.mime == "text/plain;charset=utf-8" && o.bytes == "héllo".as_bytes()));
        assert!(offers(&DragData::default()).is_empty());
    }

    #[test]
    fn uris_percent_encode_bytes_and_the_action_prefers_copy() {
        assert_eq!(file_uri(std::path::Path::new("/tmp/ä#1")), "file:///tmp/%C3%A4%231");
        assert_eq!(preferred(&[DropEffect::Move, DropEffect::Copy]), DropEffect::Copy);
        assert_eq!(preferred(&[DropEffect::Move]), DropEffect::Move);
        assert_eq!(preferred(&[]), DropEffect::None);
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;

    #[test]
    fn the_app_sets_the_drop_waits() {
        assert_eq!(drop_waits(), (DROP_READ_WAIT, DROP_FINISH_WAIT));
        set_drop_waits(Duration::from_secs(30), Duration::from_secs(1));
        assert_eq!(drop_waits(), (Duration::from_secs(30), Duration::from_secs(1)));
        set_drop_waits(DROP_READ_WAIT, DROP_FINISH_WAIT);
    }
}
