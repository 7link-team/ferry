#!/usr/bin/env python3
"""Real isolated HTTP acceptance for Workflow drafts, history and graph checks."""
import concurrent.futures
import copy
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

binary = Path(sys.argv[1]).resolve()
database_binary = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
postgres = database_binary if database_binary and database_binary.name.lower() == 'postgres.exe' else None
mysqld = database_binary if database_binary and postgres is None else None
work = Path(tempfile.mkdtemp(prefix='workflow-drafts-'))
children, logs, checks = [], [], []
with socket.socket() as probe:
    probe.bind(('127.0.0.1', 0))
    port = probe.getsockname()[1]
origin = f'http://127.0.0.1:{port}'
env = dict(os.environ, FERRY_DATA_DIR=str(work / 'server'), FERRY_BIND=f'127.0.0.1:{port}',
    FERRY_ARTIFACT_STORE='local')
for key in ('FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE'):
    env.pop(key, None)


def client():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}),
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))


owner = client()


def api(path, body=None, method=None, status=200, actor=owner, locale='en-US'):
    request = urllib.request.Request(origin + '/api' + path,
        data=json.dumps(body, ensure_ascii=False).encode() if body is not None else None,
        method=method, headers={'Content-Type': 'application/json',
            'X-Ferry-Request': '1', 'Accept-Language': locale})
    try:
        response = actor.open(request, timeout=30)
    except urllib.error.HTTPError as error:
        response = error
    raw = response.read()
    accepted = status if isinstance(status, tuple) else (status,)
    assert response.code in accepted, (path, response.code, status, raw.decode(errors='replace'))
    value = json.loads(raw) if raw else None
    return (response.code, value) if isinstance(status, tuple) else value


def launch(name):
    log = open(work / (name + '.log'), 'ab')
    logs.append(log)
    child = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    children.append(child)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        assert child.poll() is None, ('server exited', child.returncode)
        try:
            if api('/status')['ok']:
                return child
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise AssertionError('server startup timeout')


def passed(label):
    checks.append(label)
    print('PASS', label, flush=True)


def graph(value=42):
    return {'engine': 'ferry-graph', 'schema': 1, 'nodes': [
        {'id': 'start', 'name': 'Start', 'type': 'core.trigger.manual', 'type_version': 1,
            'params': {'input_schema': {'type': 'object'}}, 'position': [0, 0]},
        {'id': 'set', 'name': 'Set', 'type': 'data.set', 'type_version': 1,
            'params': {'fields': [{'path': 'answer', 'value': value}], 'keep_other_fields': True}, 'position': [200, 0]}],
        'connections': [{'from': {'node': 'start', 'port': 'main', 'index': 0},
            'to': {'node': 'set', 'port': 'main', 'index': 0}}]}


def start_mysql():
    global mysql_port
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        mysql_port = probe.getsockname()[1]
    mysql_data = work / 'mysql'
    mysql_data.mkdir()
    log = open(work / 'mysql.log', 'ab')
    logs.append(log)
    base = ['--no-defaults', '--basedir=' + str(mysqld.parent.parent), '--datadir=' + str(mysql_data)]
    initialized = subprocess.run([str(mysqld), *base, '--initialize-insecure'],
        stdout=log, stderr=log, timeout=60,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    assert initialized.returncode == 0, 'isolated MySQL initialization failed; see mysql.log'
    child = subprocess.Popen([str(mysqld), *base, f'--port={mysql_port}',
        '--bind-address=127.0.0.1', '--mysqlx=OFF', '--skip-log-bin',
        '--log-error=' + str(work / 'mysql-error.log')], stdout=log, stderr=log,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    children.append(child)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        assert child.poll() is None, 'isolated MySQL exited; see mysql-error.log'
        result = subprocess.run([str(mysqld.with_name('mysql.exe')), '--no-defaults',
            '--protocol=TCP', '--host=127.0.0.1', f'--port={mysql_port}', '--user=root',
            '--execute=CREATE DATABASE workflow_drafts CHARACTER SET utf8mb4 COLLATE utf8mb4_bin'],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        if result.returncode == 0:
            env['FERRY_DATABASE_URL'] = f'mysql://root@127.0.0.1:{mysql_port}/workflow_drafts'
            return
        time.sleep(0.1)
    raise AssertionError('isolated MySQL startup timeout')


def pg_command(program, *arguments):
    return subprocess.run([str(postgres.with_name(program)),*map(str,arguments)],
        capture_output=True,timeout=60,
        env=dict(os.environ,PGCLIENTENCODING='UTF8'),
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)


def pg_sql(statement, database='workflow_drafts'):
    result = pg_command('psql.exe','--no-psqlrc','--host=127.0.0.1',f'--port={postgres_port}',
        '--username=fixture','--dbname='+database,'--set=ON_ERROR_STOP=1',
        '--no-align','--tuples-only','--field-separator=\t','--command='+statement)
    assert result.returncode == 0,result.stderr.decode(errors='replace')
    return result.stdout.decode('utf-8')


def start_postgres():
    global postgres_port
    with socket.socket() as probe:
        probe.bind(('127.0.0.1',0))
        postgres_port = probe.getsockname()[1]
    result = pg_command('initdb.exe','-D',work/'postgres','--username=fixture',
        '--auth=trust','--encoding=UTF8','--locale=C')
    (work/'postgres-init.log').write_bytes(result.stdout + result.stderr)
    assert result.returncode == 0,'isolated PostgreSQL initialization failed; see postgres-init.log'
    log = open(work/'postgres.log','ab')
    logs.append(log)
    child = subprocess.Popen([str(postgres),'-D',str(work/'postgres'),'-p',str(postgres_port),
        '-h','127.0.0.1'],stdout=log,stderr=log,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    children.append(child)
    deadline = time.monotonic()+30
    while time.monotonic() < deadline:
        assert child.poll() is None,('isolated PostgreSQL exited',child.returncode)
        try:
            with socket.create_connection(('127.0.0.1',postgres_port),timeout=.3):
                break
        except OSError:
            time.sleep(.1)
    else:
        raise AssertionError('isolated PostgreSQL startup timeout')
    pg_sql('CREATE DATABASE workflow_drafts','postgres')
    env['FERRY_DATABASE_URL'] = f'postgres://fixture@127.0.0.1:{postgres_port}/workflow_drafts'


try:
    if mysqld:
        start_mysql()
    elif postgres:
        start_postgres()
    server = launch('server')
    api('/setup', {'name': 'Draft operator', 'email': 'draft@example.invalid',
        'password': 'temporary-draft-password',
        'token': (work / 'server' / 'setup-token').read_text().strip()})
    workspace = api('/workspaces', {'name': 'Workflow drafts', 'slug': 'workflow-drafts'})['id']
    root = f'/workspaces/{workspace}'
    group = api(root + '/workflow-groups', {'name': 'Workflow group'}, status=201)['id']
    pipeline_group = api(root + '/pipeline-groups', {'name': 'Pipeline group'}, status=201)['id']
    assert [row['id'] for row in api(root + '/workflow-groups')['items']] == [group]
    assert [row['id'] for row in api(root + '/pipeline-groups')['items']] == [pipeline_group]
    original = graph('PRIVATE_GRAPH_PAYLOAD_初稿')
    body = {'name': ' Draft ', 'group_id': group, 'definition': json.dumps(original),
        'message': '初版', 'owner_id': '0', 'published_revision_id': '999', 'trigger_run_as': '999'}
    created = api(root + '/workflows', body, status=201)
    workflow = created['id']
    path = root + '/workflows/' + workflow
    first = api(path + '/revisions')['items'][0]
    assert created['name'] == 'Draft' and created['owner_id'] != '0'
    assert created['owner_id'] == created['created_by'] and created['published_revision_id'] is None and created['trigger_run_as'] is None
    assert created['draft_revision_id'] == first['id'] and json.loads(first['definition']) == original
    passed('create records a real owner and immutable draft without accepting forged publication authority')
    api(path, {'version': created['version'], 'name': 'Metadata only'}, method='PATCH')
    assert api(path + '/revisions')['total'] == 1 and api(path + '/revisions')['items'][0] == first
    current = api(path)
    api(path, {'version': current['version'], 'definition': json.dumps(original, indent=2)}, method='PATCH')
    assert api(path + '/revisions')['total'] == 1
    passed('metadata and JSON formatting changes do not append or mutate revisions')
    current = api(path)
    api(path, {'version': current['version'] - 1, 'name': 'stale'}, method='PATCH', status=409)
    invalid = graph(); invalid['connections'][0]['to']['node'] = 'missing'
    api(path, {'version': current['version'], 'definition': json.dumps(invalid)}, method='PATCH', status=400)
    api(path, {'version': current['version'], 'message': '版' * 201}, method='PATCH', status=400)
    assert api(path) == current and api(path + '/revisions')['total'] == 1
    passed('stale, invalid and oversized-message writes roll back the definition, version and history')
    body = {'version': current['version'], 'definition': json.dumps(graph(100))}
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(api, path, dict(body, definition=json.dumps(graph(value))), 'PATCH', (200, 409)) for value in (100, 200)]
        results = [future.result() for future in futures]
    assert sorted(code for code, _ in results) == [200, 409]
    history = api(path + '/revisions')
    assert history['total'] == 2 and history['items'][0]['number'] == 2
    assert api(path)['draft_revision_id'] == history['items'][0]['id']
    assert api(path)['published_revision_id'] is None
    passed('competing saves accept one CAS write and commit its draft pointer with exactly one revision')
    api(path, {'version': api(path)['version'], 'definition': first['definition'], 'message': 'restore first'}, method='PATCH')
    restored = api(path + '/revisions')['items'][0]
    assert restored['number'] == 3 and restored['id'] != first['id'] and restored['definition'] == first['definition']
    assert api(path + '/revisions/' + first['id']) == first
    page = api(path + '/revisions?page_size=1&sort=number')
    assert page['items'] == [first] and page['total'] == 3
    passed('normal restore creates a new draft and history uses the shared pagination contract')
    api(path, {'version': api(path)['version'], 'group_id': pipeline_group}, method='PATCH', status=400)
    api(root + '/workflow-groups/' + group, method='DELETE', status=409)
    listing = api(root + '/workflows?group_id=' + group + '&lifecycle=active')
    assert listing['total'] == 1 and listing['items'][0]['id'] == workflow
    other = api('/workspaces', {'name': 'Other', 'slug': 'other'})['id']
    foreign_group = api(f'/workspaces/{other}/workflow-groups', {'name': 'Foreign'}, status=201)['id']
    api(path, {'version': api(path)['version'], 'group_id': foreign_group}, method='PATCH', status=400)
    for suffix in ('', '/revisions', '/revisions/' + first['id']):
        api(f'/workspaces/{other}/workflows/{workflow}' + suffix, status=404)
    api(root + '/workflows?workspace_id=' + other, status=400)
    api(path, actor=client(), status=401)
    passed('Workflow groups stay distinct and referenced groups, foreign tenants and scope overrides are rejected')
    reader = client()
    api('/users', {'name': 'Draft reader', 'email': 'reader@example.invalid', 'password': 'temporary-reader-password'}, actor=reader)
    invitation = api(root + '/members/invitation')['code']
    api('/workspace-memberships', {'code': invitation}, actor=reader)
    member = api(root + '/members?q=reader')['items'][0]
    role = api(root + '/roles', {'name': 'Workflow only', 'permissions': ['workflows.read']})['id']
    api(root + '/members/' + member['id'], {'version': member['version'], 'role_id': role}, method='PATCH')
    api(path, actor=reader, status=403)
    api(path + '/revisions', actor=reader, status=403)
    assert api(root + '/workflows', actor=reader)['total'] == 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        same_grants = [future.result() for future in [pool.submit(api, path + '/acl',
            {'user_id': member['user_id'], 'role': 'view'}, status=201) for _ in range(2)]]
    view_grant = same_grants[0]
    assert all(item == view_grant for item in same_grants) and api(path + '/acl')['total'] == 1
    assert api(path + '/acl', {'user_id': member['user_id'], 'role': 'view'}, status=201) == view_grant
    api(path, actor=reader)
    api(path + '/revisions', actor=reader)
    api(path + '/acl', actor=reader, status=403)
    api(path + '/acl', {'user_id': member['user_id'], 'role': 'all'}, actor=reader, status=403)
    api(path, {'version': api(path)['version'], 'name': 'forbidden'}, method='PATCH', actor=reader, status=403)
    passed('workspace read alone grants no object access; direct view is idempotent and cannot edit or delegate')
    role_row = api(root + '/roles/' + role)
    api(root + '/roles/' + role, {'version': role_row['version'], 'name': role_row['name'], 'permissions': []}, method='PATCH')
    api(path, actor=reader, status=403)
    api(path + '/revisions', actor=reader, status=403)
    passed('workspace read permission is required and live role revocation applies to definitions and history')
    role_row = api(root + '/roles/' + role)
    api(root + '/roles/' + role, {'version': role_row['version'], 'name': role_row['name'],
        'permissions': ['workflows.read', 'workflows.create', 'workflows.update', 'workflows.delete',
            'workflows.run', 'workflow-groups.update']}, method='PATCH')
    api(path, {'version': api(path)['version'], 'name': 'forbidden'}, method='PATCH', actor=reader, status=403)
    group_path = root + '/workflow-groups/' + group
    api(group_path + '/acl', actor=reader, status=403)
    api(group_path + '/acl', {'user_id': member['user_id'], 'role': 'all'}, actor=reader, status=403)
    group_grant = api(group_path + '/acl', {'user_id': member['user_id'], 'role': 'all'}, status=201)
    api(path, {'version': api(path)['version'], 'name': 'Inherited editor'}, method='PATCH', actor=reader)
    api(path + '/acl', actor=reader)
    delegated = api(path + '/acl', {'user_id': member['user_id'], 'role': 'run'}, actor=reader, status=201)
    api(path + '/acl/' + delegated['id'] + '?version=1', method='DELETE', actor=reader, status=204)
    api(path, {'version': api(path)['version'], 'group_id': '0'}, method='PATCH', actor=reader)
    api(path, actor=reader)  # Direct view survives losing inherited all.
    api(path, {'version': api(path)['version'], 'name': 'old group authority'}, method='PATCH', actor=reader, status=403)
    api(path + '/acl', actor=reader, status=403)
    api(path, {'version': api(path)['version'], 'group_id': group}, method='PATCH')
    api(group_path + '/acl/' + group_grant['id'] + '?version=0', method='DELETE', status=409)
    api(group_path + '/acl/' + group_grant['id'] + '?version=1', method='DELETE', status=204)
    api(path, {'version': api(path)['version'], 'name': 'revoked group authority'}, method='PATCH', actor=reader, status=403)
    api(path, actor=reader)
    passed('direct and resource-group grants union; moving groups or revoking inherited all takes effect immediately')
    api(path + '/acl', {'user_id': member['user_id'], 'role': 'owner'}, status=400)
    api(path + '/acl', {'user_id': '9223372036854775807', 'role': 'all'}, status=400)
    outsider = client()
    outsider_id = api('/users', {'name': 'ACL outsider', 'email': 'outsider@example.invalid',
        'password': 'temporary-outsider-password'}, actor=outsider)['id']
    api('/workspace-memberships', {'code': api(f'/workspaces/{other}/members/invitation')['code']}, actor=outsider)
    api(path + '/acl', {'user_id': outsider_id, 'role': 'view'}, status=400)
    api('/workspace-memberships', {'code': invitation}, actor=outsider)
    outsider_grant = api(path + '/acl', {'user_id': outsider_id, 'role': 'view'}, status=201)
    api(path, actor=outsider)
    outsider_row = api('/platform/users/' + outsider_id)
    api('/platform/users/' + outsider_id, {'version': outsider_row['version'], 'active': False,
        'is_superadmin': False}, method='PATCH')
    api(path + '/acl', {'user_id': outsider_id, 'role': 'view'}, status=400)
    api(path, actor=outsider, status=401)
    api(path + '/acl/' + outsider_grant['id'] + '?version=1', method='DELETE', status=204)
    api(path + '/acl', {'user_id': member['user_id'], 'role': 'all', 'workspace_id': other}, status=422)
    api(f'/workspaces/{other}/workflows/{workflow}/acl', status=404)
    api(root + '/workflows?object_override=1', actor=reader, status=400)
    api(root + '/workflow-groups/' + pipeline_group + '/acl', status=404)
    api(root + '/workflow-groups/' + foreign_group + '/acl', status=404)
    api(group_path + '/acl/' + view_grant['id'] + '?version=1', method='DELETE', status=404)
    passed('ACL rejects owner roles, inactive or foreign recipients, request scope overrides and mixed parent kinds')
    bulk = []
    for index in range(25):
        item = api(root + '/workflows', {'name': f'ACL bulk {index:02}', 'definition': json.dumps(graph())}, status=201)
        bulk.append(item)
        if index >= 22:
            item_path = root + '/workflows/' + item['id']
            api(item_path + '/acl', {'user_id': member['user_id'], 'role': 'view'}, status=201)
            api(item_path + '/acl', {'user_id': member['user_id'], 'role': 'run'}, status=201)
    page = api(root + '/workflows?q=ACL%20bulk&sort=name&page_size=2', actor=reader)
    assert page['total'] == 3 and [item['id'] for item in page['items']] == [item['id'] for item in bulk[-3:-1]]
    page = api(root + '/workflows?q=ACL%20bulk&sort=name&page_size=2&page=2', actor=reader)
    assert page['total'] == 3 and [item['id'] for item in page['items']] == [bulk[-1]['id']]
    api(root + '/workflows/' + bulk[0]['id'], actor=reader, status=403)
    api(root + '/workflows/' + bulk[0]['id'] + '/revisions', actor=reader, status=403)
    passed('authorization precedes search, pagination and total; overlapping grants never duplicate visible objects')
    owned = api(root + '/workflows', {'name': 'Reader owned', 'definition': json.dumps(graph())}, actor=reader, status=201)
    assert owned['owner_id'] == member['user_id']
    owned_path = root + '/workflows/' + owned['id']
    api(owned_path, {'version': owned['version'], 'name': 'Owner edit'}, method='PATCH', actor=reader)
    role_row = api(root + '/roles/' + role)
    api(root + '/roles/' + role, {'version': role_row['version'], 'name': role_row['name'],
        'permissions': ['workflows.read']}, method='PATCH')
    api(owned_path, actor=reader)
    api(owned_path, {'version': api(owned_path)['version'], 'name': 'owner exceeds ceiling'}, method='PATCH', actor=reader, status=403)
    api(owned_path + '/acl', {'user_id': member['user_id'], 'role': 'all'}, actor=reader, status=403)
    dormant_all = api(path + '/acl', {'user_id': member['user_id'], 'role': 'all'}, status=201)
    api(path, {'version': api(path)['version'], 'name': 'all exceeds ceiling'}, method='PATCH', actor=reader, status=403)
    api(path + '/acl', {'user_id': member['user_id'], 'role': 'run'}, actor=reader, status=403)
    api(path + '/acl/' + dormant_all['id'] + '?version=1', method='DELETE', status=204)
    api(path + '/acl/' + view_grant['id'] + '?version=0', method='DELETE', status=409)
    api(path, actor=reader)
    api(path + '/acl/' + view_grant['id'] + '?version=1', method='DELETE', status=204)
    api(path, actor=reader, status=403)
    api(path + '/revisions/' + first['id'], actor=reader, status=403)
    view_grant = api(path + '/acl', {'user_id': member['user_id'], 'role': 'view'}, status=201)
    passed('owner cannot exceed the workspace ceiling and direct ACL revocation also removes historical access')
    acl_errors = []
    for locale in ('en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'):
        response = api(path, {'version': api(path)['version'], 'definition': json.dumps(invalid)}, method='PATCH', status=400, locale=locale)
        assert 'workflow.graph' not in response['error'] and '/connections/0' in response['error']
        assert 'PRIVATE_GRAPH_PAYLOAD' not in response['error']
        response = api(path + '/acl', {'user_id': member['user_id'], 'role': 'owner'}, status=400, locale=locale)
        assert 'workflow.aclRole' not in response['error']
        acl_errors.append(response['error'])
    assert len(set(acl_errors)) == 6
    audit = api(root + '/activity?page_size=200')
    assert 'PRIVATE_GRAPH_PAYLOAD' not in json.dumps(audit)
    passed('all six locales render graph and ACL errors and audit records omit graph payloads')
    archived = api(path, {'version': api(path)['version'], 'lifecycle': 'archived'}, method='PATCH')
    assert archived['archived_at'] is not None and archived['archived_by'] == created['owner_id'] and archived['published_revision_id'] is None
    api(path, {'version': archived['version'], 'name': 'forbidden'}, method='PATCH', status=409)
    active = api(path, {'version': archived['version'], 'lifecycle': 'active'}, method='PATCH')
    assert active['archived_at'] is None and active['published_revision_id'] is None
    assert api(path + '/revisions')['total'] == 3
    disposable = api(root + '/workflows', {'name': 'Disposable', 'definition': json.dumps(graph())}, status=201)
    temporary = root + '/workflows/' + disposable['id']
    api(temporary + '?version=1', method='DELETE', status=409)
    disposable = api(temporary, {'version': 1, 'lifecycle': 'archived'}, method='PATCH')
    api(temporary + '?version=' + str(disposable['version']), method='DELETE', status=204)
    api(temporary, status=404)
    passed('archive and restore preserve history; physical deletion requires an archived version')
    caller_graph = {'engine': 'ferry-graph', 'schema': 1, 'nodes': [
        {'id': 'child', 'name': 'Child', 'type': 'core.subworkflow', 'type_version': 1,
            'params': {'subject_id': workflow, 'revision_id': first['id'], 'inputs': {}, 'wait': True, 'item_mode': 'batch'},
            'position': [0, 0]}], 'connections': []}
    caller = api(root + '/workflows', {'name': 'Caller', 'definition': json.dumps(caller_graph)}, status=201)
    archived = api(path, {'version': api(path)['version'], 'lifecycle': 'archived'}, method='PATCH')
    api(path + '?version=' + str(archived['version']), method='DELETE', status=409)
    api(root + '/workflows/' + caller['id'] + '/revisions/' + first['id'], status=404)
    passed('referenced child revisions prevent deletion and a revision cannot be read through another parent')
    before = api(path)
    history = api(path + '/revisions')
    server.terminate(); server.wait(timeout=15)
    server = launch('restarted')
    assert api(path) == before and api(path + '/revisions') == history
    api(path, actor=reader)
    assert api(path + '/acl')['items'] == [view_grant]
    assert [row['id'] for row in api(root + '/pipeline-groups')['items']] == [pipeline_group]
    passed('drafts, lifecycle, history and separate group kinds survive a real restart')
    # Use the real sibling ferry-sandbox executable, never mock the pipe reply.
    expression_graph = graph('={{ (()=>{while(true){}})() }}')
    expression_row = api(root + '/workflows', {'name': 'Expression draft',
        'definition': json.dumps(expression_graph)}, status=201)
    expression_path = root + '/workflows/' + expression_row['id']
    for value in ("={{ (()=>{throw new Error('PRIVATE_EXPRESSION')})() }}",
                  "=value={{ ({a:{b:'}}'}}).a.b }}", "={{ /}}/.test('}}') }}",
                  "={{ 1 // ignored }}\n + 2 }}", "={{ missing.upstream.value }}"):
        expression_graph = graph(value)
        expression_row = api(expression_path, {'version': expression_row['version'],
            'definition': json.dumps(expression_graph)}, method='PATCH')
        assert json.loads(expression_row['definition']) == expression_graph
    passed('real isolated compile-only save preserves Ferry templates and never evaluates loops, exceptions or missing upstream data')
    expression_history = api(expression_path + '/revisions')
    errors = []
    for locale in ('en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'):
        response = api(expression_path, {'version': expression_row['version'],
            'definition': json.dumps(graph("={{ 1 + PRIVATE_EXPRESSION + }}"))},
            method='PATCH', status=400, locale=locale)
        assert '/nodes/1/params/fields/0/value' in response['error'], response
        assert 'PRIVATE_EXPRESSION' not in json.dumps(response) and 'workflow.expressionSyntax' not in response['error']
        errors.append(response['error'])
    assert len(set(errors)) == 6
    api(expression_path, {'version': expression_row['version'],
        'definition': json.dumps(graph('=0)); }); (()=>{while(true){}})(); (function(){return ((0'))},
        method='PATCH', status=400)
    assert api(expression_path) == expression_row and api(expression_path + '/revisions') == expression_history
    api(expression_path, {'version': expression_row['version'], 'definition': json.dumps(graph('={{1+}}'))},
        method='PATCH', actor=reader, status=403)
    passed('invalid and malicious syntax returns localized field paths without leaking source, mutating CAS/history or bypassing object permissions')
    catalog = json.loads((Path(__file__).resolve().parents[1] / 'src/shared/workflow-contracts/workflow-core-nodes.json').read_text(encoding='utf-8'))
    wait = next(node for node in catalog['nodes'] if node['type'] == 'core.wait')
    typed = graph()
    typed['nodes'].append({'id': 'wait', 'name': 'Wait', 'type': 'core.wait', 'type_version': 1,
        'params': dict(wait['example_params'], mode='duration', duration_ms='={{ $json.duration }}'), 'position': [400, 0]})
    typed_row = api(root + '/workflows', {'name': 'Typed expression', 'definition': json.dumps(typed)}, status=201)
    assert json.loads(typed_row['definition']) == typed
    for patch in ({'duration_ms': 'not a number'}, {'mode': "={{ 'duration' }}"}):
        invalid_typed = copy.deepcopy(typed); invalid_typed['nodes'][-1]['params'].update(patch)
        api(root + '/workflows/' + typed_row['id'], {'version': typed_row['version'],
            'definition': json.dumps(invalid_typed)}, method='PATCH', status=400)
    passed('manifest whitelists admit deferred numeric expressions while retaining literal type checks and fixed non-expression fields')
    server.terminate(); server.wait(timeout=15)
    env['FERRY_SANDBOX_PATH'] = str(work / 'missing-sandbox.exe')
    server = launch('missing-sandbox')
    response = api(expression_path, {'version': expression_row['version'],
        'definition': json.dumps(graph('={{42}}'))}, method='PATCH', status=503)
    assert 'missing-sandbox' not in json.dumps(response)
    assert api(expression_path) == expression_row and api(expression_path + '/revisions') == expression_history
    expression_row = api(expression_path, {'version': expression_row['version'], 'name': 'Metadata still editable'}, method='PATCH')
    server.terminate(); server.wait(timeout=15)
    env.pop('FERRY_SANDBOX_PATH')
    server = launch('expressions-restarted')
    assert api(expression_path) == expression_row and api(expression_path + '/revisions') == expression_history
    assert 'PRIVATE_EXPRESSION' not in json.dumps(api(root + '/activity?page_size=200'))
    passed('missing sandbox fails closed without partial writes; metadata remains editable and raw templates/history survive restart')
    # Previews evaluate unsaved parameters over samples, with live editor ACL.
    api(root + '/variables', {'name': 'PREVIEW_FACTOR', 'scope': 'global', 'kind': 'plain', 'value': '3'}, status=201)
    api(root + '/variables', {'name': 'PREVIEW_SECRET', 'scope': 'global', 'kind': 'secret', 'value': 'PRIVATE_PREVIEW_SECRET'}, status=201)
    api(root + '/variables', {'name': 'BUILD_ONLY', 'scope': 'build', 'kind': 'plain', 'value': 'PRIVATE_BUILD_ONLY'}, status=201)
    api(f'/workspaces/{other}/variables', {'name': 'OTHER_SPACE', 'scope': 'global', 'kind': 'plain', 'value': 'PRIVATE_OTHER_SPACE'}, status=201)
    preview_path = root + '/workflows/' + typed_row['id'] + '/expression-previews'
    preview_body = {'version': typed_row['version'], 'node_path': '/nodes/2',
        'params': {'mode': 'duration', 'duration_ms': '={{ Number($vars.PREVIEW_FACTOR) * $json.value }}'},
        'data': {'items': [{'json': {'value': 2}}, {'json': {'value': 5}}], 'upstream': []},
        'selections': [0, 1]}
    preview_history = api(root + '/workflows/' + typed_row['id'] + '/revisions')
    preview_audit = api(root + '/activity?page_size=200')
    result = api(preview_path, preview_body)
    assert result['items'] == [{'mode': 'duration', 'duration_ms': 6}, {'mode': 'duration', 'duration_ms': 15}], result
    assert result['total'] == 2 and result['version'] == typed_row['version']
    assert api(root + '/workflows/' + typed_row['id']) == typed_row
    assert api(root + '/workflows/' + typed_row['id'] + '/revisions') == preview_history
    assert api(root + '/activity?page_size=200') == preview_audit
    passed('real HTTP previews resolve unsaved typed parameters per item using workspace plain variables without mutating draft, revision or audit')
    errors = []
    for locale in ('en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'):
        bad_preview = copy.deepcopy(preview_body)
        bad_preview['params']['duration_ms'] = '={{ "PRIVATE_BAD_TYPE" }}'
        response = api(preview_path, bad_preview, status=400, locale=locale)
        assert '/nodes/2/params/duration_ms' in response['error'] and 'PRIVATE' not in json.dumps(response), response
        errors.append(response['error'])
    assert len(set(errors)) == 6
    for source, status in [('={{1+}}', 400), ('={{(()=>{throw new Error("PRIVATE_PREVIEW")})()}}', 400),
                           ('={{(()=>{while(true){}})()}}', 429)]:
        bad_preview = copy.deepcopy(preview_body); bad_preview['params']['duration_ms'] = source
        response = api(preview_path, bad_preview, status=status)
        assert '/nodes/2/params/duration_ms' in response['error'] and 'PRIVATE' not in json.dumps(response)
    assert api(root + '/workflows/' + typed_row['id']) == typed_row
    passed('preview type, syntax, exception and timeout failures return localized field paths without source leakage or partial writes')
    api(preview_path, preview_body, actor=reader, status=403)
    role_row = api(root + '/roles/' + role)
    api(root + '/roles/' + role, {'version': role_row['version'], 'name': role_row['name'],
        'permissions': ['workflows.read', 'workflows.update']}, method='PATCH')
    api(preview_path, preview_body, actor=reader, status=403)
    preview_grant = api(root + '/workflows/' + typed_row['id'] + '/acl',
        {'user_id': member['user_id'], 'role': 'all'}, status=201)
    assert api(preview_path, preview_body, actor=reader)['items'] == result['items']
    api(root + '/workflows/' + typed_row['id'] + '/acl/' + preview_grant['id'] + '?version=1', method='DELETE', status=204)
    api(preview_path, preview_body, actor=reader, status=403)
    api(f'/workspaces/{other}/workflows/' + typed_row['id'] + '/expression-previews', preview_body, status=404)
    stale = copy.deepcopy(preview_body); stale['version'] = 0
    api(preview_path, stale, status=409)
    injected = copy.deepcopy(preview_body); injected['context'] = {'$vars': {'PREVIEW_FACTOR': '999'}}
    api(preview_path, injected, status=422)
    passed('preview workspace ceiling, object edit ACL, revocation, workspace isolation, version conflicts and context injection are enforced')
    safe_params = {'fields': [{'path': 'value', 'value': '={{ ({factor:$vars.PREVIEW_FACTOR,secret:typeof $vars.PREVIEW_SECRET,build:typeof $vars.BUILD_ONLY,other:typeof $vars.OTHER_SPACE,env:typeof $env,secrets:typeof secrets,mode:$execution.mode}) }}'}], 'keep_other_fields': True}
    safe_body = dict(preview_body, node_path='/nodes/1', params=safe_params)
    values = api(preview_path, safe_body)['items']
    expected = {'factor': '3', 'secret': 'undefined', 'build': 'undefined', 'other': 'undefined',
        'env': 'undefined', 'secrets': 'undefined', 'mode': 'preview'}
    assert all(row['fields'][0]['value'] == expected for row in values), values
    for node_path in ('/nodes/02', '/nodes/2/params', '/pin_data/start/0', '/nodes/999'):
        bad_path = dict(preview_body, node_path=node_path)
        api(preview_path, bad_path, status=404)
    assert 'PRIVATE_PREVIEW' not in json.dumps(api(root + '/activity?page_size=200'))
    passed('preview context excludes secrets, build overrides, other workspaces and host globals; node selection cannot address arbitrary definition data')
    from datetime import datetime, timedelta
    typed_path = root + '/workflows/' + typed_row['id']
    typed['settings'] = {'timezone': 'Asia/Kuala_Lumpur'}
    typed_row = api(typed_path, {'version': typed_row['version'], 'definition': json.dumps(typed)}, method='PATCH')
    clock_body = dict(preview_body, version=typed_row['version'], node_path='/nodes/1',
        params={'fields':[{'path':'value','value':'={{({now:$now.toISO(),today:$today.toISO(),zone:$now.zoneName,clock:typeof Date})}}'}], 'keep_other_fields':True})
    clock_history = api(typed_path + '/revisions')
    clock_audit = api(root + '/activity?page_size=200')
    started = time.time()
    clock_values = api(preview_path, clock_body)['items']
    finished = time.time()
    clock_value = clock_values[0]['fields'][0]['value']
    assert all(row['fields'][0]['value'] == clock_value for row in clock_values)
    now, today = (datetime.fromisoformat(clock_value[key]) for key in ('now','today'))
    assert started <= now.timestamp() + .001 and now.timestamp() <= finished
    assert now.utcoffset() == today.utcoffset() == timedelta(hours=8)
    assert today.date() == now.date() and (today.hour,today.minute,today.second,today.microsecond) == (0,0,0,0)
    assert clock_value['zone'] == 'Asia/Kuala_Lumpur' and clock_value['clock'] == 'undefined'
    errors = []
    for locale in ('en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'):
        invalid_zone = copy.deepcopy(typed); invalid_zone['settings']['timezone'] = 'PRIVATE_INVALID_ZONE'
        response = api(typed_path, {'version':typed_row['version'],'definition':json.dumps(invalid_zone)}, method='PATCH',status=400,locale=locale)
        assert '/settings/timezone' in response['error'] and 'PRIVATE' not in json.dumps(response)
        assert 'workflow.timezoneInvalid' not in response['error']
        errors.append(response['error'])
    assert len(set(errors)) == 6
    assert api(typed_path) == typed_row and api(typed_path + '/revisions') == clock_history
    assert api(root + '/activity?page_size=200') == clock_audit
    injected_clock = dict(clock_body, context={'$now':{'iso':'2000-01-01T00:00:00.000Z','zone':'UTC'}})
    api(preview_path,injected_clock,status=422)
    server.terminate(); server.wait(timeout=15)
    server = launch('clock-restarted')
    assert api(typed_path) == typed_row
    assert api(preview_path,clock_body)['items'][0]['fields'][0]['value']['zone'] == 'Asia/Kuala_Lumpur'
    passed('preview freezes one server instant per batch with persisted workflow timezone, day start, no native clock or client override; invalid IANA timezone is localized without writes')
    def await_run(run, state, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            current_run = api(root + '/workflow-executions/' + run['id'])
            if current_run['state'] == state:
                return current_run
            assert current_run['state'] not in ('failed', 'succeeded', 'cancelled'), current_run
            time.sleep(.05)
        raise AssertionError(('execution timeout', state, current_run))

    def execution_graph(duration=None):
        value = graph('={{ $json.answer + 1 }}')
        value['nodes'][0]['params']['input_schema'] = {
            'type':'object', 'properties':{'answer':{'type':'integer'}},
            'required':['answer'], 'additionalProperties':False}
        if duration is not None:
            value['nodes'].insert(1, {'id':'wait','name':'Wait','type':'core.wait',
                'type_version':1,'params':{'mode':'duration','duration_ms':float(duration)},'position':[100,0]})
            value['connections'] = [
                {'from':{'node':'start','port':'main','index':0},'to':{'node':'wait','port':'main','index':0}},
                {'from':{'node':'wait','port':'main','index':0},'to':{'node':'set','port':'main','index':0}}]
        return value

    def new_runnable(value):
        return api(root + '/workflows', {'name':'Manual execution',
            'definition':json.dumps(value)}, status=201)

    def start_run(row, actor=owner, **overrides):
        body = dict(mode='manual',revision='draft',version=row['version'],
            entry_node_id='start',inputs={'answer':41})
        body.update(overrides)
        return api(root + '/workflows/' + row['id'] + '/executions', body, actor=actor, status=201)

    executable = new_runnable(execution_graph())
    execution_path = root + '/workflows/' + executable['id'] + '/executions'
    request = dict(mode='manual',revision='draft',version=executable['version'],
        entry_node_id='start',inputs={'answer':41})
    for overrides, status in [({'version':0},409),({'revision':'published'},400),
            ({'entry_node_id':'set'},400),({'inputs':{'answer':'41'}},400),
            ({'inputs':{'answer':41,'extra':True}},400),({'owner_id':'0'},422)]:
        api(execution_path,dict(request,**overrides),status=status)
    api(execution_path,request,actor=reader,status=403)
    passed('manual run rejects stale revision, missing publication, non-trigger entry, invalid typed inputs and authority injection')
    for settings in ({'retry':{'max_tries':2,'wait_ms':0}},):
        deferred_graph = execution_graph()
        deferred_graph['nodes'][1]['settings'] = settings
        deferred = new_runnable(deferred_graph)
        api(root + '/workflows/' + deferred['id'] + '/executions',
            dict(request,version=deferred['version']),status=400)
    passed('drafts retain deferred node settings but manual execution rejects unsupported semantics explicitly')
    run = start_run(executable)
    finished_run = await_run(run,'succeeded')
    node_path = root + '/workflow-executions/' + run['id'] + '/node-runs'
    completed_nodes = api(node_path)['items']
    assert sorted((n['node_id'],n['state'],n['attempt']) for n in completed_nodes) == [
        ('set','succeeded',1),('start','succeeded',1)], completed_nodes
    assert finished_run['revision_id'] == executable['draft_revision_id']
    listed = api(execution_path + '?page_size=1&sort=-number')
    assert listed['total'] == 1 and listed['page_size'] == 1 and listed['items'][0]['id'] == run['id']
    assert api(execution_path + '?state=failed')['total'] == 0
    api(execution_path + '?workspace_id=' + other, status=400)
    api(f'/workspaces/{other}/workflows/{executable["id"]}/executions', status=404)
    api(execution_path, actor=client(), status=401)
    assert finished_run['data_bytes'] > 0 and finished_run['finished_at'] >= finished_run['started_at'] > 0
    assert 'context' not in finished_run and 'inputs' not in finished_run
    api(root + '/workflow-executions/' + run['id'],actor=reader,status=403)
    api(f'/workspaces/{other}/workflow-executions/' + run['id'],status=404)
    passed('real scheduler executes manual input and expression nodes against immutable revision; execution reads enforce object and workspace isolation')
    for mode, downstream_mode in (('bypass','active'),('mute','active'),('mute','bypass')):
        mode_graph = execution_graph()
        mode_graph['nodes'][1]['settings'] = {'mode':mode}
        mode_graph['nodes'][1]['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("MODE_MUST_NOT_EVALUATE")})() }}'
        observer = copy.deepcopy(mode_graph['nodes'][1])
        observer.update(id='observe',name='Observe',settings={'mode':downstream_mode})
        observer['params']['fields'][0]['value'] = '={{ $json.answer === 41 ? 42 : (()=>{throw new Error("BYPASS_CHANGED_INPUT")})() }}'
        mode_graph['nodes'].append(observer)
        mode_graph['connections'].append({'from':{'node':'set','port':'main','index':0},
            'to':{'node':'observe','port':'main','index':0}})
        mode_run = start_run(new_runnable(mode_graph))
        await_run(mode_run,'succeeded')
        mode_nodes = api(root + '/workflow-executions/' + mode_run['id'] + '/node-runs')['items']
        indexed = {n['node_id']:n for n in mode_nodes}
        assert len(indexed) == 3 and indexed['start']['state'] == 'succeeded',mode_nodes
        assert indexed['set']['state'] == ('bypassed' if mode == 'bypass' else 'skipped'),mode_nodes
        assert indexed['observe']['state'] == ('succeeded' if mode == 'bypass' else 'skipped'),mode_nodes
        assert indexed['set']['items_in'] == 1 and indexed['set']['items_out'] == (1 if mode == 'bypass' else 0),mode_nodes
        assert indexed['observe']['items_out'] == (1 if mode == 'bypass' else 0),mode_nodes
    passed('bypass preserves input and runs downstream while mute skips downstream; neither evaluates throwing node parameters')
    for source_mode, mode, always, expected in (
            ('bypass','active',True,1), ('bypass','active',False,0),
            ('bypass','bypass',True,0), ('bypass','mute',True,0), ('mute','active',True,0)):
        empty_graph = execution_graph()
        empty_graph['nodes'][0]['settings'] = {'mode':source_mode}
        empty_graph['nodes'][1]['settings'] = {'mode':mode,'always_output':always}
        empty_graph['nodes'][1]['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("EMPTY_INPUT_MUST_NOT_EVALUATE")})() }}'
        observer = copy.deepcopy(empty_graph['nodes'][1])
        observer.update(id='observe',name='Observe',settings={'mode':'active'})
        observer['params']['fields'][0]['value'] = '={{ Object.keys($json).length === 0 ? 42 : (()=>{throw new Error("EXPECTED_EMPTY_ITEM")})() }}'
        empty_graph['nodes'].append(observer)
        empty_graph['connections'].append({'from':{'node':'set','port':'main','index':0},
            'to':{'node':'observe','port':'main','index':0}})
        empty_run = start_run(new_runnable(empty_graph))
        await_run(empty_run,'succeeded')
        indexed = {n['node_id']:n for n in api(root + '/workflow-executions/' + empty_run['id'] + '/node-runs')['items']}
        assert len(indexed) == 3 and indexed['set']['items_in'] == 0,indexed
        assert indexed['set']['items_out'] == indexed['observe']['items_out'] == expected,indexed
        state = 'skipped' if source_mode == 'mute' or mode == 'mute' else ('bypassed' if mode == 'bypass' else 'succeeded')
        assert indexed['set']['state'] == state,indexed
    passed('always_output creates one empty item only for active successful zero-item work; disabled, bypass and skipped paths remain empty')
    pinned_graph = execution_graph()
    pinned_graph['pin_data'] = {'start':[{'json':{'answer':41}},{'json':{'answer':42}},{'json':{'answer':43}}]}
    pinned_graph['nodes'][1]['settings'] = {'execute_once':True}
    pinned_graph['nodes'][1]['params']['fields'][0]['value'] = '={{ $json.answer === 41 ? $input.all().length : (()=>{throw new Error("SECOND_ITEM_MUST_NOT_RUN")})() }}'
    observer = copy.deepcopy(pinned_graph['nodes'][1])
    observer.update(id='observe',name='Observe',settings={})
    observer['params']['fields'][0]['value'] = '={{ $json.answer === 3 ? 42 : (()=>{throw new Error("EXPECTED_COMPLETE_INPUT_SNAPSHOT")})() }}'
    pinned_graph['nodes'].append(observer)
    pinned_graph['connections'].append({'from':{'node':'set','port':'main','index':0},'to':{'node':'observe','port':'main','index':0}})
    pinned_run = start_run(new_runnable(pinned_graph))
    await_run(pinned_run,'succeeded')
    pinned_nodes = {n['node_id']:n for n in api(root + '/workflow-executions/' + pinned_run['id'] + '/node-runs')['items']}
    assert pinned_nodes['start']['state'] == 'pinned' and pinned_nodes['start']['items_out'] == 3,pinned_nodes
    assert pinned_nodes['set']['items_in'] == 3 and pinned_nodes['set']['items_out'] == pinned_nodes['observe']['items_out'] == 1,pinned_nodes
    all_items_graph = copy.deepcopy(pinned_graph)
    all_items_graph['nodes'][1]['settings']['execute_once'] = False
    all_items_run = start_run(new_runnable(all_items_graph))
    assert await_run(all_items_run,'failed')['end_reason'] == 'node_failed'
    passed('pinned manual input feeds three items; execute_once processes only the first while input.all retains all three; disabling it exposes the second-item failure')
    for mode in ('active','mute','bypass'):
        fixed_graph = execution_graph()
        fixed_graph['pin_data'] = {'set':[]}
        fixed_graph['nodes'][1]['settings'] = {'mode':mode,'always_output':True}
        fixed_graph['nodes'][1]['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("PIN_MUST_NOT_EVALUATE")})() }}'
        fixed_run = start_run(new_runnable(fixed_graph))
        await_run(fixed_run,'succeeded')
        fixed_nodes = {n['node_id']:n for n in api(root + '/workflow-executions/' + fixed_run['id'] + '/node-runs')['items']}
        assert fixed_nodes['set']['state'] == {'active':'pinned','mute':'skipped','bypass':'bypassed'}[mode],fixed_nodes
        assert fixed_nodes['set']['items_out'] == (1 if mode == 'bypass' else 0),fixed_nodes
    passed('empty pins remain pinned with zero output and no parameter evaluation or always_output synthesis; mute and bypass retain their explicit modes')

    def stored_rows(sql):
        if postgres:
            return [line.split('\t') for line in pg_sql(sql).strip().splitlines()]
        if mysqld:
            result = subprocess.run([str(mysqld.with_name('mysql.exe')),'--no-defaults',
                '--protocol=TCP','--host=127.0.0.1',f'--port={mysql_port}','--user=root',
                '--database=workflow_drafts','--batch','--skip-column-names','--execute='+sql],
                capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
            assert result.returncode == 0,result.stderr
            return [line.split('\t') for line in result.stdout.decode().strip().splitlines()]
        import sqlite3
        with sqlite3.connect((work / 'server' / 'ferry.sqlite').as_uri() + '?mode=ro',uri=True) as database:
            return database.execute(sql).fetchall()
    assert str(workspace).isdigit()
    asset_id,asset_hash,asset_size,asset_mime = stored_rows('SELECT id,sha256,size,mime FROM assets WHERE workspace_id=' + str(workspace) + ' ORDER BY id LIMIT 1')[0]
    asset_graph = execution_graph()
    asset_graph['pin_data'] = {'start':[{'json':{'answer':41},'binary':{'sample':{'asset':'sha256:'+asset_hash,'size':int(asset_size),'mime':asset_mime,'name':'sample.json'}}}]}
    asset_row = new_runnable(asset_graph)
    api(f'/workspaces/{other}/workflows', {'name':'Foreign pin', 'definition':json.dumps(asset_graph)}, status=409)
    revision_id = asset_row['draft_revision_id']
    refs = stored_rows('SELECT owner_id FROM asset_refs WHERE asset_id=' + str(asset_id) + " AND owner_kind='pin' AND owner_id=" + revision_id)
    assert len(refs) == 1,refs
    asset_run = start_run(asset_row)
    await_run(asset_run,'succeeded')
    asset_path = root + '/workflows/' + asset_row['id']
    original_history = api(asset_path + '/revisions')
    for change in ('missing','size','mime'):
        invalid_graph = copy.deepcopy(asset_graph)
        reference = invalid_graph['pin_data']['start'][0]['binary']['sample']
        if change == 'missing': reference['asset'] = 'sha256:' + '0'*64
        elif change == 'size': reference['size'] += 1
        else: reference['mime'] = 'application/other'
        api(asset_path,{'version':asset_row['version'],'definition':json.dumps(invalid_graph)},method='PATCH',status=409)
        assert api(asset_path) == asset_row and api(asset_path + '/revisions') == original_history
    changed_asset_graph = copy.deepcopy(asset_graph); changed_asset_graph.pop('pin_data')
    api(asset_path,{'version':asset_row['version'],'definition':json.dumps(changed_asset_graph)},method='PATCH')
    assert len(stored_rows('SELECT owner_id FROM asset_refs WHERE asset_id=' + str(asset_id) + " AND owner_kind='pin' AND owner_id=" + revision_id)) == 1
    passed('saving binary pins retains original revision asset references; foreign, missing or mismatched references roll back draft and history; later drafts retain historical pins')
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        concurrent_runs = list(pool.map(lambda _:start_run(executable),range(2)))
    assert len({value['number'] for value in [run,*concurrent_runs]}) == 3
    assert len({value['id'] for value in [run,*concurrent_runs]}) == 3
    for value in concurrent_runs:
        await_run(value,'succeeded')
    passed('competing manual requests allocate distinct durable execution identities and workspace-wide numbers')

    waiting_graph = execution_graph(4000)
    waiting_graph['pin_data'] = {'start':[{'json':{'answer':41}},{'json':{'answer':42}},{'json':{'answer':43}}]}
    waiting_graph['nodes'][2]['settings'] = {'execute_once':True}
    waiting_graph['nodes'][1]['settings'] = {'always_output':True}
    bypass_node = copy.deepcopy(waiting_graph['nodes'][2])
    bypass_node.update(id='pass',name='Pass',settings={'mode':'bypass'})
    bypass_node['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("BYPASS_MUST_NOT_EXECUTE")})() }}'
    waiting_graph['nodes'].append(bypass_node)
    waiting_graph['connections'][0]['from']['node'] = 'pass'
    waiting_graph['connections'].append({'from':{'node':'start','port':'main','index':0},
        'to':{'node':'pass','port':'main','index':0}})
    waiting_row = new_runnable(waiting_graph)
    waiting_run = start_run(waiting_row)
    parked = await_run(waiting_run,'waiting')
    before_restart = api(root + '/workflow-executions/' + waiting_run['id'] + '/node-runs')['items']
    assert sorted((n['node_id'],n['state']) for n in before_restart) == [('pass','bypassed'),('start','pinned'),('wait','waiting')]
    assert next(n for n in before_restart if n['node_id'] == 'wait')['items_out'] == 0
    changed_graph = copy.deepcopy(waiting_graph)
    changed_graph['pin_data']['start'][0]['json']['answer'] = 999
    changed_graph['nodes'][2]['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("NEW_DRAFT_MUST_NOT_RUN")})() }}'
    changed_row = api(root + '/workflows/' + waiting_row['id'],
        {'version':waiting_row['version'],'definition':json.dumps(changed_graph)},method='PATCH')
    assert changed_row['draft_revision_id'] != waiting_run['revision_id']
    server.kill(); server.wait(timeout=15)
    server = launch('execution-crash-restarted')
    recovered = await_run(waiting_run,'succeeded')
    recovered_nodes = api(root + '/workflow-executions/' + waiting_run['id'] + '/node-runs')['items']
    assert len(recovered_nodes) == 4 and all(n['attempt'] == 1 and n['state'] == {'pass':'bypassed','start':'pinned'}.get(n['node_id'],'succeeded') for n in recovered_nodes)
    assert next(n for n in before_restart if n['node_id'] == 'pass') == next(n for n in recovered_nodes if n['node_id'] == 'pass')
    assert next(n for n in before_restart if n['node_id'] == 'start') == next(n for n in recovered_nodes if n['node_id'] == 'start')
    assert next(n for n in recovered_nodes if n['node_id'] == 'set')['items_out'] == 1
    assert {n['node_id']:n['id'] for n in before_restart}.items() <= {n['node_id']:n['id'] for n in recovered_nodes}.items()
    assert recovered['waiting_ms'] > 0 and recovered['started_at'] == parked['started_at']
    assert api(node_path)['items'] == completed_nodes
    passed('force-killed server resumes a durable timed wait on its original immutable revision despite draft edits, with original node identities and no completed-work replay')

    crash_graph = execution_graph()
    crash_graph['nodes'] = crash_graph['nodes'][:1]
    crash_graph['nodes'][0]['params']['input_schema']['properties']['payload'] = {'type':'string'}
    crash_graph['connections'] = []
    previous = 'start'
    for index in range(50):
        name = f'step{index}'
        crash_graph['nodes'].append({'id':name,'name':name,'type':'data.set','type_version':1,
            'params':{'fields':[{'path':'step','value':index}],'keep_other_fields':True},'position':[index*100,0]})
        crash_graph['connections'].append({'from':{'node':previous,'port':'main','index':0},
            'to':{'node':name,'port':'main','index':0}})
        previous = name
    crash_row = new_runnable(crash_graph)
    crash_run = start_run(crash_row,inputs={'answer':41,'payload':'x'*(96*1024)})
    crash_nodes_path = root + '/workflow-executions/' + crash_run['id'] + '/node-runs'
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        at_crash = api(crash_nodes_path)['items']
        if any(n['state'] == 'running' for n in at_crash) and any(n['state'] == 'succeeded' for n in at_crash):
            break
        time.sleep(.005)
    else:
        raise AssertionError(('did not observe an active persisted node attempt',at_crash))
    server.kill(); server.wait(timeout=15)
    def stored_lease(value):
        assert value['id'].isdigit()
        sql = 'SELECT state,lease_epoch,started_at FROM workflow_executions WHERE id=' + value['id']
        if postgres:
            state,epoch,started = stored_rows(sql)[0]
            return state,int(epoch),int(started)
        if mysqld:
            result = subprocess.run([str(mysqld.with_name('mysql.exe')),'--no-defaults',
                '--protocol=TCP','--host=127.0.0.1',f'--port={mysql_port}','--user=root',
                '--database=workflow_drafts','--batch','--skip-column-names','--execute='+sql],
                capture_output=True,timeout=15,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
            assert result.returncode == 0, result.stderr
            state,epoch,started = result.stdout.decode().strip().split('\t')
            return state,int(epoch),int(started)
        import sqlite3
        with sqlite3.connect((work / 'server' / 'ferry.sqlite').as_uri() + '?mode=ro',uri=True) as database:
            return database.execute(sql).fetchone()
    crashed_lease = stored_lease(crash_run)
    assert crashed_lease[0] == 'running' and crashed_lease[1] >= 1, crashed_lease
    server = launch('running-lease-crash-restarted')
    # A hard kill cannot release the SQL lock. This 51-node asset-backed graph
    # gets 30s for the real lease TTL plus 60s for recovery/work, rather than the
    # 15s bound used by the ordinary two-node checks. This is a recovery test,
    # not a throughput benchmark; all identity/no-replay assertions stay exact.
    crash_recovered = await_run(crash_run,'succeeded',timeout=90)
    after_crash = api(crash_nodes_path)['items']
    assert len(after_crash) == 51 and all(n['state'] == 'succeeded' and n['attempt'] == 1 for n in after_crash)
    by_id = {n['id']:n for n in after_crash}
    assert all(n['id'] in by_id for n in at_crash)
    assert all(by_id[n['id']] == n for n in at_crash if n['state'] == 'succeeded')
    assert crash_recovered['revision_id'] == crash_row['draft_revision_id'] and crash_recovered['waiting_ms'] == 0
    recovered_lease = stored_lease(crash_run)
    assert recovered_lease[0] == 'succeeded' and recovered_lease[1] > crashed_lease[1]
    assert recovered_lease[2] == crashed_lease[2]
    passed('hard-killed running lease is reclaimed after real SQL expiry; active attempt resumes and completed nodes retain exact identity, versions and timestamps')

    failure_graph = execution_graph()
    failure_graph['nodes'][1]['settings'] = {'always_output':True}
    failure_graph['nodes'][1]['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("PRIVATE_RUNTIME_FAILURE")})() }}'
    failed_row = new_runnable(failure_graph)
    failed_run = start_run(failed_row)
    failure = await_run(failed_run,'failed')
    assert failure['end_reason'] == 'node_failed' and 'PRIVATE_RUNTIME_FAILURE' not in json.dumps(failure)
    passed('runtime expression failure terminates the execution without leaking tenant expression source')

    role_row = api(root + '/roles/' + role)
    api(root + '/roles/' + role, {'version':role_row['version'],'name':role_row['name'],
        'permissions':['workflows.read','workflows.run']},method='PATCH')
    revoked_row = new_runnable(execution_graph(2500))
    grant = api(root + '/workflows/' + revoked_row['id'] + '/acl',
        {'user_id':member['user_id'],'role':'run'},status=201)
    revoked_run = start_run(revoked_row,actor=reader)
    await_run(revoked_run,'waiting')
    api(root + '/workflows/' + revoked_row['id'] + '/acl/' + grant['id'] + '?version=1',method='DELETE',status=204)
    revoked = await_run(revoked_run,'failed')
    assert revoked['end_reason'] == 'permission_revoked', revoked
    assert not any(n['node_id'] == 'set' for n in api(root + '/workflow-executions/' + revoked_run['id'] + '/node-runs')['items'])
    passed('revoking object run permission during a wait prevents downstream work when the execution resumes')

    def transform_graph(values, steps, assertion):
        value = execution_graph()
        start = value['nodes'][0]
        observer = value['nodes'][1]
        value['nodes'] = [start]
        value['connections'] = []
        value['pin_data'] = {'start':[{'json':entry} for entry in values]}
        previous = 'start'
        for index, (kind, params) in enumerate(steps):
            identity = f'transform_{index}'
            value['nodes'].append({'id':identity,'name':identity,'type':kind,'type_version':1,
                'position':[index*100,0],'params':params})
            value['connections'].append({'from':{'node':previous,'port':'main','index':0},
                'to':{'node':identity,'port':'main','index':0}})
            previous = identity
        observer['params']['fields'][0]['value'] = assertion
        value['nodes'].append(observer)
        value['connections'].append({'from':{'node':previous,'port':'main','index':0},
            'to':{'node':'set','port':'main','index':0}})
        return value

    transform = transform_graph([
        {'rank':2,'key':'x','old':'A','parts':[1,2]},
        {'rank':1,'key':'y','old':'B','parts':[3,4]},
        {'rank':2,'key':'x','old':'C','parts':[5,6]},
    ],[
        ('core.sort',{'keys':[{'path':'rank','direction':'asc','nulls':'last'}]}),
        ('core.deduplicate',{'scope':'execution','keys':['key'],'keep':'last'}),
        ('core.limit',{'count':1,'offset':1}),
        ('data.rename',{'mappings':[{'from':'old','to':'renamed'}],'on_collision':'error'}),
        ('core.split',{'path':'parts','include':'all'}),
        ('core.no_op',{}),
    ],'={{ $json.renamed === "C" && !("old" in $json) && JSON.stringify($input.all().map(x=>x.json.parts)) === "[5,6]" ? 42 : (()=>{throw new Error("TRANSFORM_ORDER_OR_VALUE_CHANGED")})() }}')
    transformed_run = start_run(new_runnable(transform))
    await_run(transformed_run,'succeeded')
    transformed_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + transformed_run['id'] + '/node-runs')['items']}
    assert [transformed_nodes[f'transform_{index}']['items_out'] for index in range(6)] == [3,2,1,1,2,2],transformed_nodes
    assert all(node['state'] in ('succeeded','pinned') for node in transformed_nodes.values())
    passed('real scheduler composes stable sort, execution dedup keep-last, limit, rename, split and no-op; downstream sandbox verifies exact ordered JSON values')
    server.kill(); server.wait(timeout=15)
    server = launch('transformed-results-restarted')
    reloaded_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + transformed_run['id'] + '/node-runs')['items']}
    assert reloaded_nodes == transformed_nodes
    assert api(root + '/workflow-executions/' + transformed_run['id'])['state'] == 'succeeded'
    passed('hard restart preserves completed transformation identities, counts, versions and timestamps without replay')

    text_graph = transform_graph([{'phrase':'a😀中'}],[
        ('data.text_split',{'value':'={{$json.phrase}}','mode':'length','length':2,'overlap':1}),
    ],'={{ JSON.stringify($input.all().map(x=>x.json.text)) === JSON.stringify(["a😀","😀中"]) ? 42 : (()=>{throw new Error("TEXT_SPLIT_UNICODE_CHANGED")})() }}')
    text_run = start_run(new_runnable(text_graph))
    await_run(text_run,'succeeded')
    text_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + text_run['id'] + '/node-runs')['items']}
    assert text_nodes['transform_0']['items_out'] == text_nodes['set']['items_in'] == 2,text_nodes
    passed('text split resolves per-item parameters and counts Unicode scalars instead of bytes or UTF-16 halves')

    empty_limit = transform_graph([{'rank':1}],[('core.limit',{'count':0,'offset':0})],
        '={{ (()=>{throw new Error("EMPTY_LIMIT_MUST_NOT_EVALUATE")})() }}')
    empty_limit_run = start_run(new_runnable(empty_limit))
    await_run(empty_limit_run,'succeeded')
    empty_limit_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + empty_limit_run['id'] + '/node-runs')['items']}
    assert empty_limit_nodes['transform_0']['state'] == 'succeeded' and empty_limit_nodes['transform_0']['items_out'] == 0,empty_limit_nodes
    assert empty_limit_nodes['set']['items_in'] == empty_limit_nodes['set']['items_out'] == 0,empty_limit_nodes
    mixed = transform_graph([{'rank':1},{'rank':'1'}],[('core.sort',{'keys':[{'path':'rank','direction':'asc','nulls':'last'}]})],42)
    mixed_run = start_run(new_runnable(mixed))
    assert await_run(mixed_run,'failed')['end_reason'] == 'node_failed'
    failure_runs = [mixed_run]
    cross_run = transform_graph([{'key':'one'}],[('core.deduplicate',{'scope':'workflow','keys':['key'],'keep':'first'})],42)
    cross_row = new_runnable(cross_run)
    api(root + '/workflows/' + cross_row['id'] + '/executions',dict(request,version=cross_row['version']),status=400)
    passed('zero-limit remains successful empty output; mixed-type sorting fails without coercion; workflow-wide dedup is not faked with an execution-local set')

    filtered = transform_graph([{'id':0,'value':None},{'id':1},{'id':2,'value':'x'},{'id':3,'value':'y'}],[
        ('core.filter',{'combinator':'and','rules':[{'left':'={{$json.value}}','operator':'exists'}]}),
        ('core.filter',{'combinator':'and','rules':[{'left':'={{$json.value}}','operator':'ne','right':None}]}),
        ('core.filter',{'combinator':'and','rules':[{'left':'={{$json.value}}','operator':'matches','right':'^[xy]$'}]}),
    ],'={{ JSON.stringify($input.all().map(x=>x.json.id)) === "[2,3]" && $("transform_0").item.json.id === $json.id ? 42 : (()=>{throw new Error("FILTER_OR_LINEAGE_CHANGED")})() }}')
    filtered_row = new_runnable(filtered)
    conditional_preview = api(root + '/workflows/' + filtered_row['id'] + '/expression-previews',
        {'version':filtered_row['version'],'node_path':'/nodes/1',
            'data':{'items':[{'json':{'value':None}}],'upstream':[]},'selections':[0]})
    assert conditional_preview['items'] == [{'combinator':'and','rules':[{'left':None,'operator':'exists'}]}],conditional_preview
    filtered_run = start_run(filtered_row)
    await_run(filtered_run,'succeeded')
    filter_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + filtered_run['id'] + '/node-runs')['items']}
    assert [filter_nodes[f'transform_{index}']['items_out'] for index in range(3)] == [3,2,2],filter_nodes
    passed('conditional filter distinguishes missing from null, executes sandbox regex and preserves per-item provenance through multiple filters')

    def condition_node(identity,kind,params):
        return {'id':identity,'name':identity,'type':kind,'type_version':1,'position':[0,0],'params':params}
    def connection(source,target,output='main',output_index=0,input_port='main',input_index=0):
        return {'from':{'node':source,'port':output,'index':output_index},'to':{'node':target,'port':input_port,'index':input_index}}
    def rule(left,operator='eq',right=True):
        return {'combinator':'and','rules':[{'left':left,'operator':operator,'right':right}]}
    def observer(identity,expression):
        return condition_node(identity,'data.set',{'keep_other_fields':True,'fields':[{'path':'verified','value':expression}]})

    branched = execution_graph()
    branched['pin_data'] = {'start':[{'json':{'id':0,'yes':True}},{'json':{'id':1,'yes':False}},{'json':{'id':2,'yes':True}}]}
    branched['nodes'] = [branched['nodes'][0],condition_node('branch','core.if',rule('={{$json.yes}}')),
        observer('true_observer','={{ $json.yes === true && $("branch").item.json.id === $json.id ? true : (()=>{throw new Error("TRUE_PAIR_CHANGED")})() }}'),
        observer('false_observer','={{ $json.yes === false && $("branch").item.json.id === $json.id ? true : (()=>{throw new Error("FALSE_PAIR_OFFSET_CHANGED")})() }}'),
        condition_node('merge','core.merge',{'mode':'append','wait':'all_inputs','input_count':2}),
        observer('set','={{ JSON.stringify($input.all().map(x=>x.json.id)) === "[0,2,1]" && $("Start").item.json.id === $json.id ? true : (()=>{throw new Error("MERGE_INDEX_ORDER_CHANGED")})() }}')]
    branched['connections'] = [connection('start','branch'),connection('branch','true_observer','true'),connection('branch','false_observer','false'),
        connection('true_observer','merge',input_port='input',input_index=0),connection('false_observer','merge',input_port='input',input_index=1),connection('merge','set')]
    branch_run = start_run(new_runnable(branched))
    await_run(branch_run,'succeeded')
    branch_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + branch_run['id'] + '/node-runs')['items']}
    assert [branch_nodes[n]['items_out'] for n in ('branch','true_observer','false_observer','merge','set')] == [3,2,1,3,3],branch_nodes
    server.kill(); server.wait(timeout=15)
    server = launch('branch-results-restarted')
    assert {node['node_id']:node for node in api(root + '/workflow-executions/' + branch_run['id'] + '/node-runs')['items']} == branch_nodes
    passed('If routes each item once; false-port paired lookup uses exact source offset; append Merge preserves input-index order and all metadata survives hard restart')

    muted_branch = copy.deepcopy(branched)
    muted_branch['nodes'][1]['settings'] = {'mode':'mute'}
    muted_branch['nodes'][1]['params'] = rule('={{ (()=>{throw new Error("MUTED_BRANCH_EVALUATED")})() }}')
    muted_run = start_run(new_runnable(muted_branch))
    await_run(muted_run,'succeeded')
    muted_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + muted_run['id'] + '/node-runs')['items']}
    assert all(muted_nodes[n]['state'] == 'skipped' for n in ('branch','true_observer','false_observer','merge','set')),muted_nodes
    for settings in ({'mode':'bypass'},{'always_output':True}):
        unsupported = copy.deepcopy(branched)
        unsupported['nodes'][1]['settings'] = settings
        unsupported_row = new_runnable(unsupported)
        api(root + '/workflows/' + unsupported_row['id'] + '/executions',dict(request,version=unsupported_row['version']),status=400)
    unsupported = copy.deepcopy(branched)
    unsupported['pin_data']['branch'] = [{'json':{'id':99}}]
    unsupported_row = new_runnable(unsupported)
    api(root + '/workflows/' + unsupported_row['id'] + '/executions',dict(request,version=unsupported_row['version']),status=400)
    passed('muted multi-port node persists all skipped branches without evaluating parameters; undefined pin/bypass/always_output routing is explicitly rejected')

    for wait,expected in [('any_items','succeeded'),('all_inputs','skipped')]:
        single = json.loads(json.dumps(branched))
        single['pin_data']['start'] = [{'json':{'id':0,'yes':True}}]
        single['nodes'][-2]['params']['wait'] = wait
        single['nodes'][-1]['params']['fields'][0]['value'] = '={{ $json.id === 0 ? true : (()=>{throw new Error("MERGE_SELECTED_CHANGED")})() }}'
        single_run = start_run(new_runnable(single))
        await_run(single_run,'succeeded')
        single_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + single_run['id'] + '/node-runs')['items']}
        assert single_nodes['false_observer']['state'] == 'skipped',single_nodes
        assert single_nodes['merge']['state'] == expected,single_nodes
        assert single_nodes['set']['state'] == expected,single_nodes
    passed('Merge waits for terminal branch projections; any_items consumes selected data and all_inputs propagates an explicitly skipped input')

    waiting_merge = copy.deepcopy(branched)
    waiting_merge['nodes'].append(condition_node('branch_wait','core.wait',{'mode':'duration','duration_ms':1500}))
    for edge in waiting_merge['connections']:
        if edge['from']['node'] == 'false_observer' and edge['to']['node'] == 'merge':
            edge['from']['node'] = 'branch_wait'
    waiting_merge['connections'].append(connection('false_observer','branch_wait'))
    waiting_merge['nodes'][-3]['params']['wait'] = 'any_items'
    waiting_merge_run = start_run(new_runnable(waiting_merge))
    await_run(waiting_merge_run,'waiting')
    waiting_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + waiting_merge_run['id'] + '/node-runs')['items']}
    assert 'merge' not in waiting_nodes and waiting_nodes['branch_wait']['state'] == 'waiting',waiting_nodes
    await_run(waiting_merge_run,'succeeded')
    passed('any_items Merge does not race the first nonempty predecessor; it waits for a real durable wait in the other branch to terminate')

    empty_merge = execution_graph()
    empty_merge['nodes'] = [empty_merge['nodes'][0],condition_node('empty','core.filter',rule(False)),
        condition_node('merge','core.merge',{'mode':'append','wait':'all_inputs','input_count':1}),
        observer('set','={{ (()=>{throw new Error("EMPTY_MERGE_EVALUATED_ITEM")})() }}')]
    empty_merge['connections'] = [connection('start','empty'),connection('empty','merge',input_port='input'),connection('merge','set')]
    empty_run = start_run(new_runnable(empty_merge))
    await_run(empty_run,'succeeded')
    empty_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + empty_run['id'] + '/node-runs')['items']}
    assert all(empty_nodes[n]['state'] == 'succeeded' and empty_nodes[n]['items_out'] == 0 for n in ('empty','merge','set')),empty_nodes
    empty_merge['nodes'][-2]['params']['wait'] = 'any_items'
    empty_run = start_run(new_runnable(empty_merge))
    await_run(empty_run,'succeeded')
    empty_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + empty_run['id'] + '/node-runs')['items']}
    assert empty_nodes['empty']['state'] == 'succeeded' and empty_nodes['merge']['state'] == 'skipped',empty_nodes
    passed('executed empty Filter stays distinct from skipped: all_inputs Merge executes empty, any_items Merge skips without inventing data')

    switched = execution_graph()
    switched['pin_data'] = {'start':[{'json':{'id':0}},{'json':{'id':1}},{'json':{'id':2}}]}
    switched['nodes'] = [switched['nodes'][0],condition_node('switch','core.switch',{'cases':[
        {'key':'zero','condition':rule('={{$json.id}}','lt',1)},
        {'key':'one','condition':rule('={{$json.id}}','lt',2)}],'match':'first','fallback':True}),
        observer('zero','={{ $json.id === 0 && $("switch").item.json.id === 0 ? true : (()=>{throw new Error("CASE_ZERO_CHANGED")})() }}'),
        observer('one','={{ $json.id === 1 && $("switch").item.json.id === 1 ? true : (()=>{throw new Error("CASE_ONE_CHANGED")})() }}'),
        observer('fallback','={{ $json.id === 2 && $("switch").item.json.id === 2 ? true : (()=>{throw new Error("FALLBACK_CHANGED")})() }}')]
    switched['connections'] = [connection('start','switch'),connection('switch','zero','case',0),connection('switch','one','case',1),connection('switch','fallback','fallback')]
    switch_run = start_run(new_runnable(switched))
    await_run(switch_run,'succeeded')
    switch_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + switch_run['id'] + '/node-runs')['items']}
    assert all(switch_nodes[n]['items_out'] == 1 for n in ('zero','one','fallback')),switch_nodes
    switched['nodes'][1]['params']['match'] = 'all'
    switched['nodes'][3]['params']['fields'][0]['value'] = '={{ $json.id < 2 && $("switch").item.json.id === $json.id ? true : (()=>{throw new Error("CASE_ALL_PAIR_CHANGED")})() }}'
    switch_run = start_run(new_runnable(switched))
    await_run(switch_run,'succeeded')
    switch_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + switch_run['id'] + '/node-runs')['items']}
    assert [switch_nodes[n]['items_out'] for n in ('switch','zero','one','fallback')] == [4,1,2,1],switch_nodes
    passed('Switch first matching case and fallback have stable declared port order and exact per-port paired provenance')

    invalid_comparison = transform_graph([{'value':1}],[('core.filter',rule('={{$json.value}}','gt','1'))],42)
    invalid_run = start_run(new_runnable(invalid_comparison))
    assert await_run(invalid_run,'failed')['end_reason'] == 'node_failed'
    failure_runs.append(invalid_run)
    invalid_regex = transform_graph([{'value':'hello'}],[('core.filter',rule('={{$json.value}}','matches','['))],42)
    invalid_run = start_run(new_runnable(invalid_regex))
    assert await_run(invalid_run,'failed')['end_reason'] == 'node_failed'
    failure_runs.append(invalid_run)
    bounded_regex = transform_graph([{'value':'a'*40+'!'}],[('core.filter',rule('={{$json.value}}','matches','^(a+)+$'))],42)
    bounded_run = start_run(new_runnable(bounded_regex))
    assert await_run(bounded_run,'failed')['end_reason'] == 'node_failed'
    failure_runs.append(bounded_run)
    assert api('/status')['ok']
    passed('invalid conditional types and regex fail the execution without coercion or server-side regex evaluation')

    failed_history = {}
    for failure_run in failure_runs:
        ended = api(root + '/workflow-executions/' + failure_run['id'])
        attempts = {node['node_id']:node for node in api(root + '/workflow-executions/' + failure_run['id'] + '/node-runs')['items']}
        assert attempts['transform_0']['state'] == 'failed',attempts
        assert attempts['transform_0']['finished_at'] == ended['finished_at'] > 0,attempts
        assert all(node['state'] != 'running' for node in attempts.values()),attempts
        assert attempts['start']['state'] == 'pinned' and 'set' not in attempts,attempts
        failed_history[failure_run['id']] = attempts
    server.kill(); server.wait(timeout=15)
    server = launch('failed-attempts-restarted')
    for identity, attempts in failed_history.items():
        assert api(root + '/workflow-executions/' + identity)['state'] == 'failed'
        assert {node['node_id']:node for node in api(root + '/workflow-executions/' + identity + '/node-runs')['items']} == attempts
    passed('executor failures atomically finish active node attempts with their parent, preserve completed history and remain terminal without replay after hard restart')

    stop_graph = execution_graph()
    stop_graph['nodes'] = [stop_graph['nodes'][0],condition_node('stop','core.stop',{
        'message':'={{ "PRIVATE_STOP_MESSAGE_" + $input.first().json.answer }}','details':{'payload':'PRIVATE_STOP_DETAILS'}})]
    stop_graph['nodes'][1]['settings'] = {'always_output':True}
    stop_graph['connections'] = [connection('start','stop')]
    stop_run = start_run(new_runnable(stop_graph))
    stopped = await_run(stop_run,'failed')
    stop_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + stop_run['id'] + '/node-runs')['items']}
    assert stopped['end_reason'] == 'node_failed' and stop_nodes['stop']['state'] == 'failed',stop_nodes
    assert stop_nodes['stop']['items_out'] == 0 and stop_nodes['stop']['finished_at'] == stopped['finished_at'],stop_nodes
    assert 'PRIVATE_STOP_' not in json.dumps([stopped,stop_nodes])
    stop_history = {stop_run['id']:stop_nodes}
    passed('active Stop resolves private message/details into its attempt, fails node and execution together and never turns always_output into success or leaks private data')

    muted_stop = copy.deepcopy(stop_graph)
    muted_stop['nodes'][1]['settings'] = {'mode':'mute','always_output':True}
    muted_stop['nodes'][1]['params']['message'] = '={{ (()=>{throw new Error("MUTED_STOP_MUST_NOT_EVALUATE")})() }}'
    muted_stop_run = start_run(new_runnable(muted_stop))
    await_run(muted_stop_run,'succeeded')
    stop_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + muted_stop_run['id'] + '/node-runs')['items']}
    assert stop_nodes['stop']['state'] == 'skipped' and stop_nodes['stop']['items_out'] == 0,stop_nodes
    stop_history[muted_stop_run['id']] = stop_nodes

    skipped_stop = execution_graph()
    skipped_stop['nodes'] = [skipped_stop['nodes'][0],condition_node('branch','core.if',rule(True)),
        observer('keep',True),condition_node('stop','core.stop',{'message':'={{ (()=>{throw new Error("UNSELECTED_STOP_MUST_NOT_EVALUATE")})() }}'})]
    skipped_stop['connections'] = [connection('start','branch'),connection('branch','keep','true'),connection('branch','stop','false')]
    skipped_stop_run = start_run(new_runnable(skipped_stop))
    await_run(skipped_stop_run,'succeeded')
    stop_nodes = {node['node_id']:node for node in api(root + '/workflow-executions/' + skipped_stop_run['id'] + '/node-runs')['items']}
    assert stop_nodes['stop']['state'] == 'skipped' and stop_nodes['keep']['state'] == 'succeeded',stop_nodes
    stop_history[skipped_stop_run['id']] = stop_nodes
    passed('muted and unselected Stop nodes persist skipped outputless history without evaluating throwing parameters or synthesizing items')

    server.kill(); server.wait(timeout=15)
    server = launch('stop-history-restarted')
    for identity, attempts in stop_history.items():
        assert {node['node_id']:node for node in api(root + '/workflow-executions/' + identity + '/node-runs')['items']} == attempts
    passed('hard restart reads original Stop failure and outputless skipped snapshots without replay or fabricated output ports')

    # Earlier run-revocation checks intentionally left this role read/run-only.
    transfer_role = api(root + '/roles/' + role)
    api(root + '/roles/' + role,{'version':transfer_role['version'],'name':transfer_role['name'],
        'permissions':sorted(set(transfer_role['permissions']) | {'workflows.update','workflows.create'})},method='PATCH')
    transfer_row = new_runnable(execution_graph())
    transfer_path = root + '/workflows/' + transfer_row['id']
    transfer_api = transfer_path + '/owner-transfers'
    transfer_body = {'owner_id':member['user_id'],'version':transfer_row['version']}
    transfer_revisions = api(transfer_path + '/revisions')
    def transfer_audit():
        return [item for item in api(root + '/activity?action=transfer&resource=workflows&page_size=200')['items']
            if item['resource_id'] == transfer_row['id']]
    api(transfer_api,transfer_body,actor=reader,status=403)
    api(transfer_api,transfer_body,status=403)  # Workspace edit alone is insufficient for recipient.
    api(transfer_api,dict(transfer_body,owner_id='9223372036854775807'),status=400)
    foreign_recipient = client()
    foreign_recipient_id = api('/users',{'name':'Foreign recipient','email':'foreign-recipient@example.invalid',
        'password':'temporary-foreign-recipient-password'},actor=foreign_recipient)['id']
    api('/workspace-memberships',{'code':api(f'/workspaces/{other}/members/invitation')['code']},actor=foreign_recipient)
    api(transfer_api,dict(transfer_body,owner_id=foreign_recipient_id),status=400)
    api(transfer_api,dict(transfer_body,owner_id=outsider_id),status=403)  # Inactive account.
    api(transfer_api,dict(transfer_body,workspace_id=other),status=422)
    api(f'/workspaces/{other}/workflows/{transfer_row["id"]}/owner-transfers',transfer_body,status=404)
    assert api(transfer_path) == transfer_row and not transfer_audit()
    passed('ownership transfer rejects non-owner callers, recipients without object edit, inactive or absent members and scope injection without mutations or transfer audit')

    transfer_grant = api(transfer_path + '/acl',{'user_id':member['user_id'],'role':'all'},status=201)
    api(transfer_api,transfer_body,actor=reader,status=403)  # all still cannot transfer ownership.
    role_before = api(root + '/roles/' + role)
    api(root + '/roles/' + role,{'version':role_before['version'],'name':role_before['name'],
        'permissions':[p for p in role_before['permissions'] if p != 'workflows.update']},method='PATCH')
    api(transfer_api,transfer_body,status=403)
    role_now = api(root + '/roles/' + role)
    api(root + '/roles/' + role,{'version':role_now['version'],'name':role_before['name'],
        'permissions':role_before['permissions']},method='PATCH')
    api(transfer_api,dict(transfer_body,version=transfer_body['version']-1),status=409)
    assert api(transfer_path) == transfer_row and not transfer_audit()
    passed('ownership transfer requires current owner and live workspace editing permission even with object all; stale versions leave owner and audit unchanged')

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        transfer_results = [future.result() for future in [pool.submit(api,transfer_api,
            transfer_body,status=(200,403,409)) for _ in range(2)]]
    assert sorted(code for code,_ in transfer_results) == [200,403],transfer_results
    transferred = api(transfer_path,actor=reader)
    assert transferred['owner_id'] == member['user_id'] and transferred['version'] == transfer_row['version']+1
    for key in ('definition','draft_revision_id','published_revision_id','trigger_run_as','created_by','created_at'):
        assert transferred[key] == transfer_row[key],key
    assert api(transfer_path + '/revisions') == transfer_revisions
    audit = transfer_audit()
    assert len(audit) == 1 and audit[0]['details'] == {'old_owner_id':transfer_row['owner_id'],
        'new_owner_id':member['user_id'],'version':transferred['version']},audit
    passed('competing ownership transfers commit exactly one owner change and one old/new-owner audit while preserving revision and production authority')

    api(transfer_api,{'owner_id':transfer_row['owner_id'],'version':transferred['version']},status=403)
    api(transfer_path + '/acl/' + transfer_grant['id'] + '?version=1',method='DELETE',actor=reader,status=204)
    forged = api(transfer_path,{'version':transferred['version'],'name':'Transferred owner',
        'owner_id':transfer_row['owner_id']},method='PATCH',actor=reader)
    assert forged['owner_id'] == member['user_id']
    assert not api(transfer_path + '/acl',actor=reader)['items']
    server.kill(); server.wait(timeout=15)
    server = launch('ownership-restarted')
    assert api(transfer_path,actor=reader) == forged and transfer_audit() == audit
    passed('new owner retains edit after removing its grant; old superadmin owner cannot transfer, ordinary writes cannot inject owner, and ownership and audit survive hard restart')

    relinquished = api(root + '/workflows',{'name':'Relinquished owner','definition':json.dumps(graph())},
        actor=reader,status=201)
    relinquished_path = root + '/workflows/' + relinquished['id']
    relinquished = api(relinquished_path + '/owner-transfers',
        {'owner_id':transfer_row['owner_id'],'version':relinquished['version']},actor=reader)
    assert relinquished['owner_id'] == transfer_row['owner_id']
    api(relinquished_path,actor=reader,status=403)
    api(relinquished_path + '/owner-transfers',
        {'owner_id':member['user_id'],'version':relinquished['version']},actor=reader,status=403)
    passed('ordinary former owner immediately loses ungranted access after transfer and receives no implicit retained privileges')

    publication_graph = execution_graph(4000)
    publication_graph['nodes'].append(observer('publication_check',
        '={{ $json.answer === 42 ? true : (()=>{throw new Error("PUBLISHED_REVISION_CHANGED")})() }}'))
    publication_graph['connections'].append(connection('set','publication_check'))
    publication_row = new_runnable(publication_graph)
    publication_path = root + '/workflows/' + publication_row['id']
    publications_path = publication_path + '/publications'
    def publish_body(name, **extra):
        row = api(publication_path)
        return dict(version=row['version'],revision_id=row['draft_revision_id'],name=name,**extra)
    for bad in ('',' '*3,'x'*65):
        api(publications_path,publish_body(bad),status=400)
    api(publications_path,publish_body('note-too-long',change_note='x'*201),status=400)
    for bad in ('','x'*65):
        api(publications_path,publish_body('bad-key',request_key=bad),status=400)
    api(publications_path,dict(publish_body('injected'),run_as=member['user_id']),status=422)
    api(publications_path,actor=reader,status=403)
    api(f'/workspaces/{other}/workflows/{publication_row["id"]}/publications',status=404)
    assert api(publication_path) == publication_row and api(publications_path)['total'] == 0
    publication_errors = []
    for locale in ('en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'):
        error = api(publications_path,publish_body(''),status=400,locale=locale)['error']
        assert 'workflow.' not in error
        publication_errors.append(error)
    assert len(set(publication_errors)) == 6
    passed('publication metadata enforces trimmed names, note and request-key limits, workspace/object visibility and server-owned publisher identity without side effects')

    request_a = publish_body(' A ',change_note='n'*200,request_key='publication-a')
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        publication_results = [future.result() for future in [pool.submit(api,publications_path,
            request_a,status=(200,201)) for _ in range(2)]]
    assert sorted(code for code,_ in publication_results) == [200,201],publication_results
    publication_a = publication_results[0][1]
    assert publication_results[1][1] == publication_a and publication_a['name'] == 'A'
    assert publication_a['run_as'] == publication_row['owner_id']
    published_a = api(publication_path)
    assert published_a['published_revision_id'] == publication_row['draft_revision_id']
    assert published_a['trigger_run_as'] == publication_row['owner_id']
    assert api(publications_path,request_a) == publication_a
    api(publications_path,dict(request_a,name='changed-payload'),status=409)
    api(publications_path,publish_body('A'),status=409)
    lower_case = api(publications_path,publish_body('a'),status=201)
    assert lower_case['id'] != publication_a['id'] and api(publications_path)['total'] == 2
    limit_row = new_runnable(execution_graph())
    limit_path = root + '/workflows/' + limit_row['id']
    limit_publication = api(limit_path + '/publications',{'version':limit_row['version'],
        'revision_id':limit_row['draft_revision_id'],'name':'界'*64,'change_note':''},status=201)
    assert limit_publication['name'] == '界'*64 and limit_publication['change_note'] == ''
    limit_row = api(limit_path)
    api(limit_path + '/publications',{'version':limit_row['version'],
        'revision_id':limit_row['draft_revision_id'],'name':'A'},status=201)
    passed('publication request races create one immutable record and replay exactly; changed payload and duplicate names conflict while case-sensitive names remain distinct')

    pub_run = start_run(api(publication_path),revision='published')
    await_run(pub_run,'waiting')
    published_draft = copy.deepcopy(publication_graph)
    published_draft['nodes'][2]['params']['fields'][0]['value'] = 99
    published_draft['nodes'][3]['params']['fields'][0]['value'] = '={{ $json.answer === 99 ? true : (()=>{throw new Error("DRAFT_REVISION_CHANGED")})() }}'
    saved_b = api(publication_path,{'version':api(publication_path)['version'],
        'definition':json.dumps(published_draft)},method='PATCH')
    assert saved_b['published_revision_id'] == publication_a['revision_id'] and saved_b['trigger_run_as'] == publication_a['run_as']
    assert saved_b['draft_revision_id'] != publication_a['revision_id']
    assert api(publications_path)['total'] == 2
    publication_grant = api(publication_path + '/acl',{'user_id':member['user_id'],'role':'all'},status=201)
    request_b = publish_body('B',request_key='publication-b')
    api(publications_path,request_b,actor=reader,status=409)
    api(publications_path,dict(request_b,previous_run_as='9223372036854775807'),actor=reader,status=409)
    publication_b = api(publications_path,dict(request_b,previous_run_as=publication_a['run_as']),actor=reader,status=201)
    assert publication_b['run_as'] == member['user_id']
    published_b = api(publication_path)
    assert published_b['published_revision_id'] == saved_b['draft_revision_id'] and published_b['trigger_run_as'] == member['user_id']
    old_run = await_run(pub_run,'succeeded')
    assert old_run['revision_id'] == publication_a['revision_id'] and old_run['triggered_by'] == publication_a['run_as']
    b_run = start_run(published_b,actor=reader,revision='published')
    assert await_run(b_run,'succeeded')['revision_id'] == publication_b['revision_id']
    passed('draft saves preserve production revision and identity; another publisher must confirm the exact previous identity and in-flight old revisions remain immutable')

    published_history = api(publications_path)
    publication_role = api(root + '/roles/' + role)
    api(root + '/roles/' + role,{'version':publication_role['version'],'name':publication_role['name'],
        'permissions':[p for p in publication_role['permissions'] if p != 'workflows.run']},method='PATCH')
    api(publications_path,dict(request_b,previous_run_as=publication_a['run_as']),actor=reader,status=403)
    current_role = api(root + '/roles/' + role)
    api(root + '/roles/' + role,{'version':current_role['version'],'name':publication_role['name'],
        'permissions':publication_role['permissions']},method='PATCH')
    assert api(publications_path) == published_history
    passed('publication idempotency retries still recheck live publisher authority and cannot bypass revoked run permission')

    def publication_sql(statement):
        if postgres:
            pg_sql(statement)
        elif mysqld:
            result = subprocess.run([str(mysqld.with_name('mysql.exe')),'--no-defaults',
                '--protocol=TCP','--host=127.0.0.1',f'--port={mysql_port}','--user=root',
                '--database=workflow_drafts','--execute',statement],stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                timeout=30,creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
            assert result.returncode == 0,result.stderr.decode(errors='replace')
        else:
            import sqlite3
            with sqlite3.connect(work / 'server/ferry.sqlite') as connection_db:
                connection_db.execute(statement)
    before_fault = api(publication_path)
    if postgres:
        publication_sql("CREATE FUNCTION reject_publication_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'publication audit fault'; END $$; CREATE TRIGGER reject_publication BEFORE INSERT ON activity FOR EACH ROW EXECUTE FUNCTION reject_publication_audit()")
    elif mysqld:
        publication_sql("CREATE TRIGGER reject_publication BEFORE INSERT ON activity FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='publication audit fault'")
    else:
        publication_sql("CREATE TRIGGER reject_publication BEFORE INSERT ON activity WHEN NEW.action='publish' BEGIN SELECT RAISE(ABORT,'publication audit fault'); END")
    try:
        api(publications_path,publish_body('rollback',previous_run_as=member['user_id']),status=500)
    finally:
        if postgres:
            publication_sql('DROP TRIGGER reject_publication ON activity; DROP FUNCTION reject_publication_audit()')
        else:
            publication_sql('DROP TRIGGER reject_publication')
    assert api(publication_path) == before_fault and api(publications_path) == published_history
    passed('real database audit failures roll back publication ledger, production pointer, identity and version in the same transaction')

    unsupported_publication = execution_graph()
    unsupported_publication['nodes'][1] = condition_node('set','core.aggregate',{'mode':'collect','group_by':[]})
    unsupported_publication_row = new_runnable(unsupported_publication)
    unsupported_publication_path = root + '/workflows/' + unsupported_publication_row['id']
    api(unsupported_publication_path + '/publications',{'version':unsupported_publication_row['version'],
        'revision_id':unsupported_publication_row['draft_revision_id'],'name':'unsupported'},status=400)
    assert api(unsupported_publication_path) == unsupported_publication_row
    assert api(unsupported_publication_path + '/publications')['total'] == 0
    throwing_publication = execution_graph()
    throwing_publication['nodes'][1]['params']['fields'][0]['value'] = '={{ (()=>{throw new Error("PRIVATE_PUBLISH_NO_EVAL")})() }}'
    throwing_row = new_runnable(throwing_publication)
    throwing_path = root + '/workflows/' + throwing_row['id']
    api(throwing_path + '/publications',{'version':throwing_row['version'],
        'revision_id':throwing_row['draft_revision_id'],'name':'No evaluation'},status=201)
    passed('publication rejects unavailable runtime nodes without production mutations and syntax-checks expressions without executing tenant code')

    api(publication_path + '/unpublication',{'version':api(publication_path)['version'],
        'run_as':member['user_id']},status=422)
    withdrawn = api(publication_path + '/unpublication',{'version':api(publication_path)['version']})
    assert withdrawn['published_revision_id'] is None and withdrawn['trigger_run_as'] == member['user_id']
    assert api(publications_path,dict(request_b,previous_run_as=publication_a['run_as']),actor=reader) == publication_b
    assert api(publication_path) == withdrawn  # Retry must not silently republish.
    start_run_status = dict(mode='manual',revision='published',version=withdrawn['version'],entry_node_id='start',inputs={'answer':41})
    api(publication_path + '/executions',start_run_status,status=400)
    archived_publication = api(publication_path,{'version':withdrawn['version'],'lifecycle':'archived'},method='PATCH')
    api(publications_path,publish_body('archived',previous_run_as=member['user_id']),status=409)
    restored_publication = api(publication_path,{'version':archived_publication['version'],'lifecycle':'active'},method='PATCH')
    assert restored_publication['published_revision_id'] is None
    server.kill(); server.wait(timeout=15)
    server = launch('publication-restarted')
    assert api(publication_path) == restored_publication and api(publications_path) == published_history
    passed('unpublication and archive stop published entry selection; retries never republish and hard restart preserves immutable ledger and withdrawn production pointer')

    # Pipeline object authorization uses the same grants, with disjoint kinds.
    pipeline_role_before = api(root + '/roles/' + role)
    pipeline_permissions = sorted(set(pipeline_role_before['permissions'] +
        ['pipelines.read','pipelines.run','pipelines.create','pipelines.update','pipelines.delete',
         'pipeline-groups.read','pipeline-groups.update']))
    api(root + '/roles/' + role, {'version':pipeline_role_before['version'],
        'name':pipeline_role_before['name'],'permissions':pipeline_permissions}, method='PATCH')
    pipeline_body = {'name':'Pipeline ACL primary','definition_format':'github-actions',
        'definition':json.dumps({'jobs':{'gate':{'ferry-approval':True}}}),
        'group_id':pipeline_group,'owner_id':member['user_id']}
    pipeline_id = api(root + '/pipelines',pipeline_body,status=201)['id']
    pipeline_path = root + '/pipelines/' + pipeline_id
    pipeline_row = api(pipeline_path)
    assert pipeline_row['owner_id'] == created['owner_id']
    pipeline_revision = api(pipeline_path + '/revisions')['items'][0]
    assert api(root + '/pipelines',actor=reader)['total'] == 0
    for suffix in ('','/revisions','/revisions/'+pipeline_revision['id']):
        api(pipeline_path+suffix,actor=reader,status=403)
    api(pipeline_path+'/runs',{},method='POST',actor=reader,status=403)
    api(pipeline_path,dict(pipeline_body,version=pipeline_row['version']),method='PATCH',actor=reader,status=403)
    api(pipeline_path,method='DELETE',actor=reader,status=403)
    api(pipeline_path+'/acl',actor=reader,status=403)
    passed('pipeline creation records caller ownership and workspace permissions alone expose no object, history, edit, delete or run access')

    pipeline_view = api(pipeline_path+'/acl',{'user_id':member['user_id'],'role':'view'},status=201)
    assert api(pipeline_path,actor=reader) == pipeline_row
    assert api(pipeline_path+'/revisions/'+pipeline_revision['id'],actor=reader) == pipeline_revision
    api(pipeline_path+'/runs',{},method='POST',actor=reader,status=403)
    pipeline_group_path = root+'/pipeline-groups/'+pipeline_group
    api(pipeline_group_path+'/acl',{'user_id':member['user_id'],'role':'run'},actor=reader,status=403)
    pipeline_group_grant = api(pipeline_group_path+'/acl',{'user_id':member['user_id'],'role':'run'},status=201)
    pipeline_run = api(pipeline_path+'/runs',{},method='POST',actor=reader,status=201)
    pipeline_run_path = root+'/pipeline-runs/'+pipeline_run['id']
    assert api(pipeline_run_path,actor=reader)['pipeline_id'] == pipeline_id
    pipeline_artifact_id = '900000000000001'
    publication_sql(f"INSERT INTO pipeline_artifacts (id,workspace_id,pipeline_id,run_id,path,sha256,size,mode,created_at,expires_at) VALUES ({pipeline_artifact_id},{workspace},{pipeline_id},{pipeline_run['id']},'acl-fixture','abc',1,420,1,0)")
    assert api(root+'/pipeline-artifacts?pipeline_id='+pipeline_id,actor=reader)['total'] == 1
    assert api(root+'/pipeline-runs?pipeline_id='+pipeline_id,actor=reader)['total'] == 1
    api(pipeline_run_path+'/logs',actor=reader)
    api(pipeline_path,dict(pipeline_body,version=pipeline_row['version'],group_id=''),method='PATCH')
    api(pipeline_path+'/runs',{},method='POST',actor=reader,status=403)
    assert api(pipeline_run_path,actor=reader)['pipeline_id'] == pipeline_id
    api(pipeline_path+'/acl/'+pipeline_view['id']+'?version=1',method='DELETE',status=204)
    api(pipeline_run_path,actor=reader,status=403)
    api(pipeline_run_path+'/logs',actor=reader,status=403)
    assert api(root+'/pipeline-artifacts?pipeline_id='+pipeline_id,actor=reader)['total'] == 0
    api(root+'/pipeline-artifacts/'+pipeline_artifact_id+'/content',actor=reader,status=403)
    api(pipeline_run_path+'/cancellations',method='POST',actor=reader,status=403)
    api(pipeline_run_path+'/jobs/gate/approvals',method='POST',actor=reader,status=403)
    assert api(root+'/pipeline-runs?pipeline_id='+pipeline_id,actor=reader)['total'] == 0
    api(pipeline_group_path+'/acl/'+pipeline_group_grant['id']+'?version=1',method='DELETE',status=204)
    passed('pipeline group Run combines with direct View; moving groups and revoking grants immediately removes trigger, history, logs and run actions')

    pipeline_bulk=[]
    for index in range(5):
        item=api(root+'/pipelines',dict(pipeline_body,name=f'Pipeline ACL page {index}',group_id=''),status=201)
        pipeline_bulk.append(item['id'])
        if index>=2:
            for access_role in ('view','run'):
                api(root+'/pipelines/'+item['id']+'/acl',{'user_id':member['user_id'],'role':access_role},status=201)
    ppage=api(root+'/pipelines?q=Pipeline%20ACL%20page&sort=name&page_size=2',actor=reader)
    assert ppage['total']==3 and [item['id'] for item in ppage['items']]==pipeline_bulk[2:4]
    ppage=api(root+'/pipelines?q=Pipeline%20ACL%20page&sort=name&page_size=2&page=2',actor=reader)
    assert ppage['total']==3 and [item['id'] for item in ppage['items']]==pipeline_bulk[4:]
    passed('pipeline ACL is applied before page/count and overlapping direct grants never duplicate rows')

    pipeline_all=api(pipeline_path+'/acl',{'user_id':member['user_id'],'role':'all'},status=201)
    pipeline_row=api(pipeline_path)
    transfer_body={'owner_id':member['user_id'],'version':pipeline_row['version']}
    api(pipeline_path+'/owner-transfers',dict(transfer_body,run_as=member['user_id']),status=422)
    api(pipeline_path+'/owner-transfers',transfer_body,actor=reader,status=403)
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        transfers=[future.result() for future in [pool.submit(api,pipeline_path+'/owner-transfers',transfer_body,None,(200,403,409)) for _ in range(2)]]
    assert sorted(code for code,_ in transfers)==[200,403]
    transferred_pipeline=api(pipeline_path,actor=reader)
    assert transferred_pipeline['owner_id']==member['user_id'] and transferred_pipeline['version']==pipeline_row['version']+1
    pipeline_transfer_audit=[item for item in api(root+'/activity?action=transfer&resource=pipelines&page_size=200')['items'] if item['resource_id']==pipeline_id]
    assert len(pipeline_transfer_audit)==1 and pipeline_transfer_audit[0]['details']=={'old_owner_id':pipeline_row['owner_id'],'new_owner_id':member['user_id'],'version':transferred_pipeline['version']}
    assert api(pipeline_path+'/revisions')['items']==[pipeline_revision]
    api(pipeline_path+'/owner-transfers',{'owner_id':created['owner_id'],'version':transferred_pipeline['version']},status=403)
    api(pipeline_path+'/acl/'+pipeline_all['id']+'?version=1',method='DELETE',status=204,actor=reader)
    assert api(pipeline_path,actor=reader)['owner_id']==member['user_id']
    api(pipeline_path,dict(pipeline_body,version=transferred_pipeline['version'],owner_id=created['owner_id']),method='PATCH',actor=reader)
    assert api(pipeline_path,actor=reader)['owner_id']==member['user_id']
    server.kill(); server.wait(timeout=15)
    server=launch('pipeline-acl-restarted')
    assert api(pipeline_path,actor=reader)['owner_id']==member['user_id']
    pipeline_role_now=api(root+'/roles/'+role)
    api(root+'/roles/'+role,{'version':pipeline_role_now['version'],'name':pipeline_role_now['name'],
        'permissions':[item for item in pipeline_permissions if item!='pipelines.update']},method='PATCH')
    api(pipeline_path,dict(pipeline_body,version=api(pipeline_path)['version']),method='PATCH',actor=reader,status=403)
    api(pipeline_path+'/owner-transfers',{'owner_id':created['owner_id'],'version':api(pipeline_path)['version']},actor=reader,status=403)
    assert api(pipeline_path,actor=reader)['owner_id']==member['user_id']
    passed('pipeline owner transfer enforces active authorized recipient and current-owner CAS, preserves revisions and survives restart without owner injection')

    api(root + '/members/' + member['id'], method='DELETE', status=204)
    api(transfer_path,actor=reader,status=404)
    api(pipeline_path,actor=reader,status=404)
    api(pipeline_run_path,actor=reader,status=404)
    for suffix in ('', '/revisions', '/revisions/' + first['id']):
        api(path + suffix, actor=reader, status=404)
    api(owned_path, actor=reader, status=404)
    api(path + '/acl', {'user_id': member['user_id'], 'role': 'view'}, status=400)
    passed('removing membership revokes both granted and owned definitions, including immutable history')
    if len(sys.argv) > 3 and sys.argv[3] == '--journal':
        assert mysqld is not None
        journal_env = dict(env, FERRY_TEST_JOURNAL_MYSQL_ROOT_URL=f'mysql://root@127.0.0.1:{mysql_port}/')
        with (work / 'journal-mysql.log').open('wb') as journal_log:
            result = subprocess.run(['cargo','test','-p','ferry-server','workflows::journal::tests','--',
                '--nocapture','--skip','expiry_between_attempt_insert_and_final_parent_write'],
                cwd=Path(__file__).resolve().parents[1],env=journal_env,stdout=journal_log,stderr=journal_log,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        assert result.returncode == 0, ('MySQL node journal tests failed',work / 'journal-mysql.log')
        passed('isolated real MySQL passes portable node journal recovery, fencing, wait and contention tests; SQLite trigger injection is excluded explicitly')
        with (work / 'run-mysql.log').open('wb') as run_log:
            result = subprocess.run(['cargo','test','-p','ferry-server','workflows::run::tests','--',
                '--nocapture','--skip','failure_attempt_update_crossing_expiry_rolls_back_parent_and_child'],
                cwd=Path(__file__).resolve().parents[1],env=journal_env,stdout=run_log,stderr=run_log,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        assert result.returncode == 0, ('MySQL execution terminal tests failed',work / 'run-mysql.log')
        passed('isolated real MySQL passes active-attempt terminal atomicity, stale lease rejection, history preservation and version overflow rollback; SQLite fault trigger is excluded explicitly')
        with (work / 'content-assets-mysql.log').open('wb') as asset_log:
            result = subprocess.run(['cargo','test','-p','ferry-server','content_assets::tests','--','--nocapture'],
                cwd=Path(__file__).resolve().parents[1],env=journal_env,stdout=asset_log,stderr=asset_log,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        assert result.returncode == 0, ('MySQL content asset tests failed',work / 'content-assets-mysql.log')
        passed('isolated real MySQL passes asset quota, dedup, reference transactions, GC races and delete recovery')
    (work / 'results.json').write_text(json.dumps({'result': 'PASS', 'backend': 'mysql' if mysqld else 'postgres' if postgres else 'sqlite', 'checks': checks}, indent=2), encoding='utf-8')
    print(f'PASS: {len(checks)} Workflow draft acceptance checks', flush=True)
finally:
    for child in reversed(children):
        if postgres and child.args[0] == str(postgres):
            stopped = pg_command('pg_ctl.exe','stop','-D',work/'postgres','-m','fast','-t','20')
            assert stopped.returncode == 0,stopped.stderr.decode(errors='replace')
        if mysqld and child.args[0] == str(mysqld):
            # A Windows monitor parent may outlive or detach the real server.
            subprocess.run([str(mysqld.with_name('mysqladmin.exe')), '--no-defaults',
                '--protocol=TCP', '--host=127.0.0.1', f'--port={mysql_port}', '--user=root', 'shutdown'],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill(); child.wait(timeout=15)
    for log in logs:
        log.close()
    print('Workflow draft evidence:', work, flush=True)
    if mysqld and any(child.args[0] == str(mysqld) for child in children):
        try:
            with socket.create_connection(('127.0.0.1', mysql_port), timeout=1):
                raise AssertionError('isolated MySQL still listens after cleanup')
        except OSError:
            pass
    if postgres and any(child.args[0] == str(postgres) for child in children):
        try:
            with socket.create_connection(('127.0.0.1',postgres_port),timeout=1):
                raise AssertionError('isolated PostgreSQL still listens after cleanup')
        except OSError:
            pass
