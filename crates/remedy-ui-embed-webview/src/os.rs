//! The platform web views: the crate's only unsafe code, wrapping each platform's calls in
//! [`Native`], which the element drives.
//!
//! Every platform's `Native` offers the same safe surface: `new` (from the window's native
//! handles), `command`, `place`, `native_view` and `close`.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::Native;

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod linux;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub(crate) use linux::Native;

#[cfg(not(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
mod unsupported;
#[cfg(not(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
pub(crate) use unsupported::Native;

/// A rectangle in whole device pixels: `(left, top, right, bottom)`.
#[cfg_attr(
    not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))),
    allow(dead_code)
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PxRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

#[cfg_attr(
    not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))),
    allow(dead_code)
)]
impl PxRect {
    /// `r` (logical pixels) at `scale`, to whole device pixels.
    pub(crate) fn from_logical(r: remedy_ui_core::Rect, scale: f32) -> PxRect {
        let px = |v: f32| {
            let v = (v * scale).round();
            if v.is_finite() { v.clamp(i32::MIN as f32, i32::MAX as f32) as i32 } else { 0 }
        };
        PxRect { left: px(r.min.x), top: px(r.min.y), right: px(r.max.x), bottom: px(r.max.y) }
    }

    pub(crate) fn width(&self) -> i32 {
        self.right.saturating_sub(self.left).max(0)
    }

    pub(crate) fn height(&self) -> i32 {
        self.bottom.saturating_sub(self.top).max(0)
    }

    /// The same rectangle relative to `origin`'s top-left corner.
    pub(crate) fn relative_to(&self, origin: &PxRect) -> PxRect {
        PxRect {
            left: self.left.saturating_sub(origin.left),
            top: self.top.saturating_sub(origin.top),
            right: self.right.saturating_sub(origin.left),
            bottom: self.bottom.saturating_sub(origin.top),
        }
    }

    pub(crate) fn to_rect(self) -> remedy_ui_core::Rect {
        remedy_ui_core::Rect::new(self.left as f32, self.top as f32, self.right as f32, self.bottom as f32)
    }
}

/// Where the native view goes for a placement: the view's own box (the part of the element
/// that shows), the page's box relative to it, and the overlays to cut out, relative to it.
#[cfg_attr(
    not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))),
    allow(dead_code)
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) frame: PxRect,
    pub(crate) content: PxRect,
    pub(crate) holes: Vec<PxRect>,
}

#[cfg_attr(
    not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))),
    allow(dead_code)
)]
impl Layout {
    /// `None` when nothing of the element shows (or what shows rounds to no pixels).
    pub(crate) fn of(p: Option<&remedy_ui_view::Placement>) -> Option<Layout> {
        let p = p?;
        let frame = PxRect::from_logical(p.visible, p.scale);
        if frame.width() == 0 || frame.height() == 0 {
            return None;
        }
        let content = PxRect::from_logical(p.bounds, p.scale).relative_to(&frame);
        let holes = p
            .covered
            .iter()
            .map(|c| PxRect::from_logical(*c, p.scale).relative_to(&frame))
            .filter(|h| h.width() > 0 && h.height() > 0)
            .collect();
        Some(Layout { frame, content, holes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_core::Rect;

    #[test]
    fn a_scrolled_element_maps_to_its_visible_part_and_its_whole_box() {
        let p = remedy_ui_view::Placement {
            bounds: Rect::new(10.0, -20.0, 210.0, 80.0),
            visible: Rect::new(10.0, 0.0, 210.0, 80.0),
            covered: vec![Rect::new(100.0, 10.0, 150.0, 30.0)],
            scale: 1.5,
        };
        let l = Layout::of(Some(&p)).unwrap_or_else(|| unreachable!("shown"));
        assert_eq!(l.frame, PxRect { left: 15, top: 0, right: 315, bottom: 120 });
        assert_eq!(l.content, PxRect { left: 0, top: -30, right: 300, bottom: 120 });
        assert_eq!(l.holes, vec![PxRect { left: 135, top: 15, right: 210, bottom: 45 }]);
        assert_eq!(Layout::of(None), None);
    }
}
