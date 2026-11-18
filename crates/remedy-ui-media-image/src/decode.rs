//! Decoding: bytes to premultiplied sRGB pixels at the size they will be shown.

use std::io::Cursor;
use std::sync::Arc;

use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage, imageops};
use remedy_ui_paint::Image;

/// Why an image could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a format this build reads.
    Unrecognized,
    /// The file is damaged or not what it claims to be.
    Malformed(String),
    /// Its pixels do not fit in the memory this machine has free.
    TooLarge {
        /// Its width.
        width: u32,
        /// Its height.
        height: u32,
    },
    /// The file could not be read.
    Io(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Unrecognized => f.write_str("not an image format this app reads"),
            DecodeError::Malformed(e) => write!(f, "the image is damaged: {e}"),
            DecodeError::TooLarge { width, height } => {
                write!(f, "the image ({width}×{height}) is too large for the memory available")
            }
            DecodeError::Io(e) => write!(f, "the image could not be read: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A decoded image.
#[derive(Clone, Debug)]
pub struct Decoded {
    /// The pixels, premultiplied sRGB, possibly smaller than the source.
    pub image: Image,
    /// The source's size after orientation, in pixels.
    pub source_size: (u32, u32),
}

/// A format added to the built-in raster ones (SVG, from `remedy-ui-media-svg`).
pub trait Format: Send + Sync {
    /// True when `head` (the first bytes of a file, at least 512 when the file has them)
    /// is this format.
    fn sniff(&self, head: &[u8]) -> bool;
    /// The image's natural size in pixels.
    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError>;
    /// Decodes, at `show_at` when given (which may be larger than the natural size when the
    /// format is [`Format::scalable`]).
    fn decode(&self, bytes: &[u8], show_at: Option<(u32, u32)>) -> Result<Decoded, DecodeError>;
    /// Draws sharply at any size (vector formats).
    fn scalable(&self) -> bool {
        false
    }
}

fn formats() -> &'static std::sync::RwLock<Vec<Arc<dyn Format>>> {
    static F: std::sync::OnceLock<std::sync::RwLock<Vec<Arc<dyn Format>>>> = std::sync::OnceLock::new();
    F.get_or_init(Default::default)
}

/// Adds a format: [`decode`], [`dimensions`] and the image element read it from then on.
pub fn add_format(f: Arc<dyn Format>) {
    if let Ok(mut v) = formats().write() {
        v.push(f);
    }
}

/// The added format `head` is in, if any.
pub(crate) fn format_of(head: &[u8]) -> Option<Arc<dyn Format>> {
    formats().read().ok()?.iter().find(|f| f.sniff(head)).cloned()
}

/// The size of an image with `source` pixels drawn to fit inside (`cover` false) or to cover
/// (`cover` true) a `box_px` box, never larger than the source: the size to decode at.
pub fn fitted(source: (u32, u32), box_px: (u32, u32), cover: bool) -> (u32, u32) {
    fitted_scalable(source, box_px, cover, false)
}

/// Like [`fitted`]; a `scalable` image may be drawn larger than its natural size.
pub fn fitted_scalable(source: (u32, u32), box_px: (u32, u32), cover: bool, scalable: bool) -> (u32, u32) {
    let (sw, sh) = (source.0.max(1) as f64, source.1.max(1) as f64);
    let (bw, bh) = (box_px.0.max(1) as f64, box_px.1.max(1) as f64);
    let s = if cover { (bw / sw).max(bh / sh) } else { (bw / sw).min(bh / sh) };
    let s = if scalable { s } else { s.min(1.0) };
    (((sw * s).round() as u32).max(1), ((sh * s).round() as u32).max(1))
}

fn malformed(e: image::ImageError) -> DecodeError {
    match e {
        image::ImageError::Unsupported(_) => DecodeError::Unrecognized,
        image::ImageError::IoError(e) => DecodeError::Io(e.to_string()),
        other => DecodeError::Malformed(other.to_string()),
    }
}

/// Reads the source's size (after orientation) without decoding its pixels.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
    if let Some(f) = format_of(bytes) {
        return f.dimensions(bytes);
    }
    dimensions_from(Cursor::new(bytes))
}

/// Like [`dimensions`] for the built-in raster formats, reading only as much of `r` as the
/// header needs.
pub fn dimensions_from<R: std::io::BufRead + std::io::Seek>(r: R) -> Result<(u32, u32), DecodeError> {
    let reader = ImageReader::new(r).with_guessed_format().map_err(|e| DecodeError::Io(e.to_string()))?;
    if reader.format().is_none() {
        return Err(DecodeError::Unrecognized);
    }
    let mut dec = reader.into_decoder().map_err(malformed)?;
    let (w, h) = dec.dimensions();
    let turned = dec.orientation().is_ok_and(|o| {
        use image::metadata::Orientation as O;
        matches!(o, O::Rotate90 | O::Rotate270 | O::Rotate90FlipH | O::Rotate270FlipH)
    });
    Ok(if turned { (h, w) } else { (w, h) })
}

/// Decodes `bytes`. With `show_at`, the result is scaled down to that many pixels (when the
/// source is larger; pass [`fitted`]'s answer), which keeps thumbnails of large photos small.
///
/// The image is converted to sRGB when it carries a color profile, turned upright by its EXIF
/// orientation, and premultiplied. Memory is sized from the image: there is no fixed limit,
/// and an image whose pixels cannot be allocated is [`DecodeError::TooLarge`], never an abort.
pub fn decode(bytes: &[u8], show_at: Option<(u32, u32)>) -> Result<Decoded, DecodeError> {
    if let Some(f) = format_of(bytes) {
        return f.decode(bytes, show_at);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| DecodeError::Io(e.to_string()))?;
    if reader.format().is_none() {
        return Err(DecodeError::Unrecognized);
    }
    reader.no_limits();
    let mut dec = reader.into_decoder().map_err(malformed)?;
    let (w, h) = dec.dimensions();
    let icc = dec.icc_profile().ok().flatten();
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    // The decoded pixels, plus their RGBA copy: reserve both before the decoder allocates,
    // so a picture too large for this machine is an error and not an abort.
    let need = dec.total_bytes().saturating_add(u64::from(w) * u64::from(h) * 4);
    let need = usize::try_from(need).map_err(|_| DecodeError::TooLarge { width: w, height: h })?;
    {
        let mut probe: Vec<u8> = Vec::new();
        probe.try_reserve_exact(need).map_err(|_| DecodeError::TooLarge { width: w, height: h })?;
    }
    let mut img = DynamicImage::from_decoder(dec).map_err(malformed)?;
    img.apply_orientation(orientation);
    let source_size = (img.width(), img.height());
    let mut rgba: RgbaImage = img.into_rgba8();
    if let Some(icc) = icc {
        to_srgb(&mut rgba, &icc);
    }
    premultiply(&mut rgba);
    let rgba = match show_at {
        Some((tw, th)) if tw > 0 && th > 0 && (tw < rgba.width() || th < rgba.height()) => {
            let (tw, th) = (tw.min(rgba.width()), th.min(rgba.height()));
            // Premultiplied, so transparent pixels do not bleed their color into edges.
            if rgba.width() >= tw * 2 && rgba.height() >= th * 2 {
                imageops::thumbnail(&rgba, tw, th)
            } else {
                imageops::resize(&rgba, tw, th, imageops::FilterType::Triangle)
            }
        }
        _ => rgba,
    };
    let (dw, dh) = rgba.dimensions();
    Ok(Decoded { image: Image::new(dw, dh, Arc::from(rgba.into_raw())), source_size })
}

/// Converts straight-alpha RGBA pixels described by the ICC `profile` to sRGB in place. A
/// profile that cannot be read, or that does not describe RGB, leaves the pixels as they
/// are (they are then taken to be sRGB, as browsers do).
fn to_srgb(rgba: &mut RgbaImage, profile: &[u8]) {
    use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
    let Ok(src) = ColorProfile::new_from_slice(profile) else { return };
    if src.color_space != DataColorSpace::Rgb {
        return;
    }
    let dst = ColorProfile::new_srgb();
    let Ok(t) = src.create_transform_8bit(Layout::Rgba, &dst, Layout::Rgba, TransformOptions::default())
    else {
        return;
    };
    let mut out = vec![0u8; rgba.as_raw().len()];
    if t.transform(rgba.as_raw(), &mut out).is_ok() {
        rgba.copy_from_slice(&out);
    }
}

fn premultiply(rgba: &mut RgbaImage) {
    for [r, g, b, a] in rgba.as_chunks_mut::<4>().0 {
        if *a < 255 {
            let m = |c: u8| ((u16::from(c) * u16::from(*a) + 127) / 255) as u8;
            *r = m(*r);
            *g = m(*g);
            *b = m(*b);
        }
    }
}
