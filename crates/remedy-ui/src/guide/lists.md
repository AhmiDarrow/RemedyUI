# Lists and virtual lists

Lists are where UI engines get slow: rebuilding every row when one changes, or building
100,000 rows to show twenty. RemedyUI's lists are keyed, so a change touches only the rows it
changes, and virtual, so only the rows near the viewport exist.

| You have | Use |
|---|---|
| A short list whose rows the app arranges itself | [`widgets::for_each`](crate::widgets::for_each) |
| A list people pick from with the pointer or arrow keys | [`widgets::list`](crate::widgets::list) |
| Many rows, or rows of unknown count | [`widgets::virtual_list`](crate::widgets::virtual_list) |
| Columns, sorting, editing cells | [`data::table`](crate::data::table) (feature `data`) |
| Folders and other nested items | [`data::tree_view`](crate::data::tree_view) |
| Icons or pictures in a grid | [`data::grid_view`](crate::data::grid_view) |
| Messages, newest at the bottom | [`data::chat_list`](crate::data::chat_list) |

`for_each`, `list` and `virtual_list` take the items as a `Signal<Vec<T>>` or a
[`SignalVec`](crate::reactive::SignalVec) (below), and follow it; the `data` views take a
`Signal<Vec<T>>`.

## Keyed children

[`widgets::for_each`](crate::widgets::for_each) keeps a parent's children in step with a list:
one element per key, built once. When the list changes, rows whose key stays keep their
element and whatever state it holds (a half-typed reply, a focus, a scroll position); rows
whose key goes are removed, and new keys are built. When the order changes, elements move.

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone)]
struct Task {
    id: u32,
    title: String,
}

let tasks = Signal::new(vec![
    Task { id: 1, title: "Water the plants".into() },
    Task { id: 2, title: "Call the bank".into() },
]);
let built = Rc::new(Cell::new(0));
let b = built.clone();
let mut app = TestApp::window(move |ui, _| {
    let column = w::column(ui).gap(4.0);
    let u = ui.clone();
    let list = w::for_each(&column, tasks, |t| t.id, move |t| {
        b.set(b.get() + 1);
        w::text(&u, t.title.clone())
    });
    ui.root().child(list);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
assert_eq!(built.get(), 2);

// A new task at the front: one row is built, the other two keep theirs.
tasks.update(|t| t.insert(0, Task { id: 3, title: "Buy bread".into() }));
app.frames(1);
assert_eq!(built.get(), 3);
let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
assert_eq!(names, ["Buy bread", "Water the plants", "Call the bank"]);
```

The key must be stable and unique: an id, never the position. A row's element is built from
the item it first had; if an item's content changes under the same key, have the row read it
from a signal, or change the key.

## Selectable lists

[`widgets::list`](crate::widgets::list) adds selection on top: clicking a row or moving with
the arrow keys selects it, and the chosen key is in a `Signal<Option<K>>` the app owns.
Pressing a selected row again, or Enter, calls `on_open`. Screen readers hear a list named
by its label, and each row by what it shows, its place in the list ("2 of 3") and whether it
is selected.

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::a11y::{Action, Role};
use remedy_ui::widgets as w;

let rooms = Signal::new(vec!["General".to_string(), "Music".to_string(), "Games".to_string()]);
let selected = Signal::new(None::<String>);
let opened = Signal::new(None::<String>);
let mut app = TestApp::window(move |ui, _| {
    let u = ui.clone();
    let list = w::list(
        ui,
        "Rooms",
        rooms,
        |name| name.clone(),
        move |name| w::text(&u, name.clone()),
        selected,
        move |name| opened.set(Some(name.clone())),
    );
    ui.root().child(list);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };

// Rows are named by what they show. Click "Music" the way a screen reader does.
let reader = ui.reader_view();
let Some(row) = reader.iter().find(|n| n.role == Role::ListItem && n.name.as_deref() == Some("Music")) else {
    unreachable!("listed")
};
assert_eq!(row.position, Some((2, 3)), "second of three");
assert!(ui.accessibility_action(row.id, Action::Click));
app.frames(1);
assert_eq!(selected.get().as_deref(), Some("Music"));

// Clicking the selected row again opens it.
assert!(ui.accessibility_action(row.id, Action::Click));
assert_eq!(opened.get().as_deref(), Some("Music"));
```

## Virtual lists

[`widgets::virtual_list`](crate::widgets::virtual_list) builds only the rows near the
viewport, so its cost depends on the window's height, not the list's length. Rows may be any
height: `estimate` places rows not measured yet, and each row's real height replaces its
estimate once shown, keeping the scroll position anchored to what you were looking at.
Scrolling runs on the compositor; the UI thread only builds rows as they come into range.

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;

let lines: Signal<Vec<String>> = Signal::new((1..=100_000).map(|i| format!("Line {i}")).collect());
let mut app = TestApp::window(move |ui, _| {
    let u = ui.clone();
    let log = w::virtual_list(ui, "Log", lines, 20.0, move |_, line| w::text(&u, line.clone()).height(20.0));
    ui.root().child(w::column(ui).height(300.0).child(log));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
app.frames(2);

let rows = ui.reader_view().into_iter().filter(|n| n.name.as_deref().is_some_and(|s| s.starts_with("Line "))).count();
assert!(rows > 0 && rows < 100, "a window's worth of rows exist, not 100,000 ({rows})");
```

The build closure gets each row's index and item. Keep rows cheap to build: they are built
again when they scroll back into range.

## Tables, trees and grids

The `data` feature ([`data`](crate::data)) has the heavier views, all virtual at any size:

- [`data::table`](crate::data::table): columns ([`data::Column`](crate::data::Column)) that
  sort, resize and move, selection with the keyboard, cells edited in place, and a
  [`data::TableState`](crate::data::TableState) holding what the person changed. Sorting
  large tables runs off the UI thread.
- [`data::tree_view`](crate::data::tree_view): nested items loaded as they open, with drag
  and drop.
- [`data::grid_view`](crate::data::grid_view): icons or pictures in rows that reflow with the
  width.
- [`data::property_grid`](crate::data::property_grid): names and values in groups, as in an
  inspector.
- [`data::chat_list`](crate::data::chat_list): messages grouped by day, kept at the bottom
  as new ones arrive, older ones loaded at the top without moving what you are reading.

The file explorer and chat feed recipes use them ([`crate::guide::recipes`]).

## Lists that change in place

A `Signal<Vec<T>>` hands out the whole list when it changes, and the keyed views compare
keys to find what moved. For data that changes often in small steps (a chat, a log, a
queue), a [`SignalVec`](crate::reactive::SignalVec) records each change as an operation
(push, insert, remove, move, set). Give one to `for_each`, `list` or `virtual_list` and they
apply the operations as they come: a push builds one row, a removal or a move builds nothing,
and nothing is compared.

```
use remedy_ui::reactive::SignalVec;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::Cell;
use std::rc::Rc;

let log = SignalVec::new(vec!["Started".to_string()]);
let built = Rc::new(Cell::new(0));
let b = built.clone();
let mut app = TestApp::window(move |ui, _| {
    let u = ui.clone();
    let lines = w::for_each(&w::column(ui), log, |line| line.clone(), move |line| {
        b.set(b.get() + 1);
        w::text(&u, line.clone())
    });
    ui.root().child(lines);
});
let _ = log.push("Connected".to_string());
let _ = log.push("Synced".to_string());
let _ = log.move_item(2, 0);
app.frames(1);
assert_eq!(built.get(), 3, "one row per push; the move built nothing");
let Some(ui) = app.ui() else { unreachable!("open") };
let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
assert_eq!(names, ["Synced", "Started", "Connected"]);
```

App code that consumes the changes itself (syncing a playlist to a player) takes them with a
[`VecReader`](crate::reactive::VecReader):

```
use remedy_ui::reactive::{SignalVec, VecDiff};

let queue = SignalVec::new(vec!["a.mp3".to_string(), "b.mp3".to_string()]);
let Ok(reader) = queue.reader() else { unreachable!("on the UI thread") };
let _ = reader.take_diffs(); // the first read is the whole list

let _ = queue.push("c.mp3".to_string());
let _ = queue.move_item(2, 0);
let Ok(diffs) = reader.take_diffs() else { unreachable!("on the UI thread") };
assert!(matches!(diffs.as_slice(), [VecDiff::Push(_), VecDiff::Move { from: 2, to: 0 }]));
```

## Rows that arrive, leave and move

[`widgets::for_each_with`](crate::widgets::for_each_with) is `for_each` with
[`ListTransitions`](crate::widgets::ListTransitions): rows that arrive play an `enter`
transition, rows that leave play an `exit` one (staying, inert, until it ends), and rows
that stay glide to their new places (`moves`). All of it is instant under reduced motion.

```
use remedy_ui::motion::{Pace, Transition};
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets::{self as w, ListTransitions};

let tasks = Signal::new(vec!["Write".to_string(), "Review".to_string(), "Ship".to_string()]);
let mut app = TestApp::window(move |ui, _| {
    let motion = ui.motion(Pace::Standard);
    let transitions = ListTransitions::new()
        .enter(Transition::fade(motion))
        .exit(Transition::fade(motion))
        .moves(motion);
    let u = ui.clone();
    let rows = w::for_each_with(&w::column(ui), tasks, |t| t.clone(), move |t| w::text(&u, t.clone()), transitions);
    ui.root().child(rows);
});
tasks.update(|t| t.retain(|name| name != "Review"));
app.frames(1);
let Some(ui) = app.ui() else { unreachable!("open") };
assert!(ui.is_animating(), "the leaving row fades and the one below glides up");
app.frames(120);
assert!(!ui.is_animating());
let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
assert_eq!(names, ["Write", "Ship"]);
```
