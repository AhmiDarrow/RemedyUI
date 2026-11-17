# 2. State and elements

This part takes from three models:

- **SolidJS and Leptos:** components run once and signals update the bindings they feed, so there is no per-frame rebuild.
- **Jetpack Compose:** snapshot-consistent state and keyed lists.
- **Flutter:** separate element and render layers with relayout and repaint boundaries.

The aim is that the cost of a change matches the size of the change.

## The reactive graph

Nodes are **signals** (sources), **memos** (derived values), and **effects** (sinks that do work: write an element property, run app code).

- **Push-pull, glitch-free.** Setting a signal marks its direct dependents *dirty* and everything further downstream *check*. Nothing recomputes at that point.
  - When a value is read, or when effects flush, a memo in *check* asks its sources whether they really changed. It recomputes only if one did.
  - A memo whose new value equals its old one (`PartialEq`) stops the wave.
  - No effect ever sees half-updated state, and no memo runs twice in one flush.
- **Batching.** All changes in one event handler, post or animation step form one transaction, and effects run once at its end, in topological order.
- **Storage.** Each UI thread has one reactive runtime: a generational arena. Windows are owner scopes inside it, so a subtree can move between windows without its signals moving. A `Signal<T>` is a `Copy` handle of a runtime id, a 32-bit index and a 32-bit generation. It is `Send`, but its value is reachable only on its own UI thread.
  - A disposed signal's value is gone, and the engine does not invent one. `try_get` returns `None`. `get` treats it as the bug it is: it panics with where the signal was created, and the nearest error boundary contains it.
  - Dependency lists are small inline vectors, sized for the common case of 1–4 sources.
- **Ownership.** Every node has an owner: the component scope that created it. Unmounting a scope disposes its nodes, effects and child scopes, and runs clean-ups. Nothing leaks across mounts, and nothing has to be freed by hand.
- **Untracked reads, peeks and manual triggers** are available for the rare cases that need them.

## Stores: reactivity inside structures

Chat state is nested: rooms hold messages, which hold reactions. One signal per whole structure would update too much, and one signal per field is too much code. **Stores** give field-level tracking:

```rust
#[derive(Store)]
struct Message { id: MsgId, author: UserId, body: RichText, reactions: Vec<Reaction>, edited: bool }

let room: Store<Room> = Store::new(room);
room.messages().at(3).edited().set(true)?; // wakes readers of that field, of what contains it, of what it contains
```

`#[derive(Store)]` (crate `remedy-ui-macros`) generates typed field accessors. Each reads and writes through a path of fields and vector positions. A write wakes readers of that path, of the paths that contain it and of the paths inside it, and never its siblings. Lists that change shape often belong in a `SignalVec`; a store's vector is for nested data edited in place.

## Reactive collections

Diffing a 100,000-item list on every insert is how frameworks get slow. **`SignalVec<T>`** emits change operations instead of new copies: `Push`, `Insert(i)`, `Remove(i)`, `Move(from, to)`, `Set(i)`, `Replace`, `Clear`. Keyed list views consume the operations directly. A new message is one `Push` and one row mounts, with no diff. A plain `Vec` can still be given to `for_each`, which then falls back to a keyed diff (longest increasing subsequence for moves).

## Elements

Components are functions that run once to build elements and bindings. The **element** is the retained node the engine works on.

- **Data-oriented storage.** Elements live in one arena per window, with hot data in struct-of-arrays columns: parent, first child, next sibling, dirty flags, layout result, paint chunk id. Cold data (handlers, semantics, style source) sits in side tables keyed by element id. Tree walks touch little memory.
- **Kinds.** Built-in kinds (box, text, image, scroll, virtual list, canvas, layer host, …) are an enum handled without dynamic dispatch. Custom elements implement the `Element` trait (measure, layout, paint, hit test, semantics) and sit behind one boxed trait object.
- **Bindings.** `button(...).enabled(can_send)` creates an effect that writes the `enabled` property and sets the right dirty flag: style here, because the disabled state changes the look. A property set to a constant creates no effect at all.

## Dirty flags and boundaries

| Flag | Set when | Work it causes |
|---|---|---|
| `STYLE` | A token, state or style property changed | Re-resolve this element's style; may set `LAYOUT` or `PAINT` |
| `LAYOUT` | Size-affecting property, content or children changed | Lay out up to the nearest relayout boundary |
| `PAINT` | Visual-only property changed | Re-record this element's paint chunk |
| `COMPOSITE` | Transform, opacity, clip-rect or scroll offset changed on a layered element | Nothing on the UI thread; the compositor updates |
| `SEMANTICS` | Role, name, value, state or actions changed | Rebuild this element's AccessKit node |

- **Relayout boundary** (as in Flutter). An element whose size does not depend on its children stops `LAYOUT` from climbing the tree. That covers a fixed size, a tight constraint, or a scroll view in its scroll direction. Typing in the composer relays out the composer, not the window.
- **Repaint boundary.** An element with its own layer stops `PAINT` from forcing the parent's chunk to re-record. Scroll containers, video, animated elements, and anything the app marks `isolate_paint()` get layers. The layerization rules are in [04-rendering.md](04-rendering.md).

## Component model features

- **Context.** Any component can provide a typed value to its subtree and read the nearest one. Theme, locale, text direction, density and app services flow through context. There are no globals.
- **Slots and children.** Components take children and named slots (a `header`, a `trailing` icon) as element builders.
- **Error boundaries.** `catch_errors(fallback, child)`. A panic or returned error inside the child subtree, whether in building, a handler, an effect or a canvas closure, unmounts that subtree and shows the fallback. The error and the element path go to the app's error hook. The rest of the window carries on.
- **Suspense.** `suspense(placeholder, child)` shows the placeholder while any resource read inside the child is still loading, such as an image, a search result or a message page. It switches without a flash: there is a minimum time before showing the placeholder, and a minimum time to keep it shown.
- **Resources.** `Resource::new(source_signal, |key| fetch(key))` runs the fetch off the UI thread through the `Executor` trait. The default executor is the worker pool. An app with its own async runtime implements `Executor` for it; RemedyUI itself depends on no async runtime. A resource cancels on key change or unmount, and exposes `loading`, `ready` and `error` as signals.
- **Navigation.** `nav_stack` keeps each screen's scope alive while it is on the stack, so state and scroll position survive going back. It handles the Android back button and Escape, and plays shared-element transitions.
- **Persistence.** A signal can be marked `persist("key")`. The app's storage backend saves it on change, debounced, and restores it on mount: window size, split positions, scroll anchors, drafts.

## Why this is fast

| Operation | Cost |
|---|---|
| One new chat message | One `Push`, one row mounted, one layout of that row plus the list's anchor, one chunk recorded |
| Hovering a button | One state bit, one style re-resolve from cache, one chunk re-recorded |
| Theme accent changes | Only elements that read `accent` (tracked through the token signal) restyle |
| Scrolling | No UI-thread work until a new row enters the prefetch window |
| Nothing happening | Nothing |
