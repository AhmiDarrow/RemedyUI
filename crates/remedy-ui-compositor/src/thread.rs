//! The compositor thread: scrolling and compositor animations keep the display's rate even
//! when the UI thread is busy, because nothing here waits on it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use remedy_ui_core::{Point, Vec2};

use crate::{Compositor, FramePlan, LayerId, LayerTree, Mailbox};

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
    Resize(u32, u32, f32),
    TrimMemory,
    Stop,
}

/// What a vsync produced, reported back to the UI thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    handle: Option<JoinHandle<Compositor>>,
}

impl std::fmt::Debug for CompositorThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CompositorThread")
    }
}

fn run(
    mut compositor: Compositor,
    mut presenter: impl Presenter,
    commits: Arc<Mailbox<LayerTree>>,
    rx: Receiver<Msg>,
    reports: Sender<FrameReport>,
) -> Compositor {
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Vsync(at) => {
                if let Some(tree) = commits.take() {
                    compositor.commit(tree);
                }
                let plan = compositor.frame(at);
                let changed = !plan.damage.is_empty() || !plan.raster.is_empty() || !plan.released.is_empty();
                if changed {
                    presenter.present(&plan);
                }
                let _ = reports.send(FrameReport { at, presented: changed, animating: plan.animating });
            }
            Msg::ScrollBy(id, d) => {
                compositor.scroll_by(id, d);
            }
            Msg::ScrollAt(p, d) => {
                compositor.scroll_at(p, d);
            }
            Msg::Fling(id, v) => {
                compositor.fling(id, v);
            }
            Msg::Resize(w, h, s) => compositor.resize(w, h, s),
            Msg::TrimMemory => compositor.trim_memory(),
            Msg::Stop => break,
        }
    }
    compositor
}

impl CompositorThread {
    /// Starts a thread that owns `compositor` and presents through `presenter`.
    pub fn spawn(compositor: Compositor, presenter: impl Presenter + 'static) -> std::io::Result<Self> {
        let commits = Arc::new(Mailbox::new());
        let (tx, rx) = channel();
        let (report_tx, reports) = channel();
        let mailbox = commits.clone();
        let handle = std::thread::Builder::new()
            .name("remedy-ui compositor".into())
            .spawn(move || run(compositor, presenter, mailbox, rx, report_tx))?;
        Ok(CompositorThread { commits, tx, reports, handle: Some(handle) })
    }

    /// Hands over a new layer tree. It replaces one the compositor has not taken yet; the UI
    /// thread never waits.
    pub fn commit(&self, tree: LayerTree) {
        self.commits.post(tree);
    }

    /// A display refresh: draw the frame presented at `at`.
    pub fn vsync(&self, at: Duration) {
        let _ = self.tx.send(Msg::Vsync(at));
    }

    /// Scrolls a layer by content units.
    pub fn scroll_by(&self, layer: LayerId, delta: Vec2) {
        let _ = self.tx.send(Msg::ScrollBy(layer, delta));
    }

    /// Routes a wheel scroll at a device-pixel point.
    pub fn scroll_at(&self, point: Point, delta: Vec2) {
        let _ = self.tx.send(Msg::ScrollAt(point, delta));
    }

    /// Starts a fling.
    pub fn fling(&self, layer: LayerId, velocity: Vec2) {
        let _ = self.tx.send(Msg::Fling(layer, velocity));
    }

    /// The window changed size or scale.
    pub fn resize(&self, width: u32, height: u32, scale: f32) {
        let _ = self.tx.send(Msg::Resize(width, height, scale));
    }

    /// Frees off-screen tiles.
    pub fn trim_memory(&self) {
        let _ = self.tx.send(Msg::TrimMemory);
    }

    /// Reports of frames drawn since the last call, oldest first.
    pub fn reports(&self) -> Vec<FrameReport> {
        self.reports.try_iter().collect()
    }

    /// Waits for the next frame report, up to `timeout`.
    pub fn wait_report(&self, timeout: Duration) -> Option<FrameReport> {
        self.reports.recv_timeout(timeout).ok()
    }

    /// Stops the thread and returns its compositor.
    pub fn stop(mut self) -> Option<Compositor> {
        let _ = self.tx.send(Msg::Stop);
        self.handle.take().and_then(|h| h.join().ok())
    }
}

impl Drop for CompositorThread {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
