#!/usr/bin/env python3
"""Isolated CLI demonstration. No live server, board, or credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

PIN = "b1b3bf0846298e437da15f89d3fc599ee41663f9258c3d40b03f033804d6a04f"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--base", required=True, type=Path)
parser.add_argument("--candidate", required=True, type=Path)
parser.add_argument("--quipu", required=True, type=Path)
parser.add_argument("--profile", required=True, type=Path)
args = parser.parse_args()
binaries = {name: str(getattr(args, name).resolve()) for name in ("base", "candidate", "quipu")}
profile = args.profile.read_bytes()
assert hashlib.sha256(profile).hexdigest() == PIN, "recipient profile is not the reviewed release"
with tempfile.TemporaryDirectory(prefix="seeds-project-demo-") as temp:
    root = Path(temp)
    repo = root / "repo"
    (repo / ".seeds").mkdir(parents=True)
    (repo / ".seeds/config.toml").write_text('[pendant]\ndir = ".seeds/pendant"\n')
    (root / "home").mkdir()
    env = {"HOME": str(root / "home"), "XDG_CONFIG_HOME": str(root / "home/.config"),
           "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "SEEDS_ACTOR": "demo",
           "GIT_CONFIG_NOSYSTEM": "1"}

    def run(binary, command, expected=0):
        result = subprocess.run([binaries.get(binary, binary), *command], cwd=repo,
                                env=env, capture_output=True, text=True, timeout=30)
        assert result.returncode == expected, result.stderr
        return result

    first = run("base", ["create", "Plan migration", "--silent"]).stdout.strip()
    run("base", ["comments", "add", first, "Keep every fact and comment."])
    run("base", ["create", "Verify clone", "--deps", first, "--silent"])
    before = run("base", ["list", "--all", "--json"]).stdout
    facts = (repo / ".seeds/pendant/export.nt").read_bytes()
    print("BEFORE: legacy .seeds/pendant; two items and one comment")
    run("base", ["migrate-project"], expected=2)
    print("base migrate-project: usage refusal (exit 2)")
    migrated = json.loads(run("candidate", ["migrate-project", "--json"]).stdout)
    assert (repo / ".quipu/graph/export.nt").read_bytes() == facts
    assert run("candidate", ["list", "--all", "--json"]).stdout == before
    print("AFTER:", json.dumps(migrated, sort_keys=True))
    print("full exported facts and both item projections: identical")

    # Separate recipients: do not change policy on the refused negative store.
    result = run("quipu", ["load", ".quipu/graph", "--destination", "internal",
                            "--db", str(root / "negative.db")], expected=1)
    assert "off_vocabulary" in result.stderr
    print("profile-less plain Quipu: exit 1, off_vocabulary quarantine")
    print("LIMIT: plain Quipu does not yet name the setup command in this refusal")
    (root / "reviewed-profile.ttl").write_bytes(profile)
    run("quipu", ["shapes", "load", "seeds", str(root / "reviewed-profile.ttl"),
                  "--db", str(root / "positive.db")])
    loaded = json.loads(run("quipu", ["load", ".quipu/graph", "--destination", "internal",
                                      "--db", str(root / "positive.db")]).stdout)
    assert loaded["added"] == len(facts.splitlines())
    print("separate recipient, explicit pinned profile setup then one plain load:",
          loaded["added"], "facts")
    (repo / ".quipu/verifier.pk8").write_text("synthetic key exclusion control\n")
    run("git", ["init", "--quiet"])
    run("git", ["add", ".quipu"])
    staged = run("git", ["diff", "--cached", "--name-only"]).stdout.splitlines()
    assert ".quipu/project" in staged and ".quipu/graph/export.nt" in staged
    assert all(p in (".quipu/project", ".quipu/.gitignore") or p.startswith(".quipu/graph/")
               for p in staged)
    print("git add .quipu: only identity, ignore policy and graph; database/key excluded")
