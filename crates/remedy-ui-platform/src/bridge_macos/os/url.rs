//! Links in the app's URL schemes. macOS does not start a second copy of a running app for a
//! link: Launch Services sends the running app a "get URL" Apple event (the first launch gets
//! one too, after it starts). The bridge hears it and passes the URL on as a later launch's
//! arguments, the way single instance hands them over on the other desktops.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
use remedy_ui_core::host::Waker;

/// `kInternetEventClass` and `kAEGetURL`: both 'GURL'.
const GURL: u32 = u32::from_be_bytes(*b"GURL");
/// `keyDirectObject`: '----'.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

thread_local! {
    static URLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static WAKER: RefCell<Option<Waker>> = const { RefCell::new(None) };
    static HANDLER: RefCell<Option<Retained<Handler>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Handler` implements no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RemedyUIURLHandler"]
    struct Handler;

    impl Handler {
        #[unsafe(method(handleGetURL:withReplyEvent:))]
        fn get_url(&self, event: &NSAppleEventDescriptor, _reply: Option<&AnyObject>) {
            // SAFETY: `paramDescriptorForKeyword:` takes an AEKeyword and returns a descriptor
            // (or nil) owned by the event.
            let param: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
            let Some(url) = param.and_then(|p| p.stringValue()) else { return };
            URLS.with(|u| u.borrow_mut().push(url.to_string()));
            if let Some(w) = WAKER.with(|w| w.borrow().clone()) {
                w();
            }
        }
    }
);

/// Starts hearing "get URL" events (again: AppKit may install its own handler while the app
/// finishes launching, so the bridge calls this before the loop starts and once it runs).
pub(crate) fn listen() {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let handler = HANDLER.with(|h| {
        h.borrow_mut()
            .get_or_insert_with(|| {
                let this = Handler::alloc(mtm).set_ivars(());
                // SAFETY: NSObject's designated initializer.
                unsafe { msg_send![super(this), init] }
            })
            .clone()
    });
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let target: &NSObject = &handler;
    // SAFETY: the handler object lives in `HANDLER` for the life of the thread and answers the
    // selector with the (event, reply) signature Apple events use.
    unsafe {
        let _: () = msg_send![
            &*manager,
            setEventHandler: target,
            andSelector: sel!(handleGetURL:withReplyEvent:),
            forEventClass: GURL,
            andEventID: GURL
        ];
    }
}

/// Wakes the event loop when a link arrives.
pub(crate) fn set_waker(w: Waker) {
    WAKER.with(|slot| *slot.borrow_mut() = Some(w));
}

/// The links that arrived since the last call.
pub(crate) fn take() -> Vec<String> {
    URLS.with(|u| std::mem::take(&mut *u.borrow_mut()))
}
