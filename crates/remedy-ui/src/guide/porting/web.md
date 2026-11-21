# Moving from pywebview or an HTML app

Hearth is a Python app whose UI is a web page: pywebview shows HTML,
CSS and JavaScript, and a bridge calls Python. This guide is for any app built that way,
including Electron-style and Tauri apps whose UI is plain HTML.

With RemedyUI the UI is native elements in the same Rust program as the logic. There is no
page, no bridge, no second language and no browser process. When some content really is a
web page, it can still be embedded, behind an allow-list and one message channel.

## Concept map

| HTML app | RemedyUI |
|---|---|
| `index.html` | A builder function returning an [`crate::El`] |
| `<div>`, `<span>`, `<p>` | [`crate::widgets::column`], [`crate::widgets::row`], [`crate::widgets::text`] |
| `<button onclick>` | [`crate::widgets::button`] with a closure |
| `<input>`, `<textarea>` | [`crate::widgets::text_input`], [`crate::widgets::text_area`] |
| `<input type=password>` | [`crate::widgets::secret_input`]: zeroized, never copied, hidden from assistive technology |
| `<select>`, checkboxes, radios | [`crate::widgets::select`], [`crate::widgets::check`], [`crate::widgets::radio_group`] |
| `<img>` | [`crate::widgets::image`]; files with [`crate::image`] |
| `<video>` | [`crate::playback`] and its `player` widget |
| CSS custom properties | Theme tokens in a [`crate::style::Theme`], named in code by typed tokens (`Space::S4`, `Radius::R2`, `ColorRole::Accent`; [`crate::style::Space`]) |
| CSS rules and classes | A [`crate::style::StyleSheet`]: rules by widget name, variant and state |
| `display: flex` / `grid` | [`crate::El::row`], [`crate::El::column`], grid tracks in [`crate::El::layout`] |
| `overflow: auto` | [`crate::widgets::scroll`] |
| DOM updates from JS | Signals bound to element properties: `.text(move \|\| ..)`, `.disabled(signal)` |
| Toggling `hidden` or `display: none` | [`crate::show`]: the content exists only while shown |
| A spinner while `fetch()` runs | [`crate::suspense`] around content reading a [`crate::reactive::Resource`] |
| CSS transitions and animations | [`crate::El::animate`]; [`crate::widgets::for_each_with`] for list items arriving, leaving and moving |
| `pywebview.api.save(...)` / `window.expose` | A Rust function call |
| `fetch()` or a Python thread | [`crate::App::background`] (cancel it through the [`crate::BackgroundJob`] it returns) |
| `setTimeout` | [`crate::App::after`], [`crate::Ui::after`] |
| `localStorage` | [`crate::reactive::Signal::persist`] for state; files in [`crate::App::data_dir`] for documents; what the old app saved is imported once with [`crate::webstorage`] (see "Persisted state" in [`crate::guide::porting::react`]) |
| `window.open` | Another native window: [`crate::App::open`] |
| A remote web page shown in the app | [`crate::webview`]: the platform's web view, allow-listed |
| WebGL canvas | [`crate::gpu_embed`]: the app's wgpu rendering as an element |

## Markup and state

```html
<div class="card">
  <h2>Shared folders</h2>
  <p id="count">0 peers online</p>
  <button onclick="api.refresh()">Refresh</button>
</div>
<script>
  window.addEventListener('peers', (e) => {
    document.getElementById('count').textContent = `${e.detail} peers online`;
  });
</script>
```

The HTML version keeps the count in the DOM and updates it by id. In RemedyUI the count is a
signal and the text is bound to it:

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

/// What the old `api.refresh()` did in Python, now an ordinary function.
fn count_peers() -> u32 {
    3
}

fn shares(ui: &Ui, peers: Signal<u32>) -> El {
    w::card(ui)
        .child(w::heading(ui, 2, "Shared folders"))
        .child(w::text(ui, move || format!("{} peers online", peers.get())))
        .child(w::button(ui, "Refresh", move || peers.set(count_peers())))
}

fn app(app: &App) {
    app.open(WindowSpec::new("Hearth"), |ui, _| {
        let peers = Signal::new(0);
        ui.root().child(shares(ui, peers));
    });
}

let mut app = TestApp::new(AppConfig::new("Hearth"), app);
let Some(ui) = app.ui() else {
    unreachable!("the window is open")
};
assert!(names(&ui).contains(&"0 peers online".to_string()));
assert!(app.press(Role::Button, "Refresh"));
app.frames(1);
assert!(names(&ui).contains(&"3 peers online".to_string()));
```

There are no ids to look up and no events to dispatch between languages. Anything that sets
`peers`, from a button or a network thread, updates the text.

## The bridge becomes a call

pywebview exposes a Python object to JavaScript; every call crosses a process boundary as
JSON and comes back as a promise.

```python
class Api:
    def count_files(self, folder):
        return {"count": len(os.listdir(folder))}

webview.create_window("Hearth", "ui/index.html", js_api=Api())
```

```js
const result = await pywebview.api.count_files(folder);
document.getElementById('status').textContent = `${result.count} files`;
```

In RemedyUI the handler calls the function. Work that can be slow (disk, network, hashing)
goes to [`crate::App::background`], and its result comes back on the UI thread:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    let status = Signal::new(String::new());
    let a = app.clone();
    app.open(WindowSpec::new("Files"), move |ui, _| {
        let count = w::button(ui, "Count files", move || {
            let folder = std::env::temp_dir();
            a.background(
                move || std::fs::read_dir(folder).map(|d| d.count()).map_err(|e| e.to_string()),
                move |result| status.set(match result {
                    Ok(n) => format!("{n} files"),
                    Err(e) => format!("Could not read the folder: {e}"),
                }),
            );
        });
        ui.root().child(w::column(ui).child(count).child(w::text(ui, status)));
    });
}
# let mut app = TestApp::new(AppConfig::new("Files"), app);
# let Some(ui) = app.ui() else { unreachable!("open") };
# let Some(button) = ui.reader_view().into_iter().find(|n| n.name.as_deref() == Some("Count files")) else { unreachable!("shown") };
# assert!(ui.accessibility_action(button.id, remedy_ui::view::A11yAction::Click));
# let done = |ui: &remedy_ui::Ui| ui.reader_view().iter().any(|n| n.name.as_deref().is_some_and(|s| s.ends_with("files") && s != "Count files"));
# for _ in 0..500 {
#     app.frames(1);
#     if done(&ui) { break; }
#     std::thread::sleep(std::time::Duration::from_millis(2));
# }
# assert!(done(&ui));
```

Errors are Rust values, not rejected promises with a string inside.

## CSS becomes tokens and rules

Hearth, Portal and RemedyAI share one set of CSS variables, the Remedy themes. Those palettes
are [`crate::themes::PALETTES`], with the same names and the colour-blind variants. A
user's theme file loads with [`crate::themes::from_tokens`] (W3C design tokens).

```css
:root { --accent: #3d8b5f; --radius: 8px; }
.card { background: var(--card); border-radius: var(--radius); padding: 16px; }
button.primary { background: var(--accent); color: var(--on-accent); }
button.primary:hover { background: var(--accent-hover); }
```

The Remedy widget sheet ([`crate::themes::sheet`]) already styles cards, buttons and their
`primary` variant from the theme's tokens, in every state. Add rules only for what the app
adds:

```
use remedy_ui::style::{States, StylePatch};
use remedy_ui::{AppConfig, themes};

let config = AppConfig::new("Hearth").theme(themes::ThemeChoice::System.resolve(Some(true), false), |t| {
    let mut sheet = themes::sheet(t);
    let peer = StylePatch::new().background(t.colors.panel).radius(t.radii.large);
    sheet.rule("card", Some("peer"), States::NONE, peer);
    let hover = StylePatch::new().border(t.colors.accent).border_width(t.stroke);
    sheet.rule("card", Some("peer"), States::HOVER, hover);
    sheet
});
# let _ = config;
```

`w::card(ui).variant("peer")` picks it up. For one element, `.style(..)` takes the same
values as typed tokens: `Style::new().radius(Radius::R3).padding(Space::S4)` (see
[`crate::style::Style`]). `ThemeChoice::System` follows the platform's
light or dark setting, like `prefers-color-scheme`. Layout properties are not in the sheet:
they are set on the element ([`crate::El::layout`], [`crate::El::gap`], [`crate::El::padding`]).
See [`crate::guide::theming`] and [`crate::guide::layout`].

## When a web view is still right

Some content is a web page: help pages served from a site, a sign-in page, a web app the
app wraps. [`crate::webview`] shows the platform's web view (WebView2 on Windows, WebKitGTK
under X11 on Linux) as an element that follows its box. It is not a way to keep an HTML UI:
the page cannot reach the app's signals or files.

- Every navigation, of the page and of its frames, goes through the app's allow-list
  ([`crate::webview::WebViewOptions::allow`]). With none, only what the app loads itself is
  allowed.
- One JSON channel crosses: [`crate::webview::WebView::post_message`] and
  [`crate::webview::WebView::on_message`] in Rust, `window.remedy` in the page.
- Downloads are denied unless the app allows each one; developer tools are off unless asked
  for; new windows, permission requests and autofill are refused.

```js
remedy.onMessage((msg) => document.title = msg.title);
remedy.postMessage({ ready: true });
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::webview::{WebViewOptions, WebViewStatus, serde_json, web_view};
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    app.open(WindowSpec::new("Help"), |ui, _| {
        let options = WebViewOptions::new("Help")
            .allow(|nav| nav.url.starts_with("https://help.example.org/"));
        let (el, view) = web_view(ui, options);
        let v = view.clone();
        view.on_message(move |msg| {
            if msg.get("ready").is_some() {
                v.post_message(&serde_json::json!({ "title": "Help" }));
            }
        });
        view.navigate("https://help.example.org/start");
        if let WebViewStatus::Unsupported(why) = view.status() {
            // A headless host, or a platform without a web view: say so instead of a blank box.
            eprintln!("no web view here: {why}");
        }
        ui.root().child(el);
    });
}
// On the in-memory host there is no native window, so the view reports `Unsupported`;
// `remedy_ui::run(AppConfig::new("Help"), app)` shows the page for real.
let mut app = TestApp::new(AppConfig::new("Help"), app);
let Some(ui) = app.ui() else { unreachable!("open") };
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Help")));
```

## When the page drew with WebGL

A 3D preview drawn with WebGL moves to [`crate::gpu_embed`], which puts the app's
own wgpu rendering in an element, on the same GPU device RemedyUI's windows draw with. The
app implements [`crate::gpu_embed::GpuContent`]: `prepare` makes pipelines (again after a
device loss), and `render` records one picture into the element's texture.

```no_run
use remedy_ui::gpu_embed::{Gpu, GpuContent, PrepareError, Target, gpu_view, wgpu};
use remedy_ui::{App, AppConfig, WindowSpec};

struct Preview;

impl GpuContent for Preview {
    fn prepare(&mut self, _gpu: &Gpu<'_>) -> Result<(), PrepareError> {
        Ok(())
    }
    fn render(&mut self, _gpu: &Gpu<'_>, target: &Target<'_>, encoder: &mut wgpu::CommandEncoder) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
    }
}

fn app(app: &App) {
    app.open(WindowSpec::new("Preview"), |ui, _| {
        let (el, view) = gpu_view(ui, "3D preview", Preview);
        view.set_animating(true); // render every frame while the model turns
        ui.root().child(el);
    });
}

fn main() -> Result<(), remedy_ui::RunError> {
    remedy_ui::run(AppConfig::new("Preview"), app)
}
```

## When the app must still run in a browser

Moving off HTML does not mean leaving the browser. A RemedyUI app builds for
`wasm32-unknown-unknown` with the facade's `web` feature and runs in a page or as a PWA, with
the same code as on the desktop: `remedy_ui::run` starts it in the page (the bridge is
`remedy_ui::web` on that target). The page gets the engine's own drawing, input methods,
clipboard and screen reader support, not a DOM UI. What a browser cannot do (a tray icon,
capture protection, dragging out of the page) reports `Unsupported`, as on any platform that
lacks it. See [`crate::guide::web`].

## Pitfalls

- **Don't port the page into a web view.** Embedding the old HTML keeps the browser
  process, the bridge and the second language. Use [`crate::webview`] only for content that
  is a web page.
- **There is no cascade.** A style rule applies to the widget name and variant it names, not
  to descendants. Text color and size do pass to child text.
- **No `innerHTML`.** Build elements, or show formatted text as
  [`crate::view::RichText`]. Markdown goes through [`crate::rich`].
- **Nothing blocks the UI thread silently.** Disk and network work in a click handler freezes
  the window, as a long synchronous call would in a page. Use [`crate::App::background`].
- **Accessibility is built in, not added with ARIA.** Widgets declare their roles and names;
  check them with [`crate::Ui::reader_view`] (see [`crate::guide::accessibility`]).
