# The media player's own words. Placeholders ({ $name }) must stay; their order may change.
# The letters in brackets in tooltips (K, M, C, F, T) are the player's keys: they stay as they
# are in every language.

## Shown over the picture when the media cannot be played. { $detail } is the system's own
## explanation, in English (a file name, a decoder's message, a package to install).
playback-failed = This can't be played.
playback-failed-unsupported = This can't be played here: { $detail }
playback-failed-not-found = This can't be played because it wasn't found: { $detail }
playback-failed-format = This can't be played because it isn't media this player understands: { $detail }
playback-failed-decode = This can't be played because decoding failed: { $detail }
playback-failed-device = This can't be played because of a problem with the sound device: { $detail }
playback-failed-denied = This can't be played because access wasn't allowed: { $detail }
# A system component is missing; { $detail } says which and how to install it.
playback-failed-missing = This can't be played: { $detail }
# While the media opens or waits for data.
playback-loading = Loading

## The transport bar. Names are read by screen readers; tips show on hover.
playback-play-pause-tip = Play or pause (K)
playback-play = Play
playback-pause = Pause
# The time shown, as "1:05 / 3:20". Times are written m:ss or h:mm:ss.
playback-time = { $position } / { $duration }
# The time shown for a live stream, which has no end.
playback-time-live = { $position } · Live
playback-skipping = Skipping to { $position }
playback-preparing = Preparing the video.
playback-mute-tip = Mute (M)
playback-mute = Mute
playback-unmute = Unmute
playback-volume = Volume
# While casting, as Portal's page says it: the device's volume and mute.
playback-volume-tv = TV volume
playback-mute-tv = Mute the TV
playback-unmute-tv = Unmute the TV
playback-captions-tip = Captions (C)
# { $track } is the caption track's name ("English", "fr").
playback-captions-track = Captions: { $track }
playback-captions-off = Captions: off
# The captions button: its name, the words on it, and its menu ({ $track } is what shows).
playback-captions-loading = Captions: loading
playback-captions-auto = Captions: auto
playback-captions-auto-track = Captions: auto, { $track }
playback-captions-short-off = Off
playback-captions-short-auto = Auto
playback-captions-short-auto-track = Auto · { $track }
playback-captions-short-loading = Loading…
playback-captions-menu-off = Off
playback-captions-menu-auto = Auto
playback-captions-menu-auto-track = Auto · showing { $track }
playback-captions-menu-loading = { $label } · loading…
# The menu's timing steps; { $timing } is one of the words below ({ $seconds } a number).
playback-captions-earlier = Earlier (−0.5 s) · { $timing }
playback-captions-later = Later (+0.5 s) · { $timing }
playback-captions-reset = Reset timing
playback-timing-in-sync = in sync
playback-timing-later = now { $seconds } s later
playback-timing-earlier = now { $seconds } s earlier
# Portal's skip buttons.
playback-back-tip = Back 10 seconds (J)
playback-back = Back 10 seconds
playback-forward-tip = Forward 10 seconds (L)
playback-forward = Forward 10 seconds
playback-full-screen-tip = Full screen (F)
playback-full-screen = Full screen
playback-exit-full-screen = Exit full screen
# Theater mode: the picture wide in the page. One name; screen readers say whether it is on.
playback-theater-tip = Theater mode (T)
playback-theater = Theater mode
# Casting: the media plays on another device (a TV) and the controls drive it. One button
# name that says whether it is casting; { $device } is the device's name ("Living room TV").
playback-cast-tip = Cast to a TV or another screen
playback-cast = Cast
playback-casting = Casting
playback-cast-connecting = Connecting to { $device }
playback-cast-playing = Playing on { $device }
playback-cast-paused = Paused on { $device }
playback-cast-buffering = Loading on { $device }
playback-cast-ended = Finished on { $device }
playback-cast-failed = Can't play on { $device }
playback-cast-next = Starting { $title } on { $device }
# The seek bar's name, and its value ("0:42 of 3:20").
playback-seek = Seek
playback-seek-value = { $position } of { $duration }

## When the media ends with something set to play next. { $title } is its name.
playback-next-up = Up next: { $title }
playback-next-up-in = { $count ->
    [one] Playing in { $count } second
   *[other] Playing in { $count } seconds
}
playback-play-now = Play now
playback-next-cancel = Cancel
# While the app starts the next (a media server makes its stream).
playback-starting-next = Starting { $title }
# Over the picture once Cancel is pressed, while the app stops its work.
playback-canceling = Canceling.
# After a failure the app can try again from ({ $message } is why it failed), as Portal's page says it.
playback-retry = { $message } Press Play to try again.

## The speed button: { $speed } is the playback speed (1, 1.5, 0.75), written the locale's way.
playback-speed = Speed { $speed }×
playback-speed-tip = Playback speed (Shift+> faster, Shift+< slower)
