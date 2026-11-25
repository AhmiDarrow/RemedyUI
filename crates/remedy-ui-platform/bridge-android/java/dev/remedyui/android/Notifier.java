// System notifications: posting, replacing and withdrawing them, and hearing what the person
// does with them.
//
// Each notification is posted under the tag "remedyui:<id>", so the app's own 64-bit id
// replaces and withdraws it. Pressing it opens the app's activity (a PendingIntent for the
// activity itself: Android 12 and later forbid opening an activity from a receiver a
// notification started); the activity reports the press, at start from its launching intent
// and while running from onNewIntent (RemedyActivity). Buttons and dismissals are broadcasts
// to a receiver this process registers while it runs, not exported, carrying a token only
// this process knows, so other apps cannot press them. Every PendingIntent is immutable.
//
// The status bar's small icon is the app's own icon, made monochrome (Android draws small
// icons from their alpha alone); the large icon is the notification's picture, if it has one.

package dev.remedyui.android;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.AdaptiveIconDrawable;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.Icon;
import android.net.Uri;
import android.os.Build;
import java.security.SecureRandom;

public final class Notifier {
    /** The press of the notification itself, a button, a dismissal (what Rust decodes). */
    static final int ACTIVATED = 0;
    static final int ACTION = 1;
    static final int DISMISSED = 2;

    static final String CHANNEL_DEFAULT = "remedyui.default";
    static final String CHANNEL_QUIET = "remedyui.quiet";
    private static final String SCHEME = "remedyui-notification";
    private static final String EXTRA_ID = "dev.remedyui.notification.id";
    private static final String EXTRA_ACTION = "dev.remedyui.notification.action";
    private static final String EXTRA_TOKEN = "dev.remedyui.notification.token";
    /**
     * The not-exported alias of RemedyActivity that notifications open (declared by
     * `cargo remedy package`): only this app's own PendingIntents can start it, so a press
     * it reports is real. Apps on a plain NativeActivity open the activity itself.
     */
    private static final String OPEN_ALIAS = "dev.remedyui.android.NotificationOpen";

    private static Notifier current;

    private final Context context;
    private final long handle;
    private final NotificationManager manager;
    private final String token;
    private final String buttonAction;
    private final String dismissAction;
    private final ComponentName opens;
    private final boolean opensAlias;
    private final BroadcastReceiver receiver;
    private Icon smallIcon;

    private Notifier(Activity activity, long handle) {
        this.context = activity.getApplicationContext();
        this.handle = handle;
        this.manager = (NotificationManager) context.getSystemService(Context.NOTIFICATION_SERVICE);
        byte[] bytes = new byte[16];
        new SecureRandom().nextBytes(bytes);
        StringBuilder t = new StringBuilder();
        for (byte b : bytes) {
            t.append(String.format("%02x", b));
        }
        this.token = t.toString();
        String pkg = context.getPackageName();
        this.buttonAction = pkg + ".remedyui.NOTIFICATION_BUTTON";
        this.dismissAction = pkg + ".remedyui.NOTIFICATION_DISMISSED";
        ComponentName alias = new ComponentName(pkg, OPEN_ALIAS);
        boolean hasAlias;
        try {
            context.getPackageManager().getActivityInfo(alias, 0);
            hasAlias = true;
        } catch (PackageManager.NameNotFoundException e) {
            hasAlias = false;
        }
        this.opensAlias = hasAlias;
        this.opens = hasAlias ? alias : new ComponentName(context, activity.getClass());
        channels();
        this.receiver =
                new BroadcastReceiver() {
                    @Override
                    public void onReceive(Context c, Intent intent) {
                        received(intent);
                    }
                };
        IntentFilter filter = new IntentFilter();
        filter.addAction(buttonAction);
        filter.addAction(dismissAction);
        filter.addDataScheme(SCHEME);
        if (Build.VERSION.SDK_INT >= 33) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            // Exported before Android 13; the token keeps other apps' broadcasts out.
            context.registerReceiver(receiver, filter);
        }
    }

    /** This process's notifier, made on first use (any thread). */
    static synchronized Notifier get(Activity activity, long handle) {
        if (current == null) {
            current = new Notifier(activity, handle);
        }
        return current;
    }

    /** Stops hearing buttons (the app is ending). */
    static synchronized void close() {
        if (current != null) {
            try {
                current.context.unregisterReceiver(current.receiver);
            } catch (IllegalArgumentException e) {
                // Not registered: nothing to undo.
            }
            current = null;
        }
    }

    /** The channels the app's notifications go to: alerting, and quiet. */
    private void channels() {
        NotificationChannel alerts =
                new NotificationChannel(CHANNEL_DEFAULT, "Notifications", NotificationManager.IMPORTANCE_DEFAULT);
        NotificationChannel quiet =
                new NotificationChannel(CHANNEL_QUIET, "Quiet notifications", NotificationManager.IMPORTANCE_LOW);
        quiet.setSound(null, null);
        quiet.enableVibration(false);
        manager.createNotificationChannel(alerts);
        manager.createNotificationChannel(quiet);
    }

    /** Whether notifications reach the person (the permission, and the app's switch). */
    boolean enabled() {
        return manager.areNotificationsEnabled();
    }

    private static String tag(long id) {
        return "remedyui:" + Long.toUnsignedString(id);
    }

    private static Uri uri(long id, String part) {
        return Uri.parse(SCHEME + ":" + Long.toUnsignedString(id) + "/" + part);
    }

    private PendingIntent broadcast(String action, long id, String part, String actionId) {
        Intent intent = new Intent(action, uri(id, part)).setPackage(context.getPackageName());
        intent.putExtra(EXTRA_ID, id);
        intent.putExtra(EXTRA_TOKEN, token);
        if (actionId != null) {
            intent.putExtra(EXTRA_ACTION, actionId);
        }
        return PendingIntent.getBroadcast(
                context, 0, intent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    /** Opens (or brings forward) the app's activity, naming notification `id`. */
    private PendingIntent open(long id) {
        Intent intent = new Intent(Intent.ACTION_VIEW, uri(id, "open")).setComponent(opens);
        intent.putExtra(EXTRA_ID, id);
        // Into the app's task, onto its one activity (onNewIntent while it runs).
        intent.addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK
                        | Intent.FLAG_ACTIVITY_SINGLE_TOP
                        | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        return PendingIntent.getActivity(
                context, 0, intent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    /**
     * The notification: title, body, up to three buttons (Android's room), quiet or alerting,
     * the picture (ARGB, size by size; null for none). `app` (the app's name) is not shown:
     * Android heads every notification with the app's own label already.
     */
    Notification build(long id, String title, String body, String[] actionIds, String[] labels, boolean silent,
            String app, int[] argb, int size, int urgency, boolean ongoing) {
        Notification.Builder b =
                new Notification.Builder(context, silent ? CHANNEL_QUIET : CHANNEL_DEFAULT)
                        .setSmallIcon(smallIcon())
                        .setContentTitle(title)
                        .setAutoCancel(!ongoing)
                        .setOngoing(ongoing)
                        .setOnlyAlertOnce(ongoing)
                        .setContentIntent(open(id))
                        .setDeleteIntent(broadcast(dismissAction, id, "dismissed", null));
        if (body != null && !body.isEmpty()) {
            b.setContentText(body);
            b.setStyle(new Notification.BigTextStyle().bigText(body));
        }
        if (urgency > 0) {
            // A warning (1) or an error (2): what went wrong, ranked as such in the shade.
            b.setCategory(Notification.CATEGORY_ERROR);
        }
        if (argb != null && size > 0 && argb.length == size * size) {
            b.setLargeIcon(Bitmap.createBitmap(argb, size, size, Bitmap.Config.ARGB_8888));
        }
        int n = Math.min(3, Math.min(actionIds.length, labels.length));
        for (int i = 0; i < n; i++) {
            b.addAction(
                    new Notification.Action.Builder(
                                    (Icon) null, labels[i], broadcast(buttonAction, id, "action/" + i, actionIds[i]))
                            .build());
        }
        if (Build.VERSION.SDK_INT >= 31 && ongoing) {
            // Shown at once, not after Android's ten-second grace for foreground work.
            b.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        }
        return b.build();
    }

    /** Shows (or replaces) notification `id`; throws when the person turned them off. */
    void show(long id, Notification notification) {
        if (!enabled()) {
            throw new IllegalStateException("notifications are turned off for this app");
        }
        manager.notify(tag(id), 1, notification);
    }

    /** Takes notification `id` away. */
    void withdraw(long id) {
        manager.cancel(tag(id), 1);
    }

    private void received(Intent intent) {
        if (!token.equals(intent.getStringExtra(EXTRA_TOKEN)) || !intent.hasExtra(EXTRA_ID)) {
            return;
        }
        long id = intent.getLongExtra(EXTRA_ID, 0);
        if (buttonAction.equals(intent.getAction())) {
            String action = intent.getStringExtra(EXTRA_ACTION);
            Bridge.notificationEvent(handle, id, ACTION, action == null ? "" : action);
            // A button press leaves an ordinary notification in place; the app withdraws or
            // replaces it as the press calls for.
        } else if (dismissAction.equals(intent.getAction())) {
            Bridge.notificationEvent(handle, id, DISMISSED, null);
        }
    }

    /** Reports a notification press the activity was opened or brought forward with. */
    static void opened(Intent intent) {
        Notifier n;
        synchronized (Notifier.class) {
            n = current;
        }
        if (n != null) {
            n.report(intent);
        }
    }

    void report(Intent intent) {
        if (intent == null || !intent.hasExtra(EXTRA_ID)) {
            return;
        }
        Uri data = intent.getData();
        if (data == null || !SCHEME.equals(data.getScheme())) {
            return;
        }
        // Only this app's own PendingIntents start the alias; without it (a plain
        // NativeActivity), the activity is exported and the press is taken as it comes.
        if (opensAlias && (intent.getComponent() == null || !intent.getComponent().equals(opens))) {
            return;
        }
        long id = intent.getLongExtra(EXTRA_ID, 0);
        // Reported once: the activity's intent stays, and a later read must not repeat it.
        intent.removeExtra(EXTRA_ID);
        Bridge.notificationEvent(handle, id, ACTIVATED, null);
    }

    /** The app's icon as a status bar icon: one color, shape from alpha. */
    synchronized Icon smallIcon() {
        if (smallIcon != null) {
            return smallIcon;
        }
        int size = Math.max(24, Math.round(24 * context.getResources().getDisplayMetrics().density));
        Bitmap bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888);
        Drawable icon = context.getPackageManager().getApplicationIcon(context.getApplicationInfo());
        Canvas canvas = new Canvas(bmp);
        if (Build.VERSION.SDK_INT >= 33
                && icon instanceof AdaptiveIconDrawable
                && ((AdaptiveIconDrawable) icon).getMonochrome() != null) {
            draw(((AdaptiveIconDrawable) icon).getMonochrome(), canvas, size, true);
        } else if (icon instanceof AdaptiveIconDrawable && ((AdaptiveIconDrawable) icon).getForeground() != null) {
            draw(((AdaptiveIconDrawable) icon).getForeground(), canvas, size, true);
        } else {
            draw(icon, canvas, size, false);
        }
        int[] px = new int[size * size];
        bmp.getPixels(px, 0, size, 0, 0, size, size);
        boolean drawn = monochrome(px);
        if (drawn) {
            bmp.setPixels(px, 0, size, 0, 0, size, size);
            smallIcon = Icon.createWithBitmap(bmp);
        } else {
            // An icon with no shape to show (blank): the system's information glyph.
            smallIcon = Icon.createWithResource("android", android.R.drawable.ic_dialog_info);
        }
        return smallIcon;
    }

    /** Draws an icon layer: adaptive layers are 108 dp with the shape in the middle 72. */
    private static void draw(Drawable d, Canvas canvas, int size, boolean layer) {
        int inset = layer ? size / 4 : 0;
        d.setBounds(-inset, -inset, size + inset, size + inset);
        d.draw(canvas);
    }

    /**
     * Makes the icon's pixels white with the shape in alpha. An icon with see-through parts
     * keeps its alpha; an opaque one (a picture on a square) becomes what stands out from
     * the color at its corners. False when nothing is left to show.
     */
    static boolean monochrome(int[] px) {
        int clear = 0;
        for (int p : px) {
            if ((p >>> 24) < 32) {
                clear++;
            }
        }
        boolean alphaShape = clear * 20 > px.length; // more than 5 % see-through
        int side = (int) Math.round(Math.sqrt(px.length));
        int[] corners = {px[0], px[side - 1], px[px.length - side], px[px.length - 1]};
        int br = 0;
        int bg = 0;
        int bb = 0;
        for (int c : corners) {
            br += (c >> 16) & 0xff;
            bg += (c >> 8) & 0xff;
            bb += c & 0xff;
        }
        br /= 4;
        bg /= 4;
        bb /= 4;
        boolean any = false;
        for (int i = 0; i < px.length; i++) {
            int p = px[i];
            int a = p >>> 24;
            if (!alphaShape) {
                int dr = ((p >> 16) & 0xff) - br;
                int dg = ((p >> 8) & 0xff) - bg;
                int db = (p & 0xff) - bb;
                int d = (int) Math.min(255, Math.sqrt(dr * dr + dg * dg + db * db) * 2);
                a = a * d / 255;
            }
            if (a > 0) {
                any = true;
            }
            px[i] = (a << 24) | 0xffffff;
        }
        return any;
    }
}
