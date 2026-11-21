# Changelog

All notable changes to RemedyUI are in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and RemedyUI follows
[Semantic Versioning](https://semver.org/) from 1.0 ([docs/STABILITY.md](docs/STABILITY.md)).
Before 1.0, minor versions may break; each break is listed under "Changed" or "Removed"
with what to do, and `cargo remedy migrate` applies the mechanical ones.

## [Unreleased]

### Added

- **The player's keys from anywhere in the window** in `media-playback`, as Portal's page
  hears them page-wide: `Player::keys_from(&ui.root())` routes the keys nothing focused took
  to the player under its own `KeyScheme` (Classic or Arrows; in remote mode to the device),
  except that a focused text field keeps every key, a focused slider its own, a focused
  button Space and Enter, and nothing reaches the player while a menu or dialog has focus or
  is open; keys with Ctrl, Alt or the system key held never do. It acts only while the
  player shows (a `player` widget or `player_picture` of it is seen, compact and remote
  included), as Portal's keys only while its dock is open, so the app need not switch it.
  `Player::handle_key` applies the same key map to one key and says whether it took it; the
  `player` widget uses it too, so there is one key map. Each key either of them hears wakes the
  full-screen controls, as a key inside the player does (Portal's `wake()`).
- **Going compact closes the player's menus** in `media-playback`, as Portal's mini player
  does: `Player::set_compact(true)` closes the captions menu and any menu the app opened
  anchored to the player's buttons (the cast menu from `on_cast`). Full screen leaves them
  open, as Portal's page does. `widgets-core`'s new `close_menus_in(&el)` closes every open
  menu opened from `el` or from inside it, without moving focus.
- **Caption cue settings** in `media-playback`: WebVTT's `line` (a percentage or a line
  number), `position` (with its anchor), `size`, `align` and `vertical` (read; laid out across),
  SRT's `{\anN}`, and ASS/SSA's `\anN` with its styles' alignment and margins in the script's
  `PlayResX` by `PlayResY` are read into `Cue::settings` (`CueSettings`), from files and from
  Matroska's and MP4's embedded tracks. The `player` widget lays each such cue out by
  WebVTT's rules (`CueSettings::box_across`); cues with none stay centred at the foot,
  above the bar in full screen. `Player::caption_cues` gives the cues showing with their
  settings. `Cue` has a new `settings` field (`..Default::default()` in literals).

- **Portal's full-screen overlay** in `media-playback`. In full screen the `player`
  widget's bar floats over the foot of the picture on a gradient, the title the app gives
  (`Player::set_title(Some(..))`) over its head, and both fade 2.6 s after the last pointer,
  touch or key activity; they stay while paused or busy, while the app's status shows, while
  scrubbing, with the captions menu open, under the pointer, or with keyboard focus in the
  bar. The mouse pointer hides with them; captions sit above the bar while it shows;
  `Player::controls_shown` says which. Better than Portal's page: a faded bar stays in the
  accessibility tree and the tab order, so nothing is announced as it fades, and focus
  moving into it brings it back; a tap or press on the faded controls only brings them back
  (Portal's paused or pressed what could not be seen); a hold ending gives a whole 2.6 s
  before the fade; going full screen brings focus to the player when it is elsewhere, so
  its keys work; the gradient is the theme's background, so the theme's own colors read
  over it; and under reduced motion the controls show and hide without fading. The "Up
  next" card floats over the picture's end side above the bar, on a raised panel, as
  Portal's does: it does not fade or move with the bar, coming up shows the controls,
  under the pointer or with keyboard focus it holds them, and Tab reaches its buttons.
- **Captions are centred**, each line in its own box as a browser draws a cue (Portal's
  page): they were one box across the picture with the text at its start edge. Cue
  settings (WebVTT `align`, `position`, `line`; SRT and ASS `{\an}`) are still dropped by
  the parser, so every cue shows at the foot, centred.
- **Hiding the mouse pointer.** `El::cursor_hidden(true)` hides the pointer while it is over
  the element; the runner tells the platform through the new `Host::set_cursor_hidden`
  (a default no-op): winit's `set_cursor_visible` on Windows, macOS, X11 and Wayland
  (Android has no pointer), `cursor: none` on a page's window, recorded by the headless host
  (`HeadlessWindow::is_cursor_hidden`). `Ui::cursor_hidden` says what the window wants.
  Checked on real windows with a full-screen player: `GetCursorInfo` on Windows, XFixes'
  cursor image on X11 (WSLg's XWayland) (`remedy-ui/tests/real_cursor.rs`, run with
  `--ignored`: it takes the screen and the mouse), and the canvas's computed `cursor` in
  headless Firefox (`media-playback/tests/web_cursor.rs`, in `web-test.sh`). On Windows
  winit hides the pointer while it is over the window.
- The accessibility audit skips the contrast of text faded out entirely (opacity 0): there
  is nothing to read.
- **The player owns what Portal worked around** in `media-playback`.
  `Player::set_next_up_delay` and `set_next_up_lead` (with tracked `next_up_delay()` and
  `next_up_lead()`) change autoplay while it plays: a countdown running counts to the new
  length from when it began, `None` stops it, and at the end one starts, so the app no longer
  sets the delay to `None` and calls `play_next()` on Ended itself. `PlayerOptions::keep_awake`
  and `Player::set_keep_awake` hold the screen on while video plays here and is seen (not
  paused, ended, remote, unseen or sound only), let go on pause, end and drop; the hold is the
  system's with the new `keep-awake` feature (also in the facade), or the app's own through
  `Player::set_awake_hold` (tests use a fake). `RemoteStatus::reported_at(Instant)` and
  `age(Duration)` say when the TV reported: a playing device's position is moved on from then,
  at once and between reports. `Player::add_subtitles_stream` adds a track timed to the stream
  (subtitles a server cut to its stream); the player moves it onto the work's time, so it stays
  right across `replace_source` with a new origin. New message `playback-keep-awake` in all 11
  locales (the reason the system lists).
- **Keeping the screen on on Android and in a browser** in `services-desktop`:
  `power::inhibit(Inhibit::Display, ..)` sets the activity's `FLAG_KEEP_SCREEN_ON` through the
  Android bridge's Java (`MediaControls.keepScreenOn`; the dex is rebuilt) and takes the Screen
  Wake Lock (`navigator.wakeLock`) in a page, where it said `Unsupported` before; the browser
  lets go of the lock while the page is hidden, and a hold asks for it again when the page
  shows. Checked for real: `tools/android-keep-awake-check.sh` reads `FLAG_KEEP_SCREEN_ON`
  back from `dumpsys window` (set while a hold lives, cleared after the last), the web bridge's
  `tools/web-test.sh` runs `keep_awake_web` and media playback's `web_keep_awake` in headless
  Firefox, and on Linux the player's test serves a stand-in `org.freedesktop.ScreenSaver`
  under `dbus-run-session`.
- **Input and pictures for automation and end-to-end tests** in the facade:
  `Window::send_key`, `press_key`, `send_pointer` and `click_at` hand keys and pointer events to
  a live window as the platform would (focus, shortcuts and gestures as for real input, where
  harnesses called `Ui::key` after focusing by hand), and `Window::snapshot_png` (feature
  `devtools`) encodes the window's picture as PNG with straight alpha, reusing the devtools'
  `Snapshot::to_png`. Agents (`remedy-ui-automation`) still never send synthetic input.
- **Portal's control bar and mini player** in `media-playback`. The captions control
  does all Portal's CC does, without its problems: one button (not a toggle beside a 22 px
  arrow) that says what shows in words, "Off", "Auto · English" or "Loading…" (Portal showed
  on/off only, loading only by a pulse), and opens a menu built from the `menus` components:
  Off, Auto when the app listens (`Player::on_caption_pick`, called with a `CaptionPick`), the
  player's tracks, the app's choices not loaded yet (`Player::set_caption_choices(vec![
  CaptionChoice::new(id, label).detail("Hearing impaired · Best match")])`, details in
  words where Portal wrote "HI"), the app's note (`set_caption_note`), and the timing
  (Earlier and Later by 0.5 s, saying where it is now, "now 1 s later", and Reset), steps
  that keep the menu open (new `MenuItem::stays_open` in `widgets-core`). Each entry is a
  menu item with its mark for screen readers; the button names the state, says the timing
  when it is not in sync and whether the menu is open. `Player::set_captions_loading(true)`
  shows "Loading…" until the app shows the track; the glyph pulses as Portal's does, except
  under reduced motion. C turns captions off and back on as last picked. The pick carries to
  the next media (Off stays off, Auto is asked again, a track is picked again by its name)
  with its timing, where Portal forgot the choice and the timing each episode. The button
  shows whenever there is something to pick. `skip_buttons(true)` adds Back and
  Forward 10 seconds (new `Icon::Back10` and `Icon::Forward10` in `widgets-core`),
  `speed_button(false)` hides speed, `two_rows(true)` puts the seek bar above the buttons.
  `player_with(ui, &p, label, PlayerParts::default().end(vec![pip]).title(heading))` adds the
  app's controls after captions (where Portal's PiP is) and its title for the compact layout.
  `Player::set_compact(true)` (or `PlayerOptions::compact`) shows the mini player: the
  picture, play and pause, the time and the title, playback carrying on; it leaves full
  screen and theater mode, and coming back restores theater mode. `player_picture(ui, &p,
  label)` is the picture alone: the newest one shown takes the pictures and the player is
  seen while any of its pictures is, so moving the picture to a picture-in-picture or another
  window never pauses it. The widget uses it, so its root no longer reports visibility itself.

- **HLS on Android.** A `.m3u8` source plays through the crate's own HLS reader, as on
  Windows and Linux (it said `Unsupported` before): MediaCodec decodes the H.264, AAC and MP3
  streams (pictures on the GPU path as for files), and playlists and segments come over
  HTTP/1.1 on a socket of the crate's own (http), through Java's `HttpURLConnection` on the
  app's VM (https), or from disk. A Java fetch that waits too long is ended by a watchdog
  thread that disconnects it (12 s to the first byte, 2.5 s quiet, 60 s in all), as WinHTTP's is
  on Windows. `tests/hls.rs` passes on an Android 14 emulator, and
  `tools/android-hls-check.sh` plays the clip over https inside `app_process` (a real Java VM)
  from a local server with a self-signed certificate: answering at once, 5 s slow to answer,
  and stalling mid-segment (fetched again).
- **Media keys and the system's media controls.** The `player` widget answers the media keys
  (Play/Pause, Play, Pause, Stop, Next, Previous, Rewind and Fast-forward ±10 s, as Portal's
  TV page), and `Player::now_playing(Some(NowPlaying::new(title).subtitle(..).art(..)))`
  (feature `media-session`) shows what plays in Windows' SMTC or a browser's
  `navigator.mediaSession`, kept in step with the player's state, position, timeline and
  remote mode, its buttons coming back to the player (`desktop::media_session` for apps
  without the player). Linux (MPRIS), macOS and Android say `Unsupported` for now.
- **Retry after a failure, and the app's words on the picture**, as on Portal's page.
  `Player::on_retry(f)`: after playback fails here, Play, K, Space or a click on the picture
  (in either key scheme) calls `f` with the last position (film time on a timeline), and the
  failure ends "Press Play to try again."; the app answers with `replace_source` or fails
  again. Without a listener nothing changes. `Player::set_status(Some(StageStatus::new(text)
  .busy(true)))` says what the app is doing over the picture (Portal's resolve phases,
  "Preparing the video.", "The stream did not arrive."), announced politely; while busy a
  spinner shows, the seek bar is disabled, play and seeks wait, and `offer_cancel` shows the
  same Cancel as for skips. `Player::fail(message)` stops fetching and fails with the app's
  words (an alert, retried through `on_retry`). `Player::play()` while the player is still
  opening now plays once it opens (it was dropped, so a just-picked video under reduced
  motion did not start).

- **Scrubbing, as on Portal's page**: pressing or dragging the player's seek bar (mouse, pen
  or touch) holds the thumb, the time and the bar's value at the pointer
  (`Player::scrubbing()`) and seeks once on release; a cancelled pointer goes back. Keys on
  the bar still seek at once. On a TV and on a timeline only the release seeks. A tip over
  the bar shows the time under the pointer (film time on a timeline). A big Play glyph shows
  over a paused picture; unmuting at volume 0 sets half; while casting the volume slider is
  "TV volume" and the mute button "Mute the TV". `Player::volume_known()`: a TV's volume is
  unknown until it reports one, and the slider and volume keys wait; volume sends to a TV
  wait until it rests 300 ms.

- **A hold for a new source** (`Player::hold_for_source(at)`), for a quality or sound
  track switch where the app's server replaces the stream, as on Portal's page: skipping
  without a seek (paused, the bar disabled, ±10 presses adding up from `at`), "Preparing the
  video." over the picture, ended by `replace_source` or `cancel_skip` (which plays the old
  stream on from where it was). Skipping, holding and remote mode now leave the old engine's
  decoder alone, so a stream is not fetched meanwhile (an HLS playlist the server rewrites no
  longer fails the player); it fetches again when they end. **An empty player**
  (`Player::empty`, `Player::empty_platform`) has no source: put it in remote mode at once
  (a cast already running when the page opens) or give it one later with `replace_source` or
  `start_next`.

- **A next made later** (`NextUp::later(title)`), for an app whose server makes the next
  episode's stream only when it is due, as Portal's does. At the end (after the countdown, at
  once with a zero `next_up_delay`), on Play now, Shift+N (N) or `play_next`, the player asks
  `Player::on_next_needed`, pauses, and shows "Starting <title>" over the picture (with the
  app's `next_status` detail, announced as an alert; the bar disabled, play and seeks doing
  nothing); the app answers with `start_next(source, timeline)` (volume, speed, the embedded
  caption choice and delay carry over), `next_failed(message)` (shown; the current media
  stays usable and Play now tries again) or `cancel_next()`. Never asked while remote. A TV
  that starts its next by itself says so with `RemoteStatus::starting_next(title)`: the
  casting stage reads "Starting <title> on <device>", the bar waits and the time is blank,
  and `Player::set_remote_poster` changes the poster without leaving remote mode.
  `Player::offer_cancel(true)` shows a Cancel button under the spinner in the middle of the
  picture while the next starts or a skip runs (as Portal's page does when the add-on says
  its work may be stopped); pressing it calls `Player::on_cancel`, disables it and says
  "Canceling." until the app ends the work.

- **The "Up next" card can show before the end** (`PlayerOptions::next_up_lead`), as on
  Portal's page: the last N seconds of media over a minute long, in film time on a timeline;
  Cancel puts it away for that next, and while casting no card shows (the TV moves on by
  itself). A zero `next_up_delay` plays the next at once at the end, also under reduced
  motion. The card's title and countdown are now announced politely. The default is unchanged.

- **A player timeline for streams that hold part of a film** (`Timeline`,
  `Player::set_timeline`, `PlayerOptions::timeline`, `NextUp::timeline`). With an origin (the
  film's time at the stream's 0) and the film's length, position, duration, seeks, buffered
  ranges, the seek bar, the time and the caption timing of tracks the app adds are in film
  time. A seek outside what the stream holds goes to `Player::on_seek_outside` while the
  widget shows "Skipping to …" with the bar disabled and ±10 presses adding up;
  `Player::replace_source` hands over the new stream (volume, speed, captions and remote mode
  kept) and `Player::cancel_skip` gives up. Without a timeline nothing changes.
- **A casting TV's position moves on between reports** while it plays, for at most 15
  seconds and never past the end, as on Portal's page.

- **The player can answer to streaming-site keys** (`KeyScheme`, `PlayerOptions::keys`).
  `KeyScheme::Arrows` makes Left and Right skip 10 seconds, Up and Down change the volume by
  5% and N play next anywhere in the player (focused sliders and text fields keep their own
  keys); `KeyScheme::Classic`, the default, is unchanged. Double-clicking the picture asks for
  full screen or leaves it (with `Arrows`, a single click plays and pauses after 220 ms), and while casting the full-screen and theater buttons, F, T and
  the double click only leave a mode casting started in. With `Arrows`, a click on a control hands the
  keys back to the player, as in Portal.
- **`menu_press_through`** (`widgets-core`): a menu whose closing press also reaches what is
  under it (a player's picture plays or pauses).
- **Casting**: the stage centers "Playing on" and the device's name over the app's poster under a
  dark scrim; the speed button is hidden while remote; `RemoteStatus::new` with `length`,
  `volume` and `muted`; `Player::remote_status_sender` (a `Send` handle) delivers status news
  from any thread; and a poll from before a seek landed no longer pulls the seek bar back.
- **The seek bar shows what a stream has downloaded on more platforms** (`Decoder::buffered`,
  `Player::buffered`). GStreamer keeps a progressive stream's bytes and is asked in percent
  when it has no answer in time (before, a stream reported nothing). Media Foundation resolves
  a URL to a byte stream, reads through it, and reports the byte ranges it has cached, placed
  in time in proportion to the stream's length. MediaCodec reports how far ahead its extractor
  has cached (an estimate from the bitrate, kept at its furthest since the last seek), and
  streams URLs from threads attached to the app's Java VM, which the NDK's HTTP source needs:
  before, a URL failed to open on Android; with no VM it now says `Unsupported`. In a browser a
  URL is fetched a chunk at a time: the page opens it once its movie box is in, samples still
  downloading wait, and what has arrived is reported. AVFoundation still shows only what is
  decoded ahead. `remedy-ui-media-playback/tools/slow-serve.py` serves a file slowly for
  the browser test, and `tools/web-test.sh` starts it.
- **GPU windows present only what changed, on unmodified wgpu.** On D3D12 and Vulkan a window
  presents through a swapchain of RemedyUI's own on wgpu's device: its images keep what they
  showed, each present copies in only what changed since that image was shown and tells the
  system only those rectangles (DXGI dirty rectangles; `VK_KHR_incremental_present` where the
  device has it, which the Vulkan device is now opened with). Measured on an RTX 3080 against
  the earlier patched wgpu: about 10% less time a frame on Vulkan, level or better on D3D12.
  `present_cost` (run by hand) measures it.
- **The rest of the desktop services** (`remedy-ui-services-desktop`, `remedy_ui::desktop`
  behind the new `desktop` feature):
  - **App badges** in the host contract (`Badge::Count`/`Badge::Dot`, `Host::set_badge`,
    `App::set_badge`): a taskbar overlay icon on Windows (`ITaskbarList3`, drawn by
    `Badge::image`), the Dock tile's label on macOS, the launcher entry API on Linux (read by
    KDE, Dash to Dock, Plank; `AppConfig::app_id` names the desktop entry), and the Badging
    API for installed web apps. The headless host records it (`HeadlessHost::badge`).
  - **URL schemes** (`desktop::scheme`): `register`, `unregister`, `is_registered` and
    `urls`. A URL protocol under `HKCU\Software\Classes` on Windows; `x-scheme-handler/…` in
    the app's desktop entry and `mimeapps.list` on Linux; the bundle's `CFBundleURLTypes` on
    macOS, which `cargo remedy package` now writes from `remedy.toml`'s `[app] url_schemes`.
    `App::on_url` hears the launch's link and, with single instance, every later one through
    the existing hand-off; on macOS the bridge turns the "get URL" Apple event into the same
    hand-off.
  - **Jump lists and recent files** (`desktop::jump`): `set_tasks` (Windows jump list Tasks,
    a Linux desktop entry's actions), `add_recent` and `clear_recent`
    (`SHAddToRecentDocs`/`IApplicationDestinations`, `recently-used.xbel` edited in place,
    macOS's Open Recent list). Dock menu tasks on macOS say `Unsupported`.
  - **Power** (`desktop::power`): `on_battery`, `watch` (sleeping, resumed, power source
    changes) and `inhibit` (keep the system, or the screen too, awake while a guard lives).
    UPower, logind and the session's screen saver on Linux; power requests and callback
    notifications on Windows; IOKit power sources, assertions and system power messages on
    macOS. `App::on_power` brings the events to the UI thread.
  - **Notifications on macOS** through the User Notifications framework (buttons, a picture
    attached as a thumbnail, presses and dismissals back to `App::on_notification`); it needs
    the app in a bundle and says `Unsupported` otherwise.
  - What a platform cannot do says `DesktopError::Unsupported` (new), and a bad scheme
    `DesktopError::Invalid` (new).

- **Reference-shape tests for Arabic, Hebrew and Chinese** (`remedy-ui-text`): fixed
  paragraphs shape to HarfBuzz's glyph ids, clusters, advances and mark offsets, their runs
  come out in the order GNU FriBidi's levels give, and they wrap where ICU4C allows. The
  fixtures are small OFL Noto subsets, written with their references by
  `tools/shape_references.py`. `GlyphRunInfo` gains `advances` and `clusters`, each glyph's
  advance and the byte offset where its cluster starts.
- **A queue for next-up in the media player**: `Player::set_queue`, `enqueue` and `queue`
  hold what plays after the current media, in order. At each end the first counts down and
  plays in the same player, taken off the queue as it starts, until the queue is empty;
  the widget's "Up next" card shows each in turn. `set_next_up` now sets the queue to the
  one given (or empties it), and `next_up` is the queue's first.
- **Captions inside Matroska and WebM files on every native platform**: SubRip
  (`S_TEXT/UTF8`), ASS/SSA (`S_TEXT/ASS`, `S_TEXT/SSA`) and WebVTT (`S_TEXT/WEBVTT`, WebM's
  `D_WEBVTT/SUBTITLES` and `D_WEBVTT/CAPTIONS`) tracks, with their language or name, are read
  from the file by the media crate's own Matroska reader and show in the player's captions
  menu on Windows, macOS, Android and Linux (in place of GStreamer's late copy). Only the
  header elements, the cue index and the subtitle blocks are read; a file without an index
  (written to a pipe) is walked cluster by cluster, skipping the pictures and sound unread.
  Every size is checked against its parent and the file, and a damaged file shows no
  embedded captions rather than wrong ones. Bitmap subtitles (PGS, VobSub, DVB) are not
  read, nor text tracks compressed with zlib or encrypted (header stripping is undone).
- **Captions inside MP4 files on every platform**: MPEG-4 timed-text (`tx3g`) tracks, with
  their language, are read from the file by the media crate's own MP4 reader (only the movie
  box and the cue samples are read) and show in the player's captions menu where the
  platform's decoder hands out none (Media Foundation, AVFoundation, MediaCodec).
- **Media playback runs on Android**: the whole playback suite passes on an Android 14
  emulator (MediaCodec and AAudio).
- **Playback speed in the media player, with the pitch kept**: `Player::set_speed` (0.25× to
  4×, `MIN_SPEED`/`MAX_SPEED`; not-a-number is 1×), `Player::speed`, `step_speed` and `SPEEDS`.
  Sound goes through a pitch-preserving WSOLA time stretcher of the crate's own (no new
  dependency), and pictures follow the same clock, on the sound device or the wall. The
  widget gains a speed button (in all 11 languages) and Shift+> / Shift+<. The speed carries
  over to what plays next. At any speed the sound plays to the very end of the media.
- **A start-up self-test frame and a present watchdog for the GPU** (`remedy_ui_render_gpu::health`).
  Each rung of the backend ladder draws a small reference scene that must match the CPU
  renderer before it is used (`LadderOptions::self_test`, on by default), or the ladder moves
  down. Every present is watched from a thread: one running past `PRESENT_HANG` (3 s) marks
  the backend hung at once and lets the device go, and recovery walks the ladder past it.
  Verdicts (`health::Verdict`) are remembered per adapter, driver and backend in the app's
  data folder (`health::remember_in`, set by the runner), so the test runs once per driver and
  a backend that hung is skipped from the next start.
- **Next-up in the media player**: `Player::set_next_up(Some(NextUp::new(source, title)))`
  sets what plays after the end. At the end a countdown runs (`PlayerOptions::next_up_delay`,
  `NEXT_UP_DELAY` = 5 s; `None` waits for the person; no countdown under reduced motion),
  then the next plays in the same player, every signal about the media starting over and its
  volume, picture and visibility carried over. `Player::countdown`, `cancel_countdown` and
  `play_next`. The `player` widget shows "Up next: …" with the countdown, Play now and Cancel,
  in every shipped language; Shift+N plays it.
- **Buffered ranges on the player's seek bar**: `Player::buffered()` (a signal of time ranges)
  holds what can play without waiting: all of a local file; for a stream, what has arrived
  where the decoder says (`Decoder::buffered`, a new provided method; GStreamer answers its
  buffering query) joined with what the player has decoded ahead. The seek bar draws them
  in a light tone behind the played part. `merge_ranges` sorts and joins ranges.
- **Mipmaps for images drawn scaled down**, on both renderers alike: an image drawn smoothly
  at half its size or less is drawn from a halved copy (built once in the shared painter,
  freed with the image), so it no longer shimmers, and only the level drawn is uploaded to
  the GPU (a 1024² image shown at 64² uploads 16 KB, not 4 MB). Pixel art (nearest sampling)
  is untouched.
- **Colour-matrix filters** on any element and in display lists, on both renderers with
  matching results: `Style::saturate`, `hue_rotate`, `grayscale`, `sepia`, `brightness`,
  `contrast`, `invert` (the CSS filter functions) and `Style::color_matrix` (SVG
  `feColorMatrix`), chained into one matrix (`StylePatch::color_matrix`,
  `ResolvedStyle::color_matrix`); `Filter::ColorMatrix` and `ColorMatrix` (in
  `remedy_ui_core`, re-exported by `remedy_ui_paint`: presets, `then`, `limited`, `apply`,
  `apply_premultiplied`). vello runs no colour matrices, so RemedyUI draws the group into a
  picture of its own and runs the matrix itself (unpremultiply, matrix, clamp, premultiply),
  on the CPU and in a GPU pass; it works inside display lists and on layers the compositor
  filters. Bounded for untrusted themes: finite entries within `MAX_COLOR_MATRIX_ENTRY`, at
  most `MAX_FILTER_MATRIX_DEPTH` nested and `MAX_FILTER_MATRIX_GROUPS` per raster.
  `DisplayList::bounds_of` gives the bounds of a run of items.
- **Blur, shadows and frosted glass on any element**, on both renderers with matching results
  (within two levels): `Style::shadow` (CSS `box-shadow`, a `Shadow` value),
  `Style::blur` (`filter: blur()`), `Style::drop_shadow` (`filter: drop-shadow()`, a shadow
  of the shape of everything the element draws, text and icons included) and
  `Style::backdrop_blur` (`backdrop-filter: blur()`: the element gets a compositor layer and
  the compositor blurs every layer below it, back to its backdrop root), with the same on
  `StylePatch` and `ResolvedStyle`. Display lists gain `Item::PushFilter(Filter)` /
  `Item::PopFilter` (`Filter::Blur`, `Filter::DropShadow`; `DisplayList::push_filter`,
  `pop_filter`, `has_filters`, `Filter::outset`) and `Item::Backdrop`; layers gain
  `Layer::backdrop` (`Backdrop`, `Layer::with_backdrop`) and the composite
  `CompositeOp::Backdrop` (`CompositeOp::backdrop_root`, `backdrop_reach`). Drawn through
  vello's filter layers, so tiles stay seamless and both renderers blur alike.
  `examples/glass_and_shadows` shows all four over a Prism aurora. A filter takes children
  that are compositor layers of their own: the element becomes a layer with
  `Layer::filters`, composited through `CompositeOp::PushFilter` / `PopFilter`. Glass fades
  with its element's opacity, and scroll containers can be glass.
- **Eight more built-in Prism effects** (38 in all): `shimmer` (the sweep of light over a
  loading placeholder), `mesh_gradient` (drifting pools of colour for backdrops), `confetti`,
  `smoke`, `galaxy`, `code_rain`, `frost` (ice creeping in from the edges, by `amount`) and
  `retro_grid`. All fit `Limits::untrusted`, have golden images, and match on the GPU kernel.
- **Filters are bounded**: a filter's blur is at most `MAX_FILTER_STD_DEV` (64) device pixels
  and a drop shadow's offset at most `MAX_FILTER_OFFSET` (2048) on each axis
  (`Filter::limited`), wherever a renderer or compositor draws one, so a theme asking for a
  blur of a million pixels, infinity or NaN gets the bound and no huge buffers (tested on
  both renderers).
- **Effect tools for people and agents**: `cargo remedy rfx check <file or name>
  [--untrusted]` (each problem under its line, or the parameters and each pass's cost),
  `rfx render … --png <file>` (a frame on the CPU reference renderer, with sizes, time and
  parameter values) and `rfx library`; the same as the MCP tools `rfx_check`, `rfx_render`
  (a PNG image) and `rfx_library`, from the new `effects` group of `remedy-tools`.
- **Prism effects polished and cheaper**: effects that do not move (`Effect::animated` false:
  paper, frost) are drawn once per size and parameter change and ask for no frames.
  `starfield` draws its soft dust band at a quarter of the resolution (cost 268 to 184, at
  most a level of difference) and `heat_shimmer` its air at half (148 to 62, the same
  picture). `plasma` has a softer palette whose colours drift over time, and `fog` is
  denser by default with a haze above its banks. `lightning` forks, stays unbroken where
  it runs sideways (its core is measured across the bolt, not only sideways), tapers at
  its tip instead of stopping at a line, strikes twice down the same channel and lights
  the sky more; its still frame (reduced motion, paused) shows a strike (cost 33 to 127,
  for the picture). `film_grain` is soft clumps of any size, mostly faint, instead of
  square cells (26 to 56). The golden-image test draws an effect that shows nothing at its
  time at its still moment instead, and fails on a blank picture: lightning's had been
  blank. The compiler turns `pow(x, n)` for a whole `n` from 2 to 8 into
  multiplies (the same pixels, cheaper than `pow`), and a multiply used only by an add
  into one `fma`, in every effect, people's own included: `ripples` costs 114 instead of
  145, and most effects 3 to 10% less with identical pixels. `crystal` marches in the
  gem's own turning space (390 to 241) and has a smooth outline (a grazing ray covers
  part of its pixel; 301 with it), `orb` too, with four-sample normals (2279 to
  2058), and `volumetric_clouds` probes its light with one octave less (548 to 482); each
  within a few levels of before, on a handful of edge pixels.
- **Prism's CPU interpreter is faster with the same pixels** (every golden image unchanged,
  noise checked bit for bit against the old code): instructions read their operands in
  place instead of copying them (registers are boxed and swapped), each common operation
  has its own loop, gradient noise shares the hashes of a cell's corners (14 per 3D noise
  instead of 24) and picks its gradient from a table instead of a branch the random hash
  mispredicted. Twelve of the costliest effects at 640x400 on four cores: 1064 to about
  610 ms in all; `orb` 520 to 188, `crystal` 89 to about 45. The GPU kernel picks noise
  gradients with selects instead of a `switch`, so neighbouring pixels no longer diverge
  (not measured on a hardware GPU).
- **Prism in `.rui` markup**: `remedy_ui::effects::register_markup` adds the `effect`
  element (`effect(name: "fire", param: "intensity", value: $heat) { … }`): a built-in
  effect, one parameter driven by a number signal, children drawn over it
  (`markup_element`, `MARKUP_PARAMS` for build-time markup).
- **Prism diagnostics**: "did you mean" for a misspelled function, name, parameter, pass or
  type; an effect over its cost budget is reported at its costliest pass with the passes that
  cost most; particle and pass-count limits are reported where they are written.
  `CompileError::report(source)` and `Diagnostic::render(source)` draw each problem under its
  line with a caret.
- **Per-effect GPU timing and a GPU quality ladder** for Prism: `GpuEffect::take_gpu_time`
  samples an effect's GPU time every eighth frame with timestamp queries, without waiting
  for the GPU; the effect element steps down to half, then a quarter resolution (at most
  30 fps) when it stays over `EffectView::set_gpu_budget` (2 ms by default), and climbs back.
  `EffectStats::gpu_time`.
- `GpuCompositor::read_surface` (a picture of what a GPU window shows),
  `GpuCompositor::last_composited`, and `FramePlan::redraw_rects` (the rectangles a renderer
  that keeps its frame draws again; both compositors use it).

- **Prism, the effects system** (`remedy-ui-effects`, `remedy-ui-media-effects`, the
  `effects` feature, `remedy_ui::effects`; in `full`): `.rfx`, a language for animated,
  procedural pictures (types float to mat4, functions, `if`, constant loops; passes at their
  own resolution, blur, bloom, stateless particles with trails, feedback through `last`; noise,
  colour, 2D and 3D signed-distance shapes, cameras and GGX lighting), compiled with
  optimizations (inlining, unrolling, folding, shared work, per-frame hoisting) against
  `Limits` (`Limits::untrusted` for themes and scripts) into programs for one fixed GPU kernel
  (`remedy_ui_render_gpu::effects::GpuEffect`, GL 3.3 and WebGL2 included) and the CPU
  interpreter (`CpuEffect`), which match. `effect_view(ui, Instance)` draws one in an element
  under the ambient policy, with a still frame and a CPU quality ladder; parameters by name
  (`EffectView::set`). 30 built-in effects (`library`, `LIBRARY`): fire, embers, glow,
  light shafts, water, caustics, ripples, rain, snow, fog, clouds, volumetric clouds, aurora,
  starfield, nebula, fireflies, sparkles, bokeh, lightning, trails, bubbles, heat shimmer,
  paper, film grain, CRT overlay and screen, plasma, lava lamp, and raymarched 3D (orb,
  crystal). Reference: `docs/effects/rfx.md`; design: `docs/design/16-effects.md` and
  `17-3d.md`; `examples/effects_gallery`.
- **Ambient motion budgets** held by `tests/ambient_budget.rs` on both renderers (see
  `docs/design/09-performance-and-resources.md`). `CpuCompositor::present_frame` borrows the
  kept frame; `CpuCompositor::last_composited`.
- **The top 10 languages** (see `docs/I18N.md`): English, Spanish, Simplified Chinese,
  Hindi, Arabic, French, Brazilian Portuguese, Russian, Japanese, German and Bengali.
  Every word the engine shows itself (widget labels, menus, keyboard hints, the media player,
  the board editor, the customization pages, the agent prompts, the update strip, what screen
  readers hear) is a Fluent message in its crate's `i18n/<locale>.ftl`, compiled in as that
  crate's `STRINGS` (`remedy_ui_view::strings`: `Strings`, `engine_strings!`, `add_messages`,
  `check`), following the window's locale live, with CLDR plurals, the locale's digits and
  isolated placeholders. Word structs gain `localized(locale)` (`BoardLabels`,
  `StepperLabels`, `spell::MenuLabels`, the update banner's `BannerText`). Windows lay out
  right to left from their locale (`App::set_direction`, `Ui::set_direction`,
  `WindowDirection`, `Ui::is_rtl`); directional icons and value controls mirror; RTL
  sideways scrolling (`Layer::scroll_origin`); `LayoutStyle::inset_physical`. Text fields
  move the caret visually through mixed-direction text. Bundled Noto faces for Arabic,
  Devanagari and Bengali, and on-demand Noto Sans SC and JP (`remedy-ui-fonts-cjk`), behind
  the default `script-fonts` feature. All translations are machine-assisted and await
  review by native speakers.

- **Ambient motion with a budget**: animated backgrounds and decoration that cost nothing
  while unseen. `AmbientPolicy` (`App::set_ambient_policy`, `Ui::set_ambient_policy`):
  `AmbientMode::{Full, Light, Off}`, frames a second focused (30) and in the background (10, or
  0 to pause), on battery or power saver (`power_saving_fps`, 15), and following reduced
  motion; minimized, hidden and covered windows never draw it. Painters animate with
  `PaintCx::ambient()` / `ambient_at(fps)` (`Option<AmbientTick { t, dt, light }>`: `None`
  draws a still frame) and get a compositor layer of their own; `PaintCx::request_frame_in`
  asks for one repaint later. `El::ambient_loop(AmbientLoop::{spin, sway, drift, pulse,
  breathe})` runs transform and opacity loops on the compositor with no UI-thread work
  (`Layer::ambient`, `Compositor::set_ambient`, `FramePlan::wake_in`). Painters asking for
  frames (`ambient`, `request_frame_in`, and now `request_next_frame`) keep no frames coming
  while their element is scrolled out, clipped away or its window hidden, and resume when it
  shows (`El::is_seen`, `El::resume_when`, `Ui::tick_interval(TickKind)`). `viz` tickers and
  the stage widgets follow the policy (`Ticker::ambient`, live tickers slow in the background
  and stop when hidden). `SystemSettings::power_saving` from Windows (`GetSystemPowerStatus`:
  on battery or battery saver) and Linux (a discharging battery or the low-power platform
  profile), read again when a window gains focus.
- **`Blend::Plus`**: additive blending, so glows, light pools and beams add up like light.
- **`remedy_ui::stage::fx`**: shader-free effects for scenes: `flicker`, `twinkle`,
  `breathe` and `noise1` curves, `value_noise`, `perlin` and `fbm`, `noise_image` (a seeded,
  tileable noise texture through a color ramp, made once and shared), `blur` (a cached
  three-pass box blur), `glow` and `light_cone` (additive gradients), `scroll_tiled` and
  `shimmer` (a cached texture moving with time).

- **Stage widgets for whole alternative presentations** (`remedy-ui-widgets-stage`, the
  `stage` feature, `remedy_ui::stage`; in `full`): the parts for scene-based, skeuomorphic and
  playful layouts, all themed from tokens, accessible, keyboard operable, still under reduced
  motion, animating only while seen and idle when nothing moves. `scene`: a stage of a design
  size scaled to its box (`Fit::Contain`, `Cover`, `Stretch`, whole-number pixel scaling) with
  art `Layer`s (behind or in front, optional pointer parallax) and children at named `Anchor`s
  that stay on their spot as the window resizes (`StageTransform` maps design to box).
  `sprite` and `pixel_art`: pixel art as code (`PixelArt`: a palette of `Ink`s and rows of
  characters) or `SpriteSheet` cells, crisp at whole-number scales, with named states chosen
  by a signal and frame animation. `orbit`: children on nested rings, optionally turning.
  `path_layout`: children along a `Route` (polyline or smooth curve), spread or at fractions,
  with the route drawn solid or dashed (`LineStyle`). `knob` and `tuner` (a line or arc dial
  of `Station`s whose needle snaps and glides), `fader` (ticks, a level meter beside it),
  `timeline` (items by time, virtualized, scrubbed by drag, wheel and keys, flowing as items
  arrive, asking for history near the oldest), `big_button` and `big_toggle` (round arcade,
  plate and switch-with-lamp looks; hold-to-talk through `held`), and `display_text` (LED or
  split-flap boards that stay real text, with a marquee). The theme sheet styles `knob`,
  `tuner`, `fader`, `big_button` (variants `accent`, `danger`, `ok`, `warn`), `display_text`
  (`led`, `split_flap`) and `display_flap`.
- **Missing system components, said plainly.** `MediaError::Missing(MissingDependency)`: which
  component is missing (GStreamer, a plugin set for a named element, a VA-API or NVIDIA driver,
  the screen-sharing portal, libpulse, an X server extension, Media Foundation on Windows N),
  what it is needed for, whether it is not installed or installed but unable to run here, and the
  package to install on Debian/Ubuntu, Fedora and Arch (`playback::deps` holds the catalog and
  `any_of` for "none of these can be used"). GStreamer loading, every element, the sound output
  and microphone pipelines, a file's missing codec plugin and Media Foundation's start use it.
- **Stream decoders proven before they are chosen.** Each candidate decodes a built-in test
  picture (a 64x64 I_PCM H.264 access unit) on this machine once per process; one that is
  installed but cannot run (NVIDIA's with no usable NVIDIA GPU, as in WSL) is never chosen.
  `MediaHost::video_stream_decoder_choice` names the decoder that would be used.
- **Encoding a video stream, reading a granted screen, and recording what other programs
  play.** `MediaHost::video_stream_encoder(&VideoEncoderOptions)` gives a `VideoStreamEncoder`
  (BGRx pictures in, `EncodedPiece`s out: H.264 Annex B access units with SPS and PPS before
  every key frame, `request_keyframe`, `set_kbps`). On Linux it uses the system's GStreamer at
  run time: the graphics card first (`vah264enc`, `vah264lpenc`, `vaapih264enc`, `nvh264enc`
  first when `vendor` is NVIDIA's), then `x264enc` or `openh264enc`, each used only after it
  encoded trial pictures here (an installed `nvh264enc` without a usable NVIDIA card is passed
  over). Without an encoder it says which packages to install.
  `MediaHost::video_stream_encoder_choice(codec, hardware)` names the one it would use. `MediaHost::screen_stream(ScreenSource::PipeWire { fd, node })`
  reads the stream a desktop's ScreenCast portal granted (`pipewiresrc`) as `ScreenFrame`s,
  and its end is `NotFound`. `MediaHost::output_capture(format)` records what every *other*
  program plays, mixed. On Linux it records each program's stream through PulseAudio monitor
  streams (`libpulse.so.0` at run time; PipeWire's pulse server answers too), so this app's
  own sound is never in it.
- **Video streams decoded as they arrive.** `MediaHost::video_stream_decoder(VideoCodec::H264,
  hardware)` gives a `VideoStreamDecoder`: Annex B access units go in with `push`, NV12
  pictures come out with `next` (the newest ready, never waiting), for a shared screen or a
  call rather than a file. On Linux it is the system's GStreamer, loaded at run time:
  `h264parse` and the graphics card's decoder first (VA, VA-API, NVIDIA, V4L2), FFmpeg's or
  Cisco's on the processor after, with a reason naming what to install when none starts; a
  decoder that starts but fails once the stream flows (a driver that cannot run here) is named in
  the error and not chosen again in that process.
  Other hosts say `Unsupported` until they have one.
- **Windows that cannot hide minimize instead.** Wayland has no request that takes an open
  window off the screen, so `Window::set_visible(false)` and closing a `hide_on_close` window
  used to leave it up while the app was told it was hidden. `Window::can_hide` (and
  `Host::can_hide`, default true; false on Wayland) says so, and there both, and
  `WindowSpec::hidden`, now minimize the window, which stays reported as shown.
  `HeadlessHost::set_hides_windows` tests it.
- **The toolbar role.** `Role::Toolbar` reaches every platform as a toolbar (AccessKit's
  `Toolbar`: UI Automation, AT-SPI and Android's, ARIA `role="toolbar"` on the web), reads
  "toolbar" in the reader view and counts as a container in the devtools audit;
  `nav::toolbar` uses it instead of a plain group.
- **Focus within.** `El::on_focus_within(f)` is told when keyboard focus comes onto an element
  or anything inside it and when it leaves all of it (moving between its children says
  nothing), and `States::FOCUS_WITHIN` styles it meanwhile, so actions that show on hover can
  show for the keyboard too.
- **Frame pacing and the graphics adapter.** `AppConfig::vsync(false)` presents GPU frames as
  soon as they are drawn (`GpuCompositor::create_surface_paced` underneath);
  `App::set_frame_rate_limit(Some(fps))` draws each window at most that often, holding a
  frame asked for sooner until its turn; `App::graphics_adapter()` names the adapter, its
  maker, driver and graphics API (`GpuCompositor::adapter`) for an app's settings or a bug
  report.
- **Sharp and subpixel text.** `Ui::set_text_rendering(TextRendering::new(sharp, antialias))`:
  letters fitted to the pixel grid or as designed, and grayscale, subpixel (LCD) or as the
  system draws its own text (`TextAntialias::System`, following ClearType on Windows through
  the new `SystemSettings::subpixel_text`). The default is unchanged (sharp, grayscale).
- **An app's own "reduce motion".** `App::set_reduce_motion(true)` holds animations and
  spinners still in every window even where the system does not ask for it; off follows the
  system again.
- **Tray state and closing to the taskbar.** `App::tray_shown()` (tracked) says whether the
  tray icon asked for is up (false where the desktop has no tray); `WindowSpec::minimize_on_close`
  and `Window::set_minimize_on_close` make closing minimize the window, for an app that keeps
  running when it cannot hide in a tray.
- **File dialogs that say when there is none.** `App::try_file_dialog` answers
  `Err(Unsupported)` where the system has no dialog to show (a Linux desktop without the
  file-chooser portal), and `Ok` with nothing chosen when cancelled, so an app can offer a
  field for a path instead; `App::file_dialog` is unchanged.
- **Text cut short with "…".** `El::truncate()`: one line, cut where it would run past its box,
  shrinking down to the "…" alone; readers, copying and searching keep every word (a long
  error in a band, a long name in a narrow list).
- **Options shown but not choosable in a segmented control.** `widgets::segmented_unavailable(ui,
  label, options, selected, unavailable)`: each `(index, why)` is drawn disabled, cannot be
  pressed or reached with the arrow keys, and says `why` on hover and to assistive technology
  (sharing "to one person" when nobody else is here). `segmented` is this with none.
- **The Linux tray on X11 desktops without a StatusNotifier host.** When no
  `org.kde.StatusNotifierWatcher` answers (older panels, window managers with a plain system
  tray such as stalonetray or trayer), the tray icon docks in the freedesktop XEmbed system tray
  instead: it shows the icon at the tray's size (`TrayIcon::image_at`), names itself with the
  tooltip, reports `TrayEvent::Activate` on a primary click, and opens the menu as a pop-up on a
  secondary click (pointer, or Up, Down, Enter and Escape), reporting `TrayEvent::Menu`.
  `Unsupported` now comes only when neither tray exists, with both reasons.
- **Icons drawn at several sizes, and pixel art.** `WindowIcon::with_size` adds the artwork as
  drawn at another size, `WindowIcon::pixel_art` scales by the nearest pixel instead of
  blending, and `WindowIcon::scaled` now takes a drawn size as it is (else the smallest larger
  drawing); `drawn` and `drawn_sizes` list them. Windows and Wayland window icons use the
  drawings at the sizes they show. `TrayIcon::artwork` (`with_artwork`, `image_at`, `images`):
  the Windows tray takes the size the notification area shows at the system DPI from it, and
  a Linux status notifier offers every drawn size.
- **`remedy.toml` icon sets for every package.** `[app] icon_sizes` (PNG files of the icon
  drawn at other sizes, used as they are) and `[app] icon_scaling = "pixel"` (other sizes by
  the nearest pixel) shape the Windows `.ico` (the standard sizes plus every drawn one), the
  Linux `hicolor` icons (256 px and every drawn hicolor size), the macOS `.icns` (256, 512 and
  1024 px) and the web page's icons. Android packages get the launcher icon at every density
  and an adaptive icon: `[android] icon_foreground` (default: the icon) filling the 72 dp the
  launcher shows, on `[android] icon_background` (a color, or two for a top-to-bottom
  gradient; default: the icon's corner color when opaque, else white), with
  `[android] icon_monochrome` (a PNG whose alpha is the shape) as Android 13's themed layer,
  which the status bar also shows for the app's notifications.
- `Window::snapshot`: a picture of a window's next frame (all of it, in device pixels, drawn
  by the CPU renderer whichever renderer the window uses), for picture tours, bug reports
  and tests on a real desktop.
- `Window::on_focus` and `Window::is_focused`: told when a window gains or loses keyboard
  focus (another window in front, minimized, hidden), so an app alerts only when the person is
  elsewhere.
- `WindowSpec::activate(false)` and `WindowSpec::in_taskbar(false)` (`WindowOptions::activate`,
  `WindowOptions::taskbar`): a window that appears behind the active one without taking focus,
  and one without a taskbar button (Windows; X11 window managers through `_NET_WM_USER_TIME`
  and `_NET_WM_STATE`), for windows automated tests open on a real desktop.
  `PlatformHooks::window_options` hands bridges the options winit cannot carry.
- `El::image_sampled` and `widgets::pixel_image`: pictures scaled by repeating pixels
  (`Sampling::Nearest`), so pixel art (an app's icon on its About page) stays sharp.
- **`remedy-ui-build`**, build-script help for apps: `WindowsResources` writes the Windows
  executable's icon (an `.ico`, every size as stored) and version information (file and
  product version, description, product, company, copyright, file names) as a `.res` file
  itself and links it into the package's executables (MSVC), with no resource compiler.
  `cargo remedy package` writes its resources the same way, so it no longer needs `llvm-rc`
  or the Windows SDK's `rc`, and sets `REMEDY_PACKAGE_RESOURCES` while building so an app's
  own build script stands aside.

- **Desktop menus and the menu bar.** `widgets::menu_bar(ui, menus)`: a row of menus with
  access keys; a click opens a menu (press, drag and release chooses), hovering another title
  while one is open switches to it, F10 or a lone Alt press reaches the bar (and leaves it),
  Alt and a menu's letter opens it from anywhere in the window (a text field included), Left
  and Right move between menus open or closed, Down, Enter or Space opens one (Up at its last
  entry), Escape closes it and then gives focus back where it was; right-to-left layouts
  mirror; titles that do not fit a narrow window move into a "…" menu. `MenuItem` grows
  separators (`separator`), submenus (`submenu`, `submenu_with`: open beside their entry,
  flipping at the window's edges; Right/Enter opens, Left/Escape closes one level, resting the
  pointer opens after a moment), check and radio entries (`check`, `check_signal`, `radio`,
  `radio_signal`, taking a value or a signal), disabled entries (`disabled`: focusable but not
  chosen, as WAI-ARIA asks), shortcut text (`shortcut`, or `chord`/`chords` written in the
  person's language), an icon (`icon`), an access key (`mnemonic`: underlined while Alt is
  held and in menus opened from the keyboard; pressing it in an open menu chooses), dynamic
  labels (`label`), and type-ahead by first letter. `menu`, `context_menu` and `menu_button`
  use the same menus (and `menu_button` reports whether its menu is open, and opens with Down
  or Up). Menus taller than the window scroll inside it. `nav::Commands::menu_item(id)` makes
  an entry from a command (label, shortcut, enabled, icon, destructive, action), so menus,
  toolbars and the command palette share one source. `fit_beside`, `place_beside` and
  `fit_near_dir` place submenus and right-to-left overlays; `Icon::Dot` marks radio entries.
  Themes style `menu_bar`, `menu_bar_item`, `menu_separator` and `menu_shortcut`. The recipe
  `examples/menu_bar.rs` and the guide's Qt page show them.
- **Menu roles in accessibility**: `Role::MenuBar`, `Role::MenuItemCheckBox`,
  `Role::MenuItemRadio` and `Role::Separator`, mapped to AccessKit (UI Automation, AT-SPI,
  NSAccessibility and Android's `AccessibilityNodeInfo`, where check and radio items are
  checkable) and to ARIA in the web bridge (`menubar`, `menuitemcheckbox`, `menuitemradio`,
  `separator`, with `aria-checked`, and now `aria-posinset`/`aria-setsize`).
  `Semantics::shortcut` and `Semantics::access_key` reach AccessKit's keyboard shortcut and
  access key, and `ReaderNode` carries both (a reader announces the shortcut). The devtools
  audit treats menus and menu bars as composite widgets and reports
  `ProblemKind::NoCheckedState` (a check or radio menu item that does not say whether it is
  checked) and `ProblemKind::OrphanMenuItem` (a menu item outside any menu).
- **Window menu keys**: `Ui::add_menu_keys` hears `MenuKey::AltTap`, `MenuKey::F10` and
  `MenuKey::Mnemonic` before the focused element (only these keys), `Ui::alt_held` (tracked)
  says Alt is down, and `Ui::set_window_focused` (the runner calls it) lets Alt go when the
  window loses the keyboard. `El::keeps_focus_on_press` leaves focus where it is when an
  element is pressed (a menu bar's titles, so the field keeps it).
- `Chord::label_in(platform, locale)`: a shortcut written with the key names of the person's
  language ("Strg+Umschalt+Z", "Ctrl+Maj+Z", "Entf"; German, French, Spanish, Italian, Dutch
  and Portuguese, English otherwise; Apple's symbols on Apple platforms).
- **System notifications**: `App::notify(Notification::new(id, title, body).action(..), ..)`,
  `App::withdraw_notification` and `App::on_notification` (pressed, a button, dismissed).
  Windows toasts (with buttons, replacing by id, the app's name and icon; the engine
  registers an identity for unpackaged apps under the current user and leaves the process's
  own id alone), the Linux desktop's notification server over D-Bus (buttons, icon, quiet),
  and the browser's Notification API. `Permission::Notifications` asks the browser (and
  Android 13). The host contract gains `Host::show_notification`,
  `Host::withdraw_notification` and `HostEvent::Notification`; the headless host records
  them. The tray recipe's reminder now notifies.
- **Android notifications**: `App::notify` reaches Android's shade: an alerting and a quiet
  channel, the app's icon made monochrome for the status bar, the notification's picture, up
  to three buttons, replacing and withdrawing by id. Pressing one brings the app forward (or
  starts it) and reports `Activated`; buttons and swiping it away report `Action` and
  `Dismissed`. `Permission::Notifications` shows the prompt on Android 13 and later and,
  before 13 (granted at install), answers at once whether the app's notifications are on.
- **Foreground work**: `App::start_foreground(ForegroundTask::new(ForegroundKind::Microphone,
  notification).kind(ForegroundKind::MediaPlayback), ..)`, `App::stop_foreground` and
  `App::on_foreground_stopped`: ongoing work that keeps the app running while it is out of
  sight (a voice call with the screen off, music, a recording, an upload), shown by its
  notification, whose buttons report through `App::on_notification`. On Android it is a
  foreground service of the matching types, with what Android would refuse (a missing
  declaration or permission, microphone work started from the background) reported at once
  as `Unsupported`; on the desktop and the web, where apps keep running anyway, it answers
  `Ok` and does nothing. The host contract gains `Host::start_foreground`,
  `Host::stop_foreground`, `HostEvent::ForegroundStopped`, `ForegroundTask` and
  `ForegroundKind`; `PlatformHooks` gains the same two calls; the headless host records the
  task (`foreground`, `refuse_foreground`).
- **`[android] foreground`** in `remedy.toml` (`microphone`, `media-playback`, `camera`,
  `data-sync`): `cargo remedy package` declares the foreground service with those types and
  adds their permissions (`FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_<TYPE>`, the microphone
  or camera, `POST_NOTIFICATIONS`).
- **The microphone** (`playback::Microphone`): the default sound input on a worker of its
  own, handed to the app as it arrives in the format asked for (converted from the device's
  own where the platform does not), with its state, error and level as signals. WASAPI on
  Windows, GStreamer on Linux, AAudio on Android (unprocessed input, low latency) and
  `getUserMedia` with an audio worklet in a browser page. `MediaHost::audio_input` and the
  `AudioInput` trait let an app bring its own input; `MediaError::Denied` says when the
  person or the system's privacy settings refused a device.
- **Run-time permissions**: `App::request_permission(Permission::Microphone, ..)`. Android
  shows its system prompt (a new fragment in the bridge's Java half) and answers later;
  other platforms answer at once. The host contract gains `Host::request_permission`,
  `PermissionStarted` and `HostEvent::Permission` (before 1.0, so the exhaustive host enums
  may still grow); the headless host scripts answers (`answer_permission`,
  `permissions_answer_later`).
- **`[android] permissions`** in `remedy.toml`: the manifest permissions `cargo remedy
  package` declares (`microphone`, `internet`, `notifications`, `camera`, or full names).
- `Window::on_visibility`: told when a window is shown or hidden, including when the user's
  close button hides a `hide_on_close` window, so a tray app can remember whether its window
  was open (Hearth restores it after an update).
- `El::inline_images` and `InlineImage`: pictures inside a paragraph (custom emoji in a chat
  message), each drawn in the box of the `RichText` inline object with its id; screen
  readers, `text_content` and copied selections read each picture's `alt` in its place.
  `InlineObject` is re-exported from `remedy_ui::view`.
- `Window::set_zoom` and `Window::zoom`: a window's content drawn larger or smaller (an
  app's interface size, Ctrl and +), with layout in the smaller logical size, and pointer,
  wheel, pinch and drag positions and the input method's caret mapped through it.
- `Window::set_hide_on_close`: whether closing hides the window, changed while it is open
  (a "keep running in the tray" setting).

- **Secret copies out of ordinary fields**: `TextField::copy_as_secret(keep)` sends Ctrl+C
  and Ctrl+X (Command on Apple platforms) in a single-line, multi-line, rich or code field
  through the secure clipboard path `App::copy_secret` uses: kept out of clipboard history and
  cloud sync, and cleared after `keep` if the clipboard still holds it. Rich fields copy plain
  text only. `TextField::on_copy` is told after each copy or cut; `TextField::secret_copy`
  reads the setting. The view asks its runner with the new `Request::CopySecret` (before
  1.0, so the exhaustive `Request` may still grow; a custom runner handles it as
  `App::copy_secret` does).
- **Per-window icons**: `WindowSpec::icon` and `Window::set_icon` with the new
  `host::WindowIcon` (square premultiplied RGBA8, `scaled` to the sizes a platform shows). The
  host contract gains `WindowOptions::icon` and `Host::set_window_icon` (default
  `Unsupported`), and `PlatformHooks::set_window_icon` in the winit core. Windows sets the
  small and big icons (`WM_SETICON`) at the window's DPI; X11 sets `_NET_WM_ICON`; Wayland
  uses `xdg-toplevel-icon-v1` where the compositor has it, with a buffer for every size it
  prefers; a browser page shows it as the page's icon (or in a floating window's title bar);
  macOS and Android say `Unsupported` (their windows show the app's icon); the headless host
  keeps it in the window's options for tests.
- **WOFF and WOFF2 fonts**: `FontCollection::add_static` / `add_owned` take WOFF and WOFF2
  files as well as TrueType and OpenType (pure-Rust `wuff`; a WOFF file whose tables claim
  more than zlib could unpack is refused before unpacking). `text::FontFile` reads and checks a
  font file once and shares it, and `AppConfig::font` adds it to every window, so apps ship
  their brand's typeface. Files of one family, weight and style that each cover other scripts
  (a web font's subsets, which CSS joins with `unicode-range`) join as one family: each
  character comes from whichever part covers it.
- **Icons**: `Icon::Home` (a house) and `Icon::Image` (a framed picture), drawn on the same
  24-unit stroke grid as the others; `Icon::accessible_name` gives every icon an English
  name for where the icon alone stands for something.
- `NumberOptions::min_fraction_digits` (and the matching field) for `number_input`: fraction digits
  always shown, with trailing zeros beyond them dropped, so a field with `precision(10)` shows `5`
  and `12.25` instead of `5.0000000000`. Applies to the bound value, to the text after Enter or
  blur, and to the spoken value. Unset keeps today's behaviour (all `precision` digits shown).
  `NumberOptions::format` writes a value the same way.
- **App data on Android and in browser pages.** `App::persist_store()` now works on every
  platform, so persisted signals, window geometry and an app's own documents survive restarts
  and reloads there too (found porting Craft Calc, which ships on Windows, Android and as a
  PWA):
  - Android: `remedy_ui::android::data_dir()` (the bridge's `data_dir`) is the app's private
    internal storage (`AndroidApp::internal_data_path()`, `/data/user/<user>/<package>/files`),
    and `App::data_dir()` returns it (without an app-name segment: the folder is the app's
    own). Before, the desktop's environment variables were read there and data went nowhere.
  - Browser pages: `PersistStore::local_storage(app)` and
    `PersistStore::local_storage_with_debounce(app, debounce)` (wasm32 only) keep the same
    versioned JSON envelope and migrations in the origin's `localStorage` under
    `remedy:<app>:<key>`, debounced and written between frames; a write the browser refuses
    (a full storage) is reported by `take_errors` and `flush`. `App::persist_store()` returns
    one in a page; `App::data_dir()` stays `None` there.
  - `PersistStore::remove(key)` forgets a saved value, `PersistStore::storage_key_of(key)`
    names a page store's `localStorage` entry, and `PersistStore::load` now sees a value saved
    but not yet written. `save`/`load`/`remove` are documented as the cross-platform way to
    keep an app's documents (guide: signals, web, packaging).
  - `GeometryStore::persisted_in(&PersistStore)` and `GEOMETRY_KEY`: window geometry kept in a
    persist store, which is how a page keeps its floating windows' places across reloads.
- **`remedy-ui-services-webstorage`, the facade's `webstorage` feature
  (`remedy_ui::webstorage`).** Apps moving off a web view keep what their users saved: a
  pure-Rust, read-only reader of Chromium's localStorage store (the LevelDB folder WebView2,
  the Android System WebView and Chrome keep): Snappy-compressed tables, write-ahead logs
  whose records span blocks, the manifest's live files, the newest write of each key with
  deletions, and Chromium's Latin-1 and UTF-16 item encoding. `ChromiumStorage::read_dir`
  (or `from_files`), `origins`, `local_storage`, `first_of`; on `wasm32`,
  `browser_local_storage` reads the page's own store into the same `LocalStorage`. Items
  that are not valid Unicode (lone UTF-16 surrogates) are kept as UTF-16, not changed. Path
  helpers for Tauri on Windows (`tauri_windows_dir`) and Capacitor on Android
  (`android_webview_dir`), and their origins. The guide's "Moving from React" shows the
  first-run import. Tested on stores Chrome 157 wrote and, in a browser, on Firefox.
- **Sharing**: `App::share(window, ShareOptions, done)` hands a title, a text and files made
  in memory (`ShareFile`: name, MIME type, bytes) to the platform's share sheet, and `done`
  gets a `ShareOutcome`: `Shared`, `Cancelled`, or `Unsupported` with the reason, where the
  app falls back to a save dialog or the clipboard. Android shows its chooser
  (`ACTION_SEND` / `ACTION_SEND_MULTIPLE`; shared once the person picks a target, which is
  all Android tells), with the files written to the app's cache and served read-only to the
  chosen app by a new `SharedFiles` provider; Windows its share window (`DataTransferManager`
  through `IDataTransferManagerInterop`, the files as read-only storage items; cancelled is
  reported from Windows 10 2004); a browser page `navigator.share`, with `File` objects where
  `navigator.canShare` takes them and the title and text alone otherwise (unless
  `ShareOptions::require_files`), in answer to a click or key press. Linux and macOS answer
  unsupported with the reason. The host contract gains `Host::share`, `ShareStarted`,
  `ShareId` and `HostEvent::Share` (before 1.0, so the exhaustive host enums may still grow;
  `Host::share` has a default that answers unsupported), and `PlatformHooks::share` in the
  winit core; the headless host records shares and scripts their outcomes (`shares`,
  `answer_share`, `shares_answer_later`).
- **Remedy Symbols** (`remedy-ui-fonts`, feature `remedy-symbols`, on by default, 9 KB): a
  fallback face of 85 interface symbols the Remedy text families lack (arrows, geometric
  shapes, stars, check marks and crosses, ⚠ ☰ ≡ ⌕ ⌘ ⌂ ⋮ ⋯ ⚙, media controls), drawn from
  geometry on Remedy Sans UI's metrics by `symbols/build.py` and licensed OFL-1.1;
  `REMEDY_SYMBOLS` and `SYMBOL_CHARS`.
- **Symbol fallbacks** in `remedy-ui-text`: `FontCollection::symbol_fallbacks`,
  `set_symbol_fallbacks` and `DEFAULT_SYMBOL_FALLBACKS`.

### Changed

- `RemoteStatus` in `media-playback` has a new public field, `reported_at`: a struct
  literal adds `reported_at: None` (or `..RemoteStatus::default()`).
- `RemoteStatus::volume` is now `Option<f32>`, `None` (unknown) by default and on
  `Player::set_remote`: a struct literal writes `volume: Some(v)`; `.volume(v)` is unchanged.
- `NextUp::source` is now `Option<MediaSource>` (`None` for `NextUp::later`), and
  `RemoteStatus` has a `next` field: a struct literal adds `next: None`.

- **No patched crates.** wgpu is crates.io's 30.0.2 again (the fork is gone), and vello_cpu
  with RemedyUI's fix is its own crate, `remedy-vello-cpu`, which `render-cpu` depends on. The
  workspace patches nothing, so apps from `cargo remedy new` need no `[patch]` section, and
  `cargo deny` allows no git source.
- **`Surface` and `CompositeTarget` gain `size` and `color_matrix_group`** (required): what
  draws a colour-matrix group's picture. Only a custom renderer implements these traits; it
  renders what the callback draws into a picture of its own, runs
  `ColorMatrix::apply_to_pixels` over it, and places it back (see the CPU renderer's).
- **The GPU compositor draws only what changed.** Each window's frame is kept in a texture;
  a present composites only the plan's damage into it (each damaged rectangle a scene of its
  own size placed at its corner, only that rectangle cleared), before acquiring the swapchain
  image, then copies it to the swapchain (or draws it onto one that takes no copies). Damage
  over most of the window, a new size, a rebuilt device or the first frame draw it all.
  Tested to equal a full redraw pixel for pixel, frosted glass included. wgpu has no present
  with damage, so the GPU path presents the whole kept frame. `GpuCompositor::present_to_frame`
  keeps its frame the same way: hand it every plan. Tiles whose content holds filters raster
  on their own rather than in the small-tile atlas.
- **`Window::snapshot` of a GPU window** reads back the frame the GPU shows instead of
  drawing the frame again on the CPU, so pictures hold Prism effects, video, `gpu_view`
  textures, blurs and glass. The effects gallery's `--snapshot` no longer needs `--cpu`.
- Breaking (pre-1.0): `paint::Item` gains `PushFilter`, `PopFilter` and `Backdrop`, and
  `compositor::CompositeOp` gains `Backdrop`, `PushFilter` and `PopFilter`; exhaustive matches on them need the new arms
  (`PushFilter` closes with `PopFilter`, like `PushLayer`). `render_cpu::Surface` gains
  `push_filter_layer` (vello's `push_filter_layer`). `Layer` gains `backdrop` and
  `filters` fields (build layers with `Layer::new`), and `ResolvedStyle` gains `shadow`, `blur`, `drop_shadow` and
  `backdrop_blur`. `EffectStats::level` now counts the GPU's steps too.
- `docs/design/04-rendering.md` describes what the renderers do, and lists what is planned
  under "Not yet in code" (layer squashing, a backend self-test and present watchdog, partial
  presents on the GPU, platform presenting details and system backdrops, HDR, zero-copy
  video, mipmaps, colour-matrix filters, printing). It had described several of those as
  existing. `16-effects.md` and `10-platform-bridges.md` follow.

- The CPU compositor composites only the damage: each damaged rectangle is recorded on its
  own (tiles and clipped subtrees outside it left out) and rasterized into the kept frame.
  Both compositors clip with clip paths instead of isolated clip layers. The GPU compositor
  reuses a tile's texture when it is rastered again at the same size, and rasters a frame's
  small tiles together in one render. 50 animated sprites: 2.4 to 0.9 ms a frame on the CPU.

- A child's repaint no longer re-records its ancestors' paint (they were re-recorded and
  thrown away identical): an animated element inside a costly backdrop redraws only itself.
- Secondary text (`DerivedTokens::muted`: labels, help under controls, unselected options,
  table headings) is balanced to 7:1 instead of 4.5:1, on the background and on panels and
  cards, and body text (`DerivedTokens::text`) holds 7:1 on all three too: quieter than body
  text, never faint, in every palette dark and light.
- Menus highlight the entry with keyboard focus whether or not focus came from Tab (arrow
  keys moved focus without showing it), and the pointer resting on an entry focuses it, so
  there is one highlight for the pointer and the keyboard. Tab closes an open menu and gives
  focus back. Nothing to change in apps.
- **Android APKs carry the bridge's Java.** `cargo remedy package --target android` (and
  `tools/build-apk.sh`) now ship the Android bridge's `classes.dex` with `hasCode="true"` and
  run the app in `dev.remedyui.android.RemedyActivity`, a `NativeActivity` that also hears
  new intents (a notification pressed while the app runs), with a not-exported
  `NotificationOpen` alias, the `SharedFiles` provider `App::share` serves files through
  (not exported, per-URI grants, authority `<id>.remedy.share`; Android makes providers only
  from the APK's own code) and, for `[android] foreground`, the `Foreground` service. The
  activity keeps the native activity's library, launcher entry, configuration changes,
  keyboard mode and theme. The bridge uses the APK's classes when they are there and still
  loads its dex from memory for apps on a plain `android.app.NativeActivity`. APKs now also
  carry the app's icon (`[app] icon`, or the default one) as their launcher icon. Scripts
  that start the activity by name use `dev.remedyui.android.RemedyActivity`.
- Persisted state and pending window geometry are written when the app is suspended or goes
  out of sight (every window hidden or covered, a page's tab hidden), not only when it quits:
  Android and browsers may end a background app or a hidden tab without warning.
- `remedy-ui-reactive`'s `persist` feature brings `web-sys` and `wasm-bindgen` on
  `wasm32-unknown-unknown` only.
- **Boards for apps that build on them** (`remedy_ui::board`, from FreeSpeak's Home):
  - `BoardLabels` (on `Context::labels`): every word a board says (editor, panel, empty
    states, refused-change messages, setting editors, built-in widgets, reactions), English
    by default; `BoardLabels::translated(|en| ..)` passes them all through a catalog or the
    app's `t()`, and `{name}` placeholders are filled after translation.
    `Registry::with_builtins_in(&labels)` and `reactions_in(.., &ReactionLabels, ..)` take
    the words too.
  - `BoardOptions` (`BoardState::with_options`, `BoardState::options`): `one_widget_per_area`
    (the editor offers adding only to an empty area and refuses moving a widget into a full
    one), `row_height` (fixed rows; areas scroll what does not fit), `panel` (`Panel::Beside`
    or `Panel::Off`), `settle_after` and `area_label` (the app's names for areas).
  - `board_panel(ui, state, registry, cx)`: the editing panel on its own, for one panel
    among several boards on a page, and `BoardState` operations to drive editing from
    outside: `add_area`, `add_widget`, `move_widget`, `move_selected`, `resize_selected`,
    `move_selected_widget`, `remove_selected`, `current`.
  - `BoardState::settled`: the document as of the last finished change. Typing in a setting
    reaches it once (when the field loses focus, on Enter, after `settle_after` without a
    keystroke, with any other change, or `finish_typing()`), so apps send one change rather
    than one a keystroke; `is_typing()` says whether typing is under way.
  - Setting kinds (`remedy_ui::script::SettingKind`): `Time` (seconds since 1970, edited with
    the form pack's date and time pickers in local time, clearable), `LabeledChoice` (stored
    values with their own labels, such as rooms and roles by id; a stored value no longer
    offered stays and says so), `List` (an array of strings, an item a field, add and remove)
    and `Picture` (a source the app shows with `Context::picture` and chooses with
    `Context::choose_picture`). Script manifests declare them as `time`, `list`, `picture`
    and `choice` options of `{ value, label }`.
  - `Registry::set_addable`, `is_addable` and `addable`: a kind boards show but this viewer
    cannot add.
  - `WidgetKind::label` (default `None`): what a placed widget is called, such as its title.

- **Board areas are named once**: an area reads as its title ("News area"), else as its one
  widget's label, else by its number ("Area 2"), no longer "Area area"; placed widgets read
  as their label ("Hello widget") rather than only their kind. A refused change names the
  area by its title rather than its id. Panel fields keep focus while their own editors
  change the board (it no longer builds again on each keystroke).

### Fixed

- **The pointer hides reliably over a full-screen player on Windows.** winit hides it with
  `ShowCursor` only while it believes the pointer is inside the window; the Windows bridge
  now also answers `WM_SETCURSOR` with no cursor while it is hidden and clears the cursor
  at once when the pointer is over the window (`PlatformHooks::set_cursor_hidden`).

- The `remedy-ui` runner tests build for wasm32 again: the PNG check of
  `Window::snapshot_png`, which comes with the `devtools` feature, runs only with it.
- A cancelled pointer (`PointerPhase::Cancel`) over a pressed element panicked in the view.
- The media player could end before the last of the sound was played on a busy machine:
  the end was declared once the decoded sound was taken, while the time stretcher still held
  its last blocks (or an output a last partial block). It now waits until the output has
  been told nothing follows.
- On Linux, a notification's button press could be lost when its close came right after:
  the two signals were heard on separate threads and the close could be handled first. They
  are now heard in order on one stream.
- On Android, local files now play: they are opened by the app and handed to MediaCodec's
  extractor as a descriptor, since the media server may not open an app's own files by path.
- **Stacked glass could stall a frame for minutes.** Each pane of glass draws again what is
  under it, glass included, so every pane doubled the cost (16 stacked panes took 40 s on the
  CPU). Glass seen through more than `MAX_BACKDROP_DEPTH` (2) panes, and panes past
  `MAX_BACKDROP_REPLAYS` (128) in a raster, are now drawn without their frost, on both
  renderers and in the compositor: 40 stacked panes draw in milliseconds.
- A display list nesting thousands of groups crashed the GPU renderer: vello's layer handling
  recurses per level, so about 2000 nested `PushLayer`s overflowed a 2 MB thread stack
  (deeply nested compositor layers did the same in the composite walk). Renderers now keep
  at most `MAX_GROUP_DEPTH` (256) clips, opacity and blend groups, filters and glass layers
  open at once, in `Painter` and in `record_ops` alike: a push past it opens nothing and its
  pop closes nothing, so what it holds is drawn as if the group were not there, the groups
  around it stay balanced, and glass that deep is not drawn. Lists that deep also draw much
  faster on the CPU (tested on both renderers, directly and through the compositor).
- The compositor overflowed the stack on layer trees a few hundred layers deep: committing,
  walking (`Compositor::frame`), `LayerTree::for_each`, `LayerTree::find` and dropping a
  `Layer` each recursed once per level. They now use explicit stacks, so a tree of any depth
  is walked and dropped on any thread (tested at 20,000 levels).
- Element trees a few hundred levels deep overflowed the stack, and deep trees took time in
  the square of their depth. Now any depth works on any thread (tested at 20,000 levels on a
  2 MB test thread, and on a 512 KB one), in time that grows with the tree's size:
  - Layout runs taffy, which recurses per level, on a stack sized from the tree's depth.
  - The frame walk and memos reading memos (a chain of any length) run on a stack that grows
    (`stacker`). Disposing nested scopes and hit testing keep explicit stacks.
  - Attaching a subtree restyles only its root (its children follow when what they inherit
    changes), the style pass remembers depths on the way up, inserting a leaf skips the cycle
    walk, the accessibility tree finds merging ancestors in one pass per update, and removing
    a subtree clears its hover chain once. Unoptimized, at 10,000 levels: building 9 s to
    0.13 s, the first frame 46 s to 1.1 s, removal 0.6 s to 0.06 s; the accessibility tree's
    test at 20,000 levels 196 s to 0.3 s.
- `wasmi` 2.0.1, which runs compiled widgets (`script-wasm`), was yanked upstream: it is now
  2.0.2, the patch release that replaced it, so `cargo deny check` passes again.
- `cargo fmt --all --check` and `cargo clippy --workspace -- -D warnings` (Rust 1.97) are
  clean again: unformatted files in the bridges, script, viz and stage and board widgets,
  and a redundant `drop` in the themes' style sheet.
- Prism: an `if` that changed two or more variables merged them in a hash map's order, so
  the same source could compile to differently ordered (equally correct) programs. They now
  merge in name order.
- Animations at rest or finished sample their exact target (`Animation::sample`): colors
  move in Oklab and came back a few millionths off, so a reduced-motion `animate_style`
  never quite reached its color.
- A display list with a pop too many (`PopLayer`, `PopClip`, `PopFilter` with nothing open)
  or groups left open made the renderers panic ("some layers haven't been popped yet"); the
  painter now skips unmatched pops and closes what a list leaves open. A fuzz test draws
  random unbalanced lists, with absurd filter values, directly and through compositor tiles
  on both renderers.
- A snapshot of a window the GPU draws left out GPU content (Prism effects, `gpu_view`
  textures, video converted on the GPU): it was drawn again by a CPU renderer that cannot read
  GPU textures. It now reads back the GPU's own frame.
- Overlays placed near or beside something (tooltips, menus, submenus) are measured from
  the window's start edge, so one opened at the right edge keeps its width instead of
  wrapping its text into the few pixels left there and being placed at that size.
- Readable text: when no text color suits the background, panels and cards at once (light
  cards in a dark theme), the background keeps its contrast; the cards lost it to the
  background before only by accident of order, and the 7:1 change had reversed that.
- `remedy-tools` previews: each previewed function builds in a harness of its own, so two
  processes previewing one app (two MCP servers, `cargo remedy preview` beside one) no longer
  rewrite each other's harness mid-build and return each other's picture.
- Android: hardware keys (Back, keyboard shortcuts) reach the app again outside text fields.
  The bridge's input-method view was the window's one focusable view, so it held focus all
  the time and the input method took every key; it is now focusable only during a text
  session.
- `nav::drawer`: a press outside the panel closes it, read from the pointer on the scrim, so
  the panel no longer takes clicks it does nothing with (the accessibility check reported it
  as a control with no action); the scrim names Escape as its keyboard alternative.
- The bundled Remedy Sans and AhmiPrint fonts: curved letters no longer sit a pixel off the
  baseline. Their 1.201 builds put an outline point at the bottom and top of every curve, so
  e, s, C, G, S and the letters built from them hint onto the same rows as n and o, and 3
  reaches the baseline.
- `select`, `segmented` and `radio_group` with nothing chosen (an index past their options,
  as for a stored value none of them is): the arrows start at the first or last option and
  typing finds from the first, instead of overflowing.
- `cargo remedy package` reads a version inherited from the workspace
  (`version.workspace = true`) from `cargo metadata` instead of packaging it as 0.1.0.
- Android apps out of sight hear what other threads report. winit's Android backend ignores
  the loop's wake-ups while the activity is paused, so a notification's button, a finished
  `App::background` job or a microphone wake waited until the app was on screen again. The
  bridge now tells the core (`PlatformHooks::wait_slice`, new) to look every 100 ms while
  the activity is paused; Android freezes cached apps anyway, and one doing foreground work
  answers at once.
- On Android, pressing a notification that starts the app reaches `App::on_notification`:
  answers that come before the app's setup sets it are kept and handed over right after.
- **`App::tray_shown` and a window's maximized and full-screen state no longer vanish.**
  Their signals were made under whatever reactive owner was current where they were first
  asked for (a window's build, a binding), so that owner running again disposed them and the
  next read panicked ("the signal was disposed"). They now live in a scope of their own.
- **`App::set_tray(None)` removes the icon again.** Since `App::tray_shown`, the host was
  only told about a tray that was shown, so taking the icon away never reached it.
- **`El::truncate` and `El::nowrap` hold for a text that is bound inside a binding.** A
  text bound to a signal or closure while another binding runs (a band built by `show`)
  gets its first words only after that binding, and the two calls only marked a text that
  already existed: the band wrapped onto more lines instead of ending in "…". They now make
  the text (empty) when there is none yet, and its words keep their place on one line.
- **A chat list kept to a length no longer jumps to the top.** When the oldest messages are let
  go from the start (with or without new ones at the end) while someone reads higher up,
  `chat_list` keeps the view on what it showed (`Change::Trimmed`) instead of starting over at
  the first message.

- **A window shown again, or uncovered on X11, is drawn again.** X11 does not keep an unmapped
  or covered window's pixels, and with damage-only presents a window brought back from the
  tray stayed black until something in it changed. Showing a window and
  `HostEvent::Occluded(window, false)` now present all of it (`Compositor::damage_all`), and
  the winit bridge reports a redraw the app did not ask for (an X11 expose) as
  `Occluded(window, false)`.
- **A word longer than its line breaks instead of running past its box.** Paragraphs wrap with
  `overflow-wrap: break-word`: an invite, a link or a key pasted into a narrow text area (a
  phone) now wraps inside it. Text with spaces wraps as before, and min-content widths are
  unchanged.
- **Text taller or wider than its field scrolls inside the box.** A text area given a height (a
  dialog's invite box on a phone) drew its later lines over what sat below it, and a single line
  longer than its field ran past the field's edge. Fields now clip their text to the box, scroll
  to the caret as the person types or moves it, and scroll with the wheel (Shift for sideways)
  while there is more text that way, handing the wheel outward at either end. Clicks, the
  input method's caret box and `TextField::caret_rect` follow the scroll.
- **Removing an element no longer panics when its clean-ups change the view.** `El::remove`
  disposed the removed elements' scopes while the view was borrowed, so an `on_cleanup` that
  touched the view (a page closing the dialog it opened on the overlay as it goes away)
  panicked with "RefCell already mutably borrowed" and took the UI thread down. The scopes
  are now disposed after the view's bookkeeping, with nothing borrowed.
- **Padded text with a decided width wraps inside its padding.** A measured leaf (text, a
  text field) whose width the layout had already decided (stretched in a column) was
  measured at its border box's width, wider than the content box it is drawn in: it came out
  with too few lines and its last line drew under or past its box (a fingerprint in a
  padded code box on a phone). `MeasureInput::known_width` and `known_height` are now the
  content box's, as `available_width` and `available_height` already were.
- **A tap leaves no hover and no tooltip behind.** A lifted finger kept the element under it
  hovered, so on a touch screen every tap left its hover look on, and a tooltip opened a
  moment later and stayed over the screen until the next touch elsewhere. A touch's release
  (and any cancelled pointer) now hovers nothing; a long press still shows the tooltip while
  the finger is down. A mouse keeps hovering after its release, as before.
- **`widgets::lamp` shows the color of its kind of status.** A lamp on was always the
  theme's ok color, so a problem band's light (variant `danger`) was "all is well" green. It
  now shows its color from the style sheet: ok by default, and the danger, warn or accent
  color with that variant (`ok` names the default). Off is the line color, as before.
- `widgets::toast` offers assistive technology the click action that dismisses it. It
  reacted to a pointer click with none on its semantics, so the accessibility audit
  (`Unreachable::NoAction`) failed in every app that showed a toast.
- **An empty `show` takes no room**: a `show` with nothing to show (its condition false, no
  fallback, no exit transition still playing) no longer takes space or a gap in its row or
  column, so screens that take turns can each `grow` in one column. Apps that hid it
  themselves with `El::hidden` can drop that.
- Symbols such as ✦ ✧ ◈ ◆ ◇ ◎ ⚠ ☰ ⌕ ⇄ drew as missing-glyph boxes, with or without
  system fonts. They are in Unicode's Common script, so they took the script of the text
  around them, whose fallback fonts did not have them. A character that still has no glyph
  after its style's families and its script's fallback fonts is now laid out once more with
  the symbol fallbacks after its families (Remedy Symbols, then Segoe UI Symbol, Apple
  Symbols, Noto Sans Symbols, DejaVu Sans and the emoji fonts), keeping its span's color,
  link and other style; paragraphs without a missing glyph are shaped once, as before.
- **Text no longer grows by the display scale a second time**: a paragraph shaped at a
  device scale (`ParagraphOptions::scale`, which every text element passes) reported its
  sizes and positions in device pixels, while everything that reads them works in logical
  pixels, so on any scale above 1 text and the controls sized by it were drawn that many
  times too large: 2.625× on a 420 dpi Android phone (a default button 59 dp tall, body text
  about 35 sp), 1.5× on a Windows display at 150%. `Paragraph` now shapes at the device
  scale (hinting and rounding still land on device pixels) and reports widths, heights,
  baselines, lines, glyph runs, inline objects, carets and selections in logical pixels, and
  takes wrap widths and hit-test points in them; at scale 1 nothing changes.

## [1.0.0-rc.1] - 2026-11-18

The first release candidate of 1.0: everything since the start of the project, by milestone ([docs/ROADMAP.md](docs/ROADMAP.md)).

### Added

- **M1, foundations.** `remedy-ui-core` (geometry, scale and pixel snapping, colors in sRGB,
  linear light and Oklab, generational arenas, constant-time tokens); `remedy-ui-reactive`
  (glitch-free signals, memos, effects, scopes, `SignalVec`, stores with field-level
  reactivity, the `Waker` for posts from other threads); `remedy-ui-macros`
  (`#[derive(Store)]`); `remedy-ui-tree` (elements, dirty flags, events, hit testing, focus,
  error boundaries); the headless test harness; the workspace rules (lints, layer graph,
  unsafe boundary, `cargo deny`).
- **M2, text.** `remedy-ui-fonts` (the Remedy families as data); `remedy-ui-text` (rich
  paragraph layout over parley with bidi and inline objects, hinted grayscale and LCD glyph
  rasterization, the glyph atlas, the editing core with undo and IME composition, secret
  text, deceptive-text defenses).
- **M3, layout, style, input and semantics.** `remedy-ui-layout` (flex, grid and block through
  taffy, right-to-left, snapping, virtual lists with anchoring); `remedy-ui-style` (themes as
  tokens, contrast-balanced derived tokens, cached resolution); `remedy-ui-input` (the event
  model, gesture arena, scroll physics, rebindable commands); `remedy-ui-a11y` (semantics to
  incremental AccessKit updates); `remedy-ui-motion` (easing, tweens, springs, the frame clock).
- **M4, rendering, the compositor and the bridges.** `remedy-ui-paint` (display lists);
  `remedy-ui-render-cpu` (vello_cpu with RemedyUI's glyphs, LCD text); `remedy-ui-render-gpu`
  (wgpu and vello_gpu, a backend ladder, a pipeline cache, device-loss recovery);
  `remedy-ui-compositor` (layer trees, tiles, damage, compositor-side scrolling and
  animation, its own thread); `remedy-ui-host` (the platform-neutral host model);
  `remedy-ui-bridge-headless`; `remedy-ui-bridge-winit` and the Windows (with TSF), macOS and
  Linux bridges (clipboard, capture protection, settings, tray, file dialogs, single
  instance, screen readers).
- **M5, the first app (SecretSticky).** `remedy-ui-view` (elements, the frame pipeline, input
  routing, editable fields); the `remedy-ui` facade and runner (CPU-first windows that move
  to the GPU when they need it); `remedy-ui-themes` (the twelve Remedy palettes, light and
  dark, high contrast); `remedy-ui-widgets-core`; `remedy-ui-widgets-forms` (exact numbers,
  colors, key capture, dates, times, forms); `remedy-ui-devtools` (the inspector, previews as
  golden images); `remedy-ui-secure` (idle lock, self-clearing secret copies, privacy cover);
  `remedy-ui-services-update` (signed updates); `cargo-remedy` (`new`, `run`, `package` for
  Windows).
- **M6, data apps (SecretFolder, Hearth).** `remedy-ui-widgets-nav` (commands, tabs, rails,
  stacks, toolbars people arrange, the command palette, drawers, sheets, a custom title bar);
  `remedy-ui-widgets-data` (tables, grids, trees and property grids at any size);
  `remedy-ui-media-image` and `remedy-ui-media-svg`; `remedy-ui-services-i18n` (Fluent,
  locale data, exact decimals); `remedy-ui-services-desktop`; drag and drop within the app
  and with other apps, Linux included.
- **M7, FreeSpeak.** `remedy-ui-doc-rich` (Markdown, the rich text editor, find);
  `remedy-ui-viz` (the speaking ring, meters, sparklines, bars); `remedy-ui-services-spell`;
  `remedy-ui-media-anim`; `remedy-ui-media-isolated` (decoding in a sandboxed helper on every
  desktop platform); `remedy-ui-media-video`; boards people arrange with scripted widgets
  (Rhai and WebAssembly script engines behind one interface); chat lists, selection across
  messages, pop-out windows; `remedy-ui-bridge-android`.
- **M8, breadth.** `remedy-ui-doc-pages` (paged documents with progressive zoom, spreads,
  selection, search, thumbnails, annotations); `remedy-ui-doc-code` (code views and the
  editor with several cursors); `remedy-ui-media-playback` (the platforms' decoders, the
  player, captions); `remedy-ui-widgets-music`; `remedy-ui-widgets-extra`; full-screen
  windows; `remedy-ui-bridge-web` (apps in a browser page, with `remedy_ui_core::exec` and
  `remedy_ui_core::time` so the engine runs without threads there).
- **M9, power tools.** `remedy-ui-automation` (agents with consent); `remedy-ui-rui` (`.rui`
  markup compiled at build time or loaded for people's own layouts); `remedy-ui-embed-gpu`;
  `remedy-ui-embed-webview`; `remedy-ui-widgets-term`; `remedy-ui-customize` (themes,
  shortcuts, toolbars, menus, text and layouts people change, with undo).
- **M10, the 1.0 review** ([docs/API-REVIEW.md](docs/API-REVIEW.md)):
  - Editions: `remedy_ui::Edition` (`E2026`), `AppConfig::edition`, `Ui::edition`;
    `cargo remedy new` writes the newest edition.
  - `cargo remedy migrate` (`--dry-run`) with `crates/cargo-remedy/migrations.toml`.
  - `cargo xtask semver --baseline <rev>` (cargo-semver-checks 0.51.0) and `cargo xtask msrv`;
    `cargo xtask check` also checks publishing metadata. [docs/RELEASING.md](docs/RELEASING.md)
    and [docs/STABILITY.md](docs/STABILITY.md).
  - `remedy_ui::testing::TestApp`: tests find, press and type by role and accessible name.
  - `remedy_ui::a11y`, `remedy_ui::paint`, `remedy_ui::Store` (feature `macros`) and
    `remedy_ui::isolated` (feature `isolated`) through the facade.
  - `button_dyn`, `nav_item_dyn`, `App::tray_dyn`, `App::background_cancellable` and
    `BackgroundJob::cancel`, `Commands::export_bindings` / `import_bindings`,
    `Window::attach_agents`, `Agents::request_access`.
  - Constructors and setters for every type that became `#[non_exhaustive]`
    (`StylePatch::new()`, `KeyEvent::press`, `PointerEvent::new`, a setter per options field).
  - `cargo remedy new --template document|android`; `cargo remedy package --target
    linux|android|macos`.
  - Typed errors: `PngError`, `DesktopError`, `BackendError`, `PrepareError`, `Rejection`,
    `BindingsError`, `ParseEditionError`.
  - From the documentation guides: `TestApp::node` (a node's value and states: checked,
    selected, expanded, disabled, focused), `TestApp::key_press` / `key_release`,
    `TestApp::reader_view`, `TestApp::data_dir`; `ReaderNode::{checked, selected, expanded,
    disabled}`; `remedy_ui::{show, catch_errors, suspense}` (with `Show`,
    `SuspenseBoundary`, `CaughtError`, `Retry`) at the facade's top level; the spacing
    scale's half and layout steps (`Space::S0_5` … `S3_5`, `S10`, `S12`, `S16`, `S20`,
    `S24`, `Space::ALL`, `name` / `from_name`); `ListItems::get`; `AppConfig::web` (where the
    first window mounts in a page); `remedy_ui::web` on every target (types and docs, no web
    code natively); `cargo remedy package --target web` (any app as an installable page).

### Changed

- `.rui` spacing words are the theme's spacing scale, half steps and layout steps included
  (`s0_5` … `s24`); `remedy_ui_rui::rt::Space::Units(u8)` is now `Space::Token(Space)` and
  `spec::space_units` is `spec::space_token`. A wrong word lists the scale.
- Generated apps' README and `remedy.toml` describe `cargo remedy package --target web` and
  the other targets.
Breaking before 1.0; `cargo remedy migrate` applies the renames marked (m).

- `remedy_ui::text::StylePatch` is `TextStylePatch` (m); `remedy_ui::view::TextPatch` is
  `TextStylePatch` (m).
- Boolean states read `is_*`: `HeadlessWindow::{is_maximized, is_fullscreen, is_minimized,
  is_frame_requested}` (m), `HeadlessHost::has_exited` (m), `Player::{is_muted,
  is_fullscreen}` (m), `WebView::is_loading` (m), `Ui::is_occluded` (m),
  `Terminal::is_monospace` (m).
- `Ui::set_time(ms: u64)` is `Ui::set_clock(Duration)`, and the runner now sets it (undo
  steps group by time as they always should have).
- `widgets::label(ui, s, variant)` is deprecated for `widgets::text(ui, s).variant(variant)`
  (m).
- `remedy_ui::RunError` is an enum (`SingleInstance`, `Bridge`, `UnsupportedPlatform`,
  `NeedsFeature`); the bridges' and `exec::SpawnError`'s reasons are behind `reason()`.
- `App::open_url` reports `host::Unsupported` instead of a `String`; `App::background`
  returns a `BackgroundJob`.
- Error types replace `String` errors: `Snapshot::to_png` / `from_png` (`PngError`),
  `autostart::set`, `set_in` and `reveal` (`DesktopError`), `SpellBackend` and
  `BackendFactory` (`BackendError`; `SpellError::Backend` carries it), `GpuContent::prepare`
  (`PrepareError`), table edits, tree imports and moves and form validators (`Rejection`:
  `Err("why".into())` still works).
- Enums and option, config, settings, stats and report structs that will grow are
  `#[non_exhaustive]`: `match` needs a wildcard arm, and struct literals become the type's
  constructor or `Default` plus setters (`StylePatch::new().background(c)`,
  `MeterOptions::default().threshold(t)`, `KeyEvent::press(k).with_modifiers(m)`).
- Compiled `.rui` code reaches the runtime through `remedy_ui::markup` by default;
  `build::compile_dir_to` takes the root path first.
- `#[derive(Store)]` names the runtime the way the crate depends on RemedyUI.
- `rich::markdown` takes `impl IntoProp<String>` (a string, signal, memo or closure), and
  `rich::markdown_dyn` is gone (m): `markdown(ui, move || reply.get(), open)`.
- `data::chat_list` takes `impl Into<ListItems<T>>`: a `Signal<Vec<T>>` still works, and a
  `SignalVec<T>` now does too.
- `Space::units` returns `f32` (half steps); `Space::px` resolves through the theme as before.
- `TestApp::key` sends the key's press and then its release, as a keyboard does (it sent only
  the event given); `TestApp::find`, `find_all` and `press` match the accessible name screen
  readers get, so merged rows are found by their composed name and what they merge is not
  found on its own.
- `remedy_ui_bridge_web::run_single_instance` takes the `WebConfig` to start with; the
  bridge's `run`, `run_with`, `run_single_instance`, `launch_args` and `is_running` are
  documented on every target (they exist only in a page).
- `tools/build-web.sh` packages through `cargo remedy package --target web --example`; the
  page templates moved into `cargo-remedy`, and the demo's names and icon are in
  `crates/remedy-ui-bridge-web/remedy.toml`.
- `.github/workflows/ci.yml` runs only when started by hand: the owner's local gate is CI.
- Every published crate's manifest has keywords, categories and a readme; `remedy-ui-test` is
  `publish = false`.

### Fixed

- Scripted widgets are no longer stopped as "too slow" or "out of steps" on a busy machine.
  Their time limit counts the work done on their own thread (new `remedy_ui_core::time`
  `ThreadTime` and `WorkClock`: over a budget only when both the thread's processor time and
  the monotonic clock say so), speed measurements last several ticks of the thread clock
  (`ThreadTime::resolution`, about 15.6 ms on Windows; `remedy_ui_script::min_sample`), and
  until a device's speed is measured only the time limit applies (`Limits::step_budget`).
- On a virtual machine whose host takes the processor away, a script's own thread clock counts
  that time as run: the time limit now stops a call only when it also made its steps at the
  device's measured speed (`StepRate::ran_for`), so a starved fair call is not stopped while
  an endless loop still is.
- WebAssembly widgets compile when they load, so a call's fuel pays only for running the
  widget, never for compiling it (a first call could fail on compiling a large function).
- A page view fits its zoom as soon as it learns its size, so its first frames render only
  the tiles that show.
- A pause in typing starts a new undo step (the view's edit clock never advanced).
- Assistive technology can no longer press a disabled control, which let a screen reader (or
  anything driving one) press a trusted prompt's buttons during the moment they stay disabled.
- `submit_on_enter` works for plain multi-line fields, not only rich ones.
- `TestApp` (and so every `cargo remedy new` app's tests) no longer writes into the person's
  real data folder: a test app keeps persisted signals in a private temporary folder,
  removed when it is dropped, unless the test sets `AppConfig::data_dir`.
- A trusted prompt's button delay starts when the prompt is first seen (after the frame that
  shows it), and starts over when the window is covered or minimized and shown again: built
  outside an event, or in a window nobody could see, it was shortened or over before it
  showed.
- With nothing focused, accessibility focus is the window again, not the element that last
  had focus.
