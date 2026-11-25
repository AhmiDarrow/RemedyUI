// Holds a RemedyUI MediaSession open inside an app (tools/android-media-check.sh) while the
// script presses its buttons with `cmd media_session dispatch`; reports what reached Rust
// once Stop (the last press) arrives, or after a minute and a half.

package dev.remedyui.mediacheck;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.media.AudioAttributes;
import android.media.AudioFormat;
import android.media.AudioTrack;
import android.os.Bundle;
import android.os.SystemClock;
import android.util.Log;

public class MediaCheck extends Instrumentation {
    private static native String start(Context context);

    private static native String commands();

    private static native void stop();

    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override
    public void onStart() {
        Bundle out = new Bundle();
        int code = Activity.RESULT_CANCELED;
        try {
            System.loadLibrary("android_media_check");
            String why = start(getTargetContext());
            if (!why.isEmpty()) {
                out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "media-check: could not start: " + why + "\n");
            } else {
                Log.i("RemedyUI", "media-check: session open");
                // Android sends the media keys to the session of the app playing sound, so
                // this one plays silence meanwhile, as a player would play its media.
                int rate = 48000;
                AudioTrack track = new AudioTrack.Builder()
                        .setAudioAttributes(new AudioAttributes.Builder()
                                .setUsage(AudioAttributes.USAGE_MEDIA)
                                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                                .build())
                        .setAudioFormat(new AudioFormat.Builder()
                                .setSampleRate(rate)
                                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                                .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                                .build())
                        .setTransferMode(AudioTrack.MODE_STREAM)
                        .build();
                track.play();
                short[] silence = new short[rate / 10];
                long until = SystemClock.uptimeMillis() + 90_000;
                String seen = "";
                while (SystemClock.uptimeMillis() < until && !seen.contains("Stop")) {
                    track.write(silence, 0, silence.length);
                    seen = commands();
                }
                track.stop();
                track.release();
                stop();
                out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "media-check commands:\n" + seen + "\nmedia-check: done\n");
                code = Activity.RESULT_OK;
            }
        } catch (Throwable t) {
            out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "media-check: " + Log.getStackTraceString(t));
        }
        finish(code, out);
    }
}
