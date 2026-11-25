//! The menu-bar status item: `NSStatusItem` with the app's icon and an `NSMenu`, whose
//! entries report through a small Objective-C target class.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
};
use objc2_foundation::{NSSize, NSString};
use remedy_ui_core::host::{TrayEvent, TrayIcon, TrayItem, Unsupported, Waker};

thread_local! {
    static EVENTS: RefCell<Vec<TrayEvent>> = const { RefCell::new(Vec::new()) };
    static WAKER: RefCell<Option<Waker>> = const { RefCell::new(None) };
    static ITEM: RefCell<Option<(Retained<NSStatusItem>, Retained<Target>)>> = const { RefCell::new(None) };
}

fn push(e: TrayEvent) {
    EVENTS.with(|v| v.borrow_mut().push(e));
    if let Some(w) = WAKER.with(|w| w.borrow().clone()) {
        w();
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Target` implements no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RemedyUITrayTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(menuItemChosen:))]
        fn chosen(&self, item: &NSMenuItem) {
            if let Ok(id) = u32::try_from(item.tag()) {
                push(TrayEvent::Menu(id));
            }
        }

        #[unsafe(method(statusClicked:))]
        fn clicked(&self, _sender: Option<&AnyObject>) {
            push(TrayEvent::Activate);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Tray events since the last call.
pub(crate) fn take_events() -> Vec<TrayEvent> {
    EVENTS.with(|v| std::mem::take(&mut *v.borrow_mut()))
}

/// Wakes the event loop when the tray reports.
pub(crate) fn set_waker(w: Waker) {
    WAKER.with(|slot| *slot.borrow_mut() = Some(w));
}

/// An image from premultiplied RGBA (the bitmap format AppKit uses by default).
fn image(size: u32, rgba: &[u8]) -> Option<Retained<NSImage>> {
    let n = isize::try_from(size).ok()?;
    if rgba.len() != size as usize * size as usize * 4 {
        return None;
    }
    // SAFETY: a bitmap that allocates its own buffer of exactly `rgba.len()` bytes (8 bits ×
    // 4 samples, rows of `size * 4` bytes), which is then filled from `rgba`.
    unsafe {
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
        std::ptr::copy_nonoverlapping(rgba.as_ptr(), dst, rgba.len());
        // Menu-bar icons are 18 points tall.
        let img = NSImage::initWithSize(NSImage::alloc(), NSSize::new(18.0, 18.0));
        img.addRepresentation(&rep);
        Some(img)
    }
}

/// Shows, updates or removes the status item.
pub(crate) fn set(tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| Unsupported::new("the tray", "not on the main thread"))?;
    let Some(t) = tray else {
        if let Some((item, _)) = ITEM.with(|i| i.borrow_mut().take()) {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
        }
        return Ok(());
    };
    let (item, target) = match ITEM.with(|i| i.borrow().clone()) {
        Some(pair) => pair,
        None => {
            let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
            let target = Target::new(mtm);
            ITEM.with(|i| *i.borrow_mut() = Some((item.clone(), target.clone())));
            (item, target)
        }
    };
    let button =
        item.button(mtm).ok_or_else(|| Unsupported::new("the tray", "the status item has no button"))?;
    if let Some(img) = image(t.icon.size, &t.icon.rgba) {
        button.setImage(Some(&img));
    }
    button.setToolTip(Some(&NSString::from_str(&t.tooltip)));
    if t.menu.is_empty() {
        // Without a menu, a click is the activation.
        item.setMenu(None);
        // SAFETY: the target lives as long as the item (both kept in ITEM).
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(statusClicked:)));
        }
        return Ok(());
    }
    item.setMenu(Some(&build_menu(mtm, &t.menu, &target)));
    Ok(())
}

/// A menu of `items` whose choices go to `target`. macOS menus have no default item, so
/// `default` changes nothing here.
fn build_menu(mtm: MainThreadMarker, items: &[TrayItem], target: &Target) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    let entry = |label: &str, id: u32, enabled: bool| {
        // SAFETY: a standard menu item whose action is implemented by `Target`.
        let mi = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                Some(sel!(menuItemChosen:)),
                &NSString::from_str(""),
            )
        };
        // SAFETY: the target outlives the menu (both kept in ITEM).
        unsafe { mi.setTarget(Some(target)) };
        mi.setTag(isize::try_from(id).unwrap_or(0));
        mi.setEnabled(enabled);
        mi
    };
    for it in items {
        match it {
            TrayItem::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            TrayItem::Item { id, label, enabled, .. } => menu.addItem(&entry(label, *id, *enabled)),
            TrayItem::Check { id, label, enabled, checked } => {
                let mi = entry(label, *id, *enabled);
                mi.setState(isize::from(*checked));
                menu.addItem(&mi);
            }
            TrayItem::Submenu { label, items } => {
                let mi = NSMenuItem::new(mtm);
                mi.setTitle(&NSString::from_str(label));
                let sub = build_menu(mtm, items, target);
                sub.setTitle(&NSString::from_str(label));
                mi.setSubmenu(Some(&sub));
                mi.setEnabled(!items.is_empty());
                menu.addItem(&mi);
            }
        }
    }
    // Item enablement is ours, not the responder chain's.
    menu.setAutoenablesItems(false);
    menu
}
