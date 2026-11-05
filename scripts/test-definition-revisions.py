#!/usr/bin/env python3
"""Real HTTP, runner and restart acceptance for immutable Actions revisions."""
import http.cookiejar
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

server_binary = str(Path(sys.argv[1]).resolve())
runner_binary = str(Path(sys.argv[2]).resolve())
workers = str(Path(sys.argv[3]).resolve())
work = Path(tempfile.mkdtemp(prefix='definition-revisions-'))
processes, logs = [], []
checks = []
successful = False
with socket.socket() as probe:
    probe.bind(('127.0.0.1', 0))
    port = probe.getsockname()[1]
origin = f'http://127.0.0.1:{port}'
env = dict(os.environ, FERRY_DATA_DIR=str(work / 'server'),
           FERRY_BIND=f'127.0.0.1:{port}', ACTIONS_WORKER_DIR=workers)
for key in ('FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE'):
    env.pop(key, None)
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))


def launch(binary, args, name):
    log = open(work / (name + '.log'), 'ab')
    logs.append(log)
    child = subprocess.Popen([binary, *args], env=env, stdout=log, stderr=log,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    processes.append(child)
    return child


def api(path, body=None, method=None, status=200, client=opener):
    request = urllib.request.Request(origin + '/api' + path,
        data=json.dumps(body).encode() if body is not None else None,
        method=method, headers={'Content-Type': 'application/json',
            'X-Ferry-Request': '1', 'Accept-Language': 'en-US'})
    try:
        response = client.open(request, timeout=30)
    except urllib.error.HTTPError as error:
        response = error
    raw = response.read()
    assert response.code == status, (path, response.code, status, raw.decode(errors='replace'))
    return json.loads(raw) if raw else None


def eventually(check, label, seconds=90):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, urllib.error.URLError) as error:
            last = error
        time.sleep(0.1)
    raise AssertionError(f'Timeout: {label}: {last}')


def passed(label):
    checks.append(label)
    print('PASS', label, flush=True)


try:
    server = launch(server_binary, [], 'server')
    eventually(lambda: api('/status')['ok'], 'server startup', 30)
    api('/setup', {'name': 'Revision operator', 'email': 'revision@example.invalid',
        'password': 'temporary-revision-password',
        'token': (work / 'server' / 'setup-token').read_text().strip()})
    workspace = api('/workspaces', {'name': 'Revisions', 'slug': 'revisions'})['id']
    root = f'/workspaces/{workspace}'
    host = api(root + '/nodes', {'name': 'Revision runner', 'connection': 'runner'}, status=201)['id']
    token = api(root + '/runner-tokens', {'host_id': host}, status=201)['token']
    launch(runner_binary, ['--server', origin, '--token', token,
        '--state-dir', str(work / 'runner')], 'runner')
    eventually(lambda: api(root + '/nodes/' + host)['state'] == 'online', 'runner online', 60)
    windows = os.name == 'nt'
    def workflow(marker):
        command = 'Write-Output ' + marker if windows else 'printf "' + marker + '\\n"'
        return 'on: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: ' + command + '\n'
    original = workflow('REVISION_A')
    body = {'name': 'Revision history', 'definition_format': 'github-actions',
        'definition': original, 'message': '版' * 200,
        'executor_bindings': [{'job_id': 'build', 'target_kind': 'host', 'target_ref': host, 'image': 'host'}]}
    pipeline = api(root + '/pipelines', body, status=201)['id']
    path = root + '/pipelines/' + pipeline
    def history():
        return api(path + '/revisions')
    first = history()['items'][0]
    assert first['number'] == 1 and first['created_by'] != '0' and first['message'] == body['message']
    assert json.loads(first['definition'])['workflow'] == original
    passed('creating an Actions definition records its exact YAML, author and 200-character message')
    before = api(path)
    api(path, dict(before, name='Metadata changed'), method='PATCH')
    assert history()['total'] == 1 and history()['items'][0] == first
    passed('metadata-only saves do not create or rewrite revisions')
    api(path, before, method='PATCH', status=409)
    current = api(path)
    api(path, dict(current, definition='jobs: []'), method='PATCH', status=400)
    api(path, dict(current, message='版' * 201), method='PATCH', status=400)
    assert history()['total'] == 1 and api(path)['version'] == current['version']
    passed('stale, invalid and oversized saves roll back without changing definition or history')
    commented = '# preserve this comment\n' + original
    api(path, dict(current, definition=commented, message='comment change'), method='PATCH')
    second = history()['items'][0]
    assert second['number'] == 2 and json.loads(second['definition'])['workflow'] == commented
    passed('YAML comment changes create a new immutable revision')
    first_run = api(path + '/runs', {}, method='POST', status=201)['id']
    assert api(root + '/pipeline-runs/' + first_run)['revision_id'] == second['id']
    api(path, dict(api(path), definition=workflow('REVISION_B')), method='PATCH')
    third = history()['items'][0]
    second_run = api(path + '/runs', {}, method='POST', status=201)['id']
    def finished(run):
        row = api(root + '/pipeline-runs/' + run)
        return row if row['state'] in ('succeeded', 'failed', 'cancelled') else None
    one = eventually(lambda: finished(first_run), 'first run')
    two = eventually(lambda: finished(second_run), 'second run')
    def output(run):
        return ''.join(row['chunk'] for row in api(root + '/pipeline-runs/' + run + '/logs?after=0')['items'])
    assert one['state'] == two['state'] == 'succeeded', (one['state'], two['state'])
    assert one['revision_id'] == second['id'] and two['revision_id'] == third['id']
    assert 'REVISION_A' in output(first_run) and 'REVISION_B' not in output(first_run)
    assert 'REVISION_B' in output(second_run)
    passed('real runs freeze their revision and execute the original snapshot across concurrent edits')
    restored = json.loads(first['definition'])
    api(path, dict(api(path), definition=restored['workflow'],
        executor_bindings=restored['executor_bindings'], checkout_bindings=restored['checkout_bindings'],
        graph_layout=restored['graph_layout'], message='restore first'), method='PATCH')
    fourth = history()['items'][0]
    assert fourth['number'] == 4 and fourth['id'] != first['id']
    assert json.loads(fourth['definition']) == restored
    assert api(path + '/revisions/' + first['id']) == first
    page = api(path + '/revisions?page_size=1&sort=number')
    assert page['total'] == 4 and page['page_size'] == 1 and page['items'] == [first]
    passed('restoring through normal save creates a new revision and history uses the shared list envelope')
    other = api('/workspaces', {'name': 'Other', 'slug': 'other'})['id']
    api(f'/workspaces/{other}/pipelines/{pipeline}/revisions', status=404)
    api(f'/workspaces/{other}/pipelines/{pipeline}/revisions/{first["id"]}', status=404)
    api(path + '/revisions?workspace_id=' + other, status=400)
    guest = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    api(path + '/revisions', status=401, client=guest)
    passed('history rejects unauthenticated requests, cross-workspace reads and scope override filters')
    server.terminate()
    server.wait(timeout=15)
    server = launch(server_binary, [], 'server-restarted')
    eventually(lambda: api('/status')['ok'], 'server restart', 30)
    assert history()['total'] == 4 and api(path + '/revisions/' + first['id']) == first
    assert api(root + '/pipeline-runs/' + first_run)['revision_id'] == second['id']
    assert api(root + '/pipeline-runs/' + second_run)['revision_id'] == third['id']
    passed('revision history and execution provenance survive a real server restart')
    (work / 'results.json').write_text(json.dumps({'checks': checks, 'result': 'PASS'}, indent=2), encoding='utf-8')
    successful = True
    print(f'PASS: {len(checks)} definition revision acceptance checks', flush=True)
finally:
    for child in reversed(processes):
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=15)
    for log in logs:
        log.close()
    if successful and os.environ.get('KEEP_REVISION_TEST_DATA') != '1':
        shutil.rmtree(work)
    else:
        print('Revision acceptance evidence:', work, flush=True)
