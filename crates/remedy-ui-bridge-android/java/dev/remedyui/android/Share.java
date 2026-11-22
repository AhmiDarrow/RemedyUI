// Share sheets: the system's chooser for ACTION_SEND or ACTION_SEND_MULTIPLE.
//
// The chooser answers with an activity result, which reaches the activity or one of its
// fragments, never the native code; this fragment exists only while a chooser is open. The
// result itself says nothing about the choice, so the chooser is also given an IntentSender
// that Android calls when the person picks a target: that report is "shared", and a chooser
// that closes without one was cancelled. Android cannot tell whether the target app then
// finished (a mail may still be discarded).
//
// Files are written to the app's cache (remedy-share/<request>/<index>/<name>) and served to
// the chosen app, read-only and for that share only, by the SharedFiles provider the app's
// manifest declares (`cargo remedy package` writes it). Without that provider other apps
// cannot read the app's files at all; the share then goes without them, or not at all when
// the app requires them. The folders of earlier shares are removed when the next one starts.

package dev.remedyui.android;

import android.app.Activity;
import android.app.Fragment;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import java.io.File;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.Set;

public final class Share extends Fragment {
    private static final String TAG = "dev.remedyui.android.Share";
    /** The broadcast the chooser sends when the person picks a target. */
    private static final String CHOSEN = "dev.remedyui.android.SHARE_CHOSEN";
    /** The provider's authority after the app's package name (see SharedFiles). */
    static final String AUTHORITY_SUFFIX = ".remedy.share";
    static final int SHARED = 0;
    static final int CANCELLED = 1;
    static final int UNSUPPORTED = 2;
    /** Shares whose chooser is open; their files stay when another share starts. */
    private static final Set<Long> OPEN = new HashSet<>();

    private long handle;
    private long request;
    private Intent chooser;
    private BroadcastReceiver receiver;
    private boolean started;
    private boolean done;

    /** Android recreates fragments by name; this one is never restored (see Bridge). */
    public Share() {}

    /**
     * Opens the chooser (UI thread). `names`, `types` and `contents` describe the files;
     * `mime` labels the whole share.
     */
    static void show(Activity activity, long handle, long request, String title, String text, String[] names,
            String[] types, byte[][] contents, String mime, boolean requireFiles) {
        try {
            Intent send = intent(activity, request, title, text, names, types, contents, mime, requireFiles);
            if (send == null) {
                Bridge.shareDone(handle, request, UNSUPPORTED,
                        "the app's manifest declares no share provider, so other apps cannot read its files"
                                + " (cargo remedy package declares it)");
                return;
            }
            Share s = new Share();
            s.handle = handle;
            s.request = request;
            s.chooser = s.chooser(activity, send, title);
            synchronized (OPEN) {
                OPEN.add(request);
            }
            activity.getFragmentManager().beginTransaction().add(s, TAG + request).commitNow();
        } catch (Exception e) {
            Bridge.shareDone(handle, request, UNSUPPORTED, "the share could not be prepared: " + e);
        }
    }

    /** The ACTION_SEND or ACTION_SEND_MULTIPLE intent, or null when the files cannot go. */
    private static Intent intent(Activity activity, long request, String title, String text, String[] names,
            String[] types, byte[][] contents, String mime, boolean requireFiles) throws java.io.IOException {
        String authority = activity.getPackageName() + AUTHORITY_SUFFIX;
        boolean served = activity.getPackageManager().resolveContentProvider(authority, 0) != null;
        boolean files = names.length > 0 && served;
        if (names.length > 0 && !served && (requireFiles || (title.isEmpty() && text.isEmpty()))) {
            return null;
        }
        Intent send = new Intent(files && names.length > 1 ? Intent.ACTION_SEND_MULTIPLE : Intent.ACTION_SEND);
        send.setType(files ? mime : "text/plain");
        if (!title.isEmpty()) {
            send.putExtra(Intent.EXTRA_SUBJECT, title);
            send.putExtra(Intent.EXTRA_TITLE, title);
        }
        if (!text.isEmpty()) {
            send.putExtra(Intent.EXTRA_TEXT, text);
        }
        if (files) {
            File root = new File(activity.getCacheDir(), SharedFiles.FOLDER);
            clearEarlier(root);
            ArrayList<Uri> uris = new ArrayList<>();
            for (int i = 0; i < names.length; i++) {
                File dir = new File(root, request + "/" + i);
                if (!dir.isDirectory() && !dir.mkdirs()) {
                    throw new java.io.IOException("cannot make " + dir);
                }
                try (FileOutputStream out = new FileOutputStream(new File(dir, names[i]))) {
                    out.write(contents[i]);
                }
                uris.add(new Uri.Builder().scheme("content").authority(authority)
                        .appendPath(Long.toString(request)).appendPath(Integer.toString(i))
                        .appendPath(types[i]).appendPath(names[i]).build());
            }
            if (uris.size() == 1) {
                send.putExtra(Intent.EXTRA_STREAM, uris.get(0));
            } else {
                send.putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris);
            }
            // The grant travels with the clip data: only the chosen app may read, and only
            // these files.
            ClipData clip = ClipData.newRawUri(title, uris.get(0));
            for (int i = 1; i < uris.size(); i++) {
                clip.addItem(new ClipData.Item(uris.get(i)));
            }
            send.setClipData(clip);
            send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        }
        return send;
    }

    /** Removes the files of earlier shares whose chooser has closed. */
    private static void clearEarlier(File root) {
        File[] old = root.listFiles();
        if (old == null) {
            return;
        }
        for (File f : old) {
            // Only numeric share-request folders are temporary. A named folder (an APK
            // offered to the system installer, for example) is left alone.
            long id;
            try {
                id = Long.parseLong(f.getName());
            } catch (NumberFormatException e) {
                continue;
            }
            boolean open;
            synchronized (OPEN) {
                open = OPEN.contains(id);
            }
            if (!open) {
                delete(f);
            }
        }
    }

    private static long parse(String name) {
        try {
            return Long.parseLong(name);
        } catch (NumberFormatException e) {
            return -1;
        }
    }

    private static void delete(File f) {
        File[] children = f.listFiles();
        if (children != null) {
            for (File c : children) {
                delete(c);
            }
        }
        f.delete();
    }

    /** The chooser for `send`, which reports the person's pick to this fragment. */
    private Intent chooser(Activity activity, Intent send, String title) {
        receiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                if (intent.getLongExtra("request", -1) == request) {
                    finish(SHARED, null);
                }
            }
        };
        IntentFilter filter = new IntentFilter(CHOSEN);
        if (Build.VERSION.SDK_INT >= 33) {
            activity.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            activity.registerReceiver(receiver, filter);
        }
        Intent chosen = new Intent(CHOSEN).setPackage(activity.getPackageName()).putExtra("request", request);
        // The chooser fills in the chosen target, so the intent must stay mutable.
        int flags = PendingIntent.FLAG_UPDATE_CURRENT;
        if (Build.VERSION.SDK_INT >= 31) {
            flags |= PendingIntent.FLAG_MUTABLE;
        }
        PendingIntent pi = PendingIntent.getBroadcast(activity, (int) request, chosen, flags);
        return Intent.createChooser(send, title.isEmpty() ? null : title, pi.getIntentSender());
    }

    @Override
    public void onStart() {
        super.onStart();
        if (!started && chooser != null) {
            started = true;
            try {
                startActivityForResult(chooser, 1);
            } catch (RuntimeException e) {
                finish(UNSUPPORTED, "no share chooser: " + e);
            }
        }
    }

    @Override
    public void onActivityResult(int requestCode, int resultCode, Intent data) {
        // The pick's broadcast was sent before the chooser closed; let it arrive first.
        new Handler(Looper.getMainLooper()).post(new Runnable() {
            @Override
            public void run() {
                finish(CANCELLED, null);
            }
        });
    }

    @Override
    public void onDestroy() {
        // Removed before the chooser answered (the activity is finishing): the app never
        // waits forever.
        finish(CANCELLED, null);
        super.onDestroy();
    }

    private void finish(int outcome, String error) {
        if (done || chooser == null) {
            return;
        }
        done = true;
        synchronized (OPEN) {
            OPEN.remove(request);
        }
        Activity a = getActivity();
        if (receiver != null && a != null) {
            try {
                a.unregisterReceiver(receiver);
            } catch (IllegalArgumentException e) {
                // Already gone with the activity.
            }
        }
        if (outcome != SHARED && a != null) {
            delete(new File(new File(a.getCacheDir(), SharedFiles.FOLDER), Long.toString(request)));
        }
        Bridge.shareDone(handle, request, outcome, error);
        if (isAdded()) {
            getFragmentManager().beginTransaction().remove(this).commitAllowingStateLoss();
        }
    }
}
