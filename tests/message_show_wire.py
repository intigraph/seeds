"""Exact native message lookup against private loopback responses; no live writes."""
import http.server
import json
import os
import pathlib
import subprocess
import tempfile
import threading

G = "https://example.org/project/mail"
W = G + "/messages/2026-W46"
OLD = G + "/messages/2026-W45"
ID = "msg-0123456789abcdef0123"
IRI = "https://seeds.local/message/" + ID
SC = "https://schema.org/"


def term(value, kind="literal", datatype=None):
    result = {"type": kind, "value": value}
    if datatype:
        result["datatype"] = datatype
    return result


def rows():
    return [{"p": term(p, "uri"), "o": o} for p, o in [
        ("http://www.w3.org/1999/02/22-rdf-syntax-ns#type", term(SC + "Message", "uri")),
        (SC + "identifier", term(ID)),
        (SC + "recipient", term("https://seeds.local/principal/reader", "uri")),
        (SC + "dateSent", term("2026-11-13T00:00:00Z", datatype="http://www.w3.org/2001/XMLSchema#dateTime")),
        (SC + "text", term("exact target")),
        ("https://seeds.local/ontology/messageKind", term("note")),
    ]]


def run(mode, baseline=False, verbose=False):
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
            self.reply({"graphs": [{"iri": w, "source": None, "lifecycle": None}
                for w in [OLD, W]], "truncated": False} if self.path == "/graphs" else {"status": "ok"})

        def do_POST(self):
            if self.path != "/query":
                observed.append("UNEXPECTED WRITE " + self.path)
                self.reply({"error": "no writes"}, 403)
                return
            q = json.loads(self.rfile.read(int(self.headers["Content-Length"])))['query']
            observed.append(q)
            truncated = False
            if "SELECT ?window" in q:
                values = ([W, OLD] if mode == "ambiguous-binding" else
                          [G] if mode == "foreign-binding" else
                          [W] if mode in ["bound", "dangling-binding"] else [])
                data = [{"window": term(w, "uri")} for w in values]
            elif "SELECT ?probe" in q:
                data = [{"probe": term("complete")}]
            elif f"<{IRI}> ?p ?o" in q:
                # A neighbour is deliberately malformed (and a partition scan
                # would hit a cap). Neither belongs in an exact-target query.
                if mode == "transport-error":
                    self.reply({"error": "fixture unavailable"}, 400)
                    return
                data = rows() if f"GRAPH <{W}>" in q or mode == "duplicate" else []
                if mode in ["missing", "dangling-binding"]:
                    data = []
                elif mode == "malformed":
                    data = [r for r in data if r['p']['value'] != SC + "recipient"]
                elif mode == "wrong-id" and data:
                    data[1]['o'] = term("msg-ffffffffffffffffffff")
                elif mode == "truncated":
                    truncated = True
                elif mode == "ceiling":
                    data = (rows() * 167)[:1000]
                elif mode == "bad-row":
                    data = [{"o": term("bad")}]
            else:
                # Model the original capped neighbour read: all broad message
                # scans fail and name an unrelated malformed record.
                observed.append("PARTITION SCAN")
                self.reply({"error": "malformed neighbour at result cap"}, 400)
                return
            self.reply({"head": {"vars": ["p", "o", "window", "probe"]},
                        "results": {"bindings": data}, "truncated": truncated})

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory() as tmp:
            env = {"PATH": os.environ["PATH"], "HOME": tmp, "XDG_CONFIG_HOME": tmp,
                   "QUIPU_AGENT": "message-wire-fixture"}
            proc = subprocess.run([os.environ["MESSAGE_SD"], "--quipu",
                f"http://127.0.0.1:{server.server_port}", "--graph", G,
                "msg", "show", ID, "--all-windows", "--json"],
                env=env, cwd=tmp, capture_output=True, text=True, timeout=15)
        if verbose:
            print("sd msg show <fixture-id> --all-windows --json")
            print("exit:", proc.returncode)
            print(proc.stdout.strip())
            print(proc.stderr.strip())
        if baseline:
            assert proc.returncode != 0 and "PARTITION SCAN" in observed, (mode, proc.stderr, observed)
            print("released baseline: neighbour/cap scan blocks exact target")
            return
        assert "PARTITION SCAN" not in observed, (mode, observed)
        assert not any(q.startswith("UNEXPECTED WRITE") for q in observed)
        if mode in ["good-with-bad-neighbour", "bound"]:
            assert proc.returncode == 0, (mode, proc.stderr, observed)
            result = json.loads(proc.stdout)
            assert result["id"] == ID and result["text"] == "exact target", result
        else:
            assert proc.returncode != 0, (mode, proc.stdout, observed)
            if mode in ["malformed", "wrong-id", "bad-row", "truncated", "ceiling", "duplicate"]:
                assert ID in proc.stderr, (mode, proc.stderr)
        print(mode + ": PASS")
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    for mode in ["good-with-bad-neighbour", "bound", "malformed", "missing", "wrong-id",
                 "duplicate", "truncated", "ceiling", "bad-row", "ambiguous-binding",
                 "foreign-binding", "dangling-binding", "transport-error"]:
        run(mode)
