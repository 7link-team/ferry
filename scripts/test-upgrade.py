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

if os.environ.get('GITHUB_ACTIONS'):
    def report_failure(kind, error, trace):
        message = f'{kind.__name__}: {error}'.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::error::{message}', flush=True)
        sys.__excepthook__(kind, error, trace)
    sys.excepthook = report_failure

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
            subprocess.run([sys.executable, 'scripts/test-setup.py', origin, directory, '--legacy'], check=True)
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
            roles_before = api('/workspaces/upgrade/roles')['items']
            process.terminate(); process.wait(timeout=20)
            process=launch(new,log)
            for path in paths:
                after = api(path)
                if path == '/me':
                    # This migration adds a public privilege flag, never elevation.
                    assert after['user'].get('is_superadmin', False) is False
                    before[path]['user']['is_superadmin'] = False
                    allowed={'marketplace-sources.'+verb for verb in ['read','create','update','delete','sync']} | {'config-templates.'+verb for verb in ['read','create','update','delete']}
                    allowed |= {resource+'.'+verb for resource in ['pipeline-groups','pipeline-environments'] for verb in ['read','create','update','delete']}
                    old_spaces={w['id']:w for w in before[path]['workspaces']}
                    for migrated in after['workspaces']:
                        original=old_spaces[migrated['id']]
                        added=set(migrated['permissions'])-set(original['permissions'])
                        assert added<=allowed and set(original['permissions'])<=set(migrated['permissions'])
                        original['permissions']=migrated['permissions']
                if path == '/workspaces/upgrade/pipeline-runs/'+run:
                    # New archive metadata is additive; old runs have no archived files.
                    assert after.get('artifacts') == []
                    before[path]['artifacts'] = []
                if '/repositories/' in path:
                    assert after['provider'] == before[path].get('provider', 'git')
                    before[path]['provider'] = before[path].get('provider', 'git')
                if '/hosts/' in path:
                    assert after.get('tag_ids') == [] and after.get('tags') == []
                    before[path]['tag_ids'] = []
                    before[path]['tags'] = []
                    assert after['jump_host_id'] == before[path].get('jump_host_id', '0')
                    before[path].setdefault('jump_host_id', '0')
                if '/pipelines/' in path:
                    defaults={'definition_format':'legacy','executor_bindings':[],'checkout_bindings':[],'graph_layout':{},'group_id':'0','application_id':'0','tag_ids':[],'environment_ids':[],'tags':[],'environments':[],'group_name':'','application_name':''}
                    for key,value in defaults.items():
                        assert after[key]==before[path].get(key,value),(key,after[key])
                        before[path].setdefault(key,value)
                if '/pipeline-runs/' in path:
                    for key,value in {'environment_id':'0','environment_name':'','jobs':[],'definition_format':'legacy'}.items():
                        assert after[key]==before[path].get(key,value),(key,after[key])
                        before[path].setdefault(key,value)
                assert after==before[path], f'Upgrade changed existing resource: {path}; fields: {[key for key in set(after)|set(before[path]) if after.get(key)!=before[path].get(key)]}'
            roles_after = {r['id']:r for r in api('/workspaces/upgrade/roles')['items']}
            for role in roles_before:
                migrated = roles_after[role['id']]
                added=set(migrated['permissions'])-set(role['permissions'])
                expected={'marketplace-sources.'+verb for verb in ['read','create','update','delete','sync']} if role['name'] in ['拥有者','管理员'] else {'marketplace-sources.read'} if role['name']=='只读' else set()
                expected |= {'config-templates.'+verb for verb in ['read','create','update','delete']} if role['name'] in ['拥有者','管理员','开发'] else {'config-templates.read'} if role['name']=='只读' else set()
                expected |= {resource+'.'+verb for resource in ['pipeline-groups','pipeline-environments'] for verb in (['read','create','update','delete'] if role['name'] in ['拥有者','管理员'] else ['read'] if role['name']=='只读' else [])}
                expected -= set(role['permissions'])
                assert added==expected and set(role['permissions'])<=set(migrated['permissions']), (role['name'], sorted(added), sorted(expected))
                assert migrated['version'] == role.get('version', 1)+bool(added)
                for field in ('id','name','builtin'):
                    assert migrated[field] == role[field], f'Upgrade changed role {field}'
            members = api('/workspaces/upgrade/members')['items']
            assert len(members) == 1 and members[0]['owner'] and members[0]['version'] == 1
            custom = create('roles', {'name':'Migrated custom','permissions':['hosts.create']})
            assert 'hosts.read' in api('/workspaces/upgrade/roles/'+custom)['permissions']
            print('PASS: existing grants/membership preserved; only new built-in market permissions appended with version bump')
            deployment=create('deployments', {'name':'new-deployment','mode':'compose','target_kind':'host','target_ref':host,'workdir':'/tmp/upgrade-compose',
                'manifest':json.dumps({'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'busybox:1.36'}]})})
            assert api('/workspaces/upgrade/deployments/'+deployment)['target_ref']==host
            scheduled=create('cron-jobs', {'name':'Migrated schedule','scope':'host','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'command','target_kind':'host','target_ref':host,'command':'true','timeout_seconds':10})
            assert api('/workspaces/upgrade/cron-jobs/'+scheduled)['next_run_at']==0
            assert api('/workspaces/upgrade/cron-jobs/'+scheduled+'/runs')['items']==[]
            print('PASS: scheduled task and execution history schemas work after upgrade')
            alert=create('alert-rules', {'name':'Migrated monitor','target_kind':'host','target_ref':host,'metric':'offline','enabled':False})
            assert not api('/workspaces/upgrade/alert-rules/'+alert)['has_webhook']
            assert api('/workspaces/upgrade/alert-events')['items']==[]
            print('PASS: alert rules and event history schemas work after upgrade')
            source=create('marketplace-sources',{'name':'Migrated source','repository_id':repository,'provider':'github','api_base':'https://example.invalid/api','project':'owner/repository','branch':'main','index_path':'marketplace.json'})
            assert api('/workspaces/upgrade/marketplace-sources/'+source)['state']=='idle'
            assert api('/workspaces/upgrade/marketplace')['items']==[]
            assert {row['provider'] for row in api('/workspaces/upgrade/repository-auth')['items']}=={'github','gitlab','gitee','gitea','codeup'}
            print('PASS: market catalog and web authorization schemas work after upgrade')
            for resource, content in [('app-templates', {'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'nginx:alpine'}]}), ('pipeline-templates', {'workdir':'/tmp/upgrade','steps':[{'name':'test','run':'true'}]})]:
                template=create(resource, {'name':'After migration','content':json.dumps(content),'parameters':'[]'})
                rendered=api('/workspaces/upgrade/'+resource+'/'+template+'/actions/render', {'version':1})
                assert rendered['template_version']==1
            print('PASS: both template schemas can create and render after upgrade')
            assert api('/workspaces/upgrade/preferences')['default_page_size']==20
            assert api('/workspaces/upgrade/settings')['total']==3
            assert api('/workspaces/upgrade/dictionaries')['total']==3
            print('PASS: existing workspace gets typed settings and dictionary defaults after upgrade')
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
