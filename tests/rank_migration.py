"""Standard-library controls for the SHA-bound, no-retry migration helper."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('migration', Path(__file__).parents[1] / 'scripts/migrate-ranks.py')
migration = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(migration)


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.row = dict(id='test-a', priority=1, revision=4, rank=None,
                        due_at='2026-12-01', status='open')
        plan = root / 'plan.json'
        plan.write_text(json.dumps(dict(quipu='http://127.0.0.1', graph='urn:test', rows=[
            dict(id='test-a', rank='1', expected={k: self.row[k] for k in ('priority','revision','rank','due_at')})])))
        self.args = SimpleNamespace(plan=str(plan), receipt=str(root/'receipt.json'),
            execute=False, plan_sha256=None, sd='sd', quipu='http://127.0.0.1', graph='urn:test')

    def execution(self):
        self.args.execute = True
        self.args.plan_sha256 = hashlib.sha256(Path(self.args.plan).read_bytes()).hexdigest()

    def test_rehearsal_never_writes(self):
        with patch.object(migration.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout=json.dumps([self.row]))) as call:
            migration.run(self.args)
        self.assertEqual(call.call_count, 1)
        self.assertIn('show', call.call_args.args[0])

    def test_stale_revision_refuses_before_write(self):
        with patch.object(migration.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout=json.dumps([{**self.row,'revision':5}]))):
            with self.assertRaises(ValueError):
                migration.run(self.args)
        self.assertFalse(Path(self.args.receipt).exists())

    def test_execution_and_rollback_guard(self):
        self.execution()
        calls = []
        def call(command, **kwargs):
            calls.append(command)
            row = self.row if 'show' in command else {**self.row, 'rank':'1', 'revision':5}
            return SimpleNamespace(returncode=0, stdout=json.dumps([row]))
        with patch.object(migration.subprocess, 'run', call):
            migration.run(self.args)
        self.assertIn('--if-revision', calls[1])
        self.assertNotIn('--due', calls[1])
        rollback = json.loads(Path(self.args.receipt).read_text())['rollback_plan'][0]
        self.assertIsNone(rollback['rank'])
        self.assertEqual(rollback['expected']['revision'], 5)

    def test_unknown_write_preserves_pending_and_never_retries(self):
        self.execution()
        calls = []
        def call(command, **kwargs):
            calls.append(command)
            if 'update' in command:
                raise migration.subprocess.TimeoutExpired(command,120)
            return SimpleNamespace(returncode=0, stdout=json.dumps([self.row]))
        with patch.object(migration.subprocess, 'run', call):
            with self.assertRaises(migration.subprocess.TimeoutExpired):
                migration.run(self.args)
        self.assertEqual(len(calls), 2)
        self.assertEqual(json.loads(Path(self.args.receipt).read_text())['pending_id'], 'test-a')


if __name__ == '__main__':
    unittest.main()
