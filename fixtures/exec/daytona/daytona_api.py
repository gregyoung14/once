from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse
import json
import pathlib
import shlex
import sys


deleted_marker = pathlib.Path(sys.argv[1])
sandbox = None
files = {}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def read_body(self):
        length = int(self.headers.get("Content-Length", "0"))
        return self.rfile.read(length)

    def respond(self, status, value=None):
        body = b"" if value is None else json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def authorized(self):
        if self.headers.get("Authorization") == "Bearer token-1":
            return True
        self.respond(401)
        return False

    def do_GET(self):
        if not self.authorized():
            return
        url = urlparse(self.path)
        query = parse_qs(url.query)
        if url.path == "/api/sandbox":
            labels = json.loads(query.get("labels", ["{}"])[0])
            matches = sandbox is not None and all(
                sandbox["labels"].get(key) == value for key, value in labels.items()
            )
            self.respond(200, [sandbox] if matches else [])
            return
        if url.path == "/api/sandbox/sandbox-1":
            self.respond(200, sandbox) if sandbox is not None else self.respond(404)
            return
        if url.path == "/toolbox/sandbox-1/files/download":
            path = query.get("path", [""])[0]
            if path not in files:
                self.respond(404)
                return
            body = files[path]
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.respond(404)

    def do_POST(self):
        global sandbox
        if not self.authorized():
            return
        path = urlparse(self.path).path
        if path == "/api/sandbox":
            request = json.loads(self.read_body())
            if request.get("autoDeleteInterval") != 0:
                self.respond(400)
                return
            sandbox = {
                "id": "sandbox-1", "state": "started", "toolboxProxyUrl": None,
                "labels": request.get("labels", {}),
            }
            self.respond(200, sandbox)
            return
        if path == "/toolbox/sandbox-1/files/upload":
            self.read_body()
            self.respond(200)
            return
        if path == "/toolbox/sandbox-1/process/execute":
            request = json.loads(self.read_body())
            if request["command"].startswith("mkdir "):
                self.respond(200, {"exitCode": 0, "result": ""})
            else:
                value = request.get("envs", {}).get("VALUE", "missing")
                argv = shlex.split(request["command"])
                files[argv[argv.index(">") + 1]] = f"daytona-{value}".encode()
                files[argv[argv.index("2>") + 1]] = b""
                self.respond(200, {"exitCode": 0, "result": ""})
            return
        self.respond(404)

    def do_DELETE(self):
        global sandbox
        if not self.authorized():
            return
        if urlparse(self.path).path != "/api/sandbox/sandbox-1":
            self.respond(404)
            return
        deleted_marker.write_text("deleted")
        sandbox = None
        self.respond(204)


server = HTTPServer(("127.0.0.1", 0), Handler)
print(server.server_port, flush=True)
server.serve_forever()
