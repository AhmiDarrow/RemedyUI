// Opens an activity inside an app (tools/android-keep-awake-check.sh) and keeps its screen on
// through RemedyUI's power::inhibit(Inhibit::Display): two holds for 10 seconds, then one
// let go (the flag must stay) for 10, then the last (it must clear) for 10, while the script
// reads the window's flags with `dumpsys window`. Logs each phase as "keep-awake: <phase>".

package dev.remedyui.keepawakecheck;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.util.Log;

public class KeepAwakeCheck extends Instrumentation {
    /** The window kept on. */
    public static class Screen extends Activity {}

    private static native String hold(Activity activity);

    private static native int release();

    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    private static void phase(String what) {
        Log.i("RemedyUI", "keep-awake: " + what);
    }

    @Override
    public void onStart() {
        Bundle out = new Bundle();
        int code = Activity.RESULT_CANCELED;
        try {
            System.loadLibrary("android_keep_awake_check");
            Intent intent = new Intent(getTargetContext(), Screen.class);
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            Activity activity = startActivitySync(intent);
            waitForIdleSync();
            SystemClock.sleep(3000);
            phase("before");
            SystemClock.sleep(5000);
            String why = hold(activity);
            if (!why.isEmpty()) {
                out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "keep-awake: could not hold: " + why + "\n");
            } else {
                phase("held");
                SystemClock.sleep(10_000);
                int left = release();
                phase("one-left " + left);
                SystemClock.sleep(10_000);
                left = release();
                phase("released " + left);
                SystemClock.sleep(10_000);
                activity.finish();
                out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "keep-awake: done\n");
                code = Activity.RESULT_OK;
            }
        } catch (Throwable t) {
            out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "keep-awake: " + Log.getStackTraceString(t));
        }
        finish(code, out);
    }
}
