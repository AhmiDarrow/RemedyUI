# Accessibility

Every window has one semantics tree. Screen readers read it (UI Automation on Windows,
NSAccessibility on macOS, AT-SPI on Linux, Android's accessibility service), tests check it,
and agents operate through it ([`crate::guide::agents`]). Getting it right once serves all
three.

The tree is built from what elements declare. The engine's widgets declare it for you: a
button is a button named by its label, a switch says whether it is on, a password field
gives its label and never its text. Custom elements declare their own. Nothing is sent to a
platform until a screen reader asks, and after that only what changed.

This page covers reading the tree, describing custom elements, merging, announcements,
the keyboard, and the accessibility check.

## The tree as a screen reader meets it

[`Ui::reader_view`](crate::Ui::reader_view) lists the window as a screen reader meets it,
in reading order: each node's role, name, value, states and actions, with hidden content
left out and merged nodes folded. It is the most useful view for tests.

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;

let wifi = Signal::new(true);
let mut app = TestApp::window(move |ui, _| {
    ui.root()
        .child(w::heading(ui, 1, "Network"))
        .child(w::switch(ui, "Wi-Fi", wifi))
        .child(w::icon_button(ui, w::Icon::Settings, "Network settings", || {}));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };

let lines: Vec<String> = ui
    .reader_view()
    .into_iter()
    .filter_map(|n| n.name.map(|name| format!("{name} {:?} {}", n.role, n.states.join(" "))))
    .collect();
assert!(lines.contains(&"Network Heading(1) ".to_string()));
assert!(lines.contains(&"Wi-Fi Switch checked".to_string()));
// An icon button is named by the label it was given, not by its picture.
assert!(lines.contains(&"Network settings Button ".to_string()));
```

[`widgets::icon_button`](crate::widgets::icon_button) takes its label as a required
argument, and shows it as a tooltip too. A control with only a picture and no name is the
most common accessibility bug, so the API does not let you write one.

## Describing a custom element

An element built from scratch (a canvas, a custom control) declares what it is with
[`El::semantics`](crate::El::semantics), given a closure when it follows signals. `Semantics` and `Role` are in
[`a11y`](crate::a11y). Say what the element is (its role), what it
is called, its value, its states, and what can be done to it. The actions it lists are what
[`Ui::accessibility_action`](crate::Ui::accessibility_action), screen readers and agents can
ask for; [`El::on_click`](crate::El::on_click) handles `Click`.

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::view::A11yAction;
use remedy_ui::widgets as w;
use remedy_ui::a11y::{Action, Role, Semantics};

let stars = Signal::new(3u8);
let mut app = TestApp::window(move |ui, _| {
    // A star rating drawn on a canvas: on its own, a screen reader would find nothing here.
    let rating = w::canvas(ui, move |_cx| { /* draws stars.get() stars */ })
        .width(120.0)
        .height(24.0)
        .on_click(move || stars.update(|s| *s = *s % 5 + 1))
        .semantics(move || {
            Semantics::new(Role::Slider)
                .label("Rating")
                .value(format!("{} of 5 stars", stars.get()))
                .numeric(f64::from(stars.get()), 1.0, 5.0)
                .action(Action::Click)
        });
    ui.root().child(rating);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };

let Some(node) = ui.reader_view().into_iter().find(|n| n.name.as_deref() == Some("Rating")) else {
    unreachable!("the rating is in the tree")
};
assert_eq!(node.value.as_deref(), Some("3 of 5 stars"));
assert!(ui.accessibility_action(node.id, A11yAction::Click));
app.frames(1);
assert_eq!(stars.get(), 4);
```

Prefer the engine's widgets where one fits: a [`widgets::slider`](crate::widgets::slider)
already handles the arrow keys, increment and decrement, and its value.

## Merging compound rows

A chat message is an avatar, a name, a time and the text. A screen reader should move
message by message, not stop at each piece.
[`Semantics::merge_descendants`](crate::a11y::Semantics::merge_descendants) folds a row's
descendants into one node with a composed name:

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::a11y::{Role, Semantics};

let mut app = TestApp::window(|ui, _| {
    let message = w::row(ui)
        .gap(8.0)
        .semantics(Semantics::new(Role::ListItem).merge_descendants())
        .child(w::text(ui, "Sam").variant("strong"))
        .child(w::text(ui, "09:41").variant("muted"))
        .child(w::text(ui, "Lunch at noon?"));
    ui.root().child(message);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
assert_eq!(names.len(), 1, "one stop for the whole message: {names:?}");
assert!(names.iter().all(|n| n.contains("Sam") && n.contains("Lunch at noon?")));
```

Do not merge a row whose children have actions of their own (a Reply button): the merge
would fold them away. The accessibility check flags this.

## Announcements

Changes a person should hear without moving to them are live regions:
[`Live::Polite`](crate::a11y::Live::Polite) for new messages and status ("Saved"), waiting
until the person is idle, and [`Live::Assertive`](crate::a11y::Live::Assertive) for what
cannot wait (a call dropped, an error). [`widgets::toast`](crate::widgets::toast) announces
itself; for your own, put `.live(Live::Polite)` on the semantics of the element whose text
changes.

## The keyboard

Everything the pointer can do, the keyboard must do too.

- Controls built with the widgets, or with [`El::on_click`](crate::El::on_click), are
  focusable, and Enter or Space activates them.
- Tab and Shift+Tab move through focusable elements in reading order. Do not reorder focus
  by hand; order the elements instead.
- Dialogs, menus and popovers keep focus inside them
  ([`El::focus_scope`](crate::El::focus_scope)), put it on their first control, and give it
  back to what opened them when they close. [`widgets::modal`](crate::widgets::modal) and
  [`widgets::dialog`](crate::widgets::dialog) do all of this.
- Focus must show. The widget sheet styles `FOCUS_VISIBLE` for every widget; an app's own
  widget needs a rule for it (see [`crate::guide::theming`]).
- An element that handles pointer input but is deliberately not focusable (a resize grip)
  says how keyboard users do the same with
  [`El::keyboard_alternative`](crate::El::keyboard_alternative).

```
use remedy_ui::testing::TestApp;
use remedy_ui::input::{Key, KeyEvent};
use remedy_ui::widgets as w;

let mut app = TestApp::window(|ui, _| {
    ui.root()
        .child(w::button(ui, "Back", || {}))
        .child(w::button(ui, "Next", || {}));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let focused = |ui: &remedy_ui::Ui| ui.reader_view().into_iter().find(|n| n.focused).and_then(|n| n.name);

for expected in ["Back", "Next"] {
    app.key(KeyEvent::press(Key::Tab));
    assert_eq!(focused(&ui).as_deref(), Some(expected));
}
```

## The accessibility check

[`devtools::audit`](crate::devtools::audit) (feature `devtools`) checks a window the way an
accessibility review would and returns each [`Problem`](crate::devtools::Problem) with the
source line that built the element:

- controls with no name, and controls a screen reader cannot reach or activate;
- text below WCAG AA contrast against what is behind it;
- pointer handling with no keyboard way to do the same;
- focusable elements whose focus does not show;
- targets smaller than 24 × 24;
- content drawn past the window's edge.

The inspector (F12 in a debug build) runs it live on its Accessibility page, next to the
window as a screen reader meets it. Run it in tests too:

```
use remedy_ui::testing::TestApp;
use remedy_ui::devtools::{ProblemKind, audit};
use remedy_ui::widgets as w;

// A clickable square with no name and nothing to tell a screen reader.
let mut app = TestApp::window(|ui, _| {
    ui.root().child(ui.el("swatch").width(32.0).height(32.0).on_click(|| {}));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let problems = audit(&ui);
assert!(problems.iter().any(|p| p.kind == ProblemKind::Unnamed), "{problems:?}");

// The same control from the widget pack passes.
let mut app = TestApp::window(|ui, _| {
    ui.root().child(w::icon_button(ui, w::Icon::Plus, "Add color", || {}));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let problems = audit(&ui);
assert!(problems.is_empty(), "{:?}", problems.iter().map(|p| p.message()).collect::<Vec<_>>());
```

Every color pair the engine's widgets use meets WCAG contrast in all 24 Remedy themes (the
twelve palettes, each also in high contrast), and the engine's tests hold it there.

## Contrast and size

- Text colors come from [`Theme::derive`](crate::style::Theme::derive), balanced to meet
  WCAG contrast on the background. Use them, not fixed colors.
- [`widgets::MIN_TARGET`](crate::widgets::MIN_TARGET) is the smallest size of anything a
  pointer can press (24 px, WCAG 2.2); [`widgets::min_target`](crate::widgets::min_target)
  keeps an element at least that tall.
- The person's text size setting scales text; layouts must leave text room to grow. Previews
  at larger text scales show where they do not (see [`crate::guide::testing`]).
