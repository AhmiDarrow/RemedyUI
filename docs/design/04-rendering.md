# 4. Rendering and compositing

The renderer is where engines win or lose on smoothness, battery and "it doesn't start on my machine". RemedyUI follows the engines that got this right:

- **WebRender and Chromium's compositor:** layers, tiles and damage tracking.
- **Flutter's Impeller:** no shader compiled while the user waits.
- **Core Animation:** compositor-side animation.
- **WinUI on DirectComposition:** low-latency presenting.

## From paint to pixels

```
element paint() ──▶ paint chunk (display items) ──▶ layer tree ──commit──▶ compositor
                                                                   │
                                     raster (GPU or CPU, dirty tiles only) ◀─┘
                                                                   │
                                          composite layers ──▶ present with damage
```

- **Display items.** These are compact, plain-data commands recorded into a per-element **paint chunk**, in a bump-allocated buffer: `RoundRect`, `Border`, `Shadow`, `Gradient`, `Path`, `GlyphRun`, `Image`, `NineSlice`, `PushClip`, `PopClip`, `PushLayer`, `Filter`, `BackdropFilter`. Items hold no pointers, only ids for glyph runs, images and paths.
- **Paint chunks** are reused until their element is `PAINT`-dirty. A frame re-records only dirty chunks.
- **Layers** group chunks. A **layer tree** is what the UI thread commits.

## Layerization

A layer costs GPU memory and a composite. Not having one costs re-rasterizing a parent. RemedyUI gives a layer to:

| Gets a layer | Why |
|---|---|
| Each scroll container's content | Scrolling moves the layer on the compositor; nothing re-rasters |
| Elements with a running transform, opacity or clip animation | The compositor animates them |
| Video surfaces, external textures, animated images | Their content changes on their own clock |
| Elements with a backdrop filter | They sample what is behind them |
| Elements marked `isolate_paint()` by the app | Expensive, rarely-changing content (a chart axis, a page) |
| Overlays (menus, tooltips, modals) | They appear and disappear without touching the page below |

Everything else is drawn into its nearest layer. Adjacent siblings with no reason to be separate are merged, and layers that end up covering a tiny area are squashed back into their parent. This is Chromium's lesson about layer explosion.

## Tiles

Layers larger than the viewport (a long scrolled page, a zoomed PDF page) are split into tiles of 256 × 256 device pixels, or 512 on high-DPI screens. Only tiles in or near the viewport are rastered. A tile that leaves the prefetch area is released when memory is needed. Zooming re-rasters tiles at the new scale in the background, while the old tiles are shown scaled in the meantime, as browsers and PDF viewers do.

## The GPU renderer

The GPU renderer runs on wgpu: D3D12, Vulkan, Metal, and GL 3.3 / GLES 3.0.

- **A fixed, small set of pipelines** (uber-shaders): solid and SDF shapes, gradients, glyphs (grayscale, LCD, color), images (RGBA, YUV), paths (`vello_gpu` strips), blur, composite.
  - They are created at start-up from a **persistent pipeline cache** on disk. Nothing compiles mid-interaction.
  - A first run warms the cache during the first idle moment.
- **Batching.** Primitives are written into instance buffers and drawn in as few calls as the draw order allows.
  - Opaque items are drawn front to back with a depth test, so hidden pixels are never shaded.
  - Translucent items are drawn back to front, and neighbours that do not overlap are merged into one batch.
- **Shapes without tessellation.** Rects, rounded rects with per-corner radii, borders (with dashes) and box shadows are analytic signed-distance quads. Shadows use the closed-form Gaussian-rounded-rect approximation, with no blur pass.
- **Clips** are analytic in the shader for axis-aligned rects and rounded rects, up to a small fixed nesting depth folded into the instance. Deeper or path clips use a mask texture.
- **Paths.** Icons and other small paths are rasterized once at their device size into the atlas and drawn as images, like glyphs. Large or animating paths go through the `vello_gpu` sparse-strip pipeline, behind RemedyUI's `Path` type.
- **Atlases.** There are separate atlases for grayscale glyphs (R8), LCD glyphs and color glyphs (RGBA8), and small images and icons (RGBA8).
  - Packing uses shelf allocation per size class. Atlases grow by adding pages as needed, with no fixed count.
  - Entries not used for a number of frames are evicted when memory pressure asks for it.
  - Uploads go through a staging ring buffer, batched per frame.
- **Images** larger than an atlas slot get their own textures, and mipmaps when they are drawn scaled down. Each is decoded at the size it is shown, not at its source size (see [09](09-performance-and-resources.md)).
- **Video** planes are uploaded as they come (R8 + RG8 for NV12, three R8 planes for I420) and converted to RGB in the shader with the stream's matrix and range. Where the platform decoder gives a GPU texture (D3D11/12, VA-API dma-buf, Android `HardwareBuffer`, IOSurface), it is imported directly with no copy.
- **Blending** happens in sRGB space by default, the way browsers and design tools do, so colors match designers' mock-ups. Text uses gamma and contrast curves matched to each platform's native text. Wide-gamut and HDR windows switch to an extended-linear surface. Colors are tagged with their color space, so this changes no app code.

## The CPU renderer

The CPU renderer draws the same display list and layer tree:

- `vello_cpu` handles paths, shapes and gradients, multithreaded across tiles.
- RemedyUI's compositor handles glyphs (including LCD), images, YUV conversion and layers.
- Presenting is a pixel blit through the bridge (softbuffer class), with damage rects.

It is used where no GPU backend starts, under GL 3.3, and on machines where WARP or llvmpipe misbehave. It is also used for headless tests and for rendering to images, with the same output as the GPU path within a tested tolerance.

## Choosing, recovering and degrading

1. **Start-up.** The bridge proposes backends for the platform. Each is tried with a self-test frame that checks the rendered pixels, not just that creation succeeded. The first that passes is used, and the choice is remembered per GPU and driver version.
2. **Device loss.** The renderer re-creates the device and rebuilds textures from the atlases' CPU-side copies and the retained layer tree, then redraws the full frame. Apps are not involved.
3. **Repeated loss or a hang.** A watchdog on present demotes to the next backend for the session and reports it to the app (`RendererInfo` signal).
4. **Pressure.** Under GPU memory pressure, off-screen tiles go first, then cached layers, then atlas pages not used recently. Under heavier pressure, the renderer draws layers straight to the frame instead of caching them, until the pressure passes.

## Presenting

- **Damage.** The union of changed tile and layer bounds is the damage for the frame. Only it is rastered, and it is passed to the OS where supported: `Present1` dirty rects, Wayland `damage_buffer`, `eglSwapBuffersWithDamage`, Android partial update.
- **Windows.** Flip-model swapchain on DirectComposition, with a frame-latency waitable object. The UI thread is signalled when the next frame can start, keeping input-to-photon low. Mica and acrylic come from the system backdrop APIs.
- **macOS.** Metal through `CAMetalLayer`, with display-link pacing. Vibrancy uses `NSVisualEffectView` behind the content.
- **Linux.** Wayland frame callbacks pace frames, with fractional scale (`wp_fractional_scale_v1`) and viewporter so text is crisp at 125% and 150%. X11 uses present-extension timing where available.
- **Android.** Choreographer pacing, with `ANativeWindow` and Vulkan or GLES.
- **Present mode.** FIFO by default (no tearing, lowest power). Mailbox when the app asks for the lowest latency (drawing apps, games' menus). Variable refresh is used where the display and OS offer it.

## Effects and custom shaders

- **Built in:**
  - Gaussian blur and drop shadow on layers.
  - Backdrop blur (frosted glass), with a downsample/upsample chain limited to the damaged region.
  - Color matrix: grayscale, tint, saturation.
  - Gradient masks and blend modes.
- **App shaders** can be added as WGSL fragment effects. They are validated by `naga` before use, get only the inputs the effect declares (its texture, size, time and uniform values), and cannot read other textures or buffers. An effect that fails validation, or takes too long on the GPU, is disabled with a logged reason and the element draws without it.
- **Untrusted content** (a theme or a mod) cannot ship shaders unless the user turns that on.

## Rendering to images and printing

- **Snapshots.** Any element, window or layer can render to an image at any scale, on the GPU or CPU renderer: CI screenshots, share previews, thumbnails.
- **Printing and PDF.** The display list also translates to vector PDF, so text stays selectable and paths stay sharp. Printing goes through the platform's print dialog on each bridge that has one.
