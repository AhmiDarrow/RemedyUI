//! System notifications on macOS: the User Notifications framework (`UNUserNotificationCenter`),
//! shown in the corner and kept in Notification Center under the app's name and icon.
//!
//! The framework only serves apps in a bundle with an identifier (`cargo remedy package`
//! makes one); a bare executable says `Unsupported` rather than crash. The first notification
//! asks the person's leave (the system prompt, once), and is handed over once they answered:
//! one added while the prompt is still open would be refused. Each notification's buttons are a
//! category of its own, with the custom dismiss action on so dismissals report too. A picture
//! is attached as a PNG thumbnail beside the app's icon. Presses, buttons and dismissals come
//! back through the center's delegate, from whichever thread the framework uses, and reach the
//! event loop through the waker.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace};
use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSError, NSSet, NSString, NSURL};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationAttachment, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationDefaultActionIdentifier,
    UNNotificationDismissActionIdentifier, UNNotificationInterruptionLevel,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use remedy_ui_core::host::{
    HostEvent, Notification, NotificationResponse, TrayImage, Unsupported, Urgency, Waker,
};

#[derive(Default)]
struct Shared {
    events: Vec<HostEvent>,
    waker: Option<Waker>,
    /// Notifications the person answered since the last one was shown: the system took them
    /// away, so their categories are no longer needed.
    ended: Vec<u64>,
}

/// The person has answered the system's prompt (at some time, this run or an earlier one).
static ANSWERED: AtomicBool = AtomicBool::new(false);

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(Mutex::default)
}

fn post(e: HostEvent) {
    let waker = {
        let mut s = shared().lock().unwrap_or_else(PoisonError::into_inner);
        s.events.push(e);
        s.waker.clone()
    };
    if let Some(w) = waker {
        w();
    }
}

pub(crate) fn set_waker(w: Waker) {
    shared().lock().unwrap_or_else(PoisonError::into_inner).waker = Some(w);
}

pub(crate) fn take_events() -> Vec<HostEvent> {
    std::mem::take(&mut shared().lock().unwrap_or_else(PoisonError::into_inner).events)
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Delegate` implements no Drop.
    #[unsafe(super(NSObject))]
    #[name = "RemedyUINotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Shown while the app is in front too, as on the other desktops.
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            let id = response.notification().request().identifier().to_string();
            if let Ok(id) = id.parse::<u64>() {
                let action = response.actionIdentifier();
                // SAFETY: framework constants.
                let (default, dismiss) =
                    unsafe { (UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier) };
                let r = if &*action == default {
                    NotificationResponse::Activated
                } else if &*action == dismiss {
                    NotificationResponse::Dismissed
                } else {
                    NotificationResponse::Action(action.to_string())
                };
                shared().lock().unwrap_or_else(PoisonError::into_inner).ended.push(id);
                post(HostEvent::Notification(id, r));
            }
            handler.call(());
        }
    }
);

thread_local! {
    /// The center keeps its delegate weakly: it lives here.
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
    /// Each shown notification's category (its buttons), by the app's id.
    static CATEGORIES: RefCell<HashMap<u64, Retained<UNNotificationCategory>>> = RefCell::new(HashMap::new());
}

fn center() -> Result<Retained<UNUserNotificationCenter>, Unsupported> {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return Err(Unsupported::new(
            "system notifications",
            "macOS shows notifications only for apps in a bundle; package the app with `cargo remedy package`",
        ));
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let first = DELEGATE.with(|d| d.borrow().is_none());
    if first {
        let this = Delegate::alloc().set_ivars(());
        // SAFETY: NSObject's designated initializer.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(this), init] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
    }
    Ok(center)
}

/// Hands `request` to the system, which says on its own thread whether it was shown.
fn add(center: &UNUserNotificationCenter, request: &UNNotificationRequest) {
    let added = RcBlock::new(|error: *mut NSError| {
        if !error.is_null() {
            // SAFETY: a non-null error the framework passed for the call.
            let e = unsafe { &*error };
            eprintln!("RemedyUI: the notification was not shown: {}", e.localizedDescription());
        }
    });
    center.addNotificationRequest_withCompletionHandler(request, Some(&added));
}

/// A PNG of the picture in the temporary folder, for an attachment (which moves the file into
/// the system's store).
fn png_file(id: u64, img: &TrayImage) -> Option<Retained<NSURL>> {
    let n = isize::try_from(img.size).ok()?;
    // Checked: a size near `u32::MAX` squared does not fit.
    let size = img.size;
    let bytes = (size as usize).checked_mul(size as usize).and_then(|n| n.checked_mul(4));
    if Some(img.rgba.len()) != bytes {
        return None;
    }
    // SAFETY: a bitmap that allocates its own buffer of exactly `rgba.len()` bytes (8 bits ×
    // 4 samples, rows of `size * 4` bytes), filled from the picture, then encoded.
    let data = unsafe {
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            n,
            n,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            n * 4,
            32,
        )?;
        let dst = rep.bitmapData();
        if dst.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(img.rgba.as_ptr(), dst, img.rgba.len());
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?
    };
    let stamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let path =
        std::env::temp_dir().join(format!("remedy-notification-{}-{id}-{stamp}.png", std::process::id()));
    std::fs::write(&path, data.to_vec()).ok()?;
    Some(NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())))
}

pub(crate) fn show(n: &Notification) -> Result<(), Unsupported> {
    let center = center()?;
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&n.title));
    content.setBody(&NSString::from_str(&n.body));
    if !n.silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    // Time-sensitive breaks through Focus where the app has the entitlement (macOS 12 and
    // later; earlier versions have no interruption levels).
    if n.urgency != Urgency::Info && content.respondsToSelector(objc2::sel!(setInterruptionLevel:)) {
        content.setInterruptionLevel(UNNotificationInterruptionLevel::TimeSensitive);
    }
    let category_id = NSString::from_str(&format!("remedy-{}", n.id));
    let actions: Vec<Retained<UNNotificationAction>> = n
        .actions
        .iter()
        .map(|a| {
            UNNotificationAction::actionWithIdentifier_title_options(
                &NSString::from_str(&a.id),
                &NSString::from_str(&a.label),
                UNNotificationActionOptions::empty(),
            )
        })
        .collect();
    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &category_id,
        &NSArray::from_retained_slice(&actions),
        &NSArray::new(),
        UNNotificationCategoryOptions::CustomDismissAction,
    );
    let ended = std::mem::take(&mut shared().lock().unwrap_or_else(PoisonError::into_inner).ended);
    let all: Vec<Retained<UNNotificationCategory>> = CATEGORIES.with(|c| {
        let mut c = c.borrow_mut();
        for id in ended {
            c.remove(&id);
        }
        c.insert(n.id, category);
        c.values().cloned().collect()
    });
    center.setNotificationCategories(&NSSet::from_retained_slice(&all));
    content.setCategoryIdentifier(&category_id);
    if let Some(url) = n.icon.as_ref().and_then(|img| png_file(n.id, img)) {
        // SAFETY: a file URL to a PNG, with no options.
        if let Ok(a) = unsafe {
            UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(
                &NSString::from_str("icon"),
                &url,
                None,
            )
        } {
            content.setAttachments(&NSArray::from_retained_slice(&[a]));
        }
    }
    // The same identifier replaces a notification still shown.
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&n.id.to_string()),
        &content,
        None,
    );
    if ANSWERED.load(Ordering::Acquire) {
        add(&center, &request);
        return Ok(());
    }
    // Until the person has answered the prompt, a request would be refused: ask (the system
    // prompts once, and answers at once when it already knows) and add it on the answer.
    let waiting = center.clone();
    let asked = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        ANSWERED.store(true, Ordering::Release);
        if granted.as_bool() {
            add(&waiting, &request);
        } else {
            eprintln!("RemedyUI: notifications are turned off for this app in System Settings");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
        &asked,
    );
    Ok(())
}

pub(crate) fn withdraw(id: u64) -> Result<(), Unsupported> {
    let center = center()?;
    let ids = NSArray::from_retained_slice(&[NSString::from_str(&id.to_string())]);
    center.removeDeliveredNotificationsWithIdentifiers(&ids);
    center.removePendingNotificationRequestsWithIdentifiers(&ids);
    CATEGORIES.with(|c| c.borrow_mut().remove(&id));
    Ok(())
}
