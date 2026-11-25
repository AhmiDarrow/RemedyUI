// Keeps the app's process running while keepalive requests (remedy-ui-platform::services_net's
// `Request::keepalive`, an app's last saves) finish after it leaves the screen.
//
// A started service lifts the process out of Android's cached state, so it is not frozen
// while it is out of sight; Android lets an app that just went to the background keep its
// services for a while (about a minute), which a last save needs far less of. No
// notification and no permission: it is not foreground work. The manifest `cargo remedy
// package` writes declares it; on a plain NativeActivity starting it fails and the requests
// run on as long as Android leaves the process alone.

package dev.remedyui.android;

import android.app.Activity;
import android.app.Service;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.os.IBinder;

public final class Keepalive extends Service {
    /** Starts (`on`) or stops the service (any thread). Throws with why it did not start. */
    static void hold(Activity activity, boolean on) {
        Context context = activity.getApplicationContext();
        Intent intent = new Intent(context, Keepalive.class);
        if (!on) {
            context.stopService(intent);
            return;
        }
        ComponentName started = context.startService(intent);
        if (started == null) {
            throw new IllegalStateException(
                    "the app's manifest declares no keepalive service: package with cargo remedy package");
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        // Not restarted if Android ends it: the requests it kept are gone with the process.
        return START_NOT_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
