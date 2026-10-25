#!/usr/bin/env python3
"""Show candidate modes against a private HTTP protocol fixture, not a real board."""
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import parse_qs


class Fixture(BaseHTTPRequestHandler):
    key_id = None
    actors = []

    def log_message(self, *_args):
        pass

    def reply(self, value):
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/graphs":
            return self.reply({"graphs": []})
        assert self.path == "/health"
        self.reply({"status": "ok"})

    def do_POST(self):
        data = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        envelope = self.headers["x-quipu-attestation"]
        envelope = json.loads(base64.urlsafe_b64decode(envelope + "=" * (-len(envelope) % 4)))
        if self.path == "/identity":
            if Fixture.key_id is None:
                Fixture.key_id = envelope["key_id"]
            assert envelope["key_id"] == Fixture.key_id
            return self.reply({"authenticated": True, "auth_class": "attested_session",
                               "principal": "host-principal", "key_id": Fixture.key_id,
                               "session": "host-session"})
        if self.path == "/graphs":
            return self.reply({"graphs": []})
        if self.path == "/graph/create":
            return self.reply({"ok": True})
        if self.path == "/update":
            Fixture.actors.append(parse_qs(data.decode())["actor"][0])
            return self.reply({"asserted": 1, "retracted": 0, "tx": len(Fixture.actors)})
        assert self.path == "/query"
        query = json.loads(data)["query"]
        rows = []
        if query.startswith("SELECT ?probe"):
            rows = [{"probe": {"type": "literal", "value": "complete"}}]
        elif query.startswith("ASK"):
            return self.reply({"boolean": False})
        elif "COUNT(" in query:
            rows = [{"n": {"type": "literal", "value": "0",
                           "datatype": "http://www.w3.org/2001/XMLSchema#integer"}}]
        self.reply({"results": {"bindings": rows}})


def main():
    binary = Path(sys.argv[1]).resolve(strict=True)
    print("Target: isolated HTTP protocol fixture, one synthetic host key", flush=True)
    print("Candidate SHA256:", hashlib.sha256(binary.read_bytes()).hexdigest(), flush=True)
    print("Control is candidate enforce mode, NOT an execution of the old binary.", flush=True)
    with tempfile.TemporaryDirectory(prefix="seeds-actor-demo-") as directory:
        root = Path(directory)
        home = root / "home"
        config_dir = home / ".config/seeds"
        config_dir.mkdir(parents=True)
        key = root / "synthetic.key"
        key.write_text("2f" * 32)  # Public synthetic fixture, never an installed key.
        key.chmod(0o600)
        config = config_dir / "config.toml"
        server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
        worker = threading.Thread(target=server.serve_forever)
        worker.start()
        # Same conservative-gate branch as the Rust control; URL parsing maps
        # 127.1 to the loopback listener, without DNS or a LAN listener.
        url = f"http://127.1:{server.server_port}"
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home),
               "XDG_CONFIG_HOME": str(home / ".config"),
               "XDG_STATE_HOME": str(root / "state"), "USER": "synthetic-user"}
        try:
            for mode, actor in [("enforce", "worker-a"), (None, "worker-a"), (None, "worker-b")]:
                policy = "" if mode is None else f'actor_principal_mode="{mode}"\n'
                config.write_text(f'[quipu]\nsigning_key_file={json.dumps(str(key))}\n'
                                  'signing_session="host-session"\nsigning_introducer="urn:demo:lead"\n' + policy)
                print(f"\n$ sd --actor {actor} create 'Synthetic actor probe'  # {mode or 'default observe'}", flush=True)
                result = subprocess.run([str(binary), "--quipu", url, "--graph", "urn:demo:actors",
                                         "--actor", actor, "create", "Synthetic actor probe"],
                                        cwd=root, env=env, capture_output=True, text=True)
                print(result.stdout, end="", flush=True)
                print(result.stderr, end="", flush=True)
                print("exit:", result.returncode, flush=True)
                assert (result.returncode == 0) == (mode is None)
                time.sleep(2)
            state = json.loads((root / "state/seeds/actor-principal.json").read_text())
            print("\nActual observation file:", json.dumps(state), flush=True)
            print("Actors received by the HTTP fixture:", json.dumps(Fixture.actors), flush=True)
            assert state["mismatches_total"] == 2
            assert Fixture.actors == ["worker-a", "worker-b"]
            print("Fixture does not validate server-side cryptography or prove deployment.", flush=True)
            time.sleep(3)
        finally:
            server.shutdown()
            server.server_close()
            worker.join()


if __name__ == "__main__":
    main()
