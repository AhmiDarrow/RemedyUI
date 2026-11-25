//! What content the platform or the GPU draws for an element needs from the view: the native
//! window, which renderer draws it, and where the element is on it.

use std::rc::Rc;
use std::sync::Arc;

use remedy_ui_core::host::WindowHandles;
use remedy_ui_core::tree::Dirty;

use crate::view::element::{El, Ui};
use crate::view::{Placement, Request};

impl Ui {
    /// Tells the view which native window it draws into (the runner does, once the window
    /// exists).
    pub fn set_window_handles(&self, handles: Option<Arc<dyn WindowHandles>>) {
        self.0.borrow_mut().window_handles = handles;
    }

    /// The native window this view draws into: the parent for content the platform draws
    /// itself, such as an embedded web view. `None` on hosts without native windows.
    pub fn window_handles(&self) -> Option<Arc<dyn WindowHandles>> {
        self.0.borrow().window_handles.clone()
    }

    /// Tells the view whether the GPU renderer draws its window now (the runner does when
    /// it picks or changes the renderer).
    pub fn set_gpu_rendering(&self, on: bool) {
        let s = self.0.borrow().gpu;
        s.set_if_changed(on);
    }

    /// True while the GPU renderer draws this window, as a signal: a painter that reads it
    /// (it may, unlike the view itself, while the frame runs) records again when the window
    /// moves between renderers.
    pub fn gpu_rendering(&self) -> remedy_ui_core::reactive::Signal<bool> {
        self.0.borrow().gpu
    }

    /// Asks the runner to draw this window with the GPU renderer, where the app left the
    /// choice to it ([`Request::PreferGpu`]).
    pub fn prefer_gpu(&self) {
        self.request(Request::PreferGpu);
    }
}

impl El {
    /// Calls `f` with where the element is on its window after each frame in which that
    /// changed, including the first: its box, the part of it that shows, and the parts
    /// overlays draw over; `None` while none of it is seen (hidden, scrolled away, clipped
    /// out, or the window covered). Content the platform draws itself over the window (an
    /// embedded web view) follows its element with it.
    pub fn on_placed(self, f: impl Fn(Option<&Placement>) + 'static) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.on_placed = Some(Rc::new(f));
            d.last_placed = None;
        }
        self.ui.mark(self.id, Dirty::LAYOUT);
        self.ui.request(Request::Frame);
        self
    }
}
