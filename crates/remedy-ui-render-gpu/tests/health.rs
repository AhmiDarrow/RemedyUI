//! The start-up self-test, remembered verdicts and the present watchdog. One test, in a
//! process of its own: verdicts are the process's, and the steps mark this machine's backend
//! hung and forgive it again in order.

use std::time::Duration;

use remedy_ui_compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, Rect};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_paint::{Brush, DisplayList};
use remedy_ui_render_gpu::health::{self, Verdict};
use remedy_ui_render_gpu::*;

fn plan() -> remedy_ui_compositor::FramePlan {
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(10.0, 10.0, 50.0, 50.0), Brush::Solid(Color::rgb8(200, 40, 40)));
    let mut comp = Compositor::new(64, 64, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, 64.0, 64.0), l)]));
    comp.frame(Duration::ZERO)
}

#[test]
fn backends_are_tested_once_skipped_when_bad_and_demoted_when_they_hang() {
    let dir = std::env::temp_dir().join(format!("remedy-gpu-health-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    health::remember_in(&dir);

    // The ladder tests the backend it takes, once.
    let ctx = GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("a device: {e}"));
    let key = health::key(&ctx);
    assert_eq!(health::verdict(&key), Some(Verdict::Passed), "{}", ctx.describe());
    let tests = health::self_tests_run();
    assert!(tests >= 1);
    let again = GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("a device: {e}"));
    assert_eq!(health::key(&again), key);
    assert_eq!(health::self_tests_run(), tests, "known: not tested again");
    // It is written down for the next start.
    let file = std::fs::read_to_string(dir.join("gpu-health.txt")).unwrap_or_default();
    assert!(file.contains(&format!("{key}\tpassed")), "{file}");
    // What the test costs (once per driver): a renderer's pipelines and two 64 x 64 frames.
    let started = std::time::Instant::now();
    assert!(health::self_test(&ctx).is_ok());
    eprintln!("the self-test took {:?} on {}", started.elapsed(), ctx.describe());
    // Without the self-test, nothing is tested or skipped.
    assert!(GpuContext::new(&LadderOptions::default().self_test(false)).is_ok());

    // A backend that drew wrong or hung is skipped: the ladder moves on (to another adapter
    // or API where the machine has one, else no GPU and the CPU renderer).
    for bad in [Verdict::Failed("garbage".into()), Verdict::Hung] {
        health::record(&key, bad.clone());
        match GpuContext::new(&LadderOptions::default()) {
            Ok(other) => assert_ne!(health::key(&other), key, "{bad:?} was skipped"),
            Err(e) => eprintln!("{bad:?}: nothing else here ({e})"),
        }
        health::forget(&key);
    }
    assert!(GpuContext::new(&LadderOptions::default()).is_ok(), "forgiven, it is used again");

    // The watchdog records a hang while the present is still stuck: a present that never
    // returns still demotes the backend at the next start.
    let dog = health::PresentWatchdog::new(&ctx);
    dog.set_threshold(Duration::from_millis(40));
    let seen_while_stuck = dog.watch(|| {
        std::thread::sleep(Duration::from_millis(400));
        health::verdict(&key)
    });
    assert_eq!(seen_while_stuck, Some(Verdict::Hung), "recorded from the watchdog's thread");
    assert!(dog.hung());
    health::forget(&key);
    // A quick present is no hang.
    let calm = health::PresentWatchdog::new(&ctx);
    calm.watch(|| ());
    std::thread::sleep(Duration::from_millis(20));
    assert!(!calm.hung());

    // A compositor whose present hangs lets its device go, as a loss: its owner recovers down
    // the ladder, past the hung backend.
    let mut gc = GpuCompositor::new(GpuRenderer::new(ctx.clone(), None), LadderOptions::default());
    assert!(gc.present_to_frame(&plan(), 64, 64, Color::WHITE).is_ok());
    assert!(!gc.is_lost(), "a normal present");
    gc.watchdog().set_threshold(Duration::from_nanos(1));
    let _ = gc.present_to_frame(&plan(), 64, 64, Color::WHITE);
    assert!(gc.is_lost(), "a hung present lets the device go");
    assert_eq!(health::verdict(&key), Some(Verdict::Hung));
    match gc.recover_device() {
        Ok(()) => assert_ne!(health::key(gc.renderer().context()), key, "recovered on another backend"),
        Err(e) => eprintln!("nothing else to recover to here ({e}): the runner draws on the CPU"),
    }
    // The next start reads the hang back and skips the backend.
    let file = std::fs::read_to_string(dir.join("gpu-health.txt")).unwrap_or_default();
    assert!(file.contains(&format!("{key}\thung")), "{file}");
    health::forget(&key);
    let _ = std::fs::remove_dir_all(&dir);
}
