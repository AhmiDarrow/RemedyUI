//! The signal graph devtools read: edges, names, and why each run happened. Each test runs
//! on its own thread, so each gets a fresh runtime.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::reactive::debug::{self, NodeKind, Update};
use remedy_ui_core::reactive::*;

/// Unwraps what a test needs, failing it with `why`.
trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

fn recorder() -> (Rc<RefCell<Vec<Update>>>, debug::Tracing) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let guard = debug::trace_with(move |u| log.borrow_mut().push(u.clone()));
    (seen, guard)
}

#[test]
fn the_graph_has_every_edge_and_name() {
    let edited = Signal::new(false).named("message.edited");
    let text = Signal::new(String::from("hi")).named("message.text");
    // The note shows only for edited messages: until then the memo reads only the flag.
    let shown =
        Memo::new(move || if edited.get() { format!("edited: {}", text.get()) } else { String::new() })
            .named("message.note");
    let row = Effect::new(move || {
        let _ = shown.get();
    })
    .named("row 42");

    let s = debug::node(edited.key()).must("the signal is in the graph");
    assert_eq!(s.kind, NodeKind::Signal);
    assert_eq!(s.name.as_deref(), Some("message.edited"));
    assert_eq!(s.subscribers, vec![shown.key()]);
    assert_eq!(s.label(), "message.edited");
    assert!(s.created_at.file().ends_with("debug.rs"), "where it was created");

    let m = debug::node(shown.key()).must("the memo");
    assert_eq!(m.kind, NodeKind::Memo);
    assert_eq!(m.sources, vec![edited.key()], "an unedited message reads only the flag");
    let m_after = {
        edited.set(true);
        debug::node(shown.key()).must("the memo")
    };
    assert_eq!(m_after.sources, vec![edited.key(), text.key()]);
    assert_eq!(m_after.subscribers, vec![row.key()]);

    let e = debug::node(row.key()).must("the effect");
    assert_eq!(e.kind, NodeKind::Effect);
    assert_eq!(e.sources, vec![shown.key()]);
    assert_eq!(e.name.as_deref(), Some("row 42"));

    // Everything downstream, nearest first, each once.
    assert_eq!(debug::dependents(text.key()), vec![shown.key(), row.key()]);
    assert_eq!(debug::dependents(edited.key()), vec![shown.key(), row.key()]);
    assert!(debug::dependents(row.key()).is_empty());

    let all = debug::graph();
    for k in [edited.key(), text.key(), shown.key(), row.key()] {
        assert!(all.iter().any(|n| n.key == k), "{k} is in the whole graph");
    }
    // A disposed node leaves the graph and its edges.
    row.dispose();
    assert!(debug::node(row.key()).is_none());
    assert!(debug::node(shown.key()).must("the memo").subscribers.is_empty());
    assert!(!debug::set_name(row.key(), "gone"));
}

#[test]
fn a_run_names_the_write_that_caused_it() {
    let (seen, _tracing) = recorder();
    let count = Signal::new(0).named("count");
    let label = Effect::new(move || {
        let _ = count.get();
    })
    .named("label");
    assert_eq!(seen.borrow().len(), 1, "the first run");
    assert!(seen.borrow().first().must("first").causes.is_empty());
    assert_eq!(seen.borrow().first().must("first").because(), "first run");

    count.set(1);
    let line = line!() - 1;
    let u = seen.borrow().last().cloned().must("the second run");
    assert_eq!(u.node, label.key());
    assert_eq!(u.kind, NodeKind::Effect);
    assert_eq!(u.name.as_deref(), Some("label"));
    let c = u.causes.first().must("one cause");
    assert_eq!(u.causes.len(), 1);
    assert_eq!(c.signal, count.key());
    assert_eq!(c.signal_name.as_deref(), Some("count"));
    let at = c.written_at.must("where the write happened");
    assert!(at.file().ends_with("debug.rs"));
    assert_eq!(at.line(), line, "the write's own line, not the library's");
    assert!(c.via.is_empty(), "read directly");
    assert!(u.because().contains("`count` changed"), "{}", u.because());
}

#[test]
fn causes_keep_the_path_through_memos() {
    let (seen, _tracing) = recorder();
    let name = Signal::new(String::from("ada")).named("user.name");
    let upper = Memo::new(move || name.get().to_uppercase()).named("user.upper");
    let greeting = Memo::new(move || format!("Hi {}", upper.get())).named("greeting");
    let view = Effect::new(move || {
        let _ = greeting.get();
    });
    seen.borrow_mut().clear();
    name.set("grace".into());
    let runs = seen.borrow();
    let effect_run = runs.iter().find(|u| u.node == view.key()).must("the effect re-ran");
    let c = effect_run.causes.first().must("a cause");
    assert_eq!(c.signal, name.key());
    assert_eq!(c.via, vec![upper.key(), greeting.key()], "nearest the signal first");
    // The memos' own recomputations are recorded with their causes too.
    let memo_run = runs.iter().find(|u| u.node == upper.key()).must("the memo recomputed");
    assert_eq!(memo_run.kind, NodeKind::Memo);
    assert_eq!(memo_run.causes.first().must("a cause").signal, name.key());
}

#[test]
fn a_batch_reports_every_write_once() {
    let (seen, _tracing) = recorder();
    let a = Signal::new(1).named("a");
    let b = Signal::new(2).named("b");
    let sum = Effect::new(move || {
        let _ = a.get() + b.get();
    });
    seen.borrow_mut().clear();
    batch(|| {
        a.set(10);
        b.set(20);
        a.set(11);
    });
    let runs = seen.borrow();
    assert_eq!(runs.len(), 1, "one run for the batch");
    let u = runs.first().must("the run");
    assert_eq!(u.node, sum.key());
    let signals: Vec<_> = u.causes.iter().map(|c| c.signal).collect();
    assert_eq!(signals, vec![a.key(), b.key()]);
}

#[test]
fn a_memo_that_did_not_change_leaves_no_stale_cause() {
    let (seen, _tracing) = recorder();
    let n = Signal::new(2).named("n");
    let other = Signal::new(0).named("other");
    let parity = Memo::new(move || n.get() % 2).named("parity");
    let e = Effect::new(move || {
        let _ = parity.get();
        let _ = other.get();
    });
    seen.borrow_mut().clear();
    n.set(4); // parity stays 0: the effect does not run
    assert!(seen.borrow().iter().all(|u| u.node != e.key()));
    other.set(1);
    let u = seen.borrow().iter().rev().find(|u| u.node == e.key()).cloned().must("ran for `other`");
    let signals: Vec<_> = u.causes.iter().map(|c| c.signal).collect();
    assert_eq!(signals, vec![other.key()], "the unchanged memo's mark is not reported later");
}

#[test]
fn reactions_report_why_they_were_told() {
    let (seen, _tracing) = recorder();
    let color = Signal::new(1).named("color");
    let told = Rc::new(RefCell::new(0));
    let t2 = told.clone();
    let r = Reaction::new(move || *t2.borrow_mut() += 1).named("paint");
    let _ = r.track(|| color.get());
    color.set(2);
    assert_eq!(*told.borrow(), 1);
    let u = seen.borrow().iter().find(|u| u.node == r.key()).cloned().must("the reaction was recorded");
    assert_eq!(u.kind, NodeKind::Reaction);
    assert_eq!(u.causes.first().must("a cause").signal, color.key());
}

#[test]
fn the_current_run_is_readable_inside_it_and_nothing_records_without_tracing() {
    let x = Signal::new(0).named("x");
    let inside = Rc::new(RefCell::new(Vec::new()));
    let i2 = inside.clone();
    let _e = Effect::new(move || {
        let _ = x.get();
        i2.borrow_mut().push(debug::current());
    });
    x.set(1);
    assert!(!debug::is_tracing());
    assert!(inside.borrow().iter().all(Option::is_none), "nothing is recorded without a guard");

    let guard = debug::trace();
    assert!(debug::is_tracing());
    x.set(2);
    let last = inside.borrow().last().cloned().flatten().must("recorded while tracing");
    assert_eq!(last.causes.first().must("a cause").signal, x.key());
    assert!(debug::current().is_none(), "nothing runs between runs");
    drop(guard);
    assert!(!debug::is_tracing());
    x.set(3);
    assert!(inside.borrow().last().must("ran").is_none());
}

#[test]
fn a_hook_reading_signals_subscribes_nothing() {
    let watched = Signal::new(0).named("watched");
    let unrelated = Signal::new(0).named("unrelated");
    let _tracing = debug::trace_with(move |_| {
        let _ = unrelated.get();
    });
    let e = Effect::new(move || {
        let _ = watched.get();
    });
    watched.set(1);
    assert_eq!(debug::node(e.key()).must("effect").sources, vec![watched.key()]);
    assert!(debug::node(unrelated.key()).must("signal").subscribers.is_empty());
}

#[test]
fn nested_guards_keep_tracing_until_the_last_drops() {
    let a = debug::trace();
    let b = debug::trace();
    drop(a);
    assert!(debug::is_tracing());
    drop(b);
    assert!(!debug::is_tracing());
}
