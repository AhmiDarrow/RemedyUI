# Performance

RemedyUI aims to be as fast as the hardware allows, and measures it rather than hoping. The
engine's rule is that the cost of a change matches the size of the change, and nothing runs
while nothing changes. This page covers the budgets, how the work is split so an app keeps
inside them, and the tools that show where a frame's time went: the performance overlay and
Perfetto traces (feature `devtools`).

## Budgets

A frame has the time between two refreshes of the display it is on: 16.7 ms at 60 Hz, 8.3 ms
at 120 Hz. [`Ui::refresh_interval`](crate::Ui::refresh_interval) says what the window's
display gives. Within that:

- **Input to screen** in one frame. Handlers, bindings, style, layout and paint for a change
  run in the frame the change happened in.
- **Scrolling and compositor animation** keep the display's rate even when the UI thread is
  stalled: they run on the compositor thread.
- **An idle window does no work** and presents nothing.

The engine's tests hold reference scenes to these budgets. The 300-page document zoomed
from 1% to 6400% and back, a wheel notch a frame, keeps the UI thread's p95 near 0.2 ms
against an 8 ms budget (`crates/remedy-ui-doc-pages/tests/zoom_budget.rs`, release builds);
animated images, tables and the terminal have budget tests of their own.

## Keeping work off the frame

What makes an app slow is almost always work in the wrong place. In order of how often it
matters:

1. **Slow work off the UI thread.** File reads, key derivation, network calls, decoding and
   searching go to [`App::background`](crate::App::background), which hands the result back
   on the UI thread. The engine's own image decoding, page rendering and sorting already run
   on workers.
2. **Bind the smallest thing.** A text closure on one label updates one label. A `bind` on a
   container that rebuilds its children rebuilds them all. Read signals as close to where
   they are shown as you can.
3. **Constants are free.** A property set to a value creates no binding at all. Only use
   a closure or a signal for what changes.
4. **Virtualize long lists.** [`widgets::virtual_list`](crate::widgets::virtual_list) and the
   `data` views build only what is near the viewport; [`widgets::for_each`](crate::widgets::for_each)
   builds every row (see [`crate::guide::lists`]).
5. **Stop the wave early.** A [`Memo`](crate::reactive::Memo) whose value did not change stops
   everything downstream of it.
6. **Isolate what moves.** Content that moves or zooms on its own goes in its own layer
   ([`El::isolate_paint`](crate::El::isolate_paint)), so moving it repaints nothing.
7. **Let the engine pick the renderer.** Windows start on the CPU renderer and move to the
   GPU when their frames get expensive ([`RendererChoice`](crate::RendererChoice)); forcing
   one is for tests and special cases.

## Seeing what a frame did

[`Ui::last_frame_stats`](crate::Ui::last_frame_stats) tells what the last frame redid:
elements restyled, laid out and repainted, and whether layers and semantics were rebuilt. It
is cheap enough to check in tests that a change touches only what it should (see
[`crate::guide::testing`]). The inspector's Signals page explains each update: "row “42”
relaid out because its text binding re-ran: `message.text` changed (set at chat.rs:88)".

## Frame timings

Every phase of the engine emits a `tracing` span: events, binds, style, layout, paint, layers,
semantics, composite, raster and present, a frame span per window, and the work done on
workers. The devtools [`Recorder`](crate::devtools::Recorder) is a `tracing` subscriber that
turns them into per-frame timings while something wants them, and costs one flag check per
span otherwise.

```
use remedy_ui::devtools::Recorder;
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

let Ok(recorder) = Recorder::install() else { unreachable!("no other tracing subscriber here") };
let feed = recorder.frames(120);

let count = Signal::new(0);
let app = move |app: &App| {
    app.open(WindowSpec::new("Counter"), move |ui, _| {
        ui.root().child(w::text(ui, move || format!("{} items", count.get())));
    });
};
let mut test = TestApp::new(AppConfig::new("Counter"), app);
test.frames(2);
count.set(5);
test.frames(1);

let frames = feed.take();
assert!(!frames.is_empty());
for f in &frames {
    // Each frame's time, split by phase.
    assert!(f.total_ns() >= f.phase("layout") + f.phase("paint"));
}
```

`Recorder::install` makes the recorder the process's global `tracing` subscriber. It fails
when the app installed its own first; the spans then go to the app's subscriber.

## The performance overlay

[`PerfOverlay`](crate::devtools::PerfOverlay) draws a window's recent frames over it as
stacked bars, one color per phase, against a budget: the display's refresh unless you give
one. Frames over the budget are counted. It also shows what each frame redid and, if asked,
the damage rectangles. The inspector turns it on and off for any window; an app can show it
itself:

```
use remedy_ui::devtools::{PerfOverlay, Recorder};
use remedy_ui::testing::TestApp;
use remedy_ui::{App, AppConfig, WindowSpec};
use std::time::Duration;

let Ok(recorder) = Recorder::install() else { unreachable!("no other tracing subscriber here") };
let app = |app: &App| {
    app.open(WindowSpec::new("Editor"), |_, _| {});
};
let test = TestApp::new(AppConfig::new("Editor"), app);
let Some(ui) = test.ui() else { unreachable!("open") };

let overlay = PerfOverlay::show(&ui, &recorder, None);
assert_eq!(overlay.budget(), Duration::from_nanos(16_666_667), "a 60 Hz display's frame");
overlay.set_budget(Some(Duration::from_millis(8)));
assert_eq!(overlay.budget(), Duration::from_millis(8));
overlay.show_damage(true);
drop(overlay); // dropping the last handle removes it
```

The overlay draws itself in the window's overlay layer and never redraws for a frame that
only redrew the overlay, so it does not measure itself.

## Perfetto traces

For a whole session, capture every span of every thread and the GPU's own measured time
(where the adapter has timestamp queries) on one timeline, and open it in
[Perfetto](https://ui.perfetto.dev):

```
use remedy_ui::devtools::Recorder;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

let Ok(recorder) = Recorder::install() else { unreachable!("no other tracing subscriber here") };
recorder.start_capture();

let app = |app: &App| {
    app.open(WindowSpec::new("Traced"), |ui, _| {
        ui.root().child(w::heading(ui, 1, "Hello"));
    });
};
let mut test = TestApp::new(AppConfig::new("Traced"), app);
test.frames(3);

let trace = recorder.stop_capture();
let json = trace.to_chrome_json();
assert!(json.contains("\"layout\""), "the layout phase is in the trace");
// trace.write(Path::new("session.json")) saves it for ui.perfetto.dev.
```

The inspector's Performance page records and saves traces without code. Apps on another
device (an Android emulator) are reached through remote devtools
([`Window::serve_devtools`](crate::Window::serve_devtools)).

## Memory

The engine has no fixed caps. Caches (glyphs, images, tiles, pages, scrollback) start small,
grow with use up to a share of the machine's memory, and give memory back when the system
says it is short. Apps do not size them. What an app controls is what it keeps alive: drop
elements and signals it no longer shows (removing an element disposes its bindings; see
[`crate::guide::signals`]).
