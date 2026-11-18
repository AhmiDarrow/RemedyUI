//! Video surfaces for RemedyUI apps: a screen share, a camera, a call's tiles, a player's
//! picture. The app's decoder or capture pushes frames from any thread into a [`VideoSink`];
//! the element shows the newest one at the display's rate. Frames stay in the layout they
//! come in (NV12, I420 or RGBA) all the way to the renderer, which converts them on the GPU
//! (the CPU renderer converts them itself).
//!
//! Frames are never queued: a frame that arrives before the last one was shown replaces it
//! and counts as dropped, so a slow window never falls behind a live stream.
//!
//! Layer 5: built only from public engine APIs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Rect};
use remedy_ui_paint::{Brush, Item};
pub use remedy_ui_paint::{PixelFormat, Plane, VideoFrame, YuvMatrix};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

/// How the picture fills its element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoFit {
    /// All of it, as large as fits, centered, with bars where the shapes differ.
    #[default]
    Contain,
    /// The whole element, cropping what does not fit (a call's tile).
    Cover,
    /// Stretched to the element.
    Fill,
}

/// How a surface has been doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoStats {
    /// Frames pushed.
    pub pushed: u64,
    /// Frames drawn.
    pub shown: u64,
    /// Frames replaced by a newer one before they were drawn, or refused as malformed.
    pub dropped: u64,
}

/// What a sink and its element share.
struct Shared {
    stream: u64,
    /// The newest frame not yet taken by the element.
    pending: Mutex<Option<VideoFrame>>,
    /// A wake to the UI thread is on its way.
    woken: AtomicBool,
    /// The element is gone.
    closed: AtomicBool,
    next_id: AtomicU64,
    pushed: AtomicU64,
    shown: AtomicU64,
    dropped: AtomicU64,
}

/// Where frames go: cheap to clone, and usable from any thread.
#[derive(Clone)]
pub struct VideoSink {
    shared: Arc<Shared>,
    waker: Option<remedy_ui_reactive::Waker>,
}

thread_local! {
    /// Each surface on this (UI) thread, by stream: what takes its newest frame. The wake a
    /// sink posts carries only the stream number, so nothing tied to the UI thread crosses.
    static SURFACES: RefCell<HashMap<u64, Weak<dyn Fn()>>> = RefCell::new(HashMap::new());
}

/// Runs on the UI thread: the surface for `stream` takes its newest frame.
fn deliver(stream: u64) {
    let take = SURFACES.with(|s| s.borrow().get(&stream).and_then(Weak::upgrade));
    if let Some(take) = take {
        take();
    }
}

impl std::fmt::Debug for VideoSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VideoSink({}, {:?})", self.shared.stream, self.stats())
    }
}

/// A picture to push: the frame's bytes and how they are laid out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixel layout.
    pub format: PixelFormat,
    /// Where each plane starts and its row stride; [`Picture::packed`] fills these in for
    /// planes stored back to back.
    pub planes: [Plane; 3],
    /// The YUV matrix (ignored for RGBA).
    pub matrix: YuvMatrix,
    /// Full-range YUV rather than the video range.
    pub full_range: bool,
    /// The bytes, shared without copying.
    pub data: Arc<[u8]>,
}

impl Picture {
    /// A picture whose planes are stored back to back with no row padding (what most
    /// software decoders hand out), video-range BT.709.
    pub fn packed(width: u32, height: u32, format: PixelFormat, data: impl Into<Arc<[u8]>>) -> Self {
        let f = VideoFrame::packed(0, 0, width, height, format, data.into());
        Picture {
            width,
            height,
            format,
            planes: f.planes,
            matrix: f.matrix,
            full_range: f.full_range,
            data: f.data,
        }
    }
}

impl VideoSink {
    /// Shows `picture` as soon as the window draws next, replacing one still waiting. Returns
    /// false when the element is gone (stop sending) or the picture's planes do not fit its
    /// bytes (it is dropped).
    pub fn push(&self, picture: Picture) -> bool {
        let s = &self.shared;
        if s.closed.load(Ordering::Acquire) {
            return false;
        }
        s.pushed.fetch_add(1, Ordering::Relaxed);
        let frame = VideoFrame {
            stream: s.stream,
            id: s.next_id.fetch_add(1, Ordering::Relaxed),
            width: picture.width,
            height: picture.height,
            format: picture.format,
            matrix: picture.matrix,
            full_range: picture.full_range,
            planes: picture.planes,
            data: picture.data,
        };
        if !frame.is_valid() {
            s.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let replaced = s.pending.lock().unwrap_or_else(PoisonError::into_inner).replace(frame);
        if replaced.is_some() {
            s.dropped.fetch_add(1, Ordering::Relaxed);
        }
        // One wake on its way is enough: the element takes the newest frame when it runs.
        if !s.woken.swap(true, Ordering::AcqRel) {
            let stream = s.stream;
            let posted = self.waker.as_ref().is_some_and(|w| w.post(move || deliver(stream)).is_ok());
            if !posted {
                // The UI thread is gone: nothing will show this or any later frame.
                s.closed.store(true, Ordering::Release);
                return false;
            }
        }
        true
    }

    /// How the surface has been doing.
    pub fn stats(&self) -> VideoStats {
        let s = &self.shared;
        VideoStats {
            pushed: s.pushed.load(Ordering::Relaxed),
            shown: s.shown.load(Ordering::Relaxed),
            dropped: s.dropped.load(Ordering::Relaxed),
        }
    }

    /// False once the element is gone.
    pub fn is_open(&self) -> bool {
        !self.shared.closed.load(Ordering::Acquire)
    }
}

/// An element showing the frames pushed into the returned sink. `label` says what it shows
/// for screen readers ("Alex's screen"); the element reads as an image with that name.
#[track_caller]
pub fn video_surface(ui: &Ui, label: impl Into<String>, fit: VideoFit) -> (El, VideoSink) {
    static STREAMS: AtomicU64 = AtomicU64::new(1);
    let shared = Arc::new(Shared {
        stream: STREAMS.fetch_add(1, Ordering::Relaxed),
        pending: Mutex::new(None),
        woken: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        next_id: AtomicU64::new(1),
        pushed: AtomicU64::new(0),
        shown: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
    });
    // The frame drawn now, read by the painter: a new one repaints only this element.
    let current: Signal<Option<VideoFrame>> = Signal::new(None);
    let take = {
        let shared = shared.clone();
        move || {
            shared.woken.store(false, Ordering::Release);
            let next = shared.pending.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(f) = next {
                current.set(Some(f));
            }
        }
    };
    let take: Rc<dyn Fn()> = Rc::new(take);
    SURFACES.with(|s| s.borrow_mut().insert(shared.stream, Rc::downgrade(&take)));
    let waker = remedy_ui_reactive::waker().ok();
    let label = label.into();
    let counted = shared.clone();
    let closing = shared.clone();
    // The last frame drawn: a repaint for another reason does not count as showing a frame.
    let last_shown = std::cell::Cell::new(0u64);
    let el = ui
        .el("video_surface")
        .grow(1.0)
        .semantics(Semantics::new(Role::Image).label(label))
        // The element keeps the frame taker alive; with the element, it and the stream go.
        .on_removed(move || {
            closing.closed.store(true, Ordering::Release);
            SURFACES.with(|s| s.borrow_mut().remove(&closing.stream));
            drop(take);
        })
        .paint(move |cx| {
            let Some(frame) = current.get() else { return };
            let frame_id = frame.id;
            let (w, h) = (cx.size.width, cx.size.height);
            let (fw, fh) = (frame.width as f32, frame.height as f32);
            let s = match fit {
                VideoFit::Contain => (w / fw).min(h / fh),
                VideoFit::Cover => (w / fw).max(h / fh),
                VideoFit::Fill => 0.0,
            };
            let rect = if fit == VideoFit::Fill {
                Rect::new(0.0, 0.0, w, h)
            } else {
                let (dw, dh) = (fw * s, fh * s);
                Rect::new((w - dw) / 2.0, (h - dh) / 2.0, (w + dw) / 2.0, (h + dh) / 2.0)
            };
            let radii = CornerRadii::all(cx.style.radius);
            let whole = Rect::new(0.0, 0.0, w, h);
            // Bars where the picture's shape differs, in the element's own background.
            cx.list.fill_rounded(whole, radii, Brush::Solid(cx.style.background));
            cx.list.push_clip(whole, radii);
            cx.list.push(Item::Video { frame, rect, radii: CornerRadii::ZERO });
            cx.list.pop_clip();
            if last_shown.replace(frame_id) != frame_id {
                counted.shown.fetch_add(1, Ordering::Relaxed);
            }
        });
    (el, VideoSink { shared, waker })
}
