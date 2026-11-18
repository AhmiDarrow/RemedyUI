//! RemedyUI animated images: GIF, APNG and animated WebP in an element, for chats and feeds
//! where many play at once.
//!
//! - [`animated()`]: an element playing a file or bytes, fitted to its box like
//!   [`remedy_ui_media_image::image`]
//! - [`set_memory_budget`] and [`trim_memory`]: how much decoded animation the app keeps
//! - [`register`]: APNG first frames for the still-image loader
//! - [`route`]: where frames are decoded (here, or a sandboxed helper through an
//!   [`route::Opener`]), and animated formats of the app's own
//!
//! Frames are decoded on the image workers at the size shown, composited as each format says
//! (GIF disposal, APNG dispose and blend, WebP blending). An animation of up to 64 frames is
//! kept whole and loops with no decoding; a longer one keeps a window of frames around the
//! one shown and decodes the rest again each time round, so memory follows the size shown and
//! not the length. Elements showing the same source at the same size share one animation and
//! play in step.
//!
//! An element plays only while some of it is on screen and its window is not minimized or
//! covered. Each frame has a timer of its own length, so nothing runs for a frame that does
//! not change what is shown. When the system asks for reduced motion the first frame is
//! shown with a play button, and the animation plays only when it is pressed. A still image
//! of any format the image loader reads is shown as a still.
//!
//! GIF frames that ask for less than 20 ms show for 100 ms, as in browsers. A GIF's loop
//! count is how many times it repeats after its first play. A file damaged after its first
//! frame plays the frames before the damage; one damaged before that cannot be shown.

#[cfg(any(feature = "gif", feature = "apng", feature = "webp"))]
mod canvas;
mod formats;
mod policy;
pub mod route;
mod stream;
#[cfg(test)]
mod testing;
mod view;

use std::any::Any;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Insets, Point, Rect};
use remedy_ui_layout::{AUTO, Align, Justify, Position};
use remedy_ui_media_image::{ImageState, Loader, image_with};
use remedy_ui_paint::{Brush, Item, Path};
use remedy_ui_reactive::Signal;
use remedy_ui_style::StylePatch;
use remedy_ui_view::{El, Ui};

pub use policy::set_memory_budget;
pub use remedy_ui_media_image::{Fit, Source};
pub use stream::{cached, resident_frames, trim_memory};

use view::{Control, Lease};

/// Teaches the still-image loader ([`remedy_ui_media_image::image`] and [`decode`]) to show the
/// first frame of an animated PNG, composited, rather than the fallback image some of them
/// carry. Call it once at start-up in apps that show APNG files as stills; calling it again
/// does nothing. GIF and WebP need nothing: the loader already shows their first frame.
///
/// [`decode`]: remedy_ui_media_image::decode
pub fn register() {
    #[cfg(feature = "apng")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| remedy_ui_media_image::add_format(std::sync::Arc::new(formats::ApngFirstFrame)));
    }
}

/// An element playing `source`, decoded off the UI thread at the size it is shown (again,
/// larger, when the element grows). `alt` says what the animation shows for screen readers;
/// an empty `alt` marks it decorative.
///
/// Sizing, the placeholder while loading and the broken-image mark are those of
/// [`remedy_ui_media_image::image`]. The animation plays while some of the element is on
/// screen and stops when it scrolls away, is hidden, or the window is minimized or covered.
/// With reduced motion on, it shows its first frame and a play button (named for screen
/// readers, and reachable and pressed by keyboard) that starts and pauses it.
#[track_caller]
pub fn animated(ui: &Ui, source: Source, alt: impl Into<String>, fit: Fit) -> El {
    // When memory is short, animations nothing plays let go of their frames.
    thread_local! {
        static LISTENING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !LISTENING.with(|l| l.replace(true)) {
        remedy_ui_view::device::on_memory_pressure(|_| trim_memory());
    }
    let alt = alt.into();
    let ctl = Rc::new(Control::new(ui));
    let c2 = ctl.clone();
    let loader: Loader = Rc::new(move |source, box_px, cover, state: Signal<ImageState>| {
        Box::new(Lease::new(&c2, source, box_px.unwrap_or((1, 1)), cover, state)) as Box<dyn Any>
    });
    let el = image_with(ui, source, alt.clone(), fit, loader);

    // Whether any of it shows.
    let c = ctl.clone();
    let el = el.on_visibility(move |seen| {
        c.visible.set(seen);
        c.refresh();
    });

    // The system's motion setting, and the person's choice under it.
    let play = Signal::new(false);
    let c = ctl.clone();
    let u = ui.clone();
    let el = el.bind(move || {
        let reduced = u.reduced_motion();
        // Pressing play is a choice made under reduced motion: it does not outlive it.
        if !reduced && play.get_untracked().unwrap_or(false) {
            play.set(false);
        }
        c.reduced.set(reduced);
        c.play.set(play.get());
        c.refresh();
    });

    let button = play_button(ui, &alt, play);
    let u = ui.clone();
    let has_motion = ctl.has_motion;
    let shown = button.clone();
    let _ = button.clone().bind(move || {
        let _ = shown.clone().hidden(!(u.reduced_motion() && has_motion.get()));
    });
    el.child(button)
}

/// The play and pause button over the animation's corner.
fn play_button(ui: &Ui, alt: &str, play: Signal<bool>) -> El {
    let theme = ui.theme();
    let size = theme.type_scale.body + 3.0;
    let margin = theme.spacing.unit * 2.0;
    let name = alt.to_owned();
    let glyph = ui.el("icon").width(size).height(size).layout(|l| l.shrink = 0.0).paint(move |cx| {
        let (w, h) = (cx.size.width, cx.size.height);
        let brush = Brush::Solid(cx.style.foreground);
        if play.get() {
            // Pause: two bars.
            let (bar, gap) = (w * 0.24, w * 0.18);
            let x = (w - 2.0 * bar - gap) / 2.0;
            for i in 0..2 {
                let left = x + i as f32 * (bar + gap);
                cx.list.fill_rounded(
                    Rect::new(left, h * 0.18, left + bar, h * 0.82),
                    CornerRadii::all(1.0),
                    brush.clone(),
                );
            }
        } else {
            // Play: a triangle.
            let mut p = Path::new();
            p.move_to(Point::new(w * 0.26, h * 0.16))
                .line_to(Point::new(w * 0.84, h * 0.5))
                .line_to(Point::new(w * 0.26, h * 0.84))
                .close();
            cx.list.push(Item::FillPath { path: p, brush, even_odd: false });
        }
    });
    let backdrop = theme.colors.bg.with_alpha(0.8);
    ui.el("icon_button")
        .row()
        .align(Align::Center)
        .justify(Justify::Center)
        .style(StylePatch::default().background(backdrop))
        .layout(move |l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: AUTO, start: AUTO, end: margin, bottom: margin };
        })
        .child(glyph)
        .on_click(move || play.update(|p| *p = !*p))
        .semantics(move || {
            let verb = if play.get() { "Pause animation" } else { "Play animation" };
            let label = if name.is_empty() { verb.to_owned() } else { format!("{verb}: {name}") };
            Semantics::new(Role::Button).label(label).action(Action::Click)
        })
}
