# Showing, failing and loading

Three building blocks change what a part of a window shows, without a rebuild of the rest:

- [`show`](crate::view::show): content that is there while a condition holds;
- [`catch_errors`](crate::view::catch_errors): an error boundary, which shows a fallback when
  the content inside it fails;
- [`suspense`](crate::view::suspense): a loading boundary, which shows a placeholder while
  data read inside it is still arriving, fetched by a [`Resource`](crate::reactive::Resource).

Each builds its content in a reactive scope of its own. When the content goes (the
condition turns false, a retry, a new key), everything it made goes with it: its elements,
signals, effects and the fetches its resources started, which are cancelled.

## Showing content while a condition holds

`show(ui, when, build)` builds what `build` makes while `when` is true. `when` is a signal, a
closure or a constant. The content is built each time it appears and disposed when it goes;
[`Show::fallback`](crate::view::Show::fallback) builds what shows otherwise, and
[`Show::enter`](crate::view::Show::enter) and [`Show::exit`](crate::view::Show::exit) play
transitions ([`crate::guide::motion`]).

```
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::view::show;
use remedy_ui::widgets as w;

let error = Signal::new(None::<String>);
let mut app = TestApp::window(move |ui, _| {
    let u = ui.clone();
    let banner = show(ui, move || error.get().is_some(), move || {
        w::text(&u, move || error.get().unwrap_or_default()).variant("error")
    })
    .fallback({
        let u = ui.clone();
        move || w::text(&u, "All saved").variant("muted")
    });
    ui.root().child(banner.el());
});
let Some(ui) = app.ui() else { unreachable!("open") };
let names = |ui: &remedy_ui::Ui| ui.reader_view().into_iter().filter_map(|n| n.name).collect::<Vec<_>>();
assert_eq!(names(&ui), ["All saved"]);

error.set(Some("The disk is full".into()));
app.frames(1);
assert_eq!(names(&ui), ["The disk is full"]);
```

For content that stays built and only hides (a panel whose state should survive), use
[`El::hidden`](crate::El::hidden) instead; `show` is for content that should not exist while
it is not shown.

## Error boundaries

A bug in one panel should not take the window down. `catch_errors(ui, fallback, child)` shows
what `child` builds. When building it, or any handler, binding, effect or painter inside it,
panics, the boundary unmounts that content and shows what `fallback` builds from the
[`CaughtError`](crate::view::CaughtError) and a [`Retry`](crate::view::Retry). The rest of the
window carries on.

The error says what failed, where in the tree (`element_path`), where in the source
(`location`) and while doing what ([`FailurePhase`](crate::view::FailurePhase)). It also goes
to the window's failure list ([`Ui::failures`](crate::Ui::failures)) and error hook
([`Ui::set_error_hook`](crate::Ui::set_error_hook)), where an app can log or report it. The
inspector's Failures page lists them, with Retry for boundaries that are still failed.

```
use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::view::{FailurePhase, catch_errors};
use remedy_ui::widgets as w;

// Contact lines the app reads from a file; the second has no phone number.
let lines = Signal::new(vec!["Ada;ada@example.com;555-0100".to_string(), "Grace;grace@example.com".to_string()]);
let mut app = TestApp::window(move |ui, _| {
    let (u1, u2) = (ui.clone(), ui.clone());
    let contacts = catch_errors(
        ui,
        move |err, retry| {
            w::column(&u1)
                .child(w::text(&u1, format!("Contacts could not be shown: {}", err.message)).variant("error"))
                .child(w::button(&u1, "Try again", move || retry.retry()))
        },
        move || {
            let list = w::column(&u2);
            for line in lines.get() {
                let fields: Vec<&str> = line.split(';').collect();
                // A bug: not every line has a phone number, and this index panics.
                let phone = fields[2];
                let _ = list.clone().child(w::text(&u2, format!("{} {phone}", fields[0])));
            }
            list
        },
    );
    ui.root().child(w::heading(ui, 1, "People")).child(contacts);
});
let Some(ui) = app.ui() else { unreachable!("open") };
assert!(app.find(Role::Heading(1), "People").is_some(), "the rest of the window carries on");
assert!(app.find(Role::Button, "Try again").is_some());
let failures = ui.failures();
let Some(failure) = failures.first() else { unreachable!("recorded") };
assert_eq!(failure.phase, FailurePhase::Build);

// The data is fixed; Retry builds the content again.
lines.set(vec!["Grace;grace@example.com;555-0199".to_string()]);
assert!(app.press(Role::Button, "Try again"));
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Grace 555-0199")));
```

The boundary takes a builder, not an element, so Retry can build the content again. A panic
with no boundary above it is logged and contained where it happened, and the window goes on.

## Data that arrives later

A [`Resource`](crate::reactive::Resource) fetches a value off the UI thread for a key: when
the key changes it cancels the fetch under way and starts another, and when its owner goes
it cancels for good. The fetch gets a [`Cancel`](crate::reactive::Cancel) to check, and
returns a `Result`; [`Resource::get`](crate::reactive::Resource::get),
[`loading`](crate::reactive::Resource::loading) and [`error`](crate::reactive::Resource::error)
are tracked, so bindings follow them. Fetches run on the shared
[`WorkerPool`](crate::reactive::WorkerPool), or on an [`Executor`](crate::reactive::Executor)
provided as context, which is how an app with its own async runtime plugs it in. RemedyUI
depends on no async runtime.

`suspense(ui, placeholder, child)` builds the content at once, hidden, so its resources start
fetching, and shows it when every resource read inside it has loaded. The placeholder appears
only after a short delay ([`SUSPENSE_DELAY`](crate::view::SUSPENSE_DELAY)) and then stays at
least [`SUSPENSE_MIN_SHOWN`](crate::view::SUSPENSE_MIN_SHOWN), so quick loads show nothing and
slow ones never flash.

```
use remedy_ui::reactive::{Resource, Signal};
use remedy_ui::testing::TestApp;
use remedy_ui::view::suspense;
use remedy_ui::widgets as w;

let user = Signal::new(7u32);
let mut app = TestApp::window(move |ui, _| {
    let (u1, u2) = (ui.clone(), ui.clone());
    let profile = suspense(
        ui,
        move || w::spinner(&u1, "Loading the profile"),
        move || {
            // In a real app: a database or network call. It runs on a worker.
            let name: Resource<String, String> =
                Resource::new(move || user.get(), |id, _cancel| Ok(format!("User {id}")));
            w::text(&u2, move || name.get().unwrap_or_default())
        },
    );
    ui.root().child(profile.el());
});
let Some(ui) = app.ui() else { unreachable!("open") };
let shows = |name: &str| ui.reader_view().iter().any(|n| n.name.as_deref() == Some(name));
for _ in 0..1000 {
    if shows("User 7") {
        break;
    }
    std::thread::yield_now();
    app.frames(1);
}
assert!(shows("User 7"));

// A new key: the old fetch is cancelled and a new one runs.
user.set(8);
for _ in 0..1000 {
    if shows("User 8") {
        break;
    }
    std::thread::yield_now();
    app.frames(1);
}
assert!(shows("User 8"));
```

Put a `catch_errors` around a `suspense` to show a failed fetch with a way to retry: a
resource's error is a value ([`FetchError`](crate::reactive::FetchError)), so the content
decides how to show it, and a panic in the content reaches the boundary.
