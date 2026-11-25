// Acts on the android-demo app the way a screen reader does: through the accessibility
// framework (UiAutomation), not touches. It clicks "Tap me", focuses the "Message" field and
// asks to set its text, and reports what each action returned. Run by tools/a11y-check/run.sh.

package dev.remedyui.a11ycheck;

import android.app.Activity;
import android.app.Instrumentation;
import android.app.UiAutomation;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.accessibility.AccessibilityNodeInfo;

public final class A11yCheck extends Instrumentation {
    private static final String APP = "dev.remedyui.demo";

    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override
    public void onStart() {
        Bundle out = new Bundle();
        UiAutomation ui = getUiAutomation();
        AccessibilityNodeInfo root = null;
        long deadline = SystemClock.uptimeMillis() + 10000;
        while (SystemClock.uptimeMillis() < deadline) {
            root = ui.getRootInActiveWindow();
            if (root != null && APP.contentEquals(root.getPackageName()) && find(root, "Tap me") != null) {
                break;
            }
            SystemClock.sleep(250);
        }
        if (root == null) {
            out.putString("error", "no window from " + APP);
            finish(Activity.RESULT_CANCELED, out);
            return;
        }
        AccessibilityNodeInfo button = find(root, "Tap me");
        out.putString("tap_me", describe(button));
        if (button != null) {
            out.putBoolean("click", button.performAction(AccessibilityNodeInfo.ACTION_CLICK));
            out.putBoolean("click_again", button.performAction(AccessibilityNodeInfo.ACTION_CLICK));
        }
        SystemClock.sleep(500);
        AccessibilityNodeInfo field = find(ui.getRootInActiveWindow(), "Message");
        out.putString("message", describe(field));
        if (field != null) {
            out.putBoolean("focus", field.performAction(AccessibilityNodeInfo.ACTION_FOCUS));
            Bundle args = new Bundle();
            args.putCharSequence(
                    AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, "set by a screen reader");
            out.putBoolean("set_text", field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args));
        }
        SystemClock.sleep(500);
        AccessibilityNodeInfo after = ui.getRootInActiveWindow();
        AccessibilityNodeInfo taps = findPrefix(after, "Taps:");
        out.putString("taps_label", describe(taps));
        out.putString("message_after", describe(find(after, "Message")));
        finish(Activity.RESULT_OK, out);
    }

    private static String describe(AccessibilityNodeInfo n) {
        if (n == null) {
            return "missing";
        }
        return n.getClassName() + " desc=" + n.getContentDescription() + " text=" + n.getText()
                + " focused=" + n.isFocused() + " actions=" + n.getActionList().size();
    }

    private static AccessibilityNodeInfo find(AccessibilityNodeInfo n, String name) {
        if (n == null) {
            return null;
        }
        if (name.contentEquals(n.getContentDescription() == null ? "" : n.getContentDescription())) {
            return n;
        }
        for (int i = 0; i < n.getChildCount(); i++) {
            AccessibilityNodeInfo hit = find(n.getChild(i), name);
            if (hit != null) {
                return hit;
            }
        }
        return null;
    }

    private static AccessibilityNodeInfo findPrefix(AccessibilityNodeInfo n, String prefix) {
        if (n == null) {
            return null;
        }
        CharSequence d = n.getContentDescription();
        if (d != null && d.toString().startsWith(prefix)) {
            return n;
        }
        for (int i = 0; i < n.getChildCount(); i++) {
            AccessibilityNodeInfo hit = findPrefix(n.getChild(i), prefix);
            if (hit != null) {
                return hit;
            }
        }
        return null;
    }
}
