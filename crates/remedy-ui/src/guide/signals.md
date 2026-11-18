# Thinking in signals

A RemedyUI component is a function that runs once. It builds elements and connects them to
state. When the state changes, only the properties that read it update. Nothing re-runs the
component, nothing diffs a tree, and nothing runs while nothing changes.

The state lives in [`Signal`](crate::reactive::Signal)s. Values computed from it are
[`Memo`](crate::reactive::Memo)s. Work that follows it is an
[`Effect`](crate::reactive::Effect), or more often a binding on an element
([`El::bind`](crate::El::bind), and properties given a closure or a signal). This page shows each of them, then
batching, untracked reads and who owns what.

## Signals

A signal holds one value. Reading it inside a memo, effect or binding subscribes that reader;
writing it updates exactly the readers that read it.

```
use remedy_ui::reactive::Signal;

let count = Signal::new(0u32);
count.set(2);
count.update(|c| *c += 1);
assert_eq!(count.get(), 3);

// `with` borrows instead of cloning, for big values.
let names = Signal::new(vec!["Ada".to_string(), "Grace".to_string()]);
assert_eq!(names.with(|n| n.len()), 2);
```

A `Signal` is a `Copy` handle, so closures capture it by value with no `Rc` or `clone`. Its
value lives in the UI thread's reactive runtime, and only that thread can reach it.

- [`set`](crate::reactive::Signal::set) and [`update`](crate::reactive::Signal::update)
  always notify readers.
- [`set_if_changed`](crate::reactive::Signal::set_if_changed) notifies only when the new
  value differs, for values that are often written with what they already hold.
- [`take`](crate::reactive::Signal::take) moves the value out and leaves the default.

## Memos

A memo is a value derived from signals. It runs again only when something it read changed,
and when its new value equals the old one it stops there: nothing downstream runs.

```
use remedy_ui::reactive::{Effect, Memo, Signal};
use std::cell::Cell;
use std::rc::Rc;

let text = Signal::new("hello".to_string());
let words = Memo::new(move || text.with(|t| t.split_whitespace().count()));

let runs = Rc::new(Cell::new(0));
let r = runs.clone();
Effect::new(move || {
    let _ = words.get();
    r.set(r.get() + 1);
});
assert_eq!(runs.get(), 1);

text.set("hello world".into()); // two words: the effect runs
assert_eq!(runs.get(), 2);
text.set("hello there".into()); // still two words: the memo stops the change
assert_eq!(runs.get(), 2);
```

Memos need `PartialEq`, which is how they know a value did not change. They are lazy: a
memo nobody reads does not run.

## Effects

An effect runs once when created and again whenever a signal it read changes. Effects are
for work outside the UI: saving a draft, logging, telling another part of the app. Effects
never see half-updated state: when two signals change together, the effect runs once, after
both.

```
use remedy_ui::reactive::{Effect, Signal};
use std::cell::RefCell;
use std::rc::Rc;

let draft = Signal::new(String::new());
let saved = Rc::new(RefCell::new(Vec::new()));
let s = saved.clone();
Effect::new(move || s.borrow_mut().push(draft.get()));

draft.set("Hi".into());
draft.set("Hi there".into());
assert_eq!(*saved.borrow(), ["", "Hi", "Hi there"]);
```

An effect tracks what it read on its latest run, not what it might read. A branch that was
not taken subscribes nothing, so there are no dependency lists to keep right.

## Binds: signals on elements

On elements, use a binding instead of a free-standing effect. A binding is an effect that
belongs to its element: it runs while the element lives and is disposed with it.

- [`El::text`](crate::El::text): the element's text.
- [`El::style`](crate::El::style): its local style.
- [`El::disabled`](crate::El::disabled): whether it can be used.
- [`El::variant`](crate::El::variant): which variant of its widget it shows.
- [`El::semantics`](crate::El::semantics): what it tells screen readers.
- The widgets' labels: [`widgets::text`](crate::widgets::text),
  [`widgets::button`](crate::widgets::button) and the rest.
- [`El::bind`](crate::El::bind): anything else.

Each of these takes an [`IntoProp`](crate::view::IntoProp): a plain value sets it once and
creates no binding, while a [`Signal`](crate::reactive::Signal), a
[`Memo`](crate::reactive::Memo) or a closure binds it. Making a property follow state never
changes the call's shape.

Each one marks only the work its change needs. A new text relays out that text and repaints
it; a new color only repaints.

```
# use remedy_ui::AppConfig;
# use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

let count = Signal::new(0u32);
let app = move |app: &App| {
    app.open(WindowSpec::new("Counter"), move |ui, _| {
        let add = w::button(ui, "Add one", move || count.update(|c| *c += 1));
        let shown = w::text(ui, move || format!("Count: {}", count.get()));
        let reset = w::button(ui, "Reset", move || count.set(0)).disabled(move || count.get() == 0);
        ui.root().child(add).child(shown).child(reset);
    });
};
# let mut test = TestApp::new(AppConfig::new("Counter"), app);
# let Some(ui) = test.ui() else { unreachable!("the window is open") };
# let shows = |ui: &remedy_ui::Ui, name: &str| ui.reader_view().iter().any(|n| n.name.as_deref() == Some(name));
assert!(shows(&ui, "Count: 0"));

count.set(2); // from a handler, a timer, a background result...
test.frames(1);
assert!(shows(&ui, "Count: 2"));
```

The component ran once. Setting `count` re-ran two small closures: the text and the
disabled state.

When a value is the same for the element's whole life, pass the value, not a closure. A
constant creates no binding at all. When it is exactly a signal's value, pass the signal
itself: `w::text(ui, status)` follows `status`.

## Batching

Writes inside [`batch`](crate::reactive::batch) notify once, when the batch ends. Each event
handler, timer and posted closure already runs in a batch, so most code never calls it.
Call it yourself when you change several signals from somewhere else, such as a test or a
setup function.

```
use remedy_ui::reactive::{Effect, Signal, batch};
use std::cell::RefCell;
use std::rc::Rc;

let first = Signal::new("Ada".to_string());
let last = Signal::new("Lovelace".to_string());
let seen = Rc::new(RefCell::new(Vec::new()));
let s = seen.clone();
Effect::new(move || s.borrow_mut().push(format!("{} {}", first.get(), last.get())));

batch(|| {
    first.set("Grace".into());
    last.set("Hopper".into());
});
// One run for the batch: no effect ever saw "Grace Lovelace".
assert_eq!(*seen.borrow(), ["Ada Lovelace", "Grace Hopper"]);
```

## Reading without subscribing

Sometimes a binding needs a value without following it: the current theme for a one-off
calculation, or a signal it writes to. Wrap the read in
[`untrack`](crate::reactive::untrack), or use
[`get_untracked`](crate::reactive::Signal::get_untracked).

```
use remedy_ui::reactive::{Effect, Signal, untrack};
use std::cell::Cell;
use std::rc::Rc;

let query = Signal::new(String::from("rust"));
let page_size = Signal::new(20usize);
let runs = Rc::new(Cell::new(0));
let r = runs.clone();
Effect::new(move || {
    let q = query.get();
    let size = untrack(|| page_size.get());
    let _ = (q, size);
    r.set(r.get() + 1);
});

page_size.set(50); // read untracked: the search does not run again
assert_eq!(runs.get(), 1);
query.set("signals".into());
assert_eq!(runs.get(), 2);
```

`get_untracked` returns a `Result`, because outside a binding there is no nearby owner to
report a disposed signal to.

## Lifetimes

Every signal, memo and effect has an owner: the [`Scope`](crate::reactive::Scope), memo or
effect that was running when it was made. Disposing an owner disposes everything it owns and
runs its clean-ups ([`on_cleanup`](crate::reactive::on_cleanup)).

- An element's bindings are owned by the element. Removing the element
  ([`El::remove`](crate::El::remove)) disposes them, and runs its
  [`El::on_removed`](crate::El::on_removed) callbacks.
- A list row built by [`widgets::for_each`](crate::widgets::for_each) lives as long as its
  key stays in the list.
- Signals made at the top of a component, outside any scope, live until you dispose them.
  Give long-lived state one home (the app's state struct) instead of making new signals in
  code that runs often.

```
use remedy_ui::reactive::{Effect, Scope, Signal, on_cleanup};
use std::cell::Cell;
use std::rc::Rc;

let online = Signal::new(true);
let cleaned = Rc::new(Cell::new(false));
let c = cleaned.clone();
let (status, scope) = Scope::new(move || {
    let status = Signal::new("Online");
    Effect::new(move || status.set(if online.get() { "Online" } else { "Offline" }));
    on_cleanup(move || c.set(true));
    status
});
online.set(false);
assert_eq!(status.get(), "Offline");

scope.dispose();
assert!(cleaned.get(), "clean-ups run on dispose");
assert!(!status.is_alive(), "what the scope owned is gone");
assert!(status.try_get().is_err(), "reading it says so instead of inventing a value");
```

A clean-up registered inside an effect runs before the effect's next run as well, so an
effect that starts something (a timer, a subscription) can stop it each time.

## Context

Some values belong to a whole subtree rather than to one component: the document a screen
edits, the account that is signed in, a service. Instead of passing them through every
function, provide them on an element with [`El::provide`](crate::El::provide), and read the
nearest one below it with [`El::use_context`](crate::El::use_context), or with
[`reactive::use_context`](crate::reactive::use_context) in code built inside it
([`El::child_with`](crate::El::child_with), its bindings). A value that changes is provided
as a signal.

```
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{El, Ui};

/// Who is signed in, for everything under the account screen.
#[derive(Clone, Copy)]
struct Account {
    name: Signal<String>,
}

/// A greeting that finds the account itself, wherever it is placed below.
fn greeting(ui: &Ui) -> El {
    let account = remedy_ui::reactive::use_context::<Account>();
    w::text(ui, move || match account {
        Some(a) => format!("Signed in as {}", a.name.get()),
        None => "Not signed in".into(),
    })
}

let name = Signal::new("Ada".to_string());
let mut app = TestApp::window(move |ui, _| {
    let u = ui.clone();
    let screen = w::column(ui).provide(Account { name }).child_with(move || greeting(&u));
    ui.root().child(screen);
});
let Some(ui) = app.ui() else { unreachable!("open") };
let shows = |text: &str| ui.reader_view().iter().any(|n| n.name.as_deref() == Some(text));
assert!(shows("Signed in as Ada"));
name.set("Grace".into());
app.frames(1);
assert!(shows("Signed in as Grace"));
```

Context is for what many parts of a subtree need. A value one child needs is an argument.

## State that outlives the app

[`Signal::persist`](crate::reactive::Signal::persist) makes a signal remember its value
between runs: window splits, the last tab, a draft, the choices people made. The saved value
replaces the signal's value when it is made, and each change is saved: debounced, off the UI
thread, and atomically, so a crash leaves the old value or the new one, never half of each.
Values are saved as JSON (the type implements `serde`'s `Serialize` and `Deserialize`).

```no_run
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

remedy_ui::run(AppConfig::new("Notes"), |app: &App| {
    app.open(WindowSpec::new("Notes"), |ui, _| {
        // Restored at start; saved after every change.
        let wrap = Signal::new(true).persist("editor.wrap");
        ui.root().child(w::switch(ui, "Wrap long lines", wrap));
    });
})
.ok();
```

The app's windows are built with its [`PersistStore`](crate::reactive::PersistStore)
provided as context ([`App::persist_store`](crate::App::persist_store): the `state` folder of
the app's data folder, flushed when the app quits), which is where `persist` saves. A value
whose shape changes over releases takes a [`Persist`](crate::reactive::Persist) with a
version and a migration from older versions; a file from a newer version of the app is left
alone. [`Signal::persist_in`](crate::reactive::Signal::persist_in) saves in a store the app
opens itself:

```
use remedy_ui::reactive::{Persist, PersistStore, Scope, Signal, batch};

let dir = std::env::temp_dir().join(format!("remedy-guide-persist-{}", std::process::id()));
let store = PersistStore::open(&dir)?;
let ((), first_run) = Scope::new(|| {
    let split = Signal::new(0.3f32).persist_in(&store, Persist::new("split")).unwrap_or_else(|(s, _)| s);
    split.set(0.6);
});
batch(|| {});
store.flush()?;
first_run.dispose();

// The next start: the value comes back.
let ((), _next_run) = Scope::new(|| {
    let split = Signal::new(0.3f32).persist_in(&store, Persist::new("split")).unwrap_or_else(|(s, _)| s);
    assert_eq!(split.get(), 0.6);
});
let _ = std::fs::remove_dir_all(&dir);
# Ok::<(), std::io::Error>(())
```

## State from other threads

Signals are reached only on their UI thread. Slow work goes elsewhere and comes back:

- [`App::background`](crate::App::background) runs a closure on a worker and hands its
  result to a closure on the UI thread, which can set signals. It returns a
  [`BackgroundJob`](crate::BackgroundJob): [`cancel`](crate::BackgroundJob::cancel) it when the
  result is no longer wanted (the person typed a new search), and its result is dropped.
- [`App::background_cancellable`](crate::App::background_cancellable) also hands the work a
  [`Cancelled`](crate::Cancelled) to check between steps, so long work stops early.
- [`App::completion`](crate::App::completion) gives a handle another thread (an async
  runtime's task, a library callback) completes later.
- [`reactive::waker`](crate::reactive::waker) gives a `Send` handle that posts closures to
  the UI thread, for code below the app layer.

```
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

let digest = Signal::new(String::from("Working…"));
let app = move |app: &App| {
    // A slow computation off the UI thread; the window stays responsive meanwhile.
    // Keep the job to cancel it; once cancelled, `done` never runs.
    let _job = app.background_cancellable(
        |cancelled| {
            let mut h = 0u64;
            for i in 0..5_000_000u64 {
                if i % 100_000 == 0 && cancelled.is_cancelled() {
                    break;
                }
                h = h.rotate_left(5) ^ i;
            }
            h.to_string()
        },
        move |result| digest.set(result),
    );
    app.open(WindowSpec::new("Hasher"), move |ui, _| {
        ui.root().child(w::text(ui, digest));
    });
};
let mut test = TestApp::new(AppConfig::new("Hasher"), app);
for _ in 0..10_000 {
    if digest.get() != "Working…" {
        break;
    }
    std::thread::yield_now();
    test.frames(1);
}
assert_ne!(digest.get(), "Working…", "the result came back to the UI thread");
```

## Collections and nested state

- [`SignalVec`](crate::reactive::SignalVec) is a list that records what changed (push,
  insert, remove, move, set) instead of handing out new copies. Readers take the changes
  with a [`VecReader`](crate::reactive::VecReader).
- [`Store`](crate::reactive::Store) tracks a nested structure field by field, so a write to
  one message's `edited` flag wakes readers of that field and not its siblings.

The list widgets take either a `Signal<Vec<T>>` or a `SignalVec`, whose operations they apply
row by row; see [`crate::guide::lists`].

## Finding out why something updated

Give important signals a name with
[`Signal::named`](crate::reactive::Signal::named). The inspector's Signals page (feature
`devtools`, F12 in a debug build) shows each element's bindings, what they read, and a log
such as "row “42” relaid out because its text binding re-ran: `message.text` changed".
