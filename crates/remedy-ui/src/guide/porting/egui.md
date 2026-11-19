# Moving from egui

FreeSpeak is the egui app moving over. This guide is for anyone with an egui or eframe app.

The big change: egui runs your whole UI function every frame. RemedyUI runs it once. It
builds a tree of elements, and signals update the parts of that tree that read them. When
nothing changes, nothing runs.

## Concept map

| egui | RemedyUI |
|---|---|
| `eframe::run_native` and `App::update` | [`crate::run`] with a setup closure that opens windows ([`crate::App::open`]) |
| The UI function, run every frame | A builder function, run once per window or row |
| `ui.label("…")`, `ui.button("…")` | [`crate::widgets::text`], [`crate::widgets::button`] return an [`crate::El`] you place with `.child` |
| `if ui.button(..).clicked() { .. }` | The handler passed to the button: `button(ui, "Save", move \|\| save())` |
| Fields on your `App` struct | [`crate::reactive::Signal`]s (`Copy` handles), captured by the closures that need them |
| A value computed in `update` | [`crate::reactive::Memo`] |
| `ctx.request_repaint()` | Nothing: writing a signal schedules exactly the work it needs |
| `ui.horizontal`, `ui.vertical` | [`crate::widgets::row`], [`crate::widgets::column`], flex and grid layout |
| `ScrollArea` | [`crate::widgets::scroll`]; [`crate::widgets::virtual_list`] for long lists |
| `ScrollArea::show_rows` | [`crate::widgets::virtual_list`], with real row heights and anchoring |
| `ctx.set_visuals(Visuals { .. })` | A [`crate::style::Theme`] of tokens and a [`crate::style::StyleSheet`] |
| `egui::Window` | A real window: [`crate::App::open`] |
| `Painter` in a custom widget | [`crate::El::paint`] or [`crate::widgets::canvas`] |
| Keyboard shortcuts read with `ctx.input` | [`crate::nav::Commands`]: rebindable, shown in menus and the palette |
| `ui.add_enabled(false, ..)` | `.disabled(true)`, or `.disabled(move \|\| ..)` to follow signals |
| `if show { ui.label(..) }` | [`crate::show`]: the content is built while a condition holds, removed when it stops |
| `ui.spacing().item_spacing` | Typed tokens: `.gap(Space::S2)`, `.padding(Space::S4)` ([`crate::style::Space`]) |
| `ctx.animate_value_with_time` | [`crate::El::animate`] on the frame clock, or a painter reading [`crate::PaintCx::time`] |
| The egui demo app, to see widgets in isolation | [`crate::devtools::preview`] previews, also golden-image tests |

## A counter

egui asks every frame whether the button was clicked, and redraws everything:

```text
struct Counter { count: u32 }

impl eframe::App for Counter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(format!("Clicked {} times", self.count));
            if ui.button("Add one").clicked() {
                self.count += 1;
            }
        });
    }
}
```

RemedyUI builds the window once. The count is a signal; the label's text is bound to it, so a
click relays out that one label and nothing else:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{AppConfig, Ui};
# fn names(ui: &Ui) -> Vec<String> {
#     ui.reader_view().into_iter().filter_map(|n| n.name).collect()
# }
# use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn counter(app: &App) {
    app.open(WindowSpec::new("Counter"), |ui, _window| {
        let count = Signal::new(0u32);
        let label = w::text(ui, move || format!("Clicked {} times", count.get()));
        let add = w::button(ui, "Add one", move || count.update(|c| *c += 1));
        ui.root().child(w::column(ui).gap(8.0).child(label).child(add));
    });
}

// `remedy_ui::run(AppConfig::new("Counter"), counter)` opens it for real. Here it runs on
// the in-memory host and is checked through what a screen reader sees.
let mut app = TestApp::new(AppConfig::new("Counter"), counter);
let Some(ui) = app.ui() else {
    unreachable!("the window is open")
};
assert!(names(&ui).contains(&"Clicked 0 times".to_string()));
assert!(app.press(Role::Button, "Add one"));
assert!(names(&ui).contains(&"Clicked 1 times".to_string()));
```

The builder closure ran once. Clicking did not run it again.

## State: from `&mut self` to signals

In egui, state is fields on your app struct, and every widget borrows `&mut self` while the
frame runs. In RemedyUI, each piece of state is a [`crate::reactive::Signal`]. A signal is a
`Copy` handle, so any number of closures can capture it with `move`. There is no
`Rc<RefCell<…>>` and no borrow of the whole app.

Derived values that egui recomputes every frame become memos. A memo recomputes only when a
signal it read changed, and stops the update when its value is the same as before:

```
use remedy_ui::reactive::{Memo, Signal};

let items = Signal::new(vec![3u32, 4, 5]);
let total = Memo::new(move || items.with(|v| v.iter().sum::<u32>()));
assert_eq!(total.get(), 12);
items.update(|v| v.push(8));
assert_eq!(total.get(), 20);
```

See [`crate::guide::signals`] for effects, batching and lifetimes.

## No repaint requests

egui apps call `ctx.request_repaint()` (or repaint on a timer) to show changes that happen
outside input: a download finished, a message arrived. FreeSpeak repaints every 40 ms for this.

In RemedyUI, set a signal. The window wakes, runs the bindings that read it, and draws one
frame. Work from another thread comes back with [`crate::App::background`], which runs a
closure on a worker and hands its result to the UI thread:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{AppConfig, Ui};
# fn names(ui: &Ui) -> Vec<String> {
#     ui.reader_view().into_iter().filter_map(|n| n.name).collect()
# }
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    let status = Signal::new("Loading…".to_string());
    // Slow work off the UI thread; the result arrives on the UI thread.
    app.background(|| (1..=10u64).product::<u64>(), move |n| status.set(format!("Done: {n}")));
    app.open(WindowSpec::new("Status"), move |ui, _| {
        ui.root().child(w::text(ui, status));
    });
}

let mut app = TestApp::new(AppConfig::new("Status"), app);
let Some(ui) = app.ui() else {
    unreachable!("the window is open")
};
for _ in 0..500 {
    if names(&ui).contains(&"Done: 3628800".to_string()) {
        break;
    }
    std::thread::sleep(std::time::Duration::from_millis(2));
    app.frames(1);
}
assert!(names(&ui).contains(&"Done: 3628800".to_string()));
```

A window that is hidden or minimized keeps running: tray menus, timers and background work
do not wait for a repaint.

## Layout

egui places widgets as the frame runs, so a widget's size is known one frame late. RemedyUI
lays out the whole tree before it paints, in the same frame.

```text
ui.horizontal(|ui| {
    ui.label("Name");
    ui.text_edit_singleline(&mut self.name);
});
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::layout::Align;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    app.open(WindowSpec::new("Profile"), |ui, _| {
        let name = w::text_input(ui, "Name");
        let row = w::row(ui).gap(8.0).align(Align::Center).child(w::text(ui, "Name")).child(name.el().grow(1.0));
        ui.root().child(row);
    });
}
# let mut app = TestApp::new(AppConfig::new("Profile"), app);
# let Some(ui) = app.ui() else { unreachable!("open") };
# assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Name")));
```

The field keeps its own text. Read it with [`crate::TextField::value`], or react to edits
with [`crate::TextField::on_change`]. There is no `&mut String` tied to a frame.
[`crate::guide::layout`] covers grids, sizes and scrolling.

## Visuals become themes

egui has one global `Visuals` (FreeSpeak also keeps a global palette behind a lock). In
RemedyUI a window has a [`crate::style::Theme`]: colors by role, the type scale, spacing,
radii and motion. A [`crate::style::StyleSheet`] says how each widget looks in each state,
using only those tokens. The twelve Remedy palettes are in [`crate::themes`].

```
use remedy_ui::style::{States, StylePatch};
use remedy_ui::{AppConfig, themes};

let theme = themes::ThemeChoice::Palette("forest".into()).resolve(None, false);
let config = AppConfig::new("Chat").theme(theme, |t| {
    // The Remedy widget sheet, plus one rule of the app's own.
    let mut sheet = themes::sheet(t);
    sheet.rule("button", Some("send"), States::NONE, StylePatch::new().background(t.colors.accent));
    sheet
});
# let _ = config;
```

A widget picks its rule by name and variant: `button(..).variant("send")`. Custom painters
read the theme from [`crate::PaintCx::theme`], never from a global. See
[`crate::guide::theming`].

## Custom widgets

egui custom widgets allocate a rect and draw with a `Painter`. In RemedyUI, give an element a
size and a painter. The painter records once and again only when a signal it read changes:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::core::{Point, Vec2};
use remedy_ui::reactive::Signal;
use remedy_ui::view::TextPaint;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    app.open(WindowSpec::new("Level"), |ui, _| {
        let level = Signal::new(0.4f32);
        let meter = w::canvas(ui, move |cx| {
            let p = TextPaint { size: 14.0, weight: 400.0, color: cx.theme.colors.text, mono: true };
            let at = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
            cx.text(&format!("{:.0}%", level.get() * 100.0), at, Vec2::new(0.5, 0.5), p);
        });
        ui.root().child(meter.width(120.0).height(40.0));
    });
}
# let mut app = TestApp::new(AppConfig::new("Level"), app);
# assert!(!app.host().window_ids().is_empty());
```

Shapes go into [`crate::PaintCx::list`], a display list of [`crate::paint`] items
([`crate::paint::Brush`], [`crate::paint::Item`], [`crate::paint::Path`]). For audio meters and rings,
use [`crate::viz`] before drawing your own.

## Showing, hiding and moving

egui shows a widget by calling it in the frame and hides it by not calling it. In RemedyUI,
[`crate::show`] builds its content while a condition holds and removes it (with its
bindings and signals) when the condition stops. It can play a transition as content comes
and goes, and [`crate::El::animate`] moves a property on the frame clock; both are instant
under reduced motion:

```text
if self.show_details {
    ui.label("Connected to 3 peers");
}
if ui.button("Details").clicked() {
    self.show_details = !self.show_details;
}
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::a11y::Role;
use remedy_ui::motion::{Motion, Spring, Transition};
use remedy_ui::reactive::Signal;
use remedy_ui::show;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};
use std::time::Duration;

fn app(app: &App) {
    app.open(WindowSpec::new("Peers"), |ui, _| {
        let details = Signal::new(false);
        let fade = Transition::fade(Motion::Spring(Spring::smooth(Duration::from_millis(200))));
        let u = ui.clone();
        let info = show(ui, details, move || w::text(&u, "Connected to 3 peers")).enter(fade).el();
        let toggle = w::button(ui, "Details", move || details.update(|d| *d = !*d));
        ui.root().child(toggle).child(info);
    });
}
let mut app = TestApp::new(AppConfig::new("Peers"), app);
assert!(app.find(Role::Label, "Connected to 3 peers").is_none());
assert!(app.press(Role::Button, "Details"));
assert!(app.find(Role::Label, "Connected to 3 peers").is_some());
```

What the egui demo app gives you (each widget on its own, in every state) is a preview: a
function marked with [`crate::devtools::preview`], shown in the devtools gallery and checked
against golden images in every theme and locale it lists. See [`crate::guide::testing`].

## Pop-outs are real windows

egui draws `egui::Window` inside the one OS window. RemedyUI opens real windows with
[`crate::App::open`]. A part of a window that pops out and comes back is
[`crate::movable`]. Each window can persist its place ([`crate::WindowSpec::persist_geometry`]),
stay on top, or be frameless.

## What egui could not do, and now works

These are the egui limits FreeSpeak works around today. Remove the workarounds when you move.

| FreeSpeak works around | Remove it: RemedyUI does this |
|---|---|
| Repainting every 40 ms | Nothing runs while nothing changes |
| Widths measured one frame and used the next | Layout runs before paint in the same frame |
| A hand-written list with a height map | [`crate::widgets::virtual_list`] and [`crate::data::chat_list`] |
| Keep-alive repaints while hidden | The runtime does not depend on the window being visible |
| Forked `epaint` for hinting | Hinted grayscale and LCD text |
| Text runs as separate widgets | One rich paragraph per text ([`crate::view::RichText`]) |
| A Copy chip on every message | Labels are selectable; selection spans elements |
| No right-to-left text | Full shaping and bidi; layouts mirror |
| No screen reader support | Every widget declares semantics; screen readers connect on every desktop |
| `catch_unwind` around GPU start-up | A backend ladder with the CPU renderer as the floor |

## Pitfalls

- **Don't rebuild to update.** Moving code from `update` into a builder and then clearing
  and rebuilding children on every change brings the egui cost back. Bind the property that
  changes (`.text(move || ..)`, `.disabled(signal)`, [`crate::El::bind`]) instead, and use
  [`crate::show`] for content that comes and goes.
- **Read signals inside a closure.** `w::text(ui, format!("{}", count.get()))` reads once,
  at build time, and never updates. Pass `move || format!("{}", count.get())` instead.
- **Lists need keys.** [`crate::widgets::for_each`] and [`crate::widgets::list`] take a key per
  item. A row with the same key keeps its element and state when the list changes.
- **There is no `Response`.** Hover, focus and clicks are handlers on the element:
  [`crate::El::on_click`], [`crate::El::on_hover`], [`crate::El::on_focus_change`].
- **Shortcuts belong to commands.** Register them in [`crate::nav::Commands`] and attach it to
  the window, so people can rebind them and menus show them.
