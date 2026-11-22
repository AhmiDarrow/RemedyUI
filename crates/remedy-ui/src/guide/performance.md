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

## Drawing limits

What a display list or an effect may ask the renderers for is bounded, so a theme, a script
or a document from a stranger cannot stall a frame or exhaust memory. Every bound has a
default that suits ordinary interfaces and a public way to change it, and the CPU and GPU
renderers read the same bounds, so a raised one draws the same on either. The safety bounds
(marked *safety*) are what keep hostile input cheap: raising one lets such input cost that
much more.

| Limit | Default | How to change it |
| --- | --- | --- |
| Groups open at once: clips, layers, filters, glass (*safety*: vello recurses per level, so a much deeper one needs a larger drawing-thread stack) | 256 | `paint::PaintLimits::group_depth` |
| Glass seen through glass, drawn with its frost | 2 | `paint::PaintLimits::backdrop_depth` |
| Panes of glass drawn with their frost per raster | 128 | `paint::PaintLimits::backdrop_replays` |
| Colour-matrix groups nested / per raster | 4 / 64 | `paint::PaintLimits::filter_matrix_depth`, `filter_matrix_groups` |
| Blur of a chain of filters (*safety*) | 64 device pixels | `paint::PaintLimits::filter_std_dev` |
| Offset of a chain of drop shadows (*safety*) | 2048 device pixels | `paint::PaintLimits::filter_offset` |
| Pixels of one effect's buffers (*safety*) | 4 Mi (16 MiB) | `effects::set_max_effect_pixels` |
| Particle overdraw, times the buffer's pixels (*safety*) | 8 | `effects::set_max_overdraw` |
| Effect source, tokens, nesting, loops, instructions, registers, cost, passes, parameters, particles, frame rate | see `effects::Limits` | `effects::Effect::compile_with` |
| GPU errors kept between `take_errors` | 64 | `gpu::GpuLimits::kept_errors` |
| GPU health file: backends / line length | 64 / 512 bytes | `gpu::GpuLimits::health_entries`, `health_line` |
| A present that counts as hung | 3 s | `gpu::GpuLimits::present_hang` |
| Wait for a swapchain image (RemedyUI's own Vulkan and DXGI swapchains) | 1 s | `gpu::GpuLimits::swapchain_wait` |
| Wait for a decoded video picture's writes (Linux DMA-BUF) | 1 s | `gpu::GpuLimits::video_fence_wait` |
| Hardware video pictures kept imported per stream | 64 | `gpu::GpuLimits::video_imports` |

`PaintLimits` and `GpuLimits` hold for the whole process: build one from its default, change
what you need, and pass it to `paint::set_paint_limits` or `gpu::set_gpu_limits`. They apply
from the next frame.

```no_run
use remedy_ui::paint::{PaintLimits, set_paint_limits};

// An app drawing only its own lists, with deep glass and wide soft shadows.
set_paint_limits(PaintLimits::default().backdrop_depth(4).filter_std_dev(128.0));
```

Some numbers are fixed by the GPU API and are not limits. An effect pass samples at most
eight earlier passes (`effects::MAX_SAMPLED_PASSES`) and an effect has at most four texture
parameters (`effects::MAX_TEXTURES`), whatever `Limits::max_textures` says: the effect kernel
binds every texture a pass reads at once (15 in all), and WebGPU guarantees no more than 16
sampled textures to one shader stage. A program names at most 255 registers (the
instruction encoding's 8-bit fields); one past 64 runs on a wider form of the kernel, built
the first time a program needs it.

## Engine limits

The rest of the engine works within bounds of the same kind: each has a default and a public
way to change it. The safety bounds (marked *safety*) keep input from a stranger (a script,
a markup file, a theme) cheap; raising one lets such input cost that much more.

| Limit | Default | How to change it |
| --- | --- | --- |
| Pictures `PaintCx::cached` keeps per UI thread | 512 | `view::set_paint_cache_limit` |
| Longest step an `AmbientTick` reports | 250 ms | `view::AmbientPolicy::max_dt` |
| Largest colour-matrix entry (*safety*: far larger sums can overflow) | ±256 | `core::color_matrix::set_max_color_matrix_entry` |
| Side of an effect element's frames | 4096 px (and the GPU's texture limit) | `effects::EffectView::set_max_side` |
| Noise texture side / textures `fx::noise_image` and `fx::blur` keep | 4096 px / 64 | `stage::fx::set_fx_limits` |
| Zoom of a page view | 1% to 6400% | `pages::PageView::set_zoom_range` |
| Handlers in one script description (*safety*) | 4096 | `script::Limits::handlers` |
| Picture side / gap a script asks for | 4096 / 64 logical px | `script::Limits::image_side`, `gap` |
| Rhai expression nesting, top level / in a function (*safety*: the parser recurses on the stack) | 64 / 32 | `script::Limits::expr_depth` |
| Markup nesting (*safety*: parsing and building recurse on the UI thread's stack) | 128 | `markup::set_max_depth` |
| How long a terminal's synchronized update may hold the screen | 150 ms | `term::TerminalOptions::sync_timeout` |
| How long the devtools Remote page waits for an app | 10 s | `devtools::set_remote_timeout` |
| How long a second launch tries to reach the first (Windows) | 5 s | `AppConfig::single_instance_wait` |
| How long a second launch waits for the first's reply / for its exit | none (hands over and exits) / none; 30 s reply when only the exit wait is set | `AppConfig::single_instance_reply`, `single_instance_await_exit` |
| A question's window: width / tallest before its text scrolls | 420 / 640 logical px | `ConfirmSpec::width`, `max_height` |
| Reading a drop from another app / serving a drag out (Linux) | 2 s / 10 s | `AppConfig::drop_waits` |

Text, network and media limits are in [`crate::guide::text`], [`crate::guide::network`] and
[`crate::guide::media`].

```no_run
use remedy_ui::view::set_paint_cache_limit;

// A dashboard of many cached sparklines.
set_paint_cache_limit(4096);
```

Some numbers are internal tuning rather than limits on what an app can do, and stay fixed:
the lines a lazy piece shares with the next, how much text is read around a position to find
a grapheme boundary, how much terminal input is parsed between checks, how often gamepads are
looked for, and the stack segments deep trees grow on (they grow as far as the tree goes).
