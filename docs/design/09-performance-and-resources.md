# 9. Performance and resources

"As fast as the hardware allows" is measured, not hoped for. This document says how RemedyUI spends time and memory, how it sizes itself without fixed caps, and how CI keeps it honest.

## Where the time goes

| Phase | Steady-state rule |
|---|---|
| Input | Coalesced per frame; handlers run once per frame per gesture, with full history available |
| Reactivity | Proportional to the signals that changed and their real dependents |
| Style | Cache hit for every element whose (kind, variant, state, theme version) was seen before |
| Layout | Only dirty subtrees up to relayout boundaries; text shaping only for changed paragraphs |
| Paint | Only dirty chunks re-recorded |
| Commit | Proportional to changed chunks and layers (shared `Arc` snapshot) |
| Raster | Only damaged tiles |
| Present | Only damaged regions sent to the OS |

**No allocation proportional to the screen.** Per-frame data lives in bump arenas that are reset each frame, and retained data reuses its storage. An idle frame and a compositor-only scroll frame allocate nothing. A frame that commits allocates only for the chunks and layers that changed, from pooled buffers. Benchmarks assert both.

## Sizing without fixed caps

The engine never uses a fixed number where the right size depends on the machine or the content:

- **Caches size themselves from the device.** Glyph atlases, shaped-run caches, image caches, layer caches and tile pools start small and grow with use. Each has a target share of the machine's memory: a fraction of physical RAM for CPU caches, and of the adapter's reported budget for GPU caches. The app can change the share.
- **Memory pressure shrinks them.** The bridges forward the OS signals (`onTrimMemory` on Android, low-memory notifications on Windows, `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` on macOS, PSI on Linux, the kernel pressure wire on RemedyOS) as a pressure level. Caches evict least-recently-used entries until they are under the level's target. Off-screen tiles go first, then layer caches, then glyphs and images not drawn recently.
- **Tables grow.** Element arenas, signal arenas, atlas pages and instance buffers grow by doubling and shrink when mostly empty. Nothing has a hard maximum count.
- **Workers**: one per core beyond the UI thread, created and parked on demand ([01](01-threads-and-frames.md)).
- **Decode bounds** come from what can be shown ([08](08-security-and-privacy.md)). They are not a byte limit.

There are no fixed capacity limits. The fixed numbers that remain are tuning constants of the design (tile size, atlas page size, the clip depth folded into a shader, gesture time windows, the size above which text draws as paths). Each is documented in the code with why it is that value, and none limits how much an app can show.

## Start-up

- **Lazy everything.** Fonts, locale data, spell checkers, decoders and path shaders initialize on first use ([00](00-principles-and-modularity.md)).
- **Pipeline cache.** GPU pipelines load from the on-disk cache. On a first run, the first frame uses the pipelines it needs (a handful) and the rest warm up at idle.
- **Small first frame.** A window can show its first frame before off-screen screens are built. Navigation stacks build later screens when they are first shown.
- **Restore.** Persisted window geometry and scroll anchors are applied before the first frame, so there is no visible jump.

## Real-time content

Some apps draw at the display's rate all the time: tuners, pitch graphs and meters (GuitarRemedy, VoiceRemedy), video, live transfer progress. For these:

- **Streaming data.** `viz` primitives are retained, built-in drawing (lines, bars, needles, fills) that the compositor draws straight from a ring buffer the app writes from any thread (the audio callback, a decoder). The compositor reads the newest data at each frame, and the UI thread is not involved per frame.
- **App canvases** are different: their paint closures are app code, so they run on the UI thread, on their own layer, with no layout, and only in frames where the data they read changed. Display-rate visuals should be built from `viz` primitives.
- **Frame-synced, not polled.** A `viz` element requests frames only while its source reports new data, and stops when the microphone stops or the window hides (VoiceRemedy's "stop the mic when hidden" becomes automatic).

## Ambient motion

Decoration that moves all the time (a theme's fireflies, a flickering fire, a sweeping
lighthouse beam, a slow drift of clouds) runs under a budget, the **ambient policy**
(`AmbientPolicy`, set per app with `App::set_ambient_policy`):

- **Throttled, not every vsync.** Ambient motion runs at the policy's rate: 30 frames a second
  with focus, 10 in the background, 15 (and "light") while the system saves power, by default.
  `AmbientMode::Light` lowers the rate and tells painters to draw less; `Off` stills it.
- **Zero when unseen.** Minimized, hidden and covered windows draw none; an element scrolled
  out or clipped away schedules nothing and resumes when it shows; reduced motion stills it.
  The same holds for painters that ask for every frame (`request_next_frame`).
- **Three tools, cheapest first.** Static art is drawn once into a picture (`PaintCx::cached`,
  uploaded to the GPU once, keyed by image id). Pure transform and opacity motion is an
  `AmbientLoop` on the element's layer that the compositor samples at the ambient rate, with no
  UI-thread work, no repaint and no re-raster; only the layer's screen area is damaged. Art that
  changes uses `PaintCx::ambient()`, which schedules the element's next repaint at the rate,
  and gives the element a layer of its own so the repaint re-rasters only it; ancestors are not
  re-recorded.
- **Measured** on the runner with the reference scene (a cached backdrop, 50 animated sprites,
  a beam sweeping the window): an idle or minimized window draws no frame, the scene runs at
  30 fps focused and 10 in the background. On the CPU renderer every present still composites
  the whole window; compositing only the damage is the next step.

## Large content

- **Lists and grids** of any length: virtualization with prefetch ([03](03-layout.md)).
- **Documents and pages.** Paged content (RemedyPDF) is tiled per page and zoom level. Tiles render on workers at the current zoom while scaled tiles from other zoom levels stand in. Zooming from 1% to 6400% never renders more than the visible tiles plus a margin.
- **Long text.** Paragraphs lay out lazily as they scroll in. Editing reshapes only changed paragraphs, over a rope.
- **Large trees and tables** (SecretFolder, RemedyOS Files and Monitor): rows virtualize, columns measure from visible rows plus an estimate, and sorting runs on the data model, off the UI thread for large sets.

## Measuring

- **Benchmarks in CI.** Reference scenes run on the CPU renderer, and on GPU runners where available, at fixed sizes. The scenes are:
  - a chat with 100,000 messages (FreeSpeak, RemedyAI)
  - a 300-page PDF at three zoom levels (RemedyPDF)
  - an explorer with 50,000 files (SecretFolder)
  - a 60 Hz pitch graph and tuner (VoiceRemedy)
  - a settings form (all apps)
  - the RemedyOS desktop with ten windows
- **What is recorded:** CPU time per phase, allocations, memory, GPU time where available, binary size and start-up time.
- **Gates.** A change that makes a reference scene slower or larger beyond noise (best of N runs, compared to main) fails CI, unless the change carries a written reason and an updated baseline.
- **Tracing.** Every phase emits `tracing` spans. The devtools export a Perfetto trace of any session, with UI thread, compositor, workers and GPU on one timeline.
- **Performance overlay.** It shows frame time per phase, dirty element counts, damage rectangles, cache sizes against their targets, and dropped frames.
