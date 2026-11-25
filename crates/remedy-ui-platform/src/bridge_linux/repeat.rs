//! The desktop's keyboard repeat (delay and interval), from whichever source the session
//! has: the Wayland compositor's `wl_keyboard.repeat_info`, the X server's XKB controls,
//! GNOME's settings (through the portal, read by the caller) or KDE's `kcminputrc`.

use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use x11rb::protocol::xkb::{self, ConnectionExt as _};

/// A delay in milliseconds and a rate in repeats a second as a delay and an interval; a
/// rate of 0 (repeat turned off) is no answer, so the next source is asked.
pub(crate) fn from_rate(delay_ms: f64, rate: f64) -> Option<(Duration, Duration)> {
    if !(delay_ms >= 0.0 && rate > 0.0) {
        return None;
    }
    // A value too large for a Duration (a hand-edited settings file) is no answer either.
    Duration::try_from_secs_f64(delay_ms / 1000.0).ok().zip(Duration::try_from_secs_f64(1.0 / rate).ok())
}

/// KDE's repeat from `kcminputrc`'s `[Keyboard]` group (`RepeatDelay` in milliseconds,
/// `RepeatRate` a second). KDE writes only what the user changed, so a missing key is
/// Plasma's own default (600 ms, 25 a second).
pub(crate) fn kcminputrc(text: &str) -> Option<(Duration, Duration)> {
    let mut group = "";
    let (mut delay, mut rate) = (600.0, 25.0);
    for line in text.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g;
        } else if group == "Keyboard"
            && let Some((k, v)) = line.split_once('=')
        {
            let v = v.trim().parse::<f64>().ok();
            match k.trim() {
                "RepeatDelay" => delay = v.unwrap_or(delay),
                "RepeatRate" => rate = v.unwrap_or(rate),
                _ => {}
            }
        }
    }
    from_rate(delay, rate)
}

/// KDE's file, when this is a KDE session or the user has one.
fn kde() -> Option<(Duration, Duration)> {
    let home = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"));
    let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or(home)?;
    let text = std::fs::read_to_string(config.join("kcminputrc")).ok();
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|d| d == "KDE"));
    match text {
        Some(t) => kcminputrc(&t),
        None if kde => kcminputrc(""),
        None => None,
    }
}

/// The repeat the Wayland compositor gives keyboards (it sends `repeat_info` to every
/// keyboard of a seat, version 4 on).
fn wayland() -> Option<(Duration, Duration)> {
    std::env::var_os("WAYLAND_DISPLAY")?;
    let conn = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Wl>(&conn).ok()?;
    let qh = queue.handle();
    let _seat: WlSeat = globals.bind(&qh, 4..=9, ()).ok()?;
    let mut state = Wl::default();
    // The first round trip brings the seat's capabilities (and the keyboard is asked for
    // then), the second the keyboard's repeat.
    for _ in 0..3 {
        queue.roundtrip(&mut state).ok()?;
        if state.repeat.is_some() {
            break;
        }
    }
    let (rate, delay) = state.repeat?;
    from_rate(f64::from(delay), f64::from(rate))
}

#[derive(Default)]
struct Wl {
    keyboard: Option<WlKeyboard>,
    /// Repeats a second and the delay in milliseconds.
    repeat: Option<(i32, i32)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Wl {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Wl {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(c) } = event
            && c.contains(wl_seat::Capability::Keyboard)
            && state.keyboard.is_none()
        {
            state.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<WlKeyboard, ()> for Wl {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::RepeatInfo { rate, delay } = event {
            state.repeat = Some((rate, delay));
        }
    }
}

/// The X server's repeat (XKB's controls), when repeat is on.
fn x11() -> Option<(Duration, Duration)> {
    std::env::var_os("DISPLAY")?;
    let (c, _) = x11rb::connect(None).ok()?;
    c.xkb_use_extension(1, 0).ok()?.reply().ok()?.supported.then_some(())?;
    let r = c.xkb_get_controls(xkb::ID::USE_CORE_KBD.into()).ok()?.reply().ok()?;
    let on = r.enabled_controls.contains(xkb::BoolCtrl::REPEAT_KEYS);
    (on && r.repeat_interval > 0).then(|| {
        (
            Duration::from_millis(u64::from(r.repeat_delay)),
            Duration::from_millis(u64::from(r.repeat_interval)),
        )
    })
}

/// The session's keyboard repeat: what the display server applies first (the compositor
/// on Wayland, else the X server), then GNOME's settings (`gnome`), then KDE's file. `None`
/// when nothing says, and the defaults apply.
pub(crate) fn system(gnome: Option<(Duration, Duration)>) -> Option<(Duration, Duration)> {
    wayland()
        .or_else(|| if std::env::var_os("WAYLAND_DISPLAY").is_some() { None } else { x11() })
        .or(gnome)
        .or_else(kde)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn kde_reads_its_keyboard_group_with_plasma_defaults() {
        assert_eq!(kcminputrc(""), Some((ms(600), ms(40))));
        let file = "[Mouse]\nRepeatDelay=1\n\n[Keyboard]\nNumLock=0\nRepeatDelay=250\nRepeatRate=30.5\n";
        let (d, i) = kcminputrc(file).unwrap_or_default();
        assert_eq!(d, ms(250));
        assert!((i.as_secs_f64() - 1.0 / 30.5).abs() < 1e-9);
        assert_eq!(kcminputrc("[Keyboard]\nRepeatRate=50\n"), Some((ms(600), ms(20))));
        assert_eq!(kcminputrc("[Keyboard]\nRepeatRate=0\n"), None, "repeat off says nothing");
    }

    #[test]
    fn rates_become_intervals() {
        assert_eq!(from_rate(500.0, 25.0), Some((ms(500), ms(40))));
        assert_eq!(from_rate(500.0, 0.0), None);
        assert_eq!(from_rate(-1.0, 25.0), None);
        assert_eq!(from_rate(f64::NAN, 25.0), None);
        assert_eq!(from_rate(1e300, 25.0), None, "too long for a Duration");
        assert_eq!(from_rate(500.0, 1e-300), None);
    }

    #[test]
    fn the_session_answers_sanely_or_not_at_all() {
        // Wherever this runs (WSLg's Wayland and X11, a bare CI box), a value is plausible.
        if let Some((d, i)) = system(None) {
            assert!(
                d <= Duration::from_secs(10) && i > Duration::ZERO && i <= Duration::from_secs(10),
                "{d:?} {i:?}"
            );
        }
    }
}
