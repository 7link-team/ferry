#!/usr/bin/env python3
"""Real HTTP platform acceptance; optional empty FERRY_TEST_DATABASE_URL."""
import concurrent.futures
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

checks = 0
def check(condition, label):
    global checks
    assert condition, label
    checks += 1
    print('PASS', label, flush=True)

def client():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

with tempfile.TemporaryDirectory(prefix='platform-test-') as directory:
    data = Path(directory)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}/api'
    env = dict(os.environ, FERRY_DATA_DIR=directory, FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    if os.environ.get('FERRY_TEST_DATABASE_URL'): env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
    binary = os.environ.get('FERRY_BINARY', 'target/debug/ferry-server')
    def request(c, path, body=None, method=None, expected=200):
        req = urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None,
            method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN'})
        try: response = c.open(req, timeout=30)
        except urllib.error.HTTPError as e: response = e
        raw = response.read()
        try: value = json.loads(raw) if raw else None
        except ValueError: raise AssertionError((path, response.code, raw))
        if expected is not None: assert response.code == expected, (path, response.code, value)
        return (response.code, value) if expected is None else value
    anonymous = client()
    log = open(data / 'server.log', 'wb')
    def launch():
        proc = subprocess.Popen([binary], env=env, stdout=log, stderr=log,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        for _ in range(600):  # fresh MySQL/PostgreSQL migrations can exceed 15s on CI
            if proc.poll() is not None: raise AssertionError((data / 'server.log').read_text())
            try:
                if request(anonymous, '/status')['ok']: return proc
            except OSError: pass
            time.sleep(.1)
        raise AssertionError('server startup timed out')
    proc = launch()
    try:
        admin, owner, guest, outsider = client(), client(), client(), client()
        password = 'temporary-workspace-password'
        request(admin, '/setup', {'email':'admin@example.invalid','name':'Admin','password':password,'token':(data/'setup-token').read_text().strip()})
        for c, name in [(owner,'Owner'),(guest,'Guest'),(outsider,'Outsider')]:
            request(c, '/users', {'name':name,'email':name.lower()+'@example.invalid','password':password})
        workspace_tenant = request(owner, '/workspaces', {'name':'Tenant','slug':'tenant'})['id']
        workspace_other = request(outsider, '/workspaces', {'name':'Other','slug':'other'})['id']
        root = f'/workspaces/{workspace_tenant}'
        check(isinstance(workspace_tenant, str) and int(workspace_tenant) > 2**53,
              'workspace Snowflake ID is an exact JSON string beyond JavaScript integer precision')
        for invalid in ['tenant', 'other', '0', '-1', '1e18', '999999999999999999999999999']:
            request(owner, f'/workspaces/{invalid}/nodes', expected=404)
            request(admin, f'/workspaces/{invalid}/nodes', expected=404)
        request(owner, f'/workspaces/{workspace_other}/nodes', expected=404)
        request(outsider, root+'/nodes', expected=404)
        check(True, 'workspace IDs enforce tenant isolation; names and invalid IDs are rejected for all roles')
        code = request(owner, root+'/members/invitation')['code']
        rotated = request(owner, root+'/members/invitation', method='POST')['code']
        check(code != rotated, 'invitation rotation changes secret')
        request(guest, '/workspace-memberships', {'code':code}, expected=400)
        joined = request(guest, '/workspace-memberships', {'code':rotated})
        check(joined['id'] == workspace_tenant, 'joining returns the same canonical workspace ID')
        request(guest, '/workspace-memberships', {'code':rotated}, expected=409)
        check(request(owner, root+'/members')['total'] == 2, 'join appears in member list')
        rows = request(owner, root+'/members?q=guest')['items']
        check(len(rows)==1 and rows[0]['email']=='guest@example.invalid', 'member search uses public profile')
        member = rows[0]; mid = member['id']
        check('password' not in json.dumps(rows), 'member response has no password fields')
        owner_member = next(r for r in request(owner,root+'/members')['items'] if r['owner'])
        builtin = request(owner, root+'/roles')['items']
        owner_role = next(r for r in builtin if r['name']=='拥有者')
        viewer_role = next(r for r in builtin if r['name']=='只读')
        request(owner,root+'/members/'+owner_member['id'],method='DELETE',expected=400)
        request(owner,root+'/members/'+owner_member['id'],{'version':1,'role_id':viewer_role['id']},'PATCH',400)
        request(owner,root+'/members/'+mid,{'version':1,'role_id':owner_role['id']},'PATCH',400)
        for r in builtin:
            request(owner,root+'/roles/'+r['id'],method='DELETE',expected=400)
            request(owner,root+'/roles/'+r['id'],{'version':1,'name':r['name'],'permissions':[]},'PATCH',400)
        check(True,'owner and builtin protections enforced')
        request(guest,root+'/roles',{'name':'Forbidden','permissions':[]},expected=403)
        request(guest,root+'/members/invitation',expected=403)
        request(owner,root+'/roles',{'name':'Bad','permissions':['bogus.read']},expected=400)
        body={'name':'Operator','permissions':['hosts.create','dashboard.read']}
        rid=request(owner,root+'/roles',body)['id']
        role=request(owner,root+'/roles/'+rid)
        check('hosts.read' in role['permissions'], 'role dependency closure persisted')
        request(owner,root+'/roles',body,expected=409)
        request(owner,root+'/roles/'+rid,dict(body,version=0),'PATCH',409)
        request(owner,root+'/roles/'+rid,dict(body,version=1,name='Operator revised'),'PATCH')
        foreign=request(outsider,f'/workspaces/{workspace_other}/roles')['items'][0]['id']
        request(owner,root+'/roles/'+foreign,expected=404)
        request(owner,root+'/members/'+mid,{'version':1,'role_id':foreign},'PATCH',404)
        request(owner,root+'/members/'+mid,{'version':1,'role_id':rid},'PATCH')
        request(owner,root+'/members/'+mid,{'version':1,'role_id':rid},'PATCH',409)
        request(owner,root+'/roles/'+rid,method='DELETE',expected=409)
        check(request(guest,'/me')['workspaces'][0]['role_name']=='Operator revised','role assignment effective without re-login')
        summary=request(guest,root+'/dashboard')
        check(summary['counts']=={'hosts':0} and summary['recent'] is None and summary['failures_24h']=={}, 'dashboard hides unauthorized metrics and activities')
        host_id=request(guest,root+'/nodes',{'name':'Runner','connection':'runner'},expected=201)['id']
        summary=request(owner,root+'/dashboard')
        check(summary['counts']['hosts']==1 and summary['counts']['members']==2 and summary['counts']['roles']==6, 'dashboard counts reflect real tenant rows')
        check(summary['states']['hosts']['unknown']==1 and summary['runners'][0]['seen_at'] is None and summary['runners'][0]['lag_ms'] is None, 'unreported runner is not shown as healthy')
        check(len(summary['recent'])>0 and all('details' not in a for a in summary['recent']), 'recent activities expose safe summary')
        check(request(outsider,f'/workspaces/{workspace_other}/dashboard')['counts']['hosts']==0,'dashboard tenant isolation')
        pipeline_id=request(owner,root+'/pipelines',{'name':'Dashboard failure','target_kind':'host','target_ref':host_id,'definition':json.dumps({'workdir':'/tmp/workspace-test','steps':[{'name':'offline','run':'true'}]})},expected=201)['id']
        run_id=request(owner,root+'/pipelines/'+pipeline_id+'/runs',{},expected=201)['id']
        for _ in range(100):
            state=request(owner,root+'/pipeline-runs/'+run_id)['state']
            if state not in ('queued','running'): break
            time.sleep(.1)
        check(state=='failed','offline runner produces an actual failed execution')
        summary=request(owner,root+'/dashboard')
        check(summary['failures_24h']['pipeline-runs']==1 and summary['states']['pipeline-runs']=={'failed':1},'dashboard failure count and distribution match actual execution')
        limited={'name':'Role editor','permissions':['roles.create','roles.update','roles.delete','members.update','dashboard.read']}
        limited_id=request(owner,root+'/roles',limited)['id']
        request(owner,root+'/members/'+mid,{'version':2,'role_id':limited_id},'PATCH')
        request(guest,root+'/roles',{'name':'Escalate','permissions':['workspace.manage']},expected=400)
        request(guest,root+'/members/'+mid,{'version':3,'role_id':owner_role['id']},'PATCH',400)
        request(guest,root+'/roles/'+rid,dict(body,version=2),'PATCH',400)
        check(True,'limited role cannot grant or manage higher privileges')
        request(owner,root+'/members/'+mid,method='DELETE',expected=204)
        request(guest,root+'/dashboard',expected=404)
        request(owner,root+'/roles/'+rid,method='DELETE',expected=204)
        check(True,'removal takes effect immediately; unreferenced role can be deleted')
        with concurrent.futures.ThreadPoolExecutor() as pool:
            results=list(pool.map(lambda _: request(owner,root+'/roles',{'name':'Race','permissions':[]},expected=None), range(2)))
        check(sorted(r[0] for r in results)==[200,409],'concurrent duplicate role creation serialized')
        for resource in ['node-groups', 'pipeline-groups']:
            first = request(owner, root+'/'+resource, {'name':'First'}, expected=201)['id']
            second = request(owner, root+'/'+resource, {'name':'Second'}, expected=201)['id']
            order = {'ids':[second, first]}
            if resource == 'node-groups': order['parent_id'] = '0'
            request(owner, root+'/'+resource+'/ordering', order, method='PATCH')
            rows = request(owner, root+'/'+resource)['items']
            check([row['id'] for row in sorted(rows, key=lambda row: row['position'])] == [second, first],
                  resource+' ordering uses PATCH and persists the requested order')
        removable = request(owner, root+'/nodes', {'name':'Temporary node','connection':'runner'}, expected=201)['id']
        request(owner, root+'/nodes', {'ids':[removable]}, method='DELETE')
        request(owner, root+'/nodes/'+removable, expected=404)
        check(True, 'DELETE node collection removes only the selected node')
        settings_path = root + '/workspace-settings/1'
        initial_zone = request(owner, settings_path)
        check(initial_zone['timezone'] is None, 'legacy workspace remains explicitly unconfirmed')
        request(outsider, settings_path, expected=404)
        request(owner, settings_path, dict(initial_zone, timezone='system'), method='PATCH', expected=400)
        saved_zone = request(owner, settings_path, dict(initial_zone, timezone='Asia/Kuala_Lumpur'), method='PATCH')
        check(saved_zone['timezone'] == 'Asia/Kuala_Lumpur' and saved_zone['version'] > initial_zone['version'],
              'actual scoped shared-form PATCH confirms the workspace timezone with a fresh version')
        request(owner, settings_path, dict(initial_zone, timezone='UTC'), method='PATCH', expected=409)
        identity_zone = next(row for row in request(owner, '/me')['workspaces'] if row['id'] == workspace_tenant)
        check(identity_zone['timezone'] == 'Asia/Kuala_Lumpur', 'identity refresh returns the confirmed workspace zone')
        confirmed = request(owner, '/workspaces', {'name':'Zoned workspace','slug':'zoned-workspace','timezone':'Pacific/Auckland'})
        check(confirmed['timezone'] == 'Pacific/Auckland', 'creation persists an explicit editable IANA zone')
        request(owner, '/workspaces', {'name':'Invalid zone','slug':'invalid-zone','timezone':''}, expected=400)
        request(owner, settings_path, dict(saved_zone, timezone=''), method='PATCH', expected=400)
        check(request(owner, settings_path)['timezone'] == 'Asia/Kuala_Lumpur', 'invalid and stale writes leave timezone unchanged')
        # This additional integration requires the separately built, OS-confined sandbox.
        if os.environ.get('FERRY_TEST_WORKFLOW_TIMEZONE') == '1':
            clock_graph = {'engine':'ferry-graph','schema':1,'settings':{'timezone':'Asia/Tokyo'},'nodes':[
                {'id':'start','name':'Start','type':'core.trigger.manual','type_version':1,'position':[0,0],
                 'params':{'input_schema':{'type':'object'}}},
                {'id':'wait','name':'Wait','type':'core.wait','type_version':1,'position':[200,0],
                 'params':{'mode':'duration','duration_ms':1500}},
                {'id':'clock','name':'Clock','type':'data.set','type_version':1,'position':[400,0],
                 'params':{'fields':[{'path':'zone','value':'={{ $now.zoneName }}'}],'keep_other_fields':True}}],
                'connections':[{'from':{'node':a,'port':'main','index':0},'to':{'node':b,'port':'main','index':0}}
                               for a,b in [('start','wait'),('wait','clock')]]}
            workflow = request(owner, root+'/workflows', {'name':'Workspace clock acceptance','definition':json.dumps(clock_graph)}, expected=201)
            run_body = {'mode':'manual','revision':'draft','version':workflow['version'],'entry_node_id':'start','inputs':{}}
            execution = request(owner, root+'/workflows/'+workflow['id']+'/executions', run_body, expected=201)
            # The wait prevents the clock node finishing before this configuration change.
            latest_zone = request(owner, settings_path)
            request(owner, settings_path, dict(latest_zone,timezone='UTC'), method='PATCH')
            def wait_execution(execution):
                endpoint = root+'/workflow-executions/'+execution['id']
                deadline = time.monotonic()+30
                while time.monotonic()<deadline:
                    result = request(owner,endpoint)
                    if result['state'] in ('succeeded','failed','cancelled'): return result
                    time.sleep(0.05)
                raise AssertionError(('execution did not finish',execution['id']))
            check(wait_execution(execution)['state']=='succeeded','actual workflow executes with the confirmed business clock')
            execution_path = root+'/workflow-executions/'+execution['id']
            clock_attempt = next(row for row in request(owner,execution_path+'/node-runs')['items'] if row['node_id']=='clock')
            clock_output = request(owner,execution_path+'/node-runs/'+clock_attempt['id']+'/output')
            check(clock_output['ports'][0]['items'][0]['json']['zone']=='Asia/Kuala_Lumpur',
                  'running workflow retains its frozen workspace zone after a live timezone change')
            clock_graph['nodes'] = [clock_graph['nodes'][0],clock_graph['nodes'][2]]
            clock_graph['connections'] = [{'from':{'node':'start','port':'main','index':0},'to':{'node':'clock','port':'main','index':0}}]
            clock_graph['nodes'][1]['params']['fields'][0]['value'] = '={{ missing.value }}'
            bad = request(owner,root+'/workflows',{'name':'Expression diagnosis acceptance','definition':json.dumps(clock_graph)},expected=201)
            failed = request(owner,root+'/workflows/'+bad['id']+'/executions',dict(run_body,version=bad['version']),expected=201)
            diagnostic = wait_execution(failed)
            check(diagnostic['state']=='failed' and '/nodes/1/params/fields/0/value' in diagnostic['error'],
                  'pre-activation expression failure exposes its exact diagnostic through the authorized HTTP API')
            attempts = request(owner,root+'/workflow-executions/'+failed['id']+'/node-runs')['items']
            check(all(row['node_id']!='clock' for row in attempts),'failed expression diagnosis does not invent a node attempt')
        print(f'{checks} checks passed',flush=True)
    finally:
        proc.terminate(); proc.wait(timeout=20); log.close()
