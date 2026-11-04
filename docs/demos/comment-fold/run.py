#!/usr/bin/env python3
"""Compare supplied base/candidate binaries using disposable, local-only stores."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

for label, variable in [('BEFORE', 'COMMENT_FOLD_BEFORE'), ('AFTER', 'COMMENT_FOLD_AFTER')]:
    binary = Path(os.environ[variable]).resolve()
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        env = {'HOME': str(root), 'XDG_CONFIG_HOME': str(root / 'cfg'),
               'PATH': os.defpath, 'SEEDS_ACTOR': 'fixture'}
        def run(*args):
            result = subprocess.run([str(binary), *args], cwd=root, env=env,
                                    text=True, capture_output=True, check=True)
            return result.stdout
        def query(*args):
            return json.loads(run(*args, '--json'))
        version = run('--version').strip()
        query('init', '--prefix', 'demo')
        identifier = query('create', 'Current task', '--description', 'Current scope')['id']
        for n in range(1, 8):
            query('comments', 'add', identifier, f'evidence-{n}')
        shown = query('show', identifier)[0]
        history = query('comments', 'list', identifier)
        result = {'binary_version': version,
                  'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                  'displayed': len(shown['comments']),
                  'displayed_positions': [c['id'] for c in shown['comments']],
                  'total': shown.get('comments_total'), 'folded': shown.get('comments_folded'),
                  'stored_history': len(history)}
        if label == 'BEFORE':
            assert len(shown['comments']) == len(history) == 7
        else:
            complete = query('show', identifier, '--all-comments')[0]
            assert len(shown['comments']) == 5 and shown['comments'][0]['id'] == 3
            assert shown['comments_total'] == 7 and shown['comments_folded'] == 2
            assert complete['comments'] == history and complete['comments_folded'] == 0
            result['explicit_full_history'] = len(complete['comments'])
        print(label, json.dumps(result), flush=True)
