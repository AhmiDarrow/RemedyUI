#!/usr/bin/env python3
"""Serves a directory over HTTP with any page allowed to read it, for the browser test of HLS
(`tests/web.rs`: a playlist and its segments, fetched by URL). Prints the port it listens on
(a free one on 127.0.0.1, or on HOST: the machine's LAN address for a page that is not a secure
context, `tests/web_mse.rs`), then serves until killed.

    tools/serve-dir.py DIR [HOST]
"""

import functools
import http.server
import sys


class Open(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *args):
        pass


handler = functools.partial(Open, directory=sys.argv[1])
host = sys.argv[2] if len(sys.argv) > 2 else "127.0.0.1"
server = http.server.ThreadingHTTPServer((host, 0), handler)
print(server.server_address[1], flush=True)
server.serve_forever()
