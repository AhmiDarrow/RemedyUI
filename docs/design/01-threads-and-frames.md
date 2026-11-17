# 1. Threads and the frame pipeline

Top engines keep two promises at once. Scrolling and animation stay smooth even when app code is slow, and nothing runs when nothing changes. Chromium does this with a compositor thread, Core Animation with a render server, Flutter with separate UI and raster threads, and Qt Quick with a scene-graph render thread. RemedyUI uses the same split.

## Threads

| Thread | Owns | Never does |
|---|---|---|
| **UI thread** (one per app) | App code, signals, effects, the element tree, style, layout, paint recording, semantics | GPU calls, blocking I/O, waiting on the compositor |
| **Compositor thread** (one per app) | The committed layer tree, the GPU device, rasterization, compositing, presenting. It also runs scrolling and compositor animations | App code, layout, text shaping |
| **Worker pool** (cores − 2, at least 1) | Text shaping and measuring for rows coming into view, glyph rasterization, image and GIF decoding, path flattening, CPU raster tiles | Touch the element tree |
| **Platform thread** (where the OS demands it) | The OS event loop on macOS, and Android's main thread when that is not the UI thread | Anything slow |

The UI thread and the compositor thread meet at one place: the **commit**.

## The frame

```
UI thread:          input → effects → style → layout → paint → semantics → COMMIT ┐
                                                                                  │ (immutable snapshot)
compositor thread:  ┌─────────────────────────────────────────────────────────────┘
                    └→ apply scroll/animation deltas → raster dirty tiles → composite → present
```

1. **Begin frame.** The compositor sends a begin-frame signal on vsync, carrying the predicted presentation time. Only the UI thread's own requests or the compositor's need for a commit cause one.
2. **Input.** Events queued since the last frame are coalesced: pointer moves merge, but their history is kept for ink and prediction. They are then dispatched.
3. **Effects, style, layout, paint, semantics**, as described in the overview. Each step touches only dirty elements.
4. **Commit.** The UI thread builds a new snapshot of the layer tree. Unchanged paint chunks, layers and atlas references are shared with the previous snapshot through `Arc`, so a commit costs as much as the change, not the screen. The snapshot is handed over through a single-slot mailbox. A newer commit replaces an older one that the compositor has not yet taken.
5. **Composite.** The compositor applies its own updates on top of the snapshot: scroll offsets, and transform or opacity animations. It rasters only dirty tiles, composites, and presents with damage rectangles.

The UI thread does not wait for the compositor. If a commit is still being drawn, the next one queues in the mailbox. If the UI thread misses a frame, the compositor still presents scroll and animation updates from the last snapshot. A slow handler never stalls scrolling.

## What runs on the compositor thread

These are the properties the compositor can change on its own, without the UI thread:

- **Scroll offset** of any scroll container that has its own layer. This includes momentum, overscroll and scrollbar fades. The UI thread learns the new offset at its next frame and updates virtual-list rows from it. A scroll handler marked `passive` (the default) cannot block the scroll, the same rule as Chromium's passive listeners. Only a non-passive handler forces scroll input through the UI thread.
- **Transform, opacity and clip-rect animations** on layered elements: transitions, springs, keyframes. The UI thread sends an animation description (curve, duration or spring parameters, start time), not per-frame values, the way Core Animation does. Interrupting or retargeting one is a new description that starts from the presented value and velocity.
- **Video surfaces and external textures**: a new frame from the producer thread is composited directly, with no UI thread work.
- **Animated images** on their own layers advance frames on the compositor.

Everything else changes on the UI thread: layout, text, colors, anything that needs a repaint. This includes non-compositor animations (color, size, corner radius), which run on the UI thread from the same frame clock.

## Scheduling

- **Request kinds.** A frame can be requested for a signal change (full pipeline), a compositor-only change (no UI-thread work), or a deadline (a timer or animation step). The scheduler merges requests so one frame serves all of them.
- **Deadlines.** The UI thread aims to commit within half a frame interval of begin-frame. If layout is still running at the deadline, the compositor presents what it has (scroll and animations stay smooth), and the commit lands on the next vsync.
- **Idle.** With no requests, both threads block on their OS wait primitive. The vsync source is stopped too, so an idle window costs zero wakeups.
- **Background windows** (minimized, hidden, fully covered) present nothing. Their compositor animations pause, and the UI thread keeps processing posts and timers.
- **Worker work** is prioritized. Visible-row shaping and decoding needed for the next frame come first, then prefetch for rows about to scroll in, then idle-time cache warming. Low-priority work is cancelled when its element unmounts.

## Single-thread mode

Tests, the headless runner and platforms without threads run the UI and compositor in one thread, in the same order and with the same snapshot. Output is identical, which makes golden-image and timing tests deterministic with a fake clock.

## Threads and the app

App code runs only on the UI thread. Signals are `!Send` handles, so they cannot leak to other threads. Other threads hold a `Waker`, which is `Send + Sync + Clone`, and post closures with it:

```rust
let waker = cx.waker();
std::thread::spawn(move || {
    for msg in network.incoming() {
        waker.post(move |cx| room.messages.push(msg)); // runs on the UI thread, batched into the next frame
    }
});
```

Posts are batched. Many posts between two frames cost one frame, and the order between posts from the same thread is kept. A `Waker` for a closed window drops its posts and reports that it is closed. It never panics.
