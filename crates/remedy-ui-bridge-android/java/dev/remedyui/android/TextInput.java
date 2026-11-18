// Input methods for RemedyUI text fields on a native activity.
//
// A native activity's own view gives input methods no InputConnection, so they can only send
// key events: no composition, no suggestions, no reconversion. This invisible view sits over
// it and takes focus while a RemedyUI field edits. It keeps a copy of the field's text around
// the caret (what the input method reads) and reports what the input method does to it as
// preedit, commit and delete-surrounding events. Key events the input method sends still go
// to the native input queue (the window's input belongs to the native activity).

package dev.remedyui.android;

import android.content.Context;
import android.graphics.Matrix;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.CursorAnchorInfo;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import java.nio.charset.StandardCharsets;

final class TextInput extends View {
    private final long handle;
    private final InputMethodManager imm;
    private final Editable editable = Editable.Factory.getInstance().newEditable("");
    private boolean active;
    private boolean secret;
    private boolean multiline;
    private float caretX;
    private float caretTop;
    private float caretBottom;
    private boolean cursorUpdates;

    TextInput(Context context, long handle) {
        super(context);
        this.handle = handle;
        this.imm = (InputMethodManager) context.getSystemService(Context.INPUT_METHOD_SERVICE);
        setFocusable(true);
        setFocusableInTouchMode(true);
        setWillNotDraw(true);
        setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
    }

    private static int utf8(CharSequence s) {
        return s.toString().getBytes(StandardCharsets.UTF_8).length;
    }

    /** Starts or updates a session with the field's text around the caret (UTF-16 offsets). */
    void update(String text, int selStart, int selEnd, float x, float top, float bottom, boolean secret, boolean multiline) {
        caretX = x;
        caretTop = top;
        caretBottom = bottom;
        boolean restart = !active || secret != this.secret || multiline != this.multiline;
        this.secret = secret;
        this.multiline = multiline;
        boolean composing = BaseInputConnection.getComposingSpanStart(editable) >= 0;
        boolean textChanged = !editable.toString().equals(text);
        // The field changed without the input method (keys, the app): the copy follows, and
        // the input method hears of it as a selection change, as an EditText's would. A new
        // session (or new kind of field) restarts it.
        if (restart || (!composing && textChanged)) {
            editable.replace(0, editable.length(), text);
            BaseInputConnection.removeComposingSpans(editable);
        }
        int len = editable.length();
        int s = Math.max(0, Math.min(selStart, len));
        int e = Math.max(0, Math.min(selEnd, len));
        boolean moved = Selection.getSelectionStart(editable) != s || Selection.getSelectionEnd(editable) != e;
        if (!composing && moved) {
            Selection.setSelection(editable, s, e);
        }
        if (restart) {
            active = true;
            requestFocus();
            imm.restartInput(this);
            imm.showSoftInput(this, 0);
        } else if (!composing && (moved || textChanged)) {
            imm.updateSelection(this, s, e, -1, -1);
        }
        if (cursorUpdates) {
            sendCursorAnchor();
        }
    }

    /** The app committed the composition itself (a key typed outside the input method). */
    void finishComposition() {
        if (!active || BaseInputConnection.getComposingSpanStart(editable) < 0) {
            return;
        }
        BaseInputConnection.removeComposingSpans(editable);
        imm.restartInput(this);
    }

    /** Ends the session and puts the keyboard away. */
    void end() {
        if (!active) {
            return;
        }
        active = false;
        editable.clear();
        imm.hideSoftInputFromWindow(getWindowToken(), 0);
        clearFocus();
        imm.restartInput(this);
    }

    private void sendCursorAnchor() {
        int[] at = new int[2];
        getLocationOnScreen(at);
        Matrix m = new Matrix();
        m.setTranslate(at[0], at[1]);
        CursorAnchorInfo.Builder b = new CursorAnchorInfo.Builder();
        b.setMatrix(m);
        b.setSelectionRange(Selection.getSelectionStart(editable), Selection.getSelectionEnd(editable));
        b.setInsertionMarkerLocation(caretX, caretTop, caretBottom, caretBottom, 0);
        imm.updateCursorAnchorInfo(this, b.build());
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return active;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo out) {
        if (!active) {
            return null;
        }
        int type = InputType.TYPE_CLASS_TEXT;
        if (secret) {
            type |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
        } else {
            type |= InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
        }
        if (multiline) {
            type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE;
        }
        out.inputType = type;
        out.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                | (multiline ? EditorInfo.IME_ACTION_NONE : EditorInfo.IME_ACTION_DONE)
                | (secret ? EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING : 0);
        out.initialSelStart = Selection.getSelectionStart(editable);
        out.initialSelEnd = Selection.getSelectionEnd(editable);
        return new Connection();
    }

    /** What the input method does, applied to the copy and reported to the app. */
    private final class Connection extends BaseInputConnection {
        Connection() {
            super(TextInput.this, true);
        }

        @Override
        public Editable getEditable() {
            return editable;
        }

        private String composing() {
            int a = getComposingSpanStart(editable);
            int b = getComposingSpanEnd(editable);
            if (a < 0 || b < 0) {
                return "";
            }
            return editable.subSequence(Math.min(a, b), Math.max(a, b)).toString();
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            boolean ok = super.setComposingText(text, newCursorPosition);
            String t = text.toString();
            Bridge.imePreedit(handle, t, newCursorPosition > 0 ? utf8(t) : 0);
            return ok;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            boolean ok = super.commitText(text, newCursorPosition);
            Bridge.imeCommit(handle, text.toString());
            return ok;
        }

        @Override
        public boolean finishComposingText() {
            String c = composing();
            boolean ok = super.finishComposingText();
            if (!c.isEmpty()) {
                Bridge.imeCommit(handle, c);
            }
            return ok;
        }

        @Override
        public boolean setComposingRegion(int start, int end) {
            // Text already in the field becomes the composition (the input method went back
            // into a word): it leaves the field and returns as preedit.
            int a = Math.max(0, Math.min(Math.min(start, end), editable.length()));
            int b = Math.max(0, Math.min(Math.max(start, end), editable.length()));
            int caret = Selection.getSelectionEnd(editable);
            String region = editable.subSequence(a, b).toString();
            boolean ok = super.setComposingRegion(start, end);
            if (b > a && caret >= a && caret <= b) {
                int before = utf8(editable.subSequence(a, caret));
                int after = utf8(editable.subSequence(caret, b));
                Bridge.imeDelete(handle, before, after);
                Bridge.imePreedit(handle, region, before);
            }
            return ok;
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            int s = Selection.getSelectionStart(editable);
            int e = Selection.getSelectionEnd(editable);
            int a = Math.max(0, Math.min(s, e) - beforeLength);
            int b = Math.min(editable.length(), Math.max(s, e) + afterLength);
            int before = utf8(editable.subSequence(a, Math.min(s, e)));
            int after = utf8(editable.subSequence(Math.max(s, e), b));
            boolean ok = super.deleteSurroundingText(beforeLength, afterLength);
            if (before > 0 || after > 0) {
                Bridge.imeDelete(handle, before, after);
            }
            return ok;
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
            int s = Math.min(Selection.getSelectionStart(editable), Selection.getSelectionEnd(editable));
            int e = Math.max(Selection.getSelectionStart(editable), Selection.getSelectionEnd(editable));
            String text = editable.toString();
            int a = s;
            for (int i = 0; i < beforeLength && a > 0; i++) {
                a = text.offsetByCodePoints(a, -1);
            }
            int b = e;
            for (int i = 0; i < afterLength && b < text.length(); i++) {
                b = text.offsetByCodePoints(b, 1);
            }
            return deleteSurroundingText(s - a, b - e);
        }

        @Override
        public boolean requestCursorUpdates(int mode) {
            cursorUpdates = (mode & CURSOR_UPDATE_MONITOR) != 0;
            if ((mode & CURSOR_UPDATE_IMMEDIATE) != 0) {
                sendCursorAnchor();
            }
            return true;
        }
    }
}
