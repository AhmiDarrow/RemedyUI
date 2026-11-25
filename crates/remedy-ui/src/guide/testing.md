# Testing

RemedyUI apps are tested headless. The in-memory host, [`headless::HeadlessHost`]
(feature `headless`), implements the same host contract the Windows, macOS, Linux, Android
and web bridges do: windows are pixel surfaces, the clock moves only when the test moves it,
and input, input methods, the clipboard, file dialogs, the tray, timers and system settings
are scripted. A test runs the real app, start to finish, in milliseconds, on every platform.

[`testing::TestApp`](crate::testing::TestApp) puts an app on that host and uses it the way a
person or a screen reader does: it finds elements by role and accessible name, presses them
and types into them. Add the features for tests only:

```toml
[dev-dependencies]
remedy-ui = { version = "1", features = ["headless", "devtools"] }
```

`cargo remedy new` sets this up, with a first test and a preview.

[`headless::HeadlessHost`]: crate::headless::HeadlessHost

## Running the app

[`TestApp::new`](crate::testing::TestApp::new) runs the app a setup function opens, as `main`
would, through its first frame. It draws with the CPU renderer, so pixels are the same on
every machine, and keeps window positions in memory. What the app saves (persisted signals,
files in [`App::data_dir`](crate::App::data_dir)) goes to a private temporary folder that is
removed when the `TestApp` is dropped, never to the person's real data folder, unless the
test names a folder with [`AppConfig::data_dir`](crate::AppConfig::data_dir);
[`TestApp::data_dir`](crate::testing::TestApp::data_dir) says where. [`TestApp::window`](crate::testing::TestApp::window)
is the short form for one window built by a closure.

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

/// The app under test, as `main` runs it.
fn app(app: &App) {
    app.open(WindowSpec::new("Notes").size(400.0, 300.0), |ui, _| {
        ui.root().child(w::heading(ui, 1, "Notes"));
    });
}

let mut test = TestApp::new(AppConfig::new("Notes"), app);
let ids = test.host().window_ids();
let Some(window) = ids.first().and_then(|id| test.host().window(*id)) else { unreachable!("a window opened") };
assert_eq!(window.options().title, "Notes");
assert_eq!(window.size(), (400, 300, 1.0));
let Some(data) = test.data_dir() else { unreachable!("a test app has a data folder") };
assert!(data.starts_with(std::env::temp_dir()), "the test's own, not the person's");
```

[`TestApp::frames`](crate::testing::TestApp::frames) runs display refreshes: queued events,
due timers, then a frame for each window that asked for one, and the clock moves one refresh.
[`HeadlessHost::advance`](crate::headless::HeadlessHost::advance) (through
[`TestApp::host`](crate::testing::TestApp::host)) moves the clock without a frame.
Animations, timers and debounces happen exactly when the test says.

## Finding elements by what they are

Check a window the way a person meets it: by what things are and what they are called, not by
how the code built them. [`TestApp::find`](crate::testing::TestApp::find) takes a
[`Role`](crate::a11y::Role) and the accessible name a screen reader gets: a list row that
merges its contents is found by its composed name ("Ada, Hello there"), and what it merges is
not found on its own. [`TestApp::press`](crate::testing::TestApp::press) clicks it as
assistive technology does and runs a frame (a disabled control refuses);
[`TestApp::type_text`](crate::testing::TestApp::type_text) types into whatever has focus, key
by key. For values and states, [`TestApp::node`](crate::testing::TestApp::node) returns what a
screen reader hears of one element: its `value`, and `checked`, `selected`, `expanded`,
`disabled` and `focused`. [`TestApp::reader_view`](crate::testing::TestApp::reader_view)
(and [`Ui::reader_view`](crate::Ui::reader_view)) lists the whole window that way.

```
use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;

let volume = Signal::new(50.0);
let saved = Signal::new(String::new());
let mut test = TestApp::window(move |ui, _| {
    let name = w::text_input(ui, "Preset name");
    let n = name.clone();
    ui.root()
        .child(w::slider(ui, "Volume", volume, 0.0, 100.0, 5.0))
        .child(w::button(ui, "Mute", move || volume.set(0.0)))
        .child(name.el())
        .child(w::button(ui, "Save preset", move || saved.set(n.value())));
});
let value = |test: &TestApp| test.node(Role::Slider, "Volume").and_then(|n| n.value);

assert_eq!(value(&test).as_deref(), Some("50"));
assert!(test.press(Role::Button, "Mute"));
assert_eq!(value(&test).as_deref(), Some("0"));

let Some(field) = test.find(Role::TextInput, "Preset name") else { unreachable!("on screen") };
field.focus();
test.type_text("Quiet room");
assert!(test.node(Role::TextInput, "Preset name").is_some_and(|n| n.focused));
assert!(test.press(Role::Button, "Save preset"));
assert_eq!(saved.get(), "Quiet room");
```

A test written this way also checks that the window is accessible: a control a test cannot
find by role and name is one a screen reader cannot name either.

## Scripted input

[`TestApp::key`](crate::testing::TestApp::key) presses and releases a key, as a keyboard
does ([`KeyEvent::press`](crate::input::KeyEvent::press));
[`TestApp::key_press`](crate::testing::TestApp::key_press) and
[`TestApp::key_release`](crate::testing::TestApp::key_release) send the two halves on their
own, for keys held down. For everything else, the host queues input as the platform would
send it:

- [`HeadlessHost::send`](crate::headless::HeadlessHost::send) with any
  [`HostEvent`](crate::host::HostEvent): pointer presses and moves, keys, focus, close
  requests, tray clicks, system settings;
- [`HeadlessHost::compose_and_commit`](crate::headless::HeadlessHost::compose_and_commit): an
  input method composing and committing text;
- [`HeadlessHost::wheel`](crate::headless::HeadlessHost::wheel) and
  [`HeadlessHost::pinch`](crate::headless::HeadlessHost::pinch): scrolling and zooming;
- [`HeadlessHost::user_resize`](crate::headless::HeadlessHost::user_resize): the person
  dragging a window's edge;
- [`HeadlessHost::change_settings`](crate::headless::HeadlessHost::change_settings): dark
  mode, high contrast, text scale, reduced motion, locale.

```
use remedy_ui::core::Point;
use remedy_ui::host::HostEvent;
use remedy_ui::input::{Buttons, PointerEvent, PointerPhase};
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;

let clicks = Signal::new(0);
let mut test = TestApp::window(move |ui, _| {
    ui.root().child(w::button(ui, "Click", move || clicks.update(|c| *c += 1)).width(100.0).height(40.0));
});
let Some(window) = test.host().window_ids().first().copied() else { unreachable!("open") };

let at = Point::new(20.0, 20.0);
let down = PointerEvent::new(PointerPhase::Down, at).with_buttons(Buttons::PRIMARY);
test.host().send(HostEvent::Pointer(window, down));
test.host().send(HostEvent::Pointer(window, PointerEvent::new(PointerPhase::Up, at)));
test.frames(1);
assert_eq!(clicks.get(), 1);
```

## Fakes for the platform

Everything the app asks of the platform lands in the host, where the test can see it and
answer it:

| The app | The test |
|---|---|
| copies ([`App::copy`](crate::App::copy)) | [`HeadlessHost::clipboard`](crate::headless::HeadlessHost::clipboard) says what, and whether it was marked sensitive |
| opens a file dialog ([`App::file_dialog`](crate::App::file_dialog)) | [`HeadlessHost::answer_dialog`](crate::headless::HeadlessHost::answer_dialog) picks the files; [`HeadlessHost::dialogs`](crate::headless::HeadlessHost::dialogs) lists what was asked |
| sets a tray icon ([`App::set_tray`](crate::App::set_tray), [`App::tray_dyn`](crate::App::tray_dyn)) | [`HeadlessHost::tray`](crate::headless::HeadlessHost::tray) shows it; a `HostEvent::Tray` clicks it |
| gives a window its own icon ([`WindowSpec::icon`](crate::WindowSpec::icon), [`Window::set_icon`](crate::Window::set_icon)) | [`HeadlessWindow::options`](crate::headless::HeadlessWindow::options)`().icon` holds it |
| opens a link ([`App::open_url`](crate::App::open_url)) | [`HeadlessHost::opened_urls`](crate::headless::HeadlessHost::opened_urls) |
| drags out to other apps | [`HeadlessHost::drags_out`](crate::headless::HeadlessHost::drags_out), [`HeadlessHost::answer_drag`](crate::headless::HeadlessHost::answer_drag) |
| sizes caches from memory | [`HeadlessHost::set_total_memory`](crate::headless::HeadlessHost::set_total_memory) |
| quits ([`App::quit`](crate::App::quit)) | [`HeadlessHost::has_exited`](crate::headless::HeadlessHost::has_exited) |

```
use remedy_ui::a11y::Role;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::time::Duration;

let mut test = TestApp::window(|ui, window| {
    let app = window.app().clone();
    // Secret copies stay off clipboard history, and clear themselves after 30 seconds.
    ui.root().child(w::button(ui, "Copy password", move || app.copy_secret("hunter2", Duration::from_secs(30))));
});
assert!(test.press(Role::Button, "Copy password"));
assert_eq!(test.host().clipboard(), Some(("hunter2", true)), "copied, marked sensitive");

test.host().advance(Duration::from_secs(31));
test.frames(1);
assert!(test.host().clipboard().is_none_or(|(text, _)| text.is_empty()), "and cleared again");
```

## Checking what a frame did

A window where nothing changed should do no work at all.
[`Ui::last_frame_stats`](crate::Ui::last_frame_stats) says what the last frame did:
elements restyled, laid out and repainted, and whether layers or semantics were rebuilt.
[`HeadlessWindow::presents`](crate::headless::HeadlessWindow::presents) counts frames shown,
and [`HeadlessWindow::last_damage`](crate::headless::HeadlessWindow::last_damage) is what the
last one changed.

```
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::RefCell;
use std::rc::Rc;

let status = Signal::new("Idle".to_string());
let built = Rc::new(RefCell::new(Vec::new()));
let b = built.clone();
let mut test = TestApp::window(move |ui, _| {
    let (heading, text) = (w::heading(ui, 1, "Sync"), w::text(ui, status));
    b.borrow_mut().extend([heading.id(), text.id()]);
    ui.root().child(heading).child(text);
});
test.frames(1);
let Some(id) = test.host().window_ids().first().copied() else { unreachable!("open") };
let presents = test.host().window(id).map_or(0, |w| w.presents());

test.frames(10);
assert_eq!(test.host().window(id).map(|w| w.presents()), Some(presents), "idle frames present nothing");

status.set("Syncing…".into());
test.frames(1);
let Some(ui) = test.ui() else { unreachable!("open") };
let repainted = ui.last_frame_stats().repainted;
let [heading, text] = built.borrow()[..] else { unreachable!("two elements") };
assert!(repainted.contains(&text), "the status text was repainted");
assert!(!repainted.contains(&heading), "the heading was not");
```

`repainted` also lists the elements on the path from the root to what changed, whose
chunks hold it.

[`HeadlessWindow::surface`](crate::headless::HeadlessWindow::surface) holds the window's
pixels (RGBA, row by row), for the rare test that needs them. Presents copy only their
damage into it, so a test that checks pixels also checks that damage was complete.

## Without `TestApp`

`TestApp` is a thin layer over two public pieces, for tests that need more control: a
[`Runner`](crate::Runner) (what [`run`](crate::run) hands a platform's bridge) made with
[`Runner::with_geometry`](crate::Runner::with_geometry) and an in-memory
[`GeometryStore`](crate::GeometryStore), driven by a
[`HeadlessHost`](crate::headless::HeadlessHost) with
[`HeadlessHost::run_frames`](crate::headless::HeadlessHost::run_frames).

## Golden previews

A [`devtools::Preview`](crate::devtools::Preview) renders a piece of UI in every theme,
locale, window size and text scale you list, with the CPU renderer, so pixels are the same
on every machine. The inspector shows previews side by side
([`show_previews`](crate::show_previews)), and
[`devtools::check_golden`](crate::devtools::check_golden) turns each variation into a
golden-image test: the first run writes the images for review, later runs compare against
them and write `<name>.actual.png` beside any that differ. Set `REMEDY_UPDATE_GOLDEN=1` to
accept new renderings.

```
use remedy_ui::devtools::{GoldenError, Preview, check_golden};
use remedy_ui::widgets as w;

let preview = Preview::new("empty-inbox", |ui| w::empty_state(ui, "No messages", "New messages show here."))
    .themes(&["forest", "light"])
    .locales(&["en-US", "ar-EG"])
    .sizes(&[(360.0, 240.0)])
    .text_scales(&[1.0, 1.5]);
assert_eq!(preview.variations().len(), 8);

let dir = std::env::temp_dir().join(format!("remedy-guide-golden-{}", std::process::id()));
// The first run has nothing to compare with: it writes the images and says so.
assert!(matches!(check_golden(&preview, &dir), Err(GoldenError::Missing(_))));
// The next run matches them.
assert_eq!(check_golden(&preview, &dir), Ok(()));
let _ = std::fs::remove_dir_all(&dir);
```

Right-to-left locales lay out mirrored, and a larger text scale shows where a layout has no
room for text to grow. Commit the golden images with the tests.

## Previews declared beside the UI

`#[preview(...)]` (from [`devtools`](crate::devtools)) declares a preview next to the
function that builds it, with its themes, locales, sizes and text scales. It makes a
[`PreviewSpec`](crate::devtools::PreviewSpec) constant named after the function, for the
gallery, and a golden-image test against `tests/golden` (`test = false` leaves the test out,
`golden = "dir"` moves it). `#[previews]` on a module lists every preview in it as `all()`.

```
use remedy_ui::devtools::{golden_test, preview};
use remedy_ui::widgets as w;
use remedy_ui::{El, Ui};

#[preview(themes = ["forest", "light"], locales = ["en-US", "ar-EG"], size = (320, 120), test = false)]
fn sign_in_card(ui: &Ui) -> El {
    w::card(ui).child(w::heading(ui, 2, "Sign in")).child(w::button(ui, "Continue", || {}))
}

assert_eq!(SIGN_IN_CARD.preview().variations().len(), 4);
let dir = std::env::temp_dir().join(format!("remedy-guide-preview-{}", std::process::id()));
assert_eq!(golden_test(&SIGN_IN_CARD, &dir), Ok(()), "the first run writes the images for review");
let _ = std::fs::remove_dir_all(&dir);
```

## Accessibility in tests

[`devtools::audit`](crate::devtools::audit) runs the accessibility check on a window: unnamed
controls, low contrast, pointer-only handling, invisible focus, small targets and content
past the window's edge. An empty list in a test keeps it that way. See
[`crate::guide::accessibility`].
