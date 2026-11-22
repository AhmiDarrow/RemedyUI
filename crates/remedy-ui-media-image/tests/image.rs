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
    ui.set_locale("fr");
    settle(&ui);
    assert!(semantics(&el).contains("Non affichée"), "{}", semantics(&el));
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&remedy_ui_media_image::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
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

#[test]
fn other_decoders_can_finish_their_pixels_the_way_decode_does() {
    // Straight-alpha 4×2 pixels: the top row opaque, the bottom half transparent.
    let rgba: Vec<u8> =
        (0..8).flat_map(|x| if x < 4 { [200, 100, 50, 255] } else { [200, 100, 50, 128] }).collect();
    let img = finish_rgba(4, 2, &rgba, None, None).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((img.width, img.height), (4, 2));
    assert_eq!(px(&img, 0, 0), [200, 100, 50, 255]);
    assert_eq!(px(&img, 3, 1), [100, 50, 25, 128], "premultiplied");
    // Scaled down, never up.
    let small = finish_rgba(4, 2, &rgba, None, Some((2, 1))).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((small.width, small.height), (2, 1));
    let same = finish_rgba(4, 2, &rgba, None, Some((40, 20))).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((same.width, same.height), (4, 2));
    // Pixels that do not fit the size are an error.
    assert!(matches!(finish_rgba(5, 5, &rgba, None, None), Err(DecodeError::Malformed(_))));
    // A color profile is read once and applied to every frame.
    let p3 = moxcms::ColorProfile::new_display_p3().encode().unwrap_or_else(|e| unreachable!("{e:?}"));
    let srgb = Srgb::from_profile(&p3).unwrap_or_else(|| unreachable!("a profile"));
    let converted =
        finish_rgba(1, 1, &[40, 160, 90, 255], Some(&srgb), None).unwrap_or_else(|e| unreachable!("{e}"));
    assert_ne!(px(&converted, 0, 0), [40, 160, 90, 255]);
    assert!(Srgb::from_profile(b"not a profile").is_none());
}

#[test]
fn an_image_element_can_take_its_pixels_from_another_supplier() {
    use std::cell::Cell;
    use std::rc::Rc;
    let ui = ui();
    let asked: Rc<Cell<Option<(u32, u32)>>> = Rc::default();
    let dropped: Rc<Cell<bool>> = Rc::default();
    struct Guard(Rc<Cell<bool>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let (a2, d2) = (asked.clone(), dropped.clone());
    let loader: Loader = Rc::new(move |_, box_px, _, state| {
        a2.set(box_px);
        // Frames arrive one after the other; the element draws each.
        let pixels: std::sync::Arc<[u8]> = std::sync::Arc::from(vec![0, 0, 255, 255, 0, 0, 255, 255]);
        state.set(ImageState::Ready(remedy_ui_paint::Image::new(2, 1, pixels), (2, 1)));
        Box::new(Guard(d2.clone()))
    });
    let bytes: Arc<[u8]> = Arc::from(png(2, 1, |_, _| [0, 0, 255, 255], None));
    let el = image_with(&ui, Source::Bytes(bytes), "Supplied", Fit::Contain, loader).width(20.0).height(10.0);
    ui.root().child(el.clone());
    settle(&ui);
    assert!(asked.get().is_some_and(|(w, h)| w >= 20 && h >= 10));
    assert_eq!(el.info().map(|i| i.variant), Some(""), "ready");
    assert!(!dropped.get());
    el.remove();
    assert!(dropped.get(), "the supplier is let go with the element");
}

#[test]
fn sources_are_told_apart_and_files_notice_edits() {
    let bytes: Arc<[u8]> = Arc::from(png(1, 1, |_, _| [1, 2, 3, 255], None));
    let a = Source::Bytes(bytes.clone());
    assert_eq!(a.id(), Source::Bytes(bytes).id());
    assert_ne!(a.id(), Source::Bytes(Arc::from(png(1, 1, |_, _| [1, 2, 3, 255], None))).id());
    assert!(a.stamp().is_none());
    let dir = std::env::temp_dir().join(format!("remedy-image-id-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("a.png");
    let _ = std::fs::write(&path, png(2, 2, |_, _| [1, 2, 3, 255], None));
    let file = Source::File(path.clone());
    let (read, stamp) = file.read().unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(stamp, file.stamp());
    assert_eq!(dimensions(&read).ok(), Some((2, 2)));
    std::thread::sleep(Duration::from_millis(20));
    let _ = std::fs::write(&path, png(4, 2, |_, _| [1, 2, 3, 255], None));
    assert_ne!(stamp, file.stamp(), "an edited file has another stamp");
    let d = decode_for_box(&read, Some((1, 1)), false).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height), (1, 1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs posted results until every probe has its size.
fn sized(states: &[remedy_ui_reactive::Signal<ImageState>]) -> Vec<(u32, u32)> {
    let start = Instant::now();
    loop {
        let _ = remedy_ui_reactive::run_posted();
        let sizes: Vec<(u32, u32)> = states
            .iter()
            .filter_map(|s| match s.get_untracked().ok() {
                Some(ImageState::Sized(size)) => Some(size),
                _ => None,
            })
            .collect();
        if sizes.len() == states.len() || start.elapsed() > Duration::from_secs(30) {
            return sizes;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn probed(source: &Source) -> remedy_ui_reactive::Signal<ImageState> {
    let state = remedy_ui_reactive::Signal::new(ImageState::Loading);
    probe(source, state);
    state
}

#[test]
fn new_bytes_at_a_freed_address_get_their_own_size() {
    let _ui = ui();
    // Two images of one length (padding after the end marker is ignored).
    let padded = |w, h| -> Arc<[u8]> {
        let mut v = png(w, h, |_, _| [9, 9, 9, 255], None);
        v.resize(4096, 0);
        Arc::from(v)
    };
    let first = padded(40, 20);
    let addr = first.as_ptr() as usize;
    assert_eq!(sized(&[probed(&Source::Bytes(first.clone()))]), vec![(40, 20)]);
    drop(first);
    // Another probe lets go of the freed bytes' entry, and so of their address.
    let other = padded(1, 1);
    assert_eq!(sized(&[probed(&Source::Bytes(other.clone()))]), vec![(1, 1)]);
    // The allocator tends to hand the freed block straight back; try a few times for it.
    let mut misses = Vec::new();
    let second = loop {
        let b = padded(20, 40);
        if b.as_ptr() as usize == addr || misses.len() > 64 {
            break b;
        }
        misses.push(b);
    };
    assert_eq!(sized(&[probed(&Source::Bytes(second))]), vec![(20, 40)]);
}

#[test]
fn an_edited_file_probes_to_its_new_size() {
    let _ui = ui();
    let dir = std::env::temp_dir().join(format!("remedy-image-probe-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("pic.png");
    let _ = std::fs::write(&path, png(40, 20, |_, _| [255, 0, 0, 255], None));
    let file = Source::File(path.clone());
    assert_eq!(sized(&[probed(&file)]), vec![(40, 20)]);
    std::thread::sleep(Duration::from_millis(20));
    let _ = std::fs::write(&path, png(80, 20, |_, _| [255, 0, 0, 255], None));
    // The remembered size comes first, then the file's new one.
    let state = probed(&file);
    let start = Instant::now();
    while !matches!(state.get_untracked().ok(), Some(ImageState::Sized((80, 20))))
        && start.elapsed() < Duration::from_secs(10)
    {
        let _ = remedy_ui_reactive::run_posted();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        matches!(state.get_untracked().ok(), Some(ImageState::Sized((80, 20)))),
        "{:?}",
        state.get_untracked()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remembered_sizes_stay_bounded() {
    let _ui = ui();
    let bytes = png(3, 2, |_, _| [1, 2, 3, 255], None);
    let sources: Vec<Arc<[u8]>> = (0..10_000).map(|_| Arc::from(&bytes[..])).collect();
    let states: Vec<_> = sources.iter().map(|b| probed(&Source::Bytes(b.clone()))).collect();
    assert_eq!(sized(&states).len(), 10_000);
    // All still alive: only the most recently used are kept.
    assert!(known_sizes() <= 1024, "{}", known_sizes());
    // Freed, their sizes go.
    drop(sources);
    let last: Arc<[u8]> = Arc::from(&bytes[..]);
    let _ = sized(&[probed(&Source::Bytes(last.clone()))]);
    assert_eq!(known_sizes(), 1, "only the source still alive");
    // The app sets how many are kept.
    set_known_sizes_limit(3);
    let more: Vec<Arc<[u8]>> = (0..10).map(|_| Arc::from(&bytes[..])).collect();
    let _ = sized(&more.iter().map(|b| probed(&Source::Bytes(b.clone()))).collect::<Vec<_>>());
    assert_eq!(known_sizes(), 3);
    set_known_sizes_limit(DEFAULT_KNOWN_SIZES_LIMIT);
}

#[test]
fn work_can_be_queued_on_the_image_workers() {
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_job(Arc::new(std::sync::atomic::AtomicBool::new(false)), move || {
        let _ = tx.send(7);
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).ok(), Some(7));
    // A job cancelled before it runs is skipped.
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_job(Arc::new(std::sync::atomic::AtomicBool::new(true)), move || {
        let _ = tx.send(8);
    });
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn a_lazy_image_loads_only_once_it_comes_near_the_viewport() {
    let ui = ui();
    let bytes: Arc<[u8]> = Arc::from(png(64, 64, |_, _| [0, 200, 0, 255], None));
    let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = steps.clone();
    let pic = image_ext(
        &ui,
        Source::Bytes(bytes),
        "Far down",
        Fit::Cover,
        ImageOptions::new().lazy(200.0).on_state(move |s| {
            if !matches!(s, ImageState::Loading) {
                s2.lock().map(|mut v| v.push(format!("{s:?}").chars().take(5).collect::<String>())).ok();
            }
        }),
    )
    .width(80.0)
    .height(80.0);
    let spacer = ui.el("spacer").height(2000.0);
    let scroller = ui.el("scroll").scroll().column().height(600.0).child(spacer).child(pic.clone());
    ui.root().child(scroller.clone());
    settle(&ui);
    assert!(steps.lock().map(|v| v.is_empty()).unwrap_or(false), "1400 px below the fold: nothing loaded");
    assert_eq!(cached(), 0);
    // Scrolled to 150 px above it (within the 200 px margin): it loads.
    ui.set_scroll_offset(scroller.id(), remedy_ui_core::Vec2::new(0.0, 1250.0));
    settle(&ui);
    assert!(steps.lock().map(|v| v.iter().any(|s| s.starts_with("Ready"))).unwrap_or(false), "{steps:?}");
    assert_eq!(cached(), 1);
    ui.root().clear();
    frame(&ui);
    frame(&ui);
}
