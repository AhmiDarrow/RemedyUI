//! Pictures of the player's bar for review, drawn by the developer tools' preview renderer.
//! Run with `REMEDY_SHOTS=<dir> cargo test -p remedy-ui-media-playback --test screens -- --ignored`.

use remedy_ui_widgets::devtools::Preview;
use remedy_ui_core::input::{Key, KeyEvent};
use remedy_ui_media_playback::*;
use remedy_ui_engine::view::{El, Ui};

fn shot(name: &str, build: impl Fn(&Ui) -> El + 'static) {
    let Ok(dir) = std::env::var("REMEDY_SHOTS") else { return };
    let preview = Preview::new(name, build).sizes(&[(640.0, 400.0)]);
    let v = preview.variations().into_iter().next().unwrap_or_else(|| unreachable!("one variation"));
    let png = preview.render(&v).to_png().unwrap_or_else(|e| unreachable!("{e:?}"));
    let path = std::path::Path::new(&dir).join(format!("{name}.png"));
    std::fs::write(path, png).unwrap_or_else(|e| unreachable!("{e}"));
}

/// A player as Portal shows one: two rows, skips, no speed, the app offering Auto, an
/// embedded English track and an online choice.
fn portal(ui: &Ui) -> Player {
    let options = PlayerOptions::default().two_rows(true).skip_buttons(true).speed_button(false);
    let p = Player::empty_platform(ui, options);
    p.on_caption_pick(|_| {});
    p.add_subtitles(SubtitleTrack { label: "English".into(), language: Some("en".into()), cues: Vec::new() });
    p.set_caption_choices(vec![
        CaptionChoice::new("os:1", "English · 720p.BluRay").detail("Hearing impaired · Best match"),
    ]);
    p
}

/// Opens the captions menu once the player is laid out, as a press on it would.
fn open_menu(ui: &Ui, view: El) -> El {
    let u = ui.clone();
    let v = view.clone();
    let hook = ui.el("box").width(1.0).height(1.0).on_visibility(move |seen| {
        let mut stack = vec![v.clone()];
        while let Some(e) = stack.pop() {
            let named = e.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
            if e.info().is_some_and(|i| i.widget == "button")
                && named.is_some_and(|n| n.starts_with("Captions"))
            {
                if seen {
                    e.focus();
                    let _ = u.key(&KeyEvent::press(Key::Enter).with_text(None));
                }
                return;
            }
            stack.extend(e.child_elements());
        }
    });
    ui.el("box").column().grow(1.0).child(view).child(hook)
}

#[test]
#[ignore = "writes pictures for review"]
fn captions_control_pictures() {
    shot("after-auto", |ui| {
        let p = portal(ui);
        p.set_caption_pick(CaptionPick::Auto);
        p.set_caption_track(Some(0));
        player(ui, &p, "Episode")
    });
    shot("after-loading", |ui| {
        let p = portal(ui);
        p.set_caption_pick(CaptionPick::Choice("os:1".into()));
        p.set_captions_loading(true);
        player(ui, &p, "Episode")
    });
    shot("after-menu", |ui| {
        let p = portal(ui);
        p.set_caption_pick(CaptionPick::Auto);
        p.set_caption_track(Some(0));
        p.set_caption_delay(500);
        p.set_caption_note(Some("Searching online…".into()));
        let view = player(ui, &p, "Episode");
        open_menu(ui, view)
    });
}

/// Full screen with a title and a caption: the bar over the picture's foot, and rested.
#[test]
#[ignore = "writes pictures for review"]
fn full_screen_pictures() {
    let make = |ui: &Ui| {
        let p = portal(ui);
        let vtt =
            "WEBVTT\n\n00:00.000 --> 01:00.000\nThe captions sit above the bar,\ncentred line by line\n";
        let cues = parse_subtitles(vtt, SubtitleFormat::WebVtt);
        p.set_caption_track(Some(p.add_subtitles(SubtitleTrack {
            label: "Shown".into(),
            language: None,
            cues,
        })));
        p.set_title(Some("Episode 4 · The Long Night".into()));
        p.request_fullscreen(true);
        p
    };
    shot("full-screen-shown", move |ui| {
        let p = make(ui);
        player(ui, &p, "Episode")
    });
    // The last 20 seconds of a 1:40 work (a timeline 1:30 in): "Up next" over the picture.
    shot("full-screen-up-next", move |ui| {
        let options = PlayerOptions::default()
            .two_rows(true)
            .skip_buttons(true)
            .speed_button(false)
            .next_up_lead(Some(std::time::Duration::from_secs(20)));
        let p = Player::empty_platform(ui, options);
        p.set_timeline(Some(
            Timeline::new(std::time::Duration::from_secs(90)).length(std::time::Duration::from_secs(100)),
        ));
        p.set_next_up(Some(NextUp::new(MediaSource::url("next:"), "Episode 5 · The Morning After")));
        p.set_title(Some("Episode 4 · The Long Night".into()));
        p.request_fullscreen(true);
        player(ui, &p, "Episode")
    });
    shot("full-screen-rested", move |ui| {
        let p = make(ui);
        let view = player(ui, &p, "Episode");
        p.rest_controls();
        view
    });
}

/// A picture `aspect` wide per unit high that never sends a frame, so the stage is black
/// and letterboxed as the picture's shape says.
struct Still(f32);

struct StillDecoder(MediaInfo);

impl Decoder for StillDecoder {
    fn info(&self) -> &MediaInfo {
        &self.0
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        Ok(None)
    }
    fn seek(&mut self, _: std::time::Duration) -> Result<(), MediaError> {
        Ok(())
    }
}

impl MediaHost for Still {
    fn name(&self) -> &str {
        "still"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let mut info = MediaInfo::default();
        info.duration = Some(std::time::Duration::from_secs(60));
        info.video = Some(VideoInfo::new((self.0 * 300.0) as u32, 300, None));
        Ok(Box::new(StillDecoder(info)))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("none".into()))
    }
}

/// A rendered player showing `cues` (all at once): the picture, and the stage's bounds.
struct Shot {
    snap: remedy_ui_widgets::devtools::Snapshot,
    stage: remedy_ui_core::Rect,
}

/// Renders a 640 by 400 player showing `cues`, over a picture `aspect` wide per unit high
/// (none: no picture), full screen with its bar shown when `full`; written as `name`.png
/// when `REMEDY_SHOTS` is set.
fn cue_shot(name: &str, cues: Vec<Cue>, aspect: Option<f32>, full: bool) -> Shot {
    let seen: std::rc::Rc<std::cell::RefCell<Option<El>>> = Default::default();
    let s = seen.clone();
    let preview = Preview::new(name, move |ui| {
        let p = match aspect {
            Some(a) => {
                let p = Player::open(
                    ui,
                    std::sync::Arc::new(Still(a)),
                    MediaSource::url("still:"),
                    PlayerOptions::default(),
                );
                let start = std::time::Instant::now();
                while p.info().is_none() {
                    let _ = remedy_ui_core::reactive::run_posted();
                    assert!(start.elapsed() < std::time::Duration::from_secs(20), "the still opens");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                p
            }
            None => Player::empty_platform(ui, PlayerOptions::default()),
        };
        let track = SubtitleTrack { label: "Placed".into(), language: None, cues: cues.clone() };
        p.set_caption_track(Some(p.add_subtitles(track)));
        if full {
            p.request_fullscreen(true);
        }
        let view = player(ui, &p, "Episode");
        *s.borrow_mut() = Some(view.clone());
        view
    })
    .sizes(&[(640.0, 400.0)]);
    let v = preview.variations().into_iter().next().unwrap_or_else(|| unreachable!("one variation"));
    let snap = preview.render(&v);
    if let Ok(dir) = std::env::var("REMEDY_SHOTS") {
        let png = snap.to_png().unwrap_or_else(|e| unreachable!("{e:?}"));
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png)
            .unwrap_or_else(|e| unreachable!("{e}"));
    }
    let root = seen.borrow().clone().unwrap_or_else(|| unreachable!("built"));
    let mut stack = vec![root];
    let mut stage = None;
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == "player_cues") {
            stage = e.bounds();
        }
        stack.extend(e.child_elements());
    }
    Shot { snap, stage: stage.unwrap_or_else(|| unreachable!("the cues' layer")) }
}

impl Shot {
    /// The box around the caption text (white pixels) within `x0..x1` by `y0..y1`, as
    /// fractions of the stage.
    fn text(&self, x: (f32, f32), y: (f32, f32)) -> Option<(f32, f32, f32, f32)> {
        let s = self.stage;
        let (w, h) = (s.width(), s.height());
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for py in (s.min.y + y.0 * h) as u32..(s.min.y + y.1 * h) as u32 {
            for px in (s.min.x + x.0 * w) as u32..(s.min.x + x.1 * w) as u32 {
                if self.snap.pixel(px, py).is_some_and(|p| p[0] > 200 && p[1] > 200 && p[2] > 200) {
                    let (fx, fy) = ((px as f32 - s.min.x) / w, (py as f32 - s.min.y) / h);
                    lo = (lo.0.min(fx), lo.1.min(fy));
                    hi = (hi.0.max(fx), hi.1.max(fy));
                }
            }
        }
        (lo.0 <= hi.0).then_some((lo.0, lo.1, hi.0, hi.1))
    }

    /// The bands of rows holding caption text within `x0..x1`, top to bottom, as fractions.
    fn bands(&self, x: (f32, f32), y: (f32, f32)) -> Vec<(f32, f32)> {
        let s = self.stage;
        let h = s.height();
        let mut out: Vec<(f32, f32)> = Vec::new();
        let mut gap = 0;
        for py in (s.min.y + y.0 * h) as u32..(s.min.y + y.1 * h) as u32 {
            let lit = ((s.min.x + x.0 * s.width()) as u32..(s.min.x + x.1 * s.width()) as u32)
                .any(|px| self.snap.pixel(px, py).is_some_and(|p| p[0] > 200 && p[1] > 200 && p[2] > 200));
            let fy = (py as f32 - s.min.y) / h;
            match (lit, out.last_mut()) {
                (true, Some(b)) if gap < 6 => {
                    b.1 = fy;
                    gap = 0;
                }
                (true, _) => {
                    out.push((fy, fy));
                    gap = 0;
                }
                (false, _) => gap += 1,
            }
        }
        out
    }
}

fn cues_of(text: &str, format: SubtitleFormat) -> Vec<Cue> {
    parse_subtitles(text, format)
}

/// Cues with settings of their own where they say: WebVTT's `line:0 align:start` at the top
/// left, ASS's `\an6` at the middle right, and one with none at the foot; all on the
/// picture (no picture here: the whole stage).
#[test]
fn cue_settings_place_the_captions() {
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 line:0 align:start\nTop left\n\n00:00.000 --> 01:00.000\nAt the foot, centred\n";
    let ass = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:01:00.00,Default,,0,0,0,,{\\an6}Middle right\n";
    let mut cues = cues_of(vtt, SubtitleFormat::WebVtt);
    cues.extend(cues_of(ass, SubtitleFormat::Ass));
    let shot = cue_shot("cue-settings", cues, None, false);
    let top_left = shot.text((0.0, 0.3), (0.0, 0.2));
    assert!(top_left.is_some_and(|t| t.0 < 0.05 && t.1 < 0.1), "{top_left:?}");
    let right = shot.text((0.7, 1.0), (0.3, 0.7));
    assert!(right.is_some_and(|t| t.2 > 0.95 && (0.4..0.6).contains(&((t.1 + t.3) / 2.0))), "{right:?}");
    let foot = shot.text((0.2, 0.8), (0.7, 1.0));
    assert!(foot.is_some_and(|t| ((t.0 + t.2) / 2.0 - 0.5).abs() < 0.03), "{foot:?}");
}

/// Two speakers at once: two bottom ASS lines of one style are stacked, the later above;
/// two WebVTT cues at the top line stack downward; none overlaps.
#[test]
fn overlapping_cues_are_moved_apart() {
    let ass = "[Script Info]\nPlayResY: 400\n\n[V4+ Styles]\nFormat: Name, Alignment, MarginL, MarginR, MarginV\nStyle: Default,2,0,0,20\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:01:00.00,Default,,0,0,0,,- Where were you?\nDialogue: 0,0:00:00.00,0:01:00.00,Default,,0,0,0,,- Out.\n";
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 line:0 position:0% size:30% align:left\nOne\n\n00:00.000 --> 01:00.000 line:0 position:0% size:30% align:left\nTwo\n";
    let mut cues = cues_of(ass, SubtitleFormat::Ass);
    cues.extend(cues_of(vtt, SubtitleFormat::WebVtt));
    let shot = cue_shot("cue-overlap", cues, None, false);
    let foot = shot.bands((0.3, 0.7), (0.5, 1.0));
    assert_eq!(foot.len(), 2, "two lines at the foot, apart: {foot:?}");
    let top = shot.bands((0.0, 0.3), (0.0, 0.4));
    assert_eq!(top.len(), 2, "two cues at the top line, apart: {top:?}");
}

/// The picture is 4:3 in a wider stage: cues are placed on it, not on the bars beside it.
#[test]
fn cues_sit_on_the_letterboxed_picture() {
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 line:0 align:left\nOn the picture\n\n00:00.000 --> 01:00.000 line:0 align:right\nRight edge\n";
    let shot = cue_shot("cue-viewport", cues_of(vtt, SubtitleFormat::WebVtt), Some(4.0 / 3.0), false);
    let (w, h) = (shot.stage.width(), shot.stage.height());
    // The picture's left and right edges, as fractions of the stage.
    let pic = (w - h * 4.0 / 3.0) / 2.0 / w;
    let left = shot.text((0.0, 0.5), (0.0, 0.2));
    assert!(left.is_some_and(|t| t.0 > pic && t.0 < pic + 0.05), "{left:?} against the picture from {pic}");
    let right = shot.text((0.5, 1.0), (0.0, 0.2));
    assert!(right.is_some_and(|t| t.2 < 1.0 - pic && t.2 > 1.0 - pic - 0.05), "{right:?}");
}

/// A percentage line places the cue's start, middle or end there (`line:50%,start|center|end`).
#[test]
fn a_percentage_line_uses_its_alignment() {
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 line:50%,start position:0% size:30% align:left\nStart\n\n00:00.000 --> 01:00.000 line:50%,center position:35% size:30% align:left\nCenter\n\n00:00.000 --> 01:00.000 line:50%,end position:70% size:30% align:left\nEnd\n";
    let shot = cue_shot("cue-line-align", cues_of(vtt, SubtitleFormat::WebVtt), None, false);
    let mid = |x: (f32, f32)| shot.text(x, (0.0, 1.0)).map(|t| (t.1 + t.3) / 2.0);
    let (s, c, e) = (mid((0.0, 0.3)), mid((0.35, 0.65)), mid((0.7, 1.0)));
    assert!(
        s.is_some_and(|v| v > 0.5) && c.is_some_and(|v| (v - 0.5).abs() < 0.03) && e.is_some_and(|v| v < 0.5),
        "{s:?} {c:?} {e:?}"
    );
}

/// A bottom ASS line sits its MarginV above the picture's foot; in full screen with the bar
/// over the foot, a line with no margin is lifted above the bar.
#[test]
fn bottom_ass_lines_use_margin_v_and_clear_the_bar() {
    let ass = |mv: u32| {
        format!(
            "[Script Info]\nPlayResY: 400\n\n[V4+ Styles]\nFormat: Name, Alignment, MarginL, MarginR, MarginV\nStyle: Default,1,0,0,{mv}\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:01:00.00,Default,,0,0,0,,Bottom left\n"
        )
    };
    let shot = cue_shot("cue-ass-margin", cues_of(&ass(100), SubtitleFormat::Ass), None, false);
    // 100 of 400: the box's foot a quarter up.
    let t = shot.text((0.0, 0.5), (0.0, 1.0));
    assert!(t.is_some_and(|t| t.3 < 0.75 && t.3 > 0.68), "{t:?}");
    let shot = cue_shot("cue-ass-full-screen", cues_of(&ass(0), SubtitleFormat::Ass), None, true);
    // The bar floats over the stage's foot: its top is where the first row of it shows.
    let t = shot.text((0.0, 0.4), (0.0, 0.8));
    assert!(t.is_some_and(|t| t.3 < 0.8), "above the bar: {t:?}");
}

/// `vertical:rl` lines read downward at the right edge; `lr` at the left.
#[test]
fn vertical_cues_are_laid_out_vertically() {
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 vertical:rl line:0\nDownward on the right\n\n00:00.000 --> 01:00.000 vertical:lr line:0 align:start\nLeft side\n";
    let shot = cue_shot("cue-vertical", cues_of(vtt, SubtitleFormat::WebVtt), None, false);
    let rl = shot.text((0.85, 1.0), (0.0, 1.0));
    assert!(
        rl.is_some_and(
            |t| t.2 > 0.95 && (t.3 - t.1) * shot.stage.height() > 3.0 * (t.2 - t.0) * shot.stage.width()
        ),
        "{rl:?}"
    );
    let lr = shot.text((0.0, 0.15), (0.0, 1.0));
    assert!(
        lr.is_some_and(|t| t.0 < 0.05
            && t.1 < 0.05
            && (t.3 - t.1) * shot.stage.height() > 2.0 * (t.2 - t.0) * shot.stage.width()),
        "{lr:?}"
    );
}

/// A mixed vertical line (CSS's `text-orientation: mixed`): the Japanese upright, one
/// character under another, the Latin turned to read downward, all in one tall column.
#[test]
fn mixed_vertical_lines_set_cjk_upright_and_latin_sideways() {
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 vertical:rl line:0\n字幕とcaptions\n";
    let shot = cue_shot("cue-vertical-mixed", cues_of(vtt, SubtitleFormat::WebVtt), None, false);
    let t = shot.text((0.85, 1.0), (0.0, 1.0));
    let (w, h) = (shot.stage.width(), shot.stage.height());
    assert!(t.is_some_and(|t| t.2 > 0.95 && (t.3 - t.1) * h > 4.0 * (t.2 - t.0) * w), "{t:?}");
}
