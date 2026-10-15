#!/usr/bin/env python3
"""Compare two sd binaries against independent disposable local fixtures."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def probe(binary):
    binary = str(Path(binary).resolve())
    with tempfile.TemporaryDirectory(prefix="seeds-resume-demo-") as tmp:
        project = Path(tmp)
        store = project / "board.db"
        env = {k: v for k, v in os.environ.items() if not k.startswith(("SEEDS_", "QUIPU_"))}
        def call(*args):
            result = subprocess.run([binary, "--store", str(store), "--actor", "demo", *args],
                                    cwd=project, env=env, text=True, capture_output=True, check=True)
            return json.loads(result.stdout) if result.stdout.strip() else None
        subprocess.run([binary, "init", "--prefix", "demo"], cwd=project,
                       env=env, capture_output=True, check=True)
        item = call("create", "Resume this item", "--json")["id"]
        call("update", item, "--status", "deferred", "--defer", "2099-01-01",
             "--notes", "Evidence retained.\nresume_when: date:2099-01-01", "--json")
        print("$ sd update <fixture> --status in_progress --json")
        row = call("update", item, "--status", "in_progress", "--json")[0]
        print(json.dumps({k: row[k] for k in ("status", "defer_until", "notes")}, indent=2))
        print("$ sd update <fixture> --status in_progress --defer 2099-02-01 --json")
        row = call("update", item, "--status", "in_progress", "--defer", "2099-02-01", "--json")[0]
        print(json.dumps({k: row[k] for k in ("status", "defer_until")}, indent=2))


if __name__ == "__main__":
    for label, binary in zip(("BEFORE", "AFTER"), sys.argv[1:], strict=True):
        print(label)
        probe(binary)
