#!/usr/bin/env python3
"""Serves one file over HTTP slowly, for the browser test of what a download has brought
(`tests/web.rs`): CHUNK bytes (1 KB), then a pause (100 ms), with the length stated and any
page allowed to read it. The clip takes about 3 s: long enough that a page busy for a moment
(which hands over what queued meanwhile as one chunk) still sees it arrive in steps. Prints the port it listens on (a free one on 127.0.0.1), then serves until killed.

    tools/slow-serve.py FILE [CHUNK_BYTES] [PAUSE_MS]
"""

import http.server
import sys
import time

path = sys.argv[1]
chunk = int(sys.argv[2]) if len(sys.argv) > 2 else 1024
pause = (int(sys.argv[3]) if len(sys.argv) > 3 else 100) / 1000.0
with open(path, "rb") as f:
    data = f.read()


class Slow(http.server.BaseHTTPRequestHandler):
    def head(self):
        self.send_response(200)
        self.send_header("Content-Type", "video/mp4")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()

    def do_HEAD(self):
        self.head()

    def do_GET(self):
        self.head()
        for at in range(0, len(data), chunk):
            self.wfile.write(data[at : at + chunk])
            self.wfile.flush()
            time.sleep(pause)

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Slow)
print(server.server_address[1], flush=True)
server.serve_forever()
