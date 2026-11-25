//! The animated formats: each opens a file and hands out its frames one at a time, composited
//! onto a canvas the size of the picture (disposal and blending done as the format says).

#[cfg(any(feature = "media-anim-gif", feature = "media-anim-apng", feature = "media-anim-webp"))]
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use crate::media_image::{DecodeError, Srgb};

pub(crate) use crate::media_anim::route::Loops;

/// What a file says about its animation, once opened.
pub(crate) struct Info {
    /// The size of the picture in pixels.
    pub size: (u32, u32),
    /// How many frames one play has, when the file says.
    pub frames: Option<u64>,
    /// How many times it plays.
    pub loops: Loops,
    /// The conversion to sRGB, when the file carries a color profile.
    pub profile: Option<Srgb>,
}

/// One frame: the whole picture as it looks, and how long it stays.
pub(crate) struct Frame<'a> {
    /// Straight-alpha RGBA8 at the picture's size.
    pub rgba: &'a [u8],
    /// How long it is shown.
    pub delay: Duration,
}

/// A file being read frame by frame. Moves between threads (it is handed to whichever worker
/// decodes next), but is used by one at a time.
pub(crate) trait Frames: Send {
    fn info(&self) -> &Info;
    /// The next frame, or `None` after the last of this play.
    fn next(&mut self) -> Result<Option<Frame<'_>>, DecodeError>;
    /// Back to the first frame.
    fn restart(&mut self) -> Result<(), DecodeError>;
}

/// What a file turned out to be.
pub(crate) enum Opened {
    /// Frames to play.
    // A build with no animated format opens none.
    #[cfg_attr(
        not(any(feature = "media-anim-gif", feature = "media-anim-apng", feature = "media-anim-webp")),
        allow(dead_code)
    )]
    Animated(Box<dyn Frames>),
    /// One picture (a still image of any format the image loader reads).
    Still,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gif,
    Apng,
    Webp,
    Still,
}

/// Which animated format `bytes` start as. Stills of every format (and animations whose
/// format this build leaves out) are `Still`: the image loader shows their first picture.
fn sniff(bytes: &[u8]) -> Kind {
    if cfg!(feature = "media-anim-gif") && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Kind::Gif;
    }
    if cfg!(feature = "media-anim-apng") && has_actl(bytes) {
        return Kind::Apng;
    }
    if cfg!(feature = "media-anim-webp") && is_animated_webp(bytes) {
        return Kind::Webp;
    }
    Kind::Still
}

/// True for a PNG with an animation control chunk before its image data.
fn has_actl(bytes: &[u8]) -> bool {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return false;
    }
    let mut at = SIGNATURE.len();
    while let Some(&[l0, l1, l2, l3, t0, t1, t2, t3]) = bytes.get(at..).and_then(<[u8]>::first_chunk::<8>) {
        let len = u32::from_be_bytes([l0, l1, l2, l3]);
        match &[t0, t1, t2, t3] {
            b"acTL" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        at = match at.checked_add(12).and_then(|a| a.checked_add(len as usize)) {
            Some(next) => next,
            None => return false,
        };
    }
    false
}

/// True for a WebP whose extended header says it animates.
fn is_animated_webp(bytes: &[u8]) -> bool {
    bytes.starts_with(b"RIFF")
        && bytes.get(8..16).is_some_and(|s| s == b"WEBPVP8X")
        && bytes.get(20).is_some_and(|f| f & 0x02 != 0)
}

/// Opens `bytes`: its frames if it is an animated format this build reads, else `Still`.
pub(crate) fn open(bytes: &Arc<[u8]>) -> Result<Opened, DecodeError> {
    match sniff(bytes) {
        #[cfg(feature = "media-anim-gif")]
        Kind::Gif => Ok(Opened::Animated(Box::new(gif::Gif::open(bytes.clone())?))),
        #[cfg(feature = "media-anim-apng")]
        Kind::Apng => Ok(Opened::Animated(Box::new(apng::Apng::open(bytes.clone())?))),
        #[cfg(feature = "media-anim-webp")]
        Kind::Webp => match webp::Webp::open(bytes.clone())? {
            Some(w) => Ok(Opened::Animated(Box::new(w))),
            None => Ok(Opened::Still),
        },
        _ => Ok(Opened::Still),
    }
}

/// Makes the still-image loader show the first frame of an APNG, not its fallback image: a
/// file whose first image is not part of the animation (a "thumbnail" for readers that cannot
/// animate) would otherwise show that. GIF and WebP already show their first frame.
#[cfg(feature = "media-anim-apng")]
pub(crate) struct ApngFirstFrame;

#[cfg(feature = "media-anim-apng")]
impl crate::media_image::Format for ApngFirstFrame {
    fn sniff(&self, head: &[u8]) -> bool {
        has_actl(head)
    }

    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
        let mut dec = ::png::Decoder::new(Cursor::new(bytes));
        let info = dec.read_header_info().map_err(|e| DecodeError::Malformed(e.to_string()))?;
        Ok(info.size())
    }

    fn decode(
        &self,
        bytes: &[u8],
        show_at: Option<(u32, u32)>,
    ) -> Result<crate::media_image::Decoded, DecodeError> {
        let mut apng = apng::Apng::open(Arc::from(bytes))?;
        let size = apng.info().size;
        let profile = apng.info().profile.clone();
        let frame = apng
            .next()?
            .ok_or_else(|| DecodeError::Malformed(String::from("the animation has no frames")))?;
        let image = crate::media_image::finish_rgba(size.0, size.1, frame.rgba, profile.as_ref(), show_at)?;
        Ok(crate::media_image::Decoded::new(image, size))
    }
}

/// The delay the picture plays for: a frame never stays less than a millisecond.
#[cfg(any(feature = "media-anim-gif", feature = "media-anim-apng", feature = "media-anim-webp"))]
fn millis(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[cfg(feature = "media-anim-gif")]
mod gif {
    use super::*;
    use crate::media_anim::canvas::{Canvas, Region};

    /// What happens to the canvas before the next frame is drawn.
    enum Dispose {
        Nothing,
        /// The frame's area becomes transparent.
        Clear(Region),
        /// The canvas goes back to how it was before the frame.
        Restore,
    }

    pub(super) struct Gif {
        bytes: Arc<[u8]>,
        dec: ::gif::Decoder<Cursor<Arc<[u8]>>>,
        canvas: Canvas,
        undo: Dispose,
        saved: Option<Vec<u8>>,
        info: Info,
        /// The first frame is read when the file opens (a loop count may only be known then);
        /// it is waiting here until asked for.
        first: Option<Duration>,
    }

    fn bad(e: ::gif::DecodingError) -> DecodeError {
        match e {
            ::gif::DecodingError::Io(e) => DecodeError::Io(e.to_string()),
            other => DecodeError::Malformed(other.to_string()),
        }
    }

    /// The delay a GIF frame (in hundredths of a second) plays for. Browsers show frames that
    /// ask for less than 20 ms for 100 ms: authors of old tools wrote 0 and 1 for "fast".
    fn delay(centis: u16) -> Duration {
        let ms = u64::from(centis) * 10;
        millis(if ms < 20 { 100 } else { ms })
    }

    /// How many frames the file has, counted without decoding their pixels.
    fn count_frames(bytes: &Arc<[u8]>) -> Result<u64, DecodeError> {
        let mut opts = ::gif::DecodeOptions::new();
        opts.skip_frame_decoding(true);
        let mut scan = opts.read_info(Cursor::new(bytes.clone())).map_err(bad)?;
        let mut n = 0;
        // A file damaged part of the way counts the frames before the damage.
        while let Ok(Some(_)) = scan.next_frame_info() {
            n += 1;
        }
        Ok(n)
    }

    impl Gif {
        pub(super) fn open(bytes: Arc<[u8]>) -> Result<Gif, DecodeError> {
            let count = count_frames(&bytes)?;
            if count == 0 {
                return Err(DecodeError::Malformed(String::from("the animation has no frames")));
            }
            let mut opts = ::gif::DecodeOptions::new();
            opts.set_color_output(::gif::ColorOutput::RGBA);
            // Memory is sized from the picture, and the canvas below is reserved fallibly.
            opts.set_memory_limit(::gif::MemoryLimit::Unlimited);
            opts.check_frame_consistency(true);
            let dec = opts.read_info(Cursor::new(bytes.clone())).map_err(bad)?;
            let (w, h) = (u32::from(dec.width()), u32::from(dec.height()));
            if w == 0 || h == 0 {
                return Err(DecodeError::Malformed(String::from("the picture has no size")));
            }
            let mut gif = Gif {
                bytes,
                dec,
                canvas: Canvas::new(w, h)?,
                undo: Dispose::Nothing,
                saved: None,
                info: Info { size: (w, h), frames: Some(count), loops: Loops::Times(1), profile: None },
                first: None,
            };
            gif.first = gif.draw_next()?;
            // Extensions before the first frame have been read: the loop count is known.
            gif.info.loops = match gif.dec.repeat() {
                ::gif::Repeat::Infinite => Loops::Forever,
                // The count is how many times to repeat after the first play.
                ::gif::Repeat::Finite(n) => Loops::Times(u32::from(n) + 1),
            };
            Ok(gif)
        }

        /// Reads the next frame and draws it on the canvas; its delay, or `None` at the end.
        fn draw_next(&mut self) -> Result<Option<Duration>, DecodeError> {
            // The previous frame's disposal comes first, but the next frame must exist to
            // draw: a file that ends leaves its last frame up.
            let Some(frame) = self.dec.read_next_frame().map_err(bad)? else { return Ok(None) };
            match std::mem::replace(&mut self.undo, Dispose::Nothing) {
                Dispose::Nothing => {}
                Dispose::Clear(r) => self.canvas.clear(r),
                Dispose::Restore => {
                    if let Some(saved) = self.saved.take() {
                        self.canvas.restore(&saved);
                    }
                }
            }
            let region = Region {
                x: u32::from(frame.left),
                y: u32::from(frame.top),
                w: u32::from(frame.width),
                h: u32::from(frame.height),
            };
            self.undo = match frame.dispose {
                ::gif::DisposalMethod::Background => Dispose::Clear(region),
                ::gif::DisposalMethod::Previous => {
                    self.saved = Some(self.canvas.snapshot()?);
                    Dispose::Restore
                }
                ::gif::DisposalMethod::Any | ::gif::DisposalMethod::Keep => Dispose::Nothing,
            };
            // Transparent pixels (alpha 0) leave what is below; the rest replace it.
            self.canvas.draw(region, &frame.buffer, true);
            Ok(Some(delay(frame.delay)))
        }
    }

    impl Frames for Gif {
        fn info(&self) -> &Info {
            &self.info
        }

        fn next(&mut self) -> Result<Option<Frame<'_>>, DecodeError> {
            let delay = match self.first.take() {
                Some(d) => Some(d),
                None => self.draw_next()?,
            };
            Ok(delay.map(|delay| Frame { rgba: self.canvas.pixels(), delay }))
        }

        fn restart(&mut self) -> Result<(), DecodeError> {
            *self = Gif::open(self.bytes.clone())?;
            Ok(())
        }
    }
}

#[cfg(feature = "media-anim-apng")]
mod apng {
    use super::*;
    use crate::media_anim::canvas::{Canvas, Region, zeroed};

    enum Dispose {
        Nothing,
        Clear(Region),
        Restore,
    }

    pub(super) struct Apng {
        bytes: Arc<[u8]>,
        reader: ::png::Reader<Cursor<Arc<[u8]>>>,
        canvas: Canvas,
        /// A frame as the decoder gives it (any size up to the picture's).
        raw: Vec<u8>,
        /// The same as straight-alpha RGBA8 when it is not already.
        rgba: Vec<u8>,
        undo: Dispose,
        saved: Option<Vec<u8>>,
        left: u32,
        info: Info,
    }

    fn bad(e: ::png::DecodingError) -> DecodeError {
        match e {
            ::png::DecodingError::IoError(e) => DecodeError::Io(e.to_string()),
            other => DecodeError::Malformed(other.to_string()),
        }
    }

    fn delay(num: u16, den: u16) -> Duration {
        // A zero denominator means hundredths.
        let den = if den == 0 { 100 } else { u64::from(den) };
        let ms = u64::from(num) * 1000 / den;
        // "As fast as possible" cannot be drawn: it plays as slowly as old GIFs did.
        millis(if ms == 0 { 100 } else { ms })
    }

    impl Apng {
        pub(super) fn open(bytes: Arc<[u8]>) -> Result<Apng, DecodeError> {
            let mut dec = ::png::Decoder::new(Cursor::new(bytes.clone()));
            dec.set_transformations(::png::Transformations::EXPAND | ::png::Transformations::STRIP_16);
            let mut reader = dec.read_info().map_err(bad)?;
            let (w, h) = reader.info().size();
            let Some(actl) = reader.info().animation_control else {
                return Err(DecodeError::Malformed(String::from("no animation control")));
            };
            let profile = reader.info().icc_profile.as_deref().and_then(Srgb::from_profile);
            // Without a frame control before the image data, that image is only the fallback
            // for readers that do not animate: the animation starts after it.
            if reader.info().frame_control.is_none() {
                reader.next_frame_info().map_err(bad)?;
            }
            let raw_len = reader.output_buffer_size().ok_or(DecodeError::TooLarge { width: w, height: h })?;
            Ok(Apng {
                bytes,
                canvas: Canvas::new(w, h)?,
                raw: zeroed(raw_len, w, h)?,
                rgba: Vec::new(),
                reader,
                undo: Dispose::Nothing,
                saved: None,
                left: actl.num_frames,
                info: Info {
                    size: (w, h),
                    frames: Some(u64::from(actl.num_frames)),
                    loops: if actl.num_plays == 0 { Loops::Forever } else { Loops::Times(actl.num_plays) },
                    profile,
                },
            })
        }
    }

    impl Frames for Apng {
        fn info(&self) -> &Info {
            &self.info
        }

        fn next(&mut self) -> Result<Option<Frame<'_>>, DecodeError> {
            if self.left == 0 {
                return Ok(None);
            }
            let out = self.reader.next_frame(&mut self.raw).map_err(bad)?;
            self.left -= 1;
            let Some(fc) = self.reader.info().frame_control else {
                return Err(DecodeError::Malformed(String::from("a frame has no frame control")));
            };
            let pixels = self.raw.get(..out.buffer_size()).unwrap_or_default();
            if out.bit_depth != ::png::BitDepth::Eight {
                return Err(DecodeError::Malformed(String::from("a frame is not 8 bits per channel")));
            }
            let rgba: &[u8] = match out.color_type {
                ::png::ColorType::Rgba => pixels,
                ::png::ColorType::Rgb => {
                    self.rgba.clear();
                    self.rgba.try_reserve(pixels.len() / 3 * 4).map_err(|_| too_large(&self.info))?;
                    for [r, g, b] in pixels.as_chunks::<3>().0 {
                        self.rgba.extend_from_slice(&[*r, *g, *b, 255]);
                    }
                    &self.rgba
                }
                ::png::ColorType::Grayscale => {
                    self.rgba.clear();
                    self.rgba.try_reserve(pixels.len() * 4).map_err(|_| too_large(&self.info))?;
                    for g in pixels {
                        self.rgba.extend_from_slice(&[*g, *g, *g, 255]);
                    }
                    &self.rgba
                }
                ::png::ColorType::GrayscaleAlpha => {
                    self.rgba.clear();
                    self.rgba.try_reserve(pixels.len() * 2).map_err(|_| too_large(&self.info))?;
                    for [g, a] in pixels.as_chunks::<2>().0 {
                        self.rgba.extend_from_slice(&[*g, *g, *g, *a]);
                    }
                    &self.rgba
                }
                ::png::ColorType::Indexed => {
                    return Err(DecodeError::Malformed(String::from("a frame was left indexed")));
                }
            };
            match std::mem::replace(&mut self.undo, Dispose::Nothing) {
                Dispose::Nothing => {}
                Dispose::Clear(r) => self.canvas.clear(r),
                Dispose::Restore => {
                    if let Some(saved) = self.saved.take() {
                        self.canvas.restore(&saved);
                    }
                }
            }
            let region = Region { x: fc.x_offset, y: fc.y_offset, w: fc.width, h: fc.height };
            self.undo = match fc.dispose_op {
                ::png::DisposeOp::None => Dispose::Nothing,
                ::png::DisposeOp::Background => Dispose::Clear(region),
                ::png::DisposeOp::Previous => {
                    self.saved = Some(self.canvas.snapshot()?);
                    Dispose::Restore
                }
            };
            self.canvas.draw(region, rgba, fc.blend_op == ::png::BlendOp::Over);
            Ok(Some(Frame { rgba: self.canvas.pixels(), delay: delay(fc.delay_num, fc.delay_den) }))
        }

        fn restart(&mut self) -> Result<(), DecodeError> {
            *self = Apng::open(self.bytes.clone())?;
            Ok(())
        }
    }

    fn too_large(info: &Info) -> DecodeError {
        DecodeError::TooLarge { width: info.size.0, height: info.size.1 }
    }
}

#[cfg(feature = "media-anim-webp")]
mod webp {
    use super::*;
    use crate::media_anim::canvas::zeroed;

    pub(super) struct Webp {
        dec: image_webp::WebPDecoder<Cursor<Arc<[u8]>>>,
        /// A frame as the decoder gives it: RGB, or RGBA when the file has alpha.
        raw: Vec<u8>,
        /// The same as RGBA8.
        rgba: Vec<u8>,
        alpha: bool,
        info: Info,
    }

    fn bad(e: image_webp::DecodingError) -> DecodeError {
        match e {
            image_webp::DecodingError::IoError(e) => DecodeError::Io(e.to_string()),
            other => DecodeError::Malformed(other.to_string()),
        }
    }

    impl Webp {
        /// Opens `bytes`; `None` when the file turns out not to animate.
        pub(super) fn open(bytes: Arc<[u8]>) -> Result<Option<Webp>, DecodeError> {
            let mut dec = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(bad)?;
            if !dec.is_animated() {
                return Ok(None);
            }
            let (w, h) = dec.dimensions();
            // The background color is a hint that browsers do not follow: frames are
            // disposed to transparent.
            let _ = dec.set_background_color([0, 0, 0, 0]);
            let raw_len = dec.output_buffer_size().ok_or(DecodeError::TooLarge { width: w, height: h })?;
            let rgba_len = usize::try_from(u64::from(w) * u64::from(h) * 4)
                .map_err(|_| DecodeError::TooLarge { width: w, height: h })?;
            let profile = dec.icc_profile().ok().flatten().as_deref().and_then(Srgb::from_profile);
            let loops = match dec.loop_count() {
                image_webp::LoopCount::Forever => Loops::Forever,
                image_webp::LoopCount::Times(n) => Loops::Times(u32::from(n.get())),
            };
            Ok(Some(Webp {
                raw: zeroed(raw_len, w, h)?,
                rgba: zeroed(rgba_len, w, h)?,
                alpha: dec.has_alpha(),
                info: Info { size: (w, h), frames: Some(u64::from(dec.num_frames())), loops, profile },
                dec,
            }))
        }
    }

    impl Frames for Webp {
        fn info(&self) -> &Info {
            &self.info
        }

        fn next(&mut self) -> Result<Option<Frame<'_>>, DecodeError> {
            let ms = match self.dec.read_frame(&mut self.raw) {
                Ok(ms) => ms,
                Err(image_webp::DecodingError::NoMoreFrames) => return Ok(None),
                Err(e) => return Err(bad(e)),
            };
            if self.alpha {
                if self.rgba.len() == self.raw.len() {
                    self.rgba.copy_from_slice(&self.raw);
                }
            } else {
                for (out, [r, g, b]) in
                    self.rgba.as_chunks_mut::<4>().0.iter_mut().zip(self.raw.as_chunks::<3>().0)
                {
                    *out = [*r, *g, *b, 255];
                }
            }
            Ok(Some(Frame { rgba: &self.rgba, delay: millis(if ms == 0 { 100 } else { u64::from(ms) }) }))
        }

        fn restart(&mut self) -> Result<(), DecodeError> {
            self.dec.reset_animation();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
