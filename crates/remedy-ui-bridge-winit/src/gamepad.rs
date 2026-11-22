//! Gamepads the platform bridge reads itself (XInput on Windows, evdev on Linux): a thread
//! reads them from a [`PadSource`] and turns what they hold into keys ([`GamepadKeys`]), with
//! held directions repeating on time, and the loop hands those keys to the focused window.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use remedy_ui_host::Waker;
use remedy_ui_input::{GamepadKeys, GamepadTuning, KeyEvent, PadState};

/// The platform's gamepads, read on the bridge's own thread.
pub trait PadSource: Send + 'static {
    /// Waits until a pad changes, one is plugged in or unplugged, or `wait` passes (`None`:
    /// as long as it takes), then returns each pad's reading by its slot: `None` for a pad
    /// that was unplugged. A source that must poll returns every connected pad each time.
    fn read(&mut self, wait: Option<Duration>) -> Vec<(usize, Option<PadState>)>;
}

#[derive(Default)]
struct Shared {
    keys: Vec<KeyEvent>,
    /// A tuning or system repeat the thread has not taken yet.
    tuning: Option<GamepadTuning>,
    system: Option<(Duration, Duration)>,
    /// The system repeat last sent, so an unchanged one is not sent again.
    last_system: Option<(Duration, Duration)>,
}

/// The reading thread's mailbox.
pub(crate) struct Pads(Arc<Mutex<Shared>>);

impl Pads {
    /// Starts reading `source`; `wake` is called when keys are waiting.
    pub(crate) fn spawn(mut source: Box<dyn PadSource>, wake: Waker, system: (Duration, Duration)) -> Pads {
        let shared = Arc::new(Mutex::new(Shared {
            system: Some(system),
            last_system: Some(system),
            ..Shared::default()
        }));
        let mailbox = shared.clone();
        let started = std::thread::Builder::new().name("remedy-gamepads".into()).spawn(move || {
            let start = Instant::now();
            let mut keys = GamepadKeys::new();
            // Ends once the loop has dropped its side (the app quit).
            while Arc::strong_count(&mailbox) > 1 {
                {
                    let mut s = mailbox.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(t) = s.tuning.take() {
                        keys.set_tuning(t);
                    }
                    if let Some((d, i)) = s.system.take() {
                        keys.set_system_repeat(d, i);
                    }
                }
                let now = start.elapsed();
                // A due repeat is waited for at least a millisecond, so even a zero interval
                // leaves the thread time to sleep.
                let wait = keys.next_due().map(|at| at.saturating_sub(now).max(Duration::from_millis(1)));
                let readings = source.read(wait);
                let now = start.elapsed();
                let mut out = Vec::new();
                for (pad, state) in readings {
                    out.extend(keys.update(pad, state.as_ref(), now));
                }
                out.extend(keys.tick(now));
                if !out.is_empty() {
                    mailbox.lock().unwrap_or_else(PoisonError::into_inner).keys.extend(out);
                    wake();
                }
            }
        });
        // Without a thread there are no gamepads, as on a machine with none plugged in.
        drop(started);
        Pads(shared)
    }

    fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The app's tuning, taken at the thread's next reading.
    pub(crate) fn set_tuning(&self, tuning: GamepadTuning) {
        self.with(|s| s.tuning = Some(tuning));
    }

    /// The system keyboard's repeat, when it changed.
    pub(crate) fn set_system(&self, delay: Duration, interval: Duration) {
        self.with(|s| {
            if s.last_system != Some((delay, interval)) {
                s.last_system = Some((delay, interval));
                s.system = Some((delay, interval));
            }
        });
    }

    /// The keys the pads made since the last call.
    pub(crate) fn take(&self) -> Vec<KeyEvent> {
        self.with(|s| std::mem::take(&mut s.keys))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use remedy_ui_input::{Key, PadButtons};

    use super::*;

    /// Readings from a channel, as a platform's would arrive.
    struct Fake(mpsc::Receiver<(usize, Option<PadState>)>);

    impl PadSource for Fake {
        fn read(&mut self, wait: Option<Duration>) -> Vec<(usize, Option<PadState>)> {
            match wait {
                Some(w) => self.0.recv_timeout(w).ok().into_iter().collect(),
                None => self.0.recv().ok().into_iter().collect(),
            }
        }
    }

    fn wait_for(pads: &Pads, n: usize) -> Vec<KeyEvent> {
        let mut got = Vec::new();
        let until = Instant::now() + Duration::from_secs(10);
        while got.len() < n && Instant::now() < until {
            got.extend(pads.take());
            std::thread::sleep(Duration::from_millis(2));
        }
        got
    }

    #[test]
    fn readings_become_keys_and_a_held_direction_repeats_on_its_own() {
        let (tx, rx) = mpsc::channel();
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        let pads = Pads::spawn(
            Box::new(Fake(rx)),
            Arc::new(move || {
                w.fetch_add(1, Ordering::SeqCst);
            }),
            (Duration::from_secs(60), Duration::from_secs(60)),
        );
        pads.set_tuning(
            GamepadTuning::default()
                .with_repeat_delay(Duration::from_millis(30))
                .with_repeat_interval(Duration::from_millis(10)),
        );
        // The first reading lets the thread take the tuning before the press.
        tx.send((0, Some(PadState::default()))).ok();
        tx.send((0, Some(PadState { buttons: PadButtons::DOWN, stick: [0.0, 0.0] }))).ok();
        let keys = wait_for(&pads, 3);
        let (first, rest) = keys.split_first().unwrap_or_else(|| unreachable!("no keys"));
        assert_eq!((first.key.clone(), first.pressed, first.repeat), (Key::ArrowDown, true, false));
        assert!(!rest.is_empty() && rest.iter().all(|k| k.key == Key::ArrowDown && k.repeat), "{keys:?}");
        assert!(woken.load(Ordering::SeqCst) > 0);
        tx.send((0, None)).ok();
        let keys = wait_for(&pads, 1);
        assert!(keys.iter().any(|k| !k.pressed), "unplugging lets go: {keys:?}");
    }
}
