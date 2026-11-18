# Motion and reduced motion

Motion in RemedyUI has three rules:

- **Motion explains, it does not decorate.** Things move to show where they came from and
  where they went.
- **The person can turn it off.** When the system asks for reduced motion, animations jump
  to their end and spinners hold still. This holds for every widget, the engine's and the
  app's.
- **Motion never costs the UI thread what it does not have to.** Animations run on the
  window's frame clock with no timers, a window with nothing moving asks for no frames, and
  scrolling, flings and overscroll run on the compositor thread at the display's rate even
  while app code is busy.

This page covers the reduced-motion setting, the motion tokens, animating elements,
transitions for content that comes and goes, and painters that animate.

## Reduced motion

[`Ui::reduced_motion`](crate::Ui::reduced_motion) says whether motion should be reduced. The
runner sets it from the system's setting and changes it when the setting changes. It is
tracked: a binding that reads it runs again when it changes.

Everything that moves through the engine follows it on its own: [`El::animate`](crate::El::animate),
[`show`](crate::view::show) and list transitions, the spinner and indeterminate progress, the
[`viz`](crate::viz) meters, carousels and the terminal's cursor. Painters that animate read
[`PaintCx::reduced_motion`](crate::PaintCx::reduced_motion) and draw a still frame.

```
use remedy_ui::host::SystemSettings;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;

let mut app = TestApp::window(|ui, _| {
    let u = ui.clone();
    let hint = w::text(ui, move || if u.reduced_motion() { "Motion is reduced" } else { "Motion is on" })
        .variant("muted");
    ui.root().child(hint);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Motion is on")));

// The person turns on reduced motion in the system settings.
let mut settings = SystemSettings::default();
settings.reduced_motion = true;
app.host().change_settings(settings);
app.frames(1);
assert!(ui.reduced_motion());
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Motion is reduced")));
```

Tests can set it directly with [`Ui::set_reduced_motion`](crate::Ui::set_reduced_motion), or
through the host's settings as above.

## Motion tokens

How long things take is part of the theme ([`MotionTokens`](crate::style::MotionTokens)):
`quick` for hover and press feedback, `standard` for most transitions, `emphasized` for large
moves and page transitions. [`Ui::motion`](crate::Ui::motion) turns a
[`Pace`](crate::motion::Pace) into a [`Motion`](crate::motion::Motion): a spring of the
token's duration with no bounce. Use it instead of fixed durations, so the app moves like the
rest of the platform.

```
use remedy_ui::themes::ThemeChoice;

let theme = ThemeChoice::default().resolve(None, false);
let m = theme.motion;
assert!(m.quick < m.standard && m.standard < m.emphasized);
```

## Animating an element

[`El::animate`](crate::El::animate) moves one property of an element to a value: its
opacity, background, foreground, border, border width, radius or padding, or where and how
large it is drawn ([`Target::Translate`](crate::motion::Target::Translate) and
[`Target::Scale`](crate::motion::Target::Scale), which move it without laying anything out
again). A new target for the same property starts from where the value is, with the speed it
has, so an interrupted move turns smoothly instead of jumping. Under reduced motion the move
is instant.

- [`El::animate_then`](crate::El::animate_then) calls back once the value has arrived.
- [`El::animate_style`](crate::El::animate_style) animates every animatable property a style
  patch sets.
- [`El::set_animated`](crate::El::set_animated) jumps a property to its value.
- [`El::flip`](crate::El::flip) glides an element through a layout change about to happen
  (a reorder, an insert above it).

```
use remedy_ui::motion::{Pace, Target};
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let toast = w::card(ui).child(w::text(ui, "Saved"));
    let _ = s.set(toast.clone());
    ui.root().child(toast);
});
let (Some(ui), Some(toast)) = (app.ui(), slot.get().cloned()) else { unreachable!("built") };

// Fade it out with the theme's standard motion.
toast.animate(Target::Opacity(0.0), ui.motion(Pace::Standard));
app.frames(1);
assert!(toast.is_animating(), "it moves on the frame clock, frame by frame");

app.frames(120); // two seconds of frames
assert!(!toast.is_animating(), "and settles; the window asks for no more frames");
```

The values come from the `motion` module ([`motion`](crate::motion)): springs
([`Spring::smooth`](crate::motion::Spring::smooth), [`Spring::bouncy`](crate::motion::Spring::bouncy)),
tweens along an [`Easing`](crate::motion::Easing), and [`Animation`](crate::motion::Animation)
for a value an app animates itself. Springs are solved in closed form, so they are exact at
60, 120 or 240 Hz.

## Content that comes and goes

Content that appears and leaves plays a [`Transition`](crate::motion::Transition): a fade, a
slide, or any set of targets it starts from.

- [`show`](crate::view::show) plays `enter` when its content appears and `exit` when it
  goes; leaving content stays, inert, until its exit ends. See
  [`crate::guide::control_flow`].
- [`widgets::for_each_with`](crate::widgets::for_each_with) plays enter and exit transitions
  on list rows and glides the rows that stay to their new places. See
  [`crate::guide::lists`].

## Painters that animate

A painter whose content changes every frame (a waveform, a spinner, a 3D preview) samples
[`PaintCx::time`](crate::PaintCx::time), the frame's predicted presentation time, and calls
[`PaintCx::request_next_frame`](crate::PaintCx::request_next_frame) while it moves. It is
drawn again on the next frame, at the display's rate, and the window goes idle as soon as it
stops asking. Under reduced motion it draws a still frame and does not ask.

```
use remedy_ui::core::{CornerRadii, Rect};
use remedy_ui::paint::Brush;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::Cell;
use std::rc::Rc;

let drawn = Rc::new(Cell::new(0u32));
let d = drawn.clone();
let mut app = TestApp::window(move |ui, _| {
    let pulse = w::canvas(ui, move |cx| {
        d.set(d.get() + 1);
        // A dot whose brightness follows the frame's time.
        let phase = if cx.reduced_motion { 0.0 } else { (cx.time.as_secs_f32() * 3.0).sin() * 0.5 + 0.5 };
        let color = cx.theme.colors.accent.with_alpha(0.4 + 0.6 * phase);
        let r = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
        cx.list.fill_rounded(r, CornerRadii::all(8.0), Brush::Solid(color));
        if !cx.reduced_motion {
            cx.request_next_frame();
        }
    })
    .width(16.0)
    .height(16.0);
    ui.root().child(pulse);
});
let before = drawn.get();
app.frames(10);
assert!(drawn.get() >= before + 9, "drawn every frame while it moves");
```

[`Ui::frame_time`](crate::Ui::frame_time) is the same clock outside painters, and
[`Ui::refresh_interval`](crate::Ui::refresh_interval) tells how long a frame has on the
window's display.

## Scrolling

Scroll containers ([`widgets::scroll`](crate::widgets::scroll),
[`El::scroll`](crate::El::scroll)) scroll on the compositor thread with the platform's fling
and overscroll physics, so they keep moving smoothly while app code runs. The UI thread hears
about it only when new content comes into view ([`El::on_scroll`](crate::El::on_scroll)).
