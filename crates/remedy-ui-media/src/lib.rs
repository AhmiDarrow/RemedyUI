//! RemedyUI media: images, animated images, video surfaces and isolated decoding.
//!
//! Each part is a module behind a Cargo feature of the same name.

#[cfg(feature = "media-anim")]
pub mod media_anim;
#[cfg(feature = "media-image")]
pub mod media_image;
#[cfg(feature = "media-isolated")]
pub mod media_isolated;
#[cfg(feature = "media-video")]
pub mod media_video;
