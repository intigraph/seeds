#!/usr/bin/env python3
"""Compare two sd executables against identical, isolated local-store fixtures."""
import json
import os
from pathlib import Path
import subprocess
import tempfile


def demonstrate(label, binary):
    print(f'\n{label}: two clients read revision 1', flush=True)
    with tempfile.TemporaryDirectory(prefix='revision-demo-') as root:
        root = Path(root)
        (root / 'home').mkdir()
        (root / 'work').mkdir()
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('SEEDS_', 'BR_'))}
        env.update(HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'home/config'),
                   SEEDS_ACTOR='demo-writer', SEEDS_PREFIX='demo')

        def call(*args):
            result = subprocess.run([binary, '--store', str(root / 'fixture.db'),
                                     *args, '--json'], cwd=root / 'work', env=env,
                                    capture_output=True, text=True, check=False)
            return result.returncode, json.loads(result.stdout)

        rc, seed = call('create', 'Concurrent note editing')
        assert rc == 0
        item = seed['id']
        _, first = call('show', item)
        _, second = call('show', item)
        assert first[0]['revision'] == second[0]['revision'] == 1
        print('$ sd update ITEM --notes author-one --if-revision 1', flush=True)
        rc, _ = call('update', item, '--notes', 'author-one', '--if-revision', '1')
        assert rc == 0
        print('exit=0; current revision=2', flush=True)
        print('$ sd update ITEM --notes author-two --force --if-revision 1 --json', flush=True)
        rc, conflict = call('update', item, '--notes', 'author-two', '--force', '--if-revision', '1')
        assert rc == 4
        print(json.dumps(conflict, ensure_ascii=False, sort_keys=True), flush=True)
        _, current = call('show', item)
        assert current[0]['notes'] == 'author-one' and current[0]['revision'] == 2
        print('exit=4; first writer preserved; no stale mutation', flush=True)
        rc, _ = call('update', item, '--notes', 'author-one; rebased author-two',
                     '--force', '--if-revision', '2')
        assert rc == 0
        _, final = call('show', item)
        assert final[0]['revision'] == 3
        print('Explicit reread + rebase: revision=3; notes=' + final[0]['notes'], flush=True)


if __name__ == '__main__':
    demonstrate('BEFORE', os.environ['SEEDS_DEMO_BEFORE'])
    demonstrate('AFTER', os.environ['SEEDS_DEMO_AFTER'])
