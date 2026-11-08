"""Isolated native CLI fixture; pass the candidate sd executable as argv[1]."""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="seeds-deferral-demo-") as directory:
    root = Path(directory)
    connection = [binary, "--store", str(root / "fixture.db"), "--graph",
                  "https://example.invalid/deferral-demo", "--actor", "demo"]

    def run(*args):
        result = subprocess.run(connection + list(args), cwd=root, text=True,
                                capture_output=True, check=True)
        return result.stdout

    identifier = json.loads(run("create", "Technical fixture", "--json"))["id"]

    def observe(stage):
        destination = root / stage
        run("export", "--to", str(destination))
        data = (destination / "export.nt").read_text()
        count = re.findall(r'<https://seeds.local/ontology/observedDeferralWriteCount> "(\d+)"', data)
        origin = re.findall(r'<https://seeds.local/ontology/observedDeferralWriteCountSince> "([^"]+)"', data)
        assert len(count) == len(origin) <= 1
        value = {"stage": stage, "count": int(count[0]) if count else None,
                 "origin_present": bool(origin)}
        print(json.dumps(value, sort_keys=True))
        return value, origin

    before, _ = observe("before")
    assert before["count"] is None
    run("defer", identifier)
    first, origin = observe("first-deferral")
    assert first["count"] == 1
    run("defer", identifier)
    second, retained = observe("second-deferral")
    assert second["count"] == 2 and retained == origin
    run("undefer", identifier)
    after, retained = observe("undeferred")
    assert after["count"] == 2 and retained == origin
