// Open and save dialogs through the Storage Access Framework.
//
// The system's document picker answers with an activity result, which reaches the activity or
// one of its fragments, never the native code. This fragment exists only while a picker is
// open: it starts the picker, receives the result and removes itself. What the app gets are
// file paths it can use with ordinary file calls:
// - an opened document is copied into the app's cache, and the copy's path is returned;
// - a document to save is a new file in the app's cache, and every time the app finishes
//   writing it (closes it), its contents are copied into the chosen document.

package dev.remedyui.android;

import android.app.Activity;
import android.app.Fragment;
import android.app.FragmentManager;
import android.content.ContentResolver;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.FileObserver;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.Map;

public final class Picker extends Fragment {
    private static final String TAG = "dev.remedyui.android.Picker";
    /** Save targets the app writes to, kept watched for the life of the process. */
    private static final Map<String, FileObserver> WATCHED = new HashMap<>();

    private long handle;
    private long request;
    private Intent intent;
    private boolean save;
    private boolean started;

    /** Android recreates fragments by name; this one is never restored (see Bridge). */
    public Picker() {}

    /** Shows the picker: `save` to create a document, otherwise to open one or several. */
    static void show(Activity activity, long handle, long request, boolean save, boolean multiple,
            String[] extensions, String fileName, String title) {
        // The picker filters by type: each extension's MIME type, or everything when one is
        // unknown (a filter that hid files the app accepts would be worse than none).
        ArrayList<String> types = new ArrayList<>();
        for (String ext : extensions) {
            String t = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase(java.util.Locale.ROOT));
            if (t == null) {
                types.clear();
                break;
            }
            if (!types.contains(t)) {
                types.add(t);
            }
        }
        String[] mimeTypes = types.toArray(new String[0]);
        Intent i = new Intent(save ? Intent.ACTION_CREATE_DOCUMENT : Intent.ACTION_OPEN_DOCUMENT);
        i.addCategory(Intent.CATEGORY_OPENABLE);
        if (mimeTypes.length == 1) {
            i.setType(mimeTypes[0]);
        } else {
            i.setType("*/*");
            if (mimeTypes.length > 1) {
                i.putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes);
            }
        }
        if (save && fileName != null && !fileName.isEmpty()) {
            i.putExtra(Intent.EXTRA_TITLE, fileName);
        }
        if (!save && multiple) {
            i.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        }
        Picker p = new Picker();
        p.handle = handle;
        p.request = request;
        p.intent = i;
        p.save = save;
        FragmentManager fm = activity.getFragmentManager();
        fm.beginTransaction().add(p, TAG + request).commitNow();
    }

    @Override
    public void onStart() {
        super.onStart();
        if (!started && intent != null) {
            started = true;
            try {
                startActivityForResult(intent, 1);
            } catch (RuntimeException e) {
                finish(new String[0], "no document picker: " + e);
            }
        }
    }

    @Override
    public void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (resultCode != Activity.RESULT_OK || data == null) {
            finish(new String[0], null);
            return;
        }
        ArrayList<Uri> uris = new ArrayList<>();
        if (data.getClipData() != null) {
            for (int i = 0; i < data.getClipData().getItemCount(); i++) {
                uris.add(data.getClipData().getItemAt(i).getUri());
            }
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        ContentResolver cr = getActivity().getContentResolver();
        File dir = new File(getActivity().getCacheDir(), "documents/" + request);
        dir.mkdirs();
        ArrayList<String> paths = new ArrayList<>();
        String error = null;
        for (Uri u : uris) {
            try {
                File f = new File(dir, safeName(displayName(cr, u)));
                if (save) {
                    f.createNewFile();
                    watch(cr, f, u);
                } else {
                    try (InputStream in = cr.openInputStream(u); OutputStream out = new FileOutputStream(f)) {
                        copy(in, out);
                    }
                }
                paths.add(f.getAbsolutePath());
            } catch (Exception e) {
                error = "the document could not be read or written: " + e;
            }
        }
        finish(paths.toArray(new String[0]), error);
    }

    private void finish(String[] paths, String error) {
        Bridge.dialogDone(handle, request, paths, error);
        if (isAdded()) {
            getFragmentManager().beginTransaction().remove(this).commitAllowingStateLoss();
        }
    }

    /** Copies the cache file into the document each time the app closes it after writing. */
    private static void watch(final ContentResolver cr, final File f, final Uri target) {
        FileObserver o = new FileObserver(f.getAbsolutePath(), FileObserver.CLOSE_WRITE) {
            @Override
            public void onEvent(int event, String path) {
                try (InputStream in = new FileInputStream(f); OutputStream out = cr.openOutputStream(target, "wt")) {
                    copy(in, out);
                    Bridge.documentSaved(f.getAbsolutePath(), null);
                } catch (Exception e) {
                    Bridge.documentSaved(f.getAbsolutePath(), e.toString());
                }
            }
        };
        o.startWatching();
        synchronized (WATCHED) {
            WATCHED.put(f.getAbsolutePath(), o);
        }
    }

    private static String displayName(ContentResolver cr, Uri u) {
        try (Cursor c = cr.query(u, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                return c.getString(0);
            }
        } catch (RuntimeException e) {
            // Fall through to the last path segment.
        }
        String last = u.getLastPathSegment();
        return last != null ? last : "document";
    }

    private static String safeName(String name) {
        String n = name.replace('/', '_').replace('\0', '_');
        return n.isEmpty() || n.equals(".") || n.equals("..") ? "document" : n;
    }

    private static void copy(InputStream in, OutputStream out) throws java.io.IOException {
        byte[] buf = new byte[64 * 1024];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
    }
}
