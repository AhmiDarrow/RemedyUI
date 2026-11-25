//! SVG drawing at any size, currentColor, the image loader, and untrusted references.

use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_core::{Color, Size};
use remedy_ui_engine::paint::Image;
use remedy_ui_engine::view::Ui;
use remedy_ui_media::media_image::svg::*;
use remedy_ui_media::media_image::{Fit, ImageState, Source, decode, dimensions, image, load};
use remedy_ui_text::FontCollection;

const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
  <rect x="0" y="0" width="5" height="10" fill="#ff0000"/>
  <rect x="5" y="0" width="5" height="10" fill="currentColor"/>
</svg>"##;

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    img.pixels.get(i..i + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])
}

#[test]
fn svg_draws_sharp_at_any_size_and_current_color_is_the_text_color() {
    assert_eq!(size(SQUARE.as_bytes()).ok(), Some((10, 10)));
    let img = render(SQUARE.as_bytes(), Some((200, 200)), Some(Color::rgb8(0, 0, 255)))
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((img.width, img.height), (200, 200));
    assert_eq!(px(&img, 50, 100), [255, 0, 0, 255]);
    assert_eq!(px(&img, 150, 100), [0, 0, 255, 255]);
    // The edge between the halves stays crisp at 20x: no blended column.
    assert_eq!(px(&img, 99, 100), [255, 0, 0, 255]);
    assert_eq!(px(&img, 100, 100), [0, 0, 255, 255]);
}

#[test]
fn svg_files_never_reach_other_files() {
    let dir = std::env::temp_dir().join(format!("remedy-svg-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // A bitmap next to the SVG, and an SVG that tries to draw it.
    let red = {
        let mut raw = Vec::new();
        for _ in 0..16 {
            raw.extend_from_slice(&[255, 0, 0, 255]);
        }
        let mut out = Vec::new();
        let enc = image::codecs::png::PngEncoder::new(&mut out);
        let _ = image::ImageEncoder::write_image(enc, &raw, 4, 4, image::ExtendedColorType::Rgba8);
        out
    };
    let target = dir.join("secret.png");
    let _ = std::fs::write(&target, red);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image width="4" height="4" xlink:href="{}"/></svg>"#,
        target.display()
    );
    let img = render(svg.as_bytes(), None, None).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(px(&img, 2, 2), [0, 0, 0, 0], "the local file was not drawn");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn installed_svg_goes_through_the_image_loader_at_the_size_shown() {
    install();
    assert_eq!(dimensions(SQUARE.as_bytes()).ok(), Some((10, 10)));
    let d = decode(SQUARE.as_bytes(), Some((64, 64))).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(
        (d.image.width, d.image.height),
        (64, 64),
        "vector images decode larger than their natural size"
    );
    assert_eq!(d.source_size, (10, 10));
    // Through the loader, asked for a 64-pixel box.
    let state = remedy_ui_core::reactive::Signal::new(ImageState::Loading);
    let _req = load(&Source::Bytes(Arc::from(SQUARE.as_bytes())), Some((64, 64)), false, state);
    let start = Instant::now();
    while !remedy_ui_core::reactive::has_posted() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = remedy_ui_core::reactive::run_posted();
    match state.get_untracked().ok() {
        Some(ImageState::Ready(img, natural)) => {
            assert_eq!(natural, (10, 10));
            assert_eq!((img.width, img.height), (64, 64));
        }
        other => unreachable!("{other:?}"),
    }
}

fn ui() -> Ui {
    let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts)
}

#[test]
fn icons_take_the_text_color_and_name() {
    let ui = ui();
    let el = icon(&ui, Arc::from(SQUARE.as_bytes()), 20.0, "Status");
    let quiet = icon(&ui, Arc::from(SQUARE.as_bytes()), 20.0, "");
    ui.root().child(ui.el("row").row().child(el.clone()).child(quiet.clone()));
    let _ = ui.frame(Size::new(200.0, 100.0), 2.0);
    let b = el.bounds().unwrap_or_default();
    assert_eq!((b.width(), b.height()), (20.0, 20.0));
    let sem = format!("{:?}", el.info().and_then(|i| i.semantics));
    assert!(sem.contains("Status"), "{sem}");
    let quiet_sem = format!("{:?}", quiet.info().and_then(|i| i.semantics));
    assert!(quiet_sem.contains("hidden"), "{quiet_sem}");
    // And the image element shows SVG files.
    install();
    let pic = image(&ui, Source::Bytes(Arc::from(SQUARE.as_bytes())), "Square", Fit::Contain)
        .width(100.0)
        .height(100.0);
    ui.root().child(pic.clone());
    let _ = ui.frame(Size::new(200.0, 100.0), 2.0);
    assert!(format!("{:?}", pic.info().and_then(|i| i.semantics)).contains("Square"));
}

#[test]
fn svg_behind_a_long_prolog_is_recognised_up_to_the_sniff_limit() {
    install();
    let long = format!("<?xml version=\"1.0\"?>\n<!-- {} -->\n{SQUARE}", "licence text ".repeat(2000));
    assert!(long.len() > 16 * 1024, "longer than the loader's buffered head");
    assert_eq!(dimensions(long.as_bytes()).ok(), Some((10, 10)));
    // A file goes through the loader's header read, which buffers less than the prolog.
    let path = std::env::temp_dir().join(format!("remedy-svg-long-{}.svg", std::process::id()));
    std::fs::write(&path, &long).unwrap_or_else(|e| unreachable!("{e}"));
    let state = remedy_ui_core::reactive::Signal::new(ImageState::Loading);
    remedy_ui_media::media_image::probe(&Source::File(path.clone()), state);
    let start = Instant::now();
    while state.with_untracked(|s| matches!(s, ImageState::Loading)).unwrap_or(false)
        && start.elapsed() < Duration::from_secs(10)
    {
        let _ = remedy_ui_core::reactive::run_posted();
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = std::fs::remove_file(&path);
    assert!(matches!(state.get_untracked().ok(), Some(ImageState::Sized((10, 10)))));
    // A prolog longer than the limit is not looked through.
    assert_eq!(sniff_limit(), DEFAULT_SNIFF_LIMIT);
    set_sniff_limit(4096);
    let short = dimensions(long.as_bytes());
    set_sniff_limit(DEFAULT_SNIFF_LIMIT);
    assert!(short.is_err(), "{short:?}");
}
