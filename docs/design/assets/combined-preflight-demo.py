#!/usr/bin/env python3
"""Private wire-contract demo; no production connection or credentials."""
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
from tempfile import TemporaryDirectory
import time
import urllib.request
import urllib.parse

root = Path(__file__).resolve().parents[3]
source = (root / "src/native/remote.rs").read_text()
method = source[source.index("    fn legacy_present(&self)"):]
raw = re.search(r'let query = format!\(\s*"((?:[^"\\]|\\.)*)",\s*self.graph', method, re.S).group(1)
query = raw.replace("\\\n", "")
query = bytes(query, "utf-8").decode("unicode_escape")
graph = "https://seeds.local/project/preflight-demo"
eph = graph + "#seeds-ephemeral"
legacy = "https://seeds.local/ontology/revision"
query = query.replace("<{}>", f"<{graph}>").replace("{eph}", eph).replace("{legacy}", legacy).replace("{{", "{").replace("}}", "}")
with TemporaryDirectory() as tmp:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    url = f"http://127.0.0.1:{port}"
    env={**os.environ,"HOME":tmp,"QUIPU_AUTH_TOKEN":"private-fixture"}
    env.pop("QUIPU_AUTH_TOKEN_FILE",None)
    proc = subprocess.Popen([os.environ["SEEDS_TEST_QUIPU_SERVER"], "--db", str(Path(tmp)/"q.db"), "--bind", f"127.0.0.1:{port}"], env=env, cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try:
                with urllib.request.urlopen(url+"/health",timeout=1): break
            except OSError: time.sleep(.05)
        else: raise RuntimeError("fixture server unavailable")
        calls = 0
        def post(path, body):
            global calls
            if path == "/query": calls += 1
            encoded=urllib.parse.urlencode(body).encode() if path=="/update" else json.dumps(body).encode()
            content_type="application/x-www-form-urlencoded" if path=="/update" else "application/json"
            req=urllib.request.Request(url+path,data=encoded,headers={"Content-Type":content_type,"Accept":"application/sparql-results+json","X-Quipu-Client":"seeds-preflight-fixture","Authorization":"Bearer private-fixture"})
            with urllib.request.urlopen(req,timeout=10) as response:
                raw=response.read()
                return json.loads(raw) if raw else {}
        for g in [graph,eph]:post("/graph/create",{"graph":g})
        before=calls
        found=False
        for g in [graph,eph]:
            answer=post("/query",{"query":f"ASK {{ GRAPH <{g}> {{ ?s <{legacy}> ?r }} }}"})
            assert isinstance(answer["boolean"],bool)
            if answer["boolean"]:found=True;break
        print(f"BEFORE: clean ledger={not found}; vocabulary requests={calls-before}",flush=True)
        before=calls
        tags={r["probe"]["value"] for r in post("/query",{"query":query})["results"]["bindings"]}
        assert tags=={"complete"},tags
        print(f"AFTER: clean ledger=True; preflight requests={calls-before}; tags={sorted(tags)}",flush=True)
        post("/update",{"update":f'INSERT DATA {{ GRAPH <{eph}> {{ <urn:fixture:item> <urn:fixture:p> "new" }} }}'})
        tags={r["probe"]["value"] for r in post("/query",{"query":query})["results"]["bindings"]}
        assert tags=={"complete","eph_present"},tags
        print(f"LATER: new ephemeral data observed; tags={sorted(tags)}",flush=True)
        print("Private HTTP query-contract demo only; Rust native controls separately verify cache and refusal semantics.",flush=True)
        print("remote.rs SHA256="+hashlib.sha256(source.encode()).hexdigest(),flush=True)
    finally:
        proc.terminate()
        proc.wait(timeout=10)
