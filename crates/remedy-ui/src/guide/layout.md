# Layout

Every element is a box laid out by flex, grid or block rules, the same models CSS uses,
behind RemedyUI's own [`LayoutStyle`](crate::layout::LayoutStyle). Horizontal properties are
logical (start and end, not left and right), so a right-to-left window mirrors with no
change to the app. Edges are snapped to device pixels, so lines stay sharp at every scale.

Most layout is written with a few builder methods on [`El`](crate::El): `row`, `column`,
`gap`, `padding`, `width`, `height`, `grow`, `fill_width`, `align` and `justify`. Anything
else is one call to [`El::layout`](crate::El::layout), which edits the full style.

The samples run in a 480 × 360 window on the in-memory host ([`TestApp::window`](crate::testing::TestApp::window)) and check where things landed
with [`El::bounds`](crate::El::bounds), the box in window coordinates after the last frame.

## Rows and columns

A row puts children side by side in the text direction, a column top to bottom.
[`widgets::row`](crate::widgets::row) and [`widgets::column`](crate::widgets::column) are
elements already set up that way; `El::row` and `El::column` turn any element into one.

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

// Slots to keep the two elements in, to look at them after the frame.
let (sidebar, content) = (Rc::new(OnceCell::new()), Rc::new(OnceCell::new()));
let (s, c) = (sidebar.clone(), content.clone());
let mut app = TestApp::window(move |ui, _| {
    let side = ui.el("sidebar").width(120.0);
    let main = ui.el("content").grow(1.0);
    let _ = (s.set(side.clone()), c.set(main.clone()));
    ui.root().child(w::row(ui).gap(8.0).fill_width().height(300.0).child(side).child(main));
});
let (Some(side), Some(main)) = (sidebar.get().and_then(|e| e.bounds()), content.get().and_then(|e| e.bounds()))
else {
    unreachable!("both were laid out")
};
assert_eq!(side.width(), 120.0);
assert_eq!(main.min.x, 128.0, "after the sidebar and the gap");
assert_eq!(main.width(), 352.0, "the rest of the row");
```

`grow` shares the free space along the parent's axis: two children with `grow(1.0)` split
it evenly, `grow(2.0)` takes twice as much. Children also shrink when they do not fit;
set `shrink` to `0.0` through `El::layout` for one that must keep its size.

## Spacing from the theme

Gaps and padding come from the theme's spacing unit, so a denser or roomier theme changes
every screen at once. [`Spacing::s`](crate::style::Spacing::s) gives `n` units:

```
use remedy_ui::testing::TestApp;
use remedy_ui::core::Insets;
use remedy_ui::widgets as w;

let mut app = TestApp::window(|ui, _| {
    let space = ui.theme().spacing;
    let form = w::column(ui)
        .gap(space.s(2))
        .padding(Insets::all(space.s(4)))
        .child(w::text(ui, "Name"))
        .child(w::text(ui, "Email"));
    ui.root().child(form);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let unit = ui.theme().spacing.unit;
assert_eq!(ui.theme().spacing.s(4), unit * 4.0);
```

[`Insets`](crate::core::Insets) are logical too: `start` and `end`, mirrored in right to
left. `Insets::all` and `Insets::symmetric(vertical, horizontal)` cover most cases.

## Alignment

`align` places children across the axis ([`Align`](crate::layout::Align): `Start`,
`Center`, `End`, `Stretch`, `Baseline`); `justify` spreads them along it
([`Justify`](crate::layout::Justify): `Start`, `Center`, `End`, `SpaceBetween`,
`SpaceAround`, `SpaceEvenly`). A [`widgets::spacer`](crate::widgets::spacer) pushes what
follows it to the far end, which is how toolbars put actions on the right.

```
use remedy_ui::testing::TestApp;
use remedy_ui::layout::Align;
use remedy_ui::widgets as w;

let save = std::rc::Rc::new(std::cell::OnceCell::new());
let s = save.clone();
let mut app = TestApp::window(move |ui, _| {
    let button = w::button(ui, "Save", || {}).width(80.0);
    let _ = s.set(button.clone());
    let bar = w::row(ui)
        .fill_width()
        .align(Align::Center)
        .child(w::heading(ui, 2, "Settings"))
        .child(w::spacer(ui))
        .child(button);
    ui.root().child(bar);
});
let Some(b) = save.get().and_then(|e| e.bounds()) else { unreachable!("laid out") };
assert_eq!(b.max.x, 480.0, "the spacer pushed Save to the end of the row");
```

## Grids

A grid lays children in tracks. Set `display` to
[`Display::Grid`](crate::layout::Display::Grid) and give the column and row
[`Track`](crate::layout::Track)s: fixed (`Px`), a share of what is left (`Fr`), sized by
content (`MinContent`, `MaxContent`) or between two sizes (`MinMax`). Children fill the cells
in order unless they say where with `grid_column` and `grid_row` (start line, span).

```
use remedy_ui::testing::TestApp;
use remedy_ui::layout::{Display, Length, Track};

let cells = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
let c = cells.clone();
let mut app = TestApp::window(move |ui, _| {
    let grid = ui.el("keypad").layout(|l| {
        l.display = Display::Grid;
        l.width = Length::Px(300.0);
        l.columns = vec![Track::Fr(1.0), Track::Fr(1.0), Track::Fr(1.0)];
        l.rows = vec![Track::Px(40.0), Track::Px(40.0)];
        l.gap = (6.0, 6.0);
    });
    let keys: Vec<_> = ["7", "8", "9"].iter().map(|k| ui.el("key").text(*k)).collect();
    // "0" starts on the first line and spans all three columns of the second row.
    let zero = ui.el("key").text("0").layout(|l| l.grid_column = Some((1, 3)));
    c.borrow_mut().extend(keys.iter().cloned().chain([zero.clone()]));
    ui.root().child(grid.children(keys).child(zero));
});
let widths: Vec<f32> = cells.borrow().iter().filter_map(|e| e.bounds()).map(|b| b.width()).collect();
assert_eq!(widths, [96.0, 96.0, 96.0, 300.0]);
```

## Stacks and absolute positions

[`widgets::stack`](crate::widgets::stack) puts children on top of each other, each filling
the stack: a picture with a caption over it, a badge on an avatar. For a box at a given
place, set `position` to [`Position::Absolute`](crate::layout::Position::Absolute) and its
`inset` (logical, from the parent's padding box). Dialogs, menus and tooltips do not need
this: they go in the window's overlay layer ([`Ui::overlay`](crate::Ui::overlay)), placed
next to what opened them by [`widgets::place_near`](crate::widgets::place_near), which keeps
them inside the window.

## Scrolling

[`widgets::scroll`](crate::widgets::scroll) is a column that scrolls vertically.
[`El::scroll_x`](crate::El::scroll_x) scrolls sideways. Scrolling runs on the compositor
thread, so it keeps the display's rate even while app code is busy, and it costs the UI
thread nothing until new content comes into view.

A scroll container needs a bounded height to scroll in: give it one, or let it `grow` in a
column whose own height is bounded (the window is). `widgets::scroll` grows and lets its
minimum height go to zero for this reason. [`El::scroll_into_view`](crate::El::scroll_into_view)
and [`El::scroll_to`](crate::El::scroll_to) move it from code.

A shelf of cards can rest on card edges, as CSS scroll snapping does:
[`El::scroll_snap`](crate::El::scroll_snap) on the container (an axis, and `Mandatory` or
`Proximity`) and [`El::snap_align`](crate::El::snap_align) on each card (any descendant
outside a nested scroll container). It settles after a fling (on the point for where the
fling would have ended), a wheel gesture, a drag release, a key step or page, a focus move and
`scroll_to`, and stays on the same card when the layout changes; it glides there, or jumps
under reduced motion. [`El::scroll_to_exact`](crate::El::scroll_to_exact) does not snap.
`scroll_padding`, `snap_margin` and `snap_stop_always` are CSS's `scroll-padding`,
`scroll-margin` and `scroll-snap-stop: always`; `scroll_snap_proximity` (0.3 of the
viewport), `scroll_snap_motion` (a 250 ms decelerating tween) and `scroll_snap_wheel_idle`
(150 ms) tune it. An unhandled arrow key scrolls by
[`Ui::set_scroll_step`](crate::Ui::set_scroll_step) (40 px by default), Page Up and Page Down
by 90% of the viewport.

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;

let list = std::rc::Rc::new(std::cell::OnceCell::new());
let l = list.clone();
let mut app = TestApp::window(move |ui, _| {
    let rows = (1..=100).map(|i| w::text(ui, format!("Row {i}")).height(24.0));
    let scroller = w::scroll(ui).children(rows);
    let _ = l.set(scroller.clone());
    ui.root().child(w::column(ui).height(300.0).child(w::heading(ui, 2, "Log")).child(scroller));
});
let Some(b) = list.get().and_then(|e| e.bounds()) else { unreachable!("laid out") };
assert!(b.height() < 300.0, "the scroller keeps to the window; its 2,400 px of rows scroll");
```

## Right to left

Direction is inherited, like CSS `direction`. A window in Arabic or Hebrew is right to left
from the root ([`Ui::set_locale`](crate::Ui::set_locale) picks it from the locale), and a
single element can set [`Direction::Rtl`](crate::layout::Direction::Rtl) or `Ltr` for its
subtree. Rows run from the right, `start` padding and insets move to the right, and
alignment mirrors.

```
use remedy_ui::testing::TestApp;
use remedy_ui::layout::Direction;
use remedy_ui::widgets as w;

let first = std::rc::Rc::new(std::cell::OnceCell::new());
let f = first.clone();
let mut app = TestApp::window(move |ui, _| {
    let a = ui.el("first").width(100.0).height(20.0);
    let _ = f.set(a.clone());
    let row = w::row(ui)
        .fill_width()
        .layout(|l| l.direction = Direction::Rtl)
        .child(a)
        .child(ui.el("second").width(100.0).height(20.0));
    ui.root().child(row);
});
let Some(a) = first.get() else { unreachable!("built") };
assert!(a.is_rtl());
assert_eq!(a.bounds().map(|b| b.max.x), Some(480.0), "the first child sits at the right");
```

Draw directional things (arrows, progress) mirrored in painters when
[`PaintCx::rtl`](crate::PaintCx::rtl) is set. Text runs in its own direction whatever the
layout's: bidi is handled per paragraph.

## When the size changes

[`El::on_resize`](crate::El::on_resize) calls back with an element's new size after
layout, for content that changes with the room it has (a toolbar that moves actions into an
overflow menu). [`Ui::window_size`](crate::Ui::window_size) is the window's size. Prefer
layout rules (`grow`, `wrap`, `Fr` tracks) to code where they can express it: they need no
second frame.

## Costs

Layout is incremental. A change marks only the element it touched, and the walk up stops at
the first box whose size does not depend on its children: a fixed size, or a scroll view in
its scroll direction. Typing in a composer relays out the composer, not the window. An
element with a fixed `width` and `height` is the cheapest boundary there is.
