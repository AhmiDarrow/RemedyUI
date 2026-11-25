//! Dragging out to other apps through an AppKit dragging session: files as file URLs (Finder
//! copies or moves them), text, and links. The session runs on its own after the call; its
//! end is reported through [`take_ended`].

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSPasteboardWriting, NSView, NSWorkspace,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
use crate::bridge_winit::winit::window::WindowId;
use remedy_ui_core::host::{DragData, DropEffect, Unsupported, Waker};

thread_local! {
    static ENDED: RefCell<Vec<(WindowId, DropEffect)>> = const { RefCell::new(Vec::new()) };
    static WAKER: RefCell<Option<Waker>> = const { RefCell::new(None) };
}

/// Wakes the event loop when a drag ends.
pub(crate) fn set_waker(w: Waker) {
    WAKER.with(|slot| *slot.borrow_mut() = Some(w));
}

/// Drags out that ended since the last call.
pub(crate) fn take_ended() -> Vec<(WindowId, DropEffect)> {
    ENDED.with(|v| std::mem::take(&mut *v.borrow_mut()))
}

fn mask(allowed: &[DropEffect]) -> NSDragOperation {
    let mut m = NSDragOperation::None;
    for e in allowed {
        m |= match e {
            DropEffect::None => NSDragOperation::None,
            DropEffect::Copy => NSDragOperation::Copy,
            DropEffect::Move => NSDragOperation::Move,
            DropEffect::Link => NSDragOperation::Link,
        };
    }
    m
}

fn effect_of(op: NSDragOperation) -> DropEffect {
    if op.contains(NSDragOperation::Move) {
        DropEffect::Move
    } else if op.contains(NSDragOperation::Copy) || op.contains(NSDragOperation::Generic) {
        DropEffect::Copy
    } else if op.contains(NSDragOperation::Link) {
        DropEffect::Link
    } else {
        DropEffect::None
    }
}

/// What the source remembers: the window the drag left, and what it allows.
struct Ivars {
    window: WindowId,
    allowed: NSDragOperation,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the ivars need no Drop ordering.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RemedyUIDragSource"]
    #[ivars = Ivars]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}

    unsafe impl NSDraggingSource for Source {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operations(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            self.ivars().allowed
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _at: NSPoint, operation: NSDragOperation) {
            let window = self.ivars().window;
            ENDED.with(|v| v.borrow_mut().push((window, effect_of(operation))));
            if let Some(w) = WAKER.with(|w| w.borrow().clone()) {
                w();
            }
        }
    }
);

impl Source {
    fn new(mtm: MainThreadMarker, window: WindowId, allowed: NSDragOperation) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { window, allowed });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Starts dragging `data` out of the window whose content view is `ns_view`. Call while the
/// mouse button that began the drag is down (AppKit needs that mouse event).
pub(crate) fn drag_out(
    ns_view: std::ptr::NonNull<std::ffi::c_void>,
    window: WindowId,
    data: &DragData,
    allowed: &[DropEffect],
) -> Result<(), Unsupported> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| Unsupported::new("dragging out", "not on the main thread"))?;
    // SAFETY: winit's AppKit handle points to the window's live content NSView, used on the
    // main thread where the event loop runs.
    let view: &NSView = unsafe { ns_view.cast::<NSView>().as_ref() };
    let app = NSApplication::sharedApplication(mtm);
    let event =
        app.currentEvent().ok_or_else(|| Unsupported::new("dragging out", "no mouse event to start from"))?;
    let at = view.convertPoint_fromView(event.locationInWindow(), None);
    let ws = NSWorkspace::sharedWorkspace();
    let mut items: Vec<Retained<NSDraggingItem>> = Vec::new();
    let mut place = |writer: &ProtocolObject<dyn NSPasteboardWriting>,
                     icon: Option<Retained<objc2_app_kit::NSImage>>| {
        let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);
        let n = items.len() as f64;
        let frame =
            NSRect::new(NSPoint::new(at.x - 16.0 + n * 4.0, at.y - 16.0 - n * 4.0), NSSize::new(32.0, 32.0));
        // SAFETY: the contents are an NSImage or nothing, as the method allows.
        unsafe {
            item.setDraggingFrame_contents(frame, icon.as_deref().map(|i| &**i as &objc2::runtime::AnyObject))
        };
        items.push(item);
    };
    for f in &data.files {
        let path = NSString::from_str(&f.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        place(ProtocolObject::from_ref(&*url), Some(ws.iconForFile(&path)));
    }
    for u in &data.uris {
        if let Some(url) = NSURL::URLWithString(&NSString::from_str(u)) {
            place(ProtocolObject::from_ref(&*url), None);
        }
    }
    if let Some(t) = &data.text {
        let s = NSString::from_str(t);
        place(ProtocolObject::from_ref(&*s), None);
    }
    if items.is_empty() {
        return Err(Unsupported::new("dragging out", "nothing to drag"));
    }
    let source = Source::new(mtm, window, mask(allowed));
    let list = NSArray::from_retained_slice(&items);
    // AppKit keeps the source for the session's life.
    let _session =
        view.beginDraggingSessionWithItems_event_source(&list, &event, ProtocolObject::from_ref(&*source));
    Ok(())
}
