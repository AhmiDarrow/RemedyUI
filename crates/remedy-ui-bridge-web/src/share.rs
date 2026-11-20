//! What a page's share hands to `navigator.share`, decided from what the browser can share.
//! Platform-neutral, so it is tested on every machine.

use remedy_ui_host::{ShareOptions, Unsupported};

/// What goes to `navigator.share`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Payload {
    /// The title, the text and the files.
    WithFiles,
    /// The title and the text only.
    TextOnly,
}

/// What the browser offers, as the page found it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Browser {
    /// `navigator.share` exists (secure pages of browsers with a share sheet).
    pub(crate) share: bool,
    /// The page may act for the person now (a click or key press just happened).
    pub(crate) active: bool,
    /// `navigator.canShare` accepts the files (false when it does not exist: browsers
    /// without it share no files).
    pub(crate) files: bool,
}

fn refuse(reason: &str) -> Unsupported {
    Unsupported::new("sharing", reason)
}

/// Picks what to share, or why the browser cannot.
pub(crate) fn payload(share: &ShareOptions, browser: Browser) -> Result<Payload, Unsupported> {
    if !browser.share {
        return Err(refuse("this browser has no share sheet (navigator.share), or the page is not secure"));
    }
    if !browser.active {
        return Err(refuse("a browser opens its share sheet only in answer to a click or key press"));
    }
    if share.files.is_empty() || browser.files {
        return Ok(if share.files.is_empty() { Payload::TextOnly } else { Payload::WithFiles });
    }
    if share.require_files {
        return Err(refuse("this browser cannot share these files"));
    }
    if share.title.is_empty() && share.text.is_empty() {
        return Err(refuse("this browser cannot share these files, and there is no text to share instead"));
    }
    Ok(Payload::TextOnly)
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_host::ShareFile;

    fn recipe() -> ShareOptions {
        ShareOptions::new().title("Lavender").text("Lavender soap").file(ShareFile::new(
            "Lavender.alex-soap.json",
            "application/json",
            b"{}".to_vec(),
        ))
    }

    #[test]
    fn files_go_where_the_browser_takes_them_and_text_alone_otherwise() {
        let all = Browser { share: true, active: true, files: true };
        assert_eq!(payload(&recipe(), all), Ok(Payload::WithFiles));
        let no_files = Browser { files: false, ..all };
        assert_eq!(payload(&recipe(), no_files), Ok(Payload::TextOnly));
        assert_eq!(payload(&ShareOptions::new().text("hi"), no_files), Ok(Payload::TextOnly));
        let required = recipe().require_files(true);
        assert!(payload(&required, no_files).is_err_and(|u| u.reason.contains("cannot share these files")));
        let only_files = ShareOptions::new().file(ShareFile::new("a.json", "application/json", Vec::new()));
        assert!(payload(&only_files, no_files).is_err_and(|u| u.reason.contains("no text")));
    }

    #[test]
    fn browsers_without_a_sheet_or_a_click_refuse_with_the_reason() {
        let none = Browser { share: false, active: true, files: true };
        assert!(payload(&recipe(), none).is_err_and(|u| u.reason.contains("navigator.share")));
        let idle = Browser { share: true, active: false, files: true };
        assert!(payload(&recipe(), idle).is_err_and(|u| u.reason.contains("click or key press")));
    }
}
