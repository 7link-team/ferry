#!/usr/bin/env python3
"""Upgrade an isolated real server from before Workflow storage, retaining data."""
import hashlib
import contextlib
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

before, after = (Path(arg).resolve() for arg in sys.argv[1:3])
mysqld = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else None
work = Path(tempfile.mkdtemp(prefix='workflow-storage-upgrade-'))
children, logs, checks = [], [], []
with socket.socket() as probe:
    probe.bind(('127.0.0.1', 0))
    port = probe.getsockname()[1]
origin = f'http://127.0.0.1:{port}'
env = dict(os.environ, FERRY_DATA_DIR=str(work / 'server'), FERRY_BIND=f'127.0.0.1:{port}')
for key in ('FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE'):
    env.pop(key, None)
client = urllib.request.build_opener(urllib.request.ProxyHandler({}),
    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))


def api(path, body=None, status=200):
    request = urllib.request.Request(origin + '/api' + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
    try:
        response = client.open(request, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    raw = response.read()
    assert response.code == status, (path, response.code, status, raw)
    return json.loads(raw) if raw else None


def launch(binary, name):
    log = open(work / (name + '.log'), 'ab')
    logs.append(log)
    child = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    children.append(child)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        assert child.poll() is None, (name, 'server exited', child.returncode)
        try:
            if api('/status')['ok']:
                return child
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise AssertionError(name + ' startup timeout')


def stop(child):
    child.terminate()
    child.wait(timeout=15)


def passed(label):
    checks.append(label)
    print('PASS', label, flush=True)


def mysql_query(statement):
    result = subprocess.run([str(mysqld.with_name('mysql.exe')), '--no-defaults',
        '--protocol=TCP', '--host=127.0.0.1', f'--port={mysql_port}', '--user=root',
        '--default-character-set=utf8mb4', '--xml', '--execute=' + statement],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    assert result.returncode == 0, result.stderr.decode(errors='replace')
    return result.stdout


class MysqlCursor:
    def __init__(self, data):
        self.rows = []
        for row in ET.fromstring(data).findall('row'):
            values = []
            for field in row.findall('field'):
                value = field.text
                if value is not None and value.lstrip('-').isdigit():
                    value = int(value)
                values.append(value)
            self.rows.append(tuple(values))

    def fetchall(self):
        return self.rows

    def fetchone(self):
        return self.rows[0] if self.rows else None


class MysqlConnection:
    def execute(self, statement, parameters=()):
        for value in parameters:
            # Only application-generated numeric workspace IDs are bound here.
            statement = statement.replace('?', str(int(value)), 1)
        return MysqlCursor(mysql_query('USE workflow_storage; ' + statement))


@contextlib.contextmanager
def database():
    # Inspect only; the application remains the sole business-data writer.
    if mysqld:
        yield MysqlConnection()
    else:
        with sqlite3.connect((work / 'server' / 'ferry.sqlite').as_uri() + '?mode=ro', uri=True) as db:
            yield db


def tables(db):
    query = ('SELECT table_name FROM information_schema.tables WHERE table_schema=DATABASE()'
        if mysqld else 'SELECT name FROM sqlite_master WHERE type="table"')
    return {row[0] for row in db.execute(query).fetchall()}


def workspace_columns(db):
    if mysqld:
        return {row[0] for row in db.execute("SELECT column_name FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='workspaces'").fetchall()}
    return {row[1] for row in db.execute('PRAGMA table_info(workspaces)').fetchall()}


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
        try:
            mysql_query('SELECT 1')
            mysql_query('CREATE DATABASE workflow_storage CHARACTER SET utf8mb4 COLLATE utf8mb4_bin')
            env['FERRY_DATABASE_URL'] = f'mysql://root@127.0.0.1:{mysql_port}/workflow_storage'
            return
        except AssertionError:
            time.sleep(0.1)
    raise AssertionError('isolated MySQL startup timeout')


try:
    if mysqld:
        start_mysql()
    old = launch(before, 'before')
    api('/setup', {'name': 'Upgrade operator', 'email': 'upgrade@example.invalid',
        'password': 'temporary-upgrade-password',
        'token': (work / 'server' / 'setup-token').read_text().strip()})
    workspace = api('/workspaces', {'name': 'Before upgrade', 'slug': 'before-upgrade'})['id']
    root = f'/workspaces/{workspace}'
    host = api(root + '/nodes', {'name': 'Upgrade host', 'connection': 'runner'}, status=201)['id']
    pipeline = api(root + '/pipelines', {'name': 'Preserved pipeline',
        'definition_format': 'github-actions',
        'definition': 'on: workflow_dispatch\njobs:\n  work:\n    runs-on: self-hosted\n    steps:\n      - run: echo PRESERVED\n',
        'executor_bindings': [{'job_id': 'work', 'target_kind': 'host', 'target_ref': host, 'image': 'host'}]}, status=201)['id']
    path = root + '/pipelines/' + pipeline
    saved = api(path)
    history = api(path + '/revisions')
    stop(old)
    with database() as db:
        names = tables(db)
        assert not {'workflows', 'workflow_executions', 'workflow_node_runs'} & names
        assert 'workflow_execution_number' not in workspace_columns(db)
        old_workspace = db.execute('SELECT id,slug,name,owner_id,pipeline_run_number FROM workspaces WHERE id=?', (workspace,)).fetchone()
    passed('baseline binary has no Workflow tables or execution counter')
    current = launch(after, 'after')
    assert api(path) == saved and api(path + '/revisions') == history
    with database() as db:
        names = tables(db)
        assert {'workflows', 'workflow_executions', 'workflow_node_runs'} <= names
        assert db.execute('SELECT id,slug,name,owner_id,pipeline_run_number FROM workspaces WHERE id=?', (workspace,)).fetchone() == old_workspace
        assert db.execute('SELECT workflow_execution_number FROM workspaces WHERE id=?', (workspace,)).fetchone() == (0,)
        for table in ('workflows', 'workflow_executions', 'workflow_node_runs'):
            assert db.execute('SELECT count(*) FROM ' + table).fetchone() == (0,)
    passed('real startup migrates existing data without inventing workflows or changing pipeline history')
    created = api('/workspaces', {'name': 'After upgrade', 'slug': 'after-upgrade'})['id']
    with database() as db:
        assert db.execute('SELECT workflow_execution_number FROM workspaces WHERE id=?', (created,)).fetchone() == (0,)
        migrations = db.execute('SELECT version FROM seaql_migrations ORDER BY version').fetchall()
        assert sum(row[0] == 'workflow_storage_v1' for row in migrations) == 1
    passed('workspace creation after upgrade initializes the new counter independently')
    stop(current)
    current = launch(after, 'restarted')
    assert api(path) == saved and api(path + '/revisions') == history
    with database() as db:
        assert db.execute('SELECT version FROM seaql_migrations ORDER BY version').fetchall() == migrations
        assert db.execute('SELECT count(*) FROM workspaces WHERE workflow_execution_number != 0').fetchone() == (0,)
        if mysqld:
            ddl = [db.execute('SHOW CREATE TABLE ' + name).fetchone()
                for name in ('workflows', 'workflow_executions', 'workflow_node_runs')]
        else:
            ddl = db.execute('SELECT type,name,sql FROM sqlite_master WHERE tbl_name IN ("workflows","workflow_executions","workflow_node_runs") ORDER BY type,name').fetchall()
    passed('second startup preserves history and records the storage migration exactly once')
    result = {'result': 'PASS', 'backend': 'mysql' if mysqld else 'sqlite', 'checks': checks, 'evidence': str(work), 'schema': ddl,
        'artifacts': {str(binary): hashlib.sha256(binary.read_bytes()).hexdigest() for binary in (before, after)}}
    (work / 'results.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(f'PASS: {len(checks)} real Workflow storage upgrade checks', flush=True)
finally:
    for child in reversed(children):
        if mysqld and child.args[0] == str(mysqld):
            # Windows mysqld can have a monitor parent and a server child.
            # Terminating only Popen's parent leaves the server listening.
            subprocess.run([str(mysqld.with_name('mysqladmin.exe')), '--no-defaults',
                '--protocol=TCP', '--host=127.0.0.1', f'--port={mysql_port}',
                '--user=root', 'shutdown'], stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL, timeout=20,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=15)
    for log in logs:
        log.close()
    print('Workflow storage upgrade evidence:', work, flush=True)
    if mysqld and any(child.args[0] == str(mysqld) for child in children):
        try:
            with socket.create_connection(('127.0.0.1', mysql_port), timeout=1):
                raise AssertionError('isolated MySQL still listens after cleanup')
        except OSError:
            pass
