//! Where animations are decoded. By default here, on the image workers; an [`Opener`] set
//! with [`set_opener`] decodes them elsewhere (`remedy-ui-media-isolated` decodes in a
//! sandboxed helper). Either way a stream gets a [`FrameSource`]: frames composited at the size
//! shown, with their delays, read in order and started again for the next loop. The stream's
//! memory policy, shared playheads and pausing are the same wherever frames come from.
//!
//! [`add_format`] adds an animated format of the app's own, read here and in a helper alike.

use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use crate::media_image::{DecodeError, Srgb, finish_rgba, fitted_scalable};
use remedy_ui_engine::paint::Image;

use crate::media_anim::formats::{self, Frames};

/// How many times an animation plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: an animation loops forever or a number of times.
pub enum Loops {
    /// Without end.
    Forever,
    /// This many times in all (at least one).
    Times(u32),
}

/// What an opened animation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Animation {
    /// The picture's own size, in pixels.
    pub size: (u32, u32),
    /// The size its frames come at (fitted to the box it was opened for, never larger than
    /// `size`).
    pub show_at: (u32, u32),
    /// How many frames one play has, when the file says.
    pub frames: Option<u64>,
    /// How many times it plays.
    pub loops: Loops,
}

/// An animation being read: one composited frame after another. Moves between the image
/// workers, used by one at a time.
pub trait FrameSource: Send {
    /// What it is.
    fn animation(&self) -> &Animation;
    /// The next frame at [`Animation::show_at`] (premultiplied sRGB, as the image loader's)
    /// and how long it stays, or `None` after the last frame of the play.
    fn next(&mut self) -> Result<Option<(Image, Duration)>, DecodeError>;
    /// Back to the first frame, for the next loop.
    fn restart(&mut self) -> Result<(), DecodeError>;
}

/// What a file turned out to be.
// Exhaustive on purpose: an opener answers with a still or an animation; every router must handle both.
pub enum Opened {
    /// Frames to play.
    Animated(Box<dyn FrameSource>),
    /// One picture: the still-image loader shows it.
    Still,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Opened::Animated(s) => f.debug_tuple("Animated").field(s.animation()).finish(),
            Opened::Still => f.write_str("Still"),
        }
    }
}

/// An animated format added to GIF, APNG and WebP.
pub trait Format: Send + Sync {
    /// True when `head` (the first bytes of a file) is this format.
    fn sniff(&self, head: &[u8]) -> bool;
    /// The picture's size, read from its header.
    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError>;
    /// Opens it to play in a `box_px` box of device pixels, fitting inside it or (`cover`)
    /// covering it.
    fn open(
        &self,
        bytes: &Arc<[u8]>,
        box_px: (u32, u32),
        cover: bool,
    ) -> Result<Box<dyn FrameSource>, DecodeError>;
}

fn formats_list() -> &'static RwLock<Vec<Arc<dyn Format>>> {
    static F: OnceLock<RwLock<Vec<Arc<dyn Format>>>> = OnceLock::new();
    F.get_or_init(Default::default)
}

/// Adds an animated format: [`open_here`] and the animated element read it from then on, and
/// the still-image loader shows its first frame (so the element can size itself from it, and
/// plain image elements show it). Add it before `crate::media_isolated::helper_main` when
/// isolation is used, so helpers read it too.
pub fn add_format(f: Arc<dyn Format>) {
    formats_list().write().unwrap_or_else(PoisonError::into_inner).push(f.clone());
    crate::media_image::add_format(Arc::new(FirstFrame(f)));
}

/// An added animated format, seen by the still-image loader: its first frame.
struct FirstFrame(Arc<dyn Format>);

impl crate::media_image::Format for FirstFrame {
    fn sniff(&self, head: &[u8]) -> bool {
        self.0.sniff(head)
    }

    fn dimensions(&self, bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
        self.0.dimensions(bytes)
    }

    fn decode(
        &self,
        bytes: &[u8],
        show_at: Option<(u32, u32)>,
    ) -> Result<crate::media_image::Decoded, DecodeError> {
        let size = self.0.dimensions(bytes)?;
        let mut source = self.0.open(&Arc::from(bytes), show_at.unwrap_or(size), false)?;
        let (image, _) = source
            .next()?
            .ok_or_else(|| DecodeError::Malformed(String::from("the animation has no frames")))?;
        Ok(crate::media_image::Decoded::new(image, size))
    }
}

fn added(head: &[u8]) -> Option<Arc<dyn Format>> {
    formats_list().read().unwrap_or_else(PoisonError::into_inner).iter().find(|f| f.sniff(head)).cloned()
}

/// Opens animations somewhere other than here.
pub trait Opener: Send + Sync {
    /// Opens `bytes` as [`open_here`] would, to play in a `box_px` box, fitting or (`cover`)
    /// covering it.
    fn open(&self, bytes: &Arc<[u8]>, box_px: (u32, u32), cover: bool) -> Result<Opened, DecodeError>;
}

fn opener_slot() -> &'static RwLock<Option<Arc<dyn Opener>>> {
    static O: OnceLock<RwLock<Option<Arc<dyn Opener>>>> = OnceLock::new();
    O.get_or_init(Default::default)
}

/// Opens animations through `opener` from now on; `None` opens them here again. Animations
/// already open keep the source they have.
pub fn set_opener(opener: Option<Arc<dyn Opener>>) {
    *opener_slot().write().unwrap_or_else(PoisonError::into_inner) = opener;
}

/// The opener animations go through, when one is set.
pub fn opener() -> Option<Arc<dyn Opener>> {
    opener_slot().read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Opens through the opener when one is set, here otherwise.
pub(crate) fn open(bytes: &Arc<[u8]>, box_px: (u32, u32), cover: bool) -> Result<Opened, DecodeError> {
    match opener() {
        Some(o) => o.open(bytes, box_px, cover),
        None => open_here(bytes, box_px, cover),
    }
}

/// The size of an animation in a format this build reads (added formats first), from its
/// header; other images as the still loader reads them.
pub fn dimensions_here(bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
    match added(bytes) {
        Some(f) => f.dimensions(bytes),
        None => crate::media_image::dimensions(bytes),
    }
}

/// Opens `bytes` in this process to play in a `box_px` box: its frames if it is an animated
/// format this build reads (added formats first), else [`Opened::Still`].
pub fn open_here(bytes: &Arc<[u8]>, box_px: (u32, u32), cover: bool) -> Result<Opened, DecodeError> {
    if let Some(f) = added(bytes) {
        return f.open(bytes, box_px, cover).map(Opened::Animated);
    }
    match formats::open(bytes)? {
        formats::Opened::Still => Ok(Opened::Still),
        formats::Opened::Animated(frames) => {
            let info = frames.info();
            let animation = Animation {
                size: info.size,
                show_at: fitted_scalable(info.size, box_px, cover, false),
                frames: info.frames,
                loops: info.loops,
            };
            let profile = info.profile.clone();
            Ok(Opened::Animated(Box::new(Local { frames, animation, profile })))
        }
    }
}

/// The built-in formats' frames, finished (color-converted, premultiplied, scaled) here.
struct Local {
    frames: Box<dyn Frames>,
    animation: Animation,
    profile: Option<Srgb>,
}

impl FrameSource for Local {
    fn animation(&self) -> &Animation {
        &self.animation
    }

    fn next(&mut self) -> Result<Option<(Image, Duration)>, DecodeError> {
        let (size, show_at) = (self.animation.size, self.animation.show_at);
        match self.frames.next()? {
            Some(f) => {
                let image = finish_rgba(size.0, size.1, f.rgba, self.profile.as_ref(), Some(show_at))?;
                Ok(Some((image, f.delay)))
            }
            None => Ok(None),
        }
    }

    fn restart(&mut self) -> Result<(), DecodeError> {
        self.frames.restart()
    }
}
