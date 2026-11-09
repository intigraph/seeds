#!/usr/bin/env python3
"""Disposable embedded-store demonstration; no network or deployment state."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

binary = Path(os.environ.get("DEMO_SD", "sd"))
if not binary.is_absolute():
    import shutil
    binary = Path(shutil.which(str(binary)))
with tempfile.TemporaryDirectory(prefix="tracker-demo-") as directory:
    env = {"PATH": os.environ["PATH"], "HOME": directory,
           "XDG_CONFIG_HOME": directory, "SEEDS_ACTOR": "fixture"}
    def run(args):
        return subprocess.run([str(binary), *args], cwd=directory, env=env,
                              capture_output=True, text=True, check=False)
    setup = run(["create", "controlled fixture", "--silent"])
    assert setup.returncode == 0, setup.stderr
    item = setup.stdout.strip()
    for mode, flags in [("Preserved human output", []), ("Structured result", ["--result-json"])]:
        print(mode)
        print("$ sd comments add FIXTURE --file missing-note.txt" + (" --result-json" if flags else ""))
        output = run(["comments", "add", item, "--file", "missing-note.txt", *flags])
        print("exit:", output.returncode)
        assert output.returncode == 2
        if flags:
            value = json.loads(output.stdout)
            assert value["ok"] is False
            assert value["mutation_outcome"] == "NOT_WRITTEN"
            assert value["error"]["category"] == "MISSING_INPUT"
            print(json.dumps(value, indent=2))
        else:
            print("stdout bytes:", len(output.stdout))
            print(output.stderr.strip())
    comments = run(["comments", "list", item, "--json"])
    assert comments.returncode == 0 and json.loads(comments.stdout) == []
    print("Positive reconciliation: exact fixture has zero comments; nothing submitted.")
