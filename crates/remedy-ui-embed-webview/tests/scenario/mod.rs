//! The web view scenario every platform runs in a real window: a page from an HTML string,
//! messages both ways, a navigation the allow-list refuses, a popup and a download the page
//! cannot have, and the native view following its element as it scrolls, scrolls away and
//! is hidden.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use remedy_ui::core::Vec2;
use remedy_ui::{App, AppConfig, El, GeometryStore, Runner, Ui, WindowSpec};
use remedy_ui_embed_webview::serde_json::{Value, json};
use remedy_ui_embed_webview::*;

const PAGE: &str = r#"<!doctype html>
<html><head><title>Remedy test page</title></head>
<body style="margin:0;background:#2a6;color:#fff;font:16px sans-serif">
<p>Hello from the page.</p>
<script>
const start = location.href;
remedy.onMessage((v) => {
  if (v && v.go) {
    location.href = v.go;
    setTimeout(() => remedy.postMessage({ still_here: location.href === start }), 800);
    return;
  }
  if (v && v.popup) {
    const w = window.open(v.popup);
    remedy.postMessage({ popup_null: w === null });
    return;
  }
  if (v && v.download) {
    const a = document.createElement('a');
    a.href = 'data:application/octet-stream;base64,SGVsbG8=';
    a.download = 'hello.bin';
    document.body.appendChild(a);
    a.click();
    return;
  }
  remedy.postMessage({ echo: v });
});
remedy.postMessage({ ready: document.title });
</script>
</body></html>"#;

/// What the page and the test hold between steps.
#[derive(Default)]
struct World {
    ui: Option<Ui>,
    scroller: Option<El>,
    el: Option<El>,
    view: Option<WebView>,
    messages: Vec<Value>,
    /// Native content tops seen at each placement check, in order.
    notes: Vec<String>,
}

type Step = Box<dyn FnMut(&mut World) -> bool>;

/// Runs each step every 50 ms until it says it is done, then the next; quits when all are
/// done or after `deadline`, recording which step it stopped at.
fn drive(
    app: &App,
    world: Rc<RefCell<World>>,
    steps: Rc<RefCell<VecDeque<(&'static str, Step)>>>,
    since: Instant,
    stopped: Rc<RefCell<Option<&'static str>>>,
) {
    let a = app.clone();
    app.after(Duration::from_millis(50), move || {
        let done = {
            let mut steps = steps.borrow_mut();
            let mut w = world.borrow_mut();
            match steps.front_mut() {
                Some((_, step)) => {
                    if step(&mut w) {
                        steps.pop_front();
                    }
                    steps.is_empty()
                }
                None => true,
            }
        };
        if done {
            *stopped.borrow_mut() = None;
            a.quit();
        } else if since.elapsed() > Duration::from_secs(60) {
            *stopped.borrow_mut() = steps.borrow().front().map(|(n, _)| *n);
            a.quit();
        } else {
            drive(&a, world, steps, since, stopped);
        }
    });
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.5
}

/// Sends `value` once, then waits for a message `pred` accepts.
fn exchange(value: Value, pred: impl Fn(&Value) -> bool + 'static) -> Step {
    let mut sent = false;
    Box::new(move |w: &mut World| {
        if !sent {
            if let Some(v) = &w.view {
                v.post_message(&value);
            }
            sent = true;
        }
        w.messages.iter().any(&pred)
    })
}

/// Scrolls to `y` once, then waits until the native view's content top is where the
/// element's box (scrolled) puts it, and its frame is the part that shows.
fn scroll_and_follow(y: f32) -> Step {
    let mut scrolled = false;
    Box::new(move |w: &mut World| {
        let (Some(scroller), Some(el), Some(view), Some(ui)) = (&w.scroller, &w.el, &w.view, &w.ui) else {
            return false;
        };
        if !scrolled {
            scroller.scroll_to(Vec2::new(0.0, y));
            scrolled = true;
            return false;
        }
        let (Some(b), Some(n)) = (el.bounds(), view.native()) else { return false };
        let s = ui.scale();
        // The element sits 40 px down in the scroller.
        let expect_top = (40.0 - y) * s;
        let ok = n.visible
            && near(b.min.y * s, expect_top)
            && near(n.content.min.y, expect_top)
            && near(n.content.height(), 160.0 * s)
            && near(n.frame.min.y, expect_top.max(0.0))
            && near(n.frame.height(), (160.0 * s + expect_top.min(0.0)).max(0.0));
        if ok {
            w.notes.push(format!("scrolled {y}: content {:?} frame {:?}", n.content, n.frame));
        }
        ok
    })
}

/// Runs the scenario: `run` drives the runner on the platform's event loop until it quits.
pub fn scenario(run: impl FnOnce(Runner) -> Result<(), String>) {
    let world = Rc::new(RefCell::new(World::default()));
    let stopped: Rc<RefCell<Option<&'static str>>> = Rc::new(RefCell::new(Some("start")));
    let data_dir = std::env::temp_dir().join("remedy-ui-webview-test");
    let w2 = world.clone();
    let s2 = stopped.clone();
    let runner = Runner::with_geometry(
        AppConfig::new("RemedyUI web view test").cpu_only(),
        GeometryStore::in_memory(),
        move |app: &App| {
            let w3 = w2.clone();
            app.open(WindowSpec::new("RemedyUI web view test").size(480.0, 360.0), move |ui, _| {
                let options = WebViewOptions::new("Test page")
                    .data_dir(data_dir)
                    .allow(|r| r.source == NavigationSource::App || r.url.starts_with("data:"));
                // The app's own page shows at the platform's address for HTML strings
                // (`about:blank` on WebView2), which the app's source marks as its own.
                let (el, view) = web_view(ui, options);
                let el = el.width(300.0).height(160.0).layout(|l| l.grow = 0.0);
                let scroller = ui
                    .el("scroller")
                    .column()
                    .fill_width()
                    .height(360.0)
                    .scroll()
                    .child(ui.el("above").height(40.0).layout(|l| l.shrink = 0.0))
                    .child(el.clone())
                    .child(ui.el("below").height(900.0).layout(|l| l.shrink = 0.0));
                ui.root().child(scroller.clone());
                let w4 = w3.clone();
                view.on_message(move |v| w4.borrow_mut().messages.push(v.clone()));
                view.load_html(PAGE);
                let mut w = w3.borrow_mut();
                w.ui = Some(ui.clone());
                w.scroller = Some(scroller);
                w.el = Some(el);
                w.view = Some(view);
            });
            let steps: VecDeque<(&'static str, Step)> = VecDeque::from([
                (
                    "the page loads and says it is ready",
                    Box::new(|w: &mut World| {
                        let ready = w.view.as_ref().is_some_and(|v| v.status() == WebViewStatus::Ready);
                        ready && w.messages.contains(&json!({ "ready": "Remedy test page" }))
                    }) as Step,
                ),
                (
                    "the title arrives as a signal",
                    Box::new(|w: &mut World| {
                        w.view.as_ref().is_some_and(|v| v.title() == "Remedy test page")
                    }),
                ),
                (
                    "a message goes to the page and comes back",
                    exchange(json!({ "n": 41 }), |v| *v == json!({ "echo": { "n": 41 } })),
                ),
                (
                    "a navigation the allow-list refuses stays blocked",
                    exchange(json!({ "go": "https://example.com/" }), |v| {
                        *v == json!({ "still_here": true })
                    }),
                ),
                (
                    "a popup is refused",
                    exchange(json!({ "popup": "https://example.org/" }), |v| {
                        *v == json!({ "popup_null": true })
                    }),
                ),
                ("a download is denied", {
                    let mut sent = false;
                    Box::new(move |w: &mut World| {
                        let Some(v) = &w.view else { return false };
                        if !sent {
                            v.post_message(&json!({ "download": 1 }));
                            sent = true;
                        }
                        v.blocked().iter().any(|b| b.reason == BlockReason::Download)
                    })
                }),
                ("the view sits on its element", scroll_and_follow(0.0)),
                ("it moves when the element scrolls", scroll_and_follow(30.0)),
                ("it is cut where the element is scrolled partly out", scroll_and_follow(100.0)),
                ("it hides when the element scrolls out of sight", {
                    let mut scrolled = false;
                    Box::new(move |w: &mut World| {
                        let (Some(scroller), Some(view)) = (&w.scroller, &w.view) else { return false };
                        if !scrolled {
                            scroller.scroll_to(Vec2::new(0.0, 400.0));
                            scrolled = true;
                            return false;
                        }
                        view.native().is_some_and(|n| !n.visible)
                    })
                }),
                ("it comes back when scrolled back", scroll_and_follow(0.0)),
                ("it hides with its element", {
                    let mut hidden = false;
                    Box::new(move |w: &mut World| {
                        let (Some(el), Some(view)) = (&w.el, &w.view) else { return false };
                        if !hidden {
                            let _ = el.clone().hidden(true);
                            hidden = true;
                            return false;
                        }
                        view.native().is_some_and(|n| !n.visible)
                    })
                }),
            ]);
            drive(app, w2.clone(), Rc::new(RefCell::new(steps)), Instant::now(), s2.clone());
        },
    );
    let result = run(runner);
    assert!(result.is_ok(), "{result:?}");
    let w = world.borrow();
    let status = w.view.as_ref().map(WebView::status);
    let blocked = w.view.as_ref().map(WebView::blocked).unwrap_or_default();
    let url = w.view.as_ref().map(WebView::url).unwrap_or_default();
    assert_eq!(
        *stopped.borrow(),
        None,
        "stopped waiting; status {status:?}, messages {:?}, blocked {blocked:?}, notes {:?}",
        w.messages,
        w.notes
    );
    assert!(
        blocked
            .iter()
            .any(|b| b.url.starts_with("https://example.com") && b.reason == BlockReason::NotAllowed),
        "{blocked:?}"
    );
    assert!(blocked.iter().any(|b| b.reason == BlockReason::NewWindow), "{blocked:?}");
    assert!(!url.contains("example.com"), "still on the app's page: {url}");
    assert_eq!(w.view.as_ref().map(WebView::malformed_messages), Some(0));
}
