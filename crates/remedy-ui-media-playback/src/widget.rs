//! The `player` widget: the picture with captions over it, and a transport bar.

use std::time::Duration;

use remedy_ui_a11y::{Action, Live, Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Insets, Rect};
use remedy_ui_input::{Buttons, Key, PointerKind, PointerPhase};
use remedy_ui_layout::{AUTO, Align, Justify, Length, Position};
use remedy_ui_media_video::{VideoFit, video_surface};
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_view::motion::{Easing, Motion, Target};
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core::{self as w, Icon, MenuItem};

use crate::{CaptionPick, KeyScheme, MediaError, PlaybackState, Player};

/// Message `id` in the window's locale (tracked: read in a property closure, it follows the
/// locale).
fn tr(ui: &Ui, id: &str) -> String {
    crate::STRINGS.get(ui, id)
}

/// What is shown over the picture when the media cannot be played, in `locale`. The detail
/// after the explanation is the platform's own (English) words.
fn failure(locale: &str, e: Option<MediaError>) -> String {
    let (id, detail) = match e {
        None => return crate::STRINGS.format_in(locale, "playback-failed", &[]),
        Some(MediaError::Unsupported(r)) => ("playback-failed-unsupported", r),
        Some(MediaError::NotFound(r)) => ("playback-failed-not-found", r),
        Some(MediaError::Format(r)) => ("playback-failed-format", r),
        Some(MediaError::Decode(r)) => ("playback-failed-decode", r),
        Some(MediaError::Device(r)) => ("playback-failed-device", r),
        Some(MediaError::Denied(r)) => ("playback-failed-denied", r),
        // Which component to install, and how: for whoever looks after the machine.
        Some(MediaError::Missing(d)) => ("playback-failed-missing", d.to_string()),
    };
    crate::STRINGS.format_in(locale, id, &[("detail", detail.into())])
}

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
/// How long a click on the picture waits for a second one before it plays or pauses
/// ([`KeyScheme::Arrows`]), so a double click only toggles full screen.
const CLICK_HOLD: Duration = Duration::from_millis(220);
/// How long full screen rests with no activity before the controls fade (Portal's 2.6 s).
const WAKE: Duration = Duration::from_millis(2600);
/// How long they take to fade in or out (none under reduced motion).
const FADE: Duration = Duration::from_millis(250);

/// How far Up and Down move the volume in [`KeyScheme::Arrows`].
const VOLUME_STEP: f32 = 0.05;

/// Whether full screen can be asked for or left: while remote only leaving is offered, since a
/// full-screen casting stage would only enlarge the poster.
fn fullscreen_toggles(p: &Player) -> bool {
    p.is_fullscreen() || !p.is_remote()
}

/// Theater mode as [`fullscreen_toggles`].
fn theater_toggles(p: &Player) -> bool {
    p.is_theater() || !p.is_remote()
}

fn toggle_fullscreen(p: &Player) {
    if fullscreen_toggles(p) {
        // As Portal's mini player: the compact picture has no box to fill the screen with.
        p.set_compact(false);
        p.request_fullscreen(!p.is_fullscreen());
    }
}

fn toggle_theater(p: &Player) {
    if theater_toggles(p) {
        // T while compact opens straight into theater mode, as Portal's mini player.
        if remedy_ui_reactive::untrack(|| p.is_compact()) {
            p.set_compact(false);
            if !p.is_theater() {
                p.request_theater(true);
            }
            return;
        }
        p.request_theater(!p.is_theater());
    }
}

/// What an app adds to the [`player_with`] widget, as Portal's page has its own controls in
/// the bar: `PlayerParts::default().end(vec![pip]).title(heading)`.
#[derive(Default)]
#[non_exhaustive]
pub struct PlayerParts {
    /// The app's own controls (a picture-in-picture button), in order, in the bar after
    /// captions and before theater mode, cast and full screen: where Portal's PiP is. Not in
    /// the compact layout.
    pub end: Vec<El>,
    /// The app's title (and whatever goes beside it, such as Expand and Close), above play
    /// and the time in the [compact](Player::set_compact) layout only, as Portal's mini
    /// player. An element of its own, not one the app shows elsewhere at the same time.
    pub title: Option<El>,
    /// Shows a picture-in-picture button in the bar, after the app's own controls and before
    /// theater mode, as Portal's: pressed while the picture is in its window, hidden where
    /// there is none ([`Player::pip_available`]). Off by default.
    pub pip_button: bool,
    /// The app's own glyphs for the player's icons (Portal's SVGs), by [`Icon`] and size in
    /// logical pixels: `Some` replaces the built-in glyph, `None` keeps it. They take the
    /// button's text color as the built-in ones do (an SVG icon draws in `currentColor`).
    pub icons: Option<PlayerIcons>,
}

/// An app's glyphs for the player's icons ([`PlayerParts::icons`]): Play, Pause, Back10,
/// Forward10, Volume, VolumeMuted, Captions, PictureInPicture, Theater, Cast, Casting,
/// Fullscreen and FullscreenExit (Play also for the big Play over a paused picture).
pub type PlayerIcons = std::rc::Rc<dyn Fn(&Ui, Icon, f32) -> Option<El>>;

/// The app's glyph for `icon` at `size`, else the built-in one.
fn glyph(ui: &Ui, icons: &Option<PlayerIcons>, icon: Icon, size: f32) -> El {
    icons.as_ref().and_then(|f| f(ui, icon, size)).unwrap_or_else(|| w::icon(ui, icon, size))
}

impl PlayerParts {
    /// Sets `end`: the app's controls at the end of the bar.
    pub fn end(mut self, value: Vec<El>) -> Self {
        self.end = value;
        self
    }

    /// Sets `title`: the app's title in the compact layout.
    pub fn title(mut self, value: El) -> Self {
        self.title = Some(value);
        self
    }

    /// Sets `pip_button`: a picture-in-picture button in the bar.
    pub fn pip_button(mut self, value: bool) -> Self {
        self.pip_button = value;
        self
    }

    /// Sets `icons`: the app's glyphs for the player's icons.
    pub fn icons(mut self, value: impl Fn(&Ui, Icon, f32) -> Option<El> + 'static) -> Self {
        self.icons = Some(std::rc::Rc::new(value));
        self
    }
}

/// The picture of `player` alone: what plays, letterboxed, named `label` for screen readers.
/// It takes the pictures when shown (the newest shown one does), handing them to another
/// shown picture when hidden or removed, and the player counts as seen while any of its
/// pictures is ([`UnseenPolicy`](crate::UnseenPolicy)): so moving the picture between the
/// full widget, a compact one, a picture-in-picture or a second window never pauses it. The
/// [`player`] widget uses one.
#[track_caller]
pub fn player_picture(ui: &Ui, player: &Player, label: impl Into<String>) -> El {
    static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (surface, sink) = video_surface(ui, label, VideoFit::Contain);
    player.add_picture(id, sink);
    let (p, q) = (player.clone(), player.clone());
    surface
        .on_visibility(move |seen| p.picture_seen(id, Some(seen)))
        .on_removed(move || q.picture_seen(id, None))
}

/// In [`KeyScheme::Arrows`], after a click or tap on `control` (not a key), takes focus off
/// it once the click is done, so the next key goes to the player: Space plays and pauses
/// instead of pressing the control again, and the arrows skip instead of nudging a slider.
/// Keyboard activation keeps focus where it is. Focus goes to the player itself (focusable
/// only until it loses focus again), since keys reach the player only from inside it.
fn drops_focus_after_click(ui: &Ui, p: &Player, control: &El) {
    if p.key_scheme() != KeyScheme::Arrows {
        return;
    }
    let (me, u) = (control.clone(), ui.clone());
    let _ = control.clone().on_pointer(move |e| {
        if e.phase == PointerPhase::Up {
            let me = me.clone();
            u.after(Duration::ZERO, move || {
                if !me.is_focused() {
                    return;
                }
                let mut home = me.parent();
                while let Some(h) = home.clone().filter(|h| h.info().is_none_or(|i| i.widget != "player")) {
                    home = h.parent();
                }
                match home {
                    Some(h) => h.focusable(true).focus(),
                    None => me.blur(),
                }
            });
        }
        false
    });
}

/// The volume up or down a step, on the 5% grid, unmuting; nothing while a device's volume
/// is unknown (a step from a level it has not said could jump it to full).
fn nudge_volume(p: &Player, up: bool) {
    if !p.volume_known() {
        return;
    }
    let step = if up { VOLUME_STEP } else { -VOLUME_STEP };
    p.set_volume(((p.volume() + step) * 20.0).round().clamp(0.0, 20.0) / 20.0);
    if p.is_muted() {
        p.set_muted(false);
    }
}

/// Shown as muted: muted, or at volume 0 (as Portal's page shows it).
fn sounds_muted(p: &Player) -> bool {
    p.is_muted() || (p.volume_known() && p.volume() <= 0.0)
}

/// Mute and unmute, as Portal's page does: unmuting at volume 0 sets it to half. A device is
/// only muted or unmuted.
fn toggle_mute(p: &Player) {
    if p.is_remote() {
        p.set_muted(!p.is_muted());
    } else if sounds_muted(p) {
        p.set_muted(false);
        if p.volume() <= 0.0 {
            p.set_volume(0.5);
        }
    } else {
        p.set_muted(true);
    }
}

/// A player for `player`: its picture (with captions over it, when on) and a transport bar
/// with play and pause, the time, a seek bar, mute and volume, a captions button while there
/// is anything to pick (it says what shows, "Off", "Auto · English" or "Loading…", and opens
/// a menu: Off, Auto when the app offers it ([`Player::on_caption_pick`]), the tracks, the
/// app's [choices](Player::set_caption_choices) and note, and the timing half a second at a
/// time, the steps keeping the menu open), a theater-mode request once the app listens for one
/// ([`Player::on_theater`]), a cast button once the app opens a device menu
/// ([`Player::on_cast`]), and a full-screen request. `label` names it for screen readers
/// ("Talk recording").
///
/// In [remote mode](Player::set_remote) the picture gives way to a casting stage (the
/// device's name and what it is doing, announced as it changes, over the app's poster) and
/// every control and key acts on the device.
///
/// Keyboard, anywhere in the player: K plays and pauses, J and L jump 10 seconds, M mutes,
/// C turns captions off and back on as last picked, F asks for full screen, T for theater mode (when offered),
/// Escape leaves full screen or else theater mode, > and < (Shift+.
/// and Shift+, on most keyboards) make it faster and slower, and Shift+N plays
/// what is [next up](Player::set_next_up). On the seek bar
/// Space plays and pauses, the arrows move 5 seconds, Page Up and Page Down a tenth, Home and
/// End to either end. Space and Enter press whichever button has focus.
///
/// With [`KeyScheme::Arrows`] ([`PlayerOptions::keys`](crate::PlayerOptions::keys)), also
/// anywhere in the player: Left and Right skip 10 seconds, Up and Down change the volume by
/// 5% (unmuting), Space plays and pauses on the volume slider, N plays next, and the letters
/// work with Shift held; a focused slider keeps its own keys. In either scheme, keys with
/// Ctrl, Alt or the system key held, keys a text field takes and keys while a menu is open
/// never reach the player. [`Player::keys_from`] hears the same keys from the rest of the
/// window, as Portal's page does.
///
/// In remote mode the speed button is hidden and > and < do nothing: the device plays at its
/// own speed.
///
/// Double-clicking (or double-tapping) the picture asks for full screen or leaves it. With
/// [`KeyScheme::Arrows`] a single click or tap plays and pauses, once no second click has come
/// within 220 ms (so a double click never plays or pauses), and a click or tap on a control
/// hands the keys back to the player (Space then plays and pauses rather than pressing it
/// again); keyboard activation keeps focus on the control. In
/// remote mode the full-screen and theater buttons are disabled and F, T and a double-click
/// do nothing, except to leave full screen or theater mode when casting started in it.
///
/// While the app makes a next made [later](crate::NextUp::later) ("Starting …" over the
/// picture, as an alert), or a device starts its next by itself, the seek bar is disabled
/// and the time blank. While starting the next, skipping or [busy](Player::set_status), a
/// Cancel button shows under the spinner when the app offers one ([`Player::offer_cancel`]);
/// Escape does not press it. The app's [status](Player::set_status) shows in place of the
/// player's own words, announced politely. After a failure, when the app
/// [retries](Player::on_retry), Play, K, Space and a click on the picture (in either scheme)
/// try again.
///
/// Sound-only media shows the transport bar alone. Media with pictures pauses while none of
/// its [pictures](player_picture) is seen (see [`UnseenPolicy`](crate::UnseenPolicy)), and
/// does not start by itself under reduced motion.
///
/// [`PlayerOptions`](crate::PlayerOptions) shape the bar as Portal's: `skip_buttons` (Back and
/// Forward 10 seconds after Play), `speed_button(false)`, `two_rows` (the seek bar above the
/// buttons); and [`Player::set_compact`] switches to Portal's
/// mini player. [`player_with`] adds the app's own controls.
#[track_caller]
pub fn player(ui: &Ui, player: &Player, label: impl Into<String>) -> El {
    player_with(ui, player, label, PlayerParts::default())
}

/// The [`player`] widget with the app's own `parts`: its controls at the end of the bar and
/// its title in the compact layout.
#[track_caller]
pub fn player_with(ui: &Ui, player: &Player, label: impl Into<String>, parts: PlayerParts) -> El {
    let label = label.into();
    let theme = ui.theme();
    let u = theme.spacing.unit;
    let options = player.options().clone();
    let icons = &parts.icons;

    // Full screen: the controls show on any activity and rest WAKE later (see `overlay`).
    let menu_open = Signal::new(false);
    let rest = std::rc::Rc::new(Rest {
        ui: ui.clone(),
        player: player.clone(),
        timer: std::cell::Cell::new(None),
        hold: std::cell::OnceCell::new(),
    });
    // How far the captions sit above the bottom: over the bar while it floats there.
    let lift = Signal::new(u * 3.0);

    // The picture, with captions and messages over it.
    let surface = player_picture(ui, player, label.clone());
    // Captions centred at the foot of the picture, each line in its own box as a browser
    // draws a cue (Portal's page): lines of different lengths stay centred.
    let caption = ui.el("player_captions").column().align(Align::Center).pass_through().layout(move |l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: AUTO, start: u * 4.0, end: u * 4.0, bottom: u * 3.0 };
    });
    // Once any cue showing has settings of its own (WebVTT's line, position, size, align and
    // vertical; ASS's alignment and margins), every cue showing is laid out by WebVTT's rules
    // on the letterboxed picture and drawn here (`cue_layout`), moved apart where they meet
    // and kept above the bar.
    let placed = ui.el("player_cues").pass_through().layout(|l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: 0.0, start: 0.0, end: 0.0, bottom: 0.0 };
    });
    let (cap, pl, p, cu) = (caption.clone(), placed.clone(), player.clone(), ui.clone());
    let shown = std::cell::RefCell::new(Vec::<crate::Cue>::new());
    let caption = caption.bind(move || {
        let cues = if p.is_remote() { Vec::new() } else { p.caption_cues() };
        if *shown.borrow() == cues {
            return;
        }
        cap.clear();
        let own = cues.iter().any(|c| c.settings != crate::CueSettings::default());
        let _ = cap.clone().hidden(cues.is_empty() || own);
        let _ = pl.clone().hidden(!own);
        if !own {
            for line in cues.iter().flat_map(|c| c.text.lines()) {
                let _ = cap.clone().child(caption_line(&cu, line));
            }
        }
        *shown.borrow_mut() = cues;
    });
    // Above the bar while it floats over the picture in full screen; the placed cues are
    // drawn again when the cues, the lift or the picture's shape change.
    let cap = caption.clone();
    let caption = caption.bind(move || {
        let bottom = lift.get();
        let _ = cap.clone().layout(move |l| l.inset.bottom = bottom);
    });
    let (pl, p) = (placed.clone(), player.clone());
    let placed = placed.bind(move || {
        let cues = if p.is_remote() { Vec::new() } else { p.caption_cues() };
        if cues.iter().all(|c| c.settings == crate::CueSettings::default()) {
            return;
        }
        let bottom = lift.get();
        let aspect = p.info().and_then(|i| i.video).map(|v| v.width as f32 / v.height.max(1) as f32);
        let label = cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n");
        let _ = pl.clone().semantics(Semantics::new(Role::Group).label(label));
        let _ = pl.clone().paint(move |cx| paint_cues(cx, &cues, aspect, bottom));
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
        }));
    let msg = message.clone();
    let p = player.clone();
    let mu = ui.clone();
    let message = message.bind(move || {
        // Portal's mini player shows no words over its small picture.
        if p.is_compact() {
            let _ = msg.clone().hidden(true);
            return;
        }
        // The app's status in place of the player's words; announced politely, as Portal's
        // page does, where the player's own failures and work are alerts.
        if let Some(s) = p.status().filter(|_| !p.is_remote()) {
            let text = if s.busy && p.canceling() { tr(&mu, "playback-canceling") } else { s.text };
            let said = Semantics::new(Role::Status).label(text.clone()).live(Live::Polite);
            let _ = msg.clone().hidden(false).semantics(said);
            let _ = msg.clone().text(remedy_ui_view::RichText::plain(text));
            return;
        }
        let _ = msg.clone().semantics(Semantics::new(Role::Alert));
        let text = match (p.state(), p.skipping()) {
            (PlaybackState::Failed, _) if !p.is_remote() => {
                let why = p.app_failure().unwrap_or_else(|| failure(&mu.locale(), p.error()));
                // As Portal's page: Play asks again when the app can.
                Some(match p.retries() {
                    true => crate::STRINGS.format(&mu, "playback-retry", &[("message", why.into())]),
                    false => why,
                })
            }
            (_, Some(at)) if !p.is_remote() => {
                let head = match p.holding() {
                    true => tr(&mu, "playback-preparing"),
                    false => crate::STRINGS.format(
                        &mu,
                        "playback-skipping",
                        &[("position", format_time(at).into())],
                    ),
                };
                Some(match p.canceling() {
                    true => format!("{head}\n{}", tr(&mu, "playback-canceling")),
                    false => head,
                })
            }
            // The next, while the app makes it, as Portal's page says it: the title, then
            // what the app is doing.
            _ if p.is_remote() => None,
            _ => match p.starting_next() {
                Some(title) => {
                    let head =
                        crate::STRINGS.format(&mu, "playback-starting-next", &[("title", title.into())]);
                    // As Portal's page: once Cancel is pressed, that is what it is doing.
                    let detail = match p.canceling() {
                        true => Some(tr(&mu, "playback-canceling")),
                        false => p.starting_detail(),
                    };
                    Some(match detail {
                        Some(d) => format!("{head}\n{d}"),
                        None => head,
                    })
                }
                None => p.next_error(),
            },
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
        .child(
            ui.el("player_busy_center")
                .column()
                .align(Align::Center)
                .gap(u * 1.5)
                .pass_through()
                .child(w::spinner(ui, {
                    let u = ui.clone();
                    move || tr(&u, "playback-loading")
                }))
                .child(stage_cancel(ui, player)),
        );
    let cast = cast_stage(ui, player);
    let (big_play, glyph) = big_play(ui, icons, player);
    let on_glyph = {
        let (layer, glyph) = (big_play.clone(), glyph.clone());
        move |at: remedy_ui_core::Point| {
            layer.info().is_some_and(|i| !i.hidden) && glyph.bounds().is_some_and(|b| b.contains(at))
        }
    };
    let p = player.clone();
    // The picture fills the stage (letterboxed by the surface); the rest sits over it.
    let stage = ui
        .el("player_stage")
        .grow(1.0)
        .column()
        .child(surface.clone())
        .child(caption)
        .child(placed)
        .child(message)
        .child(cast)
        .child(busy.clone())
        .child(big_play)
        .on_pointer({
            let (u, held) = (ui.clone(), std::rc::Rc::new(std::cell::Cell::new(None)));
            let rest = rest.clone();
            move |e| {
                if e.phase != PointerPhase::Down || !e.buttons.contains(Buttons::PRIMARY) {
                    return false;
                }
                // A tap on a rested full screen brings the controls back and does no more
                // (Portal's would also play or pause).
                if e.kind == PointerKind::Touch && !remedy_ui_reactive::untrack(|| p.controls_shown()) {
                    rest.wake();
                    return true;
                }
                // A second press takes back the first one's play or pause.
                if let Some(t) = held.take() {
                    u.cancel_timer(t);
                }
                if u.click_count() == 2 {
                    toggle_fullscreen(&p);
                    return true;
                }
                // After a failure a click tries again (Portal's), in either scheme; the big Play
                // glyph plays in either scheme, after the same wait.
                let retry = p.retries() && p.state() == PlaybackState::Failed && !p.is_remote();
                if p.key_scheme() != KeyScheme::Arrows && !retry && !on_glyph(e.position) {
                    return false;
                }
                let (q, h) = (p.clone(), held.clone());
                held.set(Some(u.after(CLICK_HOLD, move || {
                    h.set(None);
                    q.toggle();
                })));
                true
            }
        })
        // The play button, Space and K, and the full-screen button and F, do what clicks do.
        .keyboard_alternative({
            let u = ui.clone();
            move || tr(&u, "playback-full-screen-tip")
        });
    let st = stage.clone();
    let p = player.clone();
    let stage = stage.bind(move || {
        let state = p.state();
        let remote = p.is_remote();
        let waiting = matches!(state, PlaybackState::Opening | PlaybackState::Buffering);
        let busy_status = p.status().is_some_and(|s| s.busy);
        let working = p.starting_next().is_some() || ((p.skipping().is_some() || busy_status) && !remote);
        let _ = busy.clone().hidden(!waiting && !working);
        let _ = surface.clone().hidden(remote);
        // Sound alone needs no picture; what failed to open shows its message there.
        let pictures = p.info().is_none_or(|i| i.video.is_some());
        let said = p.starting_next().is_some() || p.next_error().is_some() || p.status().is_some();
        let _ = st.clone().hidden(!pictures && state != PlaybackState::Failed && !remote && !said);
    });

    // The transport bar.
    let play = play_button(ui, icons, player);
    let time = clock(ui, player);
    let seek = seek_bar(ui, player);
    let p = player.clone();
    let mute = glyph_button(
        ui,
        icons,
        [Icon::Volume, Icon::VolumeMuted],
        "playback-mute-tip",
        {
            let p = player.clone();
            move || usize::from(sounds_muted(&p))
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            // While casting, as Portal's page says it: the TV's.
            move || {
                tr(
                    &u,
                    match (sounds_muted(&p), p.is_remote()) {
                        (true, false) => "playback-unmute",
                        (false, false) => "playback-mute",
                        (true, true) => "playback-unmute-tv",
                        (false, true) => "playback-mute-tv",
                    },
                )
            }
        },
        move || toggle_mute(&p),
    );
    let level: Signal<f64> = Signal::new(f64::from(player.volume()));
    // The core slider takes its name once: in the window's language when the player is made.
    let slider = w::slider(ui, tr(ui, "playback-volume"), level, 0.0, 1.0, f64::from(VOLUME_STEP));
    // Named "TV volume" while casting, as Portal's page; disabled while the device's volume
    // is unknown. Bound after the core slider's own semantics, so this name wins.
    let (sl, p, tu) = (slider.clone(), player.clone(), ui.clone());
    let slider = slider.bind(move || {
        let v = level.get();
        let name = tr(&tu, if p.is_remote() { "playback-volume-tv" } else { "playback-volume" });
        let _ = sl.clone().disabled(!p.volume_known()).semantics(
            Semantics::new(Role::Slider)
                .label(name)
                .numeric(v, 0.0, 1.0)
                .action(Action::Increment)
                .action(Action::Decrement),
        );
    });
    let slider = slider.height(theme.metric("player-volume-height", theme.spacing.control_height));
    let volume_width = theme.metric("player-volume-width", u * 14.0);
    let volume = ui.el("player_volume").width(volume_width).layout(|l| l.shrink = 0.0).child(slider.clone());
    let p = player.clone();
    let volume = volume.bind(move || {
        let v = level.get() as f32;
        // Following an unknown device volume (the local one, shown) is not setting it.
        if remedy_ui_reactive::untrack(|| p.volume_known()) {
            p.set_volume(v);
        }
    });
    // The slider follows the volume: the device's in remote mode.
    let p = player.clone();
    let volume = volume.bind(move || level.set_if_changed(f64::from(p.volume())));
    // Mute and the volume together; with `volume_on_hover` the volume shows only while the
    // pointer is over them or focus is in them (Portal's `.volume`).
    let sound = ui.el("player_sound").row().align(Align::Center);
    let sound = match options.volume_on_hover {
        // Shown, it stands a tenth of a unit off mute and three tenths before the time, as
        // Portal's margins; folded, it takes no room.
        true => {
            let space = |w: f32| ui.el("player_space").width(w).layout(|l| l.shrink = 0.0);
            let holder = ui
                .el("player_volume_holder")
                .row()
                .align(Align::Center)
                .layout(|l| l.shrink = 0.0)
                .clip()
                .child(space(u * 0.1))
                .child(volume.clone())
                .child(space(u * 0.3));
            let sound = sound.child(mute.clone()).child(holder.clone());
            w::reveal_on_hover(sound, &holder, volume_width + u * 0.4)
        }
        false => sound.gap(theme.metric("player-gap", u)).child(mute.clone()).child(volume.clone()),
    };
    let captions = captions_control(ui, icons, player, menu_open);
    let p = player.clone();
    let full = glyph_button(
        ui,
        icons,
        [Icon::Fullscreen, Icon::FullscreenExit],
        "playback-full-screen-tip",
        {
            let p = player.clone();
            move || usize::from(p.is_fullscreen())
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            move || {
                tr(&u, if p.is_fullscreen() { "playback-exit-full-screen" } else { "playback-full-screen" })
            }
        },
        move || toggle_fullscreen(&p),
    );
    let fb = full.clone();
    // The speed, as a button that steps through the usual speeds and wraps back to the
    // slowest; Shift+> and Shift+< step up and down.
    let (sp, tu) = (player.clone(), ui.clone());
    let speed = w::button(
        ui,
        move || {
            let s = sp.speed();
            crate::STRINGS.format(&tu, "playback-speed", &[("speed", s.into())])
        },
        {
            let p = player.clone();
            move || {
                let now = p.speed();
                match crate::SPEEDS.iter().copied().find(|s| *s > now + 1e-9) {
                    Some(next) => p.set_speed(next),
                    None => p.set_speed(crate::SPEEDS.first().copied().unwrap_or(1.0)),
                }
            }
        },
    );
    {
        let u = ui.clone();
        w::tooltip(&speed, move || tr(&u, "playback-speed-tip"));
    }
    // A device plays at its own speed: no speed while casting.
    let (sb, p) = (speed.clone(), player.clone());
    let speed = speed.bind(move || {
        let _ = sb.clone().hidden(p.is_remote());
    });
    let p = player.clone();
    let full = full.bind(move || {
        let pictures = p.info().is_none_or(|i| i.video.is_some());
        let _ = fb.clone().hidden(!pictures);
        let _ = fb.clone().disabled(!fullscreen_toggles(&p));
    });
    // Theater mode: one name, pressed while on, offered once the app listens.
    let p = player.clone();
    let theater = glyph_button(
        ui,
        icons,
        [Icon::Theater, Icon::Theater],
        "playback-theater-tip",
        || 0,
        {
            let u = ui.clone();
            move || tr(&u, "playback-theater")
        },
        move || toggle_theater(&p),
    );
    let (tb, p, tu) = (theater.clone(), player.clone(), ui.clone());
    let theater = theater
        .semantics(move || {
            Semantics::new(Role::Button)
                .label(tr(&tu, "playback-theater"))
                .checked(p.is_theater())
                .action(Action::Click)
        })
        .bind({
            let p = player.clone();
            move || {
                let pictures = p.info().is_none_or(|i| i.video.is_some());
                let _ = tb.clone().hidden(!pictures || !p.theater_offered());
                tb.state(remedy_ui_style::States::CHECKED, p.is_theater());
                let _ = tb.clone().disabled(!theater_toggles(&p));
            }
        });
    // Picture in picture: pressed while the picture is in its window; only where there is one.
    let pip = parts.pip_button.then(|| {
        let p = player.clone();
        let button = glyph_button(
            ui,
            icons,
            [Icon::PictureInPicture, Icon::PictureInPicture],
            "playback-pip-tip",
            || 0,
            {
                let (p, u) = (player.clone(), ui.clone());
                move || tr(&u, if p.is_pip() { "playback-pip-exit" } else { "playback-pip" })
            },
            move || if p.is_pip() { p.exit_pip() } else { p.request_pip() },
        );
        let (pb, p) = (button.clone(), player.clone());
        button.bind(move || {
            let _ = pb.clone().hidden(!p.pip_available() && !p.is_pip());
            pb.state(remedy_ui_style::States::CHECKED, p.is_pip());
        })
    });
    // Cast: one name that says whether it is casting; the app opens its menu of devices.
    let p = player.clone();
    let cast = glyph_button(
        ui,
        icons,
        [Icon::Cast, Icon::Casting],
        "playback-cast-tip",
        {
            let p = player.clone();
            move || usize::from(p.is_remote())
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            move || tr(&u, if p.is_remote() { "playback-casting" } else { "playback-cast" })
        },
        || {},
    );
    let (cb, me) = (cast.clone(), cast.clone());
    let cast = cast.on_click(move || p.request_cast(&me)).bind({
        let p = player.clone();
        move || {
            let _ = cb.clone().hidden(!p.cast_offered());
            cb.state(remedy_ui_style::States::CHECKED, p.is_remote());
        }
    });
    // Portal's Back and Forward 10 seconds, after Play.
    let skip = |glyph: Icon, tip: &'static str, name: &'static str, by: f64| {
        let p = player.clone();
        let u = ui.clone();
        glyph_button(ui, icons, [glyph, glyph], tip, || 0, move || tr(&u, name), move || p.seek_by(by))
    };
    let skips = match options.skip_buttons {
        true => vec![
            skip(Icon::Back10, "playback-back-tip", "playback-back", -JUMP_SEEK),
            skip(Icon::Forward10, "playback-forward-tip", "playback-forward", JUMP_SEEK),
        ],
        false => Vec::new(),
    };
    let speed = options.speed_button.then_some(speed);
    let skips_full = skips.clone();
    let mute_full = mute.clone();
    let mut controls = vec![&play, &mute, &slider, &captions, &theater, &cast, &full];
    controls.extend(pip.iter());
    controls.extend(skips.iter());
    controls.extend(speed.iter());
    for c in controls {
        drops_focus_after_click(ui, player, c);
    }
    let row = || ui.el("player_row").row().align(Align::Center).gap(theme.metric("player-gap", u));
    let ends = |r: El| {
        r.children(speed.clone())
            .child(captions.clone())
            .children(parts.end.iter().cloned())
            .children(pip.clone())
            .child(theater.clone())
            .child(cast.clone())
            .child(full.clone())
    };
    let bar = match options.two_rows {
        // Portal's: the seek bar alone above; Play, the skips, the sound and the time at the
        // start of the row below, the rest at its end.
        true => {
            let buttons = row()
                .child(play.clone())
                .children(skips)
                .child(sound)
                .child(time.clone())
                .child(ui.el("player_gap").grow(1.0));
            ui.el("player_bar").column().child(seek).child(ends(buttons))
        }
        false => ui.el("player_bar").column().child(ends(
            row().child(play.clone()).children(skips).child(time.clone()).child(seek).child(sound),
        )),
    };
    // On a bar narrower than the theme's `player-captions-words-min` (none by default) the
    // captions button shows its glyph alone, keeping its name, so the row fits a phone.
    let words_min = theme.metric("player-captions-words-min", 0.0);
    if words_min > 0.0 {
        let words = captions.child_elements().into_iter().nth(1);
        let _ = bar.clone().on_resize(move |size| {
            if let Some(w) = &words {
                let _ = w.clone().hidden(size.width < words_min);
            }
        });
    }
    // A line over the bar, between it and the picture: the sheet's `player_rule`, the
    // theme's `player-rule` high (none by default).
    let rule = ui.el("player_rule").height(theme.metric("player-rule", 0.0));
    let inner = bar.clone();
    let bar = ui.el("player_chrome").column().child(rule.clone()).child(bar);
    // In full screen the bar's parts take the sheet's `full` variant, the glyphs
    // `player-glyph-full`.
    {
        let mut styled = vec![inner, rule, time, play.clone(), captions.clone(), theater.clone()];
        styled.extend([cast.clone(), full.clone()]);
        styled.extend(skips_full.iter().cloned());
        styled.extend(parts.end.iter().cloned());
        styled.extend(pip.iter().cloned());
        styled.push(mute_full);
        full_screen_look(ui, player, styled);
    }

    let next = next_up_card(ui, icons, player);
    let (chrome, title) = overlay(ui, player, &bar, &next, &rest, menu_open, lift);

    // Portal's mini player: the picture at the start, the app's title over play and the time
    // beside it; the "Up next" card stays under them.
    let mini = ui
        .el("toolbar")
        .row()
        .align(Align::Center)
        .gap(u)
        .child(play_button(ui, icons, player))
        .child(clock(ui, player));
    let side = ui.el("player_side").column().grow(1.0).justify(Justify::Center).gap(u * 0.5);
    let side = match parts.title {
        Some(t) => side.child(t),
        None => side,
    }
    .child(mini);
    let main = ui.el("player_main").column().grow(1.0).gap(u * 1.5).child(stage.clone()).child(side.clone());
    let (m, st, sd, br, p) = (main.clone(), stage, side, bar.clone(), player.clone());
    let main = main.bind(move || {
        let compact = p.is_compact();
        let _ = sd.clone().hidden(!compact);
        let _ = br.clone().hidden(compact);
        let _ = match compact {
            true => m.clone().row().align(Align::Center),
            false => m.clone().column().align(Align::Stretch),
        };
        let _ = st.clone().layout(move |l| {
            l.grow = if compact { 0.0 } else { 1.0 };
            l.shrink = if compact { 0.0 } else { 1.0 };
            l.width = if compact { Length::Px(u * 17.0) } else { Length::Auto };
            l.height = if compact { Length::Px(u * 9.5) } else { Length::Auto };
        });
    });

    let p = player.clone();
    let root = ui.el("player");
    let (me, home, cursor, menus) = (root.clone(), root.clone(), root.clone(), root.clone());
    static WIDGETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let widget_id = WIDGETS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    root.column()
        .grow(1.0)
        .semantics(Semantics::new(Role::Group).label(label))
        // Focusable only while a click left focus here (see `drops_focus_after_click`).
        .on_focus_change(move |on| {
            if !on {
                let _ = me.clone().focusable(false);
            }
        })
        .child(main)
        .child(next)
        .child(chrome)
        .child(title)
        // The pointer hides with the controls.
        .bind({
            let p = player.clone();
            move || {
                let _ = cursor.clone().cursor_hidden(!p.controls_shown());
            }
        })
        // Full screen takes the keys: focus comes to the player when it is elsewhere (as the
        // focus a click leaves, `drops_focus_after_click`), so K, F and a key to wake work.
        .bind({
            let (p, me, was) = (player.clone(), home.clone(), std::cell::Cell::new(false));
            move || {
                let full = p.is_fullscreen();
                if full && !was.replace(full) && !me.states().contains(remedy_ui_style::States::FOCUS_WITHIN)
                {
                    let _ = me.clone().focusable(true);
                    me.focus();
                }
                was.set(full);
            }
        })
        // Any pointer movement or press, and any key, brings the controls back.
        .keyboard_alternative({
            let u = ui.clone();
            move || tr(&u, "playback-wake-tip")
        })
        .on_pointer({
            let rest = rest.clone();
            move |e| {
                if matches!(e.phase, PointerPhase::Move | PointerPhase::Down) {
                    rest.wake();
                }
                false
            }
        })
        // Keys from the rest of the window (`Player::keys_from`) wake the controls too.
        .bind({
            let (p, rest, was) = (player.clone(), rest.clone(), std::cell::Cell::new(None));
            move || {
                let n = p.key_wakes();
                if was.replace(Some(n)).is_some_and(|w| w != n) {
                    remedy_ui_reactive::untrack(|| rest.wake());
                }
            }
        })
        // Seen or not, for `Player::keys_from` (Portal's keys only while its dock is open).
        .on_visibility({
            let p = player.clone();
            move |seen| p.widget_seen(widget_id, Some(seen))
        })
        .on_removed({
            let p = player.clone();
            move || p.widget_seen(widget_id, None)
        })
        .on_key(move |e| {
            if e.pressed {
                rest.wake();
            }
            // Buttons press themselves on Space and the seek bar plays and pauses itself.
            apply_key(&p, e, slider.is_focused() || home.is_focused())
        })
        // Going compact closes the menus its hidden buttons opened, as Portal's mini player.
        .bind({
            let p = player.clone();
            move || {
                if p.is_compact() {
                    remedy_ui_reactive::untrack(|| w::close_menus_in(&menus));
                }
            }
        })
}

/// The player's key map ([`Player::handle_key`]): `space` when Space may play and pause in
/// [`KeyScheme::Arrows`]. True when the key was the player's.
pub(crate) fn apply_key(p: &Player, e: &remedy_ui_input::KeyEvent, space: bool) -> bool {
    if !e.pressed || e.composing || e.modifiers.ctrl || e.modifiers.alt || e.modifiers.meta {
        return false;
    }
    if let Key::Named(n) = &e.key {
        return crate::media_keys::key_press(n).is_some_and(|m| crate::media_keys::apply(p, m));
    }
    let arrows = p.key_scheme() == KeyScheme::Arrows;
    let pictures = p.info().is_none_or(|i| i.video.is_some());
    match &e.key {
        Key::Escape if p.is_fullscreen() => p.request_fullscreen(false),
        Key::Escape if p.is_theater() => p.request_theater(false),
        Key::ArrowLeft if arrows => p.seek_by(-JUMP_SEEK),
        Key::ArrowRight if arrows => p.seek_by(JUMP_SEEK),
        Key::ArrowUp if arrows => nudge_volume(p, true),
        Key::ArrowDown if arrows => nudge_volume(p, false),
        Key::Space if arrows && space => p.toggle(),
        Key::Character(c) => match (if arrows { c.to_lowercase() } else { c.clone() }).as_str() {
            "k" => p.toggle(),
            "j" => p.seek_by(-JUMP_SEEK),
            "l" => p.seek_by(JUMP_SEEK),
            "m" => toggle_mute(p),
            ">" if !p.is_remote() => p.step_speed(true),
            "<" if !p.is_remote() => p.step_speed(false),
            "c" if p.captions_offered() => p.toggle_captions(),
            "f" if pictures && fullscreen_toggles(p) => toggle_fullscreen(p),
            "t" if p.theater_offered() && pictures && theater_toggles(p) => toggle_theater(p),
            "N" | "n" if (arrows || e.modifiers.shift) && p.next_up().is_some() => p.play_next(),
            _ => return false,
        },
        _ => return false,
    }
    true
}

/// Full screen's rest: the controls show on activity and fade [`WAKE`] after the last, unless
/// something holds them then (checked when the timer fires, as Portal's page does).
struct Rest {
    ui: Ui,
    player: Player,
    timer: std::cell::Cell<Option<u64>>,
    /// Whether something holds the controls now (set once the bar is made).
    hold: std::cell::OnceCell<Box<dyn Fn() -> bool>>,
}

impl Rest {
    fn wake(self: &std::rc::Rc<Self>) {
        self.player.set_awake(true);
        if let Some(t) = self.timer.take() {
            self.ui.cancel_timer(t);
        }
        let me = self.clone();
        self.timer.set(Some(self.ui.after(WAKE, move || {
            me.timer.set(None);
            let full = remedy_ui_reactive::untrack(|| me.player.is_fullscreen());
            match full && me.hold.get().is_some_and(|h| h()) {
                true => me.wake(),
                false => me.player.set_awake(false),
            }
        })));
    }
}

/// Whether anything in `el` has focus the keyboard put there (not a click's).
fn keyboard_focus_in(el: &El) -> bool {
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.states().contains(remedy_ui_style::States::FOCUS_VISIBLE) {
            return true;
        }
        stack.extend(e.child_elements());
    }
    false
}

/// Full screen, as on Portal's page: the bar floats over the foot of the picture on a
/// gradient, the [title](Player::set_title) over its head, and both fade [`WAKE`] after the
/// last pointer, touch or key activity, except while paused or busy, while the app says
/// something, while scrubbing, with a menu open, under the pointer, or with keyboard focus in
/// the bar. The captions sit above the bar while it shows. Unlike Portal's page, a faded bar
/// stays in the accessibility tree and the tab order and focus coming into it brings it
/// back, so nothing is announced as it fades and the keyboard never loses it; a press on the
/// faded bar only brings it back; a hold ending gives a whole [`WAKE`] before the fade; the
/// gradient is the theme's background, so the theme's own colors read over it; and under
/// reduced motion the controls show and hide without fading. The bar and the title line.
#[track_caller]
fn overlay(
    ui: &Ui,
    player: &Player,
    bar: &El,
    next: &El,
    rest: &std::rc::Rc<Rest>,
    menu_open: Signal<bool>,
    lift: Signal<f32>,
) -> (El, El) {
    let theme = ui.theme();
    let u = theme.spacing.unit;
    let fill = |l: &mut remedy_ui_layout::LayoutStyle| {
        l.position = Position::Absolute;
        l.inset = Insets { top: 0.0, start: 0.0, end: 0.0, bottom: 0.0 };
    };
    // The theme's background, dense at the edge of the screen and gone toward the middle.
    let scrim = |edge_at_top: bool| {
        ui.el("player_scrim").layout(fill).pass_through().paint(move |cx| {
            let (edge, clear) = (cx.theme.colors.bg.with_alpha(0.88), cx.theme.colors.bg.with_alpha(0.0));
            let (from, to) = if edge_at_top { (edge, clear) } else { (clear, edge) };
            cx.list.fill_rect(
                Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
                Brush::Linear {
                    start: remedy_ui_core::Point::new(0.0, 0.0),
                    end: remedy_ui_core::Point::new(0.0, cx.size.height),
                    stops: vec![
                        remedy_ui_paint::Stop { offset: 0.0, color: from },
                        remedy_ui_paint::Stop { offset: 1.0, color: to },
                    ],
                },
            );
        })
    };
    let back = scrim(false);
    // Over the faded bar: a press brings it back rather than pressing what cannot be seen.
    let shield = ui
        .el("player_shield")
        .layout(fill)
        .hidden(true)
        .on_pointer({
            let rest = rest.clone();
            move |e| {
                if matches!(e.phase, PointerPhase::Move | PointerPhase::Down) {
                    rest.wake();
                }
                true
            }
        })
        .keyboard_alternative({
            let u = ui.clone();
            move || tr(&u, "playback-wake-tip")
        });
    // The bar's height, for the captions to sit above it.
    let high = Signal::new(u * 16.0);
    let chrome = ui.el("player_chrome").column().child(back.clone()).child(bar.clone()).child(shield.clone());
    let chrome = chrome.on_resize(move |size| high.set_if_changed(size.height)).on_focus_within({
        let rest = rest.clone();
        move |on| {
            if on {
                rest.wake();
            }
        }
    });
    let p = player.clone();
    let words = ui.el("player_title_text").text(move || p.title().unwrap_or_default());
    let title = ui
        .el("player_title")
        .layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: 0.0, start: 0.0, end: 0.0, bottom: AUTO };
        })
        .style(remedy_ui_style::StylePatch::default().padding(Insets {
            top: u * 2.2,
            bottom: u * 6.0,
            start: u * 3.0,
            end: u * 3.0,
        }))
        .pass_through()
        .child(scrim(true))
        .child(words);

    // What the player says holds the controls; the pointer and keyboard focus are looked at
    // when the rest is up.
    let held = {
        let p = player.clone();
        move || {
            menu_open.get()
                || p.state() != PlaybackState::Playing
                || p.status().is_some()
                || p.skipping().is_some()
                || p.starting_next().is_some()
                || p.scrubbing().is_some()
        }
    };
    let _ = rest.hold.set(Box::new({
        let (c, n, held) = (chrome.clone(), next.clone(), held.clone());
        move || {
            let hovered = |e: &El| e.states().contains(remedy_ui_style::States::HOVER);
            remedy_ui_reactive::untrack(&held)
                || hovered(&c)
                || hovered(&n)
                || keyboard_focus_in(&c)
                || keyboard_focus_in(&n)
        }
    }));
    // The "Up next" card coming up shows the controls, as on Portal's page; keyboard focus
    // coming into it does too.
    let _ = next
        .clone()
        .on_visibility({
            let rest = rest.clone();
            move |seen| {
                if seen {
                    rest.wake();
                }
            }
        })
        .on_focus_within({
            let rest = rest.clone();
            move |on| {
                if on {
                    rest.wake();
                }
            }
        });
    // Going full screen, and a hold starting or ending (a media key pausing, playback
    // resuming), shows them for a whole rest.
    let (r, p, last) = (rest.clone(), player.clone(), std::cell::Cell::new(None));
    let chrome = chrome.bind(move || {
        let now = (p.is_fullscreen(), held());
        if last.replace(Some(now)) != Some(now) && now.0 {
            remedy_ui_reactive::untrack(|| r.wake());
        }
    });
    let motion = Motion::Tween { duration: FADE, easing: Easing::EASE_IN_OUT };
    let (c, t, p, n) = (chrome.clone(), title.clone(), player.clone(), next.clone());
    let float_next = move |n: &El, full: bool, high: f32| {
        // Portal's: at the foot of the picture's end side, over the bar's place whether or
        // not the bar shows (it does not jump as the bar fades), on a raised panel; it does
        // not fade, being what the person is asked about.
        let _ = n.clone().layout(move |l| match full {
            true => {
                l.position = Position::Absolute;
                l.inset = Insets { top: AUTO, start: AUTO, end: u * 2.8, bottom: high + u * 2.0 };
                l.width = Length::Px(u * 80.0);
                l.max_width = Length::Percent(1.0);
                l.axis = remedy_ui_layout::Axis::Column;
                l.align_items = Align::Stretch;
            }
            false => {
                l.position = Position::Relative;
                l.inset = Insets::ZERO;
                l.width = Length::Auto;
                l.max_width = Length::Auto;
                l.axis = remedy_ui_layout::Axis::Column;
                l.align_items = Align::Stretch;
            }
        });
        let theme = n.ui().theme();
        let look = remedy_ui_style::StylePatch::default();
        let look = match full {
            true => look
                .background(theme.colors.panel.with_alpha(0.92))
                .border(theme.colors.line.unwrap_or(theme.colors.muted))
                .border_width(1.0)
                .radius(12.0)
                .shadow(remedy_ui_style::Shadow::new(0.0, 12.0, 30.0, Color::BLACK.with_alpha(0.45))),
            false => look
                .background(Color::TRANSPARENT)
                .border_width(0.0)
                .radius(0.0)
                .shadow(remedy_ui_style::Shadow::new(0.0, 0.0, 0.0, Color::TRANSPARENT)),
        };
        let _ = n.clone().style(look);
    };
    let chrome = chrome.bind(move || {
        let full = p.is_fullscreen() && !p.is_compact();
        let shown = p.controls_shown();
        let _ = c.clone().layout(move |l| match full {
            true => {
                l.position = Position::Absolute;
                l.inset = Insets { top: AUTO, start: 0.0, end: 0.0, bottom: 0.0 };
            }
            false => {
                l.position = Position::Relative;
                l.inset = Insets::ZERO;
            }
        });
        // The theme's `player-full-pad-top`, `-bottom` and `-side` (6, 1.8 and 2.8 units).
        let pad = match full {
            true => Insets {
                top: theme.metric("player-full-pad-top", u * 6.0),
                bottom: theme.metric("player-full-pad-bottom", u * 1.8),
                start: theme.metric("player-full-pad-side", u * 2.8),
                end: theme.metric("player-full-pad-side", u * 2.8),
            },
            false => Insets { top: 0.0, bottom: 0.0, start: 0.0, end: 0.0 },
        };
        let _ = c.clone().style(remedy_ui_style::StylePatch::default().padding(pad));
        let _ = back.clone().hidden(!full);
        float_next(&n, full, high.get());
        let _ = shield.clone().hidden(shown);
        let _ = t.clone().hidden(!full || p.title().is_none());
        let to = if shown { 1.0 } else { 0.0 };
        c.animate(Target::Opacity(to), motion);
        t.animate(Target::Opacity(to), motion);
        lift.set_if_changed(if full && shown { high.get() + u * 2.0 } else { u * 3.0 });
    });
    (chrome, title)
}

/// A big Play glyph in the middle of a paused picture, as on Portal's page: a dark disc that
/// takes the accent under the pointer; a click on it plays (the stage's handler, so a double
/// click is still full screen alone). Not in the tab order and not announced (the transport
/// bar's Play is); hidden while busy, casting, or with no picture. The layer, and the glyph.
#[track_caller]
fn big_play(ui: &Ui, icons: &Option<PlayerIcons>, player: &Player) -> (El, El) {
    // The theme's `player-big-play` across (8.5 units by default), its glyph
    // `player-big-play-glyph` (4 units) nudged `player-big-play-nudge` toward the end; under
    // the pointer the accent, the glyph then `player-big-play-hover-text` (white by default).
    let theme = ui.theme();
    let u = theme.spacing.unit;
    let size = theme.metric("player-big-play", u * 8.5);
    let nudge = theme.metric("player-big-play-nudge", 0.0);
    let hover_text = theme.colors.custom.get("player-big-play-hover-text").copied().unwrap_or(Color::WHITE);
    let look = move |hover: bool, accent: Color| {
        remedy_ui_style::StylePatch::default()
            .background(if hover { accent } else { Color::BLACK.with_alpha(0.55) })
            .foreground(if hover { hover_text } else { Color::WHITE })
            .radius(size / 2.0)
            .padding(Insets { top: 0.0, bottom: 0.0, start: nudge, end: 0.0 })
    };
    let accent = ui.theme().colors.accent;
    let button = ui
        .el("player_big_play")
        .width(size)
        .height(size)
        .row()
        .align(Align::Center)
        .justify(Justify::Center)
        .style(look(false, accent))
        .child(glyph(ui, icons, Icon::Play, theme.metric("player-big-play-glyph", u * 4.0)));
    let b = button.clone();
    let button = button.on_hover(move |on| {
        let _ = b.clone().style(look(on, accent));
    });
    let layer = ui
        .el("player_big_play_layer")
        .row()
        .align(Align::Center)
        .justify(Justify::Center)
        .layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: 0.0, start: 0.0, end: 0.0, bottom: 0.0 };
        })
        .pass_through()
        .child(button.clone());
    let (l, p) = (layer.clone(), player.clone());
    let layer = layer.bind(move || {
        let paused = matches!(p.state(), PlaybackState::Paused | PlaybackState::Ended);
        let pictures = p.info().is_some_and(|i| i.video.is_some());
        let busy = p.skipping().is_some() || p.starting_next().is_some();
        let _ = l.clone().hidden(!paused || !pictures || busy || p.is_remote() || p.is_compact());
    });
    (layer, button)
}

/// Play and pause.
#[track_caller]
fn play_button(ui: &Ui, icons: &Option<PlayerIcons>, player: &Player) -> El {
    let p = player.clone();
    glyph_button(
        ui,
        icons,
        [Icon::Play, Icon::Pause],
        "playback-play-pause-tip",
        {
            let p = player.clone();
            move || usize::from(matches!(p.state(), PlaybackState::Playing | PlaybackState::Buffering))
        },
        {
            let (p, u) = (player.clone(), ui.clone());
            move || {
                let playing = matches!(p.state(), PlaybackState::Playing | PlaybackState::Buffering);
                tr(&u, if playing { "playback-pause" } else { "playback-play" })
            }
        },
        move || p.toggle(),
    )
}

/// The time: "1:05 / 3:20", the elapsed part an element of its own (`player_clock_now`, Portal's
/// `.clock-now`) between the words before and after it, all in a `player_clock`; read out
/// whole.
#[track_caller]
fn clock(ui: &Ui, player: &Player) -> El {
    let (p, tu) = (player.clone(), ui.clone());
    let said = move || {
        // Starting the next, the old media's numbers no longer hold and the new one's are
        // not known yet.
        if p.starting_next().is_some() {
            return String::new();
        }
        let pos = format_time(p.scrubbing().unwrap_or_else(|| p.position()));
        match p.duration() {
            Some(d) => crate::STRINGS.format(
                &tu,
                "playback-time",
                &[("position", pos.into()), ("duration", format_time(d).into())],
            ),
            None if p.info().is_some() => {
                crate::STRINGS.format(&tu, "playback-time-live", &[("position", pos.into())])
            }
            None => pos,
        }
    };
    let part = || ui.el("label").nowrap();
    let (before, now, after) = (part(), ui.el("player_clock_now").nowrap(), part());
    let (b, n, a, p) = (before.clone(), now.clone(), after.clone(), player.clone());
    ui.el("player_clock")
        .row()
        .align(Align::Center)
        .layout(|l| l.shrink = 0.0)
        .child(before)
        .child(now)
        .child(after)
        .bind(move || {
            let whole = said();
            let pos = format_time(p.scrubbing().unwrap_or_else(|| p.position()));
            // The catalog may isolate the numbers (U+2068 ... U+2069).
            let clean = whole.replace(['\u{2068}', '\u{2069}'], "");
            let (lead, trail) = clean.split_once(pos.as_str()).unwrap_or((clean.as_str(), ""));
            let shown = !clean.is_empty() && clean.contains(pos.as_str());
            for (el, text) in [(&b, lead), (&n, if shown { pos.as_str() } else { "" }), (&a, trail)] {
                let _ =
                    el.clone().hidden(text.is_empty()).text(remedy_ui_view::RichText::plain(text.to_owned()));
            }
            let _ = b.parent().map(|c| c.semantics(Semantics::new(Role::Label).label(clean.clone())));
        })
}

/// The player's glyph size: the theme's `player-glyph`, else the body text and 3.
fn glyph_size(ui: &Ui) -> f32 {
    let t = ui.theme();
    t.metric("player-glyph", t.type_scale.body + 3.0)
}

/// Gives `els` (the bar and its parts) the sheet's `full` variant while full screen, and the
/// glyphs in their buttons the theme's `player-glyph-full` size then.
fn full_screen_look(ui: &Ui, player: &Player, els: Vec<El>) {
    let t = ui.theme();
    let (normal, large) = (glyph_size(ui), t.metric("player-glyph-full", glyph_size(ui)));
    for el in els {
        let p = player.clone();
        let me = el.clone();
        let _ = el.variant({
            let p = p.clone();
            move || if p.is_fullscreen() && !p.is_compact() { "full" } else { "" }
        });
        if normal == large || me.info().is_none_or(|i| i.widget != "player_button") {
            continue;
        }
        let _ = me.clone().bind(move || {
            let size = if p.is_fullscreen() && !p.is_compact() { large } else { normal };
            for g in me.child_elements() {
                let _ = g.layout(move |l| {
                    l.width = Length::Px(size);
                    l.height = Length::Px(size);
                });
            }
        });
    }
}

/// Cancel under the spinner, while the app's work (starting the next, a skip) can be stopped
/// ([`Player::offer_cancel`]), as on Portal's page; disabled once pressed.
#[track_caller]
fn stage_cancel(ui: &Ui, player: &Player) -> El {
    let p = player.clone();
    let button = w::button(
        ui,
        {
            let u = ui.clone();
            move || tr(&u, "playback-next-cancel")
        },
        move || p.press_cancel(),
    );
    drops_focus_after_click(ui, player, &button);
    let (b, p) = (button.clone(), player.clone());
    button.bind(move || {
        let _ = b.clone().hidden(!p.cancel_shown() || p.is_compact()).disabled(p.canceling());
    })
}

/// What plays where, in remote mode: the device's name and state over the app's poster, in
/// place of the picture; announced politely as it changes.
#[track_caller]
fn cast_stage(ui: &Ui, player: &Player) -> El {
    let u = ui.theme().spacing.unit;
    let fill = |l: &mut remedy_ui_layout::LayoutStyle| {
        l.position = Position::Absolute;
        l.inset = Insets { top: 0.0, start: 0.0, end: 0.0, bottom: 0.0 };
    };
    // The app's poster fills the stage behind the words, darkened so they read over any art.
    let poster =
        ui.el("player_poster").column().align(Align::Center).justify(Justify::Center).layout(fill).clip();
    let scrim = ui.el("player_scrim").layout(fill).pass_through().paint(|cx| {
        let (top, bottom) = (Color::BLACK.with_alpha(0.55), Color::BLACK.with_alpha(0.78));
        cx.list.fill_rect(
            Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
            Brush::Linear {
                start: remedy_ui_core::Point::new(0.0, 0.0),
                end: remedy_ui_core::Point::new(0.0, cx.size.height),
                stops: vec![
                    remedy_ui_paint::Stop { offset: 0.0, color: top },
                    remedy_ui_paint::Stop { offset: 1.0, color: bottom },
                ],
            },
        );
    });
    // "Playing on" small and muted, the device's name large, in the order the language puts them.
    let words = |size: f32, weight: f32, color: Color| {
        ui.el("label")
            .style(remedy_ui_style::StylePatch::default().text_size(size).weight(weight).foreground(color))
    };
    let lead_color = Color::rgb8(0xd9, 0xd1, 0xc2);
    let before = words(u * 3.6, 400.0, lead_color);
    let name_line = words(u * 6.0, 600.0, Color::rgb8(0xf4, 0xef, 0xe4));
    let after = words(u * 3.6, 400.0, lead_color);
    let text = ui
        .el("player_cast_text")
        .column()
        .align(Align::Center)
        .justify(Justify::Center)
        .gap(u * 1.2)
        .layout(fill)
        .style(remedy_ui_style::StylePatch::default().padding(Insets {
            top: u * 4.0,
            bottom: u * 4.0,
            start: u * 4.0,
            end: u * 4.0,
        }))
        .child(before.clone())
        .child(name_line.clone())
        .child(after.clone());
    let panel =
        ui.el("player_cast").column().grow(1.0).child(poster.clone()).child(scrim).child(text.clone());
    let (pa, p, tu) = (panel.clone(), player.clone(), ui.clone());
    let shown = std::cell::Cell::new(None);
    panel.bind(move || {
        let name = p.remote_name();
        let _ = pa.clone().hidden(name.is_none());
        let Some(name) = name else { return };
        let next = p.starting_next();
        let id = match p.state() {
            _ if next.is_some() => "playback-cast-next",
            PlaybackState::Opening => "playback-cast-connecting",
            PlaybackState::Paused => "playback-cast-paused",
            PlaybackState::Playing => "playback-cast-playing",
            PlaybackState::Buffering => "playback-cast-buffering",
            PlaybackState::Ended => "playback-cast-ended",
            PlaybackState::Failed => "playback-cast-failed",
        };
        let said = crate::STRINGS.format(
            &tu,
            id,
            &[("device", name.clone().into()), ("title", next.unwrap_or_default().into())],
        );
        // The words either side of the name (which the catalog may isolate, U+2068 ... U+2069).
        let clean = said.replace(['\u{2068}', '\u{2069}'], "");
        let (lead, trail) = clean.split_once(name.as_str()).unwrap_or(("", ""));
        for (line, s) in [(&before, lead.trim()), (&name_line, name.as_str()), (&after, trail.trim())] {
            let _ = line.clone().hidden(s.is_empty());
            let _ = line.clone().text(remedy_ui_view::RichText::plain(s.to_owned()));
        }
        let _ = text.clone().semantics(Semantics::new(Role::Status).label(said).live(Live::Polite));
        // The app's poster, put in when it changes.
        let art = p.remote_poster();
        if shown.get() != art.as_ref().map(El::id) {
            poster.clear();
            if let Some(a) = &art {
                let _ = poster.clone().child(a.clone());
            }
            shown.set(art.as_ref().map(El::id));
        }
    })
}

/// "Up next" at the end of the media, when something is set to play next, as Portal's card:
/// a small "UP NEXT" over the title on a line of its own (announced as "Up next: …" as it
/// shows), the countdown while it runs, a wide Play now, and an X named "Dismiss" (which
/// stops the countdown and puts the card away). Shown between the picture and the transport
/// bar, so sound-only media has it too; floating over the picture in full screen.
#[track_caller]
fn next_up_card(ui: &Ui, icons: &Option<PlayerIcons>, player: &Player) -> El {
    let u = ui.theme().spacing.unit;
    let dismissed = Signal::new(false);
    // The title and the countdown are announced as they show and change.
    let said = |text: String| Semantics::new(Role::Status).label(text).live(Live::Polite);
    // Drawn in capitals, as a label above a title is set (casing follows the language's own
    // rules); read and copied as written.
    let label =
        ui.el("player_next_label").nowrap().text_transform(remedy_ui_view::TextTransform::Upper).text({
            let u = ui.clone();
            move || tr(&u, "playback-next-up-label")
        });
    let title = ui.el("player_next_title");
    let (t, p, tu) = (title.clone(), player.clone(), ui.clone());
    let title = title.bind(move || {
        let title = p.next_up().map(|n| n.title).unwrap_or_default();
        let whole = crate::STRINGS.format(&tu, "playback-next-up", &[("title", title.clone().into())]);
        let _ = t.clone().text(remedy_ui_view::RichText::plain(title)).semantics(said(whole));
    });
    let left = ui.el("label").nowrap();
    let (lb, p, tu) = (left.clone(), player.clone(), ui.clone());
    let left = left.bind(move || {
        let countdown = p.countdown();
        let _ = lb.clone().hidden(countdown.is_none());
        // Whole seconds, rounded up: 5, 4, ... 1.
        let Some(d) = countdown else { return };
        let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
        let text = crate::STRINGS.format(&tu, "playback-next-up-in", &[("count", secs.into())]);
        let _ = lb.clone().text(remedy_ui_view::RichText::plain(text.clone())).semantics(said(text));
    });
    let p = player.clone();
    let now = w::button(
        ui,
        {
            let u = ui.clone();
            move || tr(&u, "playback-play-now")
        },
        move || p.play_next(),
    )
    // A sheet can set it apart (Portal's is in the accent); the shared one leaves it a button.
    .variant("player_play_now")
    .grow(1.0);
    let p = player.clone();
    let dismiss = glyph_button(
        ui,
        icons,
        [Icon::Close, Icon::Close],
        "playback-next-dismiss",
        || 0,
        {
            let u = ui.clone();
            move || tr(&u, "playback-next-dismiss")
        },
        move || {
            p.cancel_countdown();
            dismissed.set(true);
        },
    );
    drops_focus_after_click(ui, player, &now);
    drops_focus_after_click(ui, player, &dismiss);
    let card = ui
        .el("player_next")
        .column()
        .align(Align::Stretch)
        .gap(u)
        .style(remedy_ui_style::StylePatch::default().padding(Insets {
            top: u,
            bottom: u,
            start: u * 1.5,
            end: u * 1.5,
        }))
        .semantics(Semantics::new(Role::Group))
        .child(label)
        .child(title)
        .child(left)
        .child(ui.el("player_next_actions").row().align(Align::Center).gap(u).child(now).child(dismiss));
    let c = card.clone();
    let p = player.clone();
    let seen = std::cell::RefCell::new(None);
    card.bind(move || {
        // At the end, or near it when the app asks (Portal's last 20 seconds).
        // Portal's way: while casting, the device moves on by itself and no card shows.
        let away = p.next_up_early() && p.is_remote();
        let due = !away && (p.state() == PlaybackState::Ended || p.next_up_near());
        let next = p.next_up();
        // A new end, or something new next, brings the card back.
        if !due || *seen.borrow() != next {
            dismissed.set_if_changed(false);
        }
        *seen.borrow_mut() = next.clone();
        // Starting the next, the stage says so in place of the card.
        let shown = due && next.is_some() && !dismissed.get() && p.starting_next().is_none();
        let _ = c.clone().hidden(!shown);
        if let Some(n) = next {
            let _ = c.clone().semantics(Semantics::new(Role::Group).label(n.title));
        }
    })
}

/// One caption line in its own readable box, as a browser draws a cue's line.
fn caption_line(ui: &Ui, line: &str) -> El {
    let u = ui.theme().spacing.unit;
    ui.el("player_caption")
        .style(remedy_ui_style::StylePatch::default().padding(Insets {
            top: u * 0.5,
            bottom: u * 0.5,
            start: u * 1.5,
            end: u * 1.5,
        }))
        .text(remedy_ui_view::RichText::plain(line.to_owned()))
}

/// Draws cues laid out by WebVTT's rules on the picture shown at `aspect` in the stage,
/// above the bar `lift` up from the foot: each line in a readable box as `player_caption`
/// draws one (light text on a dark box). Vertical cues follow CSS's `text-orientation:
/// mixed`: CJK, kana, Hangul and other upright characters one under another, and runs of
/// Latin and other sideways text turned a quarter clockwise, reading downward.
fn paint_cues(cx: &mut remedy_ui_view::PaintCx<'_>, cues: &[crate::Cue], aspect: Option<f32>, lift: f32) {
    use crate::cue_layout::vertical_runs;
    let theme = cx.theme;
    let u = theme.spacing.unit;
    let vp = crate::cue_layout::viewport(cx.size, aspect);
    // The theme's caption look, if it sets one: `player-caption-height` (the text as a share
    // of the picture's height, as a browser sizes cues) or `player-caption-size`, the box's
    // `player-caption-radius` and `player-caption-pad-x`/`-y`, and the colors
    // `player-caption-text` and `player-caption-box`; else body text, white on black at 80%.
    let size = match theme.metrics.get("player-caption-height") {
        Some(share) => share * vp.height(),
        None => theme.metric("player-caption-size", theme.type_scale.body),
    };
    let token = |name: &str| theme.colors.custom.get(name).copied();
    let text = remedy_ui_view::TextPaint {
        size,
        weight: 400.0,
        color: token("player-caption-text").unwrap_or(Color::WHITE),
        mono: false,
    };
    let fill = token("player-caption-box").unwrap_or(Color::BLACK.with_alpha(0.8));
    let radius = CornerRadii::all(theme.metric("player-caption-radius", theme.radii.small));
    let pad = (theme.metric("player-caption-pad-x", u * 1.5), theme.metric("player-caption-pad-y", u * 0.5));
    // The title area: the picture above the bar, which floats over its foot in full screen.
    let title = Rect::new(vp.min.x, vp.min.y, vp.max.x, vp.max.y.min(cx.size.height - lift).max(vp.min.y));
    // An upright character's step down a vertical line, and a sideways run's width across
    // it: the line height.
    let step = cx.measure_text("M", text).height;
    let placed = crate::cue_layout::layout(cues, vp, title, pad, &mut |t, down| {
        if !down {
            return cx.measure_text(t, text);
        }
        // Down the line: a step per upright character, a sideways run's own length; across
        // it, the widest upright character or a line height for sideways text.
        let (mut along, mut across) = (0.0f32, 0.0f32);
        for (upright, run) in vertical_runs(t) {
            if upright {
                for ch in run.chars() {
                    along += step;
                    across = across.max(cx.measure_text(ch.encode_utf8(&mut [0; 4]), text).width);
                }
            } else {
                along += cx.measure_text(&run, text).width;
                across = across.max(step);
            }
        }
        remedy_ui_core::Size::new(along, across)
    });
    let mid = remedy_ui_core::Vec2::new(0.5, 0.5);
    for cue in placed {
        for line in cue.lines {
            cx.list.fill_rounded(line.rect, radius, Brush::Solid(fill));
            let c = line.rect.center();
            if cue.vertical.is_none() {
                cx.text(&line.text, c, mid, text);
                continue;
            }
            let runs = vertical_runs(&line.text);
            let len: f32 = runs
                .iter()
                .map(
                    |(up, r)| {
                        if *up { r.chars().count() as f32 * step } else { cx.measure_text(r, text).width }
                    },
                )
                .sum();
            let mut y = c.y - len / 2.0;
            for (upright, run) in runs {
                if upright {
                    for ch in run.chars() {
                        cx.text(
                            ch.encode_utf8(&mut [0; 4]),
                            remedy_ui_core::Point::new(c.x, y + step / 2.0),
                            mid,
                            text,
                        );
                        y += step;
                    }
                    continue;
                }
                // Sideways: the run turned a quarter clockwise about its middle.
                let w = cx.measure_text(&run, text).width;
                let at = remedy_ui_core::Point::new(c.x, y + w / 2.0);
                let turn = remedy_ui_paint::Affine::translate(remedy_ui_core::Vec2::new(-at.x, -at.y))
                    .then(remedy_ui_paint::Affine::rotate(std::f32::consts::FRAC_PI_2))
                    .then(remedy_ui_paint::Affine::translate(remedy_ui_core::Vec2::new(at.x, at.y)));
                cx.list.push(remedy_ui_paint::Item::PushTransform(turn));
                cx.text(&run, at, mid, text);
                cx.list.push(remedy_ui_paint::Item::PopTransform);
                y += w;
            }
        }
    }
}
/// The caption timing in words: "in sync", "now 1.5 s later", "now 0.5 s earlier".
fn timing_words(u: &Ui, ms: i64) -> String {
    let secs = (ms.unsigned_abs() as f64) / 1000.0;
    match ms.signum() {
        0 => tr(u, "playback-timing-in-sync"),
        1 => crate::STRINGS.format(u, "playback-timing-later", &[("seconds", secs.into())]),
        _ => crate::STRINGS.format(u, "playback-timing-earlier", &[("seconds", secs.into())]),
    }
}

/// The name of what shows: the track's, else the app's choice picked.
fn shown_name(p: &Player) -> Option<String> {
    let tracks = p.subtitle_tracks();
    if let Some(t) = p.caption_track().and_then(|i| tracks.get(i)) {
        return Some(t.label.clone());
    }
    match p.caption_pick() {
        CaptionPick::Choice(id) => p.caption_choices().into_iter().find(|c| c.id == id).map(|c| c.label),
        _ => None,
    }
}

/// The captions button's words: short on the button ("Off", "Auto · English", "Loading…"),
/// whole in its name ("Captions: auto, English").
fn caption_words(u: &Ui, p: &Player, short: bool) -> String {
    let id = |s: &str| {
        if short { format!("playback-captions-short-{s}") } else { format!("playback-captions-{s}") }
    };
    let name = shown_name(p);
    let fmt = |key: String, track: String| crate::STRINGS.format(u, &key, &[("track", track.into())]);
    match (p.captions_loading(), p.caption_pick(), name) {
        (true, ..) => tr(u, &id("loading")),
        (false, CaptionPick::Off, _) => tr(u, &id("off")),
        (false, CaptionPick::Auto, Some(n)) => fmt(id("auto-track"), n),
        (false, CaptionPick::Auto, None) => tr(u, &id("auto")),
        (false, _, Some(n)) if short => n,
        (false, _, Some(n)) => fmt("playback-captions-track".into(), n),
        (false, _, None) => tr(u, &id("off")),
    }
}

/// The captions control: one button that says what shows ("Off", "Auto · English",
/// "Loading…") and opens the captions menu (Off, Auto when the app offers it, the tracks, the
/// app's choices with their details, its note, and the timing a half second at a time);
/// C turns captions off and back on as last picked. Shown while it can do something.
#[track_caller]
fn captions_control(ui: &Ui, icons: &Option<PlayerIcons>, player: &Player, open: Signal<bool>) -> El {
    let u = ui.theme().spacing.unit;
    let size = glyph_size(ui);
    let glyph = glyph(ui, icons, Icon::Captions, size);
    let (p, tu) = (player.clone(), ui.clone());
    let words = ui.el("label").nowrap().text(move || caption_words(&tu, &p, true));
    let button = ui
        .el("button")
        .row()
        .align(Align::Center)
        .gap(u * 1.5)
        .layout(|l| l.shrink = 0.0)
        .child(glyph.clone())
        .child(words);
    let (me, p, mu) = (button.clone(), player.clone(), ui.clone());
    let button = button
        .on_click(move || {
            let layer = w::menu_press_through(&mu, &me, caption_menu(&mu, &p));
            open.set(true);
            let _ = layer.on_removed(move || open.set(false));
        })
        .semantics({
            let (p, u) = (player.clone(), ui.clone());
            move || {
                let s = Semantics::new(Role::Button)
                    .label(caption_words(&u, &p, false))
                    .expanded(open.get())
                    .action(Action::Click);
                match p.caption_delay() {
                    0 => s,
                    ms => s.description(timing_words(&u, ms)),
                }
            }
        });
    {
        let u = ui.clone();
        w::tooltip(&button, move || tr(&u, "playback-captions-tip"));
    }
    let (b, p) = (button.clone(), player.clone());
    let button = button.bind(move || {
        let _ = b.clone().hidden(!p.captions_offered());
        b.state(remedy_ui_style::States::CHECKED, p.caption_pick() != CaptionPick::Off);
    });
    pulse_while_loading(ui, player, &glyph);
    button
}

/// The captions menu's entries, as they are when it opens; their marks and the timing follow.
fn caption_menu(ui: &Ui, p: &Player) -> Vec<MenuItem> {
    let pick = |want: CaptionPick| {
        let (a, b) = (p.clone(), p.clone());
        let w2 = want.clone();
        (move || a.caption_pick() == want, move || b.set_caption_pick(w2.clone()))
    };
    let mut items = Vec::new();
    let (on, set) = pick(CaptionPick::Off);
    items.push(MenuItem::radio(tr(ui, "playback-captions-menu-off"), on, set));
    if remedy_ui_reactive::untrack(|| p.auto_offered()) {
        let (on, set) = pick(CaptionPick::Auto);
        let (q, u) = (p.clone(), ui.clone());
        items.push(MenuItem::radio(String::new(), on, set).label(move || {
            match (q.caption_pick() == CaptionPick::Auto, q.captions_loading(), shown_name(&q)) {
                (true, true, _) => tr(&u, "playback-captions-short-loading"),
                (true, false, Some(n)) => {
                    crate::STRINGS.format(&u, "playback-captions-menu-auto-track", &[("track", n.into())])
                }
                _ => tr(&u, "playback-captions-menu-auto"),
            }
        }));
    }
    let tracks = remedy_ui_reactive::untrack(|| p.subtitle_tracks());
    if !tracks.is_empty() {
        items.push(MenuItem::separator());
    }
    for (i, t) in tracks.into_iter().enumerate() {
        let (on, set) = pick(CaptionPick::Track(i));
        items.push(MenuItem::radio(t.label, on, set));
    }
    let choices = remedy_ui_reactive::untrack(|| p.caption_choices());
    if !choices.is_empty() {
        items.push(MenuItem::separator());
    }
    for c in choices {
        let (on, set) = pick(CaptionPick::Choice(c.id.clone()));
        let name = match &c.detail {
            Some(d) => format!("{} · {d}", c.label),
            None => c.label.clone(),
        };
        let (q, u, id) = (p.clone(), ui.clone(), c.id);
        items.push(MenuItem::radio(String::new(), on, set).label(move || {
            match q.captions_loading() && q.caption_pick() == CaptionPick::Choice(id.clone()) {
                true => crate::STRINGS.format(
                    &u,
                    "playback-captions-menu-loading",
                    &[("label", name.clone().into())],
                ),
                false => name.clone(),
            }
        }));
    }
    if let Some(note) = remedy_ui_reactive::untrack(|| p.caption_note()) {
        items.push(MenuItem::new(note, || {}).disabled(true));
    }
    // A TV shows the file as it is, as on Portal's page: no timing while remote.
    if !remedy_ui_reactive::untrack(|| p.is_remote()) {
        items.push(MenuItem::separator());
        // Portal's timing step by default: half a second, within ten minutes either way.
        let ms = |d: Duration| i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        let (by, limit) = (ms(p.options().caption_step), ms(p.options().caption_delay_limit));
        let step = |key: &'static str, by: i64| {
            let (a, b, u) = (p.clone(), p.clone(), ui.clone());
            MenuItem::new(String::new(), move || {
                let to = a.caption_delay().saturating_add(by).clamp(-limit, limit);
                a.set_caption_delay(to);
            })
            .label(move || {
                crate::STRINGS.format(&u, key, &[("timing", timing_words(&u, b.caption_delay()).into())])
            })
            .stays_open()
        };
        items.push(step("playback-captions-earlier", -by));
        items.push(step("playback-captions-later", by));
        let (a, b) = (p.clone(), p.clone());
        items.push(
            MenuItem::new(tr(ui, "playback-captions-reset"), move || a.set_caption_delay(0))
                .disabled(move || b.caption_delay() == 0)
                .stays_open(),
        );
    }
    items
}

/// Half of Portal's `subs-pulse` (1.2 s, ease-in-out, down to 0.45 at the middle).
const PULSE_HALF: Duration = Duration::from_millis(600);

/// While captions load, the glyph pulses as Portal's does (down to 0.45 and back every
/// 1.2 s): a cue beside the button's "Loading…", so none under reduced motion.
fn pulse_while_loading(ui: &Ui, player: &Player, glyph: &El) {
    // Each start or stop is a new run; a finished half of an old one ends there.
    let run = std::rc::Rc::new(std::cell::Cell::new(0u64));
    fn half(glyph: El, run: std::rc::Rc<std::cell::Cell<u64>>, me: u64, down: bool) {
        let motion = Motion::Tween { duration: PULSE_HALF, easing: Easing::EASE_IN_OUT };
        let (g, r) = (glyph.clone(), run.clone());
        glyph.animate_then(Target::Opacity(if down { 0.45 } else { 1.0 }), motion, move || {
            if r.get() == me {
                half(g, r, me, !down);
            }
        });
    }
    let (p, u, g, last) = (player.clone(), ui.clone(), glyph.clone(), std::cell::Cell::new(None));
    let _ = glyph.clone().bind(move || {
        let pulse = p.captions_loading() && !u.reduced_motion();
        if last.replace(Some(pulse)) == Some(pulse) {
            return;
        }
        let me = run.get() + 1;
        run.set(me);
        match pulse {
            true => half(g.clone(), run.clone(), me, true),
            false => g.set_animated(Target::Opacity(1.0)),
        }
    });
}

/// An icon button whose glyph (`glyphs[which()]`) and name follow the player, with the
/// message `tip` as its tooltip.
#[track_caller]
fn glyph_button(
    ui: &Ui,
    icons: &Option<PlayerIcons>,
    glyphs: [Icon; 2],
    tip: &'static str,
    which: impl Fn() -> usize + 'static,
    name: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> El {
    // The theme's `player-glyph` (the body text and 3 by default); the button's size and shape
    // are the sheet's `player_button`.
    let size = glyph_size(ui);
    let [first, second] = glyphs;
    let a = glyph(ui, icons, first, size);
    let b = glyph(ui, icons, second, size);
    let (a2, b2) = (a.clone(), b.clone());
    let button = ui
        .el("player_button")
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
    let u = ui.clone();
    w::tooltip(&button, move || tr(&u, tip));
    button
}

/// The seek bar: the played part in the accent color, what can play without waiting
/// ([`Player::buffered`]) in a lighter tone of the text color, and a thumb at the playhead.
/// Pressing or dragging (mouse, pen or touch) holds the thumb and the time at the pointer
/// ([`Player::scrubbing`]) and seeks once, on release; a cancelled drag goes back. A tip over
/// the bar shows the time under the pointer, as on Portal's page. Arrows move 5 seconds,
/// Page Up and Down a tenth, Home and End to the ends, at once.
#[track_caller]
fn seek_bar(ui: &Ui, player: &Player) -> El {
    // The theme's sizes: `player-seek-height` (a control high by default), the track
    // `player-seek-track` thick (4) and `player-seek-track-hover` under the pointer, the thumb
    // `player-thumb` across (0.6 of the height, at least 10) and `player-thumb-hover`, ringed
    // `player-thumb-ring` wide (none) in the color `player-thumb-ring`.
    let theme = ui.theme();
    let h = theme.metric("player-seek-height", theme.spacing.control_height);
    let unit = theme.spacing.unit;
    let thumb = theme.metric("player-thumb", cx_radius(h) * 2.0) / 2.0;
    let thumb_hover = theme.metric("player-thumb-hover", thumb * 2.0) / 2.0;
    let track = theme.metric("player-seek-track", 4.0) / 2.0;
    let track_hover = theme.metric("player-seek-track-hover", track * 2.0) / 2.0;
    let ring = theme.metric("player-thumb-ring", 0.0);
    // The playhead's travel stops the larger thumb's radius from either end.
    let end = thumb.max(thumb_hover);
    let bar = ui.el("player_seek").height(h).grow(1.0).focusable(true);
    let length = {
        let p = player.clone();
        move || p.duration().filter(|d| !d.is_zero())
    };
    // The time under `x`: on a timeline in whole seconds, as its seeks are.
    let (b2, p, len) = (bar.clone(), player.clone(), length.clone());
    let time_at = move |x: f32| -> Option<Duration> {
        let (b, d) = (b2.bounds()?, remedy_ui_reactive::untrack(&len)?);
        let r = (end, b.width());
        let t = ((x - b.min.x - r.0) / (r.1 - 2.0 * r.0).max(1.0)).clamp(0.0, 1.0);
        let at = d.mul_f64(f64::from(t));
        let film = remedy_ui_reactive::untrack(|| p.timeline().is_some());
        Some(if film { Duration::from_secs(at.as_secs()) } else { at })
    };
    // The tip with the time under the pointer: Portal's #dock-hover, above the bar.
    // Its look is the sheet's `player_tip`.
    let tip = ui.el("player_tip").nowrap().pass_through().hidden(true).layout(move |l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: AUTO, start: 0.0, end: AUTO, bottom: h + unit * 0.5 };
    });
    let scrub = ui.el("player_scrub").grow(1.0).row().align(Align::Center);
    let (t2, s2, p, time) = (tip.clone(), scrub.clone(), player.clone(), time_at.clone());
    let show_tip = move |x: Option<f32>| {
        let busy = remedy_ui_reactive::untrack(|| p.skipping().is_some() || p.starting_next().is_some());
        let (Some(x), Some(host), false) = (x, s2.bounds(), busy) else {
            let _ = t2.clone().hidden(true);
            return;
        };
        let Some(at) = time(x) else { return };
        let _ = t2.clone().hidden(false).text(remedy_ui_view::RichText::plain(format_time(at)));
        // Centered on the pointer, kept 4 px inside the bar's ends.
        let half = t2.bounds().map_or(0.0, |b| b.width() / 2.0);
        let x = (x - host.min.x).max(half + 4.0).min((host.width() - half - 4.0).max(half + 4.0));
        let from = if s2.is_rtl() { host.width() - x - half } else { x - half };
        let _ = t2.clone().layout(|l| l.inset.start = from);
    };
    let (p, len, time) = (player.clone(), length.clone(), time_at);
    let painted = player.clone();
    let b3 = bar.clone();
    let p2 = player.clone();
    let u = ui.clone();
    let len2 = length.clone();
    let (tip_hover, tip_moves) = (show_tip.clone(), show_tip);
    let bar = bar
        .paint(move |cx| {
            let pos = painted.scrubbing().unwrap_or_else(|| painted.position());
            let t = match len() {
                Some(d) => (pos.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0) as f32,
                None => 0.0,
            };
            let d = cx.theme.derive();
            // The theme's colors, if it has them: `track`, `player-loaded` and `player-thumb`,
            // and in full screen `player-track-full` and `player-loaded-full`.
            let token = |name: &str| cx.theme.colors.custom.get(name).copied();
            let full = painted.is_fullscreen() && !painted.is_compact();
            let pick = |name: &str| token(&format!("{name}-full")).filter(|_| full).or_else(|| token(name));
            let track_color = pick("track").unwrap_or(d.line);
            let loaded = pick("player-loaded").unwrap_or(d.text.with_alpha(0.35));
            let thumb_color = token("player-thumb").unwrap_or(d.text);
            let on = cx.states.contains(remedy_ui_style::States::HOVER)
                && !cx.states.contains(remedy_ui_style::States::DISABLED);
            let (r, half) = if on { (thumb_hover, track_hover) } else { (thumb, track) };
            let mid = cx.size.height / 2.0;
            let (x0, x1) = (end, cx.size.width - end);
            let x = x0 + (x1 - x0) * t;
            cx.list.fill_rounded(
                Rect::new(x0, mid - half, x1, mid + half),
                CornerRadii::all(half),
                Brush::Solid(track_color),
            );
            if let Some(len) = len() {
                let at = |d: Duration| {
                    x0 + (x1 - x0) * (d.as_secs_f64() / len.as_secs_f64()).clamp(0.0, 1.0) as f32
                };
                for r in if painted.is_remote() { Vec::new() } else { painted.buffered() } {
                    let (a, b) = (at(r.start), at(r.end));
                    if b > a {
                        cx.list.fill_rounded(
                            Rect::new(a, mid - half, b, mid + half),
                            CornerRadii::all(half),
                            Brush::Solid(loaded),
                        );
                    }
                }
            }
            cx.list.fill_rounded(
                Rect::new(x0, mid - half, x, mid + half),
                CornerRadii::all(half),
                Brush::Solid(cx.theme.colors.accent),
            );
            if ring > 0.0 {
                let ring_color = token("player-thumb-ring").unwrap_or(Color::TRANSPARENT);
                let o = r + ring;
                cx.list.fill_rounded(
                    Rect::new(x - o, mid - o, x + o, mid + o),
                    CornerRadii::all(o),
                    Brush::Solid(ring_color),
                );
            }
            cx.list.fill_rounded(
                Rect::new(x - r, mid - r, x + r, mid + r),
                CornerRadii::all(r),
                Brush::Solid(thumb_color),
            );
        })
        .on_hover({
            let me = b3.clone();
            move |inside| {
                if !inside {
                    tip_hover(None);
                }
                // The track and thumb grow under the pointer.
                me.repaint();
            }
        })
        .on_pointer({
            let p = player.clone();
            move |e| {
                let held = remedy_ui_reactive::untrack(|| p.scrubbing());
                let x = e.position.x;
                match e.phase {
                    PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => p.set_scrubbing(time(x)),
                    PointerPhase::Move if held.is_some() => p.set_scrubbing(time(x).or(held)),
                    PointerPhase::Up if held.is_some() => {
                        p.set_scrubbing(None);
                        p.seek(held.unwrap_or_default());
                    }
                    PointerPhase::Cancel => p.set_scrubbing(None),
                    PointerPhase::Move => {
                        tip_moves(Some(x));
                        return false;
                    }
                    _ => return false,
                }
                // A finger leaves no pointer over the bar once lifted.
                let gone = matches!(e.phase, PointerPhase::Up | PointerPhase::Cancel)
                    && e.kind != PointerKind::Mouse;
                tip_moves(if gone { None } else { Some(x) });
                true
            }
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
            let pos = p2.scrubbing().unwrap_or_else(|| p2.position());
            // Repainted as more arrives, and going in and out of full screen.
            let _ = (p2.buffered(), p2.is_fullscreen(), p2.is_compact());
            let len = length();
            // On a timeline a seek the stream cannot take goes to the app; skipping, none does.
            let seekable = p2.is_remote() || p2.timeline().is_some() || p2.info().is_some_and(|i| i.seekable);
            let _ = b3.clone().disabled(
                len.is_none()
                    || !seekable
                    || p2.skipping().is_some()
                    || p2.starting_next().is_some()
                    || (p2.status().is_some_and(|s| s.busy) && !p2.is_remote()),
            );
            let total = len.unwrap_or_default();
            let said = crate::STRINGS.format(
                &u,
                "playback-seek-value",
                &[("position", format_time(pos).into()), ("duration", format_time(total).into())],
            );
            let _ = b3.clone().semantics(
                Semantics::new(Role::Slider)
                    .label(tr(&u, "playback-seek"))
                    .numeric(pos.as_secs_f64(), 0.0, total.as_secs_f64())
                    .value(said)
                    .action(Action::Increment)
                    .action(Action::Decrement),
            );
            b3.repaint();
        });
    drops_focus_after_click(ui, player, &bar);
    scrub.child(bar).child(tip)
}

/// The thumb's radius on a bar `h` tall (as the core slider draws it).
fn cx_radius(h: f32) -> f32 {
    (h * 0.3).max(5.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_are_explained_in_the_language_asked() {
        let said = failure("de", Some(MediaError::NotFound("clip.mp4".into())));
        assert_eq!(
            said.replace(['\u{2068}', '\u{2069}'], ""),
            "Dies kann nicht wiedergegeben werden, weil es nicht gefunden wurde: clip.mp4"
        );
        assert_eq!(failure("en", None), "This can't be played.");
    }
}
