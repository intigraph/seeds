#!/usr/bin/env python3
"""Compare released and candidate CLIs using private, isolated local stores."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

p = argparse.ArgumentParser()
p.add_argument('--before', required=True)
p.add_argument('--after', required=True)
a = p.parse_args()
for label, binary in [('before', a.before), ('after', a.after)]:
    binary = str(Path(binary).resolve())
    with tempfile.TemporaryDirectory(prefix='ops-record-demo-') as temp:
        root = Path(temp)
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('SEEDS_', 'QUIPU_', 'BR_', 'SHANTY_'))}
        env.update(HOME=temp, XDG_CONFIG_HOME=str(root / 'config'))
        def run(*args):
            return subprocess.run(
                [binary, '--store', str(root / 'fixture.db'), '--graph',
                 'urn:ops-demo', '--actor', 'demo', '--json', *args],
                cwd=temp, env=env, text=True, capture_output=True, check=False)
        assert run('create', 'actionable work').returncode == 0
        result = run('create', 'machine operation', '--type', 'ops')
        print(f'{label}: create --type ops exit={result.returncode}')
        if label == 'before':
            assert result.returncode == 2
            continue
        assert result.returncode == 0, result.stderr
        record = json.loads(result.stdout)
        # Create's JSON is a seed; no real ledger or network is involved.
        ident = record['id']
        work = json.loads(run('list').stdout)
        ops = json.loads(run('list', '--type', 'ops').stdout)
        ready = json.loads(run('ready').stdout)
        stats = json.loads(run('stats').stdout)['summary']
        assert work['total'] == 1 and ops['total'] == 1
        assert len(ready) == 1
        assert stats['total_issues'] == 1 and stats['ops_issues'] == 1
        assert run('show', ident).returncode == 0
        assert run('close', ident, '--reason', 'operation resolved').returncode == 0
        print('after: backlog=1; ready=1; explicit operations=1; work total=1; ops total=1')
        print('after: direct show and normal close succeed')
