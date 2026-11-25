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
//! - [`fx`]: shader-free effects for animated scenes: flicker and twinkle curves, seeded
//!   noise textures made once, additive glows and light cones, cached blurs, scrolling water
//!   shimmer
//!
//! Every widget takes its colors and sizes from the theme (art may name [`Ink`]s: theme roles
//! or fixed colors), is described to assistive technology (controls are sliders, buttons or
//! switches with values people can read), works from the keyboard, respects reduced motion
//! (ambient motion stops; changes jump instead of gliding), animates only while seen, and does
//! no work while nothing changes. Children placed on stages, orbits, paths and timelines are
//! read and focused in the order the app gave them, not their position in the picture.
//! See `docs/design/15-widgets-and-media.md`.
//!
//! A radio's front, as a stage: a tuner of rooms across the top, a lamp-lit push-to-talk
//! button, and a board saying what is on.
//!
//! ```
//! use remedy_ui_core::Size;
//! use remedy_ui_core::reactive::Signal;
//! use remedy_ui_widgets_stage::*;
//!
//! # fn demo(ui: &remedy_ui_view::Ui) {
//! let room = Signal::new(0usize);
//! let talking = Signal::new(false);
//! let rooms = vec![Station::new("General").detail("01.0"), Station::new("Lounge").detail("02.4")];
//! let radio = scene(ui, "Radio", Size::new(1280.0, 720.0), SceneOptions::default())
//!     .anchor("dial", Anchor::rect(240.0, 40.0, 900.0, 160.0))
//!     .anchor("ptt", Anchor::point(540.0, 625.0))
//!     .anchor("board", Anchor::rect(840.0, 570.0, 380.0, 110.0))
//!     .place("dial", tuner(ui, "Room", rooms, room, TunerOptions::default()))
//!     .place("ptt", big_button(ui, "Push to talk", || {}, BigButtonOptions::default().held(talking)))
//!     .place("board", display_text(ui, move || if talking.get() { "ON AIR" } else { "STANDBY" }, DisplayOptions::default()));
//! let el: remedy_ui_view::El = radio.into();
//! # let _ = el;
//! # }
//! ```

/// The stage widgets' own words (screen reader names and descriptions, keyboard hints) in
/// every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a language,
/// with [`remedy_ui_view::strings::add_messages`] under the name `widgets-stage`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("widgets-stage");

mod button;
mod common;
mod dial;
mod display;
mod fader;
pub mod fx;
mod orbit;
mod path;
mod pixel;
mod scene;
mod timeline;

pub use button::{BigButtonOptions, ButtonLook, big_button, big_toggle};
pub use common::{Ink, LineStyle, Place};
pub use dial::{KnobOptions, Station, TunerOptions, TunerShape, ValueText, knob, tuner};
pub use display::{Board, DisplayOptions, Marquee, display_text};
pub use fader::{FaderOptions, fader};
pub use orbit::{Orbit, OrbitOptions, orbit};
pub use path::{PathLayout, PathOptions, Route, Spread, path_layout};
pub use pixel::{PixelArt, PixelScale, SpriteOptions, SpriteSheet, SpriteSource, pixel_art, sprite};
pub use scene::{Anchor, Fit, Layer, Scene, SceneOptions, StageTransform, scene};
pub use timeline::{NewestAt, Timeline, TimelineOptions, timeline};
