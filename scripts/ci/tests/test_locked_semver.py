"""Fail-closed source/feature provenance controls; live API proof is documented."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "locked_semver", Path(__file__).parents[1] / "cargo-semver-checks-locked.py"
)
ADAPTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ADAPTER)


class BaselineIdentity(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        (self.repo / "src").mkdir()
        (self.repo / "Cargo.toml").write_text(
            '[package]\nname="seeds-ai"\nversion="0.6.1"\n'
            '[features]\ndefault=["native"]\nnative=["shacl"]\nshacl=[]\n'
        )
        (self.repo / "Cargo.lock").write_text("version = 4\n")
        (self.repo / "src/lib.rs").write_text("pub fn retained() {}\n")
        self.git("init", "-q")
        self.git("add", ".")
        self.git("-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "release fixture")
        self.git("tag", "seeds-ai-v0.6.1")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], stderr=subprocess.STDOUT)

    def verify(self):
        ADAPTER.verify_baseline(self.repo / "Cargo.toml", self.repo)

    def test_release_source_positive(self):
        self.verify()

    def test_source_mutation_refused(self):
        (self.repo / "src/lib.rs").write_text("pub fn substituted() {}")
        with self.assertRaisesRegex(ValueError, "differs"):
            self.verify()

    def test_lock_mutation_refused(self):
        (self.repo / "Cargo.lock").write_text("version = 3\n")
        with self.assertRaisesRegex(ValueError, "differs"):
            self.verify()

    def test_extra_source_refused(self):
        (self.repo / "src/extra.rs").write_text("")
        with self.assertRaisesRegex(ValueError, "file set"):
            self.verify()

    def test_missing_source_refused(self):
        (self.repo / "src/lib.rs").unlink()
        with self.assertRaisesRegex(ValueError, "file set"):
            self.verify()

    def test_new_feature_requires_explicit_coverage(self):
        p = self.repo / "Cargo.toml"
        p.write_text(p.read_text() + "new_feature=[]\n")
        with self.assertRaisesRegex(ValueError, "feature set"):
            self.verify()

    def test_missing_lock_refused(self):
        (self.repo / "Cargo.lock").unlink()
        with self.assertRaisesRegex(ValueError, "lockfile"):
            self.verify()

    def test_version_forwarding_retains_real_status(self):
        with patch.dict(os.environ, {"SEEDS_SEMVER_REAL": "/fixture/real"}), \
             patch.object(ADAPTER.subprocess, "call", return_value=7) as call:
            self.assertEqual(ADAPTER.main(["--version"]), 7)
            call.assert_called_once_with(["/fixture/real", "--version"])


if __name__ == "__main__":
    unittest.main()
