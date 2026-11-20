//! The `player` widget: the picture with captions over it, and a transport bar.

use std::time::Duration;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Insets, Rect};
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_layout::{AUTO, Align, Justify, Position};
use remedy_ui_media_video::{VideoFit, video_surface};
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

use crate::{PlaybackState, Player};

/// `m:ss`, or `h:mm:ss` from an hour.
pub fn format_time(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// How far the arrow keys move the playhead.
const ARROW_SEEK: f64 = 5.0;
/// How far J and L move it, as in common players.
const JUMP_SEEK: f64 = 10.0;

/// A player for `player`: its picture (with captions over it, when on) and a transport bar
/// with play and pause, the time, a seek bar, mute and volume, a captions switch when the
/// media has caption tracks, and a full-screen request. `label` names it for screen readers
/// ("Talk recording").
///
/// Keyboard, anywhere in the player: K plays and pauses, J and L jump 10 seconds, M mutes,
/// C turns captions on and off, F asks for full screen and Escape leaves it. On the seek bar
/// Space plays and pauses, the arrows move 5 seconds, Page Up and Page Down a tenth, Home and
/// End to either end. Space and Enter press whichever button has focus.
///
/// Sound-only media shows the transport bar alone. Media with pictures pauses while the
/// player is unseen (see [`UnseenPolicy`](crate::UnseenPolicy)), and does not start by itself
/// under reduced motion.
#[track_caller]
pub fn player(ui: &Ui, player: &Player, label: impl Into<String>) -> El {
    let label = label.into();
    let theme = ui.theme();
    let u = theme.spacing.unit;

    // The picture, with captions and messages over it.
    let (surface, sink) = video_surface(ui, label.clone(), VideoFit::Contain);
    player.attach(Some(sink));
    let caption = ui
        .el("player_caption")
        .layout(move |l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: AUTO, start: u * 4.0, end: u * 4.0, bottom: u * 3.0 };
        })
        .style(remedy_ui_style::StylePatch::default().padding(Insets {
            top: u * 0.5,
            bottom: u * 0.5,
            start: u * 1.5,
            end: u * 1.5,
        }));
    let cap = caption.clone();
    let p = player.clone();
    let caption = caption.bind(move || {
        let text = p.caption_text();
        let _ = cap.clone().hidden(text.is_none());
        let _ = cap.clone().text(remedy_ui_view::RichText::plain(text.unwrap_or_default()));
    });
    // What went wrong, in the captions' readable box at the top of the picture.
    let message = ui
        .el("player_caption")
        .layout(move |l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: u * 3.0, start: u * 3.0, end: u * 3.0, bottom: AUTO };
        })
        .style(remedy_ui_style::StylePatch::default().padding(Insets {
            top: u,
            bottom: u,
            start: u * 1.5,
            end: u * 1.5,
        }))
        .semantics(Semantics::new(Role::Alert));
    let msg = message.clone();
    let p = player.clone();
    let uw = ui.clone();
    let message = message.bind(move || {
        let text = match (p.state(), p.error()) {
            (PlaybackState::Failed, Some(e)) => Some(remedy_ui_view::words::fill(
                &uw.word("This can't be played: {error}"),
                &[("error", &e.to_string())],
            )),
            (PlaybackState::Failed, None) => Some(uw.word("This can't be played.")),
            _ => None,
        };
        let _ = msg.clone().hidden(text.is_none());
        let _ = msg.clone().text(remedy_ui_view::RichText::plain(text.unwrap_or_default()));
    });
    // A spinner centered over the picture while opening or waiting for data; clicks pass.
    let busy = ui
        .el("player_busy")
        .row()
        .align(Align::Center)
        .justify(Justify::Center)
        .layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: 0.0, start: 0.0, end: 0.0, bottom: 0.0 };
        })
        .pass_through()
        .child(w::spinner(ui, ui.word("Loading")));
    // The picture fills the stage (letterboxed by the surface); the rest sits over it.
    let stage = ui
        .el("player_stage")
        .grow(1.0)
        .column()
        .child(surface)
        .child(caption)
        .child(message)
        .child(busy.clone());
    let st = stage.clone();
    let p = player.clone();
    let stage = stage.bind(move || {
        let state = p.state();
        let _ = busy.clone().hidden(!matches!(state, PlaybackState::Opening | PlaybackState::Buffering));
        // Sound alone needs no picture; what failed to open shows its message there.
        let pictures = p.info().is_none_or(|i| i.video.is_some());
        let _ = st.clone().hidden(!pictures && state != PlaybackState::Failed);
    });

    // The transport bar.
    let p = player.clone();
    let play = glyph_button(
        ui,
        [Icon::Play, Icon::Pause],
        &ui.word("Play or pause (K)"),
        {
            let p = player.clone();
            move || usize::from(matches!(p.state(), PlaybackState::Playing | PlaybackState::Buffering))
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            move || {
                if matches!(p.state(), PlaybackState::Playing | PlaybackState::Buffering) {
                    u.word("Pause")
                } else {
                    u.word("Play")
                }
            }
        },
        move || p.toggle(),
    );
    let (p, uw) = (player.clone(), ui.clone());
    let time = ui.el("label").nowrap().layout(|l| l.shrink = 0.0).text(move || {
        let pos = format_time(p.position());
        match p.duration() {
            Some(d) => format!("{pos} / {}", format_time(d)),
            None if p.info().is_some() => {
                remedy_ui_view::words::fill(&uw.word("{position} · Live"), &[("position", &pos)])
            }
            None => pos,
        }
    });
    let seek = seek_bar(ui, player);
    let p = player.clone();
    let mute = glyph_button(
        ui,
        [Icon::Volume, Icon::VolumeMuted],
        &ui.word("Mute (M)"),
        {
            let p = player.clone();
            move || usize::from(p.is_muted())
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            move || if p.is_muted() { u.word("Unmute") } else { u.word("Mute") }
        },
        move || p.set_muted(!p.is_muted()),
    );
    let level: Signal<f64> = Signal::new(f64::from(player.volume()));
    let volume = ui.el("player_volume").width(u * 14.0).layout(|l| l.shrink = 0.0).child(w::slider(
        ui,
        ui.word("Volume"),
        level,
        0.0,
        1.0,
        0.05,
    ));
    let p = player.clone();
    let volume = volume.bind(move || p.set_volume(level.get() as f32));
    let p = player.clone();
    let captions = glyph_button(
        ui,
        [Icon::Captions, Icon::Captions],
        &ui.word("Captions (C)"),
        || 0,
        {
            let (p, u) = (player.clone(), ui.clone());
            move || {
                let tracks = p.subtitle_tracks();
                match p.caption_track().and_then(|i| tracks.get(i)) {
                    Some(t) => {
                        remedy_ui_view::words::fill(&u.word("Captions: {track}"), &[("track", &t.label)])
                    }
                    None => u.word("Captions: off"),
                }
            }
        },
        move || cycle_captions(&p),
    );
    let cb = captions.clone();
    let p = player.clone();
    let captions = captions.bind(move || {
        let _ = cb.clone().hidden(p.subtitle_tracks().is_empty());
        cb.state(remedy_ui_style::States::CHECKED, p.caption_track().is_some());
    });
    let p = player.clone();
    let full = glyph_button(
        ui,
        [Icon::Fullscreen, Icon::FullscreenExit],
        &ui.word("Full screen (F)"),
        {
            let p = player.clone();
            move || usize::from(p.is_fullscreen())
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            move || if p.is_fullscreen() { u.word("Exit full screen") } else { u.word("Full screen") }
        },
        move || p.request_fullscreen(!p.is_fullscreen()),
    );
    let fb = full.clone();
    let p = player.clone();
    let full = full.bind(move || {
        let pictures = p.info().is_none_or(|i| i.video.is_some());
        let _ = fb.clone().hidden(!pictures);
    });
    let bar = ui
        .el("toolbar")
        .row()
        .align(Align::Center)
        .gap(u)
        .child(play)
        .child(time)
        .child(seek)
        .child(mute)
        .child(volume)
        .child(captions)
        .child(full);

    let p = player.clone();
    let vis = player.clone();
    ui.el("player")
        .column()
        .grow(1.0)
        .semantics(Semantics::new(Role::Group).label(label))
        .child(stage)
        .child(bar)
        .on_visibility(move |seen| vis.set_visible(seen))
        .on_key(move |e| {
            if !e.pressed || e.composing || e.modifiers.ctrl || e.modifiers.alt || e.modifiers.meta {
                return false;
            }
            match &e.key {
                Key::Escape if p.is_fullscreen() => p.request_fullscreen(false),
                Key::Character(c) => match c.as_str() {
                    "k" => p.toggle(),
                    "j" => p.seek_by(-JUMP_SEEK),
                    "l" => p.seek_by(JUMP_SEEK),
                    "m" => p.set_muted(!p.is_muted()),
                    "c" if !p.subtitle_tracks().is_empty() => cycle_captions(&p),
                    "f" if p.info().is_none_or(|i| i.video.is_some()) => {
                        p.request_fullscreen(!p.is_fullscreen())
                    }
                    _ => return false,
                },
                _ => return false,
            }
            true
        })
}

/// Off, then each track in turn, then off again.
fn cycle_captions(p: &Player) {
    let n = p.subtitle_tracks().len();
    let next = match p.caption_track() {
        None if n > 0 => Some(0),
        Some(i) if i + 1 < n => Some(i + 1),
        _ => None,
    };
    p.set_caption_track(next);
}

/// An icon button whose glyph (`glyphs[which()]`) and name follow the player.
#[track_caller]
fn glyph_button(
    ui: &Ui,
    glyphs: [Icon; 2],
    tip: &str,
    which: impl Fn() -> usize + 'static,
    name: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> El {
    let size = ui.theme().type_scale.body + 3.0;
    let [first, second] = glyphs;
    let a = w::icon(ui, first, size);
    let b = w::icon(ui, second, size);
    let (a2, b2) = (a.clone(), b.clone());
    let button = ui
        .el("icon_button")
        .row()
        .align(Align::Center)
        .justify(Justify::Center)
        .layout(|l| l.shrink = 0.0)
        .child(a)
        .child(b)
        .bind(move || {
            let second = which() == 1;
            let _ = a2.clone().hidden(second);
            let _ = b2.clone().hidden(!second);
        })
        .on_click(on_press)
        .semantics(move || Semantics::new(Role::Button).label(name()).action(Action::Click));
    w::tooltip(&button, tip);
    button
}

/// The seek bar: the played part in the accent color and a thumb at the playhead. Drag or
/// click to move; arrows move 5 seconds, Page Up and Down a tenth, Home and End to the ends.
#[track_caller]
fn seek_bar(ui: &Ui, player: &Player) -> El {
    let h = ui.theme().spacing.control_height;
    let bar = ui.el("slider").height(h).grow(1.0).focusable(true);
    let length = {
        let p = player.clone();
        move || p.duration().filter(|d| !d.is_zero())
    };
    let b2 = bar.clone();
    let p = player.clone();
    let len = length.clone();
    let seek_to_x = move |x: f32| {
        if let (Some(b), Some(d)) = (b2.bounds(), len()) {
            let r = (cx_radius(b.height()), b.width());
            let t = ((x - b.min.x - r.0) / (r.1 - 2.0 * r.0).max(1.0)).clamp(0.0, 1.0);
            p.seek(d.mul_f64(f64::from(t)));
        }
    };
    let p = player.clone();
    let len = length.clone();
    let painted = player.clone();
    let b3 = bar.clone();
    let p2 = player.clone();
    let (seek_word, seek_of) = (ui.word("Seek"), ui.word("{position} of {length}"));
    let len2 = length.clone();
    bar.paint(move |cx| {
        let pos = painted.position();
        let t = match len() {
            Some(d) => (pos.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0) as f32,
            None => 0.0,
        };
        let d = cx.theme.derive();
        let mid = cx.size.height / 2.0;
        let r = cx_radius(cx.size.height);
        let (x0, x1) = (r, cx.size.width - r);
        let x = x0 + (x1 - x0) * t;
        cx.list.fill_rounded(
            Rect::new(x0, mid - 2.0, x1, mid + 2.0),
            CornerRadii::all(2.0),
            Brush::Solid(d.line),
        );
        cx.list.fill_rounded(
            Rect::new(x0, mid - 2.0, x, mid + 2.0),
            CornerRadii::all(2.0),
            Brush::Solid(cx.theme.colors.accent),
        );
        cx.list.fill_rounded(
            Rect::new(x - r, mid - r, x + r, mid + r),
            CornerRadii::all(r),
            Brush::Solid(d.text),
        );
    })
    .on_pointer(move |e| {
        let drag = e.phase == PointerPhase::Down
            || (e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY));
        if drag {
            seek_to_x(e.position.x);
        }
        drag
    })
    .on_key(move |e| {
        if !e.pressed {
            return false;
        }
        if e.key == Key::Space {
            p.toggle();
            return true;
        }
        let Some(d) = len2() else { return false };
        let total = d.as_secs_f64();
        match e.key {
            Key::ArrowRight | Key::ArrowUp => p.seek_by(ARROW_SEEK),
            Key::ArrowLeft | Key::ArrowDown => p.seek_by(-ARROW_SEEK),
            Key::PageUp => p.seek_by(total / 10.0),
            Key::PageDown => p.seek_by(-total / 10.0),
            Key::Home => p.seek(Duration::ZERO),
            Key::End => p.seek(d),
            _ => return false,
        }
        true
    })
    .bind(move || {
        let pos = p2.position();
        let len = length();
        let _ = b3.clone().disabled(len.is_none() || !p2.info().is_some_and(|i| i.seekable));
        let total = len.unwrap_or_default();
        let _ = b3.clone().semantics(
            Semantics::new(Role::Slider)
                .label(seek_word.clone())
                .numeric(pos.as_secs_f64(), 0.0, total.as_secs_f64())
                .value(remedy_ui_view::words::fill(
                    &seek_of,
                    &[("position", &format_time(pos)), ("length", &format_time(total))],
                ))
                .action(Action::Increment)
                .action(Action::Decrement),
        );
        b3.repaint();
    })
}

/// The thumb's radius on a bar `h` tall (as the core slider draws it).
fn cx_radius(h: f32) -> f32 {
    (h * 0.3).max(5.0)
}
