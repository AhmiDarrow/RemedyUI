//! RemedyUI's stage widgets: the parts for building whole alternative presentations of an
//! app's interface — scene-based, skeuomorphic and playful layouts — that stay themed,
//! accessible and idle when nothing moves.
//!
//! - [`scene`]: a stage of a design size (say 1280 by 720) that scales to fit its box
//!   ([`Fit`]: contain, cover or stretch; whole-number scaling for pixel art), with layers of
//!   art and children at named [`Anchor`]s that stay on their spot as the window resizes;
//!   optional parallax on pointer move
//! - [`sprite`] and [`pixel_art`]: pixel art written as code ([`PixelArt`]: a palette and rows
//!   of characters) or cut from a [`SpriteSheet`], crisp at whole-number scales, with named
//!   states chosen by a signal and frame animation
//! - [`orbit`]: children on a circle or ellipse, on nested rings, optionally turning slowly
//! - [`path_layout`]: children along a polyline or smooth curve ([`Route`]), evenly or at
//!   given fractions, with the path drawn from theme tokens (solid or dashed)
//! - [`knob`] and [`tuner`]: rotary controls; a knob for a continuous value, a tuner dial
//!   (a line or an arc) of labelled stations whose needle snaps to them
//! - [`fader`]: a mixing-desk fader with ticks and an optional level meter beside it
//! - [`timeline`]: items placed by time on a horizontal track, newest at one end, scrubbed by
//!   dragging, the wheel and the keyboard, flowing smoothly as items arrive, and virtualized
//! - [`big_button`] and [`big_toggle`]: physical-looking buttons: a round arcade button, a
//!   plate key and a toggle switch with a lamp; hold-to-talk with [`BigButtonOptions::held`]
//! - [`display_text`]: board lettering (LED dot matrix or split-flap) that stays real text,
//!   with a marquee for text longer than the board
//!
//! Every widget takes its colors and sizes from the theme (art may name [`Ink`]s: theme roles
//! or fixed colors), is described to assistive technology (controls are sliders, buttons or
//! switches with values people can read), works from the keyboard, respects reduced motion
//! (ambient motion stops; changes jump instead of gliding), animates only while seen, and does
//! no work while nothing changes. Children placed on stages, orbits, paths and timelines are
//! read and focused in the order the app gave them, not their position in the picture.
//! See `docs/design/15-widgets-and-media.md`.

mod button;
mod common;
mod dial;
mod fader;
mod orbit;
mod path;
mod pixel;
mod scene;

pub use button::{BigButtonOptions, ButtonLook, big_button, big_toggle};
pub use common::{Ink, LineStyle, Place};
pub use dial::{KnobOptions, Station, TunerOptions, TunerShape, ValueText, knob, tuner};
pub use fader::{FaderOptions, fader};
pub use orbit::{Orbit, OrbitOptions, orbit};
pub use path::{PathLayout, PathOptions, Route, Spread, path_layout};
pub use pixel::{PixelArt, PixelScale, SpriteOptions, SpriteSheet, SpriteSource, pixel_art, sprite};
pub use scene::{Anchor, Fit, Layer, Scene, SceneOptions, StageTransform, scene};
