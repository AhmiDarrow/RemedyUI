//! Behavior the design promises for the reactive graph (docs/design/02-state-and-elements.md).
//! Each test runs on its own thread, so each gets a fresh runtime.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_core::reactive::*;

fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let c = Rc::new(Cell::new(0));
    (c.clone(), c)
}

#[test]
fn diamond_is_glitch_free() {
    // a → b, a → c, (b, c) → d: d must never see a new b with an old c.
    let a = Signal::new(1);
    let b = Memo::new(move || a.get() * 2);
    let c = Memo::new(move || a.get() + 10);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    Effect::new(move || log.borrow_mut().push((b.get(), c.get())));
    a.set(2);
    a.set(3);
    assert_eq!(*seen.borrow(), [(2, 11), (4, 12), (6, 13)]);
}

#[test]
fn unchanged_memo_stops_the_wave() {
    let n = Signal::new(3);
    let (runs, r) = counter();
    let parity = Memo::new(move || {
        r.set(r.get() + 1);
        n.get() % 2
    });
    let (effects, e) = counter();
    Effect::new(move || {
        parity.get();
        e.set(e.get() + 1);
    });
    assert_eq!((runs.get(), effects.get()), (1, 1));
    n.set(5); // parity still 1: memo recomputes, effect does not run
    assert_eq!((runs.get(), effects.get()), (2, 1));
    n.set(6); // parity changes
    assert_eq!((runs.get(), effects.get()), (3, 2));
}

#[test]
fn memos_are_lazy() {
    let s = Signal::new(1);
    let (runs, r) = counter();
    let m = Memo::new(move || {
        r.set(r.get() + 1);
        s.get()
    });
    s.set(2);
    s.set(3);
    assert_eq!(runs.get(), 0, "nothing read it yet");
    assert_eq!(m.get(), 3);
    assert_eq!(m.get(), 3);
    assert_eq!(runs.get(), 1);
}

#[test]
fn batch_runs_effects_once_at_the_end() {
    let a = Signal::new(0);
    let b = Signal::new(0);
    let (effects, e) = counter();
    Effect::new(move || {
        a.get();
        b.get();
        e.set(e.get() + 1);
    });
    batch(|| {
        for i in 1..=100 {
            a.set(i);
            b.set(i);
        }
        batch(|| a.set(1000)); // nested batches end with the outermost
    });
    assert_eq!(effects.get(), 2);
}

#[test]
fn dependencies_follow_the_branch_taken() {
    let use_a = Signal::new(true);
    let a = Signal::new("a");
    let b = Signal::new("b");
    let (effects, e) = counter();
    let out = Rc::new(RefCell::new(""));
    let o = out.clone();
    Effect::new(move || {
        e.set(e.get() + 1);
        *o.borrow_mut() = if use_a.get() { a.get() } else { b.get() };
    });
    b.set("b2");
    assert_eq!(effects.get(), 1, "b was not read, so changing it does nothing");
    use_a.set(false);
    assert_eq!(*out.borrow(), "b2");
    a.set("a2");
    assert_eq!(effects.get(), 2, "a is no longer read");
}

#[test]
fn untrack_does_not_subscribe() {
    let s = Signal::new(1);
    let (effects, e) = counter();
    Effect::new(move || {
        untrack(|| s.get());
        e.set(e.get() + 1);
    });
    s.set(2);
    assert_eq!(effects.get(), 1);
}

#[test]
fn disposing_a_scope_stops_everything_it_owns() {
    let s = Signal::new(0);
    let order = Rc::new(RefCell::new(Vec::new()));
    let (effects, e) = counter();
    let o = order.clone();
    let ((), scope) = Scope::new(move || {
        let o1 = o.clone();
        on_cleanup(move || o1.borrow_mut().push("first"));
        let o2 = o.clone();
        on_cleanup(move || o2.borrow_mut().push("second"));
        Effect::new(move || {
            s.get();
            e.set(e.get() + 1);
        });
    });
    s.set(1);
    assert_eq!(effects.get(), 2);
    scope.dispose();
    assert_eq!(*order.borrow(), ["second", "first"], "clean-ups run in reverse order");
    s.set(2);
    assert_eq!(effects.get(), 2, "a disposed effect never runs");
    assert!(!scope.is_alive());
}

#[test]
fn an_effect_disposes_what_it_made_before_running_again() {
    let outer = Signal::new(0);
    let inner_runs = Rc::new(Cell::new(0));
    let cleanups = Rc::new(Cell::new(0));
    let probe = Signal::new(0);
    let (ir, cl) = (inner_runs.clone(), cleanups.clone());
    Effect::new(move || {
        outer.get();
        let ir = ir.clone();
        Effect::new(move || {
            probe.get();
            ir.set(ir.get() + 1);
        });
        let cl = cl.clone();
        on_cleanup(move || cl.set(cl.get() + 1));
    });
    assert_eq!(inner_runs.get(), 1);
    outer.set(1); // the old inner effect is disposed, a new one made
    assert_eq!((inner_runs.get(), cleanups.get()), (2, 1));
    probe.set(1); // only the live inner effect runs
    assert_eq!(inner_runs.get(), 3);
}

#[test]
fn disposed_signals_report_it_and_get_panics_with_the_site() {
    let s = Signal::new(5);
    s.dispose();
    assert_eq!(s.try_get(), Err(ReactiveError::Disposed));
    assert!(!s.is_alive());
    let err = std::panic::catch_unwind(|| s.get()).err();
    let msg = err.and_then(|e| e.downcast_ref::<String>().cloned()).unwrap_or_default();
    assert!(msg.contains("disposed"), "{msg}");
}

#[test]
fn handles_do_not_reach_across_threads() {
    let s = Signal::new(1);
    let r = std::thread::spawn(move || s.try_get()).join().ok();
    assert_eq!(r, Some(Err(ReactiveError::WrongThread)));
    assert_eq!(s.get(), 1);
}

#[test]
fn a_memo_reading_itself_is_a_cycle_not_a_crash() {
    let slot: Signal<Option<Memo<i32>>> = Signal::new(None);
    let m = Memo::new(move || slot.get().and_then(|m| m.try_get().ok()).unwrap_or(0) + 1);
    slot.set(Some(m));
    // The inner read fails with Cycle, so the memo sees 0 and yields 1.
    assert_eq!(m.get(), 1);
}

#[test]
fn an_effect_may_write_what_it_reads_until_it_settles() {
    let n = Signal::new(0);
    let (effects, e) = counter();
    Effect::new(move || {
        e.set(e.get() + 1);
        let v = n.get();
        if v < 3 {
            n.set(v + 1);
        }
    });
    assert_eq!(n.get(), 3);
    assert_eq!(effects.get(), 4);
}

#[test]
fn idle_means_no_work() {
    let s = Signal::new(0);
    Effect::new(move || {
        s.get();
    });
    let before = stats();
    for _ in 0..1000 {
        let _ = s.get_untracked();
    }
    let after = stats();
    assert_eq!(before.effect_runs, after.effect_runs);
    assert_eq!(after.pending_effects, 0);
}

#[test]
fn set_if_changed_skips_equal_values() {
    let s = Signal::new(7);
    let (effects, e) = counter();
    Effect::new(move || {
        s.get();
        e.set(e.get() + 1);
    });
    s.set_if_changed(7);
    assert_eq!(effects.get(), 1);
    s.set_if_changed(8);
    assert_eq!(effects.get(), 2);
}

#[test]
fn take_leaves_the_default() {
    let s = Signal::new(String::from("draft"));
    assert_eq!(s.take(), "draft");
    assert_eq!(s.get(), "");
}

#[test]
fn writing_inside_with_reports_borrowed() {
    let s = Signal::new(1);
    let r = s.with(|_| s.try_set(2));
    assert_eq!(r, Err(ReactiveError::Borrowed));
    assert_eq!(s.get(), 1);
}

#[test]
fn a_runaway_effect_cannot_hang_the_app() {
    let n = Signal::new(0u64);
    Effect::new(move || {
        let v = n.get();
        n.set(v + 1); // never settles
    });
    assert_eq!(stats().runaway_effects, 1);
    assert_eq!(stats().pending_effects, 0);
    assert!(n.get() > 1000, "it ran until the bound, then stopped");
}

#[test]
fn a_runaway_effect_runs_again_on_the_next_change() {
    let stop = Signal::new(false);
    let n = Signal::new(0u64);
    let (effects, e) = counter();
    Effect::new(move || {
        e.set(e.get() + 1);
        if !stop.get() {
            n.set(n.get() + 1); // never settles while `stop` is off
        }
    });
    assert_eq!(stats().runaway_effects, 1);
    let before = effects.get();
    stop.set(true);
    assert_eq!(effects.get(), before + 1, "the change reached the effect that was stopped");
    assert_eq!(stats().pending_effects, 0);
}

#[test]
fn a_memo_whose_computation_panicked_computes_again() {
    let fail = Rc::new(Cell::new(true));
    let f = fail.clone();
    let m = Memo::new(move || {
        assert!(!f.get(), "the computation fails");
        7
    });
    assert!(std::panic::catch_unwind(|| m.get()).is_err());
    fail.set(false);
    assert_eq!(m.try_get(), Ok(7));
}

#[test]
fn an_effect_disposed_while_it_runs_leaves_no_subscription_behind() {
    let s = Signal::new(0);
    let slot: Rc<Cell<Option<Effect>>> = Rc::default();
    let me = slot.clone();
    let effect = Effect::new(move || {
        if let Some(e) = me.get() {
            e.dispose();
        }
        s.get();
    });
    slot.set(Some(effect));
    let live = |k| debug::node(k).map(|n| n.subscribers.len());
    assert_eq!(live(s.key()), Some(1));
    s.set(1); // the effect disposes itself, then reads
    assert_eq!(live(s.key()), Some(0));
}

#[test]
fn reactions_report_changes_to_what_they_last_tracked() {
    use remedy_ui_core::reactive::{Memo, Reaction};
    let a = Signal::new(1);
    let b = Signal::new(10);
    let told = Rc::new(Cell::new(0));
    let t2 = told.clone();
    let r = Reaction::new(move || t2.set(t2.get() + 1));
    assert_eq!(told.get(), 0, "nothing runs on creation");
    assert_eq!(r.track(|| a.get() * 2), Ok(2));
    b.set(11);
    assert_eq!(told.get(), 0, "b was not read");
    a.set(2);
    assert_eq!(told.get(), 1);
    // Re-tracking replaces the sources.
    assert_eq!(r.track(|| b.get()), Ok(11));
    a.set(3);
    assert_eq!(told.get(), 1, "a is no longer tracked");
    b.set(12);
    assert_eq!(told.get(), 2);
    // Through a memo, only a real change of its value tells.
    let parity = Memo::new(move || a.get() % 2);
    let _ = r.track(|| parity.get());
    a.set(5);
    assert_eq!(told.get(), 2, "parity stayed odd");
    a.set(6);
    assert_eq!(told.get(), 3);
    r.dispose();
    a.set(7);
    assert_eq!(told.get(), 3, "disposed");
    assert_eq!(r.track(|| 5), Ok(5), "a disposed reaction still runs its closure");
}

#[test]
fn a_disposed_reaction_still_runs_what_it_tracks_and_may_read_signals() {
    // A paint closure tracked by a reaction whose element went away in the same frame.
    let lit = Signal::new(true);
    let r = Reaction::new(|| {});
    let gone = r;
    r.dispose();
    let read = gone.track(|| lit.get_untracked().ok());
    assert!(matches!(read, Ok(Some(true)) | Err(_)), "{read:?}");
    // And signals still work afterwards.
    lit.set(false);
    assert_eq!(lit.get_untracked().ok(), Some(false));
}

#[test]
fn nested_scopes_dispose_what_they_own_first_the_last_made_first() {
    let order = Rc::new(RefCell::new(Vec::new()));
    let note = |o: &Rc<RefCell<Vec<&'static str>>>, name: &'static str| {
        let o = o.clone();
        on_cleanup(move || o.borrow_mut().push(name));
    };
    let o = order.clone();
    let ((), parent) = Scope::new(move || {
        note(&o, "parent");
        for (child, grandchild) in [("first", "first's child"), ("second", "second's child")] {
            let o = o.clone();
            let _ = Scope::new(move || {
                note(&o, child);
                let o = o.clone();
                let _ = Scope::new(move || note(&o, grandchild));
            });
        }
    });
    parent.dispose();
    assert_eq!(*order.borrow(), ["second's child", "second", "first's child", "first", "parent"]);
}

#[test]
fn ownership_and_memo_chains_of_any_depth_on_a_threads_stack() {
    // Tens of thousands deep, on a test thread's stack (2 MB): scopes owned by scopes are
    // disposed, and a chain of memos each reading the last is read and updated.
    const N: u64 = 20_000;
    let cleaned = Rc::new(Cell::new(0u64));
    let (_, outer) = Scope::new(|| ());
    let mut owner = outer;
    for _ in 0..N {
        let c = cleaned.clone();
        let next = owner.run(move || Scope::new(move || on_cleanup(move || c.set(c.get() + 1))).1);
        let Ok(next) = next else { unreachable!("the owner is alive") };
        owner = next;
    }
    outer.dispose();
    assert_eq!(cleaned.get(), N, "every level cleaned up");

    let s = Signal::new(1u64);
    let mut last = Memo::new(move || s.get());
    for _ in 0..N {
        let below = last;
        last = Memo::new(move || below.get() + 1);
    }
    assert_eq!(last.get(), 1 + N);
    s.set(2);
    assert_eq!(last.get(), 2 + N, "a write reaches the end of the chain");
}
