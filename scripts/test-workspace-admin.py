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
            method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
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
        proc = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
        for _ in range(150):
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
        request(owner, '/workspaces', {'name':'Tenant','slug':'tenant'})
        request(outsider, '/workspaces', {'name':'Other','slug':'other'})
        root = '/workspaces/tenant'
        code = request(owner, root+'/members/invitation')['code']
        rotated = request(owner, root+'/members/invitation', method='POST')['code']
        check(code != rotated, 'invitation rotation changes secret')
        request(guest, '/workspaces/actions/join', {'code':code}, expected=400)
        request(guest, '/workspaces/actions/join', {'code':rotated})
        request(guest, '/workspaces/actions/join', {'code':rotated}, expected=409)
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
        foreign=request(outsider,'/workspaces/other/roles')['items'][0]['id']
        request(owner,root+'/roles/'+foreign,expected=404)
        request(owner,root+'/members/'+mid,{'version':1,'role_id':foreign},'PATCH',404)
        request(owner,root+'/members/'+mid,{'version':1,'role_id':rid},'PATCH')
        request(owner,root+'/members/'+mid,{'version':1,'role_id':rid},'PATCH',409)
        request(owner,root+'/roles/'+rid,method='DELETE',expected=409)
        check(request(guest,'/me')['workspaces'][0]['role_name']=='Operator revised','role assignment effective without re-login')
        summary=request(guest,root+'/dashboard')
        check(summary['counts']=={'hosts':0} and summary['recent'] is None and summary['failures_24h']=={}, 'dashboard hides unauthorized metrics and activities')
        host_id=request(guest,root+'/hosts',{'name':'Runner','connection':'runner'},expected=201)['id']
        summary=request(owner,root+'/dashboard')
        check(summary['counts']['hosts']==1 and summary['counts']['members']==2 and summary['counts']['roles']==6, 'dashboard counts reflect real tenant rows')
        check(summary['states']['hosts']['unknown']==1 and summary['runners'][0]['seen_at'] is None and summary['runners'][0]['lag_ms'] is None, 'unreported runner is not shown as healthy')
        check(len(summary['recent'])>0 and all('details' not in a for a in summary['recent']), 'recent activities expose safe summary')
        check(request(outsider,'/workspaces/other/dashboard')['counts']['hosts']==0,'dashboard tenant isolation')
        pipeline_id=request(owner,root+'/pipelines',{'name':'Dashboard failure','target_kind':'host','target_ref':host_id,'definition':json.dumps({'workdir':'/tmp/workspace-test','steps':[{'name':'offline','run':'true'}]})},expected=201)['id']
        run_id=request(owner,root+'/pipelines/'+pipeline_id+'/actions/run',{},expected=201)['id']
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
        print(f'{checks} checks passed',flush=True)
    finally:
        proc.terminate(); proc.wait(timeout=20); log.close()
