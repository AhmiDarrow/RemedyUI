// Ongoing work that keeps the app running while it is out of sight: Android's foreground
// service, with the notification that shows the work is under way.
//
// The app's manifest declares this service with the foreground service types it uses
// (`[android] foreground` in remedy.toml; `cargo remedy package` writes it and the
// FOREGROUND_SERVICE permissions). Starting checks everything Android checks that can be
// known beforehand (the declaration, the permissions, the app on screen for microphone and
// camera work) and throws with what is missing, so the app hears it at once. What Android
// decides only once the service runs (it refuses after all, or a time limit runs out) reaches
// the app as "foreground stopped".

package dev.remedyui.android;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.Notification;
import android.app.Service;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;

public final class Foreground extends Service {
    /** The foreground notification's number (Android's, apart from the tagged ones). */
    static final int NOTIFICATION = 0x52454d59;

    // Foreground service types (ServiceInfo.FOREGROUND_SERVICE_TYPE_*), as Rust sends them.
    static final int DATA_SYNC = 1;
    static final int MEDIA_PLAYBACK = 2;
    static final int CAMERA = 64;
    static final int MICROPHONE = 128;

    private static final Object LOCK = new Object();
    private static Foreground running;
    private static Notification pending;
    private static int pendingTypes;
    /** The types the running service holds (0 before it has started). */
    private static int heldTypes;
    private static boolean stopAsked;
    private static long handle;
    private static final Handler MAIN = new Handler(Looper.getMainLooper());

    /** Starts the work, or changes it while it runs. Throws with what Android would refuse. */
    static void start(Activity activity, long bridge, Notification notification, int types) {
        if (types == 0) {
            throw new IllegalArgumentException("foreground work needs at least one kind");
        }
        Context context = activity.getApplicationContext();
        PackageManager pm = context.getPackageManager();
        ServiceInfo info;
        try {
            info = pm.getServiceInfo(new ComponentName(context, Foreground.class), 0);
        } catch (PackageManager.NameNotFoundException e) {
            throw new IllegalStateException(
                    "the app's manifest declares no foreground service: list the kinds in remedy.toml's "
                            + "[android] foreground and package with cargo remedy package");
        }
        if (Build.VERSION.SDK_INT >= 29) {
            int missing = types & ~info.getForegroundServiceType();
            if (missing != 0) {
                throw new IllegalStateException(
                        "the manifest's foreground service does not declare " + names(missing)
                                + ": add it to remedy.toml's [android] foreground");
            }
        }
        if (Build.VERSION.SDK_INT >= 28) {
            need(context, "android.permission.FOREGROUND_SERVICE");
        }
        if ((types & MICROPHONE) != 0) {
            granted(context, "android.permission.RECORD_AUDIO", "the microphone permission");
        }
        if ((types & CAMERA) != 0) {
            granted(context, "android.permission.CAMERA", "the camera permission");
        }
        if (Build.VERSION.SDK_INT >= 34) {
            if ((types & MICROPHONE) != 0) {
                need(context, "android.permission.FOREGROUND_SERVICE_MICROPHONE");
            }
            if ((types & CAMERA) != 0) {
                need(context, "android.permission.FOREGROUND_SERVICE_CAMERA");
            }
            if ((types & MEDIA_PLAYBACK) != 0) {
                need(context, "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK");
            }
            if ((types & DATA_SYNC) != 0) {
                need(context, "android.permission.FOREGROUND_SERVICE_DATA_SYNC");
            }
        }
        boolean isRunning;
        int held;
        synchronized (LOCK) {
            // A service already told to end is let go; a new one starts.
            isRunning = running != null && !stopAsked;
            held = heldTypes;
        }
        // Microphone and camera work may only begin while the person sees the app (Android
        // 11 and later); work already holding them may change.
        int whileInUse = types & (MICROPHONE | CAMERA) & ~held;
        if (Build.VERSION.SDK_INT >= 30 && whileInUse != 0 && !onScreen()) {
            throw new IllegalStateException(
                    "start " + names(whileInUse) + " work while the app is on screen: Android refuses it from the background");
        }
        synchronized (LOCK) {
            pending = notification;
            pendingTypes = types;
            stopAsked = false;
            handle = bridge;
        }
        if (isRunning) {
            MAIN.post(
                    new Runnable() {
                        @Override
                        public void run() {
                            Foreground f;
                            synchronized (LOCK) {
                                f = running;
                            }
                            if (f != null) {
                                f.apply();
                            }
                        }
                    });
        } else {
            // Throws (ForegroundServiceStartNotAllowedException) when Android refuses at once.
            context.startForegroundService(new Intent(context, Foreground.class));
        }
    }

    /** Ends the work and takes its notification away. */
    static void stop(Context context) {
        Foreground f;
        synchronized (LOCK) {
            stopAsked = true;
            pending = null;
            f = running;
        }
        if (f != null) {
            MAIN.post(
                    new Runnable() {
                        @Override
                        public void run() {
                            f.end();
                        }
                    });
        }
        // Not yet started: onStartCommand starts it (Android requires that) and ends it.
    }

    private static boolean onScreen() {
        ActivityManager.RunningAppProcessInfo me = new ActivityManager.RunningAppProcessInfo();
        ActivityManager.getMyMemoryState(me);
        return me.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_VISIBLE;
    }

    private static void need(Context context, String permission) {
        if (context.checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
            throw new IllegalStateException(
                    "the app's manifest does not declare " + permission + " (cargo remedy package adds it for [android] foreground)");
        }
    }

    private static void granted(Context context, String permission, String what) {
        if (context.checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
            throw new IllegalStateException(what + " is not granted: ask for it with App::request_permission first");
        }
    }

    static String names(int types) {
        StringBuilder s = new StringBuilder();
        String[] all = {"data-sync", "media-playback", "camera", "microphone"};
        int[] bits = {DATA_SYNC, MEDIA_PLAYBACK, CAMERA, MICROPHONE};
        for (int i = 0; i < bits.length; i++) {
            if ((types & bits[i]) != 0) {
                if (s.length() > 0) {
                    s.append(", ");
                }
                s.append(all[i]);
            }
        }
        return s.toString();
    }

    @Override
    public void onCreate() {
        super.onCreate();
        synchronized (LOCK) {
            running = this;
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        apply();
        // Not restarted by itself: the work belongs to the running app.
        return START_NOT_STICKY;
    }

    /** Holds the latest work's types and shows its notification (UI thread). */
    private void apply() {
        Notification n;
        int types;
        boolean stop;
        synchronized (LOCK) {
            n = pending;
            types = pendingTypes;
            stop = stopAsked;
        }
        if (n == null) {
            // Asked to stop before the service began: Android still requires startForeground.
            n = new Notification.Builder(this, Notifier.CHANNEL_QUIET)
                    .setSmallIcon(android.R.drawable.ic_dialog_info)
                    .build();
            types = 0;
            stop = true;
        }
        try {
            if (Build.VERSION.SDK_INT >= 29 && types != 0) {
                startForeground(NOTIFICATION, n, types);
            } else {
                startForeground(NOTIFICATION, n);
            }
            synchronized (LOCK) {
                heldTypes = types;
            }
        } catch (RuntimeException e) {
            // Refused now that it runs (out of sight, a permission withdrawn meanwhile).
            report("Android refused the foreground work: " + e.getMessage());
            end();
            return;
        }
        if (stop) {
            end();
        }
    }

    private void end() {
        if (Build.VERSION.SDK_INT >= 33) {
            stopForeground(Service.STOP_FOREGROUND_REMOVE);
        } else {
            stopForeground(true);
        }
        stopSelf();
    }

    private static void report(String why) {
        long h;
        synchronized (LOCK) {
            h = handle;
        }
        Bridge.foregroundStopped(h, why);
    }

    /** Android 15's daily time limits for some kinds (data sync) ran out. */
    @Override
    public void onTimeout(int startId, int type) {
        report("the system's time limit for " + names(type) + " work ran out");
        end();
    }

    /** Android 14's limit for short services ran out (other kinds come here before 15). */
    @Override
    public void onTimeout(int startId) {
        report("the system's time limit for this work ran out");
        end();
    }

    /**
     * The person swiped the app away from recents: the work belongs to the app they closed, so
     * it ends with the process (the activity's destroy may not come while the service holds it).
     */
    @Override
    public void onTaskRemoved(Intent rootIntent) {
        end();
        android.os.Process.killProcess(android.os.Process.myPid());
    }

    @Override
    public void onDestroy() {
        synchronized (LOCK) {
            if (running == this) {
                running = null;
                heldTypes = 0;
            }
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
