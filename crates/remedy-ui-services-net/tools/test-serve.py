#!/usr/bin/env python3
"""The server the HTTP client's browser test (`tests/web.rs`) talks to, with any page allowed
to read it. Prints the port it listens on (a free one on 127.0.0.1), then serves until killed.

    tools/test-serve.py

/json answers JSON, /error a 422 with a detail, /echo the method, content type and body it
got, /slow the same JSON after 1.5 seconds.
"""

import http.server
import json
import time


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def answer(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def handle_any(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        path = self.path.split("?")[0]
        if path == "/json":
            self.answer(200, {"id": 7, "name": "seven"})
        elif path == "/error":
            self.answer(422, {"detail": "the name is taken"})
        elif path == "/echo":
            self.answer(200, {
                "method": self.command,
                "type": self.headers.get("Content-Type") or "",
                "body": body.decode("utf-8", "replace"),
            })
        elif path == "/slow":
            time.sleep(1.5)
            self.answer(200, {"id": 1, "name": "slow"})
        else:
            self.answer(404, {})

    def do_OPTIONS(self):
        self.answer(200, None)

    do_GET = do_POST = do_PUT = do_DELETE = handle_any

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
