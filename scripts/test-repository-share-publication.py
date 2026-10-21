#!/usr/bin/env python3
"""Packaging/readback refusal controls; synthetic data is not native acceptance.

# arming: ci; release asset publication contract invokes this script.
"""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


publisher = load("publisher_test", ROOT / "publish_repository_share.py")
verifier = load("verifier_test", ROOT / "verify-repository-share.py")


class Publication(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.stage = self.root / "stage"
        self.stage.mkdir()
        recipe = {"model": "model.onnx", "model_sha256": "a" * 64, "tokenizer_sha256": "b" * 64, "dimension": 384}
        for name in ("export.nt", "manifest.ttl", "shapes.ttl", "queries.ttl", "alignments.ttl"):
            (self.stage / name).write_text("# synthetic fixture only\n")
        self.manifest = {
            "embedding": recipe,
            "files": {"alignments": "alignments.ttl"},
            "alignments_hash": "sha256:" + publisher.digest(self.stage / "alignments.ttl"),
        }
        (self.stage / "manifest.json").write_text(json.dumps(self.manifest))
        self.archive = self.root / "seeds-seeds-ai-v1.2.3-repository.pendant.tar.gz"
        self.revision = "1" * 40

    def prepare(self):
        publisher.package(self.stage, self.archive)
        sha = publisher.digest(self.archive)
        self.archive.with_name(self.archive.name + ".sha256").write_text(sha + "  " + self.archive.name + "\n")
        self.proof = {
            "schema": "quipu.repository-share-producer/v1",
            "source_repository": "scbrown/seeds",
            "source_revision": self.revision,
            "release_tag": "seeds-ai-v1.2.3",
            "archive_sha256": sha,
            "destination": "outward",
            "quipu_sha256": "2" * 64,
            "quipu_server_sha256": "5" * 64,
            "bobbin_sha256": "3" * 64,
            "engine_source_revision": "4" * 40,
        }
        self.proof_path = self.archive.with_name(self.archive.name + ".provenance.json")
        self.proof_path.write_text(json.dumps(self.proof))

    def test_deterministic_archive_and_exact_readback(self):
        self.prepare()
        other = self.root / "second.tar.gz"
        publisher.package(self.stage, other)
        self.assertEqual(self.archive.read_bytes(), other.read_bytes())
        self.assertEqual(verifier.verify(self.root, "seeds-ai-v1.2.3", self.revision)["source_revision"], self.revision)

    def test_required_members_and_recipe(self):
        (self.stage / "queries.ttl").unlink()
        with self.assertRaisesRegex(ValueError, "required nonempty"):
            publisher.check_share(self.stage)
        (self.stage / "queries.ttl").write_text("# query")
        (self.stage / "manifest.json").write_text(json.dumps({**self.manifest, "embedding": None}))
        with self.assertRaisesRegex(ValueError, "provider-byte"):
            publisher.check_share(self.stage)

    def test_alignment_carrier_must_be_declared_sealed_and_present(self):
        publisher.check_share(self.stage)
        undeclared = {k: v for k, v in self.manifest.items() if k not in ("files", "alignments_hash")}
        mis_hashed = {**self.manifest, "alignments_hash": "sha256:" + "0" * 64}
        for label, manifest in [("undeclared", undeclared), ("hash mismatch", mis_hashed)]:
            (self.stage / "manifest.json").write_text(json.dumps(manifest))
            with self.subTest(label), self.assertRaisesRegex(ValueError, "alignment carrier"):
                publisher.check_share(self.stage)
        (self.stage / "manifest.json").write_text(json.dumps(self.manifest))
        (self.stage / "alignments.ttl").unlink()
        with self.assertRaisesRegex(ValueError, "alignment carrier"):
            publisher.check_share(self.stage)

    def test_text_line_span_and_private_members_refuse(self):
        for predicate in ("content", "startLine", "endLine"):
            (self.stage / "export.nt").write_text(
                "<https://example.org/s> <http://aegis.gastown.local/ontology/" + predicate + '> "body" .\n'
            )
            with self.subTest(predicate=predicate), self.assertRaisesRegex(ValueError, "duplicates source"):
                publisher.check_share(self.stage)
        (self.stage / "export.nt").write_text("# empty synthetic")
        (self.stage / "policy-token").write_text("not a real token")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            publisher.check_share(self.stage)

    def test_foreign_repository_and_wrong_source_refuse(self):
        self.prepare()
        for field, value in [
            ("source_repository", "scbrown/quipu"),
            ("source_revision", "f" * 40),
            ("release_tag", "seeds-ai-v9.9.9"),
            ("archive_sha256", "f" * 64),
        ]:
            self.proof_path.write_text(json.dumps({**self.proof, field: value}))
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "provenance mismatch"):
                verifier.verify(self.root, "seeds-ai-v1.2.3", self.revision)

    def test_checksum_refuses(self):
        self.prepare()
        self.archive.with_name(self.archive.name + ".sha256").write_text("wrong")
        with self.assertRaisesRegex(ValueError, "checksum"):
            verifier.verify(self.root, "seeds-ai-v1.2.3", self.revision)

    def test_hold_absent_and_dangling_symlink_before_any_build(self):
        hold = self.root / "hold"
        with patch.dict(publisher.os.environ, {"QUIPU_SHARE_HOLD_FILE": str(hold)}):
            self.assertFalse(publisher.held())
            hold.symlink_to(self.root / "missing-target")
            self.assertTrue(publisher.held())
            with self.assertRaisesRegex(ValueError, "held before"):
                publisher.publish("invalid-tag", "invalid-revision", self.root / "output")
            self.assertFalse((self.root / "output").exists())

    def test_unreadable_hold_fails_closed(self):
        with patch.object(Path, "lstat", side_effect=PermissionError("synthetic denied")):
            with self.assertRaisesRegex(ValueError, "cannot inspect publication hold"):
                publisher.held()

    def test_generic_version_prefix_refuses_before_lookup(self):
        class UnexpectedLookup:
            def __call__(self, *args, **kwargs):
                raise AssertionError("no lookup for a foreign tag convention")
        with patch.object(publisher, "held", return_value=False), patch.object(publisher, "command", UnexpectedLookup()):
            with self.assertRaisesRegex(ValueError, "Seeds release tag"):
                publisher.publish("v1.2.3", self.revision, self.root / "output")
        with self.assertRaisesRegex(ValueError, "before lookup"):
            verifier.wait("v1.2.3", self.revision, run=UnexpectedLookup())

    def test_wait_is_bounded(self):
        calls, pauses = [], []

        class Result:
            returncode = 1

        def run(args, **kwargs):
            calls.append(args)
            return Result()

        with self.assertRaisesRegex(ValueError, "bounded"):
            verifier.wait("seeds-ai-v1.2.3", self.revision, attempts=2, pause=pauses.append, run=run)
        self.assertEqual(len(calls), 2)
        self.assertEqual(pauses, [30])


if __name__ == "__main__":
    unittest.main()
