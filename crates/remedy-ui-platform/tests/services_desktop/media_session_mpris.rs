//! The media controls on Linux for real: the session registers an MPRIS player on the session
//! bus, and the controlling side (as GNOME Shell or `playerctl` would) reads it back and presses
//! its buttons. Run under `dbus-run-session -- cargo test`; without a session bus it says so and
//! passes.

#![cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use futures_core::Stream;
use remedy_ui_platform::services_desktop::media_session::{
    Artwork, MediaCommand, MediaSession, MediaState, NowPlaying, NowPlayingSink,
};
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Proxy};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

fn get(conn: &Connection, name: &str, iface: &str, prop: &str) -> OwnedVariant {
    // A plain Get each time: a proxy's cache could answer from an older PropertiesChanged.
    let got = zbus::block_on(async {
        let props = Some("org.freedesktop.DBus.Properties");
        let m = conn.call_method(Some(name), PATH, props, "Get", &(iface, prop)).await?;
        m.body().deserialize::<OwnedVariant>()
    });
    got.unwrap_or_else(|e| unreachable!("{iface}.{prop}: {e}"))
}

fn text(v: OwnedVariant) -> String {
    String::try_from(v).unwrap_or_else(|e| unreachable!("a string: {e}"))
}

fn flag(v: OwnedVariant) -> bool {
    bool::try_from(v).unwrap_or_else(|e| unreachable!("a boolean: {e}"))
}

fn number(v: OwnedVariant) -> i64 {
    i64::try_from(v).unwrap_or_else(|e| unreachable!("an int64: {e}"))
}

fn metadata(conn: &Connection, name: &str) -> HashMap<String, OwnedVariant> {
    HashMap::try_from(get(conn, name, PLAYER, "Metadata")).unwrap_or_else(|e| unreachable!("a{{sv}}: {e}"))
}

#[test]
fn mpris_shows_what_plays_and_passes_its_buttons_on() {
    let Ok(conn) = zbus::block_on(Connection::session()) else {
        eprintln!("skipped: no session bus (run under dbus-run-session)");
        return;
    };
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let session = MediaSession::new(move |c| {
        let _ = tx.lock().map(|t| t.send(c));
    })
    .unwrap_or_else(|e| unreachable!("a session bus is here: {e}"));

    let names = zbus::block_on(async { zbus::fdo::DBusProxy::new(&conn).await?.list_names().await })
        .unwrap_or_else(|e| unreachable!("ListNames: {e}"));
    let suffix = format!(".instance{}", std::process::id());
    let name = names
        .iter()
        .map(|n| n.to_string())
        .find(|n| n.starts_with("org.mpris.MediaPlayer2.") && n.ends_with(&suffix))
        .unwrap_or_else(|| unreachable!("our player is on the bus: {names:?}"));

    assert!(!flag(get(&conn, &name, "org.mpris.MediaPlayer2", "CanRaise")));
    assert!(!text(get(&conn, &name, "org.mpris.MediaPlayer2", "Identity")).is_empty());
    assert!(!flag(get(&conn, &name, PLAYER, "CanPlay")), "nothing loaded yet");

    let png = b"\x89PNG\r\n\x1a\nnot really".to_vec();
    session.set_now_playing(Some(
        &NowPlaying::new("The Ris").subtitle("S1 E3").art(Artwork::Bytes(png.clone())),
    ));
    session.set_playback(MediaState::Playing, Duration::from_secs(30), Some(Duration::from_secs(100)));
    session.set_can_next(true);

    let m = metadata(&conn, &name);
    let field = |k: &str| m.get(k).and_then(|v| v.try_clone().ok()).unwrap_or_else(|| unreachable!("{k}"));
    assert_eq!(text(field("xesam:title")), "The Ris");
    let artist = Vec::<String>::try_from(field("xesam:artist")).unwrap_or_default();
    assert_eq!(artist, ["S1 E3"]);
    assert_eq!(number(field("mpris:length")), 100_000_000);
    let art = text(field("mpris:artUrl"));
    let file = art.strip_prefix("file://").unwrap_or_else(|| unreachable!("a file URL: {art}"));
    assert_eq!(std::fs::read(file).ok(), Some(png), "the artwork bytes are in the file");
    let track = OwnedObjectPath::try_from(field("mpris:trackid")).unwrap_or_else(|e| unreachable!("{e}"));

    assert_eq!(text(get(&conn, &name, PLAYER, "PlaybackStatus")), "Playing");
    let pos = number(get(&conn, &name, PLAYER, "Position"));
    assert!((30_000_000..40_000_000).contains(&pos), "position moves on while playing: {pos}");
    for p in ["CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek", "CanControl"] {
        assert!(flag(get(&conn, &name, PLAYER, p)), "{p}");
    }

    let player = zbus::block_on(Proxy::new(&conn, name.clone(), PATH, PLAYER))
        .unwrap_or_else(|e| unreachable!("the player proxy: {e}"));
    let call = |method: &str, want: MediaCommand, body: &dyn Fn(&Proxy<'_>) -> zbus::Result<()>| {
        body(&player).unwrap_or_else(|e| unreachable!("{method}: {e}"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).ok(), Some(want), "{method}");
    };
    let plain = |m: &'static str| move |p: &Proxy<'_>| zbus::block_on(p.call::<_, _, ()>(m, &()));
    call("Play", MediaCommand::Play, &plain("Play"));
    call("Pause", MediaCommand::Pause, &plain("Pause"));
    call("PlayPause", MediaCommand::PlayPause, &plain("PlayPause"));
    call("Stop", MediaCommand::Stop, &plain("Stop"));
    call("Next", MediaCommand::Next, &plain("Next"));
    call("Previous", MediaCommand::Previous, &plain("Previous"));
    call("Seek", MediaCommand::SeekBy(-10.0), &|p| {
        zbus::block_on(p.call::<_, _, ()>("Seek", &(-10_000_000i64)))
    });
    let to = (track.clone(), 42_000_000i64);
    call("SetPosition", MediaCommand::SeekTo(Duration::from_secs(42)), &|p| {
        zbus::block_on(p.call::<_, _, ()>("SetPosition", &to))
    });
    // Another item's position is ignored, as MPRIS says.
    let stale =
        (OwnedObjectPath::try_from("/dev/remedyui/track/0").unwrap_or_else(|e| unreachable!("{e}")), 1i64);
    zbus::block_on(player.call::<_, _, ()>("SetPosition", &stale)).unwrap_or_else(|e| unreachable!("{e}"));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "a stale track's SetPosition is ignored");

    // A jump in position is announced with Seeked.
    let (seen_tx, seen) = mpsc::channel();
    let watch = player.clone();
    let mut stream = zbus::block_on(watch.receive_signal("Seeked")).unwrap_or_else(|e| unreachable!("{e}"));
    std::thread::spawn(move || {
        let msg = zbus::block_on(std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)));
        let _ = seen_tx.send(msg.and_then(|m| m.body().deserialize::<i64>().ok()));
    });
    session.set_playback(MediaState::Paused, Duration::from_secs(80), Some(Duration::from_secs(100)));
    assert_eq!(seen.recv_timeout(Duration::from_secs(5)).ok().flatten(), Some(80_000_000));
    assert_eq!(text(get(&conn, &name, PLAYER, "PlaybackStatus")), "Paused");
    assert_eq!(number(get(&conn, &name, PLAYER, "Position")), 80_000_000);

    // The volume: what the player says reads back, and setting it asks the player.
    let volume = |c: &Connection| f64::try_from(get(c, &name, PLAYER, "Volume")).unwrap_or(-1.0);
    assert_eq!(volume(&conn), 1.0, "full until the player says");
    session.set_volume(0.25);
    assert_eq!(volume(&conn), 0.25);
    for (asked, want) in [(0.5, 0.5), (-3.0, 0.0), (7.0, 1.0)] {
        zbus::block_on(async {
            let props = Some("org.freedesktop.DBus.Properties");
            let v = zbus::Variant::from(asked);
            conn.call_method(Some(name.as_str()), PATH, props, "Set", &(PLAYER, "Volume", v)).await
        })
        .unwrap_or_else(|e| unreachable!("Set Volume: {e}"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).ok(), Some(MediaCommand::SetVolume(want)));
    }
    assert_eq!(volume(&conn), 0.25, "the shown volume waits for the player");

    session.set_can_next(false);
    assert!(!flag(get(&conn, &name, PLAYER, "CanGoNext")));

    drop(session);
    assert!(std::fs::metadata(file).is_err(), "the artwork file goes with the session");
}
