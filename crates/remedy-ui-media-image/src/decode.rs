//! Decoding: bytes to premultiplied sRGB pixels at the size they will be shown.

use std::io::Cursor;
use std::sync::Arc;

use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage, imageops};
use remedy_ui_engine::paint::Image;

/// Why an image could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
    /// Its decoder was stopped: decoding in a sandboxed helper
    /// (`remedy-ui-media-isolated`), it crashed, ran far longer than an image this size takes,
    /// or used more memory than the image needs. The reason says which.
    Stopped(String),
    /// It could not be fetched from its address ([`Source::Url`](crate::Source::Url)): no
    /// connection, an error status, or (in a page) a server of another site that does not
    /// allow it. The reason says which.
    Network(String),
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
            DecodeError::Stopped(why) => write!(f, "the image's decoder was stopped: {why}"),
            DecodeError::Network(why) => write!(f, "the image could not be fetched: {why}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A decoded image.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Decoded {
    /// The pixels, premultiplied sRGB, possibly smaller than the source.
    pub image: Image,
    /// The source's size after orientation, in pixels.
    pub source_size: (u32, u32),
}

impl Decoded {
    /// `image`, decoded from a source of `source_size` pixels (after orientation).
    pub fn new(image: Image, source_size: (u32, u32)) -> Self {
        Decoded { image, source_size }
    }
}

/// A format added to the built-in raster ones (SVG, from the `svg` module).
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

/// True when `bytes` are in a format that draws sharply at any size (an added vector format
/// such as SVG): [`fitted_scalable`] may then size them larger than their natural size.
pub fn is_scalable(bytes: &[u8]) -> bool {
    format_of(bytes).is_some_and(|f| f.scalable())
}

/// Decoding done somewhere other than this process: `remedy-ui-media-isolated` decodes in a
/// sandboxed helper. Once one is set with [`set_router`], the image loader ([`load`],
/// [`probe`], [`image()`]) and [`decode_for_box`] read sizes and decode through it; [`decode`]
/// and [`dimensions`] always decode here (they are what a router runs on the other side).
///
/// [`load`]: crate::load
/// [`probe`]: crate::probe
/// [`image()`]: crate::image
/// [`decode_for_box`]: crate::decode_for_box
pub trait Router: Send + Sync {
    /// The image's natural size, as [`dimensions`] reads it.
    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError>;
    /// Decodes as [`decode_for_box`](crate::decode_for_box) does: to show in a `box_px` box of
    /// device pixels (`None`: at its own size), fitting inside it or (`cover`) covering it.
    fn decode_for_box(
        &self,
        bytes: &[u8],
        box_px: Option<(u32, u32)>,
        cover: bool,
    ) -> Result<Decoded, DecodeError>;
}

fn router_slot() -> &'static std::sync::RwLock<Option<Arc<dyn Router>>> {
    static R: std::sync::OnceLock<std::sync::RwLock<Option<Arc<dyn Router>>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

/// Routes the image loader's decoding through `router` from now on; `None` decodes in this
/// process again. Loads already started finish where they began.
pub fn set_router(router: Option<Arc<dyn Router>>) {
    let mut slot = router_slot().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = router;
}

/// The router decoding goes through, when one is set.
pub fn router() -> Option<Arc<dyn Router>> {
    router_slot().read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
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
    let rgba: RgbaImage = img.into_rgba8();
    let srgb = icc.as_deref().and_then(Srgb::from_profile);
    let rgba = finish(rgba, srgb.as_ref(), show_at);
    let (dw, dh) = rgba.dimensions();
    Ok(Decoded { image: Image::new(dw, dh, Arc::from(rgba.into_raw())), source_size })
}

/// Straight-alpha RGBA to what is drawn: converted to sRGB, premultiplied, and scaled down to
/// `show_at` when that is smaller.
fn finish(mut rgba: RgbaImage, srgb: Option<&Srgb>, show_at: Option<(u32, u32)>) -> RgbaImage {
    if let Some(t) = srgb {
        t.apply(&mut rgba);
    }
    premultiply(&mut rgba);
    match show_at {
        Some((tw, th)) if tw > 0 && th > 0 && (tw < rgba.width() || th < rgba.height()) => {
            let (tw, th) = (tw.min(rgba.width()), th.min(rgba.height()));
            scale_down(&rgba, tw, th)
        }
        _ => rgba,
    }
}

/// Premultiplied pixels scaled down to `tw`×`th` with a Lanczos-3 filter, the filter browsers
/// downscale pictures with as they decode them (Firefox, Skia), so an image looks as sharp as
/// an `<img>` of that size. A picture many times larger is first averaged down to twice the
/// size (cheap, and exact enough for what Lanczos then does).
fn scale_down(rgba: &RgbaImage, tw: u32, th: u32) -> RgbaImage {
    let staged;
    let from = if rgba.width() >= tw * 4 && rgba.height() >= th * 4 {
        staged = imageops::thumbnail(rgba, tw * 2, th * 2);
        &staged
    } else {
        rgba
    };
    let mut out = imageops::resize(from, tw, th, imageops::FilterType::Lanczos3);
    // Lanczos rings: keep each color within its alpha, as premultiplied pixels must be.
    for [r, g, b, a] in out.as_chunks_mut::<4>().0 {
        *r = (*r).min(*a);
        *g = (*g).min(*a);
        *b = (*b).min(*a);
    }
    out
}

/// Makes `width`×`height` straight-alpha RGBA8 pixels (`rgba`, row by row) ready to draw, the
/// way [`decode`] does: converted to sRGB by `profile` when there is one, premultiplied, and
/// scaled down to `show_at` when that is smaller (never up). For decoders of other formats,
/// such as the frames of animations.
pub fn finish_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
    profile: Option<&Srgb>,
    show_at: Option<(u32, u32)>,
) -> Result<Image, DecodeError> {
    let mut copy: Vec<u8> = Vec::new();
    copy.try_reserve_exact(rgba.len()).map_err(|_| DecodeError::TooLarge { width, height })?;
    copy.extend_from_slice(rgba);
    let img = RgbaImage::from_raw(width, height, copy).ok_or_else(|| {
        DecodeError::Malformed(format!("{width}×{height} pixels need another amount of data"))
    })?;
    let img = finish(img, profile, show_at);
    let (w, h) = img.dimensions();
    Ok(Image::new(w, h, Arc::from(img.into_raw())))
}

/// A conversion from an ICC color profile to sRGB, made once and applied to many pixels
/// (every frame of an animation).
#[derive(Clone)]
pub struct Srgb(Arc<moxcms::Transform8BitExecutor>);

impl std::fmt::Debug for Srgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Srgb")
    }
}

impl Srgb {
    /// The conversion for `profile`, or `None` when it cannot be read or does not describe RGB
    /// (the pixels are then taken to be sRGB, as browsers do).
    pub fn from_profile(profile: &[u8]) -> Option<Srgb> {
        use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
        let src = ColorProfile::new_from_slice(profile).ok()?;
        if src.color_space != DataColorSpace::Rgb {
            return None;
        }
        let dst = ColorProfile::new_srgb();
        let t =
            src.create_transform_8bit(Layout::Rgba, &dst, Layout::Rgba, TransformOptions::default()).ok()?;
        Some(Srgb(t))
    }

    /// Converts straight-alpha RGBA pixels in place.
    fn apply(&self, rgba: &mut RgbaImage) {
        let mut out = vec![0u8; rgba.as_raw().len()];
        if self.0.transform(rgba.as_raw(), &mut out).is_ok() {
            rgba.copy_from_slice(&out);
        }
    }
}

/// Straight-alpha RGBA pixels to premultiplied, in place.
pub(crate) fn premultiply(rgba: &mut [u8]) {
    for [r, g, b, a] in rgba.as_chunks_mut::<4>().0 {
        if *a < 255 {
            let m = |c: u8| ((u16::from(c) * u16::from(*a) + 127) / 255) as u8;
            *r = m(*r);
            *g = m(*g);
            *b = m(*b);
        }
    }
}
