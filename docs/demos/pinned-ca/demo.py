#!/usr/bin/env python3
"""Offline CLI trust demonstration; no keys, production server or writes."""
import argparse
import hashlib
import importlib.util
import http.server
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[3]
FIXTURE_SCRIPT = ROOT / 'tests/fixtures/tls/generate.py'
FIXTURES = None


def probe(binary, label, *, foreign=False, hostname='localhost', missing=False, tampered=False):
    with tempfile.TemporaryDirectory() as temporary:
        home = Path(temporary)
        config = home / '.config/seeds'
        config.mkdir(parents=True)
        ca = home / 'roots.pem'
        original = (FIXTURES / ('foreign-ca.pem' if foreign else 'ca.pem')).read_bytes()
        if not missing:
            ca.write_bytes(original + (b'\n' if tampered else b''))
        (config / 'config.toml').write_text(
            '[quipu]\nca_file = ' + repr(str(ca)) + '\nca_sha256 = "' +
            hashlib.sha256(original).hexdigest() + '"\n')
        calls = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                calls.append(self.path)
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{}')
            def log_message(self, *args):
                pass
        server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(FIXTURES / 'server.pem', FIXTURES / 'server.key')
        server.socket = context.wrap_socket(server.socket, server_side=True)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        env = {k: v for k, v in os.environ.items() if not k.startswith(('SEEDS_', 'QUIPU_', 'XDG_'))}
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / '.config'))
        try:
            answer = subprocess.run([str(binary), '--quipu',
                f'https://{hostname}:{server.server_port}', 'identity', '--json'],
                cwd=home, env=env, capture_output=True, text=True, timeout=15)
        finally:
            server.shutdown()
            server.server_close()
            worker.join()
        output = (answer.stdout + answer.stderr).strip().replace(str(home), '$FIXTURE')
        print(f'{label}: exit={answer.returncode}, HTTPS health requests={len(calls)}')
        print(output)
        return answer, calls


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--before', required=True, type=Path)
    parser.add_argument('--after', required=True, type=Path)
    args = parser.parse_args()
    global FIXTURES
    before, after = args.before.resolve(), args.after.resolve()
    fixture = tempfile.TemporaryDirectory()
    FIXTURES = Path(fixture.name)
    generator = importlib.util.spec_from_file_location('tls_generate', FIXTURE_SCRIPT)
    module = importlib.util.module_from_spec(generator)
    generator.loader.exec_module(module)
    module.generate(FIXTURES)
    for label, binary in [('before', before), ('after', after)]:
        print(label, subprocess.check_output([str(binary), '--version'], text=True).strip())
    old, calls = probe(before, 'before / valid CA')
    assert old.returncode != 0 and not calls and 'ca_file' in old.stdout + old.stderr
    new, calls = probe(after, 'after / valid CA')
    assert calls == ['/health'] and 'identity requires an enrolled signing key' in new.stdout + new.stderr
    print('TLS succeeded; identity correctly refuses because this fixture has no signing key.')
    for label, options in [('foreign CA', {'foreign': True}), ('wrong hostname', {'hostname': '127.0.0.1'}),
                           ('missing CA', {'missing': True}), ('changed CA bytes', {'tampered': True})]:
        answer, calls = probe(after, 'after / ' + label, **options)
        assert answer.returncode != 0 and not calls
    print('All trust controls passed. This is a localhost fixture, not native identity acceptance.')
    fixture.cleanup()


if __name__ == '__main__':
    main()
