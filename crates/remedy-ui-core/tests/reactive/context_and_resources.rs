//! Context, contained effect panics, resources fetched off the UI thread, suspense
//! registration and persisted signals (docs/design/02-state-and-elements.md). Each test runs
//! on its own thread, so each gets a fresh runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

/// Fails the code under test the way a bug would: unwinding with `why`.
fn fail(why: &'static str) -> ! {
    std::panic::resume_unwind(Box::new(why))
}

/// Runs posted work until `done` holds, or fails after a while.
fn settle(done: impl Fn() -> bool) {
    let start = Instant::now();
    while !untrack(&done) {
        run_posted();
        assert!(start.elapsed() < Duration::from_secs(20), "the posted work never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Density(f32);

#[test]
fn context_reaches_descendants_and_the_nearest_wins() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let ((), outer) = Scope::new(|| {
        assert!(provide_context(Density(1.0)));
        let s = seen.clone();
        let ((), _inner) = Scope::new(move || {
            s.borrow_mut().push(use_context::<Density>());
            provide_context(Density(2.0));
            s.borrow_mut().push(use_context::<Density>());
            let s2 = s.clone();
            // An effect made here reads it each time it runs, after building is over.
            Effect::new(move || s2.borrow_mut().push(use_context::<Density>()));
        });
    });
    assert_eq!(*seen.borrow(), [Some(Density(1.0)), Some(Density(2.0)), Some(Density(2.0))]);
    assert_eq!(outer.own_context::<Density>(), Some(Density(1.0)));
    assert_eq!(use_context::<Density>(), None, "nothing is global");
    assert!(!provide_context(Density(3.0)), "outside an owner nothing is provided");
    outer.dispose();
}

#[test]
fn context_holding_a_signal_is_reactive() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    let ((), scope) = Scope::new(move || {
        let scale = Signal::new(1.0f32);
        provide_context(scale);
        Effect::new(move || {
            let sig = use_context::<Signal<f32>>().must("provided above");
            s.borrow_mut().push(sig.get());
        });
        scale.set(1.5);
    });
    assert_eq!(*seen.borrow(), [1.0, 1.5]);
    let owner = Owner::from(scope);
    assert!(owner.use_context::<Signal<f32>>().is_some());
    assert_eq!(owner.run(use_context::<Signal<f32>>).must("alive").map(|s| s.get()), Some(1.5));
}

#[test]
fn an_error_sink_contains_effect_panics_and_the_flush_goes_on() {
    let caught = Rc::new(RefCell::new(Vec::<Caught>::new()));
    let ran_after = Rc::new(RefCell::new(0));
    let trigger = Signal::new(0);
    let c = caught.clone();
    let r = ran_after.clone();
    let ((), _scope) = Scope::new(move || {
        provide_context(ErrorSink::new(move |e| c.borrow_mut().push(e)));
        Effect::new(move || {
            if trigger.get() == 1 {
                fail("disk full");
            }
        });
        Effect::new(move || {
            let _ = trigger.get();
            *r.borrow_mut() += 1;
        });
    });
    trigger.set(1);
    let caught = caught.borrow();
    assert_eq!(caught.len(), 1);
    assert_eq!(caught.first().map(|c| c.message.as_str()), Some("disk full"));
    assert!(
        caught
            .first()
            .and_then(|c| c.created_at)
            .is_some_and(|l| l.file().ends_with("context_and_resources.rs"))
    );
    assert_eq!(*ran_after.borrow(), 2, "the other effect still ran in the same flush");
}

#[test]
fn panic_handlers_take_what_no_sink_took() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    let id = add_panic_handler(move |c| {
        s.borrow_mut().push(c.message.clone());
        true
    });
    let go = Signal::new(false);
    Effect::new(move || {
        if go.get() {
            fail("no sink here");
        }
    });
    go.set(true);
    assert_eq!(*seen.borrow(), ["no sink here"]);
    remove_panic_handler(id);
}

#[test]
fn a_resource_fetches_off_the_thread_and_follows_its_key() {
    let ui_thread = std::thread::current().id();
    let fetched_on = Arc::new(Mutex::new(Vec::new()));
    let f2 = fetched_on.clone();
    let user = Signal::new(1u32);
    let r: Resource<String, String> = Resource::new(
        move || user.get(),
        move |id, _c| {
            f2.lock().map(|mut v| v.push(std::thread::current().id())).ok();
            if id == 3 { Err(format!("user {id} is gone")) } else { Ok(format!("user {id}")) }
        },
    );
    let states = Rc::new(RefCell::new(Vec::new()));
    let s = states.clone();
    Effect::new(move || s.borrow_mut().push((r.loading(), r.get())));
    settle(|| !r.loading());
    assert_eq!(untrack(|| r.get()), Some("user 1".into()));
    assert!(fetched_on.lock().must("lock").iter().all(|t| *t != ui_thread), "fetches run on workers");

    // A new key: loading again, the old value stays until the new one arrives.
    user.set(2);
    assert!(untrack(|| r.loading()));
    assert_eq!(untrack(|| r.get()), Some("user 1".into()));
    settle(|| !r.loading());
    assert_eq!(untrack(|| r.get()), Some("user 2".into()));

    user.set(3);
    settle(|| !r.loading());
    assert_eq!(untrack(|| r.error()), Some(FetchError::Failed("user 3 is gone".into())));
    assert!(untrack(|| r.ready()), "the last value is kept");
    let seen = states.borrow();
    assert_eq!(seen.first(), Some(&(true, None)));
    assert!(seen.contains(&(false, Some("user 2".into()))));
}

#[test]
fn a_changed_key_cancels_the_fetch_under_way_and_drops_its_result() {
    let started = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let (s2, c2) = (started.clone(), cancelled.clone());
    let key = Signal::new(1u32);
    let r: Resource<u32, ()> = Resource::new(
        move || key.get(),
        move |k, cancel| {
            s2.fetch_add(1, Ordering::SeqCst);
            if k == 1 {
                // A slow fetch that watches for cancellation.
                let start = Instant::now();
                while !cancel.is_cancelled() && start.elapsed() < Duration::from_secs(20) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                if cancel.is_cancelled() {
                    c2.fetch_add(1, Ordering::SeqCst);
                }
                return Ok(100);
            }
            Ok(k * 10)
        },
    );
    let start = Instant::now();
    while started.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "the first fetch never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    key.set(2);
    settle(|| !r.loading());
    assert_eq!(untrack(|| r.get()), Some(20));
    let start = Instant::now();
    while cancelled.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "the slow fetch never saw its cancellation");
        std::thread::sleep(Duration::from_millis(1));
    }
    // Its late result (100) never replaces the newer one.
    std::thread::sleep(Duration::from_millis(20));
    run_posted();
    assert_eq!(untrack(|| r.get()), Some(20));
}

#[test]
fn disposing_the_owner_cancels_the_fetch() {
    let saw_cancel = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let (c2, s2) = (saw_cancel.clone(), started.clone());
    let (r, scope) = Scope::new(move || {
        Resource::<u32, ()>::new(
            || (),
            move |(), cancel| {
                s2.fetch_add(1, Ordering::SeqCst);
                let start = Instant::now();
                while !cancel.is_cancelled() && start.elapsed() < Duration::from_secs(20) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                c2.fetch_add(usize::from(cancel.is_cancelled()), Ordering::SeqCst);
                Ok(1)
            },
        )
    });
    let start = Instant::now();
    while started.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "the fetch never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    scope.dispose();
    let start = Instant::now();
    while saw_cancel.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "the fetch was never cancelled");
        std::thread::sleep(Duration::from_millis(1));
    }
    run_posted();
    assert!(untrack(|| r.get()).is_none(), "a disposed resource has no value");
}

#[test]
fn a_panicking_fetch_is_an_error_not_a_crash() {
    let r: Resource<u32, ()> = Resource::new(|| (), |(), _| fail("decoder broke"));
    settle(|| !r.loading());
    assert_eq!(untrack(|| r.error()), Some(FetchError::Panicked("decoder broke".into())));
}

/// An executor that runs each job on a fresh thread and counts them: apps with their own
/// runtime plug in the same way.
struct Counting(Arc<AtomicUsize>);

impl Executor for Counting {
    fn spawn(&self, job: Job) {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(job);
    }
}

/// An executor that drops every job without running it.
struct Dropping;

impl Executor for Dropping {
    fn spawn(&self, job: Job) {
        drop(job);
    }
}

#[test]
fn the_executor_comes_from_context_and_a_dropped_job_is_reported() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let ((r, lost), _scope) = Scope::new(move || {
        provide_context::<Arc<dyn Executor>>(Arc::new(Counting(c)));
        let r: Resource<u32, ()> = Resource::new(|| 5u32, |k, _| Ok(k + 1));
        let lost: Resource<u32, ()> = Resource::with_executor(Arc::new(Dropping), || (), |(), _| Ok(1));
        (r, lost)
    });
    settle(|| !r.loading() && !lost.loading());
    assert_eq!(untrack(|| r.get()), Some(6));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(untrack(|| lost.error()), Some(FetchError::Lost), "nothing stays loading forever");
}

#[test]
fn resources_read_inside_a_suspense_register_with_it() {
    let pending = Rc::new(RefCell::new(Vec::new()));
    let p = pending.clone();
    let ((suspense, r), _scope) = Scope::new(move || {
        let suspense = Suspense::new();
        provide_context(suspense.clone());
        let r: Resource<u32, ()> = Resource::new(|| (), |(), _| Ok(9));
        let s2 = suspense.clone();
        Effect::new(move || p.borrow_mut().push(s2.pending()));
        // A reader inside the boundary registers the resource.
        Effect::new(move || {
            let _ = r.get();
        });
        (suspense, r)
    });
    assert_eq!(suspense.len(), 1);
    settle(|| !r.loading());
    assert!(!untrack(|| suspense.pending()));
    assert_eq!(pending.borrow().last(), Some(&false));
    assert!(pending.borrow().contains(&true));
}

#[test]
fn the_worker_pool_runs_every_job_and_survives_panics() {
    let pool = WorkerPool::new(3);
    let done = Arc::new(AtomicUsize::new(0));
    for i in 0..50 {
        let d = done.clone();
        pool.spawn(Box::new(move || {
            assert!(i % 7 != 0, "every seventh job panics");
            d.fetch_add(1, Ordering::SeqCst);
        }));
    }
    let start = Instant::now();
    while done.load(Ordering::SeqCst) < 42 {
        assert!(start.elapsed() < Duration::from_secs(20), "jobs stopped running");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(pool.size(), 3);
    assert_eq!(pool.queued(), 0);
    assert!(WorkerPool::shared().size() >= 1);
}

// ---------- persisted signals ----------

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
struct Layout {
    split: f32,
    names: Vec<String>,
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("remedy-persist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn a_persisted_signal_is_restored_and_saved_after_changes() {
    let dir = temp_dir("restore");
    let store = PersistStore::with_debounce(&dir, Duration::from_millis(50)).must("store");
    let ((), scope) = Scope::new(|| {
        provide_context(store.clone());
        let s = Signal::new(Layout { split: 0.5, names: vec![] }).persist("main window/layout");
        assert_eq!(s.get().split, 0.5, "nothing saved yet: the default stays");
        for i in 0..20u8 {
            s.update(|l| l.names.push(format!("n{i}")));
        }
        s.update(|l| l.split = 0.25);
    });
    store.flush().must("flush");
    let path = store.path_of("main window/layout");
    assert!(path.file_name().is_some_and(|n| n.to_string_lossy() == "main~20window~2flayout.json"));
    let text = std::fs::read_to_string(&path).must("written");
    assert!(text.contains("\"version\": 1"), "{text}");
    assert!(!dir.join("main~20window~2flayout.json.tmp").exists(), "the temporary file was renamed");
    scope.dispose();

    let ((), _again) = Scope::new(|| {
        provide_context(store.clone());
        let s = Signal::new(Layout { split: 0.5, names: vec![] }).persist("main window/layout");
        assert_eq!(s.get().split, 0.25);
        assert_eq!(s.get().names.len(), 20);
    });
    assert!(store.take_errors().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn many_quick_changes_make_one_write() {
    let dir = temp_dir("debounce");
    let store = PersistStore::with_debounce(&dir, Duration::from_millis(400)).must("store");
    let s = Signal::new(0u32).persist_in(&store, Persist::new("count")).must("persisted");
    for i in 1..=100 {
        s.set(i);
    }
    let path = store.path_of("count");
    assert!(!path.exists(), "nothing is written before the debounce ends");
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(20), "the debounced write never came");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(20));
    let text = std::fs::read_to_string(&path).must("written");
    assert!(text.contains("100"), "the last value is what was written: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn older_versions_migrate_and_newer_ones_are_left_alone() {
    let dir = temp_dir("versions");
    let store = PersistStore::open(&dir).must("store");
    // Version 1 kept the split as a percentage.
    std::fs::write(store.path_of("layout"), r#"{"version": 1, "value": {"percent": 40}}"#).must("write v1");
    let migrate = |from: u32, v: JsonValue| -> Result<JsonValue, MigrationError> {
        if from != 1 {
            return Err(format!("unknown version {from}").into());
        }
        let p = v.get("percent").and_then(JsonValue::as_f64).ok_or("no percent")?;
        Ok(serde_json::json!({ "split": p / 100.0, "names": [] }))
    };
    let s = Signal::new(Layout { split: 0.5, names: vec![] })
        .persist_in(&store, Persist::new("layout").version(2).migrate(migrate))
        .must("migrated");
    assert!((s.get().split - 0.4).abs() < 1e-6);

    std::fs::write(store.path_of("future"), r#"{"version": 9, "value": 1}"#).must("write v9");
    let Err((f, e)) = Signal::new(0u32).persist_in(&store, Persist::new("future")) else {
        unreachable!("a newer file is refused")
    };
    assert_eq!(e, PersistError::Newer { found: 9, current: 1 });
    f.set(5);
    store.flush().must("flush");
    let text = std::fs::read_to_string(store.path_of("future")).must("still there");
    assert!(text.contains("9"), "a newer app's file is never overwritten");

    std::fs::write(store.path_of("broken"), b"{ not json").must("write broken");
    let Err((b, e)) = Signal::new(3u32).persist_in(&store, Persist::new("broken")) else {
        unreachable!("a damaged file is reported")
    };
    assert!(matches!(e, PersistError::Format(_)));
    assert_eq!(b.get(), 3, "a damaged file leaves the default");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_store_persist_keeps_the_value() {
    let s = Signal::new(4u8).persist("nowhere");
    s.set(6);
    assert_eq!(s.get(), 6);
}
