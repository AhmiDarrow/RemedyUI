# Moving from React

Most Remedy apps today are React, in Tauri or Capacitor: SecretSticky, SecretFolder, Alex's
Craft Calc, GuitarRemedy, VoiceRemedy and the RemedyAI desktop. This guide maps what you know
from React onto RemedyUI.

The closest relatives of RemedyUI are SolidJS and Leptos, not React. A component runs once.
It builds elements and wires signals to the properties that show them. There is no virtual
DOM, no re-render and no diff: a state change updates exactly the bindings that read it.

## Concept map

| React | RemedyUI |
|---|---|
| A function component returning JSX | A Rust function returning an [`crate::El`], built with builder methods |
| Re-rendering on state change | No re-render: the function ran once; bindings update |
| Props | Function arguments (plain values, [`crate::reactive::Signal`]s, closures) |
| `useState` | [`crate::reactive::Signal::new`] |
| `useMemo` | [`crate::reactive::Memo::new`] |
| `useEffect` | [`crate::El::bind`] (lives as long as its element) or [`crate::reactive::Effect::new`] |
| `useEffect` cleanup function | [`crate::reactive::on_cleanup`] inside the effect, or [`crate::El::on_removed`] |
| Dependency arrays | None: an effect re-runs when any signal it read changes |
| `useRef` to a DOM node | The `El` itself: a cheap handle you clone and keep |
| `useRef` for a mutable value | A `Signal` you read with `get_untracked`, or `Rc<Cell<_>>` |
| `key` on list items | The key function of [`crate::widgets::for_each`] and [`crate::widgets::list`] |
| `onClick`, `onChange` | [`crate::El::on_click`], [`crate::TextField::on_change`] |
| Controlled inputs | A [`crate::TextField`] keeps its text; read it or set it |
| CSS classes and variables | Theme tokens ([`crate::style::Theme`]), a [`crate::style::StyleSheet`] and variants |
| Inline `style={{…}}` | [`crate::El::style`] with a [`crate::style::Style`] of tokens or a [`crate::style::StylePatch`]: a value, a signal or a closure |
| `{cond && <Banner/>}` | [`crate::show`], with `.fallback(..)` for the `: <Other/>` branch |
| Error boundaries | [`crate::catch_errors`] |
| `<Suspense>` with React Query or SWR | [`crate::suspense`] around content reading a [`crate::reactive::Resource`] |
| `createContext` / `useContext` | [`crate::El::provide`] and [`crate::reactive::use_context`] |
| `localStorage` for UI state | [`crate::reactive::Signal::persist`]; what the web version saved is imported with [`crate::webstorage`] |
| CSS transitions, Framer Motion | [`crate::El::animate`], [`crate::show`] transitions, [`crate::widgets::for_each_with`] |
| Storybook | [`crate::devtools::preview`]: previews in every theme and locale, also golden-image tests |
| `flex`, `grid` in CSS | [`crate::El::row`], [`crate::El::column`], [`crate::El::layout`] (flex and grid) |
| React Router | [`crate::nav::NavStack`], [`crate::nav::tabs`], [`crate::nav::nav_rail`] |
| `fetch` in an effect | [`crate::App::background`], which returns a [`crate::BackgroundJob`] to cancel |
| Tauri `invoke` to Rust | A plain Rust call: the UI is Rust |
| `navigator.share({ files, title, text })`, Capacitor's `Share` | [`crate::App::share`] with a [`crate::host::ShareOptions`] (files as name, MIME type and bytes); the callback's [`crate::host::ShareOutcome`] says shared, cancelled, or unsupported with the reason, where the app falls back to a save dialog or the clipboard |
| React Testing Library `getByRole` | [`crate::testing::TestApp::find`] and [`crate::testing::TestApp::press`] |

## A component

```js
function Greeting({ name }) {
  const [shout, setShout] = useState(false);
  return (
    <div className="row">
      <span>{shout ? `HELLO, ${name.toUpperCase()}!` : `Hello, ${name}`}</span>
      <button onClick={() => setShout(!shout)}>Shout</button>
    </div>
  );
}
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
# fn names(ui: &Ui) -> Vec<String> {
#     ui.reader_view().into_iter().filter_map(|n| n.name).collect()
# }
# use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, WindowSpec};

fn greeting(ui: &Ui, name: String) -> El {
    let shout = Signal::new(false);
    let text = w::text(ui, move || {
        if shout.get() { format!("HELLO, {}!", name.to_uppercase()) } else { format!("Hello, {name}") }
    });
    w::row(ui).gap(8.0).child(text).child(w::button(ui, "Shout", move || shout.update(|s| *s = !*s)))
}

fn app(app: &App) {
    app.open(WindowSpec::new("Greeting"), |ui, _| {
        ui.root().child(greeting(ui, "Ada".into()));
    });
}

let mut app = TestApp::new(AppConfig::new("Greeting"), app);
let Some(ui) = app.ui() else {
    unreachable!("the window is open")
};
assert!(names(&ui).contains(&"Hello, Ada".to_string()));
assert!(app.press(Role::Button, "Shout"));
app.frames(1);
assert!(names(&ui).contains(&"HELLO, ADA!".to_string()));
```

`greeting` ran once. The click changed `shout`, which re-ran only the text's closure.

## Props

Props are arguments. Pass a plain value for what never changes, and a signal (or a closure)
for what does. Builder properties take either ([`crate::view::IntoProp`]), so a component
can pass a prop straight through. Signals are `Copy`, so passing one down costs nothing and needs no lifting or
`useCallback`:

```
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{El, Ui};

/// A badge that follows the unread count it is given.
fn unread_badge(ui: &Ui, unread: Signal<u32>) -> El {
    w::badge(ui, move || unread.get().to_string())
}
# let _ = unread_badge;
```

Callbacks are closures: `on_send: impl Fn(String) + 'static`. Store them in an `Rc` when more
than one handler needs the same one.

## State and derived state

`useState` is a signal. Reading it inside a binding subscribes that binding; writing it
updates every binding that read it. `useMemo` is a memo, with no dependency array to keep
right: it tracks what it reads.

```
use remedy_ui::reactive::{Memo, Signal, batch};

let price = Signal::new(12.5f64);
let quantity = Signal::new(2u32);
let total = Memo::new(move || price.get() * f64::from(quantity.get()));
assert_eq!(total.get(), 25.0);

// Several writes, one update: like React's automatic batching.
batch(|| {
    price.set(10.0);
    quantity.set(3);
});
assert_eq!(total.get(), 30.0);
```

There are no rules of hooks. Create signals anywhere, in any order, inside conditions and
loops.

## Effects and clean-up

`useEffect` with a cleanup becomes a binding that registers [`crate::reactive::on_cleanup`].
The cleanup runs before the binding runs again and when its element is removed:

```js
useEffect(() => {
  const id = subscribe(room);
  return () => unsubscribe(id);
}, [room]);
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::reactive::{Signal, on_cleanup};
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

let log: Rc<RefCell<Vec<String>>> = Rc::default();
let room = Signal::new("general".to_string());
let (l, panel) = (log.clone(), Rc::new(RefCell::new(None)));
let p = panel.clone();
let app = move |app: &App| {
    app.open(WindowSpec::new("Rooms"), move |ui, _| {
        let el = w::column(ui).bind(move || {
            let name = room.get();
            l.borrow_mut().push(format!("join {name}"));
            let l = l.clone();
            on_cleanup(move || l.borrow_mut().push(format!("leave {name}")));
        });
        *p.borrow_mut() = Some(el.clone());
        ui.root().child(el);
    });
};
let mut app = TestApp::new(AppConfig::new("Rooms"), app);
room.set("random".into());
if let Some(el) = panel.borrow().as_ref() {
    el.remove(); // unmount: the last clean-up runs
}
assert_eq!(*log.borrow(), ["join general", "leave general", "join random", "leave random"]);
```

For a clean-up that belongs to the element rather than to one run of an effect (closing a
socket the element opened), use [`crate::El::on_removed`].

## Lists and keys

`items.map(item => <Row key={item.id} … />)` becomes [`crate::widgets::for_each`] with a key
function. Rows are built once per key, moved when the order changes, and removed when their
item goes; a row keeps its element and its state across changes.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

#[derive(Clone)]
struct Todo { id: u32, title: String }

let todos = Signal::new(vec![Todo { id: 1, title: "Buy wax".into() }]);
let app = move |app: &App| {
    app.open(WindowSpec::new("Todos"), move |ui, _| {
        let list = w::column(ui).gap(4.0);
        let u = ui.clone();
        let list = w::for_each(&list, todos, |t: &Todo| t.id, move |t| w::text(&u, t.title.clone()));
        ui.root().child(list);
    });
};
let mut app = TestApp::new(AppConfig::new("Todos"), app);
todos.update(|v| v.push(Todo { id: 2, title: "Pour candles".into() }));
app.frames(1);
let Some(ui) = app.ui() else { unreachable!("open") };
let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
assert!(names.contains(&"Buy wax".to_string()) && names.contains(&"Pour candles".to_string()));
```

For long lists, use [`crate::widgets::virtual_list`] instead of a long `map`: only the rows
near the viewport exist. For tables, trees and chat feeds, use [`crate::data`]. See
[`crate::guide::lists`].

## Context

React context passes a value down the tree without props. [`crate::El::provide`] gives a
value to an element's subtree; code built inside it ([`crate::El::child_with`]) reads the
nearest one with [`crate::reactive::use_context`], and any element below with
[`crate::El::use_context`]. Provide a signal when the value changes.

```js
const Session = createContext(null);
<Session.Provider value={session}><Profile /></Session.Provider>
function Profile() {
  const session = useContext(Session);
  return <span>Signed in as {session.user}</span>;
}
```

```
# use remedy_ui::testing::TestApp;
use remedy_ui::a11y::Role;
use remedy_ui::reactive::{Signal, use_context};
use remedy_ui::widgets as w;
use remedy_ui::{El, Ui};

#[derive(Clone)]
struct Session {
    user: Signal<String>,
}

fn profile(ui: &Ui) -> El {
    let Some(session) = use_context::<Session>() else { return w::text(ui, "Signed out") };
    w::text(ui, move || format!("Signed in as {}", session.user.get()))
}

let user = Signal::new("ada".to_string());
let mut app = TestApp::window(move |ui, _| {
    let u = ui.clone();
    let page = w::column(ui).provide(Session { user }).child_with(move || profile(&u));
    ui.root().child(page);
});
assert!(app.find(Role::Label, "Signed in as ada").is_some());
user.set("grace".into());
app.frames(1);
assert!(app.find(Role::Label, "Signed in as grace").is_some());
```

The theme, the locale and the app are already reachable without context:
[`crate::Ui::theme`], [`crate::Ui::locale`], and [`crate::Window::app`].

## Conditional content, errors and loading

`{cond && <X/>}` is [`crate::show`]: its content is built while the condition holds
and removed, with its signals and bindings, when it stops. An error boundary is
[`crate::catch_errors`]: a failure while building, in a handler, a binding or a
painter below it replaces that content with the fallback, which gets the error and a
[`crate::Retry`]; the rest of the window carries on, and the inspector's Failures page
lists what was caught. `<Suspense>` with React Query is [`crate::suspense`] around
content that reads a [`crate::reactive::Resource`]: the resource fetches off the UI thread,
refetches when its key changes and cancels the old fetch, and the suspense shows its
placeholder only if loading takes longer than [`crate::view::SUSPENSE_DELAY`].

```js
<ErrorBoundary fallback={<p>Could not load the profile</p>}>
  <Suspense fallback={<Spinner />}>
    <Profile id={id} />
  </Suspense>
</ErrorBoundary>
```

```
# use remedy_ui::testing::TestApp;
use remedy_ui::a11y::Role;
use remedy_ui::reactive::{Resource, Signal};
use remedy_ui::{catch_errors, suspense};
use remedy_ui::widgets as w;

let id = Signal::new(7u32);
let mut app = TestApp::window(move |ui, _| {
    let (u1, u2, u3) = (ui.clone(), ui.clone(), ui.clone());
    let profile = move || {
        // Fetched on a worker; a new id cancels the fetch under way.
        let name: Resource<String, String> = Resource::new(move || id.get(), |id, _cancel| Ok(format!("User {id}")));
        w::text(&u1, move || name.get().unwrap_or_default())
    };
    // The boundary builds its content again on Try again, so it takes a builder.
    let content = move || {
        let spinner = { let u = u2.clone(); move || w::spinner(&u, "Loading profile") };
        suspense(&u2, spinner, profile.clone()).el()
    };
    let guarded = catch_errors(
        ui,
        move |error, retry| {
            w::column(&u3)
                .child(w::text(&u3, format!("Could not load the profile: {}", error.message)))
                .child(w::button(&u3, "Try again", move || retry.retry()))
        },
        content,
    );
    ui.root().child(guarded);
});
for _ in 0..1000 {
    if app.find(Role::Label, "User 7").is_some() {
        break;
    }
    std::thread::yield_now();
    app.frames(1);
}
assert!(app.find(Role::Label, "User 7").is_some());
```

## Persisted state

`localStorage` for UI state (a collapsed sidebar, the last tab) is
[`crate::reactive::Signal::persist`]: `Signal::new(false).persist("sidebar.collapsed")`
restores the saved value and saves each change, debounced and off the UI thread, in the
app's state folder ([`crate::App::persist_store`]). Window size and place have their own:
[`crate::WindowSpec::persist_geometry`].

### Bringing over what the web version saved

An update must never lose what people saved. Whatever the Tauri, Capacitor or PWA version
kept in `localStorage` (settings, recipes, notes) is still on the device, in the web view's
own store, and the first run of the RemedyUI version imports it with the `webstorage`
feature ([`crate::webstorage`]):

- **Tauri on Windows** kept it in WebView2's store:
  [`crate::webstorage::tauri_windows_dir`] with the bundle identifier from `tauri.conf.json`,
  under the origin `http://tauri.localhost` (or `https://` with `useHttpsScheme`):
  [`crate::webstorage::TAURI_WINDOWS_ORIGINS`].
- **Capacitor on Android** kept it in the Android System WebView's store, in the app's data
  folder: [`crate::webstorage::android_webview_dir`] (or
  [`crate::webstorage::android_webview_dir_from_files_dir`] from the internal files folder),
  under `https://localhost` or `http://localhost`: [`crate::webstorage::CAPACITOR_ORIGINS`].
  The RemedyUI build must keep the same package name and signing key to see that folder.
- **A PWA** that stays on its origin reads the page's own store with
  `webstorage::browser_local_storage()` (on `wasm32`), which returns the same
  [`crate::webstorage::LocalStorage`].

The store is read in place and never changed, even while the old app is running, so the
import can run again until it succeeds. Write what it found into the app's own files first,
then remember that it is done:

```rust,no_run
use std::path::Path;

use remedy_ui::webstorage::{self, ChromiumStorage, LocalStorage, WebStorageError};

/// What the Tauri version saved on this machine, or `None` when it never ran here.
fn saved_by_the_web_version() -> Result<Option<LocalStorage>, WebStorageError> {
    let Some(dir) = webstorage::tauri_windows_dir("com.remedy.craftcalc") else { return Ok(None) };
    match ChromiumStorage::read_dir(&dir) {
        Ok(store) => store.first_of(&webstorage::TAURI_WINDOWS_ORIGINS),
        Err(WebStorageError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Runs at start-up until one import succeeds.
fn import_once(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let done = data_dir.join("web-storage-imported");
    if done.exists() {
        return Ok(());
    }
    // An error returns here before `done` is written: the next start tries again.
    if let Some(saved) = saved_by_the_web_version()? {
        if let Some(settings) = saved.get("craftcalc.settings") {
            std::fs::write(data_dir.join("settings.json"), settings)?;
        }
        // ... every other key the web version used, into the app's own state.
    }
    std::fs::write(done, "")?;
    Ok(())
}
# let _ = import_once;
```

Keys the web version wrote under other names in older releases are all in
[`crate::webstorage::LocalStorage::items`], so the import can take them too, and
[`crate::webstorage::ChromiumStorage::origins`] lists every origin in a store when it is not
clear which one the app had.

## Styling: CSS becomes tokens and rules

There is no CSS. Colors, sizes, spacing, radii and motion are tokens in a
[`crate::style::Theme`]. How each widget looks, in each state (hover, pressed, focus,
disabled, selected), is a rule in a [`crate::style::StyleSheet`], written only in tokens. A
variant is like a modifier class:

```css
.button.danger { background: var(--danger); }
.button.danger:hover { background: var(--danger-hover); }
```

```
use remedy_ui::style::{States, StylePatch};
use remedy_ui::{AppConfig, themes};

let config = AppConfig::new("Vault").theme(themes::ThemeChoice::default().resolve(None, false), |t| {
    let mut sheet = themes::sheet(t);
    let hover = t.colors.danger.mix_oklab(t.colors.text, 0.15);
    sheet.rule("button", Some("wipe"), States::NONE, StylePatch::new().background(t.colors.danger));
    sheet.rule("button", Some("wipe"), States::HOVER, StylePatch::new().background(hover));
    sheet
});
# let _ = config;
```

Then `w::button(ui, "Wipe vault", wipe).variant("wipe")`. The Remedy widget sheet already
has `primary`, `quiet`, `danger` and `danger_solid` button variants. The theme set that
lives in RemedyAI's `themes.ts` is [`crate::themes::PALETTES`]; a user's theme loads from a
design-tokens file with [`crate::themes::from_tokens`]. One-off overrides are
[`crate::El::style`], which takes a value, a signal or a closure, so a style that follows
state is `.style(move || ..)`. Typed tokens name the design system in code:
`.gap(Space::S3)`, `Style::new().radius(Radius::R2).background(ColorRole::Accent)` (see
[`crate::style::Space`] and [`crate::style::Style`]).

## Events and refs

Handlers are builder methods: [`crate::El::on_click`], [`crate::El::on_key`],
[`crate::El::on_pointer`], [`crate::El::on_hover`], [`crate::El::on_focus_change`],
[`crate::El::on_double_click`]. A key handler returns `true` when it handled the key, which
stops it going further (React's `stopPropagation`).

A ref is the element itself. `El` is a handle: clone it into a closure and call
[`crate::El::focus`], [`crate::El::scroll_into_view`] or [`crate::El::bounds`] later.

## Tauri commands become calls

A Tauri app sends `invoke("save_note", { … })` to Rust and awaits a promise. With RemedyUI
the UI is Rust, so call the function. Slow work goes to [`crate::App::background`], which
hands the result back on the UI thread. Tray icons, single instance, file dialogs, the
clipboard, sharing and window control are methods on [`crate::App`] and [`crate::Window`].

## Testing

React Testing Library finds elements by role and name, as a screen reader would. The
RemedyUI equivalent is [`crate::testing::TestApp`]: it runs the app on the in-memory host and
finds and presses elements by [`crate::a11y::Role`] and name, as every sample on this page
does. See [`crate::guide::testing`].

## Pitfalls

- **Reading a signal outside a closure reads it once.** `w::text(ui, count.get().to_string())`
  never updates. Pass `move || count.get().to_string()`, or the signal itself.
- **Don't rebuild children on change.** Clearing a container and building it again in a
  binding is the virtual-DOM habit, without the diff. Use `for_each` with keys, or bind the
  one property that changes.
- **No `useEffect` to sync state.** Derive with a memo instead of an effect that writes
  another signal.
- **Borrowing.** Closures are `'static`. Capture signals by `move` (they are `Copy`); clone
  `El`, `Ui` and `App` handles before moving them in.
- **Conditional content is `show`, not a rebuilt child.** Clearing and re-adding an element
  from a binding loses its state each time; [`crate::show`] builds it once per showing
  and disposes it cleanly.
- **Effects that fetch must not race.** In React an effect's clean-up ignores a stale
  response. Here [`crate::App::background`] returns a [`crate::BackgroundJob`]: cancel the
  old one when the query changes, and its result never reaches the UI.
  [`crate::App::background_cancellable`] also lets the work stop early.
