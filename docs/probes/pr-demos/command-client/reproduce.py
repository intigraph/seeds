#!/usr/bin/env python3
"""Compare real CLI headers against one private, synthetic HTTP read fixture."""
import hashlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

seen = []


class Fixture(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self):
        size = int(self.headers.get('Content-Length', '0'))
        if size > 1024 * 1024:
            raise RuntimeError('oversized fixture request')
        body = self.rfile.read(size).decode()
        seen.append((self.command, self.path, self.headers.get('X-Quipu-Client')))
        if self.path == '/query':
            if 'SELECT ?probe WHERE' in body:
                value = {'head': {'vars': ['probe']}, 'results': {'bindings': [
                    {'probe': {'type': 'literal', 'value': 'complete'}}]}, 'truncated': False}
            elif 'ASK' in body:
                value = {'head': {}, 'boolean': False, 'result': False}
            else:
                value = {'head': {'vars': []}, 'results': {'bindings': []},
                         'rows': [], 'count': 0, 'truncated': False}
        elif self.path == '/health':
            value = {'status': 'ok', 'graphs': []}
        else:
            raise RuntimeError('unexpected fixture endpoint')
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = reply
    do_POST = reply


def run(binary, label, expected, url, subject):
    begin = len(seen)
    print(f'{label}: sd --quipu {url} --graph https://example.test/project/demo '
          f'show {subject} --json', flush=True)
    with tempfile.TemporaryDirectory(prefix='command-client-demo-') as home:
        result = subprocess.run(
            [str(Path(binary).resolve()), '--quipu', url, '--graph',
             'https://example.test/project/demo', 'show', subject, '--json'],
            env={'PATH': '/usr/bin:/bin', 'HOME': home, 'XDG_CONFIG_HOME': home,
                 'XDG_STATE_HOME': home},
            capture_output=True, text=True, timeout=10)
    if result.returncode != 3:
        raise RuntimeError(f'expected synthetic missing-subject exit3, got {result.returncode}')
    requests = seen[begin:]
    labels = sorted({client for _, _, client in requests})
    if len(requests) != 5 or labels != [expected]:
        raise RuntimeError(f'unexpected fixture request attribution: {requests!r}')
    print(f'  exit={result.returncode} (synthetic subject deliberately absent)')
    print(f'  actual requests={len(requests)}; X-Quipu-Client={labels!r}', flush=True)


def main():
    if len(sys.argv) != 3:
        raise SystemExit('usage: reproduce.py BEFORE_SD AFTER_SD')
    before, after = sys.argv[1:]
    for label, binary in [('before', before), ('after', after)]:
        print(f'{label} binary SHA256={hashlib.sha256(Path(binary).read_bytes()).hexdigest()}')
    server = http.server.HTTPServer(('127.0.0.1', 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        url = f'http://127.0.0.1:{server.server_port}'
        run(before, 'before', 'seeds', url, 'sample-1')
        run(after, 'after', 'seeds/show', url, 'sample-1')
        run(after, 'argument-control', 'seeds/show', url, 'different-2')
        print('PASS: command label changes; different arguments do not change it.')
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


if __name__ == '__main__':
    main()
