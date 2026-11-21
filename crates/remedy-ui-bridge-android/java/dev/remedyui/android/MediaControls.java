// The system's media controls for remedy-ui-services-desktop's MediaSession: an Android
// MediaSession (the lock screen, the quick settings' media player, headsets, Bluetooth and the
// media keys) and its media notification. Rust binds `command` when it loads this class
// through the app's class loader, so the APK must carry the bridge's dex (cargo remedy package
// does).

package dev.remedyui.android;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.WindowManager;

public final class MediaControls extends MediaSession.Callback {
    // The commands, as Rust reads them; SEEK_BY and SEEK_TO carry milliseconds.
    static final int PLAY = 0;
    static final int PAUSE = 1;
    static final int STOP = 3;
    static final int NEXT = 4;
    static final int PREVIOUS = 5;
    static final int SEEK_BY = 6;
    static final int SEEK_TO = 7;

    /** Milliseconds the fast-forward and rewind buttons move. */
    private static final long SKIP = 10_000;
    /** The media notification's number (beside the foreground one and the tagged ones). */
    private static final int NOTIFICATION = 0x52454d5a;

    private static native void command(long handle, int what, long arg);

    /**
     * Keeps the activity's screen on while video plays (FLAG_KEEP_SCREEN_ON), or lets it turn
     * off again: on the main thread, where the window takes its flags. Throws when the context
     * is no activity (there is no window to keep on).
     */
    static void keepScreenOn(Context context, boolean on) {
        if (!(context instanceof Activity)) {
            throw new IllegalStateException("the app's context is not an activity: no window to keep on");
        }
        Activity activity = (Activity) context;
        activity.runOnUiThread(() -> {
            int flag = WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON;
            if (on) {
                activity.getWindow().addFlags(flag);
            } else {
                activity.getWindow().clearFlags(flag);
            }
        });
    }

    private final Context context;
    private final long handle;
    private final MediaSession session;
    private final NotificationManager manager;
    private boolean canNext;
    private int state = PlaybackState.STATE_NONE;
    private long position;
    private long length = -1;
    private String title;
    private String subtitle;
    private String artUrl;
    private Bitmap art;

    private MediaControls(Context context, long handle) {
        this.context = context.getApplicationContext();
        this.handle = handle;
        session = new MediaSession(this.context, "RemedyUI");
        // The callbacks come on the main thread; Rust's handler is fine on any thread.
        session.setCallback(this, new Handler(Looper.getMainLooper()));
        manager = this.context.getSystemService(NotificationManager.class);
        NotificationChannel quiet =
                new NotificationChannel(Notifier.CHANNEL_QUIET, "Quiet notifications", NotificationManager.IMPORTANCE_LOW);
        quiet.setSound(null, null);
        quiet.enableVibration(false);
        manager.createNotificationChannel(quiet);
        playback(0, 0, -1);
    }

    static MediaControls open(Context context, long handle) {
        return new MediaControls(context, handle);
    }

    /** Shows what plays (`title` null: nothing; `artUrl` and `artBytes` may be null). */
    synchronized void metadata(String title, String subtitle, String artUrl, byte[] artBytes) {
        this.title = title;
        this.subtitle = subtitle;
        this.artUrl = artUrl;
        art = artBytes == null ? null : BitmapFactory.decodeByteArray(artBytes, 0, artBytes.length);
        if (title == null) {
            session.setMetadata(null);
            session.setActive(false);
            manager.cancel(NOTIFICATION);
            return;
        }
        publish();
        session.setActive(true);
        notifyNow();
    }

    private void publish() {
        MediaMetadata.Builder m = new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE, title);
        if (subtitle != null) {
            m.putString(MediaMetadata.METADATA_KEY_ARTIST, subtitle);
            m.putString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE, subtitle);
        }
        if (artUrl != null) {
            m.putString(MediaMetadata.METADATA_KEY_ART_URI, artUrl);
            m.putString(MediaMetadata.METADATA_KEY_DISPLAY_ICON_URI, artUrl);
        }
        if (art != null) {
            m.putBitmap(MediaMetadata.METADATA_KEY_ART, art);
        }
        if (length >= 0) {
            m.putLong(MediaMetadata.METADATA_KEY_DURATION, length);
        }
        session.setMetadata(m.build());
    }

    /** `what`: 0 stopped, 1 playing, 2 paused; `lengthMs` -1 for a live stream (no seeking). */
    synchronized void playback(int what, long positionMs, long lengthMs) {
        state = what == 1 ? PlaybackState.STATE_PLAYING
                : what == 2 ? PlaybackState.STATE_PAUSED : PlaybackState.STATE_STOPPED;
        position = positionMs;
        if (lengthMs != length) {
            length = lengthMs;
            if (title != null) {
                publish();
            }
        }
        long actions = PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE | PlaybackState.ACTION_PLAY_PAUSE
                | PlaybackState.ACTION_STOP | PlaybackState.ACTION_SKIP_TO_PREVIOUS;
        if (lengthMs >= 0) {
            actions |= PlaybackState.ACTION_SEEK_TO | PlaybackState.ACTION_FAST_FORWARD | PlaybackState.ACTION_REWIND;
        }
        if (canNext) {
            actions |= PlaybackState.ACTION_SKIP_TO_NEXT;
        }
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(actions)
                .setState(state, positionMs, state == PlaybackState.STATE_PLAYING ? 1f : 0f,
                        SystemClock.elapsedRealtime())
                .build());
        if (title != null) {
            notifyNow();
        }
    }

    synchronized void canNext(boolean can) {
        canNext = can;
        int what = state == PlaybackState.STATE_PLAYING ? 1 : state == PlaybackState.STATE_PAUSED ? 2 : 0;
        playback(what, position, length);
    }

    synchronized void close() {
        manager.cancel(NOTIFICATION);
        session.setActive(false);
        session.release();
    }

    /** The media notification; Android draws its buttons from the session's playback state. */
    private void notifyNow() {
        if (!manager.areNotificationsEnabled()) {
            return;
        }
        Notification.Builder b = new Notification.Builder(context, Notifier.CHANNEL_QUIET)
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle(title)
                .setOnlyAlertOnce(true)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setStyle(new Notification.MediaStyle().setMediaSession(session.getSessionToken()));
        if (subtitle != null) {
            b.setContentText(subtitle);
        }
        if (art != null) {
            b.setLargeIcon(art);
        }
        manager.notify(NOTIFICATION, b.build());
    }

    @Override
    public void onPlay() {
        command(handle, PLAY, 0);
    }

    @Override
    public void onPause() {
        command(handle, PAUSE, 0);
    }

    @Override
    public void onStop() {
        command(handle, STOP, 0);
    }

    @Override
    public void onSkipToNext() {
        command(handle, NEXT, 0);
    }

    @Override
    public void onSkipToPrevious() {
        command(handle, PREVIOUS, 0);
    }

    @Override
    public void onFastForward() {
        command(handle, SEEK_BY, SKIP);
    }

    @Override
    public void onRewind() {
        command(handle, SEEK_BY, -SKIP);
    }

    @Override
    public void onSeekTo(long ms) {
        command(handle, SEEK_TO, ms);
    }
}
