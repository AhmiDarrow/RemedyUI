//! Audio meter: a level meter with a voice-activation threshold the user drags or sets from
//! the keyboard, and a speaking ring around an avatar, both following one level signal.
//!
//! A real app sets the level from its audio callback (through a `Waker`, or a timer that
//! reads the callback's latest peak). This recipe has no audio device: it feeds the visuals
//! a test signal, speech-like bursts generated on a timer, and says so on screen.
//!
//! Run it with `cargo run -p remedy-ui --example audio_meter`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::core::Color;
use remedy_ui::layout::Align;
use remedy_ui::reactive::Signal;
use remedy_ui::view::Space;
use remedy_ui::viz::{MeterOptions, RingOptions, level_meter, speaking_ring};
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// How often the test signal moves.
pub const STEP: Duration = Duration::from_millis(50);

/// The test signal's level at step `k`: two seconds of speech-like bursts, then a second of
/// near silence, over and over.
pub fn test_signal(k: u64) -> f32 {
    let phase = k % 60;
    if phase < 40 {
        let t = phase as f32 * 0.45;
        (0.45 + 0.3 * t.sin() + 0.1 * (t * 2.7).sin()).clamp(0.0, 1.0)
    } else {
        0.02
    }
}

/// What the panel shows and the user sets.
#[derive(Clone, Copy, Debug)]
pub struct Meter {
    /// The level, 0 to 1.
    pub level: Signal<f32>,
    /// The voice-activation threshold, 0 to 1.
    pub threshold: Signal<f32>,
    /// The test signal is paused.
    pub paused: Signal<bool>,
}

/// Opens the meter window and starts the test signal.
pub fn app(app: &App) {
    let meter = Meter { level: Signal::new(0.0), threshold: Signal::new(0.2), paused: Signal::new(false) };
    app.open(WindowSpec::new("Audio meter").size(420.0, 300.0), move |ui, _| {
        ui.root().child(panel(ui, meter));
    });
    feed(app, meter, Rc::new(Cell::new(0)));
}

/// Moves the level along the test signal every [`STEP`] while not paused.
fn feed(app: &App, meter: Meter, step: Rc<Cell<u64>>) {
    let a = app.clone();
    app.after(STEP, move || {
        if meter.paused.get_untracked().unwrap_or(false) {
            meter.level.set(0.0);
        } else {
            let k = step.get();
            step.set(k + 1);
            meter.level.set(test_signal(k));
        }
        feed(&a, meter, step);
    });
}

/// The panel: the ring around an avatar, the meter with its threshold, and the controls.
pub fn panel(ui: &Ui, m: Meter) -> El {
    let avatar = w::avatar(ui, "Test signal", ui.theme().spacing.s(12), Color::rgb8(61, 139, 95));
    let speaking = Signal::new(false);
    let ring = speaking_ring(ui, "Test signal", m.level, avatar, RingOptions::default().speaking(speaking));
    let meter = level_meter(
        ui,
        "Test signal level",
        m.level,
        MeterOptions::default().threshold(m.threshold).threshold_label("Voice activation threshold".into()),
    );
    let threshold = m.threshold;
    w::column(ui)
        .gap(Space::S3)
        .padding(Space::S6)
        .fill_width()
        .child(
            w::row(ui).gap(Space::S3).align(Align::Center).child(ring).child(
                w::text(ui, move || if speaking.get() { "Speaking".to_owned() } else { "Silent".to_owned() })
                    .variant("muted"),
            ),
        )
        .child(meter)
        .child(
            w::text(ui, move || format!("Voice activates above {:.0}%", threshold.get() * 100.0))
                .variant("muted")
                .data_text(),
        )
        .child(w::toggle_button(ui, "Pause test signal", m.paused))
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Audio meter"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
