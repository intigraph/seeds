# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Synthetic CLI demo. Set BEFORE_SD, AFTER_SD, QUIPU_CLI, QUIPU_SERVER_BINARY.

Run with uv run --no-project docs/pr-demos/bounded-search.py. Uses only private
temporary files and loopback processes. Measures requests and result parity,
not production memory. Each CLI call has a 20-request upstream fence.
"""
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def run():
    binaries = {key: str(Path(os.environ[key]).resolve()) for key in
                ("BEFORE_SD", "AFTER_SD", "QUIPU_CLI", "QUIPU_SERVER_BINARY")}
    with tempfile.TemporaryDirectory(prefix="bounded-search-demo-") as directory:
        root = Path(directory)
        config = root / ".bobbin/config.toml"
        config.parent.mkdir(mode=0o700)
        config.write_text('[quipu.embedding]\nauto_embed = false\n'
                          '[quipu.server]\nauth_token = "' + secrets.token_hex(32) + '"\n')
        env = {"HOME": str(root), "XDG_CONFIG_HOME": str(root / ".config"),
               "XDG_CACHE_HOME": str(root / ".cache"), "PATH": os.defpath, "LANG": "C.UTF-8"}
        graph = "https://seeds.local/project/bounded-search-demo"
        count = 3000
        lines = []
        for index in range(count):
            sid = f"demo-{index:04d}"
            title = "needle" + (" uniqueterm" if index == 0 else "")
            lines.append(f'<https://seeds.local/item/{sid}> '
                         'a <https://schema.org/Action> ; '
                         f'<https://schema.org/identifier> "{sid}" ; '
                         f'<https://schema.org/name> "{title}" ; '
                         '<https://seeds.local/ontology/status> "open" .\n')
        fixture = root / "fixture.ttl"
        data = "".join(lines).encode()
        fixture.write_bytes(data)
        db = root / "demo.db"

        def prepare(*arguments):
            result = subprocess.run([binaries["QUIPU_CLI"], *arguments, "--db", str(db)],
                                    cwd=root, env=env, capture_output=True, text=True, timeout=90)
            if result.returncode:
                raise RuntimeError("owned fixture preparation refused; inspect local CLI configuration")
            return result.stdout

        prepare("ingest", str(fixture), "--graph", graph, "--format", "ttl",
                "--timestamp", "2026-01-01T00:00:00Z", "--declare-count", str(count * 4),
                "--declare-sha256", hashlib.sha256(data).hexdigest(), "--chunk", "10000")
        # Two explicit batches cover the declared fixture; never retry a refusal.
        for _ in range(2):
            prepare("search-index", "backfill", "--kind", "substring", "--batch-size", "10000")
        assert json.loads(prepare("search-index", "status", "--kind", "substring"))["ready"]
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        log = (root / "server.log").open("wb")
        server = subprocess.Popen([binaries["QUIPU_SERVER_BINARY"], "--db", str(db),
                                   "--bind", f"127.0.0.1:{port}"], cwd=root, env=env,
                                  stdout=log, stderr=subprocess.STDOUT)
        opener = urllib.request.build_opener(NoRedirect)
        try:
            deadline = time.monotonic() + 15
            while True:
                try:
                    with opener.open(f"http://127.0.0.1:{port}/health", timeout=1):
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise RuntimeError("owned server did not become ready") from None
                    time.sleep(.05)

            def search(binary, query):
                budget = {"attempted": 0, "forwarded": 0}
                lock = threading.Lock()

                class Proxy(BaseHTTPRequestHandler):
                    def log_message(self, *_args):
                        pass

                    def forward(self):
                        if not ((self.command == "POST" and self.path == "/query") or
                                (self.command == "GET" and self.path in ("/health", "/version"))):
                            self.send_error(405)
                            return
                        with lock:
                            budget["attempted"] += 1
                            if budget["forwarded"] >= 20:
                                self.send_error(429, "request fence; request 21 not forwarded")
                                return
                            budget["forwarded"] += 1
                        size = int(self.headers.get("Content-Length", 0))
                        assert 0 <= size <= 256000
                        body = self.rfile.read(size) if self.command == "POST" else None
                        headers = {k: v for k, v in self.headers.items()
                                   if k.lower() in ("accept", "content-type") or
                                   k.lower().startswith("x-quipu-")}
                        request = urllib.request.Request(f"http://127.0.0.1:{port}{self.path}",
                                                         data=body, headers=headers, method=self.command)
                        try:
                            response = opener.open(request, timeout=35)
                        except urllib.error.HTTPError as error:
                            response = error
                        with response:
                            payload = response.read(16 * 1024 * 1024 + 1)
                            assert len(payload) <= 16 * 1024 * 1024
                            self.send_response(response.status)
                            for key, value in response.headers.items():
                                if key.lower() == "content-type" or key.lower().startswith("x-quipu-"):
                                    self.send_header(key, value)
                            self.send_header("Content-Length", str(len(payload)))
                            self.end_headers()
                            self.wfile.write(payload)

                    do_POST = forward
                    do_GET = forward

                transport = ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
                worker = threading.Thread(target=transport.serve_forever, daemon=True)
                worker.start()
                try:
                    result = subprocess.run([binary, "--quipu",
                        f"http://127.0.0.1:{transport.server_port}", "--graph", graph,
                        "search", query, "--full", "--all", "--limit", "25", "--json"],
                        cwd=root, env=env, capture_output=True, text=True, timeout=45)
                finally:
                    transport.shutdown()
                    transport.server_close()
                    worker.join()
                return result, budget

            print("Synthetic fixture: 3000 Actions, complete substring index.", flush=True)
            for label in ("BEFORE_SD", "AFTER_SD"):
                for query, expected in (("zzzz-control-miss", 0), ("uniqueterm", 1)):
                    result, budget = search(binaries[label], query)
                    assert result.returncode == 0
                    assert json.loads(result.stdout)["total"] == expected
                    print(f"{label}: {query}: total={expected}, requests={budget['forwarded']}", flush=True)
                print(f"{label}: sd search needle --full --all --limit 25 --json", flush=True)
                result, budget = search(binaries[label], "needle")
                if label == "BEFORE_SD":
                    assert result.returncode != 0 and budget["attempted"] >= 21 and budget["forwarded"] == 20
                    print(f"REFUSED: attempted={budget['attempted']}, forwarded=20; no partial result accepted.", flush=True)
                else:
                    assert result.returncode == 0
                    actual = json.loads(result.stdout)
                    assert actual["total"] == count and actual["hidden_closed_count"] == 0
                    assert [row["id"] for row in actual["issues"]] == [f"demo-{i:04d}" for i in range(25)]
                    print(f"PASS: total={count}, page=25, exact oracle, requests={budget['forwarded']}", flush=True)
        finally:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
            log.close()


if __name__ == "__main__":
    run()
