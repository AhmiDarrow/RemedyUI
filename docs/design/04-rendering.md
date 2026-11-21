# 4. Rendering and compositing

The renderer is where engines win or lose on smoothness, battery and "it doesn't start on my machine". RemedyUI follows the engines that got this right:

- **WebRender and Chromium's compositor:** layers, tiles and damage tracking.
- **Flutter's Impeller:** no shader compiled while the user waits.
- **Core Animation:** compositor-side animation.

This document describes what the code does. What is planned but not written yet is listed
under [Not yet in code](#not-yet-in-code) at the end, and nowhere else.

## From paint to pixels

```
element paint() ──▶ display list ──▶ layer tree ──commit──▶ compositor (plan: tiles, ops, damage)
                                                                   │
                                     raster tiles that changed (GPU or CPU) ◀─┘
                                                                   │
                     composite only the damage into the kept frame ──▶ present
```

- **Display items** (`remedy_ui::paint::Item`) are plain data recorded per element:
  `FillRect`, `Border`, `Shadow` (a soft shadow of a rounded rectangle), `FillPath`,
  `StrokePath`, `Glyphs`, `Image`, `Video`, `Texture` (a GPU texture on the renderer's
  device), `PushClip`/`PopClip`, `PushLayer`/`PopLayer` (opacity and blend groups),
  `PushFilter`/`PopFilter` (a Gaussian blur, a drop shadow of any shape, or a colour
  matrix, applied to a group), `Backdrop` (frosted glass over what the list drew before it) and
  `PushTransform`/`PopTransform`. Images, video frames and glyph runs are shared by
  reference (`Arc`), so a list is cheap to clone and send to the compositor thread.
- **Paint chunks** are reused until their element is `PAINT`-dirty. A frame re-records only
  dirty chunks; unchanged layer content keeps its version, so its tiles are not rastered again.
- **Layers** group chunks. A **layer tree** is what the UI thread commits.

## Layerization

A layer costs GPU memory and a composite. Not having one costs re-rasterizing a parent. RemedyUI gives a layer to:

| Gets a layer | Why |
|---|---|
| Each scroll container's content | Scrolling moves the layer on the compositor; nothing re-rasters |
| Elements with ambient loops or painters asking for ambient frames (`AmbientLoop`, Prism effects) | They change on their own clock; a frame costs only them |
| Elements with a backdrop blur (`Style::backdrop_blur`) | The compositor blurs what lies under them |
| Elements marked `isolate_paint()` by the app | Expensive, rarely-changing content (a chart axis, a page) |
| Overlays (menus, tooltips, modals) | They appear and disappear without touching the page below |

Everything else is drawn into its nearest layer.

## Tiles

Layers are split into tiles of 256 × 256 device pixels, or 512 at a raster scale of 2 and
above. Only tiles in the viewport are rastered, plus a one-tile prefetch ring inside scroll
containers; tiles that leave it are released. Zooming re-rasters tiles at the new scale
within a per-frame raster budget measured on the machine, visible tiles nearest the middle
first, while the old tiles are shown scaled in the meantime, as browsers and PDF viewers do.
Content with nothing to stand in for it is never held back.

## Damage

Each frame plan carries the **damage**: the device-pixel area that changed since the last
frame (layers whose content, transform, opacity, clip or order changed, tiles replacing
stand-ins, and a new size or first frame as the whole window). Damage rectangles merge when
they overlap or their union wastes little area.

- **Glass and blurs.** A change within a backdrop's reach damages the glass and everything
  its blur reads, so a frame drawn from damage alone is the same frame as a full one.
  Filter groups grow their layer's bounds by their reach, so their damage covers the blur.
- **Both renderers keep the last frame** and composite only the damaged rectangles into it
  (`FramePlan::redraw_rects`: the damage and a pixel around it; the whole frame when there is
  nothing to keep, the size changed, or the damage covers more than 60% of it). Rectangles
  that redraw glass start on a 64-pixel grid, since a blur's result depends on where its
  view starts. Both are tested to give the same pixels as a full redraw.

## The GPU renderer

The GPU renderer runs on wgpu: D3D12, Vulkan, Metal, OpenGL 3.3 / GLES 3.0, and WebGPU or
WebGL2 in a browser.

- **One painter for both renderers.** Display lists are drawn by the same `Painter` as the
  CPU renderer onto a `vello_gpu` scene (sparse strips): shapes, paths, gradients, clips,
  opacity and blend groups, filter groups and backdrops go through vello. Glyphs are
  rasterized by RemedyUI once per size and subpixel position (grayscale and LCD) and drawn as
  tinted images, so both renderers draw the same text.
- **Pipelines** are vello's, RemedyUI's video conversion and Prism's effect kernels, all made
  when the renderer is created, through a **persistent pipeline cache** on disk where the
  driver supports one. Nothing compiles mid-frame (a test counts pipelines and shaders after
  start-up).
- **Shadows and filters.** `Item::Shadow` is vello's closed-form blurred rounded rectangle,
  with no blur pass. Blur and drop-shadow filter groups and backdrops are vello filter layers
  (a multi-scale Gaussian), the same algorithm `vello_cpu` runs, so the two renderers agree
  within two levels. A drop shadow moved wholly outside the target is drawn without its
  shadow (it shows nothing there): vello_cpu draws one moved past the left or top edge under
  the shape, and vello_gpu sizes buffers to reach one, past the device's limit when nested.
- **Glass is bounded**: each pane draws again what is under it, glass included, so a stack
  of panes would double the cost with every pane. Glass seen through more than
  `MAX_BACKDROP_DEPTH` (2) panes, and panes past `MAX_BACKDROP_REPLAYS` (128) in one raster,
  are drawn without their frost, in display lists and in the compositor alike.
- **Nesting is bounded**: at most `MAX_GROUP_DEPTH` (256) clips, groups, filters and glass
  are open at once while a list draws; pushes past it open nothing and their pops close
  nothing (vello_gpu overflows its stack somewhere past 500, which would abort the process).
- **Colour matrices** (`Filter::ColorMatrix`) are RemedyUI's own pass, since vello's filter
  graph runs none. The group is drawn into a picture of its own, in device pixels, covering
  only what it draws inside the target (its corner on a 64-pixel grid, so blurs inside it
  shrink on the same grid as anywhere else); each pixel is unpremultiplied, run through the
  4 × 5 matrix, clamped and premultiplied again; the picture is placed back pixel for pixel.
  The CPU runs `ColorMatrix::apply_premultiplied` itself; the GPU renders the picture with
  vello and runs the same arithmetic in a small fragment pass made at start-up, so the two
  agree to a step of rounding (a matrix that amplifies also amplifies the renderers' own
  one-step differences in what it recolours). Pixels the group leaves transparent stay
  transparent, so the result never depends on how the frame is cut into tiles or damage
  rectangles. A matrix reaches no further than the group draws. Bounded for untrusted
  themes: entries are finite and within ±`MAX_COLOR_MATRIX_ENTRY`, at most
  `MAX_FILTER_MATRIX_DEPTH` (4) matrix groups nest and at most `MAX_FILTER_MATRIX_GROUPS`
  (64) are drawn in one raster; past those a group is drawn without its matrix.
- **Images drawn scaled down** come from mipmaps the shared `Painter` builds: an image
  drawn smoothly (`Sampling::Linear`) at half its size or less, after every transform, is
  drawn from the copy halved enough times to be drawn at more than half its own size (each
  level a rounded 2 × 2 box average of premultiplied pixels), so bilinear sampling reads
  every source pixel and a scaled-down photo or zoomed-out page does not shimmer. Levels are
  built the first time they are needed (about 30 ms for a 16-megapixel image's first level,
  once) and kept until the app drops the image; only the level drawn is uploaded to the GPU,
  so a large image shown small costs its small size there. Nearest sampling (pixel art) is
  never mipmapped. Image *elements* already decode at the size they are shown
  ([15](15-widgets-and-media.md)); mipmaps cover images drawn smaller than they were decoded
  (display lists, thumbnails of one decoded image, zooming out).
- **Images and glyphs** are uploaded to the GPU on first use: glyphs and nearest-sampled
  images packed into RGBA atlas pages, filtered or large images in textures of their own.
  Uploads of images the app dropped are freed, and `clear_caches` drops them all (they are
  made again as they are drawn). Under memory pressure the compositor frees every tile outside
  the prefetch area (`Compositor::trim_memory`).
- **Video** planes are uploaded as they come (one plane of RGBA, NV12's Y and UV, or I420's
  three planes) and a small render pass converts them to RGBA with the stream's matrix and
  range, once per frame per stream.
- **Compositor tiles** live in textures. Small tiles are rastered together in one atlas and
  copied out (every render has a fixed cost); tiles whose content holds filters raster on
  their own, so their blur lines up with the CPU's.
- **The kept frame.** Each window's frame lives in a texture of its own. A present
  composites only the damage into it: each damaged rectangle is a scene of its own size,
  placed at its corner (vello's render offset) with only that rectangle cleared, so the GPU's
  passes cover the damage alone. The frame is drawn before the swapchain image is acquired,
  so an occluded or stale swapchain loses no damage, then copied to the swapchain image (or
  drawn onto it where the swapchain takes no copies).
- **Blending** happens in sRGB space, the way browsers and design tools do: targets are plain
  8-bit RGBA or BGRA, premultiplied.

## The CPU renderer

The CPU renderer draws the same display lists and layer trees:

- `vello_cpu` (single-threaded) handles shapes, paths, gradients, clips, groups and filters
  through the shared `Painter`; RemedyUI handles glyph coverage (LCD included), images and YUV
  conversion.
- Presenting is a pixel blit through the bridge (softbuffer on desktops, a canvas in a
  browser), passing the damage rectangles where the platform takes them.

It is used where no GPU adapter is found, for windows that do not need the GPU (see below),
for headless tests and for rendering to images, with the same output as the GPU path within
a tested tolerance.

## Choosing, recovering and degrading

1. **The backend ladder.** The GPU renderer tries a hardware adapter through D3D12, Vulkan
   or Metal, then OpenGL 3.3 / GLES 3.0, then a software adapter (WARP, lavapipe, llvmpipe).
   **A self-test frame** checks each rung's device before it is used (`health::self_test`,
   on unless `LadderOptions::self_test` is off): a 64 × 64 scene of what interfaces draw
   (fills, rounded edges, a gradient, an image, a blur, a colour matrix) drawn on the device
   and on the CPU reference renderer must agree, with no validation errors and no loss, or
   the ladder moves down a rung. The verdict is remembered per adapter, driver, backend and
   engine version (`health::remember_in`, which the runner points at the app's data folder),
   so it runs once per driver: about 120 ms on llvmpipe in a release build. Browsers cannot
   read pixels back mid-frame, so there it is skipped.
2. **Which renderer a window uses.** By default (`RendererChoice::Auto`) a window starts on
   the CPU renderer, which costs no GPU driver memory, and moves to the GPU once its frames
   take longer than 6 ms on average; heavy content (a Prism effect over its cost threshold)
   asks for the GPU at once. Where a surface keeps its first renderer (Android) windows start
   on the GPU. Apps can choose GPU or CPU only.
3. **Device loss.** The renderer re-creates the device, drops its uploads and tiles, and
   redraws the full frame from the retained layer tree. Apps are not involved.
   **A present watchdog** (`health::PresentWatchdog`, a thread per compositor) watches every
   GPU present: one running past `PRESENT_HANG` (3 s; Windows resets a GPU silent for 2 s) is
   recorded as a hang at once, from the watchdog's thread, so a present that never returns
   still demotes the backend at the next start. When it does return, the device is let go as
   if lost and recovery walks the ladder past the hung backend (another API or adapter, the
   software rung, or the CPU renderer).
4. **A failure that recovery cannot fix** (no replacement device, a window whose GPU present
   fails) moves the window to the CPU renderer.

## Presenting

- **Present mode.** Frames pace to the display (vsync) by default; apps may present as soon
  as each frame is drawn instead (`AppConfig`'s vsync setting).
- **Partial presents** happen on the CPU path, where the bridge hands the damage rectangles
  to the platform (softbuffer's `present_with_damage`, or only those rectangles put on a
  browser canvas). wgpu presents whole swapchain images, so the GPU path rasters and
  composites only the damage but presents the whole kept frame (one copy).

## Effects and custom shaders

- **Prism** ([16](16-effects.md), reference [rfx.md](../effects/rfx.md)) is how anything
  draws procedural, animated pictures: fire, water and caustics, glow and bloom, light shafts,
  particles, starfields, grain, CRT screens, raymarched 3D. Effects are `.rfx` data compiled
  against cost limits into programs for one fixed GPU kernel (an interpreter RemedyUI ships,
  so nothing compiles after start-up and nothing from a theme reaches the GPU) or, with the
  same results, the CPU interpreter.
- **Blur, shadows and glass on any element**, from its style (CSS names in brackets):
  - `Style::shadow` (`box-shadow`): `Item::Shadow` under the box.
  - `Style::blur` (`filter: blur()`) and `Style::drop_shadow` (`filter: drop-shadow()`, a
    shadow of the shape of whatever the element draws): filter groups around the element's
    drawing and its children. When some of its children are layers of their own (scroll
    containers, isolated or animated elements, glass), the element becomes a layer with
    `Layer::filters` and the compositor draws the whole subtree through them
    (`CompositeOp::PushFilter`/`PopFilter`); damage there grows by the filters' reach and
    redraws the whole group, so damage frames stay exact.
  - `Style::saturate`, `hue_rotate`, `grayscale`, `sepia`, `brightness`, `contrast`,
    `invert` (`filter: saturate()` and the rest) and `Style::color_matrix` (SVG
    `feColorMatrix`): chained in order into one colour matrix, the innermost filter of the
    element (applied before a blur or drop shadow, which keeps its own colour), drawn the
    same two ways.
  - `Style::backdrop_blur` (`backdrop-filter: blur()`): the element gets a layer with a
    backdrop; the compositor blurs every layer drawn before it under its box, back to the
    innermost translucent layer or filter group around it (its backdrop root, as on the web).
    The element's opacity fades its glass too. A scroll container's glass is its viewport
    layer, which also takes the container's own background.

  Display lists drawn on their own have the same tools: `Item::PushFilter`/`PopFilter` and
  `Item::Backdrop` (which sees what the list drew before it).
- **Display-list effects**: gradients, blend modes including additive (`Blend::Plus`) for
  light that stacks.
- **Ambient loops**: endless transform and opacity motion a layer carries (`AmbientLoop`),
  run by the compositor at the window's ambient rate ([09](09-performance-and-resources.md)).
- **App shaders**: apps that need their own GPU work render it with `gpu_view` (the app's
  wgpu on RemedyUI's device, [15](15-widgets-and-media.md)). Raw WGSL is never accepted from
  untrusted content (a theme or a mod); Prism is the language for that.

## Rendering to images and printing

- **Snapshots.** Any display list renders to an image at any scale on either renderer
  (`CpuRenderer::render`, `GpuRenderer::render`): CI screenshots, share previews, thumbnails.
  `Window::snapshot` pictures a window's next frame: a window the GPU draws reads back the
  frame it keeps (`GpuCompositor::read_surface`), so the picture holds what the screen does,
  Prism effects, video, `gpu_view` textures, blurs and glass included; a window the CPU draws
  draws its frame once more.
- **Paged documents** (`remedy-ui-doc-pages`) show pages the app parses (a PDF through
  MuPDF, an e-book) as display lists.

## Not yet in code

Planned, not implemented; nothing above relies on them:

- **Merging and squashing layers**: adjacent siblings with no reason to be separate merged,
  and layers that cover a tiny area squashed back into their parent. Measured and not worth
  it yet (`tests/layers.rs` in `remedy-ui-render-gpu`; release, 1280 × 800, CPU compositor
  and llvmpipe): a small layer costs the compositor's plan about 0.8 µs a frame (1,000 icon
  layers: 0.83 ms against 0.07 ms merged) and its first frame a texture each (GPU 104 ms
  against 57 ms), but with one icon changing a frame, merged icons re-raster their shared tile
  (CPU 9.1 ms, GPU 28 ms against 0.6 ms and 1.8 ms as layers). RemedyUI gives layers only for a
  reason (scrolling, animation, glass, filters), so trees hold tens of them, where the cost
  is a few tens of microseconds a frame.
- **Partial presents on the GPU** (`Present1` dirty rects, `eglSwapBuffersWithDamage`,
  Wayland `damage_buffer`): wgpu does not offer them.
- **Platform presenting details**: a DirectComposition flip-model swapchain with a
  frame-latency waitable object, display-link pacing on macOS, Wayland frame callbacks with
  fractional scale, Android Choreographer pacing; system backdrops (Mica, acrylic,
  vibrancy); mailbox and variable-refresh present modes.
- **Wide-gamut and HDR surfaces**, and colours tagged with their colour space.
- **GPU video without copies**: importing decoder textures (D3D11/12, VA-API dma-buf,
  Android `HardwareBuffer`, IOSurface) instead of uploading planes.
- **A multithreaded CPU raster** (vello_cpu's threads; its filters do not run with them yet).
- **Printing and vector PDF**: translating display lists to PDF (selectable text, sharp
  paths) and printing through each platform's print dialog.
