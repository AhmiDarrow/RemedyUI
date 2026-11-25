//! SVG, drawn by resvg at exactly the size shown (the `svg` feature).
//!
//! - [`install`] adds SVG to the image loader, so [`image()`](crate::media_image::image()) shows SVG
//!   files and bytes like any other image, sharp at every size
//! - [`render`] draws an SVG to pixels
//! - [`icon`] draws an icon in the element's text color (`currentColor`)
//! - [`add_font`] adds the app's fonts for text in SVG files (with the `svg-text` feature,
//!   which the facade's `svg` feature enables, and which also uses the system's fonts off the web)
//!
//! SVG files are untrusted: references to other files and to the network are never
//! followed; only images embedded as `data:` URLs are drawn.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};

use crate::media_image::{DecodeError, Decoded, Format, add_format};
use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_engine::paint::{Image, Item, Sampling};
use remedy_ui_engine::view::{El, Ui};
use resvg::{tiny_skia, usvg};

fn options(color: Option<Color>) -> usvg::Options<'static> {
    // Never read other files or reach the network for `<image href=…>`.
    let mut o = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            ..usvg::ImageHrefResolver::default()
        },
        ..usvg::Options::default()
    };
    if let Some(c) = color {
        let hex = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        o.style_sheet = Some(format!("svg {{ color: #{:02x}{:02x}{:02x}; }}", hex(c.r), hex(c.g), hex(c.b)));
    }
    #[cfg(feature = "media-image-svg-text")]
    {
        o.fontdb = fonts::database();
        o.font_resolver.select_font = fonts::select_font();
    }
    o
}

#[cfg(feature = "media-image-svg-text")]
pub use fonts::add_font;

#[cfg(feature = "media-image-svg-text")]
mod fonts {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, LazyLock, Mutex, PoisonError};

    use resvg::usvg::{self, FontFamily, FontStyle, fontdb};

    /// The one font database every parse uses: the system's fonts plus the app's.
    static DB: LazyLock<Mutex<Arc<fontdb::Database>>> = LazyLock::new(|| {
        #[allow(unused_mut, reason = "no system fonts on the web")]
        let mut db = fontdb::Database::new();
        #[cfg(not(target_arch = "wasm32"))]
        db.load_system_fonts();
        Mutex::new(Arc::new(db))
    });

    /// Whether the app has added a font yet (its first family becomes the sans-serif default).
    static ADDED: AtomicBool = AtomicBool::new(false);

    pub(crate) fn database() -> Arc<fontdb::Database> {
        DB.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Adds a font (a TrueType/OpenType file, or a collection) for text in SVG files: the app's
    /// own faces, e.g. from `remedy-ui-fonts`, so SVG text uses the app's fonts. The first
    /// family added becomes the default sans-serif family, which text naming a missing family
    /// falls back to. Applies to later renders only, and [`icon`](super::icon) keeps its last
    /// drawing until its size or color changes, so add fonts at start-up.
    pub fn add_font(data: Vec<u8>) {
        let mut guard = DB.lock().unwrap_or_else(PoisonError::into_inner);
        let db = Arc::make_mut(&mut guard);
        let first = !ADDED.swap(true, Ordering::Relaxed);
        let ids = db.load_font_source(fontdb::Source::Binary(Arc::new(data)));
        if first {
            let family = ids.first().and_then(|id| db.face(*id)).and_then(|f| f.families.first());
            if let Some((name, _)) = family.cloned() {
                db.set_sans_serif_family(name);
            }
        }
    }

    /// usvg's selection, except a family the database lacks falls back to sans-serif, not serif.
    pub(crate) fn select_font() -> usvg::FontSelectionFn<'static> {
        let default = usvg::FontResolver::default_font_selector();
        Box::new(move |font, db| {
            let known = font.families().iter().any(|f| match f {
                FontFamily::Named(n) => {
                    db.faces().any(|face| face.families.iter().any(|(name, _)| name == n))
                }
                _ => true,
            });
            if known {
                return default(font, db);
            }
            let style = match font.style() {
                FontStyle::Normal => fontdb::Style::Normal,
                FontStyle::Italic => fontdb::Style::Italic,
                FontStyle::Oblique => fontdb::Style::Oblique,
            };
            db.query(&fontdb::Query {
                families: &[fontdb::Family::SansSerif, fontdb::Family::Serif],
                weight: fontdb::Weight(font.weight()),
                style,
                ..fontdb::Query::default()
            })
        })
    }
}

fn parse(bytes: &[u8], color: Option<Color>) -> Result<usvg::Tree, DecodeError> {
    usvg::Tree::from_data(bytes, &options(color)).map_err(|e| match e {
        usvg::Error::NotAnUtf8Str | usvg::Error::MalformedGZip => DecodeError::Malformed(e.to_string()),
        other => DecodeError::Malformed(other.to_string()),
    })
}

/// The SVG's natural size in pixels (its width and height, or its view box).
pub fn size(bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
    let t = parse(bytes, None)?;
    let s = t.size();
    Ok(((s.width().ceil() as u32).max(1), (s.height().ceil() as u32).max(1)))
}

/// Draws the SVG at `px` pixels (its natural size when `None`), stretched to that box;
/// `color` is what `currentColor` means. Premultiplied, ready to draw.
pub fn render(bytes: &[u8], px: Option<(u32, u32)>, color: Option<Color>) -> Result<Image, DecodeError> {
    let tree = parse(bytes, color)?;
    let natural = tree.size();
    let (w, h) =
        px.unwrap_or(((natural.width().ceil() as u32).max(1), (natural.height().ceil() as u32).max(1)));
    let (w, h) = (w.max(1), h.max(1));
    // The pixels, sized by need: an impossible size is an error, not an abort.
    let need = usize::try_from(u64::from(w) * u64::from(h) * 4)
        .map_err(|_| DecodeError::TooLarge { width: w, height: h })?;
    {
        let mut probe: Vec<u8> = Vec::new();
        probe.try_reserve_exact(need).map_err(|_| DecodeError::TooLarge { width: w, height: h })?;
    }
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or(DecodeError::TooLarge { width: w, height: h })?;
    let t = tiny_skia::Transform::from_scale(w as f32 / natural.width(), h as f32 / natural.height());
    resvg::render(&tree, t, &mut pixmap.as_mut());
    // tiny-skia's pixels are premultiplied RGBA8, as the renderers want.
    Ok(Image::new(w, h, Arc::from(pixmap.take())))
}

/// How far into a file the sniffer [`install`] adds looks for the root element, by default:
/// past the XML declaration, comments, processing instructions and DOCTYPE in front of it.
pub const DEFAULT_SNIFF_LIMIT: usize = 64 * 1024;

static SNIFF_LIMIT: AtomicUsize = AtomicUsize::new(DEFAULT_SNIFF_LIMIT);

/// Sets how many bytes the sniffer reads looking for the `<svg>` root element (default
/// [`DEFAULT_SNIFF_LIMIT`]): a file whose prolog runs longer is not recognised as SVG.
pub fn set_sniff_limit(bytes: usize) {
    SNIFF_LIMIT.store(bytes, Ordering::Relaxed);
}

/// How many bytes the sniffer reads looking for the root element.
pub fn sniff_limit() -> usize {
    SNIFF_LIMIT.load(Ordering::Relaxed)
}

fn looks_like_svg(head: &[u8]) -> bool {
    if head.starts_with(&[0x1f, 0x8b]) {
        // Gzipped: .svgz (no raster format is gzipped).
        return cfg!(feature = "media-image-svgz");
    }
    root_is_svg(head.get(..head.len().min(sniff_limit())).unwrap_or_default()).unwrap_or(false)
}

/// Whether the document's root element is `svg` (any namespace prefix), skipping a byte order
/// mark, whitespace, the XML declaration, processing instructions, comments and a DOCTYPE
/// (internal subset included). `None` when `b` ends before the root element.
fn root_is_svg(b: &[u8]) -> Option<bool> {
    let find = |from: usize, pat: &[u8]| {
        b.get(from..)?.windows(pat.len()).position(|w| w == pat).map(|p| from + p + pat.len())
    };
    let mut i = if b.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };
    loop {
        while b.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        let r = b.get(i..)?;
        if r.starts_with(b"<?") {
            i = find(i + 2, b"?>")?;
        } else if r.starts_with(b"<!--") {
            i = find(i + 4, b"-->")?;
        } else if r.starts_with(b"<!DOCTYPE") {
            i = doctype_end(b, i + 9)?;
        } else if r.starts_with(b"<") {
            let name = r.get(1..)?;
            let end = name.iter().position(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/'))?;
            let name = name.get(..end)?;
            let local = name.rsplit(|&c| c == b':').next().unwrap_or(name);
            return Some(local == b"svg");
        } else {
            return Some(false);
        }
    }
}

/// Just past the `>` closing a DOCTYPE whose body starts at `i`: quoted strings, comments and
/// the `[...]` internal subset may hold a `>` of their own.
fn doctype_end(b: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0usize;
    loop {
        let c = *b.get(i)?;
        if b.get(i..)?.starts_with(b"<!--") {
            i += 4 + b.get(i + 4..)?.windows(3).position(|w| w == b"-->")? + 3;
            continue;
        }
        match c {
            b'"' | b'\'' => i += 1 + b.get(i + 1..)?.iter().position(|&q| q == c)?,
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b'>' if depth == 0 => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
}

struct Svg;

impl Format for Svg {
    fn sniff(&self, head: &[u8]) -> bool {
        looks_like_svg(head)
    }

    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
        size(bytes)
    }

    fn decode(&self, bytes: &[u8], show_at: Option<(u32, u32)>) -> Result<Decoded, DecodeError> {
        let natural = size(bytes)?;
        let image = render(bytes, show_at, None)?;
        Ok(Decoded::new(image, natural))
    }

    fn scalable(&self) -> bool {
        true
    }
}

/// Adds SVG to the image loader (once; later calls do nothing).
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| add_format(Arc::new(Svg)));
}

/// What an icon was drawn for: pixel width and height, and color.
type RenderKey = (u32, u32, [u8; 4]);

/// An icon from SVG `bytes`, drawn in the element's text color (the SVG's `currentColor`) at
/// `size` logical pixels square, sharp at every scale. `label` names it for screen readers;
/// an empty label marks it decorative.
#[track_caller]
pub fn icon(ui: &Ui, bytes: Arc<[u8]>, size: f32, label: impl Into<String>) -> El {
    let label = label.into();
    // The last rendering, kept while the size, scale and color stay the same.
    let cache: RefCell<Option<(RenderKey, Image)>> = RefCell::new(None);
    let sem = if label.is_empty() {
        Semantics::new(Role::Image).hidden(true)
    } else {
        Semantics::new(Role::Image).label(label)
    };
    ui.el("icon").width(size).height(size).semantics(sem).paint(move |cx| {
        let px = ((cx.size.width * cx.scale).round() as u32, (cx.size.height * cx.scale).round() as u32);
        if px.0 == 0 || px.1 == 0 {
            return;
        }
        let c = cx.style.foreground;
        let key = (px.0, px.1, c.to_rgba8());
        let hit = cache.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, i)| i.clone());
        let img = match hit {
            Some(i) => i,
            None => match render(&bytes, Some(px), Some(c)) {
                Ok(i) => {
                    *cache.borrow_mut() = Some((key, i.clone()));
                    i
                }
                // A broken icon draws nothing; the label still names it.
                Err(_) => return,
            },
        };
        cx.list.push(Item::Image {
            image: img,
            rect: Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
            radii: CornerRadii::ZERO,
            sampling: Sampling::Linear,
        });
    })
}

#[cfg(test)]
mod sniff_tests {
    use super::root_is_svg;

    #[test]
    fn root_element_found_past_the_prolog() {
        assert_eq!(root_is_svg(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#), Some(true));
        let comment = format!("<!-- {} -->\n<svg/>", "x".repeat(5000));
        assert_eq!(root_is_svg(comment.as_bytes()), Some(true), "a long comment first");
        let bom = [&[0xef, 0xbb, 0xbf][..], b"  <?xml version=\"1.0\"?>\n<svg>"].concat();
        assert_eq!(root_is_svg(&bom), Some(true), "a byte order mark and declaration");
        let doctype = br#"<?xml version="1.0"?><?xml-stylesheet href="a.css"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
  <!ENTITY a "<b>">
  <!-- it's ] here -->
]>
<svg:svg xmlns:svg="http://www.w3.org/2000/svg">"#;
        assert_eq!(root_is_svg(doctype), Some(true), "a DOCTYPE with an internal subset");
    }

    #[test]
    fn other_documents_are_not_svg() {
        assert_eq!(root_is_svg(br#"<?xml version="1.0"?><html><svg/></html>"#), Some(false));
        assert_eq!(root_is_svg(b"<!-- <svg> --><feed>"), Some(false), "svg only in a comment");
        assert_eq!(root_is_svg(b"<svgx/>"), Some(false));
        assert_eq!(root_is_svg(&[0x89, b'P', b'N', b'G']), Some(false));
        assert_eq!(root_is_svg(b"<!-- unfinished"), None);
        assert_eq!(root_is_svg(b""), None);
    }
}
