// Runs the isolated-decoding tests inside an app (media-isolated/tools/android-check.sh): loads the
// android-isolated-check library and hands it the app's Context, so its helpers are the
// bridge's isolated decoding services. Reports the tests' output as the instrumentation's.

package dev.remedyui.isolatedcheck;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.os.Bundle;
import android.util.Log;

public class IsolatedCheck extends Instrumentation {
    private static native String run(Context context, String cacheDir, String filter);

    private Bundle arguments;

    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        this.arguments = arguments;
        start();
    }

    @Override
    public void onStart() {
        Bundle out = new Bundle();
        int code = Activity.RESULT_CANCELED;
        try {
            System.loadLibrary("android_isolated_check");
            Context c = getTargetContext();
            String filter = arguments == null ? "" : arguments.getString("filter", "");
            String report = run(c, c.getCacheDir().getAbsolutePath(), filter);
            out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, report);
            if (report.contains("isolated-check: PASSED")) {
                code = Activity.RESULT_OK;
            }
        } catch (Throwable t) {
            out.putString(Instrumentation.REPORT_KEY_STREAMRESULT, "isolated-check: " + Log.getStackTraceString(t));
        }
        finish(code, out);
    }
}
