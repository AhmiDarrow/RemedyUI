//! The waker and reactive vectors.
#![allow(clippy::panic, reason = "one test posts a closure that panics on purpose")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use remedy_ui_reactive::*;

#[test]
fn posts_from_other_threads_run_in_one_batch() -> Result<(), Box<dyn std::error::Error>> {
    let messages = SignalVec::new(Vec::<String>::new());
    let reader = messages.reader()?;
    let runs = Rc::new(Cell::new(0));
    let got = Rc::new(RefCell::new(Vec::new()));
    let (r, g) = (runs.clone(), got.clone());
    Effect::new(move || {
        r.set(r.get() + 1);
        g.borrow_mut().extend(reader.take_diffs().unwrap_or_default());
    });
    assert_eq!(runs.get(), 1);

    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    set_wake_handler(move || {
        w.fetch_add(1, Ordering::SeqCst);
    })?;
    let waker = waker()?;
    std::thread::spawn(move || {
        for i in 0..100 {
            let _ = waker.post(move || {
                let _ = messages.push(format!("message {i}"));
            });
        }
    })
    .join()
    .map_err(|_| "poster thread panicked")?;

    assert_eq!(wakes.load(Ordering::SeqCst), 1, "woken once, when the queue went non-empty");
    assert!(has_posted());
    let report = run_posted();
    assert_eq!(report, {
        let mut v_ = PostReport::default();
        v_.ran = 100;
        v_.panicked = 0;
        v_
    });
    assert_eq!(runs.get(), 2, "100 posts, one effect run");
    let got = got.borrow();
    assert_eq!(got.first(), Some(&VecDiff::Replace(vec![])));
    assert_eq!(got.len(), 101);
    assert_eq!(got.last(), Some(&VecDiff::Push("message 99".into())));
    Ok(())
}

#[test]
fn a_panicking_post_is_contained() -> Result<(), ReactiveError> {
    let s = Signal::new(0);
    let waker = waker()?;
    let _ = waker.post(|| std::panic::panic_any("boom"));
    let _ = waker.post(move || s.set(1));
    let report = run_posted();
    assert_eq!(report, {
        let mut v_ = PostReport::default();
        v_.ran = 1;
        v_.panicked = 1;
        v_
    });
    assert_eq!(s.get(), 1);
    Ok(())
}

#[test]
fn a_waker_reports_when_its_thread_has_ended() -> Result<(), String> {
    let waker =
        std::thread::spawn(|| waker().map_err(|e| e.to_string())).join().map_err(|_| "panicked")??;
    assert!(waker.is_closed());
    assert_eq!(waker.post(|| {}), Err(Closed));
    Ok(())
}

#[test]
fn readers_get_replace_then_only_new_operations() -> Result<(), Box<dyn std::error::Error>> {
    let v = SignalVec::new(vec![1, 2, 3]);
    let a = v.reader()?;
    assert_eq!(a.take_diffs()?, [VecDiff::Replace(vec![1, 2, 3])]);
    v.push(4)?;
    v.insert(0, 0)??;
    assert_eq!(v.remove(2)??, 2);
    v.move_item(0, 3)??;
    v.set(1, 10)??;
    assert_eq!(
        a.take_diffs()?,
        [
            VecDiff::Push(4),
            VecDiff::Insert { index: 0, value: 0 },
            VecDiff::Remove { index: 2 },
            VecDiff::Move { from: 0, to: 3 },
            VecDiff::Set { index: 1, value: 10 },
        ]
    );
    assert_eq!(a.take_diffs()?, []);
    v.with(|items| assert_eq!(items, [1, 10, 4, 0]))?;

    // A reader that joins later starts from the current contents.
    let b = v.reader()?;
    assert_eq!(b.take_diffs()?, [VecDiff::Replace(vec![1, 10, 4, 0])]);
    Ok(())
}

#[test]
fn applying_diffs_reproduces_the_vector() -> Result<(), Box<dyn std::error::Error>> {
    fn apply(model: &mut Vec<u32>, d: VecDiff<u32>) {
        match d {
            VecDiff::Replace(items) => *model = items,
            VecDiff::Insert { index, value } => model.insert(index, value),
            VecDiff::Remove { index } => {
                model.remove(index);
            }
            VecDiff::Set { index, value } => {
                if let Some(slot) = model.get_mut(index) {
                    *slot = value;
                }
            }
            VecDiff::Move { from, to } => {
                let x = model.remove(from);
                model.insert(to, x);
            }
            VecDiff::Push(v) => model.push(v),
            VecDiff::Pop => {
                model.pop();
            }
            VecDiff::Clear => model.clear(),
            other => unreachable!("a diff this test does not make: {other:?}"),
        }
    }
    let v = SignalVec::new(Vec::new());
    let r = v.reader()?;
    let mut mirror = Vec::new();
    let mut seed: u32 = 12345;
    for step in 0..2000u32 {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let len = v.len()?;
        match seed % 7 {
            0 | 1 => v.push(step)?,
            2 if len > 0 => drop(v.remove((seed as usize / 7) % len)?),
            3 => drop(v.insert((seed as usize / 7) % (len + 1), step)?),
            4 if len > 1 => drop(v.move_item((seed as usize / 7) % len, (seed as usize / 11) % len)?),
            5 if len > 0 => drop(v.set((seed as usize / 7) % len, step)?),
            6 if step % 50 == 0 => v.clear()?,
            _ => drop(v.pop()?),
        }
        if step % 13 == 0 {
            for d in r.take_diffs()? {
                apply(&mut mirror, d);
            }
            v.with(|items| assert_eq!(items, mirror.as_slice()))?;
        }
    }
    Ok(())
}

#[test]
fn out_of_range_is_an_error_not_a_panic() -> Result<(), ReactiveError> {
    let v = SignalVec::new(vec!['a']);
    assert_eq!(v.remove(5)?, Err(OutOfRange { index: 5, len: 1 }));
    assert_eq!(v.insert(3, 'b')?, Err(OutOfRange { index: 3, len: 1 }));
    assert_eq!(v.set(1, 'c')?, Err(OutOfRange { index: 1, len: 1 }));
    Ok(())
}

#[test]
fn dropped_readers_stop_holding_operations() -> Result<(), ReactiveError> {
    let v = SignalVec::new(Vec::new());
    let r = v.reader()?;
    r.take_diffs()?;
    drop(r);
    for i in 0..10_000 {
        v.push(i)?;
    }
    // With no reader, nothing is logged; a new reader starts from the contents.
    let r2 = v.reader()?;
    let d = r2.take_diffs()?;
    assert_eq!(d.len(), 1);
    assert!(matches!(d.first(), Some(VecDiff::Replace(items)) if items.len() == 10_000));
    Ok(())
}
