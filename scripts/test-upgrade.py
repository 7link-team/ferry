#!/usr/bin/env python3
"""Start the previous binary, create real data, then migrate with the new binary.
Pass OLD_SERVER NEW_SERVER. Optional FERRY_TEST_DATABASE_URL must name an empty test DB.
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
import urllib.request
import urllib.error

old, new = map(lambda p: str(Path(p).resolve()), sys.argv[1:])
with tempfile.TemporaryDirectory(prefix='server-upgrade-') as directory:
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}/api'
    env = dict(os.environ, FERRY_DATA_DIR=directory, FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    if os.environ.get('FERRY_TEST_DATABASE_URL'):
        env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    def api(path, body=None):
        request = urllib.request.Request(origin+path, data=json.dumps(body).encode() if body is not None else None,
            headers={'Content-Type':'application/json','X-Ferry-Request':'1'})
        with client.open(request, timeout=10) as response:
            return json.load(response)
    def wait(fn):
        for _ in range(150):
            try:
                result = fn()
                if result: return result
            except OSError: pass
            time.sleep(.2)
        raise AssertionError('server or run did not become ready')
    def launch(binary, log):
        process = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
        try: wait(lambda: api('/status')['ok'])
        except BaseException:
            process.terminate(); process.wait(); raise
        return process
    def create(resource, body): return api('/workspaces/upgrade/'+resource, body)['id']
    with open(Path(directory)/'server.log', 'wb') as log:
        process = launch(old, log)
        try:
            subprocess.run(['python3', 'scripts/test-setup.py', origin, directory, '--legacy'], check=True)
            api('/users', {'email':'upgrade@example.invalid','password':'correct-horse-battery','name':'upgrade'})
            api('/workspaces', {'slug':'upgrade','name':'upgrade'})
            host=create('hosts', {'name':'saved-host','connection':'runner'})
            credential=create('credentials', {'name':'saved-credential','kind':'password','username':'saved-user','secret':'saved-secret'})
            repository=create('repositories', {'name':'saved-repository','url':'https://example.invalid/repo.git','credential_id':credential})
            pipeline=create('pipelines', {'name':'saved-pipeline','repository_id':repository,'target_kind':'host','target_ref':host,
                'definition':json.dumps({'workdir':'/tmp/upgrade','steps':[{'name':'step','run':'true'}]})})
            run=api('/workspaces/upgrade/pipelines/'+pipeline+'/actions/run', {})['id']
            wait(lambda:api('/workspaces/upgrade/pipeline-runs/'+run)['state'] not in ('queued','running'))
            paths=['/me']+['/workspaces/upgrade/'+resource+'/'+id for resource,id in [('hosts',host),('credentials',credential),('repositories',repository),('pipelines',pipeline),('pipeline-runs',run)]]
            before={path:api(path) for path in paths}
            process.terminate(); process.wait(timeout=20)
            process=launch(new,log)
            for path in paths:
                after = api(path)
                if path == '/me':
                    # This migration adds a public privilege flag, never elevation.
                    assert after['user'].get('is_superadmin', False) is False
                    before[path]['user']['is_superadmin'] = False
                assert after==before[path], f'Upgrade changed existing resource: {path}'
            deployment=create('deployments', {'name':'new-deployment','mode':'compose','target_kind':'host','target_ref':host,'workdir':'/tmp/upgrade-compose',
                'manifest':json.dumps({'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'busybox:1.36'}]})})
            assert api('/workspaces/upgrade/deployments/'+deployment)['target_ref']==host
            setup = api('/setup')
            if setup['required']:
                assert not api('/me')['user']['is_superadmin'], 'legacy owner must not be automatically promoted'
                proof = (Path(directory)/'setup-token').read_text().strip()
                body = {'token': proof, 'email': 'upgrade@example.invalid', 'name': 'must-not-overwrite', 'password': 'incorrect-existing-password'}
                try:
                    api('/setup', body)
                    raise AssertionError('existing account claimed without password proof')
                except urllib.error.HTTPError as error:
                    assert error.code == 400
                assert api('/setup')['required']
                body['password'] = 'correct-horse-battery'
                api('/setup', body)
                identity = api('/me')
                assert identity['user']['is_superadmin'] and identity['user']['name'] == 'upgrade'
                assert not (Path(directory)/'setup-token').exists()
                print('PASS: legacy owner not auto-promoted; operator token plus existing password required; old name/password preserved')
            print('PASS: prior account/session, host, credential metadata, repository, pipeline and run survive; new deployment schema works')
        finally:
            if process.poll() is None: process.terminate(); process.wait(timeout=20)
