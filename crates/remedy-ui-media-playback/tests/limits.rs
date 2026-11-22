//! The app sets the bounds playback works within. Its own test binary: the bounds are the
//! process's, so no other test sees them changed.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::Duration;

use remedy_ui_media_playback::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;

struct Nothing;

impl MediaHost for Nothing {
    fn name(&self) -> &str {
        "nothing"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Err(MediaError::NotFound("nothing to open".into()))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("no device".into()))
    }
}

#[test]
fn the_subtitle_file_limit_is_the_apps() {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), FontCollection::new());
    let p = Player::empty(&ui, Arc::new(Nothing), PlayerOptions::default());
    let path = std::env::temp_dir().join(format!("remedy-limits-{}.srt", std::process::id()));
    let cue = "1\n00:00:01,000 --> 00:00:02,000\nhello\n\n";
    std::fs::write(&path, cue.repeat(100)).unwrap_or_else(|e| unreachable!("{e}"));

    assert_eq!(media_limits(), MediaLimits::default());
    assert!(p.load_subtitles(&path, "default").is_ok(), "well within 16 MB");

    set_media_limits(MediaLimits::default().max_subtitle_bytes(1024));
    assert_eq!(media_limits().max_subtitle_bytes, 1024);
    let got = p.load_subtitles(&path, "small");
    assert!(matches!(&got, Err(MediaError::Format(m)) if m.contains("over 1024 bytes")), "{got:?}");

    set_media_limits(MediaLimits::default().max_subtitle_bytes(1 << 30).hls_ahead(8));
    assert!(p.load_subtitles(&path, "raised").is_ok());
    let _ = std::fs::remove_file(&path);
    set_media_limits(MediaLimits::default());
}

#[test]
fn the_caption_step_is_the_apps() {
    let o = PlayerOptions::default();
    assert_eq!(
        (o.caption_step, o.caption_delay_limit),
        (Duration::from_millis(500), Duration::from_secs(600))
    );
    let o = o.caption_step(Duration::from_millis(100)).caption_delay_limit(Duration::from_secs(3600));
    assert_eq!(
        (o.caption_step, o.caption_delay_limit),
        (Duration::from_millis(100), Duration::from_secs(3600))
    );
}
