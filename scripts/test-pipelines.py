#!/usr/bin/env python3
"""T4 live acceptance. All children/containers are owned and cleaned by this run."""
import concurrent.futures
import http.cookiejar
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

CHECKS = 0
FAILURES = []


def check(label, condition, detail=''):
    global CHECKS
    CHECKS += 1
    if not condition:
        FAILURES.append(label)
    print(f'  {"ok" if condition else "FAIL"} {label}' + (f': {detail}' if detail else ''), flush=True)


def command(args, **kwargs):
    return subprocess.run(args, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs).stdout


def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def eventually(fn, seconds=30):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = fn()
        if value:
            return value
        time.sleep(.1)
    raise AssertionError('condition did not become true in time')


def main():
    global CHECKS
    work = Path(tempfile.mkdtemp(prefix='pipeline-check-'))
    name = 'pipeline-check-' + uuid.uuid4().hex[:12]
    server_port, ssh_port, git_port = port(), port(), port()
    base = f'http://127.0.0.1:{server_port}/api'
    processes = []
    files = []
    sample_stop = threading.Event()
    samples = {'runner': [], 'ssh': []}
    sample_errors = []
    sampler = None
    secret = 'private-' + uuid.uuid4().hex
    username = 'user-' + uuid.uuid4().hex
    root_dir = '/tmp/' + name
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def api(path, body=None, method=None, status=200):
        req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                     headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'},
                                     method=method or ('POST' if body is not None else 'GET'))
        try:
            with opener.open(req, timeout=15) as response:
                code, raw = response.status, response.read()
        except urllib.error.HTTPError as e:
            code, raw = e.code, e.read()
        if code != status:
            raise AssertionError(f'{method or req.method} {path}: HTTP {code}, wanted {status}: {raw[:300]!r}')
        return json.loads(raw) if raw else None

    def launch(args, log, env=None):
        output = open(work / log, 'wb')
        files.append(output)
        p = subprocess.Popen(args, stdout=output, stderr=output, env=env, stdin=subprocess.PIPE if "--desktop-control" in args else None)
        processes.append(p)
        return p

    def create(resource, body, ws='ops'):
        return api(f'/workspaces/{ws}/{resource}', body, status=201)['id']

    def pipeline(host, steps, ws='ops', repo=None, target_kind='host', artifacts=None):
        definition = {'workdir': root_dir + '/{{run}}',
                      'env': [{'name': 'BUILD_VALUE', 'value': 'env-{{branch}}-{{run}}'}], 'steps': steps}
        if artifacts is not None: definition['artifacts'] = artifacts
        first_command=definition['steps'][0]['run']
        definition['steps']=[dict(step) for step in steps]
        definition['steps'][0]['run']='{{ .Values.command }}'
        template=create('pipeline-templates',{'name':'live-template','content':json.dumps(definition),'parameters':json.dumps([{'name':'command','type':'string'}])},ws)
        generated=api(f'/workspaces/{ws}/pipeline-templates/{template}/actions/render',{'version':1,'values':{'command':first_command}})['definition']
        return create('pipelines', {'name': 'live-' + uuid.uuid4().hex[:8], 'target_kind': target_kind,
            'target_ref': host, 'repository_id': repo if repo is not None else repo_id,
            'definition': generated}, ws)

    def trigger(pid, ws='ops', values=None):
        return api(f'/workspaces/{ws}/pipelines/{pid}/actions/run', body=values, method='POST', status=201)

    def detail(rid, ws='ops'):
        return api(f'/workspaces/{ws}/pipeline-runs/{rid}')

    def done(rid):
        def poll():
            row = detail(rid)
            return row if row['state'] not in ('running', 'queued') else None
        return eventually(poll)

    def logs(rid):
        after, items = 0, []
        while True:
            row = api(f'/workspaces/ops/pipeline-runs/{rid}/logs?after={after}')
            if not row['items']:
                assert row['next_seq'] == after
                break
            assert all(i['seq'] > after for i in row['items'])
            items.extend(row['items'])
            after = row['next_seq']
        return items

    def local_or_ssh(kind, script):
        args = ['sh', '-c', script] if kind == 'runner' else ['docker', 'exec', name, 'sh', '-c', script]
        return command(args).decode()

    def sample():
        while not sample_stop.is_set():
            for kind in samples:
                try:
                    args = ['ps', '-axww', '-o', 'pid=,args='] if kind == 'runner' else [
                        'docker', 'exec', name, 'ps', '-axww', '-o', 'pid=,args=']
                    samples[kind].append(command(args).decode())
                except Exception as e:
                    sample_errors.append(type(e).__name__)
            sample_stop.wait(.08)

    try:
        dockerfile = b'''FROM alpine:3.21
RUN apk add --no-cache openssh-server openssh-sftp-server coreutils git git-daemon python3 procps \
 && adduser -D -s /bin/sh ferry \
 && echo 'ferry:ferry-test' | chpasswd \
 && printf '%s\\n' 'HostKey /etc/ssh/ssh_host_ed25519_key' 'PasswordAuthentication yes' 'PermitRootLogin no' 'Subsystem sftp /usr/lib/ssh/sftp-server' > /etc/ssh/sshd_config
CMD ["sh", "-c", "ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key && exec /usr/sbin/sshd -D -e"]
'''
        print('== building local SSH + Git fixture', flush=True)
        command(['docker', 'build', '-q', '-t', 'pipeline-live-test', '-'], input=dockerfile)
        command(['git', 'init', '-b', 'main', str(work / 'source')])
        (work / 'source' / 'payload.txt').write_text('local-private-repository\n')
        command(['git', '-C', str(work / 'source'), 'add', 'payload.txt'])
        command(['git', '-C', str(work / 'source'), '-c', 'user.name=fixture', '-c',
                 'user.email=fixture@example.invalid', 'commit', '-m', 'fixture'])
        commit = command(['git', '-C', str(work / 'source'), 'rev-parse', 'HEAD']).decode().strip()
        command(['git', 'clone', '--bare', str(work / 'source'), str(work / 'private.git')])
        (work / 'auth.json').write_text(json.dumps({'username': username, 'secret': secret}))
        os.chmod(work / 'auth.json', 0o600)
        command(['docker', 'run', '-d', '--name', name, '-p', f'127.0.0.1:{ssh_port}:22',
                 '-p', f'127.0.0.1:{git_port}:{git_port}', 'pipeline-live-test'])
        command(['docker', 'exec', name, 'mkdir', '-p', '/srv/git'])
        command(['docker', 'cp', str(work / 'private.git'), name + ':/srv/git/private.git'])
        command(['docker', 'cp', str(work / 'auth.json'), name + ':/srv/auth.json'])
        command(['docker', 'cp', 'scripts/pipeline-git-http.py', name + ':/srv/git_fixture.py'])
        command(['docker', 'exec', name, 'git', 'config', '--global', '--add', 'safe.directory', '/srv/git/private.git'])
        command(['docker', 'exec', '-d', name, 'python3', '/srv/git_fixture.py', '/srv/git', '/srv/auth.json', str(git_port)])
        repo_url = f'http://127.0.0.1:{git_port}/private.git'
        def git_ready():
            try:
                opener.open(repo_url + '/info/refs', timeout=1)
            except urllib.error.HTTPError as e:
                return e.code == 401
            except OSError:
                return False
        eventually(git_ready)
        check('1 unauthenticated private HTTP Git denied', git_ready(), 'HTTP 401')
        env = dict(os.environ, FERRY_DATA_DIR=str(work / 'data'), FERRY_BIND=f'127.0.0.1:{server_port}')
        env.pop('FERRY_DATABASE_URL', None)
        if os.environ.get('FERRY_TEST_DATABASE_URL'):
            env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
        server = launch(['target/debug/ferry-server', '--desktop-control'], 'server.log', env)
        def ready():
            if server.poll() is not None:
                raise AssertionError((work / 'server.log').read_text())
            try:
                return api('/status')['ok']
            except OSError:
                return False
        eventually(ready)
        command(['python3', 'scripts/test-setup.py', base, str(work / 'data')])
        api('/users', {'email': 'test@example.invalid', 'password': 'correct-horse-battery', 'name': 'test'})
        workspace = api('/workspaces', {'name': 'ops', 'slug': 'ops'})
        api('/workspaces', {'name': 'other', 'slug': 'other'})
        group = create('host-groups', {'name': 'execution-group'})
        runner = create('hosts', {'name': 'real-runner', 'connection': 'runner', 'group_id': group})
        token = api('/workspaces/ops/runner-tokens', {'host_id': runner}, status=201)['token']
        launch(['target/debug/ferry-runner', '--server', base[:-4], '--token', token,
                '--state-dir', str(work / 'runner')], 'runner.log')
        eventually(lambda: api('/workspaces/ops/hosts/' + runner)['state'] == 'online')
        ssh_cred = create('credentials', {'name': 'ssh', 'kind': 'password', 'username': 'ferry', 'secret': 'ferry-test'})
        ssh = create('hosts', {'name': 'real-ssh', 'connection': 'ssh', 'address': '127.0.0.1',
                              'port': ssh_port, 'username': 'ferry', 'credential_id': ssh_cred, 'group_id': group})
        eventually(lambda: api('/workspaces/ops/hosts/' + ssh + '/actions/test-connection', method='POST')['ok'])
        cred = create('credentials', {'name': 'git', 'kind': 'password', 'username': username, 'secret': secret})
        repo_id = create('repositories', {'name': 'private', 'url': repo_url, 'default_branch': 'main', 'credential_id': cred})
        sampler = threading.Thread(target=sample, daemon=True)
        sampler.start()
        all_runs, successful = [], []
        # Keep each credential process observable long enough for ps sampling.
        # The old ASKPASS path includes the username in Git's password-prompt argv.
        success_steps = [
            {'name': 'clone', 'run': 'set -e; helper="${GIT_CONFIG_VALUE_1:-$GIT_ASKPASS}"; mv "$helper" "$helper.real"; printf \'#!/bin/sh\\nsleep 0.5\\nexec "%s.real" "$@"\\n\' "$helper" > "$helper"; chmod 700 "$helper"; git clone --depth 1 -b {{branch}} {{repo_url}} .; echo CLONE_OK; sleep 1'},
            {'name': 'build {{run}}', 'run': 'set -e; test "$BUILD_VALUE" = "env-main-{{run}}" && cat payload.txt && test "$(git rev-parse HEAD)" = "{{commit}}" && git rev-parse HEAD && printf "BUILD_OK\\n"; printf "BUILD_STDERR\\n" >&2'}]
        for kind, host in [('runner', runner), ('ssh', ssh)]:
            print('== ' + kind, flush=True)
            pid = pipeline(host, success_steps, artifacts=["payload.txt"])
            run = trigger(pid, values={'branch': 'main', 'commit': commit}); rid = run['id']; all_runs.append(rid)
            row = done(rid); entries = logs(rid); text = ''.join(i['chunk'] for i in entries)
            successful.append(text)
            check(f'{2 if kind == "runner" else 3} {kind} two steps succeed',
                  row['state'] == 'succeeded' and [s['state'] for s in row['steps']] == ['succeeded'] * 2,
                  f"state={row['state']}, steps={[s['state'] for s in row['steps']]}")
            check(f'1 {kind} authenticated clone contains actual Git commit', commit in text and 'local-private-repository' in text)
            check(f'{kind} complete ordered stdout/stderr and incremental cursor',
                  all(t in text for t in ['CLONE_OK', 'BUILD_OK', 'BUILD_STDERR'])
                  and [i['seq'] for i in entries] == list(range(1, len(entries) + 1))
                  and [i['step_index'] for i in entries] == sorted(i['step_index'] for i in entries),
                  f"seq={[i['seq'] for i in entries]}")
            check(f'{kind} workdir and env applied', local_or_ssh(kind, f'cat {root_dir}/{run["number"]}/payload.txt').strip() == 'local-private-repository')
            artifacts = row['artifacts']
            check(kind+' successful build archives declared output with content hash', len(artifacts)==1 and artifacts[0]['path']=='payload.txt' and len(artifacts[0]['sha256'])==64)
            artifact=artifacts[0]
            with opener.open(base+'/workspaces/ops/pipeline-artifacts/'+artifact['id']+'/download') as response:
                archived=response.read()
            import hashlib
            check(kind+' artifact download matches original bytes and sha256', archived.strip()==b'local-private-repository' and hashlib.sha256(archived).hexdigest()==artifact['sha256'] and len(archived)==artifact['size'])
            local_or_ssh(kind, f"printf changed > {root_dir}/{run['number']}/payload.txt")
            with opener.open(base+'/workspaces/ops/pipeline-artifacts/'+artifact['id']+'/download') as response:
                check(kind+' archived output is independent of mutable build directory', response.read()==archived)
            missing=trigger(pipeline(host,[{'name':'build','run':'true'}],artifacts=['missing.bin']))['id']; all_runs.append(missing)
            missing_row=done(missing)
            check(kind+' missing declared artifact fails the build without publishing partial output',missing_row['state']=='failed' and missing_row['artifacts']==[])
            bad_credential = create('credentials', {'name': 'denied-'+kind, 'kind': 'password', 'username': username, 'secret': secret+'-wrong'})
            bad_repository = create('repositories', {'name': 'denied-'+kind, 'url': repo_url, 'credential_id': bad_credential})
            denied = trigger(pipeline(host, success_steps, repo=bad_repository))['id']; all_runs.append(denied)
            rejected = done(denied)
            check(kind+' invalid credentials fail without interactive fallback', rejected['state']=='failed' and rejected['steps'][0]['state']=='failed')

            # Deliberately echo the password on both streams from FILE contents;
            # the test itself never injects it into any command's argv.
            leak_steps = [{'name': 'redaction', 'run': 'd=$(dirname "${GIT_CONFIG_VALUE_1:-$GIT_ASKPASS}"); cat "$d/password"; cat "$d/password" >&2; stat -c %a "$d/password" 2>/dev/null || stat -f %Lp "$d/password"; sleep 1; exit 7'},
                          {'name': 'must skip', 'run': 'echo SHOULD_NOT_RUN; touch forbidden'}]
            rid = trigger(pipeline(host, leak_steps))['id']; all_runs.append(rid)
            row = done(rid); text = ''.join(i['chunk'] for i in logs(rid))
            check(f'4 {kind} failure stops next step', row['state'] == 'failed' and
                  [s['state'] for s in row['steps']] == ['failed', 'skipped'] and row['steps'][0]['exit_code'] == 7
                  and 'SHOULD_NOT_RUN' not in text,
                  f"state={row['state']}, steps={[s['state'] for s in row['steps']]}, exit={row['steps'][0]['exit_code']}")
            check(f'4 {kind} skipped command has no side effect', local_or_ssh(kind, f'if [ -e {root_dir}/{row["number"]}/forbidden ]; then echo present; else echo absent; fi').strip() == 'absent')
            check(f'7 {kind} failed stdout/stderr redacted and credential mode 600',
                  secret not in text and text.count('[REDACTED]') >= 2 and '600' in text)
            rid = trigger(pipeline(host, [{'name': 'timeout', 'run': 'sleep 4', 'timeout_seconds': 1},
                                          {'name': 'skip', 'run': 'echo SHOULD_NOT_RUN'}]))['id']; all_runs.append(rid)
            row = done(rid)
            check(f'5 {kind} timeout distinct from ordinary failure', row['state'] == 'failed' and
                  [s['state'] for s in row['steps']] == ['timed_out', 'skipped'] and row['steps'][0]['exit_code'] is None,
                  f"state={row['state']}, step={row['steps'][0]['state']}, exit={row['steps'][0]['exit_code']}")
            rid = trigger(pipeline(host, [{'name': 'cancel', 'run': 'echo started; sleep 5'},
                                          {'name': 'skip', 'run': 'echo SHOULD_NOT_RUN'}]))['id']; all_runs.append(rid)
            eventually(lambda: detail(rid)['steps'][0]['state'] == 'running')
            time.sleep(.4)
            api(f'/workspaces/ops/pipeline-runs/{rid}/actions/cancel', method='POST')
            row = done(rid)
            check(f'6 {kind} running cancellation', row['state'] == 'cancelled' and
                  [s['state'] for s in row['steps']] == ['cancelled', 'skipped'],
                  f"state={row['state']}, steps={[s['state'] for s in row['steps']]}")
            # Synchronize on a following run: final cancel state can precede cleanup.
            rid = trigger(pipeline(host, [{'name': 'barrier', 'run': 'true'}]))['id']; all_runs.append(rid)
            done(rid)
            check(f'7 {kind} temporary credential directories removed after success/failure/timeout/cancel',
                  local_or_ssh(kind, 'find /tmp -maxdepth 1 -name "pipeline-credentials-*" -type d') == '')
        check('3 runner and SSH Git/build results match', all(all(marker in text for marker in [commit, 'CLONE_OK', 'BUILD_OK', 'BUILD_STDERR']) for text in successful))
        grouped = pipeline(group, [{'name': 'group {{run}}', 'run': 'echo GROUP_OK'}], target_kind='host-group')
        selected = trigger(grouped); all_runs.append(selected['id'])
        row = done(selected['id'])
        check('group chooses one online host and snapshots it', row['state'] == 'succeeded' and row['host_id'] == runner
              and row['steps'][0]['name'] == 'group ' + str(row['number']), 'selected runner; expanded step name')
        empty_group = create('host-groups', {'name': 'empty'})
        empty = pipeline(empty_group, [{'name': 'empty', 'run': 'true'}], target_kind='host-group')
        api('/workspaces/ops/pipelines/' + empty + '/actions/run', method='POST', status=400)
        check('empty group refuses execution', True, 'HTTP 400')
        pid = pipeline(runner, [{'name': 'number', 'run': 'true'}])
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            runs = list(pool.map(lambda _: trigger(pid), range(4)))
        all_runs.extend(r['id'] for r in runs)
        check('concurrent workspace sequence allocation', len({r['number'] for r in runs}) == 4,
              f"numbers={sorted(r['number'] for r in runs)}")
        for r in runs:
            done(r['id'])
        # Chunk boundary and cursor coverage use > 200 fragments, forcing pagination.
        rid = trigger(pipeline(runner, [{'name': 'large log', 'run': "python3 -c 'print(\"界\" * 300000)'"}] * 4))['id']
        all_runs.append(rid); done(rid); entries = logs(rid)
        check('large UTF-8 logs chunked and paginated without loss', len(entries) > 200 and
              ''.join(i['chunk'] for i in entries) == ('界' * 300000 + '\n') * 4, f'chunks={len(entries)}')
        other_host = create('hosts', {'name': 'other-host', 'connection': 'runner'}, 'other')
        other_repo = create('repositories', {'name': 'other-repo', 'url': repo_url}, 'other')
        definition = json.dumps({'workdir': '/tmp/test', 'steps': [{'name': 'test', 'run': 'true'}]})
        for label, target, repo in [('repository', runner, other_repo), ('host', other_host, repo_id)]:
            body = {'name': 'cross', 'definition': definition, 'target_kind': 'host', 'target_ref': target, 'repository_id': repo}
            response = api('/workspaces/ops/pipelines', body, status=400)
            check(f'8 cross-workspace {label} refused', '当前工作空间' in response['error'], 'HTTP 400')
        for suffix, method in [('', 'GET'), ('/logs?after=0', 'GET'), ('/actions/cancel', 'POST')]:
            api('/workspaces/other/pipeline-runs/' + all_runs[0] + suffix, method=method, status=404)
            check('run scope isolation ' + method + suffix, True, 'HTTP 404')
        own = pipeline(other_host, [{'name': 'other', 'run': 'true'}], ws='other', repo=other_repo)
        foreign = trigger(own, 'other')
        check('workspace numbering starts independently', foreign['number'] == 1, f'number={foreign["number"]}')
        history = api('/workspaces/ops/pipeline-runs?state=succeeded&sort=-number&page_size=2')
        check('history shared list envelope/filter/order', len(history['items']) == 2 and history['page_size'] == 2
              and history['items'][0]['number'] > history['items'][1]['number'])
        sample_stop.set(); sampler.join(10)
        check('7 process sampler executed on both real hosts', not sample_errors and
              all(len(v) >= 10 for v in samples.values()), f"samples={ {k: len(v) for k,v in samples.items()} }")
        for kind, data in samples.items():
            joined = '\n'.join(data)
            check(f'7 {kind} target ps contains no credential plaintext', secret not in joined and username not in joined
                  and '/step' in joined, f'samples={len(data)}, observed step argv={"/step" in joined}, username absent={username not in joined}, password absent={secret not in joined}')
        activity = json.dumps(api('/workspaces/ops/activity?page_size=200'))
        all_text = ''.join(i['chunk'] for rid in all_runs for i in logs(rid))
        check('7 all stored logs and activity omit credential plaintext',
              all(value not in text for value in [secret, username] for text in [activity, all_text]))
        check('server log contains no credential plaintext', secret not in (work / 'server.log').read_text())
        # A real in-flight command must finish before the owned server exits.
        # File barriers keep this deterministic instead of relying on command timing.
        for kind, host in [('runner', runner), ('ssh', ssh)]:
            marker = root_dir + '/drain-' + kind
            rid = trigger(pipeline(host, [{'name': 'drain', 'run':
                f'touch {marker}.started; while [ ! -f {marker}.release ]; do sleep 0.1; done; echo DRAIN_COMPLETED'}, {'name': 'after drain', 'run': 'echo NEXT_STEP_COMPLETED'}]))['id']
            eventually(lambda: local_or_ssh(kind, f'test -f {marker}.started && echo ready || true').strip() == 'ready')
            server.stdin.close()
            def stopped_accepting():
                try:
                    api('/status')
                    return False
                except AssertionError as error:
                    return 'HTTP 503' in str(error)
            eventually(stopped_accepting)
            check(kind + ' drain waits for active work', server.poll() is None)
            local_or_ssh(kind, f'touch {marker}.release')
            check(kind + ' graceful server exit', server.wait(timeout=45) == 0)
            server = launch(['target/debug/ferry-server', '--desktop-control'], 'server-restart-' + kind + '.log', env)
            def restarted():
                try: return api('/status')['ok']
                except OSError: return False
            eventually(restarted)
            row = detail(rid)
            check(kind + ' active run persisted as succeeded after restart', row['state'] == 'succeeded'
                  and all(marker in ''.join(i['chunk'] for i in logs(rid)) for marker in ['DRAIN_COMPLETED', 'NEXT_STEP_COMPLETED']))
        # A real read-only membership exercises permission gates, not a mock Scope.
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        api('/users', {'email': 'viewer@example.invalid', 'password': 'correct-horse-battery', 'name': 'viewer'})
        api('/workspaces/actions/join', {'code': workspace['join_code']})
        for suffix in ['', '/' + all_runs[0], '/' + all_runs[0] + '/logs?after=0']:
            api('/workspaces/ops/pipeline-runs' + suffix)
            check('read-only user can read run ' + suffix, True, 'HTTP 200')
        api('/workspaces/ops/pipelines/' + pid + '/actions/run', method='POST', status=403)
        check('trigger requires pipelines.run', True, 'HTTP 403')
        api('/workspaces/ops/pipeline-runs/' + all_runs[0] + '/actions/cancel', method='POST', status=403)
        check('cancel requires pipelines.run', True, 'HTTP 403')
        check('minimum acceptance check count', CHECKS >= 39, f'checks={CHECKS + 1}, minimum=40')
        print(f'\n{CHECKS - len(FAILURES)} passed, {len(FAILURES)} failed, 0 skipped', flush=True)
        return 1 if FAILURES else 0
    finally:
        sample_stop.set()
        if sampler:
            sampler.join(10)
        for p in reversed(processes):
            p.terminate()
        for p in reversed(processes):
            try:
                p.wait(timeout=20)
            except subprocess.TimeoutExpired:
                p.kill(); p.wait()
        for f in files:
            f.close()
        subprocess.run(['docker', 'rm', '-f', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(root_dir, ignore_errors=True)
        # Failures retain private diagnostic files; no secrets printed to console.
        if FAILURES or __import__('sys').exc_info()[0] is not None:
            print('diagnostics: ' + str(work))
        else:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    raise SystemExit(main())
