# 3. Layout

The aim is layout that web and Flutter developers both find familiar, that is correct for every script and direction, and that costs time only where something changed.

## One protocol, several algorithms

Every element takes part in one layout protocol. It is the layout half of the `Element` trait, shown on its own here:

```rust
trait Layout {
    /// Size for these constraints. Pure: same inputs, same output, so it can be cached.
    fn measure(&self, cx: &mut MeasureCx, c: Constraints) -> Size;
    /// Place children inside the final size.
    fn arrange(&self, cx: &mut ArrangeCx, size: Size);
    /// Optional: intrinsic sizes and the first baseline, for alignment and grids.
    fn intrinsic(&self, cx: &mut MeasureCx, axis: Axis, other: Option<f32>) -> Intrinsic {
        cx.intrinsic_from_measure(self, axis, other)
    }
}
```

`Constraints` carry a min and max size per axis, as in Flutter, plus the available-space modes CSS needs: definite, min-content and max-content. Built-in layouts use this protocol:

| Layout | Engine | Notes |
|---|---|---|
| `row`, `column`, `wrap` | Flexbox (taffy) | `gap`, `grow`, `shrink`, `basis`, `align`, `justify`, baseline alignment |
| `grid` | CSS Grid (taffy) | Tracks, `fr`, `minmax`, named areas, auto-placement, subgrid when taffy has it |
| `stack` | RemedyUI | Children layered, each with alignment and insets. The common overlay case without absolute positioning |
| `flow` | Block and inline (taffy + text) | Documents and Markdown: paragraphs, lists, floats for images |
| `virtual_list`, `virtual_grid` | RemedyUI | See below |
| `split`, `dock` | RemedyUI | Resizable panes with min and max sizes, collapse, and tear-off to a window |
| `anchored` | RemedyUI | Overlays placed against an element or point, with flip and shift to stay on screen |
| Custom | App | Implement the `Element` trait's layout methods; it gets the same caching and boundaries |

Taffy is used through RemedyUI's own `Style` types, never exposed, so the layout engine can change without breaking apps.

## Logical directions

All layout properties are logical: `start` and `end`, `inline` and `block`, never left and right. In a right-to-left locale, rows, padding, alignment, scroll origin and directional icons all mirror without app changes. Physical properties exist for the rare things that must not mirror, such as a media player's timeline.

## Units and snapping

- **Units.** Layout works in logical pixels (dp). The window's scale factor (OS DPI × the app's zoom) converts them to device pixels.
- **Snapping.** After layout, every edge is snapped to the device-pixel grid: rounded on position, with size derived from the snapped edges so neighbours never overlap or gap. Borders and hairlines snap to whole device pixels, so a 1 dp line at 150% is one crisp pixel or two, never a blurry 1.5.
- **Text** snaps its baseline, not its box, so text sits on whole pixels when hinting is on.

## Caching and boundaries

- **Caches.** Each element caches its last `measure` result, keyed by its constraints and content version. Layout re-runs an element only when it is `LAYOUT`-dirty or its constraints changed.
- **Relayout boundaries** stop dirtiness from climbing: fixed size, tight constraints, or a scroll axis (see [02](02-state-and-elements.md)).
- **Parallel layout.** Sibling subtrees that are relayout boundaries can lay out in parallel on the worker pool when there is enough work, as Servo's layout does. The UI thread copies each subtree's layout inputs into a detached snapshot, workers lay the snapshots out, and the UI thread writes the results back. Workers never touch the live tree. This happens at a window resize, a theme change or first build. Small updates stay on the UI thread, where they are cheaper.
- **Same frame.** Layout always completes before paint in the same frame. Nothing is measured one frame late.

## Virtualization

`virtual_list` and `virtual_grid` keep lists of any length cheap. The protocol is the one Jetpack Compose's lazy layouts and the web's virtual scrollers converged on, with the anchoring browsers do:

- **Items** come from a `SignalVec` or an item provider: a count and a key and build function per index. Only items within the viewport plus a prefetch margin are mounted. The margin is about one viewport each way, adapted to scroll speed.
- **Heights.** A height cache keyed by item key keeps measured heights. Unmeasured items use an estimate, either the running average or one from the app. When an estimate is corrected, the anchor keeps the visible content still.
- **Anchoring.** The anchor is either an edge (`stick_to_end` for chat; it releases when the user scrolls up and re-engages at the end) or the first fully visible item. Insertions, removals and height changes above the anchor shift the scroll offset by the same amount, so nothing in view jumps.
- **Width changes** invalidate only the rows in view and the prefetch margin. Others re-measure as they come into view. Nothing is cleared wholesale.
- **Prefetch on workers.** Text for rows about to enter is shaped on the worker pool ahead of time, so a fling never waits on shaping.
- **Sticky headers** (day separators, section titles) stick within their section.
- **Jump to item** scrolls to any index or key, even one never measured: it estimates, mounts around the target, then corrects with the anchor.
- **Selection, find and accessibility** see the whole list, not only mounted rows. The list exposes item count and position to screen readers, and find and select-all work on the data model.

## Responsive layout

- **Window size classes**: compact (< 600 dp), medium (600–840), expanded (840–1200), large (> 1200). These are signals in context, as in Material's guidance. A component can switch structure by class: FreeSpeak's three-pane layout collapses into stacked screens on a phone.
- **Container queries.** An element can read its own available size and choose a layout, so components adapt to where they are placed, not only to the window.
- **Insets.** Safe areas, system bars and the on-screen keyboard are signals in context. Scroll views move focused inputs above the keyboard automatically.

## Debugging layout

The inspector draws each element's box, padding, gap, baseline and constraints. Layout warnings point at the element's source location: a child overflowing a fixed parent, an unbounded height inside a scroll, a flex child that can never shrink. They appear in debug builds instead of being silent. Release builds clip overflows and log once.
