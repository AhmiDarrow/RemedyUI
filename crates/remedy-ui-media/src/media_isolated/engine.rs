//! The app's side: a pool of helpers, the router the image loader decodes through, and the
//! budgets each request runs under.
//!
//! A request takes an idle helper (or starts one), hands it the bytes in shared memory, and
//! waits for its reply under a watch. A helper that crashes, runs past its time, uses more
//! memory than its image allows or answers nonsense is stopped and dropped; the image gets
//! [`DecodeError::Stopped`] and a fresh helper is started in the background. A helper that
//! simply cannot read an image (damaged, unknown) stays in the pool.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::media_image::{DecodeError, Decoded, Router, fitted_scalable, set_router};
use remedy_ui_engine::paint::Image;

use crate::media_isolated::os::{Child, Killer, LIMITS_MEMORY, Shm, layers_from_bits};
use crate::media_isolated::protocol::{Reply, Request};
use crate::media_isolated::watchdog::{self, Fired, Limits};
use crate::media_isolated::{COUNTERS, EnableError, Isolation, Layer};

#[cfg(feature = "media-isolated-anim")]
mod anim;

// A request's time and memory budgets are the app's (`IsolationLimits`).
fn slack() -> u32 {
    crate::media_isolated::isolation_limits().time_slack
}
pub(crate) fn per_byte() -> u64 {
    crate::media_isolated::isolation_limits().memory_per_byte
}
pub(crate) fn per_source_pixel() -> u64 {
    crate::media_isolated::isolation_limits().memory_per_source_pixel
}
pub(crate) fn per_output_pixel() -> u64 {
    crate::media_isolated::isolation_limits().memory_per_output_pixel
}

/// The timing picture's side, in pixels.
const CALIBRATION_SIDE: u32 = 256;

struct Costs {
    /// Starting a helper, until it says it is shut in.
    spawn: Duration,
    /// Seconds per unit of work (a byte read or a pixel decoded or written).
    per_unit: f64,
}

struct Helper {
    child: Child,
    /// Its memory when it came up (the base its limits and recycling are measured from).
    start_memory: u64,
}

pub(crate) struct Engine {
    exe: PathBuf,
    layers: OnceLock<Vec<Layer>>,
    /// Starting the first helper, until it said it was shut in.
    spawn_cost: OnceLock<Duration>,
    costs: OnceLock<Costs>,
    idle: Mutex<Vec<Helper>>,
    /// Helpers hosting animations.
    #[cfg(feature = "media-isolated-anim")]
    anim: Mutex<Vec<Arc<anim::Host>>>,
    live: AtomicBool,
}

/// The image's decoder error for a helper that could not be started.
fn startup_error(e: Startup) -> DecodeError {
    let (Startup::Refused(why) | Startup::Failed(why)) = e;
    DecodeError::Stopped(format!("no decoding helper could be started: {why}"))
}

/// Why a helper could not be started.
enum Startup {
    /// It started but could not shut itself in (the platform lacks the sandbox).
    Refused(String),
    Failed(String),
}

/// How a request failed.
enum Fail {
    /// The image could not be decoded; the helper is fine.
    Image(DecodeError),
    /// The helper was stopped (or must be): the image gets this error.
    Helper(DecodeError),
    /// The animation's helper had already been stopped by another animation's request: it
    /// opens again elsewhere.
    #[cfg(feature = "media-isolated-anim")]
    Gone,
}

fn engine_slot() -> &'static Mutex<Option<Arc<Engine>>> {
    static E: OnceLock<Mutex<Option<Arc<Engine>>>> = OnceLock::new();
    E.get_or_init(Default::default)
}

fn units(input: usize, source_px: u64, output_px: u64) -> u64 {
    (input as u64).saturating_add(source_px).saturating_add(output_px)
}

pub(crate) fn enable() -> Result<Isolation, EnableError> {
    let mut slot = engine_slot().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(e) = slot.as_ref() {
        return Ok(Isolation { layers: e.layers.get().cloned().unwrap_or_default() });
    }
    let exe = std::env::current_exe().map_err(|e| EnableError::Failed(format!("no executable path: {e}")))?;
    let engine = Arc::new(Engine {
        exe,
        layers: OnceLock::new(),
        spawn_cost: OnceLock::new(),
        costs: OnceLock::new(),
        idle: Mutex::new(Vec::new()),
        #[cfg(feature = "media-isolated-anim")]
        anim: Mutex::new(Vec::new()),
        live: AtomicBool::new(true),
    });
    let started = start(&engine);
    if started.is_err() {
        // Nothing stays registered with the system for helpers that never served.
        crate::media_isolated::os::forget();
    }
    let layers = started?;
    set_router(Some(Arc::new(Route(engine.clone()))));
    #[cfg(feature = "media-isolated-anim")]
    crate::media_anim::route::set_opener(Some(Arc::new(anim::AnimRoute(engine.clone()))));
    *slot = Some(engine);
    Ok(Isolation { layers })
}

/// Starts the first helper and times it; the layers it runs under.
fn start(engine: &Arc<Engine>) -> Result<Vec<Layer>, EnableError> {
    let t = Instant::now();
    let mut helper = engine.spawn().map_err(|e| match e {
        Startup::Refused(why) => EnableError::Unsupported(why),
        Startup::Failed(why) => EnableError::Failed(why),
    })?;
    let spawn = t.elapsed();
    let _ = engine.spawn_cost.set(spawn);

    // Time decoding on this machine, in the helper as it will run: the best of a few runs
    // of a picture of known size (the first warms the decoder up).
    let png = calibration_png().map_err(EnableError::Failed)?;
    let input = shm_with(&png).map_err(|e| EnableError::Failed(e.to_string()))?;
    let px = u64::from(CALIBRATION_SIDE) * u64::from(CALIBRATION_SIDE);
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        match engine.decode_on(&mut helper, &input, png.len(), None, false) {
            Ok(d) if d.image.width == CALIBRATION_SIDE && d.image.height == CALIBRATION_SIDE => {}
            Ok(_) => return Err(EnableError::Failed("the helper decoded the timing picture wrongly".into())),
            Err(Fail::Image(e) | Fail::Helper(e)) => {
                return Err(EnableError::Failed(format!(
                    "the helper could not decode the timing picture: {e}"
                )));
            }
            #[cfg(feature = "media-isolated-anim")]
            Err(Fail::Gone) => return Err(EnableError::Failed("the helper went while being timed".into())),
        }
        best = best.min(t.elapsed());
    }
    let per_unit = best.as_secs_f64() / units(png.len(), px, px).max(1) as f64;
    let _ = engine.costs.set(Costs { spawn, per_unit });
    engine.give_back(helper);
    Ok(engine.layers.get().cloned().unwrap_or_default())
}

pub(crate) fn disable() {
    let engine = engine_slot().lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(e) = engine {
        set_router(None);
        #[cfg(feature = "media-isolated-anim")]
        crate::media_anim::route::set_opener(None);
        e.live.store(false, Ordering::Relaxed);
        e.idle.lock().unwrap_or_else(PoisonError::into_inner).clear();
        // Animations playing now go on, decoded in the app.
        #[cfg(feature = "media-isolated-anim")]
        e.stop_animation_helpers();
        crate::media_isolated::os::forget();
    }
}

pub(crate) fn isolation() -> Option<Isolation> {
    let slot = engine_slot().lock().unwrap_or_else(PoisonError::into_inner);
    slot.as_ref().map(|e| Isolation { layers: e.layers.get().cloned().unwrap_or_default() })
}

/// A picture of known size whose decoding is timed: a gradient with noise, so it neither
/// compresses to nothing nor is all noise.
fn calibration_png() -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let side = CALIBRATION_SIDE;
    let mut x: u32 = 0x9E37_79B9;
    let mut raw = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for col in 0..side {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let n = (x & 0x1F) as u8;
            raw.extend_from_slice(&[
                (col as u8).wrapping_add(n),
                (y as u8) ^ n,
                n.wrapping_mul(7),
                255 - (n >> 1),
            ]);
        }
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&raw, side, side, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn shm_with(bytes: &[u8]) -> Result<Shm, DecodeError> {
    let mut s = Shm::new(bytes.len())
        .map_err(|e| DecodeError::Io(format!("no shared memory for {} bytes: {e}", bytes.len())))?;
    s.write(bytes).map_err(|e| DecodeError::Io(e.to_string()))?;
    Ok(s)
}

impl Engine {
    /// The time a request of `units` may take: unbounded only while the machine is being
    /// timed (on a picture the app made itself).
    fn budget(&self, units: u64) -> Option<Duration> {
        let c = self.costs.get()?;
        let work = Duration::try_from_secs_f64(c.per_unit * units as f64).unwrap_or(Duration::MAX);
        Some(c.spawn.saturating_add(work).saturating_mul(slack()))
    }

    /// How long a helper that has stopped answering gets to finish ending (as long as
    /// starting one may take) before it is stopped.
    fn grace(&self) -> Duration {
        self.spawn_cost.get().map_or(Duration::ZERO, |c| c.saturating_mul(slack()))
    }

    /// The watch for work given `budget`: that much more of the helper's processor time, and
    /// in any case `time_slack` times as long on the clock (a helper that blocks rather than works).
    /// Without a way to read its processor time, the budget is on the clock.
    fn limits(killer: &Killer, budget: Option<Duration>, memory: Option<u64>) -> Limits {
        let Some(b) = budget else { return Limits { memory, ..Limits::default() } };
        match killer.cpu_time() {
            Some(used) => Limits {
                cpu: Some((used.saturating_add(b), b / 8)),
                deadline: Instant::now().checked_add(b.saturating_mul(slack())),
                memory,
            },
            None => Limits { cpu: None, deadline: Instant::now().checked_add(b), memory },
        }
    }

    fn spawn(&self) -> Result<Helper, Startup> {
        let t = Instant::now();
        let mut child = Child::spawn(&self.exe).map_err(|e| Startup::Failed(e.to_string()))?;
        // Until the machine is timed, starting up is not limited: nothing hostile has been
        // read yet, and `helper_main` runs before the app does anything.
        let budget = self.costs.get().map(|c| c.spawn.saturating_mul(slack()));
        let armed = watchdog::arm(Self::limits(&child.killer(), budget, None), child.killer())
            .ok_or_else(|| Startup::Failed("the watchdog thread could not start".into()))?;
        let reply = child.recv();
        let fired = armed.disarm();
        match reply.ok().and_then(|p| Reply::decode(&p)) {
            Some(Reply::Ready { layers }) if fired == Fired::No => {
                let layers = layers_from_bits(layers);
                if *self.layers.get_or_init(|| layers.clone()) != layers {
                    return Err(Startup::Failed(format!(
                        "a helper came up with other sandbox layers: {layers:?}"
                    )));
                }
                COUNTERS.started.fetch_add(1, Ordering::Relaxed);
                let start_memory = child.memory_in_use().unwrap_or(0);
                Ok(Helper { child, start_memory })
            }
            Some(Reply::Refused(why)) => Err(Startup::Refused(why)),
            _ => {
                let (how, err) = child.wait_exit(t.elapsed().saturating_mul(slack()));
                let err = err.trim();
                let detail = if err.is_empty() { String::new() } else { format!(": {err}") };
                Err(Startup::Failed(format!("the helper did not come up ({how}){detail}")))
            }
        }
    }

    fn take(&self) -> Result<Helper, DecodeError> {
        if let Some(h) = self.idle.lock().unwrap_or_else(PoisonError::into_inner).pop() {
            return Ok(h);
        }
        self.spawn().map_err(startup_error)
    }

    /// Back to the pool, unless it has grown past twice its starting size (a decoder that
    /// keeps memory, or a helper hoarding regions): then a fresh one takes its place.
    fn give_back(self: &Arc<Self>, mut h: Helper) {
        h.child.drain_stderr();
        if !self.live.load(Ordering::Relaxed) {
            return;
        }
        let grown = h.child.memory_in_use().is_some_and(|m| m > h.start_memory.saturating_mul(2));
        if grown {
            COUNTERS.recycled.fetch_add(1, Ordering::Relaxed);
            drop(h);
            self.replace();
            return;
        }
        self.idle.lock().unwrap_or_else(PoisonError::into_inner).push(h);
    }

    /// Starts a helper in the background to take a stopped one's place.
    fn replace(self: &Arc<Self>) {
        let me = self.clone();
        let _ = std::thread::Builder::new().name("remedy-decoder-restart".into()).spawn(move || {
            if !me.live.load(Ordering::Relaxed)
                || !me.idle.lock().unwrap_or_else(PoisonError::into_inner).is_empty()
            {
                return;
            }
            if let Ok(h) = me.spawn()
                && me.live.load(Ordering::Relaxed)
            {
                me.idle.lock().unwrap_or_else(PoisonError::into_inner).push(h);
            }
        });
    }

    fn run<T>(
        self: &Arc<Self>,
        f: impl FnOnce(&Engine, &mut Helper) -> Result<T, Fail>,
    ) -> Result<T, DecodeError> {
        let mut h = self.take()?;
        match f(self, &mut h) {
            Ok(v) => {
                self.give_back(h);
                Ok(v)
            }
            Err(Fail::Image(e)) => {
                self.give_back(h);
                Err(e)
            }
            Err(Fail::Helper(e)) => {
                drop(h);
                self.replace();
                Err(e)
            }
            // Only animation helpers are shared; a still's helper is never gone under it.
            #[cfg(feature = "media-isolated-anim")]
            Err(Fail::Gone) => {
                drop(h);
                self.replace();
                Err(DecodeError::Stopped("its helper was stopped".into()))
            }
        }
    }

    /// One request: `regions` handed over, the reply awaited within the time `units` of work
    /// allow, with memory limited to the helper's size now plus its starting size plus `need`
    /// and the regions. `what` names the work in errors.
    fn exchange(
        &self,
        h: &mut Helper,
        request: Request,
        regions: &[(&Shm, bool)],
        units: u64,
        need: u64,
        what: &str,
    ) -> Result<Reply, DecodeError> {
        COUNTERS.requests.fetch_add(1, Ordering::Relaxed);
        let now = h.child.memory_in_use().unwrap_or(h.start_memory);
        let mapped: u64 = regions.iter().map(|(s, _)| s.len() as u64).sum();
        let limit = now.saturating_add(h.start_memory).saturating_add(need).saturating_add(mapped);
        if LIMITS_MEMORY {
            h.child.limit_memory(limit).map_err(|e| {
                DecodeError::Stopped(format!("the helper's memory could not be limited: {e}"))
            })?;
        }
        let budget = self.budget(units);
        let killer = h.child.killer();
        let armed = watchdog::arm(Self::limits(&killer, budget, (!LIMITS_MEMORY).then_some(limit)), killer)
            .ok_or_else(|| DecodeError::Stopped("the watchdog thread could not start".into()))?;
        let got = h.child.send(&request.encode(), regions).and_then(|()| h.child.recv());
        let fired = armed.disarm();
        h.child.end_request();
        match got.ok().and_then(|p| Reply::decode(&p)) {
            Some(reply) if fired == Fired::No => Ok(reply),
            _ => Err(self.stopped(h, fired, budget, what)),
        }
    }

    /// Why a helper stopped answering; it is dropped after this.
    fn stopped(&self, h: &mut Helper, fired: Fired, budget: Option<Duration>, what: &str) -> DecodeError {
        let memory = || {
            COUNTERS.over_memory.fetch_add(1, Ordering::Relaxed);
            DecodeError::Stopped(format!("it used more memory than {what} needs"))
        };
        match fired {
            Fired::Time => {
                COUNTERS.timed_out.fetch_add(1, Ordering::Relaxed);
                let secs = budget.map_or(0.0, |b| b.as_secs_f64());
                DecodeError::Stopped(format!(
                    "it worked past {secs:.1} s, far longer than {what} takes on this machine"
                ))
            }
            Fired::Memory => memory(),
            Fired::No => {
                let (how, err) = h.child.wait_exit(self.grace());
                // Rust's allocation-failure handler says this before it aborts.
                if err.contains("memory allocation of") {
                    memory()
                } else {
                    COUNTERS.crashed.fetch_add(1, Ordering::Relaxed);
                    DecodeError::Stopped(format!("it crashed on {what} ({how})"))
                }
            }
        }
    }

    fn probe(&self, h: &mut Helper, input: &Shm, len: usize) -> Result<((u32, u32), bool), Fail> {
        let what = format!("the header of a {len}-byte image");
        let need = (len as u64).saturating_mul(per_byte());
        match self.exchange(
            h,
            Request::Probe { len: len as u64, animated: false },
            &[(input, false)],
            units(len, 0, 0),
            need,
            &what,
        ) {
            Ok(Reply::Size { size, scalable }) => Ok((size, scalable)),
            Ok(Reply::Failed(e)) => Err(Fail::Image(e)),
            Ok(_) => Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
            Err(e) => Err(Fail::Helper(e)),
        }
    }

    fn decode_on(
        &self,
        h: &mut Helper,
        input: &Shm,
        len: usize,
        box_px: Option<(u32, u32)>,
        cover: bool,
    ) -> Result<Decoded, Fail> {
        let (source, scalable) = self.probe(h, input, len)?;
        let show = box_px.map_or(source, |b| fitted_scalable(source, b, cover, scalable));
        let too_large = || Fail::Image(DecodeError::TooLarge { width: source.0, height: source.1 });
        let source_px = u64::from(source.0) * u64::from(source.1);
        let output_px = u64::from(show.0) * u64::from(show.1);
        let out_len = output_px.checked_mul(4).and_then(|n| usize::try_from(n).ok()).ok_or_else(too_large)?;
        let out = Shm::new(out_len).map_err(|_| too_large())?;
        let need = (len as u64)
            .saturating_mul(per_byte())
            .saturating_add(source_px.saturating_mul(per_source_pixel()))
            .saturating_add(output_px.saturating_mul(per_output_pixel()));
        let what = format!("a {}×{} image", source.0, source.1);
        let request = Request::Decode { len: len as u64, show };
        let work = units(len, source_px, output_px);
        match self.exchange(h, request, &[(input, false), (&out, true)], work, need, &what) {
            Ok(Reply::Pixels { size, source }) => {
                if size.0 == 0 || size.1 == 0 || size.0 > show.0 || size.1 > show.1 {
                    return Err(Fail::Helper(DecodeError::Stopped(format!(
                        "it returned {}×{} pixels for {}×{}",
                        size.0, size.1, show.0, show.1
                    ))));
                }
                let n = size.0 as usize * size.1 as usize * 4;
                let pixels = out.read(n).map_err(|_| too_large())?;
                Ok(Decoded::new(Image::new(size.0, size.1, Arc::from(pixels)), source))
            }
            Ok(Reply::Failed(e)) => Err(Fail::Image(e)),
            Ok(_) => Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
            Err(e) => Err(Fail::Helper(e)),
        }
    }
}

/// The router the image loader decodes through while isolation is on.
struct Route(Arc<Engine>);

impl Router for Route {
    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
        let input = shm_with(bytes)?;
        self.0.run(|e, h| e.probe(h, &input, bytes.len()).map(|(size, _)| size))
    }

    fn decode_for_box(
        &self,
        bytes: &[u8],
        box_px: Option<(u32, u32)>,
        cover: bool,
    ) -> Result<Decoded, DecodeError> {
        let input = shm_with(bytes)?;
        self.0.run(|e, h| e.decode_on(h, &input, bytes.len(), box_px, cover))
    }
}
