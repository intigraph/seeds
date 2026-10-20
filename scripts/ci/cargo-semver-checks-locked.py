#!/usr/bin/env python3
"""Release-plz adapter: compare locked rustdoc, never freshly resolved wrappers.

# arming: library invoked by the release workflow's release-plz process
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib


FLAGS = "-Z unstable-options --document-private-items --document-hidden-items --output-format=json --cap-lints=allow"
FEATURES = {"default", "native", "shacl"}


def manifest(path):
    path = Path(path).resolve()
    return path / "Cargo.toml" if path.is_dir() else path


def checked_source(path):
    data = tomllib.loads(path.read_text())
    if data["package"]["name"] != "seeds-ai":
        raise ValueError("expected seeds-ai")
    if set(data.get("features", {})) != FEATURES:
        raise ValueError("feature set changed: explicitly review rustdoc feature coverage")
    if not path.with_name("Cargo.lock").is_file():
        raise ValueError("source has no release lockfile")
    return data


def verify_baseline(path, repo):
    """Bind the passed baseline to the release tag, including its source/lock."""
    data = checked_source(path)
    version = data["package"]["version"]
    tag = "seeds-ai-v" + version
    if not all(c in "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-+" for c in version):
        raise ValueError("invalid release version")
    commit = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "--verify", tag + "^{commit}"], text=True
    ).strip()
    names = subprocess.check_output(
        ["git", "-C", str(repo), "ls-tree", "-r", "--name-only", commit,
         "--", "Cargo.toml", "Cargo.lock", "src"], text=True
    ).splitlines()
    expected = {"Cargo.toml", "Cargo.lock"}
    expected.update(p.relative_to(path.parent).as_posix() for p in (path.parent / "src").rglob("*") if p.is_file())
    if set(names) != expected:
        raise ValueError("baseline source file set differs from release tag")
    for name in names:
        original = subprocess.check_output(["git", "-C", str(repo), "show", commit + ":" + name])
        if (path.parent / name).read_bytes() != original:
            raise ValueError("baseline differs from release tag: " + name)
    print("locked semver baseline " + json.dumps({"tag": tag, "commit": commit,
          "lock_sha256": hashlib.sha256(path.with_name("Cargo.lock").read_bytes()).hexdigest()}), file=sys.stderr)


def rustdoc(path, output, cargo="cargo"):
    checked_source(path)
    lock = path.with_name("Cargo.lock")
    before = lock.read_bytes()
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    env["RUSTDOCFLAGS"] = (env.get("RUSTDOCFLAGS", "") + " " + FLAGS).strip()
    metadata = json.loads(subprocess.check_output(
        [cargo, "metadata", "--locked", "--no-deps", "--format-version", "1", "--manifest-path", str(path)], env=env
    ))
    # Cargo's target path may be shared. Copy the JSON before another arm builds.
    target = Path(metadata["target_directory"]) / "doc" / "seeds.json"
    target.unlink(missing_ok=True)
    subprocess.run([cargo, "rustdoc", "--locked", "--manifest-path", str(path),
                    "--lib", "--all-features"], env=env, check=True)
    if lock.read_bytes() != before:
        raise ValueError("rustdoc changed the source lockfile")
    shutil.copyfile(target, output)
    print("locked semver rustdoc " + json.dumps({"version": metadata["packages"][0]["version"],
          "lock_sha256": hashlib.sha256(before).hexdigest(),
          "rustdoc_sha256": hashlib.sha256(Path(output).read_bytes()).hexdigest()}), file=sys.stderr)


def main(argv=None):
    args = list(sys.argv[1:] if argv is None else argv)
    real = os.environ["SEEDS_SEMVER_REAL"]
    if args in (["--version"], ["semver-checks", "--version"]):
        return subprocess.call([real, *args])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("subcommand", choices=["semver-checks"])
    parser.add_argument("command", choices=["check-release"])
    parser.add_argument("--manifest-path", required=True)
    parser.add_argument("--baseline-root", required=True)
    parser.add_argument("--package", required=True, choices=["seeds-ai"])
    parser.add_argument("--release-type", required=True, choices=["minor"])
    parser.add_argument("--color", choices=["never"], default="never")
    a = parser.parse_args(args)
    current, baseline = manifest(a.manifest_path), manifest(a.baseline_root)
    verify_baseline(baseline, Path(os.environ["SEEDS_SEMVER_REPO"]).resolve())
    with tempfile.TemporaryDirectory(prefix="seeds-locked-semver-") as directory:
        directory = Path(directory)
        current_json, baseline_json = directory / "current.json", directory / "baseline.json"
        rustdoc(current, current_json)
        rustdoc(baseline, baseline_json)
        # These are supported cargo-semver-checks inputs. Preserve its real exit
        # status and stdout: release-plz distinguishes incompatible(100) from
        # compilation/tool errors. No suppression, baseline switch or API patch.
        return subprocess.call([real, "semver-checks", "check-release", "--color", a.color,
                                "--release-type", a.release_type, "--current-rustdoc", str(current_json),
                                "--baseline-rustdoc", str(baseline_json)])


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as exc:
        print("locked semver refused: " + str(exc), file=sys.stderr)
        sys.exit(101)
