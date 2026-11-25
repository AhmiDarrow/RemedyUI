//! The player as a Portal app sets it up (Portal's theme and style sheet, its SVG glyphs, its
//! picture-in-picture button through `PlayerParts::end`), playing a still of Portal's test
//! file, drawn by the CPU renderer and checked against the values Portal's page has
//! (`web/src/styles.css`, `web/src/themes.ts`, the SVGs in `web/index.html`), so its look
//! cannot drift. `REMEDY_SHOTS=<dir>` also writes every state's picture.

use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_core::input::{PointerEvent, PointerPhase, ScrollPhysics};
use remedy_ui_core::{Point, Size};
use remedy_ui_engine::compositor::Compositor;
use remedy_ui_engine::render_cpu::CpuCompositor;
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_media::media_video::{Picture, PixelFormat};
use remedy_ui_media_playback::*;
use remedy_ui_text::FontCollection;
use remedy_ui_widgets::base::Icon;
use remedy_ui_widgets::devtools::Snapshot;

/// Portal's glyphs, as its page draws them: filled paths, and `.line` paths stroked 1.8 wide.
fn portal_svg(icon: Icon) -> Option<&'static str> {
    Some(match icon {
        Icon::Play => r#"<path d="M8 5.5v13l10.5-6.5z"/>"#,
        Icon::Pause => r#"<path d="M7.5 5.5h3v13h-3zM13.5 5.5h3v13h-3z"/>"#,
        Icon::Back10 => {
            r#"<path class="line" d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4.5v3.7h3.7"/><text x="12" y="15.6">10</text>"#
        }
        Icon::Forward10 => {
            r#"<path class="line" d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v3.7h-3.7"/><text x="12" y="15.6">10</text>"#
        }
        Icon::Volume => {
            r#"<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path class="line" d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>"#
        }
        Icon::VolumeMuted => {
            r#"<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path class="line" d="M16 9.5l5 5M21 9.5l-5 5"/>"#
        }
        Icon::Captions => {
            r#"<rect class="line" x="3" y="5.5" width="18" height="13" rx="2"/><path class="line" d="M10.5 10.3a2.2 2.2 0 1 0 0 3.4M17 10.3a2.2 2.2 0 1 0 0 3.4"/>"#
        }
        Icon::Theater => {
            r#"<rect class="line" x="2" y="6.5" width="20" height="11" rx="2"/><path class="line" d="M6 6.5v11M2 10h4"/>"#
        }
        Icon::Fullscreen => r#"<path class="line" d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5"/>"#,
        Icon::Close => r#"<path class="line" d="M6 6l12 12M18 6L6 18"/>"#,
        Icon::FullscreenExit => r#"<path class="line" d="M9 4v5H4M20 9h-5V4M15 20v-5h5M4 15h5v5"/>"#,
        _ => return None,
    })
}

/// Portal's picture-in-picture glyph (its own control, not the player's).
const PIP: &str = r#"<rect class="line" x="3" y="5" width="18" height="14" rx="2"/><rect x="12" y="11.5" width="7" height="5.5" rx="1"/>"#;

/// An SVG of Portal's, with the page's CSS for `.line` and `text` written onto it.
fn svg(ui: &Ui, body: &str, size: f32) -> El {
    let body = body
        .replace(
            r#"class="line""#,
            r#"fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round""#,
        )
        .replace("<text ", r#"<text font-family="Remedy Sans" font-weight="700" font-size="6.5" text-anchor="middle" "#);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">{body}</svg>"#
    );
    remedy_ui_media::media_image::svg::icon(ui, Arc::from(svg.into_bytes()), size, "")
}

/// What a Portal app hands the player for its glyphs.
fn portal_icons(ui: &Ui, icon: Icon, size: f32) -> Option<El> {
    Some(svg(ui, portal_svg(icon)?, size))
}

/// Portal's picture-in-picture button, styled as the player's own buttons.
fn pip_button(ui: &Ui) -> El {
    let size = ui.theme().metric("player-glyph", 16.0);
    ui.el("player_button")
        .row()
        .align(remedy_ui_core::layout::Align::Center)
        .justify(remedy_ui_core::layout::Justify::Center)
        .child(svg(ui, PIP, size))
        .semantics(
            remedy_ui_core::a11y::Semantics::new(remedy_ui_core::a11y::Role::Button)
                .label("Picture in picture"),
        )
}

/// A frame of Portal's test file (tests/fixtures/portal-frame.png), as a decoder gives it.
fn still() -> Picture {
    let png = include_bytes!("fixtures/portal-frame.png");
    let s = Snapshot::from_png(png).unwrap_or_else(|e| unreachable!("{e:?}"));
    Picture::packed(s.width, s.height, PixelFormat::Rgba, s.rgba)
}

/// Three minutes of the still, ten pictures a second, no sound.
struct Still(Picture);

struct StillDecoder {
    info: MediaInfo,
    picture: Picture,
    next: u64,
}

impl Decoder for StillDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        if self.next >= 1800 {
            return Ok(None);
        }
        let at = Duration::from_millis(self.next * 100);
        self.next += 1;
        Ok(Some(Sample::Video {
            at,
            duration: Some(Duration::from_millis(100)),
            picture: self.picture.clone(),
        }))
    }
    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.next = to.as_millis() as u64 / 100;
        Ok(())
    }
}

impl MediaHost for Still {
    fn name(&self) -> &str {
        "still"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let mut info = MediaInfo::default();
        info.duration = Some(Duration::from_secs(180));
        info.seekable = true;
        info.video = Some(VideoInfo::new(self.0.width, self.0.height, Some(10.0)));
        Ok(Box::new(StillDecoder { info, picture: self.0.clone(), next: 0 }))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("no sound in a still".into()))
    }
}

/// A window with Portal's theme `id` and sheet, run by hand and drawn on the CPU.
struct Window {
    ui: Ui,
    size: Size,
    comp: Compositor,
    cpu: CpuCompositor,
    timers: Vec<(u64, Instant)>,
    started: Instant,
}

impl Window {
    fn new(id: &str, size: Size) -> Window {
        let theme =
            remedy_ui_widgets::themes::portal_theme(id).unwrap_or_else(|| unreachable!("Portal has {id}"));
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        // SVG text (the "10" in the skips) in the app's faces, once for the process.
        static SVG_FONTS: std::sync::Once = std::sync::Once::new();
        SVG_FONTS.call_once(|| {
            remedy_ui_fonts::all()
                .into_iter()
                .for_each(|f| remedy_ui_media::media_image::svg::add_font(f.data.to_vec()))
        });
        let ui = Ui::new(theme.clone(), remedy_ui_widgets::themes::portal_sheet(&theme), fonts);
        ui.set_reduced_motion(true);
        let (w, h) = (size.width as u32, size.height as u32);
        Window {
            ui,
            size,
            comp: Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED),
            cpu: CpuCompositor::new(),
            timers: Vec::new(),
            started: Instant::now(),
        }
    }

    fn pump(&mut self) {
        let _ = remedy_ui_core::reactive::run_posted();
        let f = self.ui.frame(self.size, 1.0);
        if let Some(l) = f.layers {
            self.comp.commit(l);
        }
        for r in self.ui.take_requests() {
            match r {
                remedy_ui_engine::view::Request::Timer { id, delay } => {
                    self.timers.push((id, Instant::now() + delay))
                }
                remedy_ui_engine::view::Request::CancelTimer(id) => self.timers.retain(|(t, _)| *t != id),
                _ => {}
            }
        }
        let now = Instant::now();
        let due: Vec<u64> = self.timers.iter().filter(|(_, at)| *at <= now).map(|(id, _)| *id).collect();
        self.timers.retain(|(_, at)| *at > now);
        for id in due {
            self.ui.fire_timer(id);
        }
    }

    fn wait(&mut self, what: &str, ok: impl Fn() -> bool) {
        let start = Instant::now();
        while !ok() {
            self.pump();
            assert!(start.elapsed() < Duration::from_secs(30), "waited for {what}");
            std::thread::sleep(Duration::from_millis(3));
        }
        for _ in 0..4 {
            self.pump();
        }
    }

    fn shot(&mut self) -> Snapshot {
        self.pump();
        let (w, h) = (self.size.width as u32, self.size.height as u32);
        let plan = self.comp.frame(self.started.elapsed());
        let bg = self.ui.theme().colors.bg;
        let rgba = self.cpu.present(&plan, w, h, bg).map(|f| f.pixels).unwrap_or_default();
        Snapshot { width: w, height: h, rgba }
    }

    fn hover(&mut self, at: Point) {
        let _ = self.ui.pointer(&PointerEvent::new(PointerPhase::Move, at));
        self.pump();
    }
}

/// What to show.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Paused,
    Playing,
    HoverBar,
    Theater,
    Full,
    FullNext,
    Captions,
    HoverVolume,
}

/// Portal's dock player in `win`: two rows, the skips, no speed button, the volume on hover,
/// captions on offer, theater mode, the app's picture-in-picture; 20 seconds into the still.
fn portal_player(win: &mut Window, look: Look) -> (Player, El) {
    portal_player_with(win, look, NextUpPlacement::Below)
}

/// [`portal_player`] with its "Up next" card at `placement`.
fn portal_player_with(win: &mut Window, look: Look, placement: NextUpPlacement) -> (Player, El) {
    let ui = win.ui.clone();
    let options = PlayerOptions::default()
        .two_rows(true)
        .skip_buttons(true)
        .speed_button(false)
        .volume_on_hover(true)
        .next_up_lead(Some(Duration::from_secs(20)));
    let p = Player::open(&ui, Arc::new(Still(still())), MediaSource::url("still:portal"), options);
    p.on_caption_pick(|_| {});
    // A line at the foot and one placed near the top, as WebVTT's settings put them.
    let vtt = "WEBVTT\n\n00:00.000 --> 03:00.000\nRemedyUI cast test\n\n00:00.000 --> 03:00.000 line:10%\nA positioned line\n";
    let cues = if look == Look::Captions { parse_subtitles(vtt, SubtitleFormat::WebVtt) } else { Vec::new() };
    p.add_subtitles(SubtitleTrack { label: "English".into(), language: Some("en".into()), cues });
    if look == Look::Captions {
        p.set_caption_track(Some(0));
    }
    p.on_theater(|_| {});
    p.on_fullscreen(|_| {});
    p.set_title(Some("RemedyUI cast test".into()));
    // Portal's page hides its picture-in-picture under 640 px.
    let pip = if win.size.width >= 640.0 { vec![pip_button(&ui)] } else { Vec::new() };
    let parts = PlayerParts::default().icons(portal_icons).end(pip).next_up_placement(placement);
    let view = player_with(&ui, &p, "RemedyUI cast test", parts);
    let _ = ui.root().child(view.clone());
    win.wait("open", || p.info().is_some());
    let at = if look == Look::FullNext { 165 } else { 20 };
    if look == Look::FullNext {
        p.set_next_up(Some(NextUp::later("Episode 2")));
    }
    p.seek(Duration::from_secs(at));
    p.play();
    win.wait("playing", || p.state() == PlaybackState::Playing);
    if !matches!(look, Look::Playing | Look::Full | Look::FullNext | Look::HoverVolume) {
        p.pause();
        win.wait("paused", || p.state() == PlaybackState::Paused);
    }
    match look {
        Look::Theater => p.request_theater(true),
        Look::Full | Look::FullNext => p.request_fullscreen(true),
        _ => {}
    }
    for _ in 0..6 {
        win.pump();
    }
    (p, view)
}

/// The first shown element named `widget` under `root`, depth first.
fn find(root: &El, widget: &str) -> El {
    let mut stack = vec![root.clone()];
    while let Some(e) = stack.pop() {
        let shown = e.bounds().is_some_and(|b| b.width() > 0.0);
        if shown && e.info().is_some_and(|i| i.widget == widget && !i.hidden) {
            return e;
        }
        stack.extend(e.child_elements().into_iter().rev());
    }
    unreachable!("no {widget}")
}

fn bounds(view: &El, widget: &str) -> remedy_ui_core::Rect {
    find(view, widget).bounds().unwrap_or_else(|| unreachable!("{widget} laid out"))
}

/// Renders the player in Portal's palette `id` at `size`, `look`, writing the picture as
/// `name` when asked; the picture and the player's element.
fn render(name: &str, id: &str, size: (f32, f32), look: Look) -> (Snapshot, El) {
    let mut win = Window::new(id, Size::new(size.0, size.1));
    let (_p, view) = portal_player(&mut win, look);
    if look == Look::HoverBar {
        let seek = bounds(&view, "player_seek");
        win.hover(Point::new(seek.min.x + seek.width() * 0.5, seek.center().y));
        win.pump();
    }
    if look == Look::HoverVolume {
        let mute = bounds(&view, "player_sound");
        win.hover(Point::new(mute.min.x + 18.0, mute.center().y));
        win.pump();
    }
    let shot = win.shot();
    if let Ok(dir) = std::env::var("REMEDY_SHOTS") {
        let png = shot.to_png().unwrap_or_else(|e| unreachable!("{e:?}"));
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png)
            .unwrap_or_else(|e| unreachable!("{e}"));
    }
    (shot, view)
}

/// The largest difference between two colors' channels (alpha left out).
fn off(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).take(3).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(u8::MAX)
}

fn hex(s: &str) -> [u8; 3] {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or_default();
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The pixel at `(x, y)` is `want`, within 3 per channel (opaque, so premultiplied is plain).
fn assert_pixel(shot: &Snapshot, x: f32, y: f32, want: &str, what: &str) {
    let got =
        shot.pixel(x.round() as u32, y.round() as u32).unwrap_or_else(|| unreachable!("in the picture"));
    let want = hex(want);
    assert!(off(&got, &want) <= 3, "{what} at ({x}, {y}): {got:?}, Portal's is {want:?}");
}

/// Whether any pixel in the box is within 24 of `want` on every channel.
fn has_color(shot: &Snapshot, b: remedy_ui_core::Rect, want: &str) -> bool {
    let want = hex(want);
    let (x0, y0, x1, y1) = (b.min.x as u32, b.min.y as u32, b.max.x as u32, b.max.y as u32);
    (y0..y1).any(|y| (x0..x1).any(|x| shot.pixel(x, y).is_some_and(|p| off(&p, &want) <= 24)))
}

#[test]
fn player_wears_portals_look() {
    for (id, raised, line, accent, track, text, muted) in [
        ("portal", "#211e18", "#3a3428", "#e2b15a", "#2c281f", "#f4efe4", "#b3a894"),
        ("light", "#ffffff", "#a8a8b8", "#7c3aed", "#a8a8b8", "#14141f", "#3d3d52"),
        ("deutan", "#121821", "#34465a", "#56b4e9", "#34465a", "#e9f0f6", "#a9b9c7"),
    ] {
        let (shot, view) = render(&format!("portal-{id}-paused"), id, (730.0, 480.0), Look::Paused);
        // `.chrome`: on the raised surface under a 1 px line.
        let rule = bounds(&view, "player_rule");
        assert_eq!(rule.height(), 1.0, "{id}: the line over the bar");
        // The bar is 67.4 px high (Portal's is 67.375), so the line straddles two rows of
        // pixels: the row under the picture shows a good part of it over the raised surface.
        let (want, under) = (hex(line), hex(raised));
        let px = shot.pixel(40, rule.max.y.floor() as u32).unwrap_or_default();
        assert!(off(&px, &under) >= off(&want, &under) / 5, "{id}: no line over the bar: {px:?}");
        let gap = bounds(&view, "player_gap");
        assert_pixel(&shot, gap.center().x, gap.center().y, raised, "the bar");
        // `.icon-btn`: 2.25 rem round buttons with 1.3 rem glyphs, 0.1 rem apart.
        let play = bounds(&view, "player_button");
        assert_eq!((play.width().round(), play.height().round()), (36.0, 36.0), "{id}: Play's size");
        // `.volume-bar`: folded until hover or focus.
        assert_eq!(bounds(&view, "player_sound").width().round(), 36.0, "{id}: the volume folds");
        // `.seek-bar`: 1.35 rem high, a 4 px track, played in the accent, the rest the track.
        let seek = bounds(&view, "player_seek");
        assert_eq!(seek.height().round(), 22.0, "{id}: the seek bar's height");
        let mid = seek.center().y;
        assert_pixel(&shot, seek.min.x + seek.width() * 0.05, mid, accent, "the played part");
        assert_pixel(&shot, seek.min.x + seek.width() * 0.8, mid, track, "the track");
        assert_pixel(&shot, seek.min.x + seek.width() * 0.8, mid - 4.0, raised, "above the 4 px track");
        // `.clock`: the elapsed time in the text color, the rest muted.
        assert!(has_color(&shot, bounds(&view, "player_clock_now"), text), "{id}: the elapsed time");
        let clock = bounds(&view, "player_clock");
        let rest = remedy_ui_core::Rect::new(
            bounds(&view, "player_clock_now").max.x,
            clock.min.y,
            clock.max.x,
            clock.max.y,
        );
        assert!(has_color(&shot, rest, muted), "{id}: the duration muted");
        // `.stage-play`: 4.25 rem across.
        let big = bounds(&view, "player_big_play");
        assert_eq!((big.width().round(), big.height().round()), (68.0, 68.0), "{id}: the big Play");
    }
}

#[test]
fn the_seek_bar_grows_under_the_pointer_and_full_screen_takes_portals_sizes() {
    let (shot, view) = render("portal-portal-hover-bar", "portal", (730.0, 480.0), Look::HoverBar);
    let seek = bounds(&view, "player_seek");
    // `.seek-bar:hover`: a 6 px track.
    let x = seek.min.x + seek.width() * 0.8;
    assert_pixel(&shot, x, seek.center().y - 2.5, "#2c281f", "the 6 px track under the pointer");
    assert!(find(&view, "player_tip").info().is_some_and(|i| i.text.is_some()), "the time under the pointer");
    let (_, view) = render("portal-portal-full", "portal", (1280.0, 720.0), Look::Full);
    // `.is-full .icon-btn`: 2.7 rem.
    let play = bounds(&view, "player_button");
    assert_eq!(play.width().round(), 43.0, "full screen's buttons");
    // `.is-full > .chrome`: no surface under the bar; `.stage-top`: the heading face at 1.3 rem.
    let bar = find(&view, "player_bar").info().unwrap_or_else(|| unreachable!());
    assert_eq!((bar.variant, bar.style.background.a), ("full", 0.0), "the bar over the picture");
    let title = find(&view, "player_title_text").info().unwrap_or_else(|| unreachable!());
    assert!(title.style.heading && (title.style.text_size - 20.8).abs() < 0.01, "{:?}", title.style);
}

#[test]
fn up_next_reads_as_portals_card_with_a_spaced_label() {
    let (shot, view) = render("portal-portal-full-next-check", "portal", (1280.0, 720.0), Look::FullNext);
    // `.upnext-label`: UP NEXT at 0.75 rem, 0.08 em apart, over the title.
    let label = find(&view, "player_next_label");
    let info = label.info().unwrap_or_else(|| unreachable!());
    // Drawn in capitals; its text (what is read and copied) as written.
    assert_eq!(info.text.as_deref(), Some("Up next"));
    assert_eq!(info.style.text_transform, remedy_ui_core::style::TextTransform::Upper);
    assert_eq!(info.style.letter_spacing, remedy_ui_core::style::LetterSpacing::Em(0.08));
    // Laid out with the label's look, and without its spacing: seven clusters, 0.96 px each.
    let ui = view.ui().clone();
    let words = |spaced: bool| {
        let look = remedy_ui_core::style::StylePatch::default().text_size(12.0);
        let look =
            if spaced { look.letter_spacing(remedy_ui_core::style::LetterSpacing::Em(0.08)) } else { look };
        let el = ui.el("label").nowrap().style(look).text("UP NEXT").layout(|l| {
            l.position = remedy_ui_core::layout::Position::Absolute;
            l.inset = remedy_ui_core::Insets {
                top: 0.0,
                start: 0.0,
                end: remedy_ui_core::layout::AUTO,
                bottom: remedy_ui_core::layout::AUTO,
            };
        });
        let _ = ui.root().child(el.clone());
        el
    };
    let (a, b) = (words(true), words(false));
    let _ = ui.frame(Size::new(1280.0, 720.0), 1.0);
    let width = |e: &El| e.bounds().map_or(0.0, |r| r.width());
    let (spaced, unspaced) = (width(&a), width(&b));
    assert!((spaced - unspaced - 7.0 * 0.96).abs() < 1.0, "spaced {spaced}, plain {unspaced}");
    let title = bounds(&view, "player_next_title");
    assert!(title.min.y >= bounds(&view, "player_next_label").max.y, "the title under the label");
    // `#dock-upnext-play`: the accent; and an X named Dismiss beside it.
    let now = bounds(&view, "player_next_actions");
    assert!(has_color(&shot, now, "#e2b15a"), "Play now in the accent");
    let named = |e: &El| e.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
    let mut stack = vec![view.clone()];
    let mut dismiss = false;
    while let Some(e) = stack.pop() {
        dismiss |= named(&e).as_deref() == Some("Dismiss");
        stack.extend(e.child_elements());
    }
    assert!(dismiss, "an X named Dismiss");
}

#[test]
fn a_phone_sized_page_takes_portals_narrow_sizes() {
    // `@media (max-width: 720px)`: `.icon-btn` 2.1 rem, `.clock` 0.8 rem; the bar fits 390 px.
    let mut win = Window::new("portal", Size::new(390.0, 844.0));
    let (p, view) = portal_player(&mut win, Look::Paused);
    let play = find(&view, "player_button");
    let b = play.bounds().unwrap_or_else(|| unreachable!());
    assert_eq!((b.width().round(), play.info().map(|i| i.variant)), (34.0, Some("narrow")));
    let clock = find(&view, "player_clock").info().unwrap_or_else(|| unreachable!());
    assert!((clock.style.text_size - 12.8).abs() < 0.01, "{:?}", clock.style.text_size);
    let mut stack = vec![view.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == "player_button" && !i.hidden) {
            let r = e.bounds().unwrap_or_default();
            assert!(r.max.x <= 390.5, "every button inside the page: {r:?}");
        }
        stack.extend(e.child_elements());
    }
    // `.dock.mini`: the picture 7 rem wide here, as high as its shape makes it.
    p.set_compact(true);
    win.pump();
    win.pump();
    let v = p.info().and_then(|i| i.video).unwrap_or_else(|| unreachable!("a picture"));
    let stage = bounds(&view, "player_stage");
    let high = 112.0 * v.height as f32 / v.width as f32;
    assert_eq!(stage.width().round(), 112.0);
    assert!((stage.height() - high).abs() < 1.0, "{stage:?}, want {high} high");
    // Wider than 720 px: the normal sizes, and an 8.5 rem picture.
    let mut win = Window::new("portal", Size::new(1280.0, 720.0));
    let (p, view) = portal_player(&mut win, Look::Paused);
    assert_eq!(find(&view, "player_button").info().map(|i| i.variant), Some(""));
    p.set_compact(true);
    win.pump();
    win.pump();
    assert_eq!(bounds(&view, "player_stage").width().round(), 136.0);
}

#[test]
fn up_next_can_float_in_the_pictures_corner() {
    // Portal's `.upnext`: over the picture's bottom end corner, 20 rem wide, 0.8 rem in; the
    // picture keeps its size.
    let mut win = Window::new("portal", Size::new(1280.0, 720.0));
    let (p, view) = portal_player_with(&mut win, Look::Playing, NextUpPlacement::Corner);
    let before = bounds(&view, "player_stage");
    p.set_next_up(Some(NextUp::later("Episode 2")));
    p.seek(Duration::from_secs(170));
    win.wait("the card", || find_shown(&view, "player_next").is_some());
    let stage = bounds(&view, "player_stage");
    let card = bounds(&view, "player_next");
    assert_eq!(stage, before, "the picture keeps its size");
    assert!((card.width() - 320.0).abs() < 0.5, "{card:?}");
    assert!((stage.max.x - card.max.x - 12.8).abs() < 0.5, "{card:?} in {stage:?}");
    assert!((stage.max.y - card.max.y - 12.8).abs() < 1.0, "{card:?} over {stage:?}");
    let look = find(&view, "player_next").info().unwrap_or_else(|| unreachable!());
    assert!(look.style.radius == 12.0 && look.style.backdrop_blur == 10.0, "a raised panel");
}

/// The first shown element named `widget`, if any.
fn find_shown(root: &El, widget: &str) -> Option<El> {
    let mut stack = vec![root.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == widget && !i.hidden)
            && e.bounds().is_some_and(|b| b.width() > 0.0)
        {
            return Some(e);
        }
        stack.extend(e.child_elements());
    }
    None
}

/// Pictures of every state for comparing with Portal's page side by side.
#[test]
#[ignore = "writes pictures for review"]
fn portal_state_pictures() {
    if std::env::var("REMEDY_SHOTS").is_err() {
        return;
    }
    let _ = render("portal-portal-playing", "portal", (730.0, 480.0), Look::Playing);
    let _ = render("portal-portal-theater", "portal", (1280.0, 600.0), Look::Theater);
    let _ = render("portal-portal-narrow-390", "portal", (370.0, 280.0), Look::Paused);
    let _ = render("portal-portal-full-next", "portal", (1280.0, 720.0), Look::FullNext);
    let _ = render("portal-portal-captions", "portal", (730.0, 480.0), Look::Captions);
    let (_, view) = render("portal-portal-hover-volume", "portal", (730.0, 480.0), Look::HoverVolume);
    // `.volume:hover .volume-bar`: 5.25 rem, with its margins.
    assert_eq!(bounds(&view, "player_volume_holder").width().round(), 86.0, "the volume shows on hover");
}
