//! What RemedyUI has learned about this machine's graphics backends: which passed the start-up
//! self-test, which drew wrong, and which hung presenting. Remembered per adapter, driver and
//! backend (and engine version), in memory and, once an app names a folder
//! ([`remember_in`]), on disk: the self-test runs once per driver, and a backend that hung is
//! skipped from then on, the next start included.
//!
//! - **The self-test** ([`self_test`]) draws a small scene that exercises what interfaces use
//!   (fills, rounded edges, gradients, images, a blur, a colour matrix) on the device and on
//!   the CPU reference renderer and compares them. The backend ladder runs it on each rung
//!   that gives a device ([`LadderOptions::self_test`](crate::LadderOptions)) and moves down
//!   a rung when it fails: a driver that draws garbage is never shown to anyone.
//! - **The present watchdog** ([`PresentWatchdog`]) watches each present from a thread of its
//!   own. A present running past [`PRESENT_HANG`] is recorded as a hang at once (so a hang that
//!   never returns still demotes the backend at the next start) and the device is treated as
//!   lost: the compositor's owner rebuilds it from the ladder, which skips the hung rung.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::{GpuContext, GpuError};

/// How long a present may run before the backend counts as hung. Windows resets a GPU that
/// does not answer for two seconds; a present this long is not a slow frame.
pub const PRESENT_HANG: Duration = Duration::from_secs(3);

/// What a backend did here.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// It drew the self-test right.
    Passed,
    /// It drew the self-test wrong (what was wrong).
    Failed(String),
    /// A present hung.
    Hung,
}

impl Verdict {
    /// True when the ladder may use the backend.
    pub fn usable(&self) -> bool {
        matches!(self, Verdict::Passed)
    }

    /// Why the ladder passes over a backend with this verdict, in words.
    pub fn reason(&self) -> String {
        match self {
            Verdict::Passed => "it passed its self-test".into(),
            Verdict::Failed(why) => format!("it drew its self-test wrong here ({why})"),
            Verdict::Hung => "a present hung on it here".into(),
        }
    }

    fn word(&self) -> String {
        match self {
            Verdict::Passed => "passed".into(),
            Verdict::Failed(why) => format!("failed {}", why.replace(['\n', '\t'], " ")),
            Verdict::Hung => "hung".into(),
        }
    }

    fn parse(s: &str) -> Option<Verdict> {
        match s.split_once(' ').map_or((s, ""), |(a, b)| (a, b)) {
            ("passed", _) => Some(Verdict::Passed),
            ("hung", _) => Some(Verdict::Hung),
            ("failed", why) => Some(Verdict::Failed(why.into())),
            _ => None,
        }
    }
}

/// The most backends remembered: a machine has a handful; a hand-edited file cannot grow it.
const MAX_ENTRIES: usize = 64;
/// The longest line read from the file.
const MAX_LINE: usize = 512;
const FILE: &str = "gpu-health.txt";

struct Health {
    verdicts: HashMap<String, Verdict>,
    dir: Option<PathBuf>,
    tests_run: u64,
}

static HEALTH: Mutex<Option<Health>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Health) -> T) -> T {
    let mut slot = HEALTH.lock().unwrap_or_else(PoisonError::into_inner);
    let h = slot.get_or_insert_with(|| Health { verdicts: HashMap::new(), dir: None, tests_run: 0 });
    f(h)
}

/// The key a backend is remembered by: the adapter, its driver and the backend, and this
/// engine's version (a new version tests again).
pub fn key(ctx: &GpuContext) -> String {
    let i = ctx.adapter.get_info();
    let raw = format!(
        "{} {:04x}:{:04x} {:?} {} {} {:?} v{}",
        i.name,
        i.vendor,
        i.device,
        i.backend,
        i.driver,
        i.driver_info,
        i.device_type,
        env!("CARGO_PKG_VERSION")
    );
    raw.chars().map(|c| if c.is_control() || c == '\t' { ' ' } else { c }).take(MAX_LINE / 2).collect()
}

/// Keeps verdicts in `dir` (an app's data or cache folder) from now on, reading what an
/// earlier run left there. Verdicts already known in this process are kept too.
pub fn remember_in(dir: impl Into<PathBuf>) {
    let dir = dir.into();
    let read = read_file(&dir);
    with(|h| {
        for (k, v) in read {
            h.verdicts.entry(k).or_insert(v);
        }
        h.dir = Some(dir);
        write_file(h);
    });
}

/// What is known of the backend `key`, if anything.
pub fn verdict(key: &str) -> Option<Verdict> {
    with(|h| h.verdicts.get(key).cloned())
}

/// Records `verdict` for the backend `key` (written through when a folder is set).
pub fn record(key: &str, verdict: Verdict) {
    with(|h| {
        if h.verdicts.len() >= MAX_ENTRIES && !h.verdicts.contains_key(key) {
            return;
        }
        h.verdicts.insert(key.to_string(), verdict);
        write_file(h);
    });
}

/// Forgets what is known of the backend `key` (after a driver update the person reports
/// fixed a hang; tools and tests).
pub fn forget(key: &str) {
    with(|h| {
        h.verdicts.remove(key);
        write_file(h);
    });
}

/// How many self-tests this process has run (each backend once).
pub fn self_tests_run() -> u64 {
    with(|h| h.tests_run)
}

fn read_file(dir: &std::path::Path) -> Vec<(String, Verdict)> {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE)) else { return Vec::new() };
    text.lines()
        .filter(|l| l.len() <= MAX_LINE)
        .filter_map(|l| {
            let (k, v) = l.split_once('\t')?;
            Some((k.to_string(), Verdict::parse(v)?))
        })
        .take(MAX_ENTRIES)
        .collect()
}

fn write_file(h: &Health) {
    let Some(dir) = &h.dir else { return };
    let mut lines: Vec<String> = h.verdicts.iter().map(|(k, v)| format!("{k}\t{}", v.word())).collect();
    lines.sort();
    let body = lines.join("\n") + "\n";
    let path = dir.join(FILE);
    let tmp = path.with_extension("tmp");
    // Best effort: a read-only folder only costs testing again next start.
    let _ = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&tmp, body))
        .and_then(|()| std::fs::rename(&tmp, &path));
}

/// The self-test scene, 64 × 64 at scale 1: what interfaces draw most.
fn scene() -> remedy_ui_paint::DisplayList {
    use remedy_ui_core::{Color, ColorMatrix, CornerRadii, Point, Rect};
    use remedy_ui_paint::{Brush, DisplayList, Filter, Image, Item, Sampling, Stop};
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(0.0, 0.0, 32.0, 32.0), Brush::Solid(Color::rgb8(230, 40, 30)));
    l.fill_rect(Rect::new(32.0, 0.0, 64.0, 32.0), Brush::Solid(Color::rgb8(30, 180, 60)));
    l.fill_rounded(
        Rect::new(4.0, 36.0, 28.0, 60.0),
        CornerRadii::all(8.0),
        Brush::Solid(Color::rgb8(40, 90, 230)),
    );
    l.fill_rect(
        Rect::new(32.0, 32.0, 64.0, 48.0),
        Brush::Linear {
            start: Point::new(32.0, 0.0),
            end: Point::new(64.0, 0.0),
            stops: vec![Stop { offset: 0.0, color: Color::BLACK }, Stop { offset: 1.0, color: Color::WHITE }],
        },
    );
    let mut px = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u8 {
        for x in 0..16u8 {
            px.extend([x * 16, y * 16, 128, 255]);
        }
    }
    l.push(Item::Image {
        image: Image::new(16, 16, std::sync::Arc::from(px)),
        rect: Rect::new(32.0, 48.0, 48.0, 64.0),
        radii: CornerRadii::ZERO,
        sampling: Sampling::Nearest,
    });
    l.push_filter(Filter::Blur { radius: 2.0 });
    l.fill_rect(Rect::new(50.0, 50.0, 60.0, 60.0), Brush::Solid(Color::BLACK));
    l.pop_filter();
    l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::grayscale(1.0) });
    l.fill_rect(Rect::new(12.0, 12.0, 20.0, 20.0), Brush::Solid(Color::rgb8(250, 200, 0)));
    l.pop_filter();
    l
}

/// Draws the self-test scene on `ctx` and on the CPU reference renderer and compares them:
/// `Err` with what was wrong when the device drew it wrong, failed, reported validation
/// errors or was lost. Records the verdict for the backend. Reading pixels back needs a
/// native device (a browser reads back only between frames): there it passes untested.
pub fn self_test(ctx: &GpuContext) -> Result<(), GpuError> {
    if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        return Ok(());
    }
    let key = key(ctx);
    with(|h| h.tests_run += 1);
    let result = run_self_test(ctx);
    record(
        &key,
        match &result {
            Ok(()) => Verdict::Passed,
            Err(e) => Verdict::Failed(e.to_string()),
        },
    );
    result
}

fn run_self_test(ctx: &GpuContext) -> Result<(), GpuError> {
    use remedy_ui_core::Color;
    let list = scene();
    let want = remedy_ui_render_cpu::CpuRenderer::new()
        .render(&list, 64, 64, 1.0, Color::WHITE)
        .map_err(|e| GpuError::Render(format!("the CPU reference: {e}")))?;
    let mut gpu = crate::GpuRenderer::new(ctx.clone(), None);
    let got = gpu.render(&list, 64, 64, 1.0, Color::WHITE)?;
    if ctx.is_lost() {
        return Err(GpuError::Lost);
    }
    if let Some(e) = ctx.take_errors().into_iter().next() {
        return Err(GpuError::Render(format!("the self-test raised a GPU error: {e}")));
    }
    let off = compare(&want.pixels, &got.pixels);
    // Both renderers agree within a few levels on all of it (the filter tests hold them to
    // that); a broken driver is off by far on much of it.
    if off.wrong * 100 > 64 * 64 || off.worst > 96 {
        return Err(GpuError::Render(format!(
            "the self-test frame came out wrong: {} of 4096 pixels off by more than 16, worst {}",
            off.wrong, off.worst
        )));
    }
    Ok(())
}

struct Off {
    wrong: usize,
    worst: u8,
}

fn compare(a: &[u8], b: &[u8]) -> Off {
    if a.len() != b.len() {
        return Off { wrong: a.len().max(b.len()) / 4, worst: 255 };
    }
    let mut off = Off { wrong: 0, worst: 0 };
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        off.worst = off.worst.max(d);
        if d > 16 {
            off.wrong += 1;
        }
    }
    off
}

/// Watches presents from a thread of its own; see the [module](self). Clones share one watch.
#[derive(Clone)]
pub struct PresentWatchdog {
    inner: Arc<Watch>,
}

struct Watch {
    key: String,
    /// When the present running started (nanoseconds on the watchdog's clock), 0 when none.
    started: AtomicU64,
    /// The hang threshold, in nanoseconds.
    hang: AtomicU64,
    hung: AtomicBool,
    epoch: remedy_ui_core::time::Instant,
}

impl std::fmt::Debug for PresentWatchdog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PresentWatchdog({}, hung: {})", self.inner.key, self.hung())
    }
}

impl Watch {
    fn now(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX).max(1)
    }

    /// True (and recorded) when a present has run past the threshold.
    fn check(&self) -> bool {
        let started = self.started.load(Ordering::Acquire);
        if started == 0 || self.hung.load(Ordering::Acquire) {
            return false;
        }
        if self.now().saturating_sub(started) > self.hang.load(Ordering::Relaxed) {
            self.hung.store(true, Ordering::Release);
            record(&self.key, Verdict::Hung);
            return true;
        }
        false
    }
}

impl PresentWatchdog {
    /// A watchdog for presents on `ctx`, with the thread that watches them (native only; a
    /// browser has no thread to watch from, and its presents never block).
    pub fn new(ctx: &GpuContext) -> Self {
        let inner = Arc::new(Watch {
            key: key(ctx),
            started: AtomicU64::new(0),
            hang: AtomicU64::new(u64::try_from(PRESENT_HANG.as_nanos()).unwrap_or(u64::MAX)),
            hung: AtomicBool::new(false),
            epoch: remedy_ui_core::time::Instant::now(),
        });
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let weak = Arc::downgrade(&inner);
            // A thread that sleeps and looks: gone with the watchdog.
            let _ = std::thread::Builder::new().name("remedy-present-watchdog".into()).spawn(move || {
                while let Some(w) = weak.upgrade() {
                    let step = Duration::from_nanos(w.hang.load(Ordering::Relaxed) / 4)
                        .clamp(Duration::from_millis(1), Duration::from_millis(250));
                    w.check();
                    drop(w);
                    std::thread::sleep(step);
                }
            });
        }
        PresentWatchdog { inner }
    }

    /// Runs `present`, watched.
    pub fn watch<T>(&self, present: impl FnOnce() -> T) -> T {
        self.inner.started.store(self.inner.now(), Ordering::Release);
        let out = present();
        // A present that ran past the threshold but returned before the thread looked.
        self.inner.check();
        self.inner.started.store(0, Ordering::Release);
        out
    }

    /// True once a present hung: the backend is recorded as hung and should be let go.
    pub fn hung(&self) -> bool {
        self.inner.hung.load(Ordering::Acquire)
    }

    /// Sets how long a present may run (for tests; [`PRESENT_HANG`] by default).
    #[doc(hidden)]
    pub fn set_threshold(&self, d: Duration) {
        self.inner.hang.store(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_round_trip_through_the_file_and_bad_lines_are_ignored() {
        for v in [Verdict::Passed, Verdict::Hung, Verdict::Failed("off by 200".into())] {
            assert_eq!(Verdict::parse(&v.word()), Some(v));
        }
        assert_eq!(Verdict::parse("maybe"), None);
        let dir = std::env::temp_dir().join(format!("remedy-health-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let long = "x".repeat(MAX_LINE * 2);
        let _ = std::fs::write(
            dir.join(FILE),
            format!("good\tpassed\nbad\tnonsense\nnotab\n{long}\thung\nslow\thung\nwrong\tfailed garbage\n"),
        );
        let read = read_file(&dir);
        assert_eq!(read.len(), 3, "{read:?}");
        assert!(read.contains(&("slow".into(), Verdict::Hung)));
        assert!(read.contains(&("wrong".into(), Verdict::Failed("garbage".into()))));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(compare(&[0; 8], &[0; 4]).worst == 255, "a frame of the wrong size is wrong");
        // The reference scene against a black frame (what a broken driver often gives).
        let Ok(want) = remedy_ui_render_cpu::CpuRenderer::new().render(
            &scene(),
            64,
            64,
            1.0,
            remedy_ui_core::Color::WHITE,
        ) else {
            unreachable!("renders")
        };
        let off = compare(&want.pixels, &vec![0; want.pixels.len()]);
        assert!(off.wrong * 100 > 64 * 64 && off.worst > 96, "garbage is caught");
        assert_eq!(compare(&want.pixels, &want.pixels).wrong, 0);
    }
}
