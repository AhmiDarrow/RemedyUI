//! The microphone: sound in, on a worker of its own, handed to the app as it arrives.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use remedy_ui_core::exec::{Next, Worker};
use remedy_ui_reactive::Signal;

use crate::host::{AudioFormat, AudioInput, MediaError, MediaHost};

/// How often the level is reported while listening: thirty times a second moves a meter
/// smoothly for people without waking the UI thread for every device period.
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);

/// How a microphone opens.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MicrophoneOptions {
    /// The sample rate and channels the app wants; the platform converts from the device's
    /// own. 48 kHz mono by default: what pitch detection, levels and speech need.
    pub format: AudioFormat,
}

impl MicrophoneOptions {
    /// Sets `format`: the sample rate and channels the app wants.
    pub fn format(mut self, value: AudioFormat) -> Self {
        self.format = value;
        self
    }
}

impl Default for MicrophoneOptions {
    fn default() -> Self {
        MicrophoneOptions { format: AudioFormat::new(48_000, 1) }
    }
}

/// What a microphone is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MicrophoneState {
    /// The device is being opened (in a browser, the person may be answering its prompt).
    #[default]
    Starting,
    /// Sound is arriving.
    Listening,
    /// It stopped: [`Microphone::stop`] was called.
    Stopped,
    /// It could not be opened, or the device failed or went away;
    /// [`Microphone::error`] says why.
    Failed,
}

/// What the worker reports to the UI thread.
#[derive(Default)]
struct Status {
    state: MicrophoneState,
    error: Option<MediaError>,
    /// The loudest sample since the last report, 0 to 1.
    peak: f32,
}

#[derive(Default)]
struct Shared {
    status: Mutex<Status>,
    /// A report is waiting on the UI thread: further ones merge into it.
    posted: AtomicBool,
    /// The app asked it to stop.
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type Notify = Box<dyn Fn() -> bool + Send>;

/// What hears the sound, on the microphone's worker.
type OnSound = Box<dyn FnMut(&[f32]) + Send>;

thread_local! {
    /// Each microphone on this (UI) thread, by number: what takes its news.
    static MICS: RefCell<HashMap<u64, Weak<Inner>>> = RefCell::new(HashMap::new());
}

fn deliver(id: u64) {
    let m = MICS.with(|m| m.borrow().get(&id).and_then(Weak::upgrade));
    if let Some(m) = m {
        m.pull();
    }
}

struct Inner {
    id: u64,
    shared: Arc<Shared>,
    worker: RefCell<Option<Worker>>,
    format: AudioFormat,
    state: Signal<MicrophoneState>,
    error: Signal<Option<MediaError>>,
    level: Signal<f32>,
}

impl Inner {
    fn pull(&self) {
        self.shared.posted.store(false, Ordering::Release);
        let (state, error, peak) = {
            let mut s = lock(&self.shared.status);
            (s.state, s.error.clone(), std::mem::take(&mut s.peak))
        };
        remedy_ui_reactive::batch(|| {
            self.state.set_if_changed(state);
            self.error.set_if_changed(error);
            self.level.set_if_changed(if state == MicrophoneState::Listening { peak } else { 0.0 });
        });
        if matches!(state, MicrophoneState::Failed | MicrophoneState::Stopped) {
            self.worker.borrow_mut().take();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let id = self.id;
        let _ = MICS.try_with(|m| m.borrow_mut().remove(&id));
    }
}

/// The worker's state: the open device, the app's handler and what it reports.
struct Capture {
    host: Arc<dyn MediaHost>,
    format: AudioFormat,
    input: Option<Box<dyn AudioInput>>,
    on_sound: OnSound,
    /// Converts a device's own format to the asked-for one, where the platform did not.
    convert: Option<Converter>,
    /// The device's samples before conversion.
    raw: Vec<f32>,
    shared: Arc<Shared>,
    notify: Notify,
    buf: Vec<f32>,
    period: Duration,
    since_report: Duration,
    done: bool,
}

impl Capture {
    fn report(&self, f: impl FnOnce(&mut Status)) {
        f(&mut lock(&self.shared.status));
        if !self.shared.posted.swap(true, Ordering::AcqRel) && !(self.notify)() {
            // The UI thread is gone: nothing will read the sound or the reports.
            self.shared.stop.store(true, Ordering::Release);
        }
    }

    fn fail(&mut self, e: MediaError) -> Next {
        self.input = None;
        self.done = true;
        self.report(|s| {
            s.state = MicrophoneState::Failed;
            s.error = Some(e);
        });
        Next::Idle
    }

    fn turn(&mut self) -> Next {
        if self.done {
            return Next::Idle;
        }
        if self.shared.stop.load(Ordering::Acquire) {
            self.input = None;
            self.done = true;
            self.report(|s| s.state = MicrophoneState::Stopped);
            return Next::Idle;
        }
        if self.input.is_none() {
            match self.host.audio_input(self.format) {
                Ok(i) => {
                    let got = i.format();
                    if got.channels == 0 || got.sample_rate == 0 {
                        return self.fail(MediaError::Device(
                            "the sound input reports no channels or no rate".into(),
                        ));
                    }
                    self.convert = (got != self.format).then(|| Converter::new(got, self.format));
                    self.period = i.period().clamp(Duration::from_millis(1), LEVEL_INTERVAL);
                    self.input = Some(i);
                }
                Err(e) => return self.fail(e),
            }
        }
        let Some(input) = self.input.as_mut() else { return Next::Idle };
        self.buf.clear();
        let read = match self.convert.as_mut() {
            None => input.read(&mut self.buf),
            Some(c) => {
                self.raw.clear();
                input.read(&mut self.raw).inspect(|_| c.convert(&self.raw, &mut self.buf))
            }
        };
        match read {
            Err(e) => self.fail(e),
            Ok(0) => {
                // Nothing yet: a browser still asking the person, or a device between periods.
                Next::After(self.period)
            }
            // Converted to nothing yet (a few frames held for the interpolation).
            Ok(_) if self.buf.is_empty() => Next::Now,
            Ok(_) => {
                (self.on_sound)(&self.buf);
                let peak =
                    self.buf.iter().fold(0.0f32, |m, s| if s.is_finite() { m.max(s.abs()) } else { m });
                let heard =
                    self.format.duration_of((self.buf.len() / usize::from(self.format.channels)) as u64);
                self.since_report += heard;
                {
                    let mut s = lock(&self.shared.status);
                    s.peak = s.peak.max(peak.min(1.0));
                }
                let first = lock(&self.shared.status).state != MicrophoneState::Listening;
                if first || self.since_report >= LEVEL_INTERVAL {
                    self.since_report = Duration::ZERO;
                    self.report(|s| s.state = MicrophoneState::Listening);
                }
                // More may already be waiting (the device hands over several periods at once
                // when the machine was busy).
                Next::Now
            }
        }
    }
}

/// The microphone: opens the default sound input on a worker of its own and hands its sound
/// to the app as it arrives, as interleaved 32-bit float samples in the asked-for format.
///
/// `on_sound` runs on the worker (natively a thread of its own; in a browser page a
/// cooperative task), never on the UI thread, so analysis such as pitch detection does not
/// hold up drawing; send results back with [`remedy_ui_reactive::waker`] or a channel. Its
/// state, error and level are signals on the UI thread. Cheap to clone; capturing stops when
/// [`Microphone::stop`] is called or the last clone is dropped.
///
/// On Android, ask first with `App::request_permission(Permission::Microphone, ..)` (and
/// declare `microphone` in `remedy.toml`'s `[android] permissions`); without it the device
/// gives silence or refuses. A browser asks the person itself when the microphone opens.
///
/// ```no_run
/// # fn demo() {
/// use remedy_ui_media_playback::{Microphone, MicrophoneOptions};
/// let mic = Microphone::open_platform(MicrophoneOptions::default(), |samples| {
///     // 48 kHz mono samples, as they arrive.
///     let _ = samples.len();
/// });
/// // `mic.level()` drives a meter; `mic.error()` says why it failed.
/// # let _ = mic;
/// # }
/// ```
#[derive(Clone)]
pub struct Microphone(Rc<Inner>);

impl std::fmt::Debug for Microphone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Microphone")
            .field("id", &self.0.id)
            .field("format", &self.0.format)
            .finish_non_exhaustive()
    }
}

impl Microphone {
    /// Opens the default sound input of `host`, on the UI thread. It starts in
    /// [`MicrophoneState::Starting`]; it is [`MicrophoneState::Failed`] with an
    /// [`error`](Microphone::error) when the input cannot be opened.
    pub fn open(
        host: Arc<dyn MediaHost>,
        options: MicrophoneOptions,
        on_sound: impl FnMut(&[f32]) + Send + 'static,
    ) -> Microphone {
        Self::start(Ok(host), options, Box::new(on_sound))
    }

    /// Opens the platform's default sound input ([`platform_host`](crate::platform_host)).
    pub fn open_platform(
        options: MicrophoneOptions,
        on_sound: impl FnMut(&[f32]) + Send + 'static,
    ) -> Microphone {
        Self::start(crate::platform_host(), options, Box::new(on_sound))
    }

    fn start(
        host: Result<Arc<dyn MediaHost>, MediaError>,
        options: MicrophoneOptions,
        on_sound: OnSound,
    ) -> Microphone {
        static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = IDS.fetch_add(1, Ordering::Relaxed);
        let format = options.format;
        let shared = Arc::new(Shared::default());
        let waker = remedy_ui_reactive::waker();
        let started = match (host, waker) {
            _ if format.channels == 0 || format.sample_rate == 0 => {
                Err(MediaError::Device("a sound format with no channels or no rate".into()))
            }
            (Ok(host), Ok(waker)) => {
                let notify: Notify = Box::new(move || waker.post(move || deliver(id)).is_ok());
                let s = shared.clone();
                Worker::spawn_timed(
                    "remedy-microphone",
                    move |_| Capture {
                        host,
                        format,
                        input: None,
                        on_sound,
                        convert: None,
                        raw: Vec::new(),
                        shared: s,
                        notify,
                        buf: Vec::new(),
                        period: LEVEL_INTERVAL,
                        since_report: Duration::ZERO,
                        done: false,
                    },
                    Capture::turn,
                )
                .map_err(|e| MediaError::Device(format!("could not start the microphone's worker: {e}")))
            }
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(MediaError::Unsupported(format!("no UI thread to report to: {e}"))),
        };
        let (worker, failed) = match started {
            Ok(w) => (Some(w), None),
            Err(e) => (None, Some(e)),
        };
        let inner = Rc::new(Inner {
            id,
            shared,
            worker: RefCell::new(worker),
            format,
            state: Signal::new(if failed.is_some() {
                MicrophoneState::Failed
            } else {
                MicrophoneState::Starting
            }),
            error: Signal::new(failed),
            level: Signal::new(0.0),
        });
        MICS.with(|m| m.borrow_mut().insert(id, Rc::downgrade(&inner)));
        Microphone(inner)
    }

    /// Stops capturing. The state becomes [`MicrophoneState::Stopped`] once the worker has
    /// let the device go; `on_sound` is not called after that.
    pub fn stop(&self) {
        self.0.shared.stop.store(true, Ordering::Release);
        if let Some(w) = self.0.worker.borrow().as_ref() {
            w.notify();
        }
    }

    /// The layout of the samples `on_sound` receives.
    pub fn format(&self) -> AudioFormat {
        self.0.format
    }

    /// What it is doing. Tracked.
    pub fn state(&self) -> MicrophoneState {
        self.0.state.get()
    }

    /// Why it failed, once it has. Tracked.
    pub fn error(&self) -> Option<MediaError> {
        self.0.error.get()
    }

    /// How loud it is now: the loudest sample of the last thirtieth of a second, 0 to 1 (a
    /// level meter's input). 0 when not listening. Tracked.
    pub fn level(&self) -> f32 {
        self.0.level.get()
    }
}

/// Converts interleaved sound from one rate and channel count to another, a chunk at a time,
/// carrying its place across chunks so the joins are seamless.
///
/// Channels first: several to one is their average, one to several is a copy in each, and
/// otherwise the first channels are kept and missing ones are silent. Then the rate, with
/// Catmull-Rom (cubic) interpolation between the four nearest frames: smooth enough for
/// pitch detection and meters, with no lag beyond two input frames.
pub(crate) struct Converter {
    from: AudioFormat,
    to: AudioFormat,
    /// Input frames advanced per output frame.
    step: f64,
    /// Where the next output frame falls, counted from the oldest kept frame.
    pos: f64,
    /// The last frames of the previous chunk (already in output channels), needed by the
    /// interpolation across the join.
    tail: Vec<f32>,
    /// This chunk's frames in output channels, after the tail.
    mixed: Vec<f32>,
}

impl Converter {
    pub(crate) fn new(from: AudioFormat, to: AudioFormat) -> Converter {
        let step = f64::from(from.sample_rate) / f64::from(to.sample_rate.max(1));
        // One silent frame before the start, so the first output frame has a left neighbour.
        let ch = usize::from(to.channels);
        Converter { from, to, step, pos: 1.0, tail: vec![0.0; ch], mixed: Vec::new() }
    }

    /// Appends `input` (whole frames in the source format) converted to `out`.
    pub(crate) fn convert(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let (fc, tc) = (usize::from(self.from.channels), usize::from(self.to.channels));
        if fc == 0 || tc == 0 {
            return;
        }
        self.mixed.clear();
        self.mixed.extend_from_slice(&self.tail);
        for frame in input.chunks_exact(fc) {
            if tc == 1 {
                self.mixed.push(frame.iter().sum::<f32>() / fc as f32);
            } else if fc == 1 {
                self.mixed.extend(std::iter::repeat_n(frame.first().copied().unwrap_or(0.0), tc));
            } else {
                self.mixed.extend((0..tc).map(|c| frame.get(c).copied().unwrap_or(0.0)));
            }
        }
        let frames = self.mixed.len() / tc;
        if self.from.sample_rate == self.to.sample_rate {
            // Channels only: everything after the carried frame goes out as it is.
            out.extend_from_slice(self.mixed.get(tc..).unwrap_or_default());
            self.tail.clear();
            self.tail.extend_from_slice(self.mixed.get((frames - 1) * tc..).unwrap_or_default());
            return;
        }
        let mixed = &self.mixed;
        let at = |i: isize, c: usize| -> f32 {
            let i = i.clamp(0, frames as isize - 1) as usize;
            mixed.get(i * tc + c).copied().unwrap_or(0.0)
        };
        // An output frame needs the frame after its right neighbour too: stop two short of
        // the end and carry the rest.
        while self.pos + 2.0 <= frames as f64 {
            let i = self.pos.floor() as isize;
            let t = (self.pos - self.pos.floor()) as f32;
            for c in 0..tc {
                let (p0, p1, p2, p3) = (at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c));
                let v = p1
                    + 0.5
                        * t
                        * (p2 - p0
                            + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
                out.push(v);
            }
            self.pos += self.step;
        }
        // Keep from one frame before the next output's frame, so its left neighbour stays.
        let keep_from = (self.pos.floor() as usize).saturating_sub(1).min(frames);
        self.pos -= keep_from as f64;
        self.tail.clear();
        self.tail.extend_from_slice(self.mixed.get(keep_from * tc..).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f64, frames: usize, channels: u16) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin() as f32;
                std::iter::repeat_n(v, usize::from(channels))
            })
            .collect()
    }

    /// The frequency from the rising zero crossings.
    fn frequency(samples: &[f32], rate: u32) -> f64 {
        let ups = samples.array_windows::<2>().filter(|[a, b]| *a < 0.0 && *b >= 0.0).count();
        ups as f64 * f64::from(rate) / samples.len() as f64
    }

    #[test]
    fn resampling_keeps_pitch_level_and_length_across_chunks() {
        let mut c = Converter::new(AudioFormat::new(44_100, 2), AudioFormat::new(48_000, 1));
        let mut out = Vec::new();
        // Uneven chunks, as devices hand them over.
        for chunk in sine(44_100, 440.0, 44_100, 2).chunks(2 * 441 + 6) {
            c.convert(chunk, &mut out);
        }
        let n = out.len() as i64;
        assert!((n - 48_000).abs() <= 3, "one second in is one second out: {n}");
        let f = frequency(&out, 48_000);
        assert!((f - 440.0).abs() < 1.5, "the note stays A4: {f}");
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.98..=1.02).contains(&peak), "no level change: {peak}");
        // The joins between chunks are smooth: no step larger than the sine's own (2π·440/48000 ≈ 0.058).
        let max_step = out.array_windows::<2>().map(|[a, b]| (b - a).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.065, "seamless joins: {max_step}");
    }

    #[test]
    fn channels_mix_down_and_copy_up() {
        let mut down = Converter::new(AudioFormat::new(48_000, 2), AudioFormat::new(48_000, 1));
        let mut out = Vec::new();
        down.convert(&[1.0, 0.0, 0.5, 0.5], &mut out);
        down.convert(&[-1.0, -1.0], &mut out);
        assert_eq!(out, vec![0.5, 0.5, -1.0]);
        let mut up = Converter::new(AudioFormat::new(48_000, 1), AudioFormat::new(48_000, 2));
        let mut out = Vec::new();
        up.convert(&[0.25, -0.5], &mut out);
        assert_eq!(out, vec![0.25, 0.25, -0.5, -0.5]);
    }

    #[test]
    fn downsampling_halves_the_frames_and_keeps_pitch() {
        let mut c = Converter::new(AudioFormat::new(96_000, 1), AudioFormat::new(48_000, 1));
        let mut out = Vec::new();
        for chunk in sine(96_000, 1000.0, 96_000, 1).chunks(960) {
            c.convert(chunk, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() <= 3, "{}", out.len());
        assert!((frequency(&out, 48_000) - 1000.0).abs() < 2.0);
    }
}
