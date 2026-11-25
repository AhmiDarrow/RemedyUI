// The isolated service images are decoded in (remedy-ui-media::media_isolated): declared in the
// manifest with android:isolatedProcess="true", so each instance runs in a process of its own
// under a fresh user id with no permissions: no network, none of the app's files, no other
// apps' services.
//
// The app starts one per decoding helper (start, with an instance name of its own) and hands it
// two descriptors through the binder: one end of a Unix socket requests and replies go over
// (shared memory travels on it as descriptors), and the write end of a pipe for its standard
// error. The service loads the app's native library and runs the helper on a thread of its
// own (run, in remedy-ui-media::media_isolated), which ends the process when the app closes the
// socket. Stopping a helper is unbinding it: the system kills an isolated process nobody is
// bound to.

package dev.remedyui.android;

import android.app.Service;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.os.Binder;
import android.os.Build;
import android.os.IBinder;
import android.os.Parcel;
import android.os.ParcelFileDescriptor;
import android.os.RemoteException;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

public final class Decoder extends Service {
    private static final int START = IBinder.FIRST_CALL_TRANSACTION;

    /** The helper's loop, in the app's native library: never returns (it ends the process). */
    private static native void run(int socket, int stderr);

    private boolean started;

    @Override
    public IBinder onBind(Intent intent) {
        return new Binder() {
            @Override
            protected boolean onTransact(int code, Parcel data, Parcel reply, int flags)
                    throws RemoteException {
                if (code != START) {
                    return super.onTransact(code, data, reply, flags);
                }
                // Only the app itself, and only once per process.
                if (Binder.getCallingUid() != getApplicationInfo().uid) {
                    throw new SecurityException("only the app may start its decoder");
                }
                synchronized (Decoder.this) {
                    if (started) {
                        throw new IllegalStateException("this decoder is already running");
                    }
                    started = true;
                }
                String library = data.readString();
                ParcelFileDescriptor socket = data.readFileDescriptor();
                ParcelFileDescriptor stderr = data.readFileDescriptor();
                if (library == null || socket == null || stderr == null) {
                    throw new IllegalArgumentException("a decoder needs its library and descriptors");
                }
                System.loadLibrary(library);
                int s = socket.detachFd();
                int e = stderr.detachFd();
                new Thread(() -> run(s, e), "remedy-decoder").start();
                return true;
            }
        };
    }

    /** One helper's binding, seen from the app. */
    private static final class Link implements ServiceConnection {
        final Context context;
        final CountDownLatch up = new CountDownLatch(1);
        volatile IBinder binder;
        boolean bound = true;

        Link(Context context) {
            this.context = context;
        }

        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
            binder = service;
            up.countDown();
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {}

        @Override
        public void onBindingDied(ComponentName name) {
            up.countDown();
        }

        @Override
        public void onNullBinding(ComponentName name) {
            up.countDown();
        }
    }

    /**
     * Starts the helper `instance` (a new isolated process) running `library`'s decoder on the
     * two descriptors, which are duplicated for it (the caller keeps and closes its own).
     * Waits up to `waitMs` for the process to come up. Called from any thread but the main one.
     */
    public static ServiceConnection start(
            Context context, String instance, String library, int socket, int stderr, long waitMs)
            throws Exception {
        if (Build.VERSION.SDK_INT < 29) {
            throw new UnsupportedOperationException(
                    "isolated decoding needs Android 10 (an isolated service process per helper)");
        }
        Context app = context.getApplicationContext() != null ? context.getApplicationContext() : context;
        Link link = new Link(app);
        Intent intent = new Intent(app, Decoder.class);
        if (!app.bindIsolatedService(intent, Context.BIND_AUTO_CREATE, instance, Runnable::run, link)) {
            stop(link);
            throw new IllegalStateException(
                    "the decoding service could not be bound: the manifest must declare "
                            + "dev.remedyui.android.Decoder with android:isolatedProcess=\"true\"");
        }
        if (!link.up.await(waitMs, TimeUnit.MILLISECONDS) || link.binder == null) {
            stop(link);
            throw new IllegalStateException("the decoding service did not come up");
        }
        Parcel data = Parcel.obtain();
        try (ParcelFileDescriptor s = ParcelFileDescriptor.fromFd(socket);
                ParcelFileDescriptor e = ParcelFileDescriptor.fromFd(stderr)) {
            data.writeString(library);
            data.writeFileDescriptor(s.getFileDescriptor());
            data.writeFileDescriptor(e.getFileDescriptor());
            link.binder.transact(START, data, null, 0);
        } catch (Exception e) {
            stop(link);
            throw e;
        } finally {
            data.recycle();
        }
        return link;
    }

    /** Stops a helper: unbound, its process is killed. Safe to call more than once. */
    public static void stop(ServiceConnection connection) {
        Link link = (Link) connection;
        synchronized (link) {
            if (!link.bound) {
                return;
            }
            link.bound = false;
        }
        try {
            link.context.unbindService(link);
        } catch (IllegalArgumentException alreadyGone) {
            // Never bound, or already unbound by the system.
        }
    }
}
