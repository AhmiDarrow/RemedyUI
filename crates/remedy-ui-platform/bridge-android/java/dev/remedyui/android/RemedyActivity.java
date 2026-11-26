// The activity RemedyUI apps run in when packaged with their bridge's Java in the APK
// (`cargo remedy package --target android`, tools/build-apk.sh).
//
// It is Android's NativeActivity, which loads the app's library named by the manifest's
// android.app.lib_name, plus what NativeActivity drops: a new intent for the running activity.
// Pressing one of the app's notifications starts this activity with an intent naming the
// notification; while the app runs Android hands that intent to onNewIntent (NativeActivity
// neither keeps nor forwards it), and the bridge reports the press to the app.

package dev.remedyui.android;

import android.app.NativeActivity;
import android.content.Intent;

public class RemedyActivity extends NativeActivity {
    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        // getIntent() then names the latest launch, as launch options expect.
        setIntent(intent);
        Notifier.opened(intent);
    }

    // NativeActivity.onDestroy waits for the app's native main to return, and winit's event
    // loop never ends for a destroy the app did not ask for: Android restarting the activity
    // for a change the manifest cannot claim (assetsPaths, as after a system update or new
    // wallpaper colours), or "don't keep activities". Waiting would freeze the app. winit
    // allows one event loop per process, so the process ends here and Android starts the
    // activity in a new one. By now the window is gone, and the app has been told it is
    // leaving (HostEvent::Suspended) and had its chance to save.
    @Override
    protected void onDestroy() {
        if (!isFinishing()) {
            android.os.Process.killProcess(android.os.Process.myPid());
        }
        super.onDestroy();
    }
}
