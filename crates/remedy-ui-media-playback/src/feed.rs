//! Feeding a sound output by callback: the output asks for samples as it has room, on a
//! worker of its own, so an app that makes sound (an instrument, a metronome, a game) does
//! not poll the device from its own thread.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use remedy_ui_core::exec::{Next, Worker};

use crate::host::{AudioFormat, AudioOutput, Latency, MediaError, MediaHost};

/// What a feed's worker and its handle share.
struct Shared {
    stop: AtomicBool,
    written: AtomicU64,
    played: AtomicU64,
    /// The device's buffer, in frames (0 until the output is open).
    buffer: AtomicU64,
    error: Mutex<Option<MediaError>>,
}

/// A sound output fed by a callback ([`AudioFeed::start`]). It plays until it is dropped or
/// [`AudioFeed::stop`]ped.
pub struct AudioFeed {
    shared: Arc<Shared>,
    format: AudioFormat,
    worker: Worker,
}

impl std::fmt::Debug for AudioFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AudioFeed({} Hz, {} channels)", self.format.sample_rate, self.format.channels)
    }
}

struct State {
    out: Option<Box<dyn AudioOutput>>,
    fill: Box<dyn FnMut(&mut [f32], u64) + Send>,
    block: Vec<f32>,
    tick: Duration,
}

impl AudioFeed {
    /// Opens `host`'s sound output for `format` with `latency` and plays what `fill` writes.
    ///
    /// `fill(samples, position)` is called whenever the device has room, on the feed's own
    /// worker (a thread; in a browser page a task on the page's thread): `samples` is
    /// interleaved, zeroed, and as many whole frames as fit; `position` is the frame the
    /// first of them is, counted from the start. It must not block: it runs inside the
    /// latency it is there to keep.
    ///
    /// The output opens on the worker, so a device that fails to open is reported by
    /// [`AudioFeed::error`], not here; this fails only when no worker could start.
    pub fn start(
        host: Arc<dyn MediaHost>,
        format: AudioFormat,
        latency: Latency,
        fill: impl FnMut(&mut [f32], u64) + Send + 'static,
    ) -> Result<AudioFeed, MediaError> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            written: AtomicU64::new(0),
            played: AtomicU64::new(0),
            buffer: AtomicU64::new(0),
            error: Mutex::new(None),
        });
        let (made, stepped) = (shared.clone(), shared.clone());
        let worker = Worker::spawn_timed(
            "remedy-audio-feed",
            move |_| {
                let out = host.audio_output_with(format, latency).and_then(|mut out| {
                    out.resume()?;
                    Ok(out)
                });
                let (out, tick) = match out {
                    Ok(out) => {
                        let buffer = out.buffer();
                        made.buffer.store(format.frames_in(buffer), Ordering::Relaxed);
                        // Four refills per buffer, between one and ten milliseconds apart.
                        let tick = (buffer / 4).clamp(Duration::from_millis(1), Duration::from_millis(10));
                        (Some(out), tick)
                    }
                    Err(e) => {
                        *made.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
                        (None, Duration::ZERO)
                    }
                };
                State { out, fill: Box::new(fill), block: Vec::new(), tick }
            },
            move |s: &mut State| {
                if stepped.stop.load(Ordering::Relaxed) {
                    if let Some(mut out) = s.out.take() {
                        let _ = out.pause();
                    }
                    return Next::Idle;
                }
                let Some(out) = s.out.as_mut() else { return Next::Idle };
                let channels = usize::from(format.channels.max(1));
                let step = out.room().and_then(|room| {
                    if room > 0 {
                        s.block.clear();
                        s.block.resize(room * channels, 0.0);
                        let at = stepped.written.load(Ordering::Relaxed);
                        (s.fill)(&mut s.block, at);
                        out.write(&s.block)?;
                        stepped.written.store(at + room as u64, Ordering::Relaxed);
                    }
                    out.played()
                });
                match step {
                    Ok(played) => {
                        stepped.played.store(played, Ordering::Relaxed);
                        Next::After(s.tick)
                    }
                    Err(e) => {
                        // The device went away: say so and stop asking.
                        *stepped.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
                        s.out = None;
                        Next::Idle
                    }
                }
            },
        )
        .map_err(|e| MediaError::Unsupported(format!("no worker for the sound output: {}", e.reason())))?;
        Ok(AudioFeed { shared, format, worker })
    }

    /// The format `fill` writes.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Frames `fill` has written so far.
    pub fn written(&self) -> u64 {
        self.shared.written.load(Ordering::Relaxed)
    }

    /// Frames heard so far: the audio clock, as of the feed's last refill.
    pub fn played(&self) -> u64 {
        self.shared.played.load(Ordering::Relaxed)
    }

    /// How much sound the device holds when full: the longest a written sample waits to be
    /// heard. Zero until the output has opened.
    pub fn buffer(&self) -> Duration {
        self.format.duration_of(self.shared.buffer.load(Ordering::Relaxed))
    }

    /// Why the output is not playing, when it could not open or the device went away.
    pub fn error(&self) -> Option<MediaError> {
        self.shared.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Stops playing and closes the output. Dropping the feed does the same.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.worker.notify();
    }
}

impl Drop for AudioFeed {
    fn drop(&mut self) {
        self.stop();
    }
}
