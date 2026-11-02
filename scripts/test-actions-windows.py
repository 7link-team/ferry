#!/usr/bin/env python3
"""Real Actions pipelines on a Windows host: a local server, a current-user
runner and the Windows Actions worker, all from this checkout.

Usage: python scripts/test-actions-windows.py [SERVER] [RUNNER] [WORKERS_DIR]
"""
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

assert os.name == 'nt', 'Windows only'
server_binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/ferry-server.exe').resolve())
runner_binary = str(Path(sys.argv[2] if len(sys.argv) > 2 else 'target/debug/ferry-runner.exe').resolve())
workers = str(Path(sys.argv[3] if len(sys.argv) > 3 else 'target/debug/actions-workers').resolve())
assert Path(workers, 'actions-worker-windows-amd64.exe').is_file(), 'build the Windows Actions worker first'
NO_WINDOW = 0x08000000
checks = 0


def check(label, condition, detail=''):
    global checks
    if not condition:
        raise SystemExit(f'FAIL {label} {detail}')
    checks += 1
    print('PASS', label, flush=True)


def leftovers():
    return sorted(p.name for p in Path(tempfile.gettempdir()).glob('ferry-actions-*'))


with tempfile.TemporaryDirectory(prefix='actions-windows-') as directory:
    work = Path(directory)
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env = dict(os.environ, FERRY_DATA_DIR=str(work / 'server'), FERRY_BIND=f'127.0.0.1:{port}', ACTIONS_WORKER_DIR=workers)
    env.pop('FERRY_DATABASE_URL', None)
    logs = [open(work / 'server.log', 'wb'), open(work / 'runner.log', 'wb')]
    processes = [subprocess.Popen([server_binary], env=env, stdout=logs[0], stderr=logs[0], creationflags=NO_WINDOW)]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    before = leftovers()

    def api(path, body=None, method=None, status=200):
        request = urllib.request.Request(origin + '/api' + path, data=json.dumps(body).encode() if body is not None else None,
                                         method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
        try:
            response = opener.open(request, timeout=30)
        except urllib.error.HTTPError as error:
            response = error
        value = json.loads(response.read() or b'null')
        assert status is None or response.code == status, (path, response.code, value)
        return value

    def eventually(probe, seconds=120, label='condition'):
        deadline = time.time() + seconds
        while time.time() < deadline:
            try:
                value = probe()
                if value:
                    return value
            except (OSError, AssertionError):
                pass
            time.sleep(0.5)
        raise SystemExit(f'FAIL timed out waiting for {label}; see {work}')

    try:
        eventually(lambda: api('/status')['ok'], 30, 'server')
        api('/setup', {'name': 'Windows Builder', 'email': 'builder@example.invalid', 'password': 'temporary-password',
                       'token': (work / 'server' / 'setup-token').read_text().strip()})
        workspace = api('/workspaces', {'name': 'Windows', 'slug': 'windows'})['id']
        root = f'/workspaces/{workspace}'
        host = api(root + '/nodes', {'name': 'windows-runner', 'connection': 'runner'}, status=201)['id']
        token = api(root + '/runner-tokens', {'host_id': host}, status=201)['token']
        processes.append(subprocess.Popen([runner_binary, '--server', origin, '--token', token, '--state-dir', str(work / 'runner')],
                                          stdout=logs[1], stderr=logs[1], creationflags=NO_WINDOW))
        node = eventually(lambda: (lambda row: row if row['state'] == 'online' else None)(api(f'{root}/nodes/{host}')), 60, 'runner online')
        check('the runner reports a Windows host', node['os'] == 'windows', node['os'])
        secret = 'win-secret-' + os.urandom(6).hex()
        api(root + '/variables', {'name': 'TOKEN', 'scope': 'build', 'kind': 'secret', 'value': secret}, status=201)

        def pipeline(name, jobs, image='host', on='workflow_dispatch'):
            body = {'name': name, 'definition_format': 'github-actions',
                    'definition': json.dumps({'name': name, 'on': on, 'jobs': jobs}),
                    'executor_bindings': [{'job_id': job, 'target_kind': 'host', 'target_ref': host, 'image': image} for job in jobs]}
            return api(root + '/pipelines', body, status=201)['id']

        def run(pipeline_id, body=None):
            return api(f'{root}/pipelines/{pipeline_id}/runs', body or {}, method='POST', status=201)['id']

        def finished(run_id, seconds=300):
            return eventually(lambda: (lambda row: row if row['state'] in ('succeeded', 'failed', 'cancelled') else None)(api(f'{root}/pipeline-runs/{run_id}')),
                              seconds, 'run to finish')

        def text(run_id):
            return ''.join(item['chunk'] for item in api(f'{root}/pipeline-runs/{run_id}/logs?after=0')['items'])

        jobs = {
            'produce': {'runs-on': 'self-hosted', 'outputs': {'value': '${{ steps.out.outputs.value }}'}, 'steps': [
                {'id': 'out', 'run': '"value=hello-from-windows" >> $env:GITHUB_OUTPUT; Write-Output "PWSH_OK $($PSVersionTable.PSVersion.Major)"'},
                {'run': 'Write-Output "secret:${{ secrets.TOKEN }}"'},
                {'shell': 'cmd', 'run': 'echo CMD_OK'},
            ]},
            'consume': {'runs-on': 'self-hosted', 'needs': ['produce'], 'steps': [
                {'run': 'Write-Output "GOT ${{ needs.produce.outputs.value }}"'},
            ]},
        }
        first = run(pipeline('Windows host pipeline', jobs))
        row, output = finished(first), None
        output = text(first)
        check('a run-step workflow succeeds on the Windows host', row['state'] == 'succeeded', f"{row['state']} {output[-2000:]}")
        check('run steps use PowerShell by default and shell: cmd works', 'PWSH_OK' in output and 'CMD_OK' in output, output[-2000:])
        check('job outputs pass between jobs through GITHUB_OUTPUT', 'GOT hello-from-windows' in output, output[-2000:])
        check('the secret is masked in logs and run data', secret not in output and secret not in json.dumps(row) and 'secret:***' in output, output[-1500:])
        check('the run records the Windows host by name', row['host_name'] == 'windows-runner')

        rejected = run(pipeline('Container on Windows', {'box': {'runs-on': 'container', 'steps': [{'run': 'echo no'}]}}, image='alpine:3.20'))
        row = finished(rejected)
        check('container jobs are refused on a Windows host with a clear reason', row['state'] == 'failed' and '不支持容器镜像' in text(rejected), text(rejected)[-800:])

        long = run(pipeline('Cancelled on Windows', {'hold': {'runs-on': 'self-hosted', 'steps': [{'run': 'Write-Output HOLDING; Start-Sleep -Seconds 300'}]}}))
        eventually(lambda: 'HOLDING' in text(long), 120, 'long step to start')
        api(f'{root}/pipeline-runs/{long}/cancellations', {}, method='POST')
        row = finished(long, 120)
        check('a running Windows job can be cancelled', row['state'] == 'cancelled', row['state'])

        cleanup = run(pipeline('Cleanup after cancel', {
            'hold': {'runs-on': 'self-hosted', 'steps': [{'run': 'Write-Output HOLDING; Start-Sleep -Seconds 300'}]},
            'always': {'runs-on': 'self-hosted', 'needs': ['hold'], 'if': '${{ always() }}', 'steps': [{'run': 'Write-Output "CLEANUP ${{ needs.hold.result }}"'}]},
            'normal': {'runs-on': 'self-hosted', 'needs': ['hold'], 'steps': [{'run': 'Write-Output SHOULD_NOT_RUN'}]},
        }))
        eventually(lambda: 'HOLDING' in text(cleanup), 120, 'held job to start')
        api(f'{root}/pipeline-runs/{cleanup}/cancellations', {}, method='POST')
        row = finished(cleanup, 180)
        states = {job['key']: job['state'] for job in row['jobs']}
        output = text(cleanup)
        check('a cancelled run still runs its if: always() jobs, not the others', row['state'] == 'cancelled' and 'CLEANUP cancelled' in output
              and 'SHOULD_NOT_RUN' not in output and states.get('always') == 'succeeded' and states.get('normal') == 'skipped', f'{states} {output[-1200:]}')

        tolerated = run(pipeline('Tolerated failure', {
            'flaky': {'runs-on': 'self-hosted', 'continue-on-error': True, 'steps': [{'run': 'exit 3'}]},
            'after': {'runs-on': 'self-hosted', 'needs': ['flaky'], 'steps': [{'run': 'Write-Output "AFTER ${{ needs.flaky.result }}"'}]},
        }))
        row = finished(tolerated)
        check('job continue-on-error lets dependants and the run succeed', row['state'] == 'succeeded' and 'AFTER success' in text(tolerated), f"{row['state']} {text(tolerated)[-800:]}")

        dispatch = pipeline('Dispatch inputs', {'show': {'runs-on': 'self-hosted', 'steps': [
            {'run': 'Write-Output "FLAVOR ${{ inputs.flavor }} EVENT ${{ github.event.inputs.flavor }} DEBUG ${{ inputs.debug }}"'}]}},
            on={'workflow_dispatch': {'inputs': {'flavor': {'type': 'choice', 'options': ['lite', 'full'], 'default': 'lite'}, 'debug': {'type': 'boolean'}}}})
        api(f'{root}/pipelines/{dispatch}/runs', {'inputs': {'flavor': 'other'}}, method='POST', status=400)
        given = run(dispatch, {'inputs': {'flavor': 'full', 'debug': True}})
        row = finished(given)
        check('workflow_dispatch inputs are validated, typed and delivered', row['state'] == 'succeeded' and 'FLAVOR full EVENT full DEBUG true' in text(given), text(given)[-800:])

        api(root + '/pipeline-environments', {'name': 'production', 'target_kind': 'host', 'target_ref': host, 'requires_approval': True}, status=201)
        gated = run(pipeline('Protected environment', {'ship': {'runs-on': 'self-hosted', 'environment': {'name': 'production', 'url': 'https://example.invalid/app'},
                                                                  'steps': [{'run': 'Write-Output SHIPPED'}]}}))
        waiting = eventually(lambda: (lambda row: row if any(job['state'] == 'waiting' for job in row['jobs']) else None)(api(f'{root}/pipeline-runs/{gated}')), 60, 'approval gate')
        check('a job of a protected environment waits for approval before running', 'SHIPPED' not in text(gated)
              and waiting['environments']['ship']['url'] == 'https://example.invalid/app', json.dumps(waiting.get('environments')))
        api(f'{root}/pipeline-runs/{gated}/jobs/ship/approvals', {}, method='POST')
        row = finished(gated)
        check('the approved job runs', row['state'] == 'succeeded' and 'SHIPPED' in text(gated), text(gated)[-800:])

        eventually(lambda: leftovers() == before, 60, 'temporary directories to be removed')
        check('private job directories are removed afterwards', leftovers() == before, str(leftovers()))
        print(f'PASS: {checks} Windows Actions checks')
    finally:
        for process in reversed(processes):
            process.terminate()
            try:
                process.wait(20)
            except subprocess.TimeoutExpired:
                process.kill()
        for log in logs:
            log.close()
