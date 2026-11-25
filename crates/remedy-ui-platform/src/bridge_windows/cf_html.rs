//! Windows' "HTML Format" clipboard data: a header of byte offsets, then an HTML document
//! whose copied part sits between `<!--StartFragment-->` and `<!--EndFragment-->`.

/// The header with room for each offset (ten digits each), filled in by [`wrap`].
const HEADER: &str =
    "Version:0.9\r\nStartHTML:{SH}\r\nEndHTML:{EH}\r\nStartFragment:{SF}\r\nEndFragment:{EF}\r\n";

/// `html` as "HTML Format" bytes, ending with a NUL as the format asks.
pub(crate) fn wrap(html: &str) -> Vec<u8> {
    let pad = |n: usize| format!("{n:010}");
    let header_len = HEADER
        .replace("{SH}", &pad(0))
        .replace("{EH}", &pad(0))
        .replace("{SF}", &pad(0))
        .replace("{EF}", &pad(0))
        .len();
    let before = "<html><body>\r\n<!--StartFragment-->";
    let after = "<!--EndFragment-->\r\n</body></html>";
    let start_html = header_len;
    let start_fragment = start_html + before.len();
    let end_fragment = start_fragment + html.len();
    let end_html = end_fragment + after.len();
    let header = HEADER
        .replace("{SH}", &pad(start_html))
        .replace("{EH}", &pad(end_html))
        .replace("{SF}", &pad(start_fragment))
        .replace("{EF}", &pad(end_fragment));
    let mut out = Vec::with_capacity(end_html + 1);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(before.as_bytes());
    out.extend_from_slice(html.as_bytes());
    out.extend_from_slice(after.as_bytes());
    out.push(0);
    out
}

/// The HTML document in "HTML Format" bytes (fragment markers kept, so a reader can take only
/// the copied part). Data without a usable header is taken as HTML as it is.
pub(crate) fn unwrap(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let bytes = bytes.get(..end).unwrap_or_default();
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(512)).unwrap_or_default()).into_owned();
    let offset = |key: &str| -> Option<usize> {
        let rest = head.split_once(key)?.1;
        rest.lines().next()?.trim().parse::<usize>().ok()
    };
    let part = |start: &str, end: &str| match (offset(start), offset(end)) {
        (Some(s), Some(e)) if s <= e => bytes.get(s..e.min(bytes.len())),
        _ => None,
    };
    // StartHTML is -1 when the source gave no document around its fragment.
    let doc = part("StartHTML:", "EndHTML:").or_else(|| part("StartFragment:", "EndFragment:"));
    String::from_utf8_lossy(doc.unwrap_or(bytes)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_point_at_the_document_and_the_fragment() {
        let html = "<p>caf\u{e9} <b>bold</b></p>";
        let bytes = wrap(html);
        assert_eq!(bytes.last(), Some(&0));
        let text = String::from_utf8_lossy(bytes.strip_suffix(&[0]).unwrap_or_default()).into_owned();
        let num = |k: &str| -> usize {
            text.split_once(k)
                .and_then(|(_, r)| r.lines().next())
                .and_then(|v| v.parse().ok())
                .unwrap_or(usize::MAX)
        };
        let (sf, ef) = (num("StartFragment:"), num("EndFragment:"));
        assert_eq!(bytes.get(sf..ef), Some(html.as_bytes()), "the fragment is exactly the copied HTML");
        let doc = unwrap(&bytes);
        assert!(doc.starts_with("<html><body>") && doc.ends_with("</body></html>"), "{doc}");
        assert!(doc.contains("<!--StartFragment--><p>caf\u{e9} <b>bold</b></p><!--EndFragment-->"));
    }

    #[test]
    fn data_without_a_header_is_html_as_it_is() {
        assert_eq!(unwrap(b"<b>hi</b>\0junk"), "<b>hi</b>");
        assert_eq!(
            unwrap(b"Version:0.9\r\nStartHTML:99999\r\nEndHTML:5\r\n<b>x</b>"),
            "Version:0.9\r\nStartHTML:99999\r\nEndHTML:5\r\n<b>x</b>"
        );
    }

    #[test]
    fn a_fragment_without_a_document_is_the_html() {
        let head = |sf: usize, ef: usize| {
            format!(
                "Version:0.9\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:{sf:010}\r\nEndFragment:{ef:010}\r\n"
            )
        };
        let n = head(0, 0).len();
        assert_eq!(unwrap(format!("{}<b>hi</b>", head(n, n + 9)).as_bytes()), "<b>hi</b>");
    }
}
