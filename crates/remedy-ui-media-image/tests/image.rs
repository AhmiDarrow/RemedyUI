//! Decoding (alpha, color profiles, orientation, scaling, damaged and impossible files) and
//! the image element (off-thread loading, sizing, sharing, freeing, failures, edited files).

use std::sync::Arc;
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use remedy_ui_core::Size;
use remedy_ui_media_image::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};

fn png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4], icc: Option<Vec<u8>>) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            raw.extend_from_slice(&px(x, y));
        }
    }
    let mut out = Vec::new();
    let mut enc = PngEncoder::new(&mut out);
    if let Some(p) = icc {
        let _ = enc.set_icc_profile(p);
    }
    let _ = enc.write_image(&raw, w, h, ExtendedColorType::Rgba8);
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 90, 200]).collect();
    let mut out = Vec::new();
    let _ = JpegEncoder::new_with_quality(&mut out, 90).write_image(&raw, w, h, ExtendedColorType::Rgb8);
    out
}

/// `jpeg` with an EXIF block saying the camera was turned (orientation 6: rotate 90°).
fn turned_jpeg(w: u32, h: u32) -> Vec<u8> {
    let plain = jpeg(w, h);
    // TIFF header (little endian), one IFD entry: Orientation (0x0112), SHORT, 1, value 6.
    let tiff: Vec<u8> = [
        &b"II*\0"[..],
        &8u32.to_le_bytes(),
        &1u16.to_le_bytes(),
        &0x0112u16.to_le_bytes(),
        &3u16.to_le_bytes(),
        &1u32.to_le_bytes(),
        &6u16.to_le_bytes(),
        &[0, 0],
        &0u32.to_le_bytes(),
    ]
    .concat();
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let len = u16::try_from(app1.len() + 2).unwrap_or(u16::MAX);
    let mut out = plain.get(..2).map(<[u8]>::to_vec).unwrap_or_default();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(plain.get(2..).unwrap_or_default());
    out
}

fn px(img: &remedy_ui_paint::Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    img.pixels.get(i..i + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])
}

#[test]
fn pixels_come_out_premultiplied() {
    let bytes = png(2, 1, |x, _| if x == 0 { [200, 100, 50, 255] } else { [200, 100, 50, 128] }, None);
    let d = decode(&bytes, None).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(d.source_size, (2, 1));
    assert_eq!(px(&d.image, 0, 0), [200, 100, 50, 255]);
    assert_eq!(px(&d.image, 1, 0), [100, 50, 25, 128]);
}

#[test]
fn large_images_decode_at_the_size_they_are_shown() {
    let bytes = jpeg(1600, 1200);
    assert_eq!(dimensions(&bytes).ok(), Some((1600, 1200)));
    let at = fitted((1600, 1200), (200, 200), false);
    let d = decode(&bytes, Some(at)).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height), (200, 150));
    assert_eq!(d.source_size, (1600, 1200));
    // Never enlarged.
    let d = decode(&bytes, Some((4000, 3000))).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height), (1600, 1200));
}

#[test]
fn exif_orientation_turns_photos_upright() {
    let bytes = turned_jpeg(64, 32);
    assert_eq!(dimensions(&bytes).ok(), Some((32, 64)));
    let d = decode(&bytes, None).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height), (32, 64));
}

#[test]
fn colors_in_a_wide_gamut_profile_are_converted_to_srgb() {
    // Pure Display P3 red is outside sRGB: converted, it clips to sRGB's red with green and
    // blue at 0; read as sRGB it would stay (255, 0, 0) too, so use a mid tone that moves.
    let p3 = moxcms::ColorProfile::new_display_p3().encode().unwrap_or_else(|e| unreachable!("{e:?}"));
    let bytes = png(1, 1, |_, _| [40, 160, 90, 255], Some(p3));
    let d = decode(&bytes, None).unwrap_or_else(|e| unreachable!("{e}"));
    let [r, g, b, a] = px(&d.image, 0, 0);
    assert_eq!(a, 255);
    assert_ne!([r, g, b], [40, 160, 90], "the P3 color was converted");
    // P3 green is more saturated than sRGB green: in sRGB the green channel rises or red falls.
    assert!(g >= 160 || r < 40, "{r} {g} {b}");
    // The same pixels with no profile are taken as sRGB.
    let plain =
        decode(&png(1, 1, |_, _| [40, 160, 90, 255], None), None).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(px(&plain.image, 0, 0), [40, 160, 90, 255]);
}

#[test]
fn damaged_unknown_and_impossible_files_are_errors_not_crashes() {
    assert_eq!(decode(b"not an image at all", None).err(), Some(DecodeError::Unrecognized));
    let mut cut = png(8, 8, |_, _| [1, 2, 3, 255], None);
    cut.truncate(cut.len() / 2);
    assert!(matches!(decode(&cut, None), Err(DecodeError::Malformed(_) | DecodeError::Io(_))));
    // A header claiming more pixels than any machine has memory for.
    let mut huge = png(1, 1, |_, _| [0, 0, 0, 255], None);
    // IHDR width and height live at bytes 16..24; the chunk CRC is not checked for them first.
    if let Some(dims) = huge.get_mut(16..24) {
        dims.copy_from_slice(&[0x7F, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF]);
    }
    let r = decode(&huge, None);
    assert!(matches!(r, Err(DecodeError::TooLarge { .. } | DecodeError::Malformed(_))), "{r:?}");
}

// ---------- the element ----------

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
    let _ = ui.take_requests();
}

/// Frames until no loads are in flight (results arrive through the waker).
fn settle(ui: &Ui) {
    let start = Instant::now();
    for _ in 0..4 {
        frame(ui);
        while !remedy_ui_reactive::has_posted() {
            if start.elapsed() > Duration::from_secs(10) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
            if start.elapsed() > Duration::from_millis(300) {
                break;
            }
        }
        let _ = remedy_ui_reactive::run_posted();
    }
    frame(ui);
}

fn semantics(el: &El) -> String {
    format!("{:?}", el.info().and_then(|i| i.semantics))
}

#[test]
fn an_image_element_loads_off_thread_sizes_itself_and_shares_its_pixels() {
    let ui = ui();
    let bytes: Arc<[u8]> = Arc::from(png(300, 150, |x, _| [x as u8, 0, 0, 255], None));
    let a = image(&ui, Source::Bytes(bytes.clone()), "A red ramp", Fit::Contain);
    let b = image(&ui, Source::Bytes(bytes.clone()), "", Fit::Cover).width(80.0).height(80.0);
    ui.root().child(ui.el("col").column().child(a.clone()).child(b.clone()));
    frame(&ui);
    assert_eq!(a.info().map(|i| i.variant), Some("loading"));
    settle(&ui);
    // Unsized, it takes the image's own size and aspect.
    let ab = a.bounds().unwrap_or_default();
    assert_eq!((ab.width(), ab.height()), (300.0, 150.0));
    assert_eq!(a.info().map(|i| i.variant), Some(""));
    assert!(semantics(&a).contains("A red ramp"));
    assert!(
        semantics(&b).contains("hidden: true") || semantics(&b).contains("hidden"),
        "decorative: {}",
        semantics(&b)
    );
    // Two sizes of the same source; both alive.
    assert_eq!(cached(), 2);
    // Gone from the window, the pixels are freed.
    ui.root().clear();
    frame(&ui);
    frame(&ui);
    assert_eq!(cached(), 0);
}

#[test]
fn an_image_that_cannot_be_shown_says_why() {
    let ui = ui();
    let el = image(&ui, Source::Bytes(Arc::from(&b"garbage"[..])), "Holiday photo", Fit::Contain)
        .width(100.0)
        .height(100.0);
    ui.root().child(el.clone());
    settle(&ui);
    let s = semantics(&el);
    assert!(s.contains("Not shown"), "{s}");
    assert_eq!(el.info().map(|i| i.variant), Some(""));
}

#[test]
fn an_edited_file_shows_its_new_pixels() {
    let dir = std::env::temp_dir().join(format!("remedy-image-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("pic.png");
    let _ = std::fs::write(&path, png(40, 20, |_, _| [255, 0, 0, 255], None));
    let ui = ui();
    let first = image(&ui, Source::File(path.clone()), "Picture", Fit::Contain).width(40.0).height(20.0);
    ui.root().child(first.clone());
    settle(&ui);
    // Rewritten, larger and in another color, then shown by a new element.
    std::thread::sleep(Duration::from_millis(20));
    let _ = std::fs::write(&path, png(80, 20, |_, _| [0, 0, 255, 255], None));
    let state = remedy_ui_reactive::Signal::new(ImageState::Loading);
    let _req = load(&Source::File(path.clone()), Some((40, 20)), false, state);
    settle(&ui);
    match state.get_untracked().ok() {
        Some(ImageState::Ready(img, size)) => {
            assert_eq!(size, (80, 20));
            assert_eq!(px(&img, 0, 0), [0, 0, 255, 255]);
        }
        other => unreachable!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
