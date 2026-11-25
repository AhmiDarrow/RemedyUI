//! Platforms whose bridge cannot host a native web view yet. The element reports
//! `WebViewStatus::Unsupported` with the reason; nothing is faked.

use std::rc::Rc;

use remedy_ui_core::host::WindowHandles;
use remedy_ui_engine::view::{Placement, WeakUi};

use crate::embed_webview::{Command, Hub, NativeView};

#[cfg(target_os = "macos")]
const REASON: &str = "web views on macOS need WKWebView placed as a child NSView of the app's window, which the \
                      macOS bridge does not host yet";
#[cfg(target_os = "android")]
const REASON: &str = "web views on Android need android.webkit.WebView in the activity's view hierarchy, which \
                      waits for the Android bridge";
#[cfg(target_os = "ios")]
const REASON: &str = "web views on iOS need WKWebView, which waits for the iOS bridge";
#[cfg(not(any(target_os = "macos", target_os = "android", target_os = "ios")))]
const REASON: &str = "this platform has no web view RemedyUI can host";

/// No web view: never made.
pub(crate) enum Native {}

impl Native {
    pub(crate) fn new(_handles: &dyn WindowHandles, _hub: Rc<Hub>, _ui: WeakUi) -> Result<Native, String> {
        Err(REASON.into())
    }

    pub(crate) fn command(&self, _c: Command) {
        match *self {}
    }

    pub(crate) fn place(&self, _p: Option<&Placement>) {
        match *self {}
    }

    pub(crate) fn native_view(&self) -> Option<NativeView> {
        match *self {}
    }

    pub(crate) fn close(self) {
        match self {}
    }
}
