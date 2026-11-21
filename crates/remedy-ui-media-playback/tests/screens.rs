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
        let vtt = "WEBVTT\n\n00:00.000 --> 01:00.000\nThe captions sit above the bar\n";
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
    shot("full-screen-rested", move |ui| {
        let p = make(ui);
        let view = player(ui, &p, "Episode");
        p.rest_controls();
        view
    });
}
