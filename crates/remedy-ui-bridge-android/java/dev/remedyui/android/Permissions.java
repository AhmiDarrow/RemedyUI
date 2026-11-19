// Run-time permission prompts.
//
// The system's prompt answers with onRequestPermissionsResult, which reaches the activity or
// one of its fragments, never the native code. This fragment exists only while a prompt is
// up: it asks, receives the answer, reports it and removes itself. A permission already
// granted is reported at once, with no prompt. A permission the app's manifest does not
// declare is refused by the system without a prompt, and reported as refused.

package dev.remedyui.android;

import android.app.Activity;
import android.app.Fragment;
import android.content.pm.PackageManager;

public final class Permissions extends Fragment {
    private static final String TAG = "dev.remedyui.android.Permissions";

    private long handle;
    private long request;
    private String permission;
    private boolean asked;
    private boolean done;

    /** Android recreates fragments by name; this one is never restored (see Bridge). */
    public Permissions() {}

    /** Asks for `permission` unless it is already granted (UI thread). */
    static void request(Activity activity, long handle, long request, String permission) {
        if (activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) {
            Bridge.permissionDone(handle, request, true);
            return;
        }
        Permissions p = new Permissions();
        p.handle = handle;
        p.request = request;
        p.permission = permission;
        activity.getFragmentManager().beginTransaction().add(p, TAG + request).commitNow();
    }

    @Override
    public void onStart() {
        super.onStart();
        if (!asked && permission != null) {
            asked = true;
            requestPermissions(new String[] {permission}, 1);
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] results) {
        // An empty answer means the prompt was interrupted (the activity went away): refused.
        boolean granted = results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED;
        finish(granted);
    }

    @Override
    public void onDestroy() {
        // Removed before the system answered (the activity is finishing): report a refusal so
        // the app never waits forever.
        finish(false);
        super.onDestroy();
    }

    private void finish(boolean granted) {
        if (done || permission == null) {
            return;
        }
        done = true;
        Bridge.permissionDone(handle, request, granted);
        if (isAdded()) {
            getFragmentManager().beginTransaction().remove(this).commitAllowingStateLoss();
        }
    }
}
