#!/usr/bin/env python3
"""Real Actions pipelines on a Windows host: a local server, a current-user
runner and the Windows Actions worker, all from this checkout.

Usage: python scripts/test-actions-windows.py [SERVER] [RUNNER] [WORKERS_DIR]
"""
import contextlib
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
        print('SERVER LOG', (work / 'server.log').read_text(encoding='utf-8', errors='replace')[-4000:], flush=True)
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
                                         method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN'})
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
        if os.environ.get('FERRY_TEST_EXECUTION_CAPACITY_ONLY') == '1':
            details = {'label': label, 'server_log': (work / 'server.log').read_text(encoding='utf-8', errors='replace'),
                       'runner_log': (work / 'runner.log').read_text(encoding='utf-8', errors='replace'), 'runs': []}
            for run_id in capacity_runs:
                try:
                    details['runs'].append({'run': api(f'{root}/pipeline-runs/{run_id}'), 'logs': api(f'{root}/pipeline-runs/{run_id}/logs?after=0')})
                except Exception as error:
                    details['runs'].append({'id': run_id, 'error': str(error)})
            evidence = Path('data/maintenance') / f'alpha40-capacity-failure-{int(time.time() * 1000)}.json'
            evidence.write_text(json.dumps(details, ensure_ascii=False, indent=2), encoding='utf-8')
            print('FAIL EVIDENCE', evidence, flush=True)
        raise SystemExit(f'FAIL timed out waiting for {label}; see {work}')

    capacity_runs = []
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
            run_id = api(f'{root}/pipelines/{pipeline_id}/runs', body or {}, method='POST', status=201)['id']
            if os.environ.get('FERRY_TEST_EXECUTION_CAPACITY_ONLY') == '1':
                capacity_runs.append(run_id)
            return run_id

        def finished(run_id, seconds=300):
            return eventually(lambda: (lambda row: row if row['state'] in ('succeeded', 'failed', 'cancelled') else None)(api(f'{root}/pipeline-runs/{run_id}')),
                              seconds, 'run to finish')

        def text(run_id):
            return ''.join(item['chunk'] for item in api(f'{root}/pipeline-runs/{run_id}/logs?after=0')['items'])

        echo = run(pipeline('Minimal Windows echo', {
            'build': {'name': 'Task 1', 'runs-on': 'self-hosted', 'timeout-minutes': 30,
                      'steps': [{'name': 'Step 1', 'run': 'echo hello'}]},
        }))
        echo_result = finished(echo)
        echo_output = text(echo)
        check('the minimal echo hello workflow succeeds on Windows',
              echo_result['state'] == 'succeeded' and 'hello' in echo_output,
              f"{echo_result['state']} {echo_output}")
        if os.environ.get('FERRY_TEST_EXECUTION_CAPACITY_ONLY') == '1':
            def policy(enabled=True, shared=False, capacity=1):
                current = api(f'{root}/nodes/{host}')
                api(f'{root}/nodes/{host}', dict(current, execution={'enabled': enabled, 'shared': shared, 'capacity': capacity}), method='PATCH')
                return api(f'{root}/nodes/{host}')

            def occupancy():
                return api(f'{root}/nodes/{host}')['execution_status']

            def cancel(run_id):
                api(f'{root}/pipeline-runs/{run_id}/cancellations', {}, method='POST')
                return finished(run_id, 120)

            holder = pipeline('Capacity real holder', {'hold': {'runs-on': 'self-hosted', 'steps': [{'run': 'Write-Output CAPACITY_HOLDER; Start-Sleep -Seconds 180'}]}})
            if os.environ.get('FERRY_TEST_EXECUTION_POOL_ONLY') != '1':
                check('existing hosts default to private one-slot execution', node['execution'] == {'enabled': True, 'shared': False, 'capacity': 1})
                first_hold = run(holder)
                eventually(lambda: 'CAPACITY_HOLDER' in text(first_hold), 120, 'first real holder')
                second_hold = run(holder)
                eventually(lambda: api(f'{root}/pipeline-runs/{second_hold}')['state'] == 'running', 30, 'second run admitted to scheduler')
                time.sleep(2)
                check('capacity one admits one real worker and queues the next run', occupancy()['active'] == 1 and 'CAPACITY_HOLDER' not in text(second_hold))
                check('cancelling a capacity waiter finishes without starting it', cancel(second_hold)['state'] == 'cancelled' and 'CAPACITY_HOLDER' not in text(second_hold))
                draining = policy(enabled=False)
                check('disabling execution drains the active worker', draining['execution_status']['state'] == 'draining' and draining['execution_status']['active'] == 1)
                disabled_waiter = run(holder)
                time.sleep(2)
                check('disabled host does not start another worker', 'CAPACITY_HOLDER' not in text(disabled_waiter) and occupancy()['active'] == 1)
                cancel(disabled_waiter)
                cancel(first_hold)
                eventually(lambda: occupancy()['active'] == 0, 60, 'cancel cleanup releases capacity')
                check('drained disabled host reports disabled', occupancy()['state'] == 'disabled')
            policy(enabled=True, shared=True, capacity=2)
            pool_body = {'name': 'Shared pool real workers', 'definition_format': 'github-actions',
                         'definition': json.dumps({'on': 'workflow_dispatch', 'jobs': {
                             job: {'runs-on': ['self-hosted', 'windows', 'x64'], 'steps': [{'run': f'Write-Output POOL_{job}; Start-Sleep -Seconds 180'}]} for job in ['one', 'two']}}),
                         'executor_bindings': [{'job_id': job, 'target_kind': 'pool', 'target_ref': '', 'image': 'host'} for job in ['one', 'two']]}
            pooled = api(root + '/pipelines', pool_body, status=201)['id']
            pooled_run = run(pooled)
            eventually(lambda: 'POOL_one' in text(pooled_run) and 'POOL_two' in text(pooled_run), 120, 'two real pool workers')
            status = occupancy()
            check('shared pool places two workers and exposes ownership', status['active'] == 2 and status['state'] == 'busy' and {job['job_key'] for job in status['running_jobs']} == {'one', 'two'})
            policy(enabled=True, shared=True, capacity=1)
            third_hold = run(holder)
            time.sleep(2)
            check('reducing capacity drains excess workers before replacement', occupancy()['active'] == 2 and 'CAPACITY_HOLDER' not in text(third_hold))
            cancel(third_hold)
            cancel(pooled_run)
            eventually(lambda: occupancy()['active'] == 0, 60, 'pool cleanup releases every slot')
            check('cancelled pool workers release slots after cleanup', occupancy()['state'] == 'available')
            # Force loss of this fixture's durable lease while a real child is
            # running. Renewal must cancel and await cleanup before failure.
            lost_lease_run = run(holder)
            eventually(lambda: 'CAPACITY_HOLDER' in text(lost_lease_run), 120, 'lease-loss real holder')
            import sqlite3
            with contextlib.closing(sqlite3.connect(work / 'server' / 'ferry.sqlite', timeout=30)) as fixture_db:
                fixture_db.execute('UPDATE host_execution_slots SET expires_at = 0 WHERE run_id = ?', (int(lost_lease_run),))
                fixture_db.commit()
            lost_result = finished(lost_lease_run, 120)
            check('lease renewal loss cancels the real worker and awaits terminal cleanup', lost_result['state'] == 'failed' and occupancy()['active'] == 0)
            eventually(lambda: leftovers() == before, 60, 'lease-loss private directory cleanup')
            recovered_run = run(pipeline('Capacity recovery', {'recover': {'runs-on': 'self-hosted', 'steps': [{'run': 'Write-Output CAPACITY_RECOVERED'}]}}))
            check('a worker can start after lease-loss cleanup', finished(recovered_run)['state'] == 'succeeded' and 'CAPACITY_RECOVERED' in text(recovered_run))
            matrix_active = str(work / 'matrix-active').replace("'", "''")
            matrix_command = ("$dir='" + matrix_active + "'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; "
                              "$marker=Join-Path $dir '${{ matrix.index }}'; Set-Content -Path $marker -Value active; "
                              "try { if ((Get-ChildItem -File $dir).Count -gt 1) { throw 'HOST_CAPACITY_OVERBOOKED' }; "
                              "Write-Output 'MATRIX_${{ matrix.index }}'; Start-Sleep -Seconds 2 } finally { Remove-Item -LiteralPath $marker -ErrorAction SilentlyContinue }")
            matrix_run = run(pipeline('Matrix legs obey host capacity', {'matrix': {
                'runs-on': 'self-hosted', 'strategy': {'matrix': {'index': [1, 2, 3]}, 'max-parallel': 3, 'fail-fast': False},
                'steps': [{'run': matrix_command}],
            }}))
            matrix_peak = 0
            matrix_deadline = time.time() + 180
            while time.time() < matrix_deadline:
                matrix_peak = max(matrix_peak, occupancy()['active'])
                matrix_result = api(f'{root}/pipeline-runs/{matrix_run}')
                if matrix_result['state'] in ('succeeded', 'failed', 'cancelled'):
                    break
                time.sleep(0.25)
            else:
                raise AssertionError('matrix legs did not finish')
            matrix_text = text(matrix_run)
            check('real matrix legs run individually under one-slot capacity', matrix_result['state'] == 'succeeded' and matrix_peak == 1 and all(f'MATRIX_{index}' in matrix_text for index in [1, 2, 3]), matrix_text[-2000:])
            print(f'PASS: {checks} real Windows execution capacity checks')
            raise SystemExit(0)

        if os.environ.get('FERRY_TEST_ACTIONS_ECHO_ONLY') == '1':
            raise SystemExit(0)

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
        check('a run-step workflow succeeds on the Windows host', row['state'] == 'succeeded', f"{row['state']} {output}")
        check('run steps use PowerShell by default and shell: cmd works', 'PWSH_OK' in output and 'CMD_OK' in output, output[-2000:])
        check('job outputs pass between jobs through GITHUB_OUTPUT', 'GOT hello-from-windows' in output, output[-2000:])
        check('the secret is masked in logs and run data', secret not in output and secret not in json.dumps(row) and 'secret:***' in output, output[-1500:])
        check('the run records the Windows host by name', row['host_name'] == 'windows-runner')

        rejected = run(pipeline('Container on Windows', {'box': {'runs-on': 'container', 'steps': [{'run': 'echo no'}]}}, image='alpine:3.20'))
        row = finished(rejected)
        check('container jobs are refused on a Windows host with a clear reason', row['state'] == 'failed' and '不支持容器镜像' in text(rejected), text(rejected)[-800:])

        legacy_definition = {
            'workdir': str(work / 'persistent-legacy').replace('\\', '/'),
            'env': [
                {'name': 'LEGACY_RUN', 'value': 'context-{{run}}'},
                {'name': 'TOKEN', 'valueFrom': 'variable/TOKEN'},
            ],
            'steps': [{'name': 'Legacy shell {{run}}', 'timeout_seconds': 61,
                       'run': 'printf "LEGACY_RUN=%s\\n" "$LEGACY_RUN"; '
                              'printf "LEGACY_TOKEN=%s\\n" "$TOKEN"; '
                              'if [ -f marker ]; then echo PERSISTED; else echo FIRST; fi; '
                              'printf marker > marker; false; echo LEGACY_SHELL_OK'}],
        }
        # Model the existing user-selected checkout directory retained during
        # conversion; mkdir also avoids sandbox-denied ancestor traversal in sh.
        Path(legacy_definition['workdir']).mkdir()
        legacy = api(root + '/pipelines', {
            'name': 'Converted legacy Windows', 'target_kind': 'host', 'target_ref': host,
            'definition': json.dumps(legacy_definition),
        }, status=201)['id']
        stored = api(f'{root}/pipelines/{legacy}')
        check('legacy API writes return the Actions editor format', stored['definition_format'] == 'github-actions')
        legacy_first = run(legacy)
        first_row = finished(legacy_first)
        first_output = text(legacy_first)
        check('converted legacy shell and runtime environment work on Windows',
              first_row['state'] == 'succeeded' and 'LEGACY_SHELL_OK' in first_output
              and f"LEGACY_RUN=context-{first_row['number']}" in first_output,
              f"{first_row} {first_output}")
        check('step names show their expressions evaluated',
              any(step['name'].endswith(f"Legacy shell {first_row['number']}") for step in first_row['steps']),
              str([step['name'] for step in first_row['steps']]))
        check('converted legacy variable references stay masked',
              secret not in first_output and 'LEGACY_TOKEN=***' in first_output, first_output[-1000:])
        legacy_second = run(legacy)
        second_row = finished(legacy_second)
        second_output = text(legacy_second)
        check('converted legacy workdirs persist across Windows runs',
              second_row['state'] == 'succeeded' and 'FIRST' in first_output and 'PERSISTED' in second_output,
              f"{second_row['state']} {second_output[-2000:]}")

        # Declared artifacts become upload-artifact steps, which the worker runs
        # itself: no Node.js is needed on the host (CI also runs this without it).
        archived = api(root + '/pipelines', {
            'name': 'Converted legacy artifact', 'target_kind': 'host', 'target_ref': host,
            'definition': json.dumps({'workdir': legacy_definition['workdir'], 'artifacts': ['dist/app.txt'],
                                      'steps': [{'name': 'Build', 'run': 'mkdir -p dist; echo built > dist/app.txt'}]}),
        }, status=201)['id']
        archived_run = run(archived)
        archived_row = finished(archived_run)
        check('converted legacy artifacts publish without Node.js on the host',
              archived_row['state'] == 'succeeded' and len(archived_row['artifacts']) == 1,
              f"{archived_row['state']} {archived_row['artifacts']} {text(archived_run)[-1500:]}")

        exits = api(root + '/pipelines', {
            'name': 'Converted legacy exact exit', 'target_kind': 'host', 'target_ref': host,
            'definition': json.dumps({'workdir': legacy_definition['workdir'],
                                      'steps': [{'name': 'Exit seven', 'run': 'echo EXACT_EXIT; exit 7'}]}),
        }, status=201)['id']
        exit_run = run(exits)
        exit_row = finished(exit_run)
        check('converted legacy steps retain their exact exit codes',
              exit_row['state'] == 'failed'
              and any(step['exit_code'] == 7 and 'Exit seven' in step['name'] for step in exit_row['steps']),
              f"{exit_row['state']} {exit_row['steps']} {text(exit_run)[-1500:]}")

        repository_secret = 'git-secret-' + os.urandom(6).hex()
        repository_username = 'git-user-' + os.urandom(6).hex()
        credential = api(root + '/credentials', {
            'name': 'Legacy git credentials', 'kind': 'password', 'scope': 'build',
            'username': repository_username, 'secret': repository_secret,
        }, status=201)['id']
        repository_url = 'https://example.invalid/team/repo.git'
        repository = api(root + '/repositories', {
            'name': 'Legacy credential repository', 'url': repository_url,
            'default_branch': 'main', 'credential_id': credential,
        }, status=201)['id']
        credential_pipeline = api(root + '/pipelines', {
            'name': 'Legacy authenticated git without checkout', 'target_kind': 'host',
            'target_ref': host, 'repository_id': repository,
            'definition': json.dumps({'workdir': legacy_definition['workdir'], 'steps': [{
                'name': 'Read git credential',
                'run': 'printf "protocol=https\\nhost=example.invalid\\n\\n" | git credential fill; '
                       'echo REPOSITORY={{repo_url}}',
            }]}),
        }, status=201)['id']
        credential_run = run(credential_pipeline)
        credential_row = finished(credential_run)
        credential_output = text(credential_run)
        check('legacy git credentials work without an implicit repository checkout',
              credential_row['state'] == 'succeeded'
              and 'username=[REDACTED]' in credential_output and 'password=[REDACTED]' in credential_output,
              f"{credential_row['state']} {credential_output}")
        check('repository credentials remain masked and the repository URL stays credential-free',
              repository_username not in credential_output and repository_secret not in credential_output
              and f'REPOSITORY={repository_url}' in credential_output,
              credential_output)

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
        # A failed capacity assertion must still terminate remote child processes
        # through the product cancellation path before stopping its runner.
        for run_id in capacity_runs:
            try:
                current = api(f'{root}/pipeline-runs/{run_id}')
                if current['state'] not in ('succeeded', 'failed', 'cancelled'):
                    api(f'{root}/pipeline-runs/{run_id}/cancellations', {}, method='POST', status=None)
                    finished(run_id, 120)
            except (OSError, AssertionError, SystemExit):
                pass
        for process in reversed(processes):
            process.terminate()
            try:
                process.wait(20)
            except subprocess.TimeoutExpired:
                process.kill()
        for log in logs:
            log.close()
