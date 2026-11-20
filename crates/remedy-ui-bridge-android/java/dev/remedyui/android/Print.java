// Printing: the page is laid out by a WebView that is never shown, and the system's print
// manager prints it (printers, Save as PDF) through the WebView's print adapter.

package dev.remedyui.android;

import android.app.Activity;
import android.content.Context;
import android.print.PrintAttributes;
import android.print.PrintManager;
import android.webkit.WebView;
import android.webkit.WebViewClient;

final class Print {
    /** The page being printed: the print adapter needs its WebView alive until it is done. */
    private static WebView current;

    private Print() {}

    /** Loads `html` and opens the print dialog once it has loaded (UI thread). */
    static void show(Activity activity, String title, String html) {
        final WebView view = new WebView(activity);
        view.setWebViewClient(new WebViewClient() {
            private boolean started;

            @Override
            public void onPageFinished(WebView v, String url) {
                if (started) {
                    return;
                }
                started = true;
                PrintManager pm = (PrintManager) activity.getSystemService(Context.PRINT_SERVICE);
                if (pm != null) {
                    pm.print(title, v.createPrintDocumentAdapter(title), new PrintAttributes.Builder().build());
                }
            }
        });
        view.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null);
        current = view;
    }
}
