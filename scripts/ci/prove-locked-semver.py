#!/usr/bin/env python3
"""Compare the actual release with itself and with a removed public module.

# arming: agent-invoked; requires pinned cargo-semver-checks and Rust 1.98.1
"""
import argparse
import importlib.util
import io
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--real", required=True, help="cargo-semver-checks 0.50.0 binary")
    parser.add_argument("--tag", default="seeds-ai-v0.6.1")
    args = parser.parse_args()
    repo = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
    spec = importlib.util.spec_from_file_location("adapter", Path(__file__).with_name("cargo-semver-checks-locked.py"))
    adapter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(adapter)
    with tempfile.TemporaryDirectory(prefix="locked-semver-proof-") as directory:
        directory = Path(directory)
        release = directory / "release"
        release.mkdir()
        archive = subprocess.check_output(["git", "archive", args.tag])
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(release, filter="data")
        adapter.verify_baseline(release / "Cargo.toml", repo)
        adapter.rustdoc(release / "Cargo.toml", directory / "release.json")
        mutant = directory / "mutant"
        shutil.copytree(release, mutant)
        lib = mutant / "src/lib.rs"
        old = lib.read_text()
        if "pub mod rank;" not in old:
            raise ValueError("negative-control target changed: review the proof")
        lib.write_text(old.replace("pub mod rank;", "mod rank;"))
        adapter.rustdoc(mutant / "Cargo.toml", directory / "mutant.json")
        for name, current, expected in [("release-self", "release.json", 0),
                                        ("removed-public-module", "mutant.json", 100)]:
            result = subprocess.run([args.real, "semver-checks", "check-release", "--release-type", "minor",
                                     "--current-rustdoc", str(directory / current),
                                     "--baseline-rustdoc", str(directory / "release.json")])
            print(f"{name}: exit {result.returncode}, expected {expected}", flush=True)
            if result.returncode != expected:
                raise SystemExit(1)


if __name__ == "__main__":
    main()
