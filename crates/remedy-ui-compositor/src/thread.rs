//! The compositor thread: scrolling and compositor animations keep the display's rate even
//! when the UI thread is busy, because nothing here waits on it.
//!
//! Natively it is a thread of its own. In a browser page without threads it is a cooperative
//! task on the page's thread ([`remedy_ui_core::exec`]) that runs between input and frames,
//! with the same messages, order and reports.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use remedy_ui_core::exec::Pool;
use remedy_ui_core::{Point, Vec2};

use crate::{Compositor, FramePlan, LayerId, LayerTree, Mailbox, TileRequest};

/// What the compositor thread hands each frame to: a renderer plus a surface.
pub trait Presenter: Send {
    /// Rasters and presents `plan`. Called only for frames that changed something.
    fn present(&mut self, plan: &FramePlan);
}

enum Msg {
    Vsync(Duration),
    ScrollBy(LayerId, Vec2),
    ScrollAt(Point, Vec2),
    Fling(LayerId, Vec2),
    ScrollTo(LayerId, Vec2),
    ScrollStep(LayerId, Vec2),
    ScrollToExact(LayerId, Vec2),
    Resize(u32, u32, f32),
    TrimMemory,
    Stop(Sender<Compositor>),
}

/// What a vsync produced, reported back to the UI thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameReport {
    /// The presentation time of the frame.
    pub at: Duration,
    /// True when it presented; false when nothing changed and nothing was presented.
    pub presented: bool,
    /// True while scrolling or animations continue: keep sending vsyncs.
    pub animating: bool,
}

/// A running compositor thread. Dropping it stops the thread.
pub struct CompositorThread {
    commits: Arc<Mailbox<LayerTree>>,
    tx: Sender<Msg>,
    reports: Receiver<FrameReport>,
    worker: Pool,
}

impl std::fmt::Debug for CompositorThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CompositorThread")
    }
}

/// What the compositor thread owns while it runs.
struct Running<P> {
    /// `None` once stopped and handed back.
    compositor: Option<Compositor>,
    presenter: P,
    commits: Arc<Mailbox<LayerTree>>,
    rx: Receiver<Msg>,
    reports: Sender<FrameReport>,
}

impl<P: Presenter> Running<P> {
    /// Handles the next message; false when none is waiting (or the thread has stopped).
    fn step(&mut self) -> bool {
        let Some(compositor) = self.compositor.as_mut() else { return false };
        let msg = match self.rx.try_recv() {
            Ok(m) => m,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return false,
        };
        match msg {
            Msg::Vsync(at) => {
                if let Some(tree) = self.commits.take() {
                    compositor.commit(tree);
                }
                let plan = compositor.frame(at);
                let changed = !plan.damage.is_empty() || !plan.raster.is_empty() || !plan.released.is_empty();
                if changed {
                    let started = remedy_ui_core::time::Instant::now();
                    let present = remedy_ui_core::phase!("present").entered();
                    self.presenter.present(&plan);
                    drop(present);
                    // What the frame cost sizes the zoom budget (compositing included, so
                    // the estimate errs on the side of smooth frames).
                    let pixels: u64 = plan.raster.iter().map(TileRequest::pixels).sum();
                    compositor.record_raster(pixels, started.elapsed());
                }
                let _ = self.reports.send(FrameReport { at, presented: changed, animating: plan.animating });
            }
            Msg::ScrollBy(id, d) => {
                compositor.scroll_by(id, d);
            }
            Msg::ScrollAt(p, d) => {
                compositor.wheel_at(p, d);
            }
            Msg::ScrollTo(id, o) => {
                compositor.scroll_to_snapped(id, o);
            }
            Msg::ScrollStep(id, o) => {
                compositor.scroll_step(id, o);
            }
            Msg::ScrollToExact(id, o) => {
                compositor.set_scroll_offset(id, o);
            }
            Msg::Fling(id, v) => {
                compositor.fling(id, v);
            }
            Msg::Resize(w, h, s) => compositor.resize(w, h, s),
            Msg::TrimMemory => compositor.trim_memory(),
            Msg::Stop(back) => {
                if let Some(c) = self.compositor.take() {
                    let _ = back.send(c);
                }
                return false;
            }
        }
        true
    }
}

impl CompositorThread {
    /// Starts a thread that owns `compositor` and presents through `presenter` (a
    /// cooperative task in a browser page).
    pub fn spawn(compositor: Compositor, presenter: impl Presenter + 'static) -> std::io::Result<Self> {
        let commits = Arc::new(Mailbox::new());
        let (tx, rx) = channel();
        let (report_tx, reports) = channel();
        let running = Mutex::new(Running {
            compositor: Some(compositor),
            presenter,
            commits: commits.clone(),
            rx,
            reports: report_tx,
        });
        // One worker: messages are handled in the order they were sent.
        let worker = Pool::new("remedy-ui compositor", 1, move || {
            running.lock().unwrap_or_else(PoisonError::into_inner).step()
        });
        Ok(CompositorThread { commits, tx, reports, worker })
    }

    fn send(&self, msg: Msg) {
        if self.tx.send(msg).is_ok() {
            self.worker.notify();
        }
    }

    /// Hands over a new layer tree. It replaces one the compositor has not taken yet; the UI
    /// thread never waits.
    pub fn commit(&self, tree: LayerTree) {
        self.commits.post(tree);
    }

    /// A display refresh: draw the frame presented at `at`.
    pub fn vsync(&self, at: Duration) {
        self.send(Msg::Vsync(at));
    }

    /// Scrolls a layer by content units.
    pub fn scroll_by(&self, layer: LayerId, delta: Vec2) {
        self.send(Msg::ScrollBy(layer, delta));
    }

    /// Routes a wheel scroll at a device-pixel point ([`Compositor::wheel_at`]: a snapping
    /// container settles once the wheel is still).
    pub fn scroll_at(&self, point: Point, delta: Vec2) {
        self.send(Msg::ScrollAt(point, delta));
    }

    /// Scrolls a layer to `offset`, a snapping one to its nearest snap point
    /// ([`Compositor::scroll_to_snapped`]).
    pub fn scroll_to(&self, layer: LayerId, offset: Vec2) {
        self.send(Msg::ScrollTo(layer, offset));
    }

    /// Steps a layer toward `offset` as a key or focus move does ([`Compositor::scroll_step`]).
    pub fn scroll_step(&self, layer: LayerId, offset: Vec2) {
        self.send(Msg::ScrollStep(layer, offset));
    }

    /// Scrolls a layer to exactly `offset` ([`Compositor::set_scroll_offset`]).
    pub fn scroll_to_exact(&self, layer: LayerId, offset: Vec2) {
        self.send(Msg::ScrollToExact(layer, offset));
    }

    /// Starts a fling.
    pub fn fling(&self, layer: LayerId, velocity: Vec2) {
        self.send(Msg::Fling(layer, velocity));
    }

    /// The window changed size or scale.
    pub fn resize(&self, width: u32, height: u32, scale: f32) {
        self.send(Msg::Resize(width, height, scale));
    }

    /// Frees off-screen tiles.
    pub fn trim_memory(&self) {
        self.send(Msg::TrimMemory);
    }

    /// Reports of frames drawn since the last call, oldest first.
    pub fn reports(&self) -> Vec<FrameReport> {
        self.reports.try_iter().collect()
    }

    /// Waits for the next frame report, up to `timeout` (in a browser page, by running the
    /// compositor's queued work).
    pub fn wait_report(&self, timeout: Duration) -> Option<FrameReport> {
        remedy_ui_core::exec::recv_timeout(&self.reports, timeout).ok()
    }

    /// Stops the thread and returns its compositor, once the messages sent before have been
    /// handled.
    pub fn stop(self) -> Option<Compositor> {
        let (back, compositor) = channel();
        self.send(Msg::Stop(back));
        remedy_ui_core::exec::recv(&compositor).ok()
    }
}

impl Drop for CompositorThread {
    fn drop(&mut self) {
        // The worker hands the compositor to nobody and leaves; `stop` already did if called.
        let (back, _) = channel();
        self.send(Msg::Stop(back));
    }
}
