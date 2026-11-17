//! The harness drives a small scene the way the M1 exit criteria describe.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_reactive::{Effect, Signal, waker};
use remedy_ui_test::Harness;
use remedy_ui_tree::{Dirty, Phase};

#[derive(Debug)]
struct Click;

#[test]
fn an_idle_window_does_nothing_frame_after_frame() {
    let h = Harness::new();
    h.element(h.tree().root(), "a");
    h.frame();
    for _ in 0..100 {
        assert!(h.frame().is_idle());
    }
}

#[test]
fn timers_fire_in_time_order_only_when_the_clock_moves() {
    let h = Harness::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    for (ms, name) in [(300, "c"), (100, "a"), (200, "b"), (100, "a2")] {
        let l = log.clone();
        h.after(Duration::from_millis(ms), move || l.borrow_mut().push(name));
    }
    assert_eq!(h.advance(Duration::from_millis(99)), 0);
    assert_eq!(h.advance(Duration::from_millis(150)), 3);
    assert_eq!(*log.borrow(), ["a", "a2", "b"]);
    assert_eq!(h.pending_timers(), 1);
    assert_eq!(h.now(), Duration::from_millis(249));
}

#[test]
fn a_click_updates_one_label_in_one_frame() -> Result<(), remedy_ui_tree::TreeError> {
    let h = Harness::new();
    let root = h.tree().root();
    let button = h.element(root, "button");
    let count = Signal::new(0);
    let mut labels = Vec::new();
    for i in 0..50 {
        let label = h.element(root, "label");
        let tree = h.tree().clone();
        h.tree().in_scope(label, move || {
            Effect::new(move || {
                if i == 0 {
                    count.get();
                }
                tree.mark(label, Dirty::PAINT | Dirty::SEMANTICS);
            });
        })?;
        labels.push(label);
    }
    h.tree().on::<Click>(button, Phase::Bubble, move |_, _| count.update(|c| *c += 1))?;
    h.frame();

    h.send(button, &Click)?;
    let f = h.frame();
    assert_eq!(f.semantics, labels.get(..1).unwrap_or_default());
    assert_eq!(
        f.paint,
        [root, labels.first().copied().unwrap_or(root)],
        "the label and the path to its layer"
    );
    Ok(())
}

#[test]
fn work_posted_from_a_thread_lands_in_the_next_frame() -> Result<(), Box<dyn std::error::Error>> {
    let h = Harness::new();
    let status = Signal::new(String::new());
    let w = waker()?;
    std::thread::spawn(move || {
        for i in 0..10 {
            let _ = w.post(move || status.set(format!("chunk {i}")));
        }
    })
    .join()
    .map_err(|_| "thread panicked")?;
    let f = h.frame();
    assert_eq!(f.posts.ran, 10);
    assert_eq!(status.get(), "chunk 9");
    Ok(())
}

#[test]
fn tab_walks_focus_and_find_matches_paths() -> Result<(), remedy_ui_tree::TreeError> {
    let h = Harness::new();
    let dialog = h.element(h.tree().root(), "dialog");
    let ok = h.element(dialog, "ok");
    let cancel = h.element(dialog, "cancel");
    h.tree().set_focusable(ok, true)?;
    h.tree().set_focusable(cancel, true)?;
    assert_eq!(h.find("dialog/cancel"), Some(cancel));
    assert_eq!(h.tab(), Some(ok));
    assert_eq!(h.tab(), Some(cancel));
    assert_eq!(h.shift_tab(), Some(ok));
    Ok(())
}
