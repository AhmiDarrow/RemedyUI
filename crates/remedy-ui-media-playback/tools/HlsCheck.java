// Runs examples/android-hls-check.rs inside app_process (a real Java VM): serves an HLS fixture
// over https on 127.0.0.1 with a self-signed certificate that this process trusts, loads the
// library and calls its run(port). Paths under /plain/ answer at once, under /slow/ after 5 s,
// and under /stall/ the second segment stops half way for 4 s the first time it is asked for.
// Prints "hls-https-check: PASSED" when every check and every request count holds.
//
//   app_process / HlsCheck <lib.so> <fixture dir> <keystore.p12> <password>

import java.io.*;
import java.net.*;
import java.security.KeyStore;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;
import javax.net.ssl.*;

public class HlsCheck {
    static native int run(int port);

    static final Map<String, AtomicInteger> asked = new ConcurrentHashMap<>();
    static final Set<String> stalled = ConcurrentHashMap.newKeySet();

    public static void main(String[] args) throws Exception {
        File root = new File(args[1]);
        char[] pass = args[3].toCharArray();
        KeyStore ks = KeyStore.getInstance("PKCS12");
        try (InputStream in = new FileInputStream(args[2])) {
            ks.load(in, pass);
        }
        KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
        kmf.init(ks, pass);
        TrustManagerFactory tmf = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
        tmf.init(ks);
        SSLContext ctx = SSLContext.getInstance("TLS");
        ctx.init(kmf.getKeyManagers(), tmf.getTrustManagers(), null);
        // The client (HttpURLConnection, from the native side) trusts this certificate only.
        HttpsURLConnection.setDefaultSSLSocketFactory(ctx.getSocketFactory());

        SSLServerSocket server = (SSLServerSocket) ctx.getServerSocketFactory()
                .createServerSocket(0, 50, InetAddress.getByName("127.0.0.1"));
        Thread accept = new Thread(() -> {
            while (true) {
                try {
                    Socket s = server.accept();
                    new Thread(() -> serve(s, root)).start();
                } catch (IOException e) {
                    return;
                }
            }
        });
        accept.setDaemon(true);
        accept.start();

        System.load(args[0]);
        int failed = run(server.getLocalPort());
        failed += expect("slow/aac/index.m3u8", 1);
        failed += expect("slow/aac/seg00000.ts", 1);
        failed += expect("stall/aac/seg00001.ts", 2);
        System.err.println("hls-https-check: requests " + asked);
        System.err.println("hls-https-check: " + (failed == 0 ? "PASSED" : "FAILED (" + failed + ")"));
        System.exit(failed == 0 ? 0 : 1);
    }

    static int expect(String path, int times) {
        AtomicInteger n = asked.get(path);
        int got = n == null ? 0 : n.get();
        if (got == times) return 0;
        System.err.println("hls-https-check: " + path + " asked " + got + " times, not " + times);
        return 1;
    }

    static void serve(Socket s, File root) {
        try (Socket sock = s) {
            BufferedReader r = new BufferedReader(new InputStreamReader(sock.getInputStream(), "ISO-8859-1"));
            String line = r.readLine();
            if (line == null) return;
            String path = line.split(" ")[1].replaceFirst("^/", "");
            while ((line = r.readLine()) != null && !line.isEmpty()) {}
            asked.computeIfAbsent(path, k -> new AtomicInteger()).incrementAndGet();
            String[] parts = path.split("/", 2);
            String mode = parts[0];
            File f = new File(root, parts.length > 1 ? parts[1] : "");
            OutputStream out = sock.getOutputStream();
            if (mode.equals("slow")) Thread.sleep(5000);
            if (!f.isFile()) {
                out.write("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".getBytes("ISO-8859-1"));
                return;
            }
            byte[] body = java.nio.file.Files.readAllBytes(f.toPath());
            out.write(("HTTP/1.1 200 OK\r\nContent-Length: " + body.length + "\r\nConnection: close\r\n\r\n")
                    .getBytes("ISO-8859-1"));
            if (mode.equals("stall") && f.getName().equals("seg00001.ts") && stalled.add(path)) {
                out.write(body, 0, body.length / 2);
                out.flush();
                Thread.sleep(4000);
                out.write(body, body.length / 2, body.length - body.length / 2);
            } else {
                out.write(body);
            }
            out.flush();
        } catch (Exception e) {
            // The client gave up (a stalled download dropped): nothing to answer.
        }
    }
}
