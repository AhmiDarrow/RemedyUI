//! Pictures of the player's bar for review, drawn by the developer tools' preview renderer.
//! Run with `REMEDY_SHOTS=<dir> cargo test -p remedy-ui-media-playback --test screens -- --ignored`.

use remedy_ui_devtools::Preview;
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_media_playback::*;
use remedy_ui_view::{El, Ui};

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

/// Cues with settings of their own where they say: WebVTT's `line:0 align:start` at the top
/// left, ASS's `\an6` at the middle right, WebVTT's `line:50% position:50% size:40%` in the
/// middle, and one with none centred at the foot as before.
#[test]
fn cue_settings_place_the_captions() {
    let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000 line:0 align:start\nTop left\n\n00:00.000 --> 01:00.000 line:50% position:50% size:40%\nMiddle\n\n00:00.000 --> 01:00.000\nAt the foot, centred\n";
    let ass = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:01:00.00,Default,,0,0,0,,{\\an6}Middle right\n";
    let mut cues = parse_subtitles(vtt, SubtitleFormat::WebVtt);
    cues.extend(parse_subtitles(ass, SubtitleFormat::Ass));
    let seen: std::rc::Rc<std::cell::RefCell<Option<El>>> = Default::default();
    let s = seen.clone();
    let preview = Preview::new("cue-settings", move |ui| {
        let p = Player::empty_platform(ui, PlayerOptions::default());
        let track = SubtitleTrack { label: "Placed".into(), language: None, cues: cues.clone() };
        p.set_caption_track(Some(p.add_subtitles(track)));
        let view = player(ui, &p, "Episode");
        *s.borrow_mut() = Some(view.clone());
        view
    })
    .sizes(&[(640.0, 400.0)]);
    let v = preview.variations().into_iter().next().unwrap_or_else(|| unreachable!("one variation"));
    let png = preview.render(&v).to_png().unwrap_or_else(|e| unreachable!("{e:?}"));
    if let Ok(dir) = std::env::var("REMEDY_SHOTS") {
        std::fs::write(std::path::Path::new(&dir).join("cue-settings.png"), png)
            .unwrap_or_else(|e| unreachable!("{e}"));
    }
    // Every caption box drawn, with its centre.
    let root = seen.borrow().clone().unwrap_or_else(|| unreachable!("built"));
    let (mut stack, mut boxes) = (vec![root.clone()], Vec::new());
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == "player_caption" && !i.hidden)
            && let Some(b) = e.bounds()
            && b.width() > 0.0
        {
            boxes.push(b);
        }
        stack.extend(e.child_elements());
    }
    let area = root.bounds().unwrap_or_else(|| unreachable!("laid out"));
    let rel = |b: &remedy_ui_core::Rect| {
        let c = b.center();
        ((c.x - area.min.x) / area.width(), (c.y - area.min.y) / area.height(), *b)
    };
    let found: Vec<_> = boxes.iter().map(rel).collect();
    let has = |what: &str, f: &dyn Fn(f32, f32) -> bool| {
        assert!(found.iter().any(|(x, y, _)| f(*x, *y)), "{what}: {found:?}");
    };
    assert_eq!(found.len(), 4, "{found:?}");
    has("top left", &|x, y| x < 0.25 && y < 0.2);
    has("middle right", &|x, y| x > 0.75 && (0.2..0.6).contains(&y));
    has("middle", &|x, y| (0.4..0.6).contains(&x) && (0.2..0.6).contains(&y));
    // The bar sits under the picture here, so the foot's caption is in the lower part.
    has("foot, centred", &|x, y| (0.45..0.55).contains(&x) && y > 0.55);
}
