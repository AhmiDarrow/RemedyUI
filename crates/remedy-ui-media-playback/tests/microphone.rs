//! The microphone handle on a scripted input: sound reaches the app on the worker, converted
//! where the device gives its own format, with state, level and errors as signals on the UI
//! thread; and on the platform's own input where this machine has one.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remedy_ui_media_playback::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn wait(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    loop {
        let _ = remedy_ui_reactive::run_posted();
        if ok() {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "waited for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// An input that hands out a 0.5-amplitude square wave in its own format, a period at a
/// time, then fails when told to (a device unplugged).
struct ScriptedInput {
    format: AudioFormat,
    period: Duration,
    started: Instant,
    given: u64,
    unplug: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl AudioInput for ScriptedInput {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn period(&self) -> Duration {
        self.period
    }

    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError> {
        if self.unplug.load(Ordering::Acquire) {
            return Err(MediaError::Device("the microphone was unplugged".into()));
        }
        let due = self.format.frames_in(self.started.elapsed());
        let n = due.saturating_sub(self.given);
        for i in 0..n {
            let v = if ((self.given + i) / 50).is_multiple_of(2) { 0.5 } else { -0.5 };
            into.extend(std::iter::repeat_n(v, usize::from(self.format.channels)));
        }
        self.given += n;
        Ok(n as usize)
    }
}

impl Drop for ScriptedInput {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}

struct ScriptedHost {
    /// The format the device gives, whatever is asked.
    device: Option<AudioFormat>,
    unplug: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl MediaHost for ScriptedHost {
    fn name(&self) -> &str {
        "scripted"
    }

    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Err(MediaError::Unsupported("sound in only".into()))
    }

    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Unsupported("sound in only".into()))
    }

    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        let Some(device) = self.device else {
            return Err(MediaError::Denied("the person said no".into()));
        };
        let _ = format;
        Ok(Box::new(ScriptedInput {
            format: device,
            period: Duration::from_millis(5),
            started: Instant::now(),
            given: 0,
            unplug: self.unplug.clone(),
            dropped: self.dropped.clone(),
        }))
    }
}

fn host(device: Option<AudioFormat>) -> (Arc<dyn MediaHost>, Arc<AtomicBool>, Arc<AtomicBool>) {
    let unplug = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    (Arc::new(ScriptedHost { device, unplug: unplug.clone(), dropped: dropped.clone() }), unplug, dropped)
}

#[test]
fn sound_reaches_the_app_in_the_asked_for_format_with_a_level() {
    let _ui = ui();
    // The device gives 44.1 kHz stereo; the app asked for 48 kHz mono.
    let (h, unplug, dropped) = host(Some(AudioFormat::new(44_100, 2)));
    let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
    let on_ui = Arc::new(AtomicBool::new(false));
    let ui_thread = std::thread::current().id();
    let (h2, o2) = (heard.clone(), on_ui.clone());
    let mic = Microphone::open(h, MicrophoneOptions::default(), move |s| {
        if std::thread::current().id() == ui_thread {
            o2.store(true, Ordering::Release);
        }
        h2.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend_from_slice(s);
    });
    assert_eq!(mic.format(), AudioFormat::new(48_000, 1));
    wait("listening", || mic.state() == MicrophoneState::Listening);
    wait("a quarter second of sound", || heard.lock().map_or(0, |h| h.len()) >= 12_000);
    assert!(!on_ui.load(Ordering::Acquire), "the sound is handled off the UI thread");
    wait("a level", || mic.level() > 0.4);
    assert!(
        mic.level() <= 0.65,
        "the level is the loudest sample (cubic overshoot at the edges aside): {}",
        mic.level()
    );
    let peak = heard.lock().map_or(0.0, |h| h.iter().fold(0.0f32, |m, v| m.max(v.abs())));
    assert!((0.45..0.65).contains(&peak), "converted, not rescaled: {peak}");

    // A device that goes away fails the microphone and lets the device go.
    unplug.store(true, Ordering::Release);
    wait("the failure", || mic.state() == MicrophoneState::Failed);
    assert_eq!(mic.error(), Some(MediaError::Device("the microphone was unplugged".into())));
    assert_eq!(mic.level(), 0.0);
    wait("the device let go", || dropped.load(Ordering::Acquire));
}

#[test]
fn stopping_lets_the_device_go_and_no_sound_follows() {
    let _ui = ui();
    let (h, _, dropped) = host(Some(AudioFormat::new(48_000, 1)));
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let mic = Microphone::open(h, MicrophoneOptions::default(), move |_| {
        c2.fetch_add(1, Ordering::AcqRel);
    });
    wait("listening", || mic.state() == MicrophoneState::Listening);
    mic.stop();
    wait("stopped", || mic.state() == MicrophoneState::Stopped);
    wait("the device let go", || dropped.load(Ordering::Acquire));
    let after = calls.load(Ordering::Acquire);
    std::thread::sleep(Duration::from_millis(50));
    let _ = remedy_ui_reactive::run_posted();
    assert_eq!(calls.load(Ordering::Acquire), after, "nothing is heard after stopping");
    assert_eq!(mic.error(), None);
}

#[test]
fn dropping_the_last_handle_stops_it() {
    let _ui = ui();
    let (h, _, dropped) = host(Some(AudioFormat::new(48_000, 1)));
    let mic = Microphone::open(h, MicrophoneOptions::default(), |_| {});
    wait("listening", || mic.state() == MicrophoneState::Listening);
    drop(mic);
    wait("the device let go", || dropped.load(Ordering::Acquire));
}

#[test]
fn a_refused_microphone_says_why() {
    let _ui = ui();
    let (h, _, _) = host(None);
    let mic = Microphone::open(h, MicrophoneOptions::default(), |_| {});
    wait("the refusal", || mic.state() == MicrophoneState::Failed);
    assert!(matches!(mic.error(), Some(MediaError::Denied(_))));
    let bad =
        Microphone::open(host(None).0, MicrophoneOptions::default().format(AudioFormat::new(0, 1)), |_| {});
    assert_eq!(bad.state(), MicrophoneState::Failed, "a format with no rate fails at once");
}

/// The platform's microphone, where this machine has one: either it hears, or it says why
/// not (no device, privacy settings). It never hangs or fails silently.
#[test]
fn the_platform_microphone_hears_or_says_why() {
    let _ui = ui();
    let Ok(h) = platform_host() else {
        eprintln!("no platform media host here, skipping");
        return;
    };
    let got = Arc::new(AtomicUsize::new(0));
    let g2 = got.clone();
    let mic = Microphone::open(h, MicrophoneOptions::default(), move |s| {
        g2.fetch_add(s.len(), Ordering::AcqRel);
    });
    wait("an outcome", || {
        matches!(mic.state(), MicrophoneState::Failed) || got.load(Ordering::Acquire) >= 4_800
    });
    match mic.error() {
        Some(e) => eprintln!("this machine's microphone: {e}"),
        None => assert_eq!(mic.state(), MicrophoneState::Listening),
    }
}
