//! Animations in helpers. Each open animation is a session on one helper, which keeps its
//! decoder; frames are asked for one at a time with a budget sized from the frame.
//!
//! Helpers that host animations are shared by many sessions (a request has its helper to
//! itself while it runs). A new animation goes to a helper that is free at the moment, the one
//! with fewest animations; another helper is started only when all are busy, up to one per
//! image worker. A helper that crashes, runs past its time or memory, or answers nonsense is
//! stopped: the animation whose request it was fails, and every other animation it held opens
//! again on a fresh helper and reads forward to where it was. An animation that once stopped a
//! helper is not given another.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::media_anim::route::{Animation, FrameSource, Loops, Opened, Opener, open_here};
use crate::media_image::{DecodeError, fitted_scalable};
use remedy_ui_engine::paint::Image;

use super::{
    Engine, Fail, Helper, per_byte, per_output_pixel, per_source_pixel, shm_with, startup_error, units,
};
use crate::media_isolated::COUNTERS;
use crate::media_isolated::os::Shm;
use crate::media_isolated::protocol::{Reply, Request};

/// A helper hosting animations.
pub(super) struct Host {
    /// `None` once it has been stopped.
    helper: Mutex<Option<Helper>>,
    dead: AtomicBool,
    sessions: AtomicUsize,
    /// Sessions whose animations went away, closed before the host's next request.
    closing: Mutex<Vec<u32>>,
}

/// How many image workers there are: the most requests that can run at once.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1)
}

fn pixels(p: (u32, u32)) -> u64 {
    u64::from(p.0) * u64::from(p.1)
}

impl Host {
    /// Runs `f` with the helper to itself, after closing finished sessions. An error of `f`'s
    /// own request stops the helper; a helper already stopped (or stopped while closing
    /// another animation's session) is [`Fail::Gone`], which is no fault of the caller's.
    fn run<T>(&self, engine: &Engine, f: impl FnOnce(&mut Helper) -> Result<T, Fail>) -> Result<T, Fail> {
        let mut slot = self.helper.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(h) = slot.as_mut() else { return Err(Fail::Gone) };
        let closing = std::mem::take(&mut *self.closing.lock().unwrap_or_else(PoisonError::into_inner));
        for session in closing {
            if engine.exchange(h, Request::Close { session }, &[], 1, 0, "closing an animation").is_err() {
                self.dead.store(true, Ordering::Relaxed);
                slot.take();
                return Err(Fail::Gone);
            }
        }
        let result = f(h);
        if let Err(Fail::Helper(_)) = &result {
            self.dead.store(true, Ordering::Relaxed);
            // Dropping it stops it.
            slot.take();
        }
        result
    }

    fn open(
        &self,
        engine: &Engine,
        input: &Shm,
        len: usize,
        box_px: (u32, u32),
        cover: bool,
    ) -> Result<Option<(u32, Animation)>, Fail> {
        self.run(engine, |h| {
            let what = format!("the header of a {len}-byte animation");
            let need = (len as u64).saturating_mul(per_byte());
            let probe = Request::Probe { len: len as u64, animated: true };
            let size = match engine.exchange(h, probe, &[(input, false)], units(len, 0, 0), need, &what) {
                Ok(Reply::Size { size, .. }) => size,
                Ok(Reply::Failed(e)) => return Err(Fail::Image(e)),
                Ok(_) => return Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
                Err(e) => return Err(Fail::Helper(e)),
            };
            let show = fitted_scalable(size, box_px, cover, false);
            let (src, out) = (pixels(size), pixels(show));
            let need = need
                .saturating_add(src.saturating_mul(per_source_pixel()))
                .saturating_add(out.saturating_mul(per_output_pixel()));
            let what = format!("opening a {}×{} animation", size.0, size.1);
            let open = Request::Open { len: len as u64, box_px, cover };
            match engine.exchange(h, open, &[(input, false)], units(len, src, out), need, &what) {
                Ok(Reply::Animation(o)) => {
                    let valid = o.size == size && o.show_at.0 > 0 && o.show_at.1 > 0 && o.show_at == show;
                    if !valid {
                        return Err(Fail::Helper(DecodeError::Stopped(format!(
                            "it opened a {}×{} animation at {}×{} for a {}×{} file shown at {}×{}",
                            o.size.0, o.size.1, o.show_at.0, o.show_at.1, size.0, size.1, show.0, show.1
                        ))));
                    }
                    self.sessions.fetch_add(1, Ordering::Relaxed);
                    let loops = o.loops.map_or(Loops::Forever, Loops::Times);
                    Ok(Some((o.session, Animation { size, show_at: show, frames: o.frames, loops })))
                }
                Ok(Reply::Still) => Ok(None),
                Ok(Reply::Failed(e)) => Err(Fail::Image(e)),
                Ok(_) => Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
                Err(e) => Err(Fail::Helper(e)),
            }
        })
    }

    /// What a frame of `a` may cost: the work, and the memory beyond the helper's size.
    fn frame_cost(a: &Animation) -> (u64, u64) {
        let (src, out) = (pixels(a.size), pixels(a.show_at));
        let need =
            src.saturating_mul(per_source_pixel()).saturating_add(out.saturating_mul(per_output_pixel()));
        (src.saturating_add(out), need)
    }

    fn next(&self, engine: &Engine, session: u32, a: &Animation) -> Result<Option<(Image, Duration)>, Fail> {
        let too_large = || Fail::Image(DecodeError::TooLarge { width: a.size.0, height: a.size.1 });
        let len =
            pixels(a.show_at).checked_mul(4).and_then(|n| usize::try_from(n).ok()).ok_or_else(too_large)?;
        let out = Shm::new(len).map_err(|_| too_large())?;
        let (work, need) = Self::frame_cost(a);
        let what = format!("a frame of a {}×{} animation", a.size.0, a.size.1);
        self.run(engine, |h| {
            match engine.exchange(h, Request::Next { session }, &[(&out, true)], work, need, &what) {
                Ok(Reply::Frame { size, delay_us }) => {
                    if size.0 == 0 || size.1 == 0 || size.0 > a.show_at.0 || size.1 > a.show_at.1 {
                        return Err(Fail::Helper(DecodeError::Stopped(format!(
                            "it returned a {}×{} frame for {}×{}",
                            size.0, size.1, a.show_at.0, a.show_at.1
                        ))));
                    }
                    let pixels = out.read(size.0 as usize * size.1 as usize * 4).map_err(|_| too_large())?;
                    let image = Image::new(size.0, size.1, Arc::from(pixels));
                    Ok(Some((image, Duration::from_micros(delay_us))))
                }
                Ok(Reply::End) => Ok(None),
                Ok(Reply::Failed(e)) => Err(Fail::Image(e)),
                Ok(_) => Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
                Err(e) => Err(Fail::Helper(e)),
            }
        })
    }

    /// Reads past `count` frames; false when the play ended first.
    fn skip(&self, engine: &Engine, session: u32, count: u64, a: &Animation) -> Result<bool, Fail> {
        let (work, need) = Self::frame_cost(a);
        let what = format!("{count} frames of a {}×{} animation", a.size.0, a.size.1);
        let request = Request::Skip { session, count };
        self.run(engine, |h| {
            match engine.exchange(h, request, &[], work.saturating_mul(count), need, &what) {
                Ok(Reply::Done) => Ok(true),
                Ok(Reply::End) => Ok(false),
                Ok(Reply::Failed(e)) => Err(Fail::Image(e)),
                Ok(_) => Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
                Err(e) => Err(Fail::Helper(e)),
            }
        })
    }

    fn restart(&self, engine: &Engine, session: u32, len: usize, a: &Animation) -> Result<(), Fail> {
        // Starting over may read the whole file again (counting a GIF's frames) and draws
        // nothing until the next frame.
        let (work, need) = Self::frame_cost(a);
        let work = work.saturating_add(len as u64);
        let need = need.saturating_add((len as u64).saturating_mul(per_byte()));
        let what = format!("starting a {}×{} animation over", a.size.0, a.size.1);
        self.run(engine, |h| match engine.exchange(h, Request::Restart { session }, &[], work, need, &what) {
            Ok(Reply::Done) => Ok(()),
            Ok(Reply::Failed(e)) => Err(Fail::Image(e)),
            Ok(_) => Err(Fail::Helper(DecodeError::Stopped("it answered out of turn".into()))),
            Err(e) => Err(Fail::Helper(e)),
        })
    }

    fn close(&self, session: u32) {
        self.sessions.fetch_sub(1, Ordering::Relaxed);
        if !self.dead.load(Ordering::Relaxed) {
            self.closing.lock().unwrap_or_else(PoisonError::into_inner).push(session);
        }
    }
}

impl Engine {
    /// The helper a new animation goes to.
    fn anim_host(&self) -> Result<Arc<Host>, DecodeError> {
        let mut hosts = self.anim.lock().unwrap_or_else(PoisonError::into_inner);
        hosts.retain(|h| !h.dead.load(Ordering::Relaxed));
        let free = hosts
            .iter()
            .filter(|h| h.helper.try_lock().is_ok())
            .min_by_key(|h| h.sessions.load(Ordering::Relaxed))
            .cloned();
        if let Some(h) = free {
            return Ok(h);
        }
        if hosts.len() < workers() {
            let helper = self.spawn().map_err(startup_error)?;
            let host = Arc::new(Host {
                helper: Mutex::new(Some(helper)),
                dead: AtomicBool::new(false),
                sessions: AtomicUsize::new(0),
                closing: Mutex::new(Vec::new()),
            });
            hosts.push(host.clone());
            return Ok(host);
        }
        hosts
            .iter()
            .min_by_key(|h| h.sessions.load(Ordering::Relaxed))
            .cloned()
            .ok_or_else(|| DecodeError::Stopped("no helper could be started".into()))
    }

    pub(super) fn stop_animation_helpers(&self) {
        let hosts = std::mem::take(&mut *self.anim.lock().unwrap_or_else(PoisonError::into_inner));
        for h in hosts {
            h.dead.store(true, Ordering::Relaxed);
            h.helper.lock().unwrap_or_else(PoisonError::into_inner).take();
        }
    }
}

/// Where an animation's frames come from now.
enum At {
    /// A session on a helper.
    Helper(Arc<Host>, u32),
    /// Isolation was turned off: decoded here.
    Here(Box<dyn FrameSource>),
    /// Nowhere yet: its helper was stopped by another animation's request.
    Lost,
}

/// An animation decoded in a helper.
struct Remote {
    engine: Arc<Engine>,
    bytes: Arc<[u8]>,
    box_px: (u32, u32),
    cover: bool,
    animation: Animation,
    at: At,
    /// Frames read in this play (where a reopened session reads forward to).
    delivered: u64,
    /// Set when one of its own requests stopped a helper.
    poisoned: Option<DecodeError>,
}

impl Remote {
    /// Makes sure it has somewhere to read from: on a fresh helper (or here, once isolation is
    /// off), read forward to where it was.
    fn ensure(&mut self) -> Result<(), DecodeError> {
        if let Some(e) = &self.poisoned {
            return Err(e.clone());
        }
        match &self.at {
            At::Helper(host, _) if !host.dead.load(Ordering::Relaxed) => return Ok(()),
            At::Here(_) => return Ok(()),
            At::Helper(..) | At::Lost => {}
        }
        if !self.engine.live.load(Ordering::Relaxed) {
            let Opened::Animated(mut here) = open_here(&self.bytes, self.box_px, self.cover)? else {
                return Err(DecodeError::Malformed("the file is no longer an animation".into()));
            };
            for _ in 0..self.delivered {
                if here.next()?.is_none() {
                    break;
                }
            }
            self.at = At::Here(here);
            return Ok(());
        }
        COUNTERS.reopened.fetch_add(1, Ordering::Relaxed);
        let opened = (|| {
            let host = self.engine.anim_host().map_err(Fail::Helper)?;
            let input = shm_with(&self.bytes).map_err(Fail::Image)?;
            let (session, a) =
                host.open(&self.engine, &input, self.bytes.len(), self.box_px, self.cover)?.ok_or_else(
                    || Fail::Image(DecodeError::Malformed("the file is no longer an animation".into())),
                )?;
            if a != self.animation {
                return Err(Fail::Image(DecodeError::Malformed("the animation opened differently".into())));
            }
            self.at = At::Helper(host.clone(), session);
            if self.delivered > 0 {
                host.skip(&self.engine, session, self.delivered, &a)?;
            }
            Ok(())
        })();
        // Gone again (another animation stopped the fresh helper): the next ask opens again.
        self.settle(opened).map(|_| ())
    }

    /// The outcome of a request: a helper it stopped poisons it; `None` when its helper was
    /// gone and it must open again and ask again.
    fn settle<T>(&mut self, r: Result<T, Fail>) -> Result<Option<T>, DecodeError> {
        match r {
            Ok(v) => Ok(Some(v)),
            Err(Fail::Image(e)) => Err(e),
            Err(Fail::Helper(e)) => {
                self.poisoned = Some(e.clone());
                self.at = At::Lost;
                Err(e)
            }
            Err(Fail::Gone) => {
                self.at = At::Lost;
                Ok(None)
            }
        }
    }

    /// Asks `there` of the helper its frames come from (`here` once isolation is off),
    /// opening again on a fresh helper, and reading forward, each time another animation's
    /// request has stopped the one it was on.
    fn ask<T>(
        &mut self,
        here: impl Fn(&mut dyn FrameSource) -> Result<T, DecodeError>,
        there: impl Fn(&Host, &Engine, u32, &Animation) -> Result<T, Fail>,
    ) -> Result<T, DecodeError> {
        loop {
            self.ensure()?;
            let r = match &mut self.at {
                At::Here(source) => return here(source.as_mut()),
                At::Helper(host, session) => there(&host.clone(), &self.engine, *session, &self.animation),
                At::Lost => Err(Fail::Gone),
            };
            if let Some(v) = self.settle(r)? {
                return Ok(v);
            }
        }
    }
}

impl FrameSource for Remote {
    fn animation(&self) -> &Animation {
        &self.animation
    }

    fn next(&mut self) -> Result<Option<(Image, Duration)>, DecodeError> {
        let frame = self.ask(|here| here.next(), |host, engine, session, a| host.next(engine, session, a))?;
        self.delivered += u64::from(frame.is_some());
        Ok(frame)
    }

    fn restart(&mut self) -> Result<(), DecodeError> {
        let len = self.bytes.len();
        self.ask(|here| here.restart(), |host, engine, session, a| host.restart(engine, session, len, a))?;
        self.delivered = 0;
        Ok(())
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        if let At::Helper(host, session) = &self.at {
            host.close(*session);
        }
    }
}

/// The opener animations go through while isolation is on.
pub(super) struct AnimRoute(pub(super) Arc<Engine>);

impl Opener for AnimRoute {
    fn open(&self, bytes: &Arc<[u8]>, box_px: (u32, u32), cover: bool) -> Result<Opened, DecodeError> {
        let engine = &self.0;
        let input = shm_with(bytes)?;
        loop {
            let host = engine.anim_host()?;
            return match host.open(engine, &input, bytes.len(), box_px, cover) {
                Ok(Some((session, animation))) => Ok(Opened::Animated(Box::new(Remote {
                    engine: engine.clone(),
                    bytes: bytes.clone(),
                    box_px,
                    cover,
                    animation,
                    at: At::Helper(host, session),
                    delivered: 0,
                    poisoned: None,
                }))),
                Ok(None) => Ok(Opened::Still),
                Err(Fail::Image(e) | Fail::Helper(e)) => Err(e),
                // Stopped by another animation's request just now: open on another.
                Err(Fail::Gone) => continue,
            };
        }
    }
}
