//! RemedyUI SVG: drawn by resvg at exactly the size shown.
//!
//! - [`install`] adds SVG to the image loader, so [`remedy_ui_media_image::image()`] shows SVG
//!   files and bytes like any other image, sharp at every size
//! - [`render`] draws an SVG to pixels
//! - [`icon`] draws an icon in the element's text color (`currentColor`)
//!
//! SVG files are untrusted: references to other files and to the network are never
//! followed; only images embedded as `data:` URLs are drawn.

use std::cell::RefCell;
use std::sync::{Arc, Once};

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_media_image::{DecodeError, Decoded, Format, add_format};
use remedy_ui_paint::{Image, Item, Sampling};
use remedy_ui_view::{El, Ui};
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
    #[cfg(feature = "text")]
    {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        o.fontdb = Arc::new(db);
    }
    o
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

fn looks_like_svg(head: &[u8]) -> bool {
    if head.starts_with(&[0x1f, 0x8b]) {
        // Gzipped: .svgz (no raster format is gzipped).
        return cfg!(feature = "svgz");
    }
    let text = String::from_utf8_lossy(head.get(..head.len().min(1024)).unwrap_or_default());
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE") || t.starts_with("<svg"))
        && t.contains("<svg")
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
