//! An app's own fetcher ([`set_fetcher`]): it is asked for addresses, its replies are shown,
//! and it is told when nobody wants a fetch any more. Its own test binary, since the fetcher
//! is set for the whole process.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use remedy_ui_core::Size;
use remedy_ui_media_image::*;
use remedy_ui_core::reactive::Cancel;
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;

type Asked = Arc<Mutex<Vec<(FetchRequest, Cancel, Option<FetchDone>)>>>;

struct Held(Asked);

impl Fetch for Held {
    fn fetch(&self, request: FetchRequest, cancel: Cancel, done: FetchDone) {
        if let Ok(mut a) = self.0.lock() {
            a.push((request, cancel, Some(done)));
        }
    }
}

fn ui() -> Ui {
    set_cache_dir(None);
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn pump(ui: &Ui, done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
        let _ = remedy_ui_core::reactive::run_posted();
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn an_apps_fetcher_supplies_the_bytes_and_hears_cancellation() {
    let asked: Asked = Arc::default();
    set_fetcher(Some(Arc::new(Held(asked.clone()))));
    let ui = ui();
    let count = || asked.lock().map(|a| a.len()).unwrap_or(0);

    // Answered: the element shows the reply.
    let el =
        image(&ui, Source::Url("https://cdn.example/a.png".into()), "A", Fit::Contain).width(8.0).height(4.0);
    ui.root().child(el.clone());
    assert!(pump(&ui, || count() == 1));
    let mut png = Vec::new();
    let _ = PngEncoder::new(&mut png).write_image(&[9; 8 * 4 * 4], 8, 4, ExtendedColorType::Rgba8);
    let done = asked.lock().ok().and_then(|mut a| a.first_mut().and_then(|x| x.2.take()));
    assert_eq!(
        asked.lock().ok().and_then(|a| a.first().map(|x| x.0.url.clone())).as_deref(),
        Some("https://cdn.example/a.png")
    );
    if let Some(done) = done {
        done(Ok(FetchReply::new(200, vec![], png)));
    }
    assert!(pump(&ui, || el.info().is_some_and(|i| i.variant.is_empty()) && cached() == 1));
    assert_eq!(count(), 1, "one fetch for the probe and the load");

    // Never answered, then the element goes: the fetcher is told.
    let slow =
        image(&ui, Source::Url("https://cdn.example/b.png".into()), "B", Fit::Contain).width(8.0).height(4.0);
    ui.root().child(slow.clone());
    assert!(pump(&ui, || count() == 2));
    let flag = asked.lock().ok().and_then(|a| a.get(1).map(|x| x.1.clone())).unwrap_or_default();
    assert!(!flag.is_cancelled());
    slow.remove();
    assert!(flag.is_cancelled(), "cancelled with the element");
    assert_eq!(fetching(), 0);

    // An error reply is a failure, shown as one.
    let bad =
        image(&ui, Source::Url("https://cdn.example/c.png".into()), "C", Fit::Contain).width(8.0).height(4.0);
    ui.root().child(bad.clone());
    assert!(pump(&ui, || count() == 3));
    let done = asked.lock().ok().and_then(|mut a| a.get_mut(2).and_then(|x| x.2.take()));
    if let Some(done) = done {
        done(Ok(FetchReply::new(500, vec![], vec![])));
    }
    assert!(pump(&ui, || format!("{:?}", bad.info().and_then(|i| i.semantics)).contains("500")));
    set_fetcher(None);
}
