#!/usr/bin/env python3
"""Start the previous binary, create real data, then migrate with the new binary.
Pass OLD_SERVER NEW_SERVER. Optional FERRY_TEST_DATABASE_URL must name an empty test DB.
For the successful POSIX run comparison set OLD_RUNNER_BINARY and RUNNER_BINARY;
the new server must have its Actions worker available. alpha.1 requires POSIX sh.
"""
import http.cookiejar
from contextlib import closing
import json
import os
from pathlib import Path
import socket
import sqlite3
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
    upgraded = False
    runner = None
    old_runner = os.environ.get("OLD_RUNNER_BINARY")
    new_runner = os.environ.get("RUNNER_BINARY", str(Path(new).with_name("ferry-runner" + (".exe" if os.name == "nt" else ""))))
    def api(path, body=None):
        # Exercise each binary's own contract while comparing unchanged stored IDs/data.
        if upgraded:
            path = path.replace('/workspaces/upgrade/', f'/workspaces/{workspace_id}/')
            path = path.replace('/hosts/', '/nodes/').replace('/host-groups/', '/node-groups/')
            path = path.replace('/repository-auth', '/repository-providers').replace('/actions/render', '/renderings')
        request = urllib.request.Request(origin+path, data=json.dumps(body).encode() if body is not None else None,
            headers={'Content-Type':'application/json','X-Ferry-Request':'1', 'Accept-Language': 'zh-CN'})
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
        process = subprocess.Popen([binary], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
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
            workspace_id = api('/workspaces', {'slug':'upgrade','name':'upgrade'})['id']
            host=create('hosts', {'name':'saved-host','connection':'runner'})
            credential=create('credentials', {'name':'saved-credential','kind':'password','username':'saved-user','secret':'saved-secret'})
            repository=create('repositories', {'name':'saved-repository','url':'https://example.invalid/repo.git','credential_id':credential})
            original_definition=json.dumps({'workdir':'/tmp/upgrade','steps':[{'name':'step','run':'true'}]})
            pipeline=create('pipelines', {'name':'saved-pipeline','repository_id':repository,'target_kind':'host','target_ref':host,
                'definition':original_definition})
            run=api('/workspaces/upgrade/pipelines/'+pipeline+'/actions/run', {})['id']
            wait(lambda:api('/workspaces/upgrade/pipeline-runs/'+run)['state'] not in ('queued','running'))
            # alpha.1 executes POSIX sh scripts. Its own runner is needed to
            # test a successful pre-upgrade execution, rather than an offline failure.
            executable_pipeline = None
            if old_runner and os.name != 'nt':
                execute_host=create('hosts', {'name':'executable-host','connection':'runner'})
                token=api('/workspaces/upgrade/runner-tokens', {'host_id':execute_host})['token']
                runner=subprocess.Popen([old_runner,'--server',origin[:-4],'--token',token,'--state-dir',str(Path(directory)/'runner')],stdout=log,stderr=log)
                wait(lambda:api('/workspaces/upgrade/hosts/'+execute_host)['state']=='online')
                executable_pipeline=create('pipelines', {'name':'executable-legacy','target_kind':'host','target_ref':execute_host,
                    'definition':json.dumps({'workdir':str(Path(directory)/'build-{{run}}'),'steps':[{'name':'result','run':'printf UPGRADE_SAME_RESULT'}]})})
                executed=api('/workspaces/upgrade/pipelines/'+executable_pipeline+'/actions/run', {})['id']
                wait(lambda:api('/workspaces/upgrade/pipeline-runs/'+executed)['state'] not in ('queued','running'))
                assert api('/workspaces/upgrade/pipeline-runs/'+executed)['state']=='succeeded'
                old_output=api('/workspaces/upgrade/pipeline-runs/'+executed+'/logs?after=0')['items']
                assert 'UPGRADE_SAME_RESULT' in ''.join(row['chunk'] for row in old_output)
                runner.terminate();runner.wait(timeout=20);runner=None
            else:
                print('SKIP: successful alpha.1 run comparison requires OLD_RUNNER_BINARY and a POSIX host; conversion/history checks still run')
            paths=['/me']+['/workspaces/upgrade/'+resource+'/'+id for resource,id in [('hosts',host),('credentials',credential),('repositories',repository),('pipelines',pipeline),('pipeline-runs',run)]]
            before={path:api(path) for path in paths}
            roles_before = api('/workspaces/upgrade/roles')['items']
            process.terminate(); process.wait(timeout=20)
            upgraded = True
            process=launch(new,log)
            if not os.environ.get('FERRY_TEST_DATABASE_URL'):
                databases=list(Path(directory).glob('*.sqlite'))
                assert len(databases)==1, databases
                with closing(sqlite3.connect(databases[0])) as database:
                    saved=database.execute('SELECT definition, legacy_definition_backup FROM pipelines WHERE id=?',(int(pipeline),)).fetchone()
                original_backup=before['/workspaces/upgrade/pipelines/'+pipeline]['definition']
                assert saved[1]==original_backup, 'migration must preserve the original legacy JSON'
                assert json.loads(saved[0])['engine']=='github-actions'
            if executable_pipeline:
                token=api('/workspaces/upgrade/runner-tokens', {'host_id':host})['token']
                runner=subprocess.Popen([new_runner,'--server',origin[:-4],'--token',token,'--state-dir',str(Path(directory)/'new-runner')],stdout=log,stderr=log)
                wait(lambda:api('/workspaces/upgrade/hosts/'+host)['state']=='online')
                converted=api('/workspaces/upgrade/pipelines/'+executable_pipeline)
                assert converted['definition_format']=='github-actions'
                executed=api('/workspaces/upgrade/pipelines/'+executable_pipeline+'/runs', {})['id']
                wait(lambda:api('/workspaces/upgrade/pipeline-runs/'+executed)['state'] not in ('queued','running'))
                assert api('/workspaces/upgrade/pipeline-runs/'+executed)['state']=='succeeded'
                output=api('/workspaces/upgrade/pipeline-runs/'+executed+'/logs?after=0')['items']
                assert 'UPGRADE_SAME_RESULT' in ''.join(row['chunk'] for row in output)
                print('PASS: legacy pipeline executes the same command result before and after Actions migration')
            for resource in ('insight-reports', 'code-reviews'):
                try:
                    api('/workspaces/upgrade/' + resource)
                    raise AssertionError('Retired feature endpoint is still available: ' + resource)
                except urllib.error.HTTPError as error:
                    assert error.code == 404
            print('PASS: retired report and review endpoints remain unavailable after upgrade')
            for path in paths:
                after = api(path)
                if '/credentials/' in path:
                    # Existing repository credentials retain their identity and gain build ownership.
                    assert after['scope'] == 'build'
                    before[path]['scope'] = 'build'
                if path == '/me':
                    # This migration adds a public privilege flag, never elevation.
                    assert after['user'].get('is_superadmin', False) is False
                    before[path]['user']['is_superadmin'] = False
                    allowed={'marketplace-sources.'+verb for verb in ['read','create','update','delete','sync']} | {'config-templates.'+verb for verb in ['read','create','update','delete']}
                    allowed |= {resource+'.'+verb for resource in ['pipeline-groups','pipeline-environments'] for verb in ['read','create','update','delete']}
                    allowed |= {'code-insights.'+verb for verb in ['read','create','update','delete','sync']}
                    allowed |= {'workflows.'+verb for verb in ['read','create','update','delete','run']}
                    allowed |= {'workflow-groups.'+verb for verb in ['read','create','update','delete']}
                    allowed |= {'credentials.use', 'credentials.update_price', 'credentials.update_budget'}
                    old_spaces={w['id']:w for w in before[path]['workspaces']}
                    for migrated in after['workspaces']:
                        original=old_spaces[migrated['id']]
                        added=set(migrated['permissions'])-set(original['permissions'])
                        assert added<=allowed and set(original['permissions'])<=set(migrated['permissions']), ('unexpected workspace permission changes', sorted(added-allowed), sorted(set(original['permissions'])-set(migrated['permissions'])))
                        original['permissions']=migrated['permissions']
                        # Additive: whether the user created the workspace; this user created it.
                        assert migrated['relation'] == 'owner', migrated['relation']
                        original['relation']='owner'
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
                    # Read-only platform metadata is now exposed; a runner that
                    # never enrolled has reported neither value.
                    for key in ('os', 'agent_version'):
                        assert after[key] == before[path].get(key, ''), (key, after[key])
                        before[path].setdefault(key, '')
                    # Existing hosts inherit the confirmed global update policy.
                    assert after['runner_auto_update'] == before[path].get('runner_auto_update', 'inherit')
                    before[path].setdefault('runner_auto_update', 'inherit')
                    # This fixture never enrolled an agent; no reported version means
                    # no available update. The field is additive, not persisted data.
                    if 'agent_update_available' in after:
                        assert after['agent_version'] == ''
                        assert after['agent_update_available'] is False
                        before[path]['agent_update_available'] = False
                    if 'agent_update_state' in after:
                        assert after['agent_update_state'] is None
                        before[path]['agent_update_state'] = None
                    # Existing hosts stay private with one execution slot. These
                    # additive fields must not alter any persisted host data.
                    for key, default in {
                        'execution': {'enabled': True, 'shared': False, 'capacity': 1},
                        'execution_status': {'active': 0, 'capacity': 1, 'state': 'available', 'running_jobs': []},
                    }.items():
                        assert after[key] == before[path].get(key, default), (key, after[key])
                        before[path].setdefault(key, default)
                if '/pipelines/' in path:
                    assert after['owner_id'] == before['/me']['user']['id']
                    before[path]['owner_id'] = after['owner_id']
                    assert after['definition_format']=='github-actions'
                    assert "run: 'true'" in after['definition'] or 'run: true' in after['definition']
                    assert after['executor_bindings']==[{'job_id':'build','target_kind':'host','target_ref':host,'image':'host'}]
                    assert after['checkout_bindings']==[]
                    for key in ('definition','definition_format','executor_bindings','checkout_bindings','graph_layout'):
                        before[path][key]=after[key]
                    defaults={'group_id':'0','application_id':'0','tag_ids':[],'environment_ids':[],'tags':[],'environments':[],'group_name':'','application_name':''}
                    for key,value in defaults.items():
                        assert after[key]==before[path].get(key,value),(key,after[key])
                        before[path].setdefault(key,value)
                if '/pipeline-runs/' in path:
                    for key,value in {'revision_id':'0','environment_id':'0','environment_name':'','jobs':[],'definition_format':'legacy','inputs':{},'definition_snapshot':None}.items():
                        assert after[key]==before[path].get(key,value),(key,after[key])
                        before[path].setdefault(key,value)
                    # Runs recorded before name snapshots are named live from their IDs.
                    assert after['pipeline_name'] and after['triggered_by_name'] and isinstance(after['host_name'], str), after
                    for key in ('pipeline_name','host_name','triggered_by_name'):
                        before[path][key]=after[key]
                assert after==before[path], f'Upgrade changed existing resource: {path}; fields: {[key for key in set(after)|set(before[path]) if after.get(key)!=before[path].get(key)]}'
            print('PASS: legacy pipeline converts to Actions with its host binding; original JSON backup and historical run remain intact')
            roles_after = {r['id']:r for r in api('/workspaces/upgrade/roles')['items']}
            for role in roles_before:
                migrated = roles_after[role['id']]
                added=set(migrated['permissions'])-set(role['permissions'])
                expected={'marketplace-sources.'+verb for verb in ['read','create','update','delete','sync']} if role['name'] in ['拥有者','管理员'] else {'marketplace-sources.read'} if role['name']=='只读' else set()
                expected |= {'config-templates.'+verb for verb in ['read','create','update','delete']} if role['name'] in ['拥有者','管理员','开发'] else {'config-templates.read'} if role['name']=='只读' else set()
                expected |= {resource+'.'+verb for resource in ['pipeline-groups','pipeline-environments'] for verb in (['read','create','update','delete'] if role['name'] in ['拥有者','管理员'] else ['read'] if role['name']=='只读' else [])}
                expected |= {'code-insights.'+verb for verb in (['read','create','update','delete','sync'] if role['name'] in ['拥有者','管理员','开发'] else ['read'] if role['name']=='只读' else [])}
                expected |= {'workflows.'+verb for verb in (['read','create','update','delete','run'] if role['name'] in ['拥有者','管理员','开发'] else ['read'] if role['name']=='只读' else [])}
                expected |= {'workflow-groups.'+verb for verb in (['read','create','update','delete'] if role['name'] in ['拥有者','管理员','开发'] else ['read'] if role['name']=='只读' else [])}
                if role['name'] in ['拥有者','管理员']:
                    expected |= {'credentials.use', 'credentials.update_price', 'credentials.update_budget'}
                expected -= set(role['permissions'])
                assert added==expected and set(role['permissions'])<=set(migrated['permissions']), (role['name'], sorted(added), sorted(expected))
                assert migrated['version'] == role.get('version', 1)+bool(added)
                for field in ('id','name','builtin'):
                    assert migrated[field] == role[field], f'Upgrade changed role {field}'
            members = api('/workspaces/upgrade/members')['items']
            assert len(members) == 1 and members[0]['owner'] and members[0]['version'] == 1
            custom = create('roles', {'name':'Migrated custom','permissions':['hosts.create']})
            assert 'hosts.read' in api('/workspaces/upgrade/roles/'+custom)['permissions']
            print('PASS: existing grants/membership preserved; only specified built-in resource permissions appended with version bump')
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
                if resource=='pipeline-templates':
                    assert json.loads(rendered['definition'])['engine']=='github-actions'
                    assert json.loads(api('/workspaces/upgrade/'+resource+'/'+template)['content'])['engine']=='github-actions'
            print('PASS: both template schemas can create and render after upgrade')
            assert api('/workspaces/upgrade/preferences')['default_page_size']==20
            assert api('/workspaces/upgrade/settings')['total']==5
            seeded_dictionaries = api('/workspaces/upgrade/dictionaries')
            assert seeded_dictionaries['total'] == 2
            assert {row['key'] for row in seeded_dictionaries['items']} == {
                'environment', 'app-category',
            }
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
            converted_pipeline=api('/workspaces/upgrade/pipelines/'+pipeline)
            process.terminate();process.wait(timeout=20)
            process=launch(new,log)
            assert api('/workspaces/upgrade/pipelines/'+pipeline)==converted_pipeline
            if not os.environ.get('FERRY_TEST_DATABASE_URL'):
                with closing(sqlite3.connect(databases[0])) as database:
                    backup=database.execute('SELECT legacy_definition_backup FROM pipelines WHERE id=?',(int(pipeline),)).fetchone()[0]
                assert backup==original_backup
            print('PASS: reopening the upgraded database leaves conversion and original backup unchanged')
            print('PASS: prior account/session, host, credential metadata, repository, pipeline and run survive; new deployment schema works')
        finally:
            if runner is not None and runner.poll() is None: runner.terminate();runner.wait(timeout=20)
            if process.poll() is None: process.terminate(); process.wait(timeout=20)
