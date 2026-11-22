# Media: images, animations, video and playback

This guide covers showing pictures and playing media:

- still images: PNG, JPEG, GIF, WebP, BMP and ICO ([`crate::image`], feature `image`)
- SVG files and tinted icons ([`crate::svg`], feature `svg`)
- animated GIF, APNG and WebP ([`crate::anim`], feature `anim`)
- video surfaces fed by the app's own decoder or capture ([`crate::video`], feature `video`)
- sound and video files and streams through the platform's decoders, with a player widget
  and captions ([`crate::playback`], feature `playback`)

Nothing here decodes on the UI thread. Images are decoded at the size they are shown, so a
4000-pixel photo in a 200-pixel thumbnail costs a 200-pixel decode.

## Images

[`crate::image::image()`] makes an element showing a [`crate::image::Source`]: a file, read
off the UI thread, or bytes already in memory (a download, a decrypted file, an embedded
asset). The element:

- shows the theme's placeholder while it loads, then the image, fitted to its box by
  [`crate::image::Fit`] (`Contain`, `Cover`, `Fill`, `ScaleDown`);
- decodes again, larger, when the element grows;
- keeps the image's aspect ratio, and takes the image's own size when nothing else sizes
  it;
- shows a broken-image mark when the image cannot be shown, and tells screen readers why.

`alt` says what the image shows for screen readers. An empty `alt` marks it decorative, so
screen readers skip it. Elements showing the same source share one decoded copy, and it is
freed when nothing shows it.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::image::{Fit, Source, image};
use std::sync::Arc;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let photo = Source::File("holiday.jpg".into());
let logo: Arc<[u8]> = Arc::from(&b"<svg xmlns='http://www.w3.org/2000/svg' width='40' height='20'/>"[..]);

ui.root()
    .child(image(&ui, photo, "The beach at sunset", Fit::Cover).width(200.0).height(120.0))
    .child(image(&ui, Source::Bytes(logo), "", Fit::Contain));
# app.frames(1);

// The photo is named for screen readers; the decorative logo is left out.
let reader = ui.reader_view();
assert!(reader.iter().any(|n| n.name.as_deref() == Some("The beach at sunset")));
assert!(!reader.iter().any(|n| n.name.as_deref() == Some("")));
```

### Images from addresses

`Source::Url("https://…")` fetches the image off the UI thread. Elements showing the same
address share one fetch, cancelled when they all go. Fetched bytes stay in a bounded
memory cache. Natively they are also kept in the app's cache folder (`images` under
`%LOCALAPPDATA%\<name>\cache`, `~/Library/Caches/<name>` or `~/.cache/<name>`), following
the server's `Cache-Control` and revalidated with its `ETag` or `Last-Modified`. In a
browser page the browser's own HTTP cache does that. [`crate::image::image_ext`] adds the
app's own placeholder, a fallback built when the image cannot be shown (a letter tile), and
a listener whose `Sized` and `Ready` carry the image's real size, so wide or square art can
be framed to fit:

```no_run
# fn demo(ui: &remedy_ui::Ui) {
use remedy_ui::image::{Fit, ImageOptions, ImageState, Source, image_ext};

let ui2 = ui.clone();
let poster = image_ext(
    ui,
    Source::Url("https://example.com/posters/42.jpg".into()),
    "Poster",
    Fit::Cover,
    ImageOptions::new()
        .fallback(move || ui2.el("letter-tile").text("A"))
        .on_state(|s| {
            if let ImageState::Sized((w, h)) = s {
                let _square = w == h;
            }
        }),
);
# let _ = poster;
# }
```

In a browser page images are drawn on the canvas, so an image from another site shows only
if its server allows it with CORS headers (`Access-Control-Allow-Origin`). If not, the load
fails with `DecodeError::Network` and the reason says why. Fetch such images through your
own server instead (a same-origin image proxy). The engine does not lay a DOM `<img>` over
the canvas, because that image could not be clipped, rounded, layered under popups and
dialogs, faded, transformed or hit-tested with the rest of the UI.

To fetch images with your own HTTP client (cookies, auth headers), implement
[`crate::image::Fetch`] and install it with [`crate::image::set_fetcher`].

For custom elements (an avatar, a preview), [`crate::image::load`] is the loader the
element uses: it reports [`crate::image::ImageState`] (`Loading`, `Sized`, `Ready`,
`Failed`) into a signal. [`crate::image::decode`] decodes bytes on the calling thread, at a
size you choose, and returns premultiplied pixels:

```
use remedy_ui::image::{DecodeError, decode};

remedy_ui::svg::install(); // SVG through the same loader (`run` does this for you)
let svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='64' height='32'/>";
let shown = decode(svg, Some((32, 16)))?;
assert_eq!(shown.source_size, (64, 32), "the source's own size");
assert_eq!((shown.image.width, shown.image.height), (32, 16), "drawn at the size asked for");

// Damaged or unknown data is an error, never a crash.
assert!(decode(b"not an image", None).is_err());
# Ok::<(), DecodeError>(())
```

## SVG and icons

With the `svg` feature, [`run`](crate::run) adds SVG to the image loader
([`crate::svg::install`]), so [`crate::image::image()`] shows SVG files and bytes like any
other image, drawn at exactly the size shown. [`crate::svg::icon`] draws an SVG icon in the
element's text color (the SVG's `currentColor`), so icons follow the theme and the element's
state. SVG files are untrusted: references to other files and to the network are never
followed.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use std::sync::Arc;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let bell: Arc<[u8]> = Arc::from(
    &br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
           <path fill="currentColor" d="M12 2a7 7 0 0 0-7 7v5l-2 3h18l-2-3V9a7 7 0 0 0-7-7z"/>
         </svg>"#[..],
);
ui.root().child(remedy_ui::svg::icon(&ui, bell, 24.0, "Notifications"));
# app.frames(1);
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Notifications")));

// Or straight to pixels, with `currentColor` set.
let pixels = remedy_ui::svg::render(b"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'/>", Some((16, 16)), None);
assert!(pixels.is_ok_and(|img| img.width == 16));
```

The core widgets also carry a set of built-in vector icons ([`crate::widgets::Icon`],
[`crate::widgets::icon`]) that need no feature: actions (add, close, copy, delete, search,
send, refresh), places and things (home, folder, file, image, note, computer, people), media
controls, and text formatting. An icon drawn with `icon` is decorative, because whatever holds
it names it (an [`icon_button`](crate::widgets::icon_button)'s label). Where the icon alone
stands for something (a file's kind in a list), [`Icon::accessible_name`](crate::widgets::Icon::accessible_name)
gives its English name to translate and hand to the element's semantics.

```
use remedy_ui::a11y::{Role, Semantics};
use remedy_ui::testing::TestApp;
use remedy_ui::widgets::{self as w, Icon};

let mut app = TestApp::window(|ui, _| {
    let kind = w::icon(ui, Icon::Image, 16.0)
        .semantics(Semantics::new(Role::Image).label(Icon::Image.accessible_name()));
    ui.root().child(w::row(ui).child(w::icon_button(ui, Icon::Home, "Home", || {})).child(kind));
});
assert!(app.find(Role::Button, "Home").is_some());
assert!(app.find(Role::Image, "Image").is_some());
```

## Animated images

[`crate::anim::animated()`] plays GIF, APNG and animated WebP, for chats and feeds where many
play at once. It takes the same [`crate::anim::Source`] and [`crate::anim::Fit`] as images.

- Frames are decoded on the image workers at the size shown. A short animation is kept
  whole; a longer one keeps a window of frames, so memory follows the size shown, not the
  length.
- It plays only while some of it is on screen and its window is not minimized or covered.
  Each frame has a timer of its own length, so nothing runs between frames.
- Under reduced motion it shows the first frame with a play button, named for screen
  readers and operable by keyboard, and plays only when pressed.
- Elements showing the same source at the same size share one animation and play in step.

[`crate::anim::set_memory_budget`] sets how much memory decoded frames may use in all, and
[`crate::anim::trim_memory`] lets animations nothing plays give theirs back (the element does
this itself under memory pressure). [`crate::anim::register`] teaches the still-image loader
to show an APNG's first frame.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::anim::{Fit, Source, animated};
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let sticker = animated(&ui, Source::File("party.gif".into()), "A dancing cat", Fit::Contain);
ui.root().child(sticker.width(120.0).height(120.0));
# app.frames(1);
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("A dancing cat")));
```

## Video surfaces

A video surface shows frames the app pushes: a screen share, a camera, a call's tiles, the
picture of the app's own decoder. [`crate::video::video_surface`] returns the element and a
[`crate::video::VideoSink`]. The sink is cheap to clone and works from any thread. Frames
stay in the layout they come in (NV12, I420 or RGBA) all the way to the renderer, which
converts them on the GPU.

Frames are never queued. A frame that arrives before the last one was drawn replaces it and
counts as dropped, so a slow window never falls behind a live stream.
[`VideoSink::stats`](crate::video::VideoSink::stats) says how many were pushed, shown and
dropped, and [`VideoSink::push`](crate::video::VideoSink::push) returns false once the
element is gone, which tells the producer to stop.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::video::{Picture, PixelFormat, VideoFit, video_surface};
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let (screen, sink) = video_surface(&ui, "Alex's screen", VideoFit::Contain);
ui.root().child(screen.width(320.0).height(180.0));

// The capture thread pushes frames; here, one grey 4 x 4 RGBA frame.
let feed = sink.clone();
let pushed = std::thread::spawn(move || {
    feed.push(Picture::packed(4, 4, PixelFormat::Rgba, vec![128u8; 4 * 4 * 4]))
});
assert!(pushed.join().is_ok_and(|ok| ok));

for _ in 0..200 {
    app.frames(1);
    if sink.stats().shown > 0 {
        break;
    }
    std::thread::sleep(std::time::Duration::from_millis(5));
}
assert_eq!(sink.stats().shown, 1);
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Alex's screen")));
```

## Playback

[`crate::playback::Player`] opens a [`crate::playback::MediaSource`] (a file or a URL) on its
own threads and plays it. Its state, position, length, volume, captions and errors are
signals, so the UI follows them like any other state.

- [`Player::open_platform`](crate::playback::Player::open_platform) uses the platform's own
  decoders: Media Foundation and WASAPI on Windows, the system's GStreamer on Linux (loaded
  at run time, never bundled), AVFoundation and Audio Queues on macOS, MediaCodec and AAudio
  on Android.
- Sound goes to the default output, and its clock (how much has been heard) decides when
  each picture shows, so sound and picture stay together however busy the machine is.
  Without a sound device the monotonic clock does.
- [`crate::playback::player()`] is the widget: the picture with captions over it and a
  transport bar (play and pause, the time, a seek bar, mute and volume, captions, and a
  full-screen request). K plays and pauses, J and L jump 10 seconds, M mutes, C turns
  captions on and off, F asks for full screen; on the seek bar the arrows move 5 seconds.
- In full screen the bar floats over the picture with the title the app gives
  ([`Player::set_title`](crate::playback::Player::set_title)) and fades with the mouse
  pointer 2.6 s after the last activity, unless paused, busy, scrubbing, hovered, a menu is
  open or keyboard focus is in the bar; a faded bar stays reachable by keyboard and screen
  readers, and any key, touch or movement brings it back.
- Media with pictures pauses while unseen ([`crate::playback::UnseenPolicy`]) and does not
  start by itself under reduced motion.

```no_run
use remedy_ui::playback::{MediaSource, Player, PlayerOptions, player};
use remedy_ui::{App, AppConfig, WindowSpec};

remedy_ui::run(AppConfig::new("Talks"), |app: &App| {
    app.open(WindowSpec::new("Talk").size(960.0, 600.0), |ui, window| {
        let talk = Player::open_platform(ui, MediaSource::file("talk.mp4"), PlayerOptions::default());
        // The player asks for full screen; the window does it.
        let w = window.clone();
        talk.on_fullscreen(move |on| w.set_fullscreen(on));
        ui.root().child(player(ui, &talk, "Talk recording"));
    });
})
.ok();
```

An app with its own decoder implements [`crate::playback::MediaHost`] (a
[`crate::playback::Decoder`] for the source and an [`crate::playback::AudioOutput`] for the
sound) and opens it with [`Player::open`](crate::playback::Player::open). Here is a host that
decodes a second of silence, with no sound device, so the player keeps time by the clock:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::playback::{
    AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource,
    PlaybackState, Player, PlayerOptions, Sample, player,
};
use std::sync::Arc;
use std::time::Duration;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

const RATE: u32 = 8000;

struct Silence {
    info: MediaInfo,
    next: u32,
}

impl Decoder for Silence {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        // Ten tenths of a second of sound, then the end.
        if self.next == 10 {
            return Ok(None);
        }
        let at = Duration::from_millis(u64::from(self.next) * 100);
        self.next += 1;
        Ok(Some(Sample::Audio { at, samples: vec![0.0; (RATE / 10) as usize] }))
    }
    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.next = u32::try_from(to.as_millis() / 100).unwrap_or(10).min(10);
        Ok(())
    }
}

struct SilenceHost;

impl MediaHost for SilenceHost {
    fn name(&self) -> &str {
        "silence"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let mut info = MediaInfo::default();
        info.duration = Some(Duration::from_secs(1));
        info.seekable = true;
        info.audio = Some(AudioFormat::new(RATE, 1));
        Ok(Box::new(Silence { info, next: 0 }))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("this host has no sound device".into()))
    }
}

let p = Player::open(&ui, Arc::new(SilenceHost), MediaSource::url("silence:"), PlayerOptions::default());
ui.root().child(player(&ui, &p, "One second of silence"));
for _ in 0..400 {
    app.frames(1);
    if p.state() == PlaybackState::Paused {
        break;
    }
    std::thread::sleep(Duration::from_millis(5));
}
assert_eq!(p.state(), PlaybackState::Paused, "opened, waiting for play");
assert_eq!(p.duration(), Some(Duration::from_secs(1)));

// Sound-only media shows the transport bar alone, operable by keyboard and screen readers.
let reader = ui.reader_view();
assert!(reader.iter().any(|n| n.name.as_deref() == Some("Play")));
assert!(reader.iter().any(|n| n.name.as_deref() == Some("Seek")));
```

A player that cannot open its source is [`PlaybackState::Failed`](crate::playback::PlaybackState::Failed)
with [`Player::error`](crate::playback::Player::error) saying why (a
[`crate::playback::MediaError`]: not found, not playable, not supported here...), and the
widget shows that message over the picture.

## Captions

The player reads WebVTT, SRT and the basics of ASS/SSA. With
[`PlayerOptions::find_subtitles`](crate::playback::PlayerOptions::find_subtitles) on (the
default) it finds caption files next to a local file: the same name with `.vtt`, `.srt`,
`.ass` or `.ssa`, or with a language before it (`film.en.srt`).
[`Player::load_subtitles`](crate::playback::Player::load_subtitles) reads a file the person
picks, [`Player::add_subtitles`](crate::playback::Player::add_subtitles) adds a track the app
made, and [`Player::set_caption_track`](crate::playback::Player::set_caption_track) chooses
which one shows. [`crate::playback::parse_subtitles`] reads a file's text on its own:

```
use remedy_ui::playback::{SubtitleFormat, SubtitleTrack, parse_subtitles};
use std::time::Duration;

let vtt = "WEBVTT\n\n00:00.500 --> 00:02.000\nHello there.\n\n00:02.500 --> 00:04.000\nWelcome back.\n";
assert_eq!(SubtitleFormat::sniff(vtt), Some(SubtitleFormat::WebVtt));
let cues = parse_subtitles(vtt, SubtitleFormat::WebVtt);
assert_eq!(cues.len(), 2);

let track = SubtitleTrack { label: "English".into(), language: Some("en".into()), cues };
assert_eq!(track.text_at(Duration::from_secs(1)).as_deref(), Some("Hello there."));
assert_eq!(track.text_at(Duration::from_millis(2200)), None, "between cues");
```

## The microphone

[`Microphone`](crate::playback::Microphone) opens the default sound input on a worker of its
own and hands its sound to the app as it arrives: 48 kHz mono by default
([`MicrophoneOptions`](crate::playback::MicrophoneOptions) asks for another rate or channel
count, and the platform or the engine converts from the device's own). The handler runs on
the worker, never on the UI thread, so pitch detection or a recorder does not hold up
drawing; results go back with [`crate::reactive::waker`]. The microphone's
[`state`](crate::playback::Microphone::state), [`error`](crate::playback::Microphone::error)
and [`level`](crate::playback::Microphone::level) (for a meter) are tracked signals.

The sound is the microphone's own, without voice-call processing (echo cancelling, noise
suppression, gain control) that would bend a sung or played note. WASAPI captures it on
Windows, the system's GStreamer on Linux, AAudio on Android, and `getUserMedia` with an audio
worklet in a browser page (which asks the person itself, and only on `https:` or localhost
pages).

Android asks at run time: call [`App::request_permission`](crate::App::request_permission)
with [`Permission::Microphone`](crate::host::Permission::Microphone) when the person reaches
for the feature, and list `microphone` in `remedy.toml`'s `[android] permissions`. A refusal,
or Windows' privacy settings keeping apps from the microphone, is a
[`MediaError::Denied`](crate::playback::MediaError::Denied) that says where to change it.

```no_run
use remedy_ui::App;
use remedy_ui::host::Permission;
use remedy_ui::playback::{Microphone, MicrophoneOptions};
use std::cell::RefCell;
use std::rc::Rc;

fn start_tuner(app: &App, mic: Rc<RefCell<Option<Microphone>>>) {
    app.request_permission(Permission::Microphone, move |granted| {
        if !granted {
            return; // Say why the tuner cannot listen, and how to allow it.
        }
        *mic.borrow_mut() = Some(Microphone::open_platform(MicrophoneOptions::default(), |samples| {
            // 48 kHz mono, as it arrives: estimate the pitch here.
            let _ = samples;
        }));
    });
}
```

### Listening with the screen off

Android stops hearing the microphone, and may stop the app, once it leaves the screen. Work
that must go on (a voice call, a recording) runs as foreground work:
[`App::start_foreground`](crate::App::start_foreground) with a
[`ForegroundTask`](crate::host::ForegroundTask) of `ForegroundKind::Microphone` (and
`MediaPlayback` when it also plays the far side), shown by a notification whose buttons reach
[`App::on_notification`](crate::App::on_notification). Start it while the app is on screen,
after the microphone permission, and list the kinds in `remedy.toml`'s `[android] foreground`
([`crate::guide::packaging`]). Desktop and web apps keep listening anyway, and there the call
answers `Ok` and does nothing.

```no_run
use remedy_ui::App;
use remedy_ui::host::{ForegroundKind, ForegroundTask, Notification, NotificationResponse, Permission};

const CALL: u64 = 1;

fn start_call(app: &App) {
    let app2 = app.clone();
    app.request_permission(Permission::Microphone, move |granted| {
        if !granted {
            return;
        }
        let shown = Notification::new(CALL, "In a call", "Ada").action("hang-up", "Hang up").silent(true);
        let task = ForegroundTask::new(ForegroundKind::Microphone, shown).kind(ForegroundKind::MediaPlayback);
        app2.start_foreground(task, |started| {
            if let Err(why) = started {
                eprintln!("the call cannot go on out of sight: {why}");
            }
        });
    });
    let app3 = app.clone();
    app.on_notification(move |id, response| {
        if id == CALL && response == NotificationResponse::Action("hang-up".into()) {
            app3.stop_foreground(); // and end the call itself
        }
    });
}
```

## Decoding in a sandbox

Images and animations come from strangers in chats and feeds. [`crate::isolated`] (feature
`isolated`) decodes them in a separate, sandboxed helper process (an AppContainer on Windows,
seccomp-bpf and Landlock on Linux, Seatbelt on macOS) that gets bytes and returns pixels over
shared memory. A decoder that crashes, hangs or runs away with memory takes down only the
helper: the image shows its error state, the helper starts again and the app carries on.

The helper is the app's own executable started again, so an app ships one binary. Call
[`isolated::helper_main`](crate::isolated::helper_main) first thing in `main` (in the helper
it never returns), and [`isolated::enable`](crate::isolated::enable) once at start-up: from
then on [`crate::image`] and [`crate::anim`] decode through helpers.
[`isolated::stats`](crate::isolated::stats) says how many helpers started and why any were
stopped.

```no_run
use remedy_ui::{App, AppConfig, isolated};

fn main() {
    isolated::helper_main();
    remedy_ui::run(AppConfig::new("Chat"), |app: &App| {
        // Starting the first helper takes as long as starting a small program: off the UI thread.
        app.background(
            || isolated::enable().map(|how| format!("{how:?}")).map_err(|e| e.to_string()),
            |sandbox| eprintln!("image decoding: {sandbox:?}"),
        );
    })
    .ok();
}
```

When `enable` fails (a platform where no sandbox can be built), nothing changes and images
decode in the app as before.
