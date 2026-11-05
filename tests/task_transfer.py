"""GitHub one-shot contract controls; loopback only, inert named FD credential."""
import copy
import http.server
import json
import os
import pathlib
import subprocess
import tempfile
import threading
import urllib.parse


def main():
    binary = os.environ['TRANSFER_SD']
    issues, comments, parents, dependencies, requests = {}, {}, {}, {}, []
    behavior = {'deps': True, 'refuse': False, 'redirect': False, 'drop_labels': False}
    base = '/repos/fixture/board/issues'

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def reply(self, data, status=200):
            body = json.dumps(data).encode() if status != 204 else b''
            self.send_response(status)
            self.send_header('Content-Length', str(len(body)))
            if status == 302:
                self.send_header('Location', '/credential-must-not-follow')
            self.end_headers()
            self.wfile.write(body)

        def dispatch(self):
            path = urllib.parse.urlsplit(self.path).path
            query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
            body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            body = json.loads(body) if body else {}
            requests.append((self.command, path, body))
            assert self.headers.get('Authorization') == 'Bearer inert-transfer-fixture'
            if behavior['refuse']:
                return self.reply({'error': 'inert-transfer-fixture'}, 403)
            if behavior['redirect']:
                return self.reply({}, 302)
            if path == base:
                if self.command == 'GET':
                    rows = list(issues.values())
                    start = (int(query.get('page', ['1'])[0]) - 1) * 100
                    return self.reply(rows[start:start + 100])
                if self.command == 'POST':
                    number = max(issues, default=0) + 1
                    value = {'id': number + 1000, 'number': number,
                             'html_url': f'https://github.example/fixture/board/issues/{number}',
                             'title': body['title'], 'body': body['body'],
                             'state': 'open', 'labels': [] if behavior['drop_labels'] else body['labels'],
                             'assignees': [{'login': name} for name in body.get('assignees', [])]}
                    issues[number] = value
                    comments[number] = []
                    dependencies[number] = []
                    return self.reply(value, 201)
            tail = path.removeprefix(base + '/').split('/')
            if not tail[0].isdigit() or int(tail[0]) not in issues:
                return self.reply({}, 404)
            number = int(tail[0])
            issue = issues[number]
            if len(tail) == 1:
                if self.command == 'PATCH':
                    issue.update(body)
                    if 'assignees' in body:
                        issue['assignees'] = [{'login': name} for name in body['assignees']]
                return self.reply(issue)
            if tail[1] == 'comments':
                if self.command == 'POST':
                    comments[number].append({'id': len(comments[number]) + 5000,
                                             'body': body['body'], 'user': {'login': 'transfer-bot'},
                                             'created_at': '2026-01-02T00:00:00Z'})
                    return self.reply(comments[number][-1], 201)
                return self.reply(comments[number])
            if tail[1] == 'parent':
                return self.reply(issues[parents[number]]) if number in parents else self.reply({}, 404)
            if tail[1] == 'sub_issues':
                child = body['sub_issue_id'] - 1000
                parents[child] = number
                return self.reply(issues[child], 201)
            if tail[1] == 'sub_issue':
                del parents[body['sub_issue_id'] - 1000]
                return self.reply({}, 204)
            if tail[1:3] == ['dependencies', 'blocked_by']:
                if not behavior['deps']:
                    return self.reply({}, 404)
                if self.command == 'POST':
                    dependencies[number].append(body['issue_id'] - 1000)
                    return self.reply(issues[body['issue_id'] - 1000], 201)
                if self.command == 'DELETE':
                    dependencies[number].remove(int(tail[3]) - 1000)
                    return self.reply({}, 204)
                return self.reply([issues[n] for n in dependencies[number]])
            return self.reply({}, 404)

        do_GET = do_POST = do_PATCH = do_DELETE = dispatch

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory() as temp, tempfile.TemporaryFile() as credential:
            root = pathlib.Path(temp)
            credential.write(b'inert-transfer-fixture')
            credential.flush()
            config = root / 'adapter.toml'
            config.write_text(f'endpoint = "http://127.0.0.1:{server.server_port}"\nrepository = "fixture/board"\n')
            env = {'PATH': os.environ['PATH'], 'HOME': temp, 'XDG_CONFIG_HOME': str(root / '.config')}

            def run(*args, ledger='one', code=0, transfer=False):
                call = [binary, '--store', str(root / f'{ledger}.db'), '--graph',
                        f'https://seeds.example/project/{ledger}', '--json', *args]
                if transfer:
                    call += ['--adapter-config', str(config), '--transfer-principal', 'fixture-bot',
                             '--transfer-credential-fd', str(credential.fileno())]
                result = subprocess.run(call, cwd=root, env=env, pass_fds=(credential.fileno(),),
                                        capture_output=True, text=True, timeout=30)
                assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
                assert 'inert-transfer-fixture' not in result.stdout + result.stderr
                data = json.loads(result.stdout) if result.stdout.strip() else None
                if os.environ.get('TRANSFER_DEMO') == '1' and args[0] in ('export', 'import'):
                    print('$ sd ' + ' '.join(args), flush=True)
                    if result.returncode:
                        print(json.dumps(data), flush=True)
                    else:
                        print(json.dumps({'schema': data['schema'], 'direction': data['direction'],
                                          'dry_run': data['dry_run'], 'operations': [
                                              {key: row[key] for key in ('action', 'conflict')}
                                              for row in data['operations']]}), flush=True)
                return data

            parent = run('create', 'Parent', '--type', 'epic')['id']
            child = run('create', 'Child', '--priority', '1', '--description', 'Before',
                        '--assignee', 'fixture-user')['id']
            run('comments', 'add', child, 'Original comment')
            run('dep', 'add', child, parent)
            run('update', child, '--parent', parent)
            selection = f'{parent},{child}'
            preview = run('export', '--to', 'github', '--ids', selection, transfer=True)
            assert preview['dry_run'] and not issues
            assert all(method == 'GET' for method, _, _ in requests)
            requests.clear()
            run('export', '--to', 'github', '--ids', selection, '--execute', transfer=True)
            assert len(issues) == 2 and len(comments[2]) == 1
            assert parents[2] == 1 and dependencies[2] == [1]
            assert 'assignee' not in issues[2] and issues[2]['assignees'] == [{'login': 'fixture-user'}]
            run('export', '--to', 'github', '--ids', selection, '--execute', transfer=True)
            assert len(issues) == 2 and len(comments[2]) == 1
            # Same-board roundtrip reuses the existing seed IDs.
            preview = run('import', '--from', 'github', '--ids', '1,2', transfer=True)
            assert {row['seed'] for row in preview['operations']} == {parent, child}
            original_comments = run('comments', 'list', child)
            run('import', '--from', 'github', '--ids', '1,2', '--execute', transfer=True)
            assert run('comments', 'list', child) == original_comments
            # A different board imports atomically and reruns without duplicates.
            run('create', 'Unrelated', ledger='two')
            run('import', '--from', 'github', '--ids', '1,2', '--execute', ledger='two', transfer=True)
            before = run('list', '--all', '--limit', '0', ledger='two')
            run('import', '--from', 'github', '--ids', '1,2', '--execute', ledger='two', transfer=True)
            assert run('list', '--all', '--limit', '0', ledger='two') == before
            # Changed source is a conflict until the caller chooses its side.
            run('update', child, '--description', 'After', '--force')
            run('export', '--to', 'github', '--ids', selection, '--execute', transfer=True, code=4)
            run('export', '--to', 'github', '--ids', selection, '--execute', '--overwrite', transfer=True)
            assert issues[2]['body'].startswith('After')
            # Dependency API unavailable: retain visible body fallback, no 403 bypass.
            behavior['deps'] = False
            run('export', '--to', 'github', '--ids', selection, '--execute', '--overwrite', transfer=True)
            assert '## Seeds dependencies' in issues[2]['body']
            behavior['deps'] = True
            # Parent and dependency removals are reflected on rerun.
            run('dep', 'remove', child, parent)
            run('update', child, '--parent', '')
            run('export', '--to', 'github', '--ids', selection, '--execute', '--overwrite', transfer=True)
            assert child not in parents and 2 not in parents and not dependencies[2]
            # Native closed state survives a fresh create, not only an update.
            closed = run('create', 'Closed')['id']
            run('close', closed, '--reason', 'fixture complete')
            run('export', '--to', 'github', '--ids', closed, '--execute', transfer=True)
            assert issues[3]['state'] == 'closed'
            behavior['refuse'] = True
            requests.clear()
            run('export', '--to', 'github', '--ids', selection, transfer=True, code=5)
            assert len(requests) == 1 and requests[0][0] == 'GET'
            behavior['refuse'] = False
            behavior['redirect'] = True
            requests.clear()
            run('export', '--to', 'github', '--ids', selection, transfer=True, code=5)
            assert len(requests) == 1
            behavior['redirect'] = False
            # A PR cannot become a seed, and lost labels cannot count as success.
            issues[1]['pull_request'] = {}
            run('import', '--from', 'github', '--ids', '1', transfer=True, code=5)
            del issues[1]['pull_request']
            behavior['drop_labels'] = True
            dropped = run('create', 'Dropped labels')['id']
            run('export', '--to', 'github', '--ids', dropped, '--execute', transfer=True, code=5)
            assert run('show', dropped)[0]['external_ref'] is None
            # The origin marker recovers that partial create instead of duplicating.
            behavior['drop_labels'] = False
            issues[4]['labels'] = ['seeds:status:open', 'seeds:priority:2', 'seeds:type:task']
            run('export', '--to', 'github', '--ids', dropped, '--execute', transfer=True)
            assert len(issues) == 4
            print('GitHub transfer fixtures: preview, native mappings/relations/comments, replay, updates, fallback, refusals, redirect, partial-create recovery PASS')
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == '__main__':
    main()
