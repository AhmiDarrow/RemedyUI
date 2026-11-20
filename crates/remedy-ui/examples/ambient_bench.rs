//! Ambient-motion cost in a real window: runs the reference scene (a cached backdrop, 50
//! animated sprites, a sweeping beam) through phases and prints, for each, the scene's frames a
//! second and this process's CPU time as a share of one core.
//!
//! Phases: an idle window; the scene with the ambient API, focused; the same with another
//! window focused (FreeSpeak in the background); minimized; then the naive way (one painter
//! drawing everything every frame) focused, for comparison.
//!
//! ```text
//! cargo run -p remedy-ui --example ambient_bench --release [-- seconds-per-phase]
//! ```
//!
//! CPU time is read from the system for this process only, so other programs' load does not
//! count. Windows and Linux (other systems print frames a second without CPU).

#[path = "../tests/ambient_scene/mod.rs"]
mod ambient_scene;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ambient_scene::{Drive, HEIGHT, WIDTH, scene};
use remedy_ui::{App, AppConfig, Window, WindowSpec};

/// Times a painter of the scene ran: the scene's frames.
static PAINTED: AtomicU64 = AtomicU64::new(0);

/// This process's CPU time (user + kernel), where the system says.
fn cpu_time() -> Option<Duration> {
    if cfg!(target_os = "linux") {
        // utime and stime, fields 14 and 15 of /proc/self/stat, in clock ticks (100 a second
        // on Linux unless the kernel was built otherwise). The command name, field 2, may
        // hold spaces: count from after its closing parenthesis.
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let rest = stat.get(stat.rfind(')')? + 2..)?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
        return Some(Duration::from_millis(ticks * 10));
    }
    if !cfg!(windows) {
        return None;
    }
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).TotalProcessorTime.TotalMilliseconds"),
        ])
        .output()
        .ok()?;
    let ms: f64 = String::from_utf8_lossy(&out.stdout).trim().replace(',', ".").parse().ok()?;
    Some(Duration::from_secs_f64(ms / 1000.0))
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Idle,
    Focused,
    Background,
    Minimized,
    Naive,
}

const PHASES: [Phase; 5] = [Phase::Idle, Phase::Focused, Phase::Background, Phase::Minimized, Phase::Naive];

struct Bench {
    win: Window,
    other: Option<Window>,
    at: usize,
    started: Option<(Instant, Option<Duration>, u64)>,
    secs: u64,
}

fn counted(ui: &remedy_ui::Ui, drive: Drive) -> remedy_ui::El {
    // A tiny watcher painted with the scene: counts the scene's frames.
    let watch = ui.el("frame-counter").layout(|l| {
        l.width = remedy_ui::layout::Length::Px(1.0);
        l.height = remedy_ui::layout::Length::Px(1.0);
    });
    let watch = match drive {
        Drive::Ambient => watch.paint(|cx| {
            if cx.ambient().is_some() {
                PAINTED.fetch_add(1, Ordering::Relaxed);
            }
        }),
        Drive::EveryFrame => watch.paint(|cx| {
            PAINTED.fetch_add(1, Ordering::Relaxed);
            cx.request_next_frame();
        }),
    };
    scene(ui, drive, 50, true).child(watch)
}

fn step(app: &App, bench: Rc<RefCell<Bench>>) {
    let (secs, phase_done) = {
        let mut b = bench.borrow_mut();
        let phase = b.at.checked_sub(1).and_then(|i| PHASES.get(i)).copied();
        let done = b.started.take().zip(phase).map(|((t0, cpu0, n0), phase)| {
            let wall = t0.elapsed().as_secs_f64();
            let fps = (PAINTED.load(Ordering::Relaxed) - n0) as f64 / wall;
            let core = cpu0.zip(cpu_time()).map(|(a, b)| (b.saturating_sub(a)).as_secs_f64() / wall * 100.0);
            (phase, fps, core)
        });
        (b.secs, done)
    };
    if let Some((p, fps, core)) = phase_done {
        match core {
            Some(c) => println!("{:<12} {fps:>6.1} scene fps  {c:>6.2} % of a core", format!("{p:?}")),
            None => println!("{:<12} {fps:>6.1} scene fps", format!("{p:?}")),
        }
    }
    let at = bench.borrow().at;
    let Some(&phase) = PHASES.get(at) else {
        app.quit();
        return;
    };
    {
        let mut b = bench.borrow_mut();
        let win = b.win.clone();
        match phase {
            Phase::Idle => {}
            Phase::Focused => {
                if let Some(ui) = win.ui() {
                    ui.root().child(counted(&ui, Drive::Ambient));
                }
            }
            Phase::Background => {
                // Another window takes the focus.
                let other = app.open(WindowSpec::new("Other window").size(300.0, 200.0), |_, _| {});
                other.focus();
                b.other = Some(other);
            }
            Phase::Minimized => {
                if let Some(o) = b.other.take() {
                    o.close();
                }
                win.set_minimized(true);
            }
            Phase::Naive => {
                win.set_minimized(false);
                win.focus();
                if let Some(ui) = win.ui() {
                    for c in ui.root().child_elements() {
                        c.remove();
                    }
                    ui.root().child(counted(&ui, Drive::EveryFrame));
                }
            }
        }
        b.at += 1;
    }
    // Let the change settle for a second, then measure.
    let (a2, b2) = (app.clone(), bench.clone());
    app.after(Duration::from_secs(1), move || {
        b2.borrow_mut().started = Some((Instant::now(), cpu_time(), PAINTED.load(Ordering::Relaxed)));
        let (a3, b3) = (a2.clone(), b2.clone());
        a2.after(Duration::from_secs(secs), move || step(&a3, b3));
    });
}

fn main() {
    let secs = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    println!("{} build, {secs} s a phase", if cfg!(debug_assertions) { "debug" } else { "release" });
    let result = remedy_ui::run(AppConfig::new("Ambient bench"), move |app| {
        let win = app.open(WindowSpec::new("Ambient bench").size(WIDTH, HEIGHT), |_, _| {});
        let bench = Rc::new(RefCell::new(Bench { win, other: None, at: 0, started: None, secs }));
        let a = app.clone();
        app.after(Duration::from_secs(1), move || step(&a, bench));
    });
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
