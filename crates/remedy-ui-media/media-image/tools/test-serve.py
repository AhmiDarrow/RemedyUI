#!/usr/bin/env python3
"""Serves images for the browser test of images from addresses (`tests/media_image/web.rs`): one any page
may read (CORS), one without CORS headers, an image proxy that serves it with CORS (`/proxy?url=`), and
one that trickles for minutes. Prints the port
it listens on (a free one on 127.0.0.1), then serves until killed.

    media-image/tools/test-serve.py
"""

import http.server
import struct
import time
import zlib


def png(w, h):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    rows = b"".join(b"\0" + b"\xc0\x40\x20\xff" * w for _ in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


WIDE = png(30, 10)
STATE = {"slow": ""}


class Images(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/proxy" and "nocors.png" in self.path:
            # What an app's own image proxy does: the bytes, readable by the page.
            path = "/cors.png"
        if path in ("/cors.png", "/nocors.png"):
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(WIDE)))
            if path == "/cors.png":
                self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(WIDE)
        elif path == "/slow.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(WIDE) * 1000))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            try:
                for _ in range(1000):
                    self.wfile.write(b"\0" * 16)
                    self.wfile.flush()
                    time.sleep(0.05)
            except OSError:
                # The client let go: what `/status` reports.
                STATE["slow"] = "cut"
        elif path == "/status":
            body = STATE["slow"].encode()
            self.send_response(200)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Content-Length", "0")
            self.end_headers()

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Images)
server.daemon_threads = True
print(server.server_address[1], flush=True)
server.serve_forever()
