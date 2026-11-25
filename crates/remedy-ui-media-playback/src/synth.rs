//! Making sound: oscillator voices with envelopes, a percussive click, a mixer for many at
//! once, and a transport that starts them on the beat, exactly, on the device's own clock
//! (tempo, looping, a count-in, a metronome with subdivisions), telling the interface where
//! the music is.
//!
//! The pieces are plain values and can be used alone: [`Mixer::render`] fills any buffer.
//! [`Transport`] puts them on a sound output.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use remedy_ui_reactive::Signal;

use crate::feed::AudioFeed;
use crate::host::{AudioFormat, Latency, MediaError, MediaHost};

/// An oscillator's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Wave {
    /// A pure tone.
    #[default]
    Sine,
    /// A softer, hollow tone with odd harmonics (a flute, a plucked string's body).
    Triangle,
}

impl Wave {
    /// The wave's value at `phase`, in turns (0 to 1 is one cycle), from -1 to 1.
    pub fn at(self, phase: f32) -> f32 {
        let p = phase - phase.floor();
        match self {
            Wave::Sine => (p * std::f32::consts::TAU).sin(),
            Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
        }
    }
}

/// How a note's loudness rises and falls: attack to full, decay to the sustain level, held
/// while the note lasts, then release to silence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adsr {
    /// The rise to full loudness.
    pub attack: Duration,
    /// The fall from full to the sustain level.
    pub decay: Duration,
    /// The level held, 0 to 1.
    pub sustain: f32,
    /// The fall to silence after the note ends.
    pub release: Duration,
}

impl Adsr {
    /// A plucked string: at once, dying away, nothing held.
    pub const PLUCK: Adsr = Adsr {
        attack: Duration::from_millis(2),
        decay: Duration::from_millis(600),
        sustain: 0.0,
        release: Duration::from_millis(80),
    };
    /// A held tone with soft edges (an organ, a tuning note).
    pub const HELD: Adsr = Adsr {
        attack: Duration::from_millis(10),
        decay: Duration::from_millis(60),
        sustain: 0.8,
        release: Duration::from_millis(120),
    };

    /// The loudness `t` seconds after the note began, for a note held `held` seconds.
    pub fn level(&self, t: f32, held: f32) -> f32 {
        let (a, d, r) = (self.attack.as_secs_f32(), self.decay.as_secs_f32(), self.release.as_secs_f32());
        let sustain = self.sustain.clamp(0.0, 1.0);
        let on = |t: f32| {
            if t < a {
                t / a
            } else if t < a + d {
                1.0 + (sustain - 1.0) * (t - a) / d
            } else {
                sustain
            }
        };
        if t < 0.0 {
            0.0
        } else if t <= held {
            on(t)
        } else if r > 0.0 && t < held + r {
            on(held) * (1.0 - (t - held) / r)
        } else {
            0.0
        }
    }

    /// How long a note held `held` seconds sounds, release included.
    fn sounds(&self, held: f32) -> f32 {
        held + self.release.as_secs_f32()
    }
}

/// A sound a [`Mixer`] plays.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Sound {
    /// A pitched note.
    Tone {
        /// Its shape.
        wave: Wave,
        /// Its pitch, in hertz.
        frequency: f32,
        /// How loud, 0 to 1.
        gain: f32,
        /// How long it is held before its release.
        length: Duration,
        /// Its envelope.
        envelope: Adsr,
    },
    /// A short percussive tick (a metronome, a tap): a pitched burst that dies away within
    /// `length`.
    Click {
        /// Its pitch, in hertz (1000 to 2000 reads as a tick).
        frequency: f32,
        /// How loud, 0 to 1.
        gain: f32,
        /// How long until it is gone.
        length: Duration,
    },
}

impl Sound {
    /// A note of `frequency` hertz held for `length`.
    pub fn tone(wave: Wave, frequency: f32, length: Duration) -> Sound {
        Sound::Tone { wave, frequency, gain: 0.5, length, envelope: Adsr::PLUCK }
    }

    /// The note of a MIDI number (69 is the A of 440 Hz) held for `length`.
    pub fn midi(wave: Wave, note: f32, length: Duration) -> Sound {
        Sound::tone(wave, midi_frequency(note), length)
    }

    /// A tick at `frequency` hertz.
    pub fn click(frequency: f32) -> Sound {
        Sound::Click { frequency, gain: 0.6, length: Duration::from_millis(30) }
    }

    /// The same sound at another loudness.
    pub fn gain(mut self, to: f32) -> Sound {
        match &mut self {
            Sound::Tone { gain, .. } | Sound::Click { gain, .. } => *gain = to.clamp(0.0, 1.0),
        }
        self
    }

    /// The same note with another envelope (a click has its own).
    pub fn envelope(mut self, to: Adsr) -> Sound {
        if let Sound::Tone { envelope, .. } = &mut self {
            *envelope = to;
        }
        self
    }

    /// How long it sounds, in seconds.
    fn seconds(&self) -> f32 {
        match self {
            Sound::Tone { length, envelope, .. } => envelope.sounds(length.as_secs_f32()),
            Sound::Click { length, .. } => length.as_secs_f32(),
        }
    }

    /// Its value `t` seconds in, with its oscillator at `phase` turns.
    fn sample(&self, t: f32, phase: f32) -> f32 {
        match self {
            Sound::Tone { wave, gain, length, envelope, .. } => {
                wave.at(phase) * gain * envelope.level(t, length.as_secs_f32())
            }
            Sound::Click { gain, length, .. } => {
                // A quick linear attack (no click of its own), then an exponential fall that
                // is 60 dB down at the end.
                let len = length.as_secs_f32().max(1e-4);
                let attack = (t / 0.001).min(1.0);
                Wave::Sine.at(phase) * gain * attack * (-6.9 * t / len).exp()
            }
        }
    }

    fn frequency(&self) -> f32 {
        match self {
            Sound::Tone { frequency, .. } | Sound::Click { frequency, .. } => *frequency,
        }
    }
}

/// The frequency of a MIDI note number (fractions bend it): 69 is 440 Hz.
pub fn midi_frequency(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

struct Voice {
    sound: Sound,
    /// Frames until it starts (it was asked for inside a block that has not got there yet).
    wait: u64,
    /// Frames played.
    at: u64,
    phase: f32,
}

/// Many sounds at once, mixed into one stream.
pub struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
    /// The most voices kept: a new one past it takes the place of the oldest.
    polyphony: usize,
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mixer({} Hz, {} sounding)", self.rate, self.voices.len())
    }
}

impl Mixer {
    /// A mixer for `sample_rate` frames per second, with up to 64 sounds at once.
    pub fn new(sample_rate: u32) -> Mixer {
        Mixer { rate: sample_rate.max(1), voices: Vec::new(), polyphony: 64 }
    }

    /// Changes how many sounds may play at once (at least one).
    pub fn set_polyphony(&mut self, voices: usize) {
        self.polyphony = voices.max(1);
        let over = self.voices.len().saturating_sub(self.polyphony);
        self.voices.drain(..over);
    }

    /// Starts `sound` `delay` frames from the start of the next [`Mixer::render`].
    pub fn play(&mut self, sound: Sound, delay: u64) {
        if self.voices.len() >= self.polyphony {
            self.voices.remove(0);
        }
        self.voices.push(Voice { sound, wait: delay, at: 0, phase: 0.0 });
    }

    /// How many sounds are playing or waiting to.
    pub fn sounding(&self) -> usize {
        self.voices.len()
    }

    /// Stops everything at once.
    pub fn silence(&mut self) {
        self.voices.clear();
    }

    /// Adds the next frames of every sound to `out`: interleaved, `channels` to a frame, the
    /// same signal in each. What is there already stays (so several mixers can share a
    /// buffer); the sum is not clipped here.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let rate = self.rate as f32;
        for v in &mut self.voices {
            let step = v.sound.frequency() / rate;
            for frame in out.chunks_exact_mut(channels) {
                if v.wait > 0 {
                    v.wait -= 1;
                    continue;
                }
                let s = v.sound.sample(v.at as f32 / rate, v.phase);
                for c in frame {
                    *c += s;
                }
                v.at += 1;
                v.phase = (v.phase + step).fract();
            }
        }
        self.voices.retain(|v| v.wait > 0 || (v.at as f32 / rate) < v.sound.seconds());
    }
}

/// A metronome's clicks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metronome {
    /// Beats in a bar: the first of each is accented.
    pub beats_per_bar: u32,
    /// Clicks to a beat: 1 for the beat alone, 2 for eighths, 3 for triplets, 4 for
    /// sixteenths.
    pub subdivisions: u32,
    /// The first beat of a bar.
    pub accent: Sound,
    /// The other beats.
    pub beat: Sound,
    /// The clicks between beats.
    pub subdivision: Sound,
}

impl Metronome {
    /// The usual clicks for bars of `beats_per_bar` beats, one click to a beat.
    pub fn new(beats_per_bar: u32) -> Metronome {
        Metronome {
            beats_per_bar: beats_per_bar.max(1),
            subdivisions: 1,
            accent: Sound::click(1760.0),
            beat: Sound::click(1320.0).gain(0.45),
            subdivision: Sound::click(990.0).gain(0.25),
        }
    }

    /// The same metronome with `subdivisions` clicks to a beat.
    pub fn subdivided(mut self, subdivisions: u32) -> Metronome {
        self.subdivisions = subdivisions.max(1);
        self
    }

    /// The click at subdivision `k` counted from beat 0.
    fn click(&self, k: i64) -> Sound {
        let sub = i64::from(self.subdivisions.max(1));
        if k.rem_euclid(sub) != 0 {
            self.subdivision
        } else if (k.div_euclid(sub)).rem_euclid(i64::from(self.beats_per_bar.max(1))) == 0 {
            self.accent
        } else {
            self.beat
        }
    }
}

/// The music a [`Transport`] plays, and where it is in it. Positions are beats (quarter
/// notes) from the start of the piece; a count-in runs through negative beats up to 0.
pub struct Sequencer {
    rate: u32,
    mixer: Mixer,
    /// Sorted by beat.
    events: Vec<(f64, Sound)>,
    tempo: f64,
    looped: Option<(f64, f64)>,
    metronome: Option<Metronome>,
    playing: bool,
    beat: f64,
    /// The beat a count-in leads up to: until the position gets there only clicks sound.
    /// Negative infinity when there is none.
    count_in_until: f64,
}

impl std::fmt::Debug for Sequencer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sequencer(beat {:.3}, {} bpm, playing: {})", self.beat, self.tempo, self.playing)
    }
}

impl Sequencer {
    /// A stopped sequencer for `sample_rate` frames per second, at 120 beats a minute.
    pub fn new(sample_rate: u32) -> Sequencer {
        Sequencer {
            rate: sample_rate.max(1),
            mixer: Mixer::new(sample_rate),
            events: Vec::new(),
            tempo: 120.0,
            looped: None,
            metronome: None,
            playing: false,
            beat: 0.0,
            count_in_until: f64::NEG_INFINITY,
        }
    }

    /// The sounds of the piece, each with the beat it starts on.
    pub fn set_events(&mut self, mut events: Vec<(f64, Sound)>) {
        events.retain(|(b, _)| b.is_finite());
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.events = events;
    }

    /// Beats a minute, from the next frame on (1 to 1000).
    pub fn set_tempo(&mut self, bpm: f64) {
        if bpm.is_finite() {
            self.tempo = bpm.clamp(1.0, 1000.0);
        }
    }

    /// The tempo, in beats a minute.
    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    /// Plays the beats from `from` up to `to` again and again, or with `None` straight on.
    pub fn set_loop(&mut self, range: Option<(f64, f64)>) {
        self.looped = range.filter(|(a, b)| a.is_finite() && b.is_finite() && b > a);
    }

    /// Clicks along, or with `None` does not.
    pub fn set_metronome(&mut self, metronome: Option<Metronome>) {
        self.metronome = metronome;
    }

    /// Starts playing at `beat`, after `count_in` beats of the metronome's clicks (or the
    /// usual ones when there is no metronome): the position runs from `beat - count_in`.
    pub fn play_from(&mut self, beat: f64, count_in: f64) {
        let count_in = if count_in.is_finite() { count_in.max(0.0) } else { 0.0 };
        self.beat = beat - count_in;
        self.count_in_until = if count_in > 0.0 { beat } else { f64::NEG_INFINITY };
        self.playing = true;
    }

    /// Stops where it is. Sounds already started ring out.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Whether it is playing.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The beat the next frame rendered is at.
    pub fn beat(&self) -> f64 {
        self.beat
    }

    /// Plays `sound` now, whatever the transport is doing (a tapped note).
    pub fn play_now(&mut self, sound: Sound) {
        self.mixer.play(sound, 0);
    }

    /// Fills `out` (interleaved, `channels` to a frame, zeroed by the caller) with the next
    /// frames: sounds start on the very frame their beat falls on.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        if self.playing {
            let per_frame = self.tempo / 60.0 / f64::from(self.rate);
            let mut done = 0usize;
            // A loop's end splits the block: the rest starts again from the loop's start.
            while done < frames {
                let left = frames - done;
                let reach = self.beat + left as f64 * per_frame;
                let (end, wraps) = match self.looped {
                    Some((_, to)) if self.beat < to && reach >= to => (to, true),
                    _ => (reach, false),
                };
                let span = (((end - self.beat) / per_frame).ceil() as usize).clamp(1, left);
                self.start_between(self.beat, end, done as u64, per_frame);
                done += span;
                self.beat = match (wraps, self.looped) {
                    (true, Some((from, _))) => from,
                    _ => self.beat + span as f64 * per_frame,
                };
                if self.beat >= self.count_in_until {
                    self.count_in_until = f64::NEG_INFINITY;
                }
            }
        }
        self.mixer.render(out, channels);
    }

    /// Starts every sound whose beat is in `from..to`, `offset` frames plus its own place
    /// into the block.
    fn start_between(&mut self, from: f64, to: f64, offset: u64, per_frame: f64) {
        let delay = |beat: f64| offset + ((beat - from) / per_frame).round().max(0.0) as u64;
        let first = self.events.partition_point(|(b, _)| *b < from);
        // Nothing of the piece sounds during a count-in.
        for (beat, sound) in self.events.iter().skip(first).take_while(|(b, _)| *b < to) {
            if *beat >= self.count_in_until {
                self.mixer.play(*sound, delay(*beat));
            }
        }
        // Clicks: the metronome's, and during a count-in the usual ones without it.
        let counting = from < self.count_in_until;
        let clicks = match (self.metronome, counting) {
            (Some(m), _) => Some(m),
            (None, true) => Some(Metronome::new(4)),
            (None, false) => None,
        };
        if let Some(m) = clicks {
            let sub = f64::from(m.subdivisions.max(1));
            let mut k = (from * sub).ceil() as i64;
            while (k as f64) / sub < to {
                let beat = k as f64 / sub;
                if self.metronome.is_some() || beat < self.count_in_until {
                    self.mixer.play(m.click(k), delay(beat));
                }
                k += 1;
            }
        }
    }
}

thread_local! {
    /// The position signals of the transports made on this (the interface's) thread: the
    /// audio worker posts positions here by number, since a signal stays on its thread.
    static POSITIONS: RefCell<HashMap<u64, Signal<Option<f32>>>> = RefCell::new(HashMap::new());
}

static NEXT_TRANSPORT: AtomicU64 = AtomicU64::new(1);

/// A [`Sequencer`] playing on a sound output with the lowest latency the device gives:
/// start it once and steer it from the interface.
///
/// ```no_run
/// # use std::time::Duration;
/// # use remedy_ui_media_playback::{platform_host, synth::*};
/// # fn demo() -> Result<(), remedy_ui_media_playback::MediaError> {
/// let transport = Transport::start(platform_host()?)?;
/// transport.edit(|s| {
///     s.set_tempo(96.0);
///     s.set_metronome(Some(Metronome::new(4).subdivided(2)));
///     s.set_events(vec![(0.0, Sound::midi(Wave::Triangle, 64.0, Duration::from_millis(400)))]);
///     s.play_from(0.0, 4.0);
/// });
/// let playhead = transport.position(); // a signal, for a tab's or a staff's playhead
/// # let _ = playhead; Ok(()) }
/// ```
pub struct Transport {
    id: u64,
    seq: Arc<Mutex<Sequencer>>,
    position: Signal<Option<f32>>,
    feed: AudioFeed,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Transport({:?})", self.feed)
    }
}

fn locked(seq: &Mutex<Sequencer>) -> MutexGuard<'_, Sequencer> {
    seq.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Transport {
    /// Opens `host`'s output (48 kHz stereo, interactive latency) and starts a stopped
    /// sequencer on it. Call it on the interface's thread: [`Transport::position`] is a
    /// signal of that thread.
    pub fn start(host: Arc<dyn MediaHost>) -> Result<Transport, MediaError> {
        let format = AudioFormat::new(48_000, 2);
        let seq = Arc::new(Mutex::new(Sequencer::new(format.sample_rate)));
        let id = NEXT_TRANSPORT.fetch_add(1, Ordering::Relaxed);
        let position = Signal::new(None);
        POSITIONS.with(|p| p.borrow_mut().insert(id, position));
        let waker = remedy_ui_reactive::waker().ok();
        let audio = seq.clone();
        let mut told: Option<(u64, Option<f32>)> = None;
        let feed = AudioFeed::start(host, format, Latency::Interactive, move |out, at| {
            let (beat, rate) = {
                let mut s = locked(&audio);
                s.render(out, usize::from(format.channels));
                (s.is_playing().then(|| s.beat() as f32), f64::from(format.sample_rate))
            };
            for v in out.iter_mut() {
                *v = v.clamp(-1.0, 1.0);
            }
            // The interface hears of the position about sixty times a second, and once more
            // when it stops.
            let due = told.is_none_or(|(when, was)| {
                (was.is_some() != beat.is_some()) || (at.saturating_sub(when)) as f64 >= rate / 60.0
            });
            if due && told.map(|(_, was)| was) != Some(beat) {
                told = Some((at, beat));
                if let Some(w) = &waker {
                    let _ = w.post(move || {
                        POSITIONS.with(|p| {
                            if let Some(signal) = p.borrow().get(&id) {
                                let _ = signal.try_set(beat);
                            }
                        });
                    });
                }
            }
        })?;
        Ok(Transport { id, seq, position, feed })
    }

    /// Changes the music or the transport (tempo, loop, metronome, play, stop) in one step
    /// the audio never sees half of. Keep it short: the audio waits for it.
    pub fn edit<R>(&self, f: impl FnOnce(&mut Sequencer) -> R) -> R {
        f(&mut locked(&self.seq))
    }

    /// Plays `sound` now (a tapped note, a fretboard press).
    pub fn play_now(&self, sound: Sound) {
        locked(&self.seq).play_now(sound);
    }

    /// Where the music is, in beats from the start of the piece (negative during a count-in),
    /// about sixty times a second; `None` while stopped. What is heard lags it by the
    /// output's latency ([`Transport::latency`]).
    pub fn position(&self) -> Signal<Option<f32>> {
        self.position
    }

    /// The longest a sound waits in the device before it is heard.
    pub fn latency(&self) -> Duration {
        self.feed.buffer()
    }

    /// Why nothing is heard, when the output could not open or the device went away.
    pub fn error(&self) -> Option<MediaError> {
        self.feed.error()
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        let _ = POSITIONS.try_with(|p| p.borrow_mut().remove(&self.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frames at which the output goes from silence to sound.
    /// The `i`th onset (far away when there is none, so comparisons fail).
    fn nth(found: &[usize], i: usize) -> usize {
        found.get(i).copied().unwrap_or(usize::MAX)
    }

    fn onsets(samples: &[f32]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut quiet = 2_000usize;
        for (i, s) in samples.iter().enumerate() {
            if s.abs() > 1e-4 {
                if quiet >= 2_000 {
                    out.push(i);
                }
                quiet = 0;
            } else {
                quiet += 1;
            }
        }
        out
    }

    #[test]
    fn waves_and_envelopes_have_their_shapes() {
        assert!(Wave::Sine.at(0.25) > 0.999 && Wave::Sine.at(0.75) < -0.999);
        assert!((Wave::Triangle.at(0.5) - 1.0).abs() < 1e-6 && (Wave::Triangle.at(0.0) + 1.0).abs() < 1e-6);
        assert!(Wave::Triangle.at(0.25).abs() < 1e-6);
        let e = Adsr {
            attack: Duration::from_millis(100),
            decay: Duration::from_millis(100),
            sustain: 0.5,
            release: Duration::from_millis(200),
        };
        assert!((e.level(0.05, 1.0) - 0.5).abs() < 1e-5, "half way up the attack");
        assert!((e.level(0.15, 1.0) - 0.75).abs() < 1e-5, "half way down the decay");
        assert!((e.level(0.6, 1.0) - 0.5).abs() < 1e-5, "held");
        assert!((e.level(1.1, 1.0) - 0.25).abs() < 1e-5, "half way through the release");
        assert_eq!(e.level(1.3, 1.0), 0.0);
        assert!((midi_frequency(69.0) - 440.0).abs() < 1e-3 && (midi_frequency(57.0) - 220.0).abs() < 1e-3);
    }

    #[test]
    fn the_mixer_plays_many_sounds_and_lets_them_end() {
        let mut m = Mixer::new(48_000);
        m.play(Sound::click(1000.0), 0);
        m.play(Sound::click(1500.0), 480);
        assert_eq!(m.sounding(), 2);
        let mut out = vec![0.0f32; 48_000 * 2];
        m.render(&mut out, 2);
        assert!(out.as_chunks::<2>().0.iter().all(|[l, r]| l == r), "the same in both channels");
        assert!(out.iter().take(960 * 2).any(|s| s.abs() > 0.05));
        assert_eq!(m.sounding(), 0, "both are over within the second");
        m.set_polyphony(1);
        m.play(Sound::click(1000.0), 0);
        m.play(Sound::click(1000.0), 0);
        assert_eq!(m.sounding(), 1);
    }

    #[test]
    fn sounds_start_on_the_exact_frame_of_their_beat_at_any_block_size() {
        // 120 beats a minute at 48 kHz: a beat is 24 000 frames.
        for block in [64usize, 480, 1000, 4096] {
            let mut s = Sequencer::new(48_000);
            s.set_events(vec![
                (0.0, Sound::click(1000.0)),
                (1.0, Sound::click(1000.0)),
                (2.5, Sound::click(1000.0)),
            ]);
            s.play_from(0.0, 0.0);
            let mut all = Vec::new();
            while all.len() < 72_000 {
                let mut out = vec![0.0f32; block];
                s.render(&mut out, 1);
                all.extend(out);
            }
            let found = onsets(&all);
            let near = |at: usize, want: usize| at.abs_diff(want) <= 2;
            assert!(
                found.len() == 3
                    && near(nth(&found, 0), 0)
                    && near(nth(&found, 1), 24_000)
                    && near(nth(&found, 2), 60_000),
                "block {block}: {found:?}"
            );
        }
    }

    #[test]
    fn tempo_loops_count_ins_and_subdivisions_keep_time() {
        // A count-in of two beats clicks at -2 and -1, then the piece starts at 0.
        let mut s = Sequencer::new(48_000);
        s.set_events(vec![(0.0, Sound::click(1000.0))]);
        s.play_from(0.0, 2.0);
        assert_eq!(s.beat(), -2.0);
        let mut out = vec![0.0f32; 60_000];
        s.render(&mut out, 1);
        let found = onsets(&out);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(nth(&found, 1).abs_diff(24_000) <= 2 && nth(&found, 2).abs_diff(48_000) <= 2);
        // Twice the tempo: beats half as far apart. A loop of one beat repeats its sound.
        let mut s = Sequencer::new(48_000);
        s.set_tempo(240.0);
        s.set_loop(Some((0.0, 1.0)));
        s.set_events(vec![(0.0, Sound::click(1000.0))]);
        s.play_from(0.0, 0.0);
        let mut out = vec![0.0f32; 40_000];
        s.render(&mut out, 1);
        let found = onsets(&out);
        assert!(
            found.len() == 4 && nth(&found, 1).abs_diff(12_000) <= 2 && nth(&found, 3).abs_diff(36_000) <= 2,
            "{found:?}"
        );
        assert!(s.beat() < 1.0, "inside the loop");
        // A metronome in 3 with two clicks to a beat: six clicks in three beats.
        let mut s = Sequencer::new(48_000);
        s.set_metronome(Some(Metronome::new(3).subdivided(2)));
        s.play_from(0.0, 0.0);
        let mut out = vec![0.0f32; 71_000];
        s.render(&mut out, 1);
        assert_eq!(onsets(&out).len(), 6);
        s.stop();
        assert!(!s.is_playing());
    }
}
