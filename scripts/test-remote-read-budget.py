#!/usr/bin/env python3
"""Real sd bootstrap must share the host ledger, not one token per child."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import unittest

SD = os.environ.get('SEEDS_TEST_SD') or shutil.which('sd')

class Admission(unittest.TestCase):
    def probe(self, limit, precharged=False, verb="list", *, children=1, retry=False, redirect=False, corrupt=False):
        calls = []
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self): self.answer()
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length', '0')))
                self.answer()
            def answer(self):
                calls.append(self.path)
                body = json.dumps({'head': {'vars': []}, 'results': {'bindings': []}}).encode()
                self.send_response(503 if retry and len(calls) == 1 else 302 if redirect and self.path == '/health' else 200)
                if redirect and self.path == '/health':
                    self.send_header('Location', '/redirected')
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *_): pass
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                base = 'http://127.0.0.1:' + str(server.server_port)
                env = {'PATH': os.environ['PATH'], 'HOME': str(root),
                       'XDG_CACHE_HOME': str(root / 'cache'),
                       'XDG_CONFIG_HOME': str(root / 'config'),
                       'ST_QUIPU_READS_PER_MINUTE': str(limit)}
                if precharged or corrupt:
                    directory = root / 'cache/shantytown/read-budget-v1'
                    directory.mkdir(parents=True, mode=0o700)
                    key = hashlib.sha256(json.dumps(['http', '127.0.0.1', server.server_port]).encode()).hexdigest()
                    ledger = directory / (key + '.json')
                    ledger.write_text('{broken' if corrupt else json.dumps([time.time()]))
                    ledger.chmod(0o600)
                command = [SD, '--quipu', base, '--graph', 'urn:fixture:jve', verb, '--json']
                processes = [subprocess.Popen(command, env=env, cwd=root, stdout=subprocess.PIPE,
                                               stderr=subprocess.PIPE, text=True) for _ in range(children)]
                try:
                    results = [subprocess.CompletedProcess(command, process.wait(timeout=15), *process.communicate())
                               for process in processes]
                finally:
                    for process in processes:
                        if process.poll() is None:
                            process.kill()
                        process.communicate()
                return calls, results[0]
            finally:
                server.shutdown(); server.server_close(); thread.join()

    def test_where_is_a_zero_http_location_proof(self):
        calls, result = self.probe(0, verb='where')
        self.assertEqual(calls, [])
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_default_off_control_observes_bootstrap_fanout(self):
        calls, _ = self.probe(0)
        self.assertGreaterEqual(len(calls), 2, calls)

    def test_one_token_caps_all_bootstrap_requests(self):
        calls, result = self.probe(1)
        self.assertLessEqual(len(calls), 1, calls)
        self.assertNotEqual(result.returncode, 0)

    def test_python_precharge_prevents_first_sdk_request(self):
        calls, result = self.probe(1, precharged=True)
        self.assertEqual(calls, [])
        self.assertNotEqual(result.returncode, 0)

    def test_concurrent_sdk_children_share_one_host_limit(self):
        calls, _ = self.probe(3, children=4)
        self.assertLessEqual(len(calls), 3, calls)

    def test_transient_retry_requires_another_admission(self):
        calls, result = self.probe(1, retry=True)
        self.assertEqual(calls, ['/health'])
        self.assertIn('local Quipu read budget:', result.stderr)

    def test_enabled_budget_never_follows_an_uncharged_redirect(self):
        calls, _ = self.probe(10, redirect=True)
        self.assertNotIn('/redirected', calls)

    def test_corrupt_ledger_refuses_before_any_http(self):
        calls, result = self.probe(1, corrupt=True)
        self.assertEqual(calls, [])
        self.assertIn('local Quipu read budget:', result.stderr)

if __name__ == '__main__': unittest.main()
