#!/usr/bin/env python3
"""Compare old/candidate sd against new private stores, never a shared server.

Set BEFORE_SD and AFTER_SD to the two executables. The default old executable
is sd on PATH; the candidate defaults to target/debug/sd.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

before = os.environ.get('BEFORE_SD') or shutil.which('sd')
after = str(Path(os.environ.get('AFTER_SD', 'target/debug/sd')).resolve())
print('Private ASK deferral control (fresh local stores; no production calls)')
print('case                 before exit    candidate exit')
with tempfile.TemporaryDirectory(prefix='ask-deferral-demo-') as directory:
    root = Path(directory)
    home = root / 'home'
    home.mkdir()
    env = {'PATH': '/usr/local/bin:/usr/bin:/bin', 'HOME': str(home),
           'XDG_CONFIG_HOME': str(home / '.config'), 'USER': 'fixture'}
    for label in ['needs-stiwi', 'blocked:human', 'desk', 'decision-needed', 'title-prefix', 'non-ASK']:
        codes = []
        for name, binary in [('before', before), ('candidate', after)]:
            title = 'ASK Stiwi: choose' if label == 'title-prefix' else 'Technical control'
            base = [binary, '--store', str(root / (name + '.db')),
                    '--graph', 'https://example.invalid/ask-demo']
            args = ['create', title, '--json']
            if label not in ('title-prefix', 'non-ASK'):
                args += ['--labels', label]
            item = json.loads(subprocess.check_output(base + args, env=env, cwd=home, text=True))
            result = subprocess.run(base + ['defer', item['id'], '--until', '2027-01-01'],
                                    env=env, cwd=home, text=True, capture_output=True)
            codes.append(result.returncode)
        assert codes == ([0, 0] if label == 'non-ASK' else [0, 5]), (label, codes)
        print(f'{label:20} {codes[0]:11} {codes[1]:17}')
print('PASS: five ASK cases refuse; ordinary deferral still succeeds.')
