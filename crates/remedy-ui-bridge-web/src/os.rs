//! The one unsafe call of the web bridge: lending a window's canvas to wgpu as a raw handle.
//!
//! raw-window-handle's `WindowHandle::borrow_raw` is `unsafe` because a handle must stay valid
//! for as long as it is borrowed. A web handle is only a number: wgpu looks the canvas up by
//! its `data-raw-handle` attribute when it makes a surface, and the bridge keeps that
//! attribute on the window's GPU canvas for the window's whole life, so the number names that
//! canvas as long as anything can use it.

use remedy_ui_core::host::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    WebDisplayHandle, WebWindowHandle, WindowHandle,
};

/// A window's GPU canvas, by the `data-raw-handle` id it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CanvasHandle {
    pub(crate) id: u32,
}

impl HasWindowHandle for CanvasHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Web(WebWindowHandle::new(self.id));
        // SAFETY: a web handle carries no pointer, only the id of a canvas the bridge keeps
        // marked with it while the window exists (see the module documentation).
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for CanvasHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Web(WebDisplayHandle::new());
        // SAFETY: the web display handle is a marker with no data.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
