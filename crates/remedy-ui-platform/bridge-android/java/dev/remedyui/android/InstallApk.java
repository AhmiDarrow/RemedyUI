// Offers an APK the app already wrote under the share folder
// (cache/remedy-share/<request>/<index>/<name>) to the system package installer, so the
// person can tap Install. The file is served through SharedFiles (the same FileProvider
// shares use) and written into a PackageInstaller session; that is what actually starts
// the system confirmation. Existing app data is kept when the package and signing key
// match. Android 8 and later also need the "install unknown apps" setting for this app
// (REQUEST_INSTALL_PACKAGES): if it is off, the settings page for this app is opened
// instead, and the caller can try again once the person returns.

package dev.remedyui.android;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.IntentFilter;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;
import android.util.Log;
import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;

public final class InstallApk {
    private static final String TAG = "RemedyUI";
    /** Status callback for a committed install session. */
    private static final String ACTION_STATUS = "dev.remedyui.android.INSTALL_STATUS";

    private static final BroadcastReceiver RECEIVER = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
            if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                // The system hands us the confirmation activity; we have to start it.
                Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                if (confirm != null) {
                    confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                    context.startActivity(confirm);
                    Log.i(TAG, "package installer confirmation started");
                }
            } else if (status == PackageInstaller.STATUS_SUCCESS) {
                Log.i(TAG, "package installer finished: the package was installed");
            } else {
                String msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                Log.e(TAG, "package installer status " + status + (msg != null ? (": " + msg) : ""));
            }
        }
    };

    private InstallApk() {}

    /** The installer was started. */
    public static final String STARTED = "started";
    /** The settings page was opened so the person can allow installs from this app. */
    public static final String NEED_PERMISSION = "need_permission";

    /** True when this app may offer an APK to the system installer. */
    public static boolean canInstall(Activity activity) {
        if (Build.VERSION.SDK_INT < 26) {
            return true;
        }
        return activity.getPackageManager().canRequestPackageInstalls();
    }

    /**
     * Starts the system installer for the file at
     * cache/remedy-share/<request>/<index>/<name>, or opens the install-permission
     * settings when that is not yet allowed. `mime` is the content URI path segment
     * (the same encoding shares use).
     */
    public static String install(Activity activity, String request, int index, String mime, String name)
            throws java.io.IOException {
        if (!canInstall(activity)) {
            Intent settings = new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES)
                    .setData(Uri.parse("package:" + activity.getPackageName()))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            activity.startActivity(settings);
            return NEED_PERMISSION;
        }
        String authority = activity.getPackageName() + Share.AUTHORITY_SUFFIX;
        Uri uri = new Uri.Builder()
                .scheme("content")
                .authority(authority)
                .appendPath(request)
                .appendPath(Integer.toString(index))
                .appendPath(mime)
                .appendPath(name)
                .build();
        File apk = new File(new File(new File(new File(activity.getCacheDir(), SharedFiles.FOLDER), request),
                Integer.toString(index)), name);
        long size = apk.isFile() ? apk.length() : -1;
        PackageInstaller installer = activity.getPackageManager().getPackageInstaller();
        PackageInstaller.SessionParams params =
                new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
        int sessionId = installer.createSession(params);
        PackageInstaller.Session session = installer.openSession(sessionId);
        try {
            try (InputStream in = activity.getContentResolver().openInputStream(uri);
                    OutputStream out = session.openWrite("pkg", 0, size)) {
                if (in == null) {
                    throw new java.io.FileNotFoundException(uri.toString());
                }
                byte[] buf = new byte[64 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) {
                    out.write(buf, 0, n);
                }
                session.fsync(out);
            }
            Intent callback = new Intent(ACTION_STATUS).setPackage(activity.getPackageName());
            // API 33+ needs an exported/not-exported flag; this receiver is only for our
            // own pending intent, so it is not exported. The constant is 0x4 (RECEIVER_NOT_EXPORTED).
            try {
                activity.unregisterReceiver(RECEIVER);
            } catch (IllegalArgumentException ignored) {
            }
            IntentFilter filter = new IntentFilter(ACTION_STATUS);
            if (Build.VERSION.SDK_INT >= 33) {
                activity.registerReceiver(RECEIVER, filter, 0x4);
            } else {
                activity.registerReceiver(RECEIVER, filter);
            }
            int flags = PendingIntent.FLAG_UPDATE_CURRENT;
            if (Build.VERSION.SDK_INT >= 31) {
                flags |= PendingIntent.FLAG_MUTABLE;
            }
            PendingIntent pi = PendingIntent.getBroadcast(activity, sessionId, callback, flags);
            session.commit(pi.getIntentSender());
            Log.i(TAG, "package installer session " + sessionId + " committed for " + name);
            return STARTED;
        } catch (Exception e) {
            try {
                session.abandon();
            } catch (Exception ignored) {
            }
            Log.e(TAG, "package installer session failed: " + e);
            if (e instanceof java.io.IOException) {
                throw (java.io.IOException) e;
            }
            throw new java.io.IOException(e);
        }
    }
}
