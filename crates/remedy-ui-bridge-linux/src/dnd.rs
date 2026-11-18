//! What a drag out of the app offers other apps on Linux, whichever display server carries
//! it: files as `text/uri-list` (and their paths as text), links as `text/uri-list`, text as
//! UTF-8 under the names X11 and Wayland apps ask for.

use remedy_ui_host::{DragData, DropEffect};

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

    use remedy_ui_host::{DragData, DropEffect};

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
