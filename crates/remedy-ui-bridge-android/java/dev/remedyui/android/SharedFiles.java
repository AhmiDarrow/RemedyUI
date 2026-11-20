// Serves the files of a share to the app the person chose, read-only.
//
// Other apps cannot read an app's own files by path; they read them through a content URI
// that the app serves and that the share grants them. This provider is that server, with no
// AndroidX: `cargo remedy package` puts it in the APK's own classes.dex (the rest of the
// bridge's Java is loaded from memory, which a provider cannot be) and declares it, not
// exported, with per-URI grants, under the authority <package>.remedy.share. It serves only
// files under the app's cache folder remedy-share, at
// content://<authority>/<request>/<index>/<MIME type>/<name>.

package dev.remedyui.android;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.List;

public final class SharedFiles extends ContentProvider {
    /** The cache folder whose files are served. */
    static final String FOLDER = "remedy-share";

    @Override
    public boolean onCreate() {
        return true;
    }

    /** The file `uri` names, or why it is not served. */
    private File file(Uri uri) throws FileNotFoundException {
        List<String> parts = uri.getPathSegments();
        if (parts.size() != 4 || getContext() == null) {
            throw new FileNotFoundException(uri.toString());
        }
        File root = new File(getContext().getCacheDir(), FOLDER);
        File f = new File(new File(new File(root, parts.get(0)), parts.get(1)), parts.get(3));
        try {
            // Nothing outside the share folder, whatever the segments hold.
            if (!f.getCanonicalPath().startsWith(root.getCanonicalPath() + File.separator) || !f.isFile()) {
                throw new FileNotFoundException(uri.toString());
            }
        } catch (IOException e) {
            throw new FileNotFoundException(uri + ": " + e);
        }
        return f;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new SecurityException("shared files are read-only");
        }
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs,
            String sortOrder) {
        File f;
        try {
            f = file(uri);
        } catch (FileNotFoundException e) {
            return null;
        }
        String[] columns = projection != null
                ? projection
                : new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        int n = 0;
        String[] cols = new String[columns.length];
        Object[] values = new Object[columns.length];
        for (String c : columns) {
            if (OpenableColumns.DISPLAY_NAME.equals(c)) {
                cols[n] = c;
                values[n++] = f.getName();
            } else if (OpenableColumns.SIZE.equals(c)) {
                cols[n] = c;
                values[n++] = f.length();
            }
        }
        String[] outCols = new String[n];
        Object[] outValues = new Object[n];
        System.arraycopy(cols, 0, outCols, 0, n);
        System.arraycopy(values, 0, outValues, 0, n);
        MatrixCursor cursor = new MatrixCursor(outCols, 1);
        cursor.addRow(outValues);
        return cursor;
    }

    @Override
    public String getType(Uri uri) {
        List<String> parts = uri.getPathSegments();
        return parts.size() == 4 ? parts.get(2) : null;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("shared files are read-only");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("shared files are read-only");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("shared files are read-only");
    }
}
