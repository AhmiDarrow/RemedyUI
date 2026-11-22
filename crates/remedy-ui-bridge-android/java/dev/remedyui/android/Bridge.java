// The Java half of RemedyUI's Android bridge: what a native activity cannot reach from C.
//
// Android reports memory trims, configuration changes, setting changes and window insets only
// to Java objects implementing its listener interfaces. This class implements them and hands
// each report to the bridge's native code, which wakes the app's event loop. It is compiled
// into dex/classes.dex (tools/build-dex.sh). APKs `cargo remedy package` makes carry that dex
// as the app's code (RemedyActivity, the foreground service), and the bridge uses those
// classes; an app on a plain NativeActivity ships no Java, and the bridge loads the same dex
// from memory.

package dev.remedyui.android;

import android.app.Activity;
import android.app.Application;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ComponentCallbacks2;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Point;
import android.net.Uri;
import android.os.Bundle;
import android.view.DragEvent;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import android.content.ContentResolver;
import android.content.res.Configuration;
import android.database.ContentObserver;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.view.DisplayCutout;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.graphics.Color;

public final class Bridge
        implements ComponentCallbacks2, View.OnApplyWindowInsetsListener, View.OnLayoutChangeListener {
    private final long handle;
    private final Activity activity;
    private final View decor;
    private final View content;
    private final ContentObserver observer;

    private static native void trimMemory(long handle, int level);

    private static native void configurationChanged(long handle);

    private static native void safeArea(long handle, int top, int right, int bottom, int left);

    static native void imePreedit(long handle, String text, int cursorBytes);

    static native void imeCommit(long handle, String text);

    static native void imeDelete(long handle, int beforeBytes, int afterBytes);

    static native void permissionDone(long handle, long request, boolean granted);

    static native void shareDone(long handle, long request, int outcome, String error);

    static native void dialogDone(long handle, long request, String[] paths, String error);

    static native void documentSaved(String path, String error);

    static native void dragEnded(long handle, int effect);

    static native void a11ySetText(long handle, String text);

    /** A notification was pressed (kind 0), one of its buttons (1, with its id), dismissed (2). */
    static native void notificationEvent(long handle, long id, int kind, String action);

    /** The platform ended the app's foreground work, or refused it once it ran. */
    static native void foregroundStopped(long handle, String why);

    /** The activity was paused (true) or resumed: winit ignores the loop's wake-ups while paused. */
    static native void activityPaused(long handle, boolean paused);

    private final TextInput input;
    private final Application.ActivityLifecycleCallbacks lifecycle;
    /** The drag this activity started, while it is under way. */
    private Object drag;

    /** Starts listening. Runs on the UI thread. */
    public Bridge(long handle, Activity activity) {
        this.handle = handle;
        this.activity = activity;
        this.input = new TextInput(activity, handle);
        // The document picker's fragment lives only while a picker is open; a restored
        // activity must not try to recreate it (its class is not in the app's package).
        this.lifecycle =
                new Application.ActivityLifecycleCallbacks() {
                    @Override
                    public void onActivitySaveInstanceState(Activity a, Bundle out) {
                        if (a == Bridge.this.activity) {
                            out.remove("android:fragments");
                        }
                    }

                    @Override
                    public void onActivityCreated(Activity a, Bundle b) {}

                    @Override
                    public void onActivityStarted(Activity a) {}

                    @Override
                    public void onActivityResumed(Activity a) {
                        if (a == Bridge.this.activity) {
                            activityPaused(Bridge.this.handle, false);
                        }
                    }

                    @Override
                    public void onActivityPaused(Activity a) {
                        if (a == Bridge.this.activity) {
                            activityPaused(Bridge.this.handle, true);
                        }
                    }

                    @Override
                    public void onActivityStopped(Activity a) {}

                    @Override
                    public void onActivityDestroyed(Activity a) {}
                };
        activity.getApplication().registerActivityLifecycleCallbacks(lifecycle);
        // Opened by pressing one of the app's notifications: report it.
        Notifier.get(activity, handle).report(activity.getIntent());
        // A native activity draws on the whole window's surface, under the system bars. Lay
        // the view out the same way (edge to edge, bars see-through), so the view's bounds,
        // where accessibility nodes are placed, match the surface, and the safe area tells
        // the app what the bars cover.
        Window window = activity.getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
        } else {
            window.getDecorView()
                    .setSystemUiVisibility(
                            View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        window.setStatusBarColor(Color.TRANSPARENT);
        window.setNavigationBarColor(Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams attrs = window.getAttributes();
            attrs.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            window.setAttributes(attrs);
        }
        this.decor = window.getDecorView();
        this.content = contentView(activity);
        this.observer =
                new ContentObserver(new Handler(Looper.getMainLooper())) {
                    @Override
                    public void onChange(boolean selfChange) {
                        configurationChanged(Bridge.this.handle);
                    }
                };
        activity.registerComponentCallbacks(this);
        ContentResolver resolver = activity.getContentResolver();
        // Reduced motion follows the animator duration scale; contrast follows high-contrast text.
        resolver.registerContentObserver(
                Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, observer);
        resolver.registerContentObserver(
                Settings.Secure.getUriFor("high_text_contrast_enabled"), false, observer);
        decor.setOnApplyWindowInsetsListener(this);
        content.addOnLayoutChangeListener(this);
        decor.requestApplyInsets();
        // A 1 px target for the input method only. A full-screen view on top of the
        // drawing surface would eat taps once it became focusable for a text session.
        ViewGroup root = (ViewGroup) decor.findViewById(android.R.id.content);
        root.addView(input, new ViewGroup.LayoutParams(1, 1));
        content.setOnDragListener(
                new View.OnDragListener() {
                    @Override
                    public boolean onDrag(View v, DragEvent e) {
                        switch (e.getAction()) {
                            case DragEvent.ACTION_DRAG_STARTED:
                                // Hear how this activity's own drag ends.
                                return e.getLocalState() != null && e.getLocalState() == drag;
                            case DragEvent.ACTION_DROP:
                                return false;
                            case DragEvent.ACTION_DRAG_ENDED:
                                if (e.getLocalState() != null && e.getLocalState() == drag) {
                                    drag = null;
                                    dragEnded(Bridge.this.handle, e.getResult() ? 1 : 0);
                                }
                                return true;
                            default:
                                return true;
                        }
                    }
                });
    }

    /**
     * Adds "set text" to the accessibility nodes of text fields (UI thread, after AccessKit's
     * adapter installed its delegate): the adapter does not offer it. A screen reader's set
     * text focuses the field through the adapter, then reaches the app as the new text.
     */
    public void extendAccessibility() {
        content.post(
                new Runnable() {
                    @Override
                    public void run() {
                        final View.AccessibilityDelegate inner = content.getAccessibilityDelegate();
                        if (inner == null) {
                            return;
                        }
                        content.setAccessibilityDelegate(
                                new View.AccessibilityDelegate() {
                                    @Override
                                    public AccessibilityNodeProvider getAccessibilityNodeProvider(View host) {
                                        AccessibilityNodeProvider p = inner.getAccessibilityNodeProvider(host);
                                        return p == null ? null : new SettableText(p);
                                    }
                                });
                    }
                });
    }

    /** The adapter's nodes, with "set text" on editable, non-secret fields. */
    private final class SettableText extends AccessibilityNodeProvider {
        private final AccessibilityNodeProvider inner;

        SettableText(AccessibilityNodeProvider inner) {
            this.inner = inner;
        }

        @Override
        public AccessibilityNodeInfo createAccessibilityNodeInfo(int id) {
            AccessibilityNodeInfo info = inner.createAccessibilityNodeInfo(id);
            if (info != null && !info.isPassword() && "android.widget.EditText".contentEquals(
                    info.getClassName() == null ? "" : info.getClassName())) {
                info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_TEXT);
            }
            return info;
        }

        @Override
        public AccessibilityNodeInfo findFocus(int focus) {
            return inner.findFocus(focus);
        }

        @Override
        public boolean performAction(int id, int action, Bundle arguments) {
            if (action != AccessibilityNodeInfo.ACTION_SET_TEXT) {
                return inner.performAction(id, action, arguments);
            }
            AccessibilityNodeInfo info = createAccessibilityNodeInfo(id);
            if (info == null || info.isPassword()) {
                return false;
            }
            // Focus first, so the request that follows names this field.
            if (!inner.performAction(id, AccessibilityNodeInfo.ACTION_FOCUS, null)) {
                return false;
            }
            CharSequence text = arguments == null ? null
                    : arguments.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE);
            a11ySetText(handle, text == null ? "" : text.toString());
            return true;
        }
    }

    /** Starts or updates a text session (UI thread). */
    public void textInput(String text, int selStart, int selEnd, float x, float top, float bottom,
            boolean secret, boolean multiline, boolean numeric) {
        input.update(text, selStart, selEnd, x, top, bottom, secret, multiline, numeric);
    }

    /** The app committed the input method's composition itself (UI thread). */
    public void finishComposition() {
        input.finishComposition();
    }

    /** Ends the text session (UI thread). */
    public void endTextInput() {
        input.end();
    }

    /** Asks for a run-time permission unless it is already granted (UI thread). */
    public void requestPermission(long request, String permission) {
        Permissions.request(activity, handle, request, permission);
    }

    /** Opens the share chooser (UI thread). */
    public void share(long request, String title, String text, String[] names, String[] types, byte[][] contents,
            String mime, boolean requireFiles) {
        Share.show(activity, handle, request, title, text, names, types, contents, mime, requireFiles);
    }

    /** Opens the print dialog for an HTML page (UI thread). */
    public void print(String title, String html) {
        Print.show(activity, title, html);
    }

    /** Opens the document picker (UI thread). */
    public void pick(long request, boolean save, boolean multiple, String[] extensions, String fileName,
            String title) {
        Picker.show(activity, handle, request, save, multiple, extensions, fileName, title);
    }

    /**
     * Starts a system drag of text and links out of the window (UI thread, while a finger is
     * down on it). Other apps take it in split screen or freeform windows.
     */
    public void startDrag(String text, String[] uris, int shadowSize) {
        ClipData clip = null;
        if (text != null) {
            clip = ClipData.newPlainText("", text);
        }
        for (String u : uris) {
            ClipData.Item item = new ClipData.Item(Uri.parse(u));
            if (clip == null) {
                clip = new ClipData("", new String[] {ClipDescription.MIMETYPE_TEXT_URILIST}, item);
            } else {
                clip.addItem(item);
            }
        }
        Object token = new Object();
        drag = token;
        boolean started = clip != null
                && content.startDragAndDrop(clip, new Shadow(shadowSize), token,
                        View.DRAG_FLAG_GLOBAL | View.DRAG_FLAG_GLOBAL_URI_READ);
        if (!started) {
            drag = null;
            dragEnded(handle, -1);
        }
    }

    /** A plain square under the finger. */
    private static final class Shadow extends View.DragShadowBuilder {
        private final int size;
        private final Paint paint = new Paint();

        Shadow(int size) {
            this.size = Math.max(1, size);
            paint.setColor(0x80808080);
        }

        @Override
        public void onProvideShadowMetrics(Point outSize, Point outTouch) {
            outSize.set(size, size);
            outTouch.set(size / 2, size / 2);
        }

        @Override
        public void onDrawShadow(Canvas canvas) {
            canvas.drawRect(0, 0, size, size, paint);
        }
    }

    /**
     * Shows or replaces notification `id` (any thread): title, body, buttons (ids and labels),
     * quiet or alerting, the app's name, a picture (ARGB, size by size; null for none).
     * Throws when the person turned notifications off.
     */
    public static void showNotification(Activity activity, long handle, long id, String title, String body,
            String[] actionIds, String[] labels, boolean silent, String app, int[] argb, int size) {
        Notifier n = Notifier.get(activity, handle);
        n.show(id, n.build(id, title, body, actionIds, labels, silent, app, argb, size, false));
    }

    /** Takes notification `id` away (any thread). */
    public static void withdrawNotification(Activity activity, long handle, long id) {
        Notifier.get(activity, handle).withdraw(id);
    }

    /** Whether the app's notifications reach the person (any thread). */
    public static boolean notificationsEnabled(Activity activity, long handle) {
        return Notifier.get(activity, handle).enabled();
    }

    /**
     * Starts or changes the app's foreground work of `types` (foreground service type bits),
     * shown by notification `id` (any thread). Throws with what Android would refuse.
     */
    public static void startForeground(Activity activity, long handle, long id, String title, String body,
            String[] actionIds, String[] labels, boolean silent, String app, int[] argb, int size, int types) {
        Notifier n = Notifier.get(activity, handle);
        Foreground.start(activity, handle, n.build(id, title, body, actionIds, labels, silent, app, argb, size, true),
                types);
    }

    /** Ends the app's foreground work (any thread). */
    public static void stopForeground(Activity activity) {
        Foreground.stop(activity);
    }

    /** The view the native activity draws in: where accessibility nodes live. */
    public static View contentView(Activity activity) {
        View root = activity.getWindow().getDecorView().findViewById(android.R.id.content);
        if (root instanceof ViewGroup && ((ViewGroup) root).getChildCount() > 0) {
            return ((ViewGroup) root).getChildAt(0);
        }
        return root;
    }

    /** Stops listening. Runs on the UI thread. */
    public void close() {
        Foreground.stop(activity);
        Notifier.close();
        input.end();
        ((ViewGroup) decor.findViewById(android.R.id.content)).removeView(input);
        content.setOnDragListener(null);
        activity.getApplication().unregisterActivityLifecycleCallbacks(lifecycle);
        activity.unregisterComponentCallbacks(this);
        activity.getContentResolver().unregisterContentObserver(observer);
        decor.setOnApplyWindowInsetsListener(null);
        content.removeOnLayoutChangeListener(this);
    }

    @Override
    public void onTrimMemory(int level) {
        trimMemory(handle, level);
    }

    @Override
    public void onLowMemory() {
        // The native activity reports this itself (the bridge's memory warning).
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        configurationChanged(handle);
    }

    @Override
    public WindowInsets onApplyWindowInsets(View view, WindowInsets insets) {
        WindowInsets out = view.onApplyWindowInsets(insets);
        report();
        return out;
    }

    @Override
    public void onLayoutChange(
            View view, int left, int top, int right, int bottom, int oldLeft, int oldTop, int oldRight,
            int oldBottom) {
        report();
    }

    /** The insets that reach into the window's surface, in its pixels. */
    private void report() {
        WindowInsets insets = content.getRootWindowInsets();
        if (insets == null) {
            return;
        }
        int top;
        int right;
        int bottom;
        int left;
        if (Build.VERSION.SDK_INT >= 30) {
            android.graphics.Insets i =
                    insets.getInsets(
                            WindowInsets.Type.systemBars()
                                    | WindowInsets.Type.displayCutout()
                                    | WindowInsets.Type.ime());
            top = i.top;
            right = i.right;
            bottom = i.bottom;
            left = i.left;
        } else {
            top = insets.getSystemWindowInsetTop();
            right = insets.getSystemWindowInsetRight();
            bottom = insets.getSystemWindowInsetBottom();
            left = insets.getSystemWindowInsetLeft();
            if (Build.VERSION.SDK_INT >= 28) {
                DisplayCutout cut = insets.getDisplayCutout();
                if (cut != null) {
                    top = Math.max(top, cut.getSafeInsetTop());
                    right = Math.max(right, cut.getSafeInsetRight());
                    bottom = Math.max(bottom, cut.getSafeInsetBottom());
                    left = Math.max(left, cut.getSafeInsetLeft());
                }
            }
        }
        // The native activity's surface is the whole window, so the window's insets are the
        // surface's.
        safeArea(handle, Math.max(0, top), Math.max(0, right), Math.max(0, bottom), Math.max(0, left));
    }
}
