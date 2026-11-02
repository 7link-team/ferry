#!/usr/bin/env python3
"""Real previous-version dictionary/settings upgrade. Pass OLD_SERVER NEW_SERVER.

FERRY_TEST_DATABASE_URL optionally names an empty disposable MySQL/Postgres DB.
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
import urllib.parse
import urllib.request


old, new = (str(Path(binary).resolve()) for binary in sys.argv[1:])
with tempfile.TemporaryDirectory(prefix='typed-maintenance-upgrade-') as directory:
    data = Path(directory)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}/api'
    env = dict(os.environ, FERRY_DATA_DIR=directory, FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    if os.environ.get('FERRY_TEST_DATABASE_URL'):
        env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}),
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def request(path, body=None, method=None, expected=200):
        req = urllib.request.Request(origin+path,
            data=json.dumps(body).encode() if body is not None else None,
            method=method, headers={'Content-Type':'application/json','X-Ferry-Request':'1', 'Accept-Language': 'zh-CN'})
        try:
            response = client.open(req, timeout=30)
        except urllib.error.HTTPError as error:
            response = error
        raw = response.read()
        result = json.loads(raw) if raw else None
        assert response.code == expected, (path, response.code, result)
        return result

    with open(data/'server.log', 'wb') as log:
        def launch(binary):
            process = subprocess.Popen([binary], env=env, stdout=log, stderr=log,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
            for _ in range(200):
                if process.poll() is not None:
                    raise AssertionError((data/'server.log').read_text(encoding='utf8',errors='replace'))
                try:
                    if request('/status')['ok']:
                        return process
                except OSError:
                    pass
                time.sleep(.1)
            process.terminate()
            process.wait(timeout=20)
            raise AssertionError('server startup timed out')

        process = launch(old)
        try:
            request('/setup', {'email':'upgrade@example.invalid','name':'Upgrade',
                'password':'temporary-upgrade-password','token':(data/'setup-token').read_text().strip()})
            workspace = request('/workspaces', {'name':'Upgrade','slug':'upgrade'})['id']
            root = '/workspaces/'+workspace
            builtin_setting = next(row for row in request(root+'/settings')['items'] if row['key']=='default_page_size')
            request(root+'/settings/'+builtin_setting['id'], {'value':37,'version':1}, 'PATCH')
            builtin = next(row for row in request(root+'/dictionaries')['items'] if row['key']=='host-state')
            builtin = request(root+'/dictionaries/'+builtin['id'])
            labels = json.loads(builtin['items'])
            labels.reverse()
            labels[0]['label'] = 'Preserved label'
            request(root+'/dictionaries/'+builtin['id'], dict(builtin,items=json.dumps(labels)), 'PATCH')
            legacy_items = [{'value':'one','label':'One'},{'value':'two','label':'Two'}]
            legacy = request(root+'/dictionaries', {'key':'legacy','name':'Legacy',
                'description':'Preserve this content','items':json.dumps(legacy_items)}, expected=201)['id']
            process.terminate()
            process.wait(timeout=20)
            process = launch(new)
            for attempt in range(2):
                prefs = request(root+'/preferences')
                assert prefs['default_page_size']==37
                assert prefs['dictionaries']['host-state']==labels
                row = request(root+'/dictionaries/'+legacy)
                assert row['id']==legacy and row['version']==1 and row['description']=='Preserve this content'
                assert row['category']=='general' and row['value_type']=='selection'
                assert json.loads(row['items'])==legacy_items and json.loads(row['value_json'])==legacy_items
                summary = next(r for r in request(root+'/dictionaries')['items'] if r['id']==legacy)
                assert json.loads(summary['value_json'])==legacy_items
                setting = request(root+'/settings/'+builtin_setting['id'])
                assert setting['builtin'] and setting['value']==37 and json.loads(setting['value_json'])==37
                assert json.loads(setting['schema_json'])=={'min':1,'max':200,'decimals':0}
                assert request(root+'/settings?q='+urllib.parse.quote(setting['name']))['total']==1
                if attempt == 0:
                    process.terminate()
                    process.wait(timeout=20)
                    process = launch(new)
            typed = request(root+'/settings', {'key':'new_json','name':'New JSON','category':'upgrade',
                'value_type':'json','value_json':'{"after":"upgrade"}','schema_json':'{}'}, expected=201)['id']
            assert json.loads(request(root+'/settings/'+typed)['value_json'])=={'after':'upgrade'}
            print('PASS: previous-version settings, dictionary IDs/content/order/labels, legacy list projections, typed metadata, repeated startup and new typed CRUD')
        finally:
            process.terminate()
            process.wait(timeout=20)
