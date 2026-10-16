#!/usr/bin/env python3
"""Actual CLI + disposable Quipu server; no production endpoint or credentials."""
import http.client
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request


def main():
    before = os.environ["SEEDS_DEMO_BEFORE_SD"]
    after = os.environ["SEEDS_DEMO_AFTER_SD"]
    server = os.environ["SEEDS_DEMO_QUIPU_SERVER"]
    with tempfile.TemporaryDirectory(prefix="seeds-bulk-demo-") as directory:
        root = Path(directory)
        environment = {"HOME": str(root), "XDG_CONFIG_HOME": str(root / "config"),
                       "PATH": os.environ["PATH"], "SEEDS_ACTOR": "demo", "SEEDS_PREFIX": "demo"}
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            port = reserve.getsockname()[1]
        process = subprocess.Popen([server, "--db", str(root / "fixture.db"),
                                    "--bind", f"127.0.0.1:{port}"],
                                   cwd=root, env=environment,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for attempt in range(100):
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1):
                        break
                except OSError:
                    if process.poll() is not None or attempt == 99:
                        raise RuntimeError("private fixture failed to start") from None
                    time.sleep(0.1)
            state = {}

            class Proxy(http.server.BaseHTTPRequestHandler):
                def log_message(self, *_):
                    pass

                def handle_request(self):
                    size = int(self.headers.get("Content-Length", 0))
                    assert size <= 16 * 1024 * 1024
                    body = self.rfile.read(size)
                    key = "health" if self.path == "/health" else "query"
                    if self.path in ("/health", "/query") and state[key]:
                        state[key] -= 1
                        state["injected"].append(self.path)
                        self.send_response(503 if key == "health" else 502)
                        self.send_header("Content-Length", "0")
                        self.end_headers()
                        return
                    if self.path == "/update":
                        state["updates"] += 1
                    headers = {name: self.headers[name] for name in ("Content-Type", "Accept")
                               if name in self.headers}
                    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
                    try:
                        connection.request(self.command, self.path, body, headers)
                        response = connection.getresponse()
                        result = response.read()
                        self.send_response(response.status)
                        self.send_header("Content-Length", str(len(result)))
                        self.end_headers()
                        self.wfile.write(result)
                    finally:
                        connection.close()

                do_GET = handle_request
                do_POST = handle_request

            proxy = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
            thread = threading.Thread(target=proxy.serve_forever, daemon=True)
            thread.start()
            try:
                markdown = "## Epic\n### Type\nepic\n"
                for number in range(9):
                    markdown += (f"## Task {number + 1}\n### Parent\n@1\n"
                                 "### Assignee\nowner\n### Labels\nrelease\n")
                file = root / "tasks.md"
                file.write_text(markdown)
                args = ["--quipu", f"http://127.0.0.1:{proxy.server_port}",
                        "--graph", "urn:demo:bulk", "create",
                        "-f", str(file), "--json"]
                for label, binary in [("BEFORE", before), ("AFTER", after)]:
                    state.update(health=1, query=1, injected=[], updates=0)
                    version = subprocess.check_output([binary, "--version"], env=environment,
                                                      text=True).strip()
                    print(f"{label}: {version}; actual CLI, private empty graph", flush=True)
                    print("$ sd create -f tasks.md --json  # epic + nine children", flush=True)
                    started = time.monotonic()
                    result = subprocess.run([binary, *args], cwd=root, env=environment,
                                            capture_output=True, text=True, timeout=90)
                    print(f"exit={result.returncode}; elapsed={time.monotonic()-started:.3f}s; "
                          f"injected={state['injected']}; update_requests={state['updates']}")
                    if label == "BEFORE":
                        assert result.returncode != 0 and state["updates"] == 0
                        assert "503" in result.stderr
                        print("Observed: gateway refusal, zero batch writes.")
                    else:
                        assert result.returncode == 0, "candidate refused private fixture"
                        seeds = json.loads(result.stdout)
                        assert len(seeds) == 10 and state["updates"] == 1
                        epic = seeds[0]["id"]
                        assert all(seed["parent"] == epic and seed["assignee"] == "owner"
                                   and seed["labels"] == ["release"] for seed in seeds[1:])
                        assert state["injected"] == ["/health", "/query"]
                        print("Observed: both transient failures recovered; 10 items, 9 parent "
                              "links, assignees/labels intact; ONE batch update.")
            finally:
                proxy.shutdown()
                proxy.server_close()
                thread.join()
        finally:
            process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    main()
