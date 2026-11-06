"""Run only against isolated disposable stores; pass baseline and candidate sd."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def demonstrate(binary, baseline):
    with tempfile.TemporaryDirectory() as directory:
        env = {k:v for k,v in os.environ.items() if not k.startswith(('SEEDS_', 'BR_'))}
        env.update(HOME=directory, XDG_CONFIG_HOME=directory)
        prefix = [binary, '--store', str(Path(directory)/'fixture.db'), '--graph', 'urn:demo:work']
        def call(*args, check=True):
            result = subprocess.run(prefix + list(args), cwd=directory, env=env, capture_output=True, text=True)
            if check and result.returncode:
                raise RuntimeError(result.stderr)
            return result
        first = call('create','first','--silent').stdout.strip()
        second = call('create','second','--silent').stdout.strip()
        for identifier in (first,second):
            call('dep','add',identifier,'--blocker','urn:demo:gate','--blocker-graph','urn:demo:foreign','--ask','ASK { FILTER(false) }')
            call('dep','remove',identifier,'--blocker','urn:demo:gate')
        result=call('blockers','reconcile',first,'--json',check=False)
        if baseline:
            assert result.returncode != 0
            print('BEFORE: targeted reconcile rejected; exit',result.returncode)
            return
        assert result.returncode==0,result.stderr
        changed=json.loads(result.stdout)['changed']
        assert len(changed)==1 and changed[0]['issue_id']==first
        sibling=json.loads(call('show',second,'--json').stdout)[0]['status']
        assert sibling=='blocked'
        repeat=json.loads(call('blockers','reconcile',first,'--json').stdout)['changed']
        assert repeat==[]
        events=json.loads(call('blockers','events',first,'--json').stdout)['events']
        assert len(events)==1 and events[0]['issue_id']==first
        print('AFTER: selected seed opened; sibling remains',sibling)
        print('REPLAY: changed',repeat,'; selected pending event count',len(events))


demonstrate(sys.argv[1],True)
demonstrate(sys.argv[2],False)
