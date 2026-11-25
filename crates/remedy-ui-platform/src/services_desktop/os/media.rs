//! The System Media Transport Controls, through a `MediaPlayer` with its own command handling
//! off (so a Win32 app needs no window handle and no package identity).

use std::sync::Arc;
use std::time::Duration;

use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::Playback::MediaPlayer;
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton as Button,
    SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference};
use windows::core::{HSTRING, Ref};

use crate::services_desktop::DesktopError;
use crate::services_desktop::media_session::{Artwork, Handler, MediaCommand, MediaState, NowPlaying};

/// Seconds the Rewind and Fast-forward buttons move.
const SKIP: f64 = 10.0;

pub(crate) struct Session {
    _player: MediaPlayer,
    smtc: SystemMediaTransportControls,
    pressed: i64,
    moved: i64,
}

fn span(d: Duration) -> TimeSpan {
    TimeSpan { Duration: i64::try_from(d.as_nanos() / 100).unwrap_or(i64::MAX) }
}

/// What a button means, or `None` for one the session does not offer.
pub(crate) fn command(b: Button) -> Option<MediaCommand> {
    Some(match b {
        Button::Play => MediaCommand::Play,
        Button::Pause => MediaCommand::Pause,
        Button::Stop => MediaCommand::Stop,
        Button::Next => MediaCommand::Next,
        Button::Previous => MediaCommand::Previous,
        Button::FastForward => MediaCommand::SeekBy(SKIP),
        Button::Rewind => MediaCommand::SeekBy(-SKIP),
        _ => return None,
    })
}

impl Session {
    pub(crate) fn new(handler: Handler) -> Result<Session, DesktopError> {
        let e = |e: windows::core::Error| DesktopError::unsupported("Media controls", e.message());
        let player = MediaPlayer::new().map_err(e)?;
        player.CommandManager().and_then(|m| m.SetIsEnabled(false)).map_err(e)?;
        let smtc = player.SystemMediaTransportControls().map_err(e)?;
        let on = |r: windows::core::Result<()>| r.map_err(e);
        on(smtc.SetIsPlayEnabled(true))?;
        on(smtc.SetIsPauseEnabled(true))?;
        on(smtc.SetIsStopEnabled(true))?;
        on(smtc.SetIsPreviousEnabled(true))?;
        on(smtc.SetIsFastForwardEnabled(true))?;
        on(smtc.SetIsRewindEnabled(true))?;
        on(smtc.SetIsEnabled(false))?;
        let handler = Arc::new(handler);
        let h = handler.clone();
        let pressed = smtc
            .ButtonPressed(&TypedEventHandler::new(
                move |_, args: Ref<'_, SystemMediaTransportControlsButtonPressedEventArgs>| {
                    if let Some(c) = args.ok().and_then(|a| a.Button()).ok().and_then(command) {
                        h(c);
                    }
                    Ok(())
                },
            ))
            .map_err(e)?;
        let moved = smtc
            .PlaybackPositionChangeRequested(&TypedEventHandler::new(
                move |_, args: Ref<'_, PlaybackPositionChangeRequestedEventArgs>| {
                    if let Ok(t) = args.ok().and_then(|a| a.RequestedPlaybackPosition()) {
                        let ns = u64::try_from(t.Duration).unwrap_or(0).saturating_mul(100);
                        handler(MediaCommand::SeekTo(Duration::from_nanos(ns)));
                    }
                    Ok(())
                },
            ))
            .map_err(e)?;
        Ok(Session { _player: player, smtc, pressed, moved })
    }

    pub(crate) fn now_playing(&self, now: Option<&NowPlaying>) {
        let _ = self.show(now);
    }

    fn show(&self, now: Option<&NowPlaying>) -> windows::core::Result<()> {
        let d = self.smtc.DisplayUpdater()?;
        d.ClearAll()?;
        let Some(now) = now else {
            d.Update()?;
            return self.smtc.SetIsEnabled(false);
        };
        d.SetType(MediaPlaybackType::Video)?;
        let v = d.VideoProperties()?;
        v.SetTitle(&HSTRING::from(now.title.as_str()))?;
        if let Some(s) = &now.subtitle {
            v.SetSubtitle(&HSTRING::from(s.as_str()))?;
        }
        // A picture that cannot be used (an address Windows does not parse) is left out: the
        // title still shows.
        let _ = (|| -> windows::core::Result<()> {
            match &now.art {
                Some(Artwork::Url(u)) => {
                    let uri = Uri::CreateUri(&HSTRING::from(u.as_str()))?;
                    d.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&uri)?)
                }
                Some(Artwork::Bytes(b)) => {
                    let s = InMemoryRandomAccessStream::new()?;
                    let w = DataWriter::CreateDataWriter(&s)?;
                    w.WriteBytes(b)?;
                    // An in-memory store: done at once, so waiting on it here does not stall.
                    w.StoreAsync()?.join()?;
                    w.DetachStream()?;
                    s.Seek(0)?;
                    d.SetThumbnail(&RandomAccessStreamReference::CreateFromStream(&s)?)
                }
                None => Ok(()),
            }
        })();
        d.Update()?;
        self.smtc.SetIsEnabled(true)
    }

    pub(crate) fn playback(&self, state: MediaState, position: Duration, length: Option<Duration>) {
        let status = match state {
            MediaState::Playing => MediaPlaybackStatus::Playing,
            MediaState::Paused => MediaPlaybackStatus::Paused,
            MediaState::Stopped => MediaPlaybackStatus::Stopped,
        };
        let _ = self.smtc.SetPlaybackStatus(status);
        let _ = (|| -> windows::core::Result<()> {
            let t = SystemMediaTransportControlsTimelineProperties::new()?;
            let end = length.unwrap_or(position);
            t.SetStartTime(span(Duration::ZERO))?;
            t.SetMinSeekTime(span(Duration::ZERO))?;
            t.SetEndTime(span(end))?;
            t.SetMaxSeekTime(span(if length.is_some() { end } else { Duration::ZERO }))?;
            t.SetPosition(span(position.min(end)))?;
            self.smtc.UpdateTimelineProperties(&t)
        })();
    }

    pub(crate) fn can_next(&self, can: bool) {
        let _ = self.smtc.SetIsNextEnabled(can);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.smtc.RemoveButtonPressed(self.pressed);
        let _ = self.smtc.RemovePlaybackPositionChangeRequested(self.moved);
        let _ = self.smtc.SetIsEnabled(false);
    }
}
