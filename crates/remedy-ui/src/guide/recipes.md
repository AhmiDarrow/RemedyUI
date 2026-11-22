# Recipes

Nine small apps, each a whole program in `crates/remedy-ui/examples/`. Each one keeps its
app in a `pub fn` beside `main`, so `crates/remedy-ui/tests/recipes.rs` can open the same app
on the in-memory host and check it through the accessibility tree, the way a screen reader
or an agent meets it. `cargo test -p remedy-ui --test recipes` runs them all.

Run a recipe from the repository root:

```text
cargo run -p remedy-ui --example chat_feed
```

The recipes are written the way apps on RemedyUI are: properties that change take a closure
or a signal (`w::text(ui, move || ..)`, `.disabled(move || ..)`), content that comes and goes
is a [`show`](crate::show), and gaps and padding are spacing tokens
([`Space`](crate::view::Space)) that follow the theme.

## How the tests check a recipe

Every test starts the recipe with [`TestApp::new`](crate::testing::TestApp::new), which runs
it on the in-memory host ([`HeadlessHost`](crate::headless::HeadlessHost)) on the CPU
renderer, and reads the window with [`Ui::reader_view`](crate::Ui::reader_view): each node's
role ([`a11y::Role`](crate::a11y::Role)), name, value and states, in reading order. It acts as
a person would: [`TestApp::press`](crate::testing::TestApp::press) clicks a control found by
role and name the way assistive technology does, [`TestApp::type_text`](crate::testing::TestApp::type_text)
and [`TestApp::key`](crate::testing::TestApp::key) type, and
[`TestApp::host`](crate::testing::TestApp::host) sends pointer presses and tray events and
moves the clock. Then it reads the tree again. This is the whole pattern, on a one-switch app:

```
use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;

let on = Signal::new(false);
let mut app = TestApp::window(move |ui, _| {
    ui.root().child(w::switch(ui, "Wi-Fi", on));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let states = |ui: &remedy_ui::Ui| {
    ui.reader_view().into_iter().find(|n| n.role == Role::Switch && n.name.as_deref() == Some("Wi-Fi")).map(|n| n.states)
};
assert_eq!(states(&ui), Some(vec!["not checked"]));
assert!(app.press(Role::Switch, "Wi-Fi"));
assert_eq!(states(&ui), Some(vec!["checked"]));
assert_eq!(on.get_untracked().ok(), Some(true));
```

## Chat feed

`examples/chat_feed.rs`. A conversation in a
[`chat_list`](crate::data::chat_list) with a composer under it.

- **Messages** live in one `Signal<Vec<Message>>`. Each message has a key (`id`) and a day.
  The list builds only the rows near the view, puts a header before each day's first
  message, and follows the newest message while the reader is at the bottom. Reading higher
  up, it stays put, and its own "Jump to latest" button counts what arrived.
- **Older history.** The list calls its `older` closure when the view nears the top. The
  recipe keeps earlier messages in an archive and prepends twenty at a time; the list keeps
  the view on what it showed.
- **The composer** is a [`text_area`](crate::widgets::text_area), so a message can run to
  several lines. [`TextField::submit_on_enter`](crate::TextField::submit_on_enter) makes Enter
  send and Shift+Enter break the line, as chats do.
  [`TextField::on_submit`](crate::TextField::on_submit) and the Send button share one closure:
  it ignores blank text, pushes the message, empties the field and calls
  [`ChatState::jump_to_latest`](crate::data::ChatState::jump_to_latest), so what you send is
  always in view.
- **Rows** mark their text with [`El::data_text`](crate::El::data_text) (names and what
  people wrote are not translated) and make it selectable.

The test finds the list and the newest message, sees that the list loaded older history
because three messages leave the view at the top, sets the composer's text and presses Enter,
then finds the new message and an empty composer. Pressing Send with nothing written adds
nothing.

## File explorer

`examples/file_explorer.rs`, run with a folder: `cargo run -p remedy-ui --example
file_explorer -- ~/Documents`. A [`tree_view`](crate::data::tree_view) of folders beside a
[`table`](crate::data::table) of the chosen folder, in a [`split`](crate::widgets::split).

- **Reading the disk.** `read_folder` lists a folder with `std::fs`, folders first, each group
  in [`natural_cmp`](crate::data::natural_cmp) order (`file2` before `file10`), and returns
  the error as text when the folder cannot be read. The window shows it with
  [`show`](crate::show), which builds the error's label only while there is one.
- **The tree loads on demand.** The root starts open with its folders. When another folder
  opens for the first time, the tree calls `load`, which answers with
  [`TreeState::set_children`](crate::data::TreeState::set_children).
- **The table follows the tree.** A binding on the tree's selection reads the chosen folder
  into the table's rows. The Size column sorts by the number of bytes, not the text it shows.
  Activating a folder in the table (Enter or a double click) selects it in the tree.

The test makes a temporary folder with a subfolder and two files, opens the explorer on it,
and checks the tree and the table by role and name, the order, and a size written for
people ("2.0 KB"). Clicking the subfolder in the tree, as a screen reader does, replaces
the table's rows with its file.

## Document viewer

`examples/document_viewer.rs`. A [`page_view`](crate::pages::page_view) with a
[`thumbnails`](crate::pages::thumbnails) rail and controls for pages and zoom.

- **Pages come from a [`PageSource`](crate::pages::PageSource).** A real viewer wraps its PDF
  or e-book engine in one. The recipe's `Report` draws twelve A5 pages itself: a colored band
  and dark bars where each line's words are, filled straight into the pixels of the tile it
  is asked for, at the scale asked. `render` runs on page workers off the UI thread and
  returns early when the tile is cancelled. `text` gives each page's lines as runs, which
  makes selection, Ctrl+C, search and the screen reader's page text work.
- **The view state** is a [`PageView`](crate::pages::PageView): fitted to the width at the
  start ([`Fit::Width`](crate::pages::Fit::Width)), with `current_page` and `zoom` as signals
  the labels read. The buttons call `go_to_page`, `zoom_in`, `zoom_out` and `set_fit`.
- **Layout.** The rail takes its own width and the pages the rest of the row; both scroll
  inside the window, so the layout needs no sizes of its own.

The test finds the document by name, reads "Page 1 of 12" and the first page's text as a
screen reader hears it, presses "Next page" and finds page 2 and its text, then presses
"Zoom in" and sees the zoom change.

## Settings page

`examples/settings_page.rs`. A [`nav_rail`](crate::nav::nav_rail) of sections beside the
chosen section's controls.

- **Settings are signals** in one `Copy` struct. Controls read and write them; nothing is
  copied back and forth.
- **They are kept.** Each signal is made with
  [`Signal::persist`](crate::reactive::Signal::persist) and a key. Every window's content is
  built with the app's persisted-state store provided
  ([`App::persist_store`](crate::App::persist_store), the `state` folder of the app's data
  folder), so a setting saves itself when it changes, off the UI thread, and comes back at
  the next start.
- **Sections.** Each section is a [`show`](crate::show) on the rail's selected id: it
  is built when chosen and disposed, with its bindings, when another is.
- **The theme** is a [`select`](crate::widgets::select) over the Remedy
  [`PALETTES`](crate::themes::PALETTES) and a High contrast switch. A binding turns them into
  a theme ([`themes::theme`](crate::themes::theme),
  [`high_contrast`](crate::themes::high_contrast)) and calls
  [`App::set_theme`](crate::App::set_theme), which restyles every window.
- **Dependent controls.** "Play a sound" and "Volume" pass a closure to
  [`El::disabled`](crate::El::disabled), so they are disabled, for the pointer, the keyboard
  and screen readers, while notifications are off.

The tests check that Appearance is the selected tab, switch to Notifications through the
rail, turn notifications off and find the sound switch and the volume slider disabled; then,
with the app's data folder in a temporary folder, start the app again and find notifications
still off.

## Menu bar

`examples/menu_bar.rs`. A document reader's File, Edit, View, Go and Help menus, as a desktop
app has them, from [`menu_bar`](crate::widgets::menu_bar).

- **One source for menus, shortcuts and the palette.** The reader's actions are
  [`Commands`](crate::nav::Commands); [`Commands::menu_item`](crate::nav::Commands::menu_item)
  makes each menu entry from its command: the label, the first shortcut (written in the
  person's language: "Strg+S" in German), whether it can run now, and the action. Closing the
  document disables Save and Close, and the menu shows them disabled the next time it opens.
- **Check and radio entries** show signals: Book Mode and Full Screen are
  [`MenuItem::check_signal`](crate::widgets::MenuItem::check_signal), Page Appearance and UI
  Scale are submenus ([`MenuItem::submenu`](crate::widgets::MenuItem::submenu)) of
  [`MenuItem::radio_signal`](crate::widgets::MenuItem::radio_signal). Ctrl+B and the menu
  toggle the same signal.
- **The keyboard reaches everything.** Every title and entry has an access key
  ([`MenuItem::mnemonic`](crate::widgets::MenuItem::mnemonic)): Alt+V opens View from
  anywhere (a text field included: the window offers these keys to the bar before focus
  gets them), F10 or a lone Alt reaches the bar, Left and Right move between menus, Right and
  Left open and close submenus, Escape closes one level, letters choose. The underlines show
  while Alt is held and in menus opened from the keyboard.
- **Edges.** Menus open below their title and flip above it, submenus open beside their entry
  and flip to the other side, at the window's edges; a menu taller than the window scrolls
  inside it; in a window too narrow for every title, the titles that do not fit go into a
  "…" menu at the end. In a right-to-left layout the bar and its menus mirror.

The test reads each title as a menu item that opens a menu, with its access key; opens View
with Alt+V and finds the shortcut text and the unchecked Book Mode; chooses Book Mode with B
and turns it back with Ctrl+B; goes into Page Appearance and picks Night by its letters;
moves from File to Edit with Right; and closes the document from File to find Save and Close
disabled there.

## Tray app

`examples/tray_app.rs`. A break reminder that lives in the system tray.

- **Lives in the tray.** [`App::keep_running`](crate::App::keep_running) keeps the app
  alive with no window showing, and
  [`WindowSpec::hide_on_close`](crate::WindowSpec::hide_on_close) makes closing the window
  hide it; [`Window::on_visibility`](crate::Window::on_visibility) tells the app when that
  happens, so it can remember whether its window was open for the next start.
- **The tray menu follows the state.** [`App::tray_dyn`](crate::App::tray_dyn) builds the
  tray from the paused signal, so the menu says "Pause reminders" or "Resume reminders" and
  changes when the signal does.
  [`App::on_tray`](crate::App::on_tray) handles a click on the icon and the menu's ids.
  The icon is drawn in code as premultiplied RGBA.
- **One instance.** `main` uses
  [`AppConfig::single_instance`](crate::AppConfig::single_instance); a second launch hands
  its arguments to [`App::on_second_instance`](crate::App::on_second_instance), which shows
  the running window. A command-line launch that needs an answer (`--stop` from an
  installer) uses [`AppConfig::single_instance_reply`](crate::AppConfig::single_instance_reply):
  the handler's return value becomes that launch's exit code.
- **The reminder** is [`App::after`](crate::App::after) scheduling itself every twenty
  minutes. When not paused, it marks a break as due, shows the window and sends a system
  notification with [`App::notify`](crate::App::notify): a
  [`Notification`](crate::host::Notification) with a "Break taken" button.
  [`App::on_notification`](crate::App::on_notification) takes the break when the button is
  pressed and shows the window when the notification itself is. Showing it again replaces
  it (same id), and where the platform has none the window alone does the reminding.
  Windows shows it as a toast (the engine registers the app's name for its toasts under the
  current user), Linux through the desktop's notification server, Android in its shade
  (with the app's icon in the status bar), a browser page through the Notification API after
  [`App::request_permission`](crate::App::request_permission)`(Permission::Notifications, ..)`.

The test reads the tray's tooltip and menu from the host, closes the window and sees it
hidden while the app runs on, moves the host's clock twenty minutes and finds the window
shown with the reminder and a notification sent, takes the break from the notification's
button, waits for the next one and presses "Break taken" in the window, pauses from the tray menu and sees the menu
and the window both say so, and quits from the menu.

## Sticky-note windows

`examples/sticky_notes.rs`. Each note is its own window.

- **The window.** [`WindowSpec::frameless`](crate::WindowSpec::frameless),
  [`always_on_top`](crate::WindowSpec::always_on_top) and
  [`persist_geometry`](crate::WindowSpec::persist_geometry) under the note's own key, so each
  note opens where it was left. Deleting a note also calls
  [`App::forget_geometry`](crate::App::forget_geometry).
- **Moving it.** With no title bar, the header is the handle:
  [`El::drag_region`](crate::El::drag_region) calls
  [`Window::begin_move`](crate::Window::begin_move) on a press, except on the header's buttons.
- **Saving.** Each note is a text file in the app's data folder
  ([`App::data_dir`](crate::App::data_dir)), written on every change, so closing the app
  loses nothing. A failed write shows its error in the note, through
  [`show`](crate::show).
- **More notes.** "New note" picks a number no note has yet and opens it beside the others.
- **Its icon.** [`WindowSpec::icon`](crate::WindowSpec::icon) gives each note window an icon
  of its own (in the note's color, say) in the taskbar and window switchers;
  [`Window::set_icon`](crate::Window::set_icon) changes it when the color does. Where
  windows have no icons of their own (macOS, Android, Wayland compositors without
  `xdg-toplevel-icon`), the app's icon shows.
- **Copying out.** A note that holds passwords keeps them in an ordinary multi-line field
  with [`TextField::copy_as_secret`](crate::TextField::copy_as_secret): Ctrl+C and Ctrl+X
  copy as secrets, kept out of clipboard history and cleared after the time given.

The test opens the notes in a temporary folder and checks the window is frameless and on
top, sets the note's text and reads the file back, presses the header and sees the host asked
to move the window, opens a second note, and deletes the first: its file and its window go.

## Audio meter

`examples/audio_meter.rs`. A [`level_meter`](crate::viz::level_meter) and a
[`speaking_ring`](crate::viz::speaking_ring) around an avatar, both following one
`Signal<f32>`.

- **Feeding the level.** A real app sets the level from its audio callback, through a
  [`Waker`](crate::reactive::Waker) or a timer reading the callback's latest peak. The recipe
  has no audio device: [`App::after`](crate::App::after) steps a generated test signal
  (speech-like bursts, then near silence) every 50 ms, and the window calls it a test signal.
- **The threshold** is a marker on the meter
  ([`MeterOptions::threshold`](crate::viz::MeterOptions::threshold)) the user drags or moves
  with the arrow keys; screen readers meet it as a slider.
- **No wasted work.** The visuals smooth what they show and stop scheduling frames while the
  level holds still; screen readers hear the level in twentieths, not fifty times a second.

The test checks the ring, the meter and its threshold slider by role and name, sees the level
moving, raises the threshold one step as a screen reader would and reads the new value, then
pauses the signal and sees the level fall to zero.

## Unlock screen

`examples/unlock_screen.rs`. A vault that stays covered until the master password opens it.

- **The lock.** [`lock_screen`](crate::widgets::lock_screen) covers the window with the
  unlock form and takes everything under it out of the frame and the accessibility tree.
  A binding on the `locked` signal adds the lock screen or removes it. A wrong password's
  message is a [`show`](crate::show).
- **The password** is a [`secret_input`](crate::widgets::secret_input) with a
  [`reveal_toggle`](crate::widgets::reveal_toggle): masked, held in a zeroizing buffer, never
  copied, never given to a screen reader, and handed to the platform only as a private
  input session.
  [`TextField::take_secret`](crate::TextField::take_secret) moves it out of the field, which
  empties, and it is wiped when dropped.
- **The check** is a function the app supplies. A real vault derives a key from the password
  (Argon2, scrypt) with [`App::background`](crate::App::background) and tries its file with
  it; `main` compares with the password given on the command line.
- **Locking when idle.** [`App::idle_lock`](crate::App::idle_lock) calls back after five
  minutes without real input; the callback clears any secret the app copied
  ([`App::clear_secret`](crate::App::clear_secret)) and locks. The window is also
  [capture protected](crate::WindowSpec::capture_protected).

The test checks that only the password field is in the tree while locked, types a wrong
password key by key and presses Enter, finds the error and that the typed text appears
nowhere in the tree, types the right one and presses Unlock, finds the vault, then moves the
clock past five minutes and finds it locked again.
