# Offline native CLI wire controls; no production server or signing credentials.
import json
import os
import pathlib
import subprocess
import tempfile
import threading
from urllib.parse import parse_qs, urlsplit
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

G = "https://seeds.local/project/aegis"
S = "https://seeds.local/item/aegis-probe"
SC = "https://schema.org/"
ON = "https://seeds.local/ontology/"


def term(v, t="literal"):
    d = {"type": t, "value": str(v)}
    if t == "integer":
        d.update(type="literal", datatype="http://www.w3.org/2001/XMLSchema#integer")
    return d


def run_fixture(mode, size):
    queries = []
    metadata_requests = []
    responses = []
    subjects = [S] + [
        f"https://seeds.local/item/aegis-other-{i:04}" for i in range(size - 1)
    ]

    def facts(s, tx=None):
        ident = s.rsplit("/", 1)[1]
        return [
            (SC + "identifier", term(ident)),
            (
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
                term(SC + "Action", "uri"),
            ),
            (
                SC + "name",
                term(
                    "version "
                    + str(3 if tx is None or tx >= 30 else 2 if tx >= 20 else 1)
                ),
            ),
            (ON + "status", term("open")),
            (ON + "priority", term(0, "integer")),
            (
                SC + "version",
                term(3 if tx is None or tx >= 30 else 2 if tx >= 20 else 1, "integer"),
            ),
        ]

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def reply(self, d, status=200):
            b = json.dumps(d).encode()
            responses.append(len(b))
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(b)))
            self.end_headers()
            self.wfile.write(b)

        def do_GET(self):
            path = urlsplit(self.path)
            if path.path != "/transactions":
                self.reply({"status": "ok"})
                return
            params = parse_qs(path.query)
            assert set(params) == {"since", "limit"} and params["limit"] == ["1"], params
            since = int(params["since"][0])
            metadata_requests.append(since)
            if mode == "empty-feed":
                self.reply({"count": 0, "transactions": []})
                return
            if mode == "truncated-feed":
                self.reply({"count": 0, "transactions": [], "truncated": True})
                return
            if mode == "oversized-feed":
                self.reply({"count": 0, "transactions": [], "padding": "x" * 65536})
                return
            if mode == "bad-feed":
                self.reply({"count": 1, "transactions": [{"id": since}]})
                return
            if mode == "wide-feed":
                self.reply({"count": 2, "transactions": [{"id": since + 1}, {"id": since + 2}]})
                return
            if mode == "feed-error":
                self.reply({"error": "unavailable"}, 503)
                return
            commits = [1, 10, 20, 30] if mode == "gapped" else range(1, 31)
            rows = [{"id": tx} for tx in commits if tx > since][:1]
            self.reply({"count": len(rows), "transactions": rows})

        def do_POST(self):
            if self.path != "/query":
                self.send_error(405)
                return
            b = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            q = b["query"]
            queries.append(b)
            if "tx" in b and (b["tx"] > 30 or (mode == "gapped" and b["tx"] not in [1,10,20,30])):
                self.reply({"error": "not an existing committed source transaction"}, 400)
                return
            if "tx" in b and mode == "refused":
                self.reply(
                    {
                        "error": "as_of_tx refused for attached databases/file-local transaction IDs"
                    },
                    400,
                )
                return
            if q.lstrip().startswith("ASK"):
                self.reply({"boolean": False})
                return
            eph = "#seeds-ephemeral" in q
            rows = []
            if "COUNT(*)" in q:
                rows = [{"n": term(0 if eph else size * 6, "integer")}]
            elif q.startswith("SELECT ?s WHERE") and SC + "Action" in q and not eph:
                rows = [{"s": term(s, "uri")} for s in sorted(subjects)]
            elif q.startswith("SELECT ?s ?p ?o") and not eph:
                selected = [s for s in subjects if "<" + s + ">" in q]
                rows = [
                    {"s": term(s, "uri"), "p": term(p, "uri"), "o": o}
                    for s in selected
                    if mode != "missing"
                    and (b.get("tx") is None or b["tx"] >= 10)
                    and not (
                        mode == "recreated"
                        and b.get("tx") is not None
                        and 15 <= b["tx"] < 20
                    )
                    for p, o in facts(s, b.get("tx"))
                ]
            self.reply(
                {
                    "head": {"vars": list(rows[0]) if rows else []},
                    "results": {"bindings": rows},
                    "count": len(rows),
                    "truncated": mode == "truncated"
                    and q.startswith("SELECT ?s ?p ?o"),
                }
            )

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory() as td:
        env = {
            **os.environ,
            "HOME": td,
            "XDG_CONFIG_HOME": td,
            "QUIPU_AGENT": "history-wire-fixture",
            "SEEDS_AGENT_NAME": "history-wire-fixture",
        }
        for k in [
            "SEEDS_QUIPU_TOKEN",
            "QUIPU_AUTH_TOKEN",
            "SEEDS_QUIPU",
            "SEEDS_STORE",
        ]:
            env.pop(k, None)
        command = ["show", "aegis-probe", "--at", "31", "--json"] if mode == "future" else ["history", "aegis-probe", "--json"]
        proc = subprocess.run(
            [
                os.environ.get(
                    "HISTORY_SD",
                    str(
                        pathlib.Path(__file__).resolve().parents[1] / "target/debug/sd"
                    ),
                ),
                "--quipu",
                f"http://127.0.0.1:{server.server_port}",
                "--graph",
                G,
                *command,
            ],
            check=False,
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
        )
    server.shutdown()
    return {
        "mode": mode,
        "board_items": size,
        "exit": proc.returncode,
        "queries": len(queries),
        "metadata_requests": len(metadata_requests),
        "response_bytes": sum(responses),
        "requests": queries,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


results = [
    run_fixture(mode, size)
    for mode in ["refused", "versions", "gapped", "truncated", "missing", "recreated", "bad-feed", "wide-feed", "feed-error", "empty-feed", "truncated-feed", "oversized-feed", "future"]
    for size in [1, 101]
]

print(
    json.dumps(
        [
            {k: v for k, v in r.items() if k not in ("requests", "stdout", "stderr")}
            for r in results
        ]
    )
)
for mode in ["refused", "versions", "gapped", "truncated", "missing", "recreated", "bad-feed", "wide-feed", "feed-error", "empty-feed", "truncated-feed", "oversized-feed", "future"]:
    pair = [r for r in results if r["mode"] == mode]
    assert pair[0]["queries"] == pair[1]["queries"], pair
    assert pair[0]["metadata_requests"] == pair[1]["metadata_requests"], pair
    assert pair[0]["metadata_requests"] <= 64, pair
    assert pair[0]["response_bytes"] == pair[1]["response_bytes"], pair
    for r in pair:
        if mode != "future":
            assert all(
                "COUNT(" not in b["query"]
                and ("ASK" in b["query"] or "<" + S + ">" in b["query"])
                for b in r["requests"]
            ), r
        if mode in ("versions", "gapped"):
            assert r["exit"] == 0, r
            versions = json.loads(r["stdout"])["versions"]
            assert [v["tx"] for v in versions] == [10, 20, 30], versions
            assert [v["revision"] for v in versions] == [1, 2, 3], versions
        else:
            assert r["exit"] != 0, r
            if mode == "future":
                assert "existing committed source transaction" in r["stderr"], r
                assert r["metadata_requests"] == 0, r
            if mode == "refused":
                assert "as_of_tx" in r["stderr"], r
            if mode == "truncated":
                assert "truncated" in r["stderr"], r
            if mode == "recreated":
                assert "continuous version coverage" in r["stderr"], r
                assert "error" in json.loads(r["stdout"]), r
                assert "versions" not in json.loads(r["stdout"]), r
print(
    "PASS: exact historical versions, unrelated growth invariance, refusal/truncation/missing and every request scoped"
)
