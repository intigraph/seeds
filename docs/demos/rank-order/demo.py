"""Two isolated CLI fixtures; no shared board contact. Set RANK_BASE_SD/RANK_HEAD_SD."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

for label, binary in [('base', os.environ['RANK_BASE_SD']), ('head', os.environ['RANK_HEAD_SD'])]:
    print(label, 'binary SHA256', hashlib.sha256(Path(binary).read_bytes()).hexdigest(), flush=True)
    environment = {k:v for k,v in os.environ.items() if not k.startswith(('SEEDS_', 'BR_'))}
    with tempfile.TemporaryDirectory(prefix='rank-demo-') as tmp:
        root=Path(tmp);(root/'home/.config').mkdir(parents=True)
        environment.update(HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'home/.config'))
        def call(*args, refuse=False):
            result=subprocess.run([binary,*args,'--json'],cwd=root,env=environment,capture_output=True,text=True,timeout=10)
            if refuse:
                print('refusal exit:',result.returncode,flush=True)
                print(result.stderr.splitlines()[0],flush=True)
                assert result.returncode==2
                return None
            assert result.returncode==0,result.stderr
            return json.loads(result.stdout)
        print(subprocess.check_output([binary,'--version'],text=True).strip(),flush=True)
        call('init','--prefix','demo')
        seeds=[call('create',name,'--priority','1') for name in ['left','right','moving']]
        ids=[s['id'] for s in seeds]
        print('$ sd ready: initial titles', [r['title'] for r in call('ready')],flush=True)
        if label=='base':
            print('$ sd update moving --before right',flush=True)
            call('update',ids[2],'--before',ids[1],refuse=True)
        else:
            call('update',ids[0],'--rank','1');call('update',ids[1],'--rank','2')
            before=call('show',*ids)
            print('$ sd update moving --before right',flush=True)
            result=call('update',ids[2],'--before',ids[1])
            print('moving rank:',result[0]['rank'],flush=True)
            after=call('show',*ids)
            print('$ sd ready --sort priority,rank:',[(r['title'],r['rank']) for r in call('ready','--sort','priority,rank')],flush=True)
            assert all(b['due_at']==a['due_at'] for b,a in zip(before,after))
            assert before[:2]==after[:2]
            print('neighbor records unchanged; all due dates unchanged',flush=True)
