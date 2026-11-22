#!/usr/bin/env python3
"""Actual CLI behavior on two private scratch stores; never a live board."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

for label, binary in [('before', os.environ['DEMO_BEFORE']), ('after', os.environ['DEMO_AFTER'])]:
    binary = str(Path(binary).resolve())
    with tempfile.TemporaryDirectory(prefix='ask-demo-') as temp:
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('SEEDS_', 'QUIPU_', 'SHANTY_', 'BR_'))}
        env.update(HOME=temp, XDG_CONFIG_HOME=temp + '/config')
        def run(*args):
            return subprocess.run([binary, '--store', temp + '/fixture.db', '--graph',
                                   'urn:ask-demo', '--actor', 'demo', '--json', *args],
                                  cwd=temp, env=env, text=True, capture_output=True, check=False)
        made = run('create', 'Implement the selected option')
        assert made.returncode == 0, made.stderr
        work = json.loads(made.stdout)['id']
        made = run('create', 'Which option?', '--type', 'ask', '--unblocks', work,
                   '--default', 'Keep the current option', '--expires-at', '2026-12-01T12:00:00Z')
        print(f'{label}: create --type ask with unblocks/default/expires-at: exit {made.returncode}')
        if label == 'before':
            assert made.returncode == 2
            print('before: Ask input is unsupported')
            continue
        assert made.returncode == 0, made.stderr
        ask = json.loads(made.stdout)['id']
        listing = json.loads(run('list').stdout)
        asks = json.loads(run('list', '--type', 'ask').stdout)
        count = json.loads(run('count').stdout)
        ready = json.loads(run('ready').stdout)
        assert listing['total'] == 1 and asks['total'] == 1
        assert count['count'] == 1 and len(ready) == 1
        assert ready[0]['id'] == work
        record = json.loads(run('show', ask).stdout)[0]
        assert record['unblocks'] == work
        assert record['default'] == 'Keep the current option'
        assert record['expires_at'] == '2026-12-01T12:00:00Z'
        print('after: board list=1; board count=1; ready=1; explicit Ask list=1')
        print('after: exact Ask read retains linked item, fallback and expiry')
        assert run('close', ask, '--reason', 'Operator selected the current option').returncode == 0
        print('after: Ask closes through the ordinary lifecycle; linked work remains ready')
        assert len(json.loads(run('ready').stdout)) == 1
