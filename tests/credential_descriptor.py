"""Offline CLI credential controls. Only loopback fake servers and inert tokens."""
import http.server
import json
import os
import pathlib
import subprocess
import tempfile
import threading


def main():
    binary = os.environ["CREDENTIAL_SD"]
    observed = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def reply(self, data, status=200):
            body = json.dumps(data).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            observed.append((self.path, self.headers.get("Authorization")))
            self.reply({"graphs": []} if self.path == "/graphs" else {"status": "ok"})

        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
            observed.append((self.path, self.headers.get("Authorization")))
            if self.path == "/query" and "ASK" in body:
                self.reply({"boolean": False})
            elif self.path == "/query":
                self.reply({"head": {"vars": ["probe"]}, "results": {"bindings": [{"probe": {"type": "literal", "value": "complete"}}] if "SELECT ?probe" in body else []}, "truncated": False})
            else:
                # Refuse the first authenticated request. No fake write succeeds.
                self.reply({"error": "fixture refusal"}, 403)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / ".config").mkdir()
            url = f"http://127.0.0.1:{server.server_port}"
            foreign = root / "foreign"
            foreign.write_text("foreign-file-fixture")
            env = {"PATH": os.environ["PATH"], "HOME": tmp,
                   "XDG_CONFIG_HOME": str(root / ".config"),
                   "SEEDS_QUIPU_TOKEN": "foreign-inline-fixture",
                   "SEEDS_QUIPU_TOKEN_FILE": str(foreign)}

            def run(value=None, selected=True, malformed=False, selection_override=None, destination=None):
                observed.clear()
                child = dict(env)
                with tempfile.TemporaryFile() as f:
                    if value is not None:
                        f.write(value)
                        f.flush()
                    if selected:
                        child.update(SEEDS_QUIPU_CREDENTIAL_PRINCIPAL="own-fixture",
                                     SEEDS_QUIPU_TOKEN_FD=str(f.fileno()) if not malformed else "2147483647")
                    if destination is not None:
                        child["SEEDS_QUIPU_CREDENTIAL_DESTINATION"] = destination
                    if selection_override is not None:
                        child.pop("SEEDS_QUIPU_CREDENTIAL_PRINCIPAL", None)
                        child.pop("SEEDS_QUIPU_TOKEN_FD", None)
                        child.update(selection_override)
                    result = subprocess.run([binary, "--quipu", url, "--graph",
                        "https://example.org/test-board", "create", "fixture", "--json"],
                        cwd=tmp, env=child, pass_fds=(f.fileno(),), capture_output=True, timeout=15)
                for secret in (b"own-fixture-value", b"foreign-inline-fixture", b"foreign-file-fixture"):
                    assert secret not in result.stdout + result.stderr, "credential appeared in output"
                return result, list(observed)

            result, requests = run(b"own-fixture-value")
            assert result.returncode != 0  # fake server refuses authenticated write
            auth = [auth for _, auth in requests if auth]
            if os.environ.get("CREDENTIAL_EXPECT_LEGACY") == "1":
                assert auth == ["Bearer foreign-inline-fixture"], (result.stderr, requests)
                print("legacy CLI: selected descriptor ignored; inherited inline fixture sent")
                return
            assert auth == ["Bearer own-fixture-value"], (result.stderr, requests)
            result, requests = run(b"own-fixture-value", destination=url)
            assert [auth for _, auth in requests if auth] == ["Bearer own-fixture-value"]
            result, requests = run(b"own-fixture-value", destination="https://other.example")
            assert result.returncode != 0 and not requests, "destination changed after provider admission"
            result, requests = run(b"other-own-value")
            assert [auth for _, auth in requests if auth] == ["Bearer other-own-value"]
            for value, malformed in [(b"", False), (b"bad\nvalue", False), (b"x" * 4097, False), (None, True)]:
                result, requests = run(value, malformed=malformed)
                assert result.returncode != 0 and not requests, (result.stderr, requests)
            for principal, descriptor in [("", ""), (" ", " "), ("", " "), (" ", ""), ("", None), (None, "")]:
                selection = {}
                if principal is not None:
                    selection["SEEDS_QUIPU_CREDENTIAL_PRINCIPAL"] = principal
                if descriptor is not None:
                    selection["SEEDS_QUIPU_TOKEN_FD"] = descriptor
                result, requests = run(selection_override=selection)
                assert result.returncode != 0 and not requests, "present invalid selection fell back"
            for key in ("SEEDS_QUIPU_CREDENTIAL_PRINCIPAL", "SEEDS_QUIPU_TOKEN_FD"):
                result, requests = run(selection_override={key: b"\xff"})
                assert result.returncode != 0 and not requests, "non-Unicode selection fell back"
            result, requests = run(selected=False)
            assert [auth for _, auth in requests if auth] == ["Bearer foreign-inline-fixture"]
            del env["SEEDS_QUIPU_TOKEN"]
            result, requests = run(b"own-fixture-value")
            assert [auth for _, auth in requests if auth] == ["Bearer own-fixture-value"]
            result, requests = run(selected=False)
            assert [auth for _, auth in requests if auth] == ["Bearer foreign-file-fixture"]
            print("descriptor CLI: own/rotation, foreign inline/file isolation, 4 fail-closed + 8 invalid-presence cases, next-deselected defaults PASS")
            # Admission must precede init and every other command's early exits.
            # Missing configured token files are not read; no HTTP is performed.
            env.pop("SEEDS_QUIPU_TOKEN", None)
            env["SEEDS_QUIPU_TOKEN_FILE"] = str(root / "absent-token")
            observed.clear()
            command = [binary, "--quipu", url, "--graph", "https://example.org/test-board",
                       "init", "--credential-admission"]
            admitted = subprocess.run(command, cwd=tmp, env=env, capture_output=True, timeout=10)
            proof = json.loads(admitted.stdout)
            assert admitted.returncode == 0 and proof == {
                "contract": "seeds-credential-admission/v1", "admitted": True,
                "destination": url, "destination_binding": True}, admitted.stderr
            assert not observed and not (root / ".seeds").exists()
            for destination in ("http://remote.example", "https://", "https://u:p@example.org",
                                "https://example.org/?secret=fixture"):
                denied = subprocess.run([binary, "--quipu", destination, "init", "--credential-admission"],
                                        cwd=tmp, env=env, capture_output=True, timeout=10)
                assert denied.returncode != 0 and not observed
                assert b"secret=fixture" not in denied.stdout + denied.stderr
            denied = subprocess.run([binary, "--store", str(root / "must-not-create"),
                                     "init", "--credential-admission"], cwd=tmp, env=env,
                                    capture_output=True, timeout=10)
            assert denied.returncode != 0 and not (root / "must-not-create").exists()
            (root / ".seeds").mkdir()
            (root / ".seeds/config.toml").write_text('[quipu]\nurl="https://example.org"\n')
            denied = subprocess.run([binary, "show", "fixture", "--credential-admission"],
                                    cwd=tmp, env=env, capture_output=True, timeout=10)
            assert denied.returncode != 0 and not observed, "untrusted project destination admitted"
            print("admission CLI: no HTTP/token-file read/init mutation, destination bind, trust/transport/URL/local refusals PASS")
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == "__main__":
    main()
