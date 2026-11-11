#!/usr/bin/env python3
"""Real HTTP and runner scheduled-task acceptance; optional empty FERRY_TEST_DATABASE_URL."""
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import shlex
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

def command(script):
    args=[sys.executable, '-c', script]
    return subprocess.list2cmdline(args) if os.name == 'nt' else shlex.join(args)

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
        proc = subprocess.Popen([binary,"--desktop-control"], env=env, stdout=log, stderr=log, stdin=subprocess.PIPE, creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0)
        for _ in range(150):
            if proc.poll() is not None: raise AssertionError((data / 'server.log').read_text(encoding='utf8',errors='replace'))
            try:
                if request(anonymous, '/status')['ok']: return proc
            except OSError: pass
            time.sleep(.1)
        raise AssertionError('server startup timed out')
    runner = None
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
        invitation=request(owner,root+'/members/invitation')['code']
        request(guest,'/workspace-memberships',{'code':invitation})
        group=request(owner,root+'/node-groups',{'name':'Cron group'},expected=201)['id']
        host=request(owner,root+'/nodes',{'name':'Cron runner','connection':'runner','group_id':group},expected=201)['id']
        token=request(owner,root+'/runner-tokens',{'host_id':host},expected=201)['token']
        runner=subprocess.Popen([os.environ.get('RUNNER_BINARY','target/debug/ferry-runner'),'--server',base[:-4],'--token',token,'--state-dir',str(data/'runner')],stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0)
        def eventually(fn,timeout=30):
            until=time.monotonic()+timeout
            while time.monotonic()<until:
                value=fn()
                if value:return value
                time.sleep(.1)
            raise AssertionError('timed out waiting for expected state')
        eventually(lambda:request(owner,root+'/nodes/'+host)['state']=='online')
        eventually(lambda:(result[0]==200 and result[1].get('ok')) if (result:=request(owner,root+'/nodes/'+host+'/connection-tests',{},expected=None)) else False)
        def create(body,c=owner): return request(c,root+'/cron-jobs',body,expected=201)['id']
        def job(id):return request(owner,root+'/cron-jobs/'+id)
        def runs(id):return request(owner,root+'/cron-jobs/'+id+'/runs')['items']
        def execute(id):return request(owner,root+'/cron-jobs/'+id+'/runs',{},expected=202)['id']
        def run(id):return request(owner,root+'/cron-runs/'+id)
        def done(id):return eventually(lambda: (r if r['state'] not in ('queued','running') else None) if (r:=run(id)) else None)
        def update(id,**values):return request(owner,root+'/cron-jobs/'+id,dict(job(id),**values),'PATCH')
        body={'name':'Runner command','scope':'host','schedule':'* * * * *','timezone':'Asia/Shanghai','enabled':False,'action':'command','target_kind':'host','target_ref':host,'command':command("print('CRON_OK')"),'timeout_seconds':10,'max_retries':0,'retry_delay_seconds':1}
        cid=create(body)
        check(job(cid)['next_run_at']==0 and job(cid)['run_as_name']=='Owner','disabled job and execution identity persisted')
        request(guest,root+'/cron-jobs',body,expected=403)
        request(guest,root+'/cron-jobs/'+cid+'/runs',{},expected=403)
        request(outsider,f'/workspaces/{workspace_other}/cron-jobs/'+cid,expected=404)
        request(outsider,f'/workspaces/{workspace_other}/cron-jobs/'+cid+'/runs',expected=404)
        request(owner,root+'/nodes/'+host,method='DELETE',expected=409)
        check(True,'tenant, permission and target reference guards enforced')
        for changes in [{'schedule':'bad'},{'timezone':'Bad/Zone'},{'scope':'build'},{'timeout_seconds':0},{'max_retries':11}]:
            request(owner,root+'/cron-jobs',dict(body,**changes),expected=400)
        request(owner,root+'/cron-jobs/'+cid,dict(body,version=0),'PATCH',409)
        check(True,'schedule/action/timeout/retry validation and CAS enforced')
        first=execute(cid);finished=done(first)
        check(finished['state']=='succeeded' and 'CRON_OK' in finished['output'],'real runner command succeeds and records output')
        request(outsider,f'/workspaces/{workspace_other}/cron-runs/'+first,expected=404)
        check(json.loads(finished['config'])['command']==body['command'],'run records immutable configuration snapshot')
        update(cid,command=command("print('UPDATED')"))
        check(json.loads(run(first)['config'])['command']==body['command'],'editing job does not alter old snapshot')
        grouped=create(dict(body,name='Group command',target_kind='host-group',target_ref=group))
        request(owner,root+'/node-groups/'+group,method='DELETE',expected=409)
        check(done(execute(grouped))['state']=='succeeded','real group command uses member HostChannel')
        counter=repr(str(data/'attempts'))
        retry=create(dict(body,name='Retry',command=command(f'from pathlib import Path; import sys; p=Path({counter}); n=int(p.read_text())+1 if p.exists() else 1; p.write_text(str(n)); sys.exit(0 if n>=3 else 1)'),max_retries=2))
        result=done(execute(retry))
        check(result['state']=='succeeded' and result['attempt']==3 and (data/'attempts').read_text().strip()=='3','configured retry succeeds on third real attempt')
        uncertain=create(dict(body,name='Timeout',command=command("import time; time.sleep(3); print('LATE')"),timeout_seconds=1,max_retries=2))
        pending=execute(uncertain)
        request(owner,root+'/cron-jobs/'+uncertain,method='DELETE',expected=409)
        result=done(pending)
        check(result['state']=='needs-review' and result['attempt']==1,'uncertain timeout never retries despite configured retries')
        request(owner,root+'/cron-jobs/'+uncertain+'/runs',{},expected=409)
        request(owner,root+'/cron-runs/'+pending+'/acknowledgements',{'version':result['version'],'note':''},expected=400)
        time.sleep(3) # The remote sleep must finish before the operator confirms it.
        request(owner,root+'/cron-runs/'+pending+'/acknowledgements',{'version':result['version'],'note':'Remote command has finished.'})
        check(run(pending)['state']=='failed','operator acknowledgement resolves uncertain execution')
        request(owner,root+'/cron-jobs/'+uncertain,method='DELETE',expected=204)
        overlap=create(dict(body,name='Overlap',schedule='* * * * * *',enabled=True,command=command("import time; time.sleep(2); print('SCHEDULED')")))
        eventually(lambda:any(r['state']=='running' for r in runs(overlap)))
        request(owner,root+'/cron-jobs/'+overlap+'/runs',{},expected=409)
        eventually(lambda:any(r['state']=='skipped' for r in runs(overlap)))
        update(overlap,enabled=False)
        eventually(lambda:all(r['state'] not in ('queued','running') for r in runs(overlap)))
        check(any(r['state']=='succeeded' for r in runs(overlap)),'schedule actually executes; overlap creates skipped history')
        pipe=request(owner,root+'/pipelines',{'name':'Cron pipeline','target_kind':'host','target_ref':host,'definition':json.dumps({'workdir':str(data/'build'),'steps':[{'name':'Build','run':'echo CRON_PIPELINE'}]})},expected=201)['id']
        build=create(dict(body,name='Scheduled pipeline',scope='build',action='pipeline',target_kind='pipeline',target_ref=pipe,command=''))
        result=done(execute(build))
        check(result['state']=='succeeded' and result['downstream_kind']=='pipeline-runs' and result['downstream_id']!='0','build schedule follows real pipeline run')
        check(request(owner,root+'/pipeline-runs/'+result['downstream_id'])['state']=='succeeded','child run really succeeds')
        request(owner,root+'/pipelines/'+pipe,method='DELETE',expected=409)
        check(request(owner,root+'/cron-jobs?scope=build')['total']==1 and request(owner,root+'/cron-jobs?scope=deploy')['total']==0,'shared resource filters three scopes')
        graph={'engine':'ferry-graph','schema':1,'settings':{'timezone':'UTC'},'nodes':[
            {'id':'start','name':'Scheduled entry','type':'core.trigger.manual','type_version':1,'position':[0,0],
             'params':{'input_schema':{'type':'object','properties':{'message':{'type':'string'}},'required':['message']}}}
        ],'connections':[],'pin_data':{'start':[{'json':{'message':'NOT_PRODUCTION_INPUT'}}]}}
        workflow=request(owner,root+'/workflows',{'name':'Cron workflow','definition':json.dumps(graph)},expected=201)
        workflow_body=dict(body,name='Scheduled workflow',scope='build',action='workflow',target_kind='workflow',target_ref=workflow['id'],command='',entry_node_id='start',inputs={'message':'Real scheduled input'})
        request(owner,root+'/cron-jobs',workflow_body,expected=400)
        request(owner,root+'/workflows/'+workflow['id']+'/publications',{'version':workflow['version'],'revision_id':workflow['draft_revision_id'],'name':'v1'},expected=201)
        request(owner,root+'/cron-jobs',dict(workflow_body,inputs={}),expected=400)
        request(owner,root+'/cron-jobs',dict(workflow_body,entry_node_id='missing'),expected=400)
        request(outsider,f'/workspaces/{workspace_other}/cron-jobs',workflow_body,expected=404)
        workflow_cron=create(workflow_body)
        result=done(execute(workflow_cron))
        child=request(owner,root+'/workflow-executions/'+result['downstream_id'])
        check(result['state']=='succeeded' and result['downstream_kind']=='workflow-executions' and result['workflow_id']==workflow['id'],'workflow cron links to its real child execution')
        check(child['state']=='succeeded' and child['mode']=='production' and child['trigger_kind']=='schedule' and child['revision_id']==workflow['draft_revision_id'],'published workflow runs in production with schedule identity')
        node=request(owner,root+'/workflow-executions/'+child['id']+'/node-runs')['items'][0]
        output=request(owner,root+'/workflow-executions/'+child['id']+'/node-runs/'+node['id']+'/output')
        check('Real scheduled input' in json.dumps(output) and 'NOT_PRODUCTION_INPUT' not in json.dumps(output),'scheduled workflow outputs real inputs instead of pinned test data')
        check(request(owner,root+'/cron-jobs?scope=build')['total']==2,'build schedules contain both pipelines and node workflows')
        update(workflow_cron,enabled=True,schedule='* * * * * *')
        automatic=eventually(lambda:next((r for r in runs(workflow_cron) if r['trigger_kind']=='scheduled' and r['state']=='succeeded'),None))
        update(workflow_cron,enabled=False)
        check(automatic['downstream_kind']=='workflow-executions','real timer automatically starts a published workflow')
        # A scheduled identity must not retain permissions after its role is revoked.
        from datetime import datetime,timezone,timedelta
        role=request(owner,root+'/roles',{'name':'Scheduler','permissions':['cron-jobs.create','cron-jobs.update','cron-jobs.run','hosts.terminal']})['id']
        member=request(owner,root+'/members?q=guest')['items'][0]
        request(owner,root+'/members/'+member['id'],{'version':member['version'],'role_id':role},'PATCH')
        instant=datetime.now(timezone.utc)+timedelta(seconds=5)
        denied_marker=data/'must-not-run'
        revoked=create(dict(body,name='Revoked identity',enabled=True,schedule=f'{instant.second} {instant.minute} {instant.hour} * * *',timezone='UTC',command=command(f'from pathlib import Path; Path({str(denied_marker)!r}).touch()')),guest)
        viewer=next(r for r in request(owner,root+'/roles')['items'] if r['name']=='只读')
        request(owner,root+'/members/'+member['id'],{'version':member['version']+1,'role_id':viewer['id']},'PATCH')
        eventually(lambda:any(r['state']=='failed' for r in runs(revoked)))
        check(not denied_marker.exists(),'automatic trigger rechecks revoked actor permissions before executing')
        # Persist a frequent enabled schedule, stop the service while no command is active, and restart.
        missed=create(dict(body,name='Missed',schedule='* * * * * *',enabled=True,command=command('pass')))
        proc.stdin.close();proc.wait(timeout=30)
        time.sleep(2)
        proc=launch()
        eventually(lambda:any(r['state']=='skipped' for r in runs(missed)))
        update(missed,enabled=False)
        check(any('停机' in run(r['id'])['output'] for r in runs(missed) if r['state']=='skipped'),'downtime occurrence recorded as skipped, not replayed')
        eventually(lambda:request(owner,root+'/nodes/'+host)['state']=='online')
        eventually(lambda:(result[0]==200 and result[1].get('ok')) if (result:=request(owner,root+'/nodes/'+host+'/connection-tests',{},expected=None)) else False)
        # Desktop-owned drain must wait for the active command and preserve its final result.
        marker=data/'drain-started';release=data/'drain-release'
        draining=create(dict(body,name='Drain',command=command(f'from pathlib import Path; import time; Path({str(marker)!r}).touch(); release=Path({str(release)!r}); exec("while not release.exists():\\n time.sleep(.1)"); print("DRAIN_DONE")')))
        active=execute(draining);eventually(marker.exists)
        proc.stdin.close();time.sleep(.4)
        check(proc.poll() is None,'desktop drain waits for active scheduled command')
        release.write_text('release');proc.wait(timeout=30);proc=launch()
        check(run(active)['state']=='succeeded' and 'DRAIN_DONE' in run(active)['output'],'graceful shutdown preserves command completion before exit')
        events=request(owner,root+'/activity?resource=cron-jobs&page_size=200')['items']
        check(any(e['action']=='scheduled' for e in events) and all('command' not in e['details'] and 'CRON_OK' not in json.dumps(e['details']) for e in events),'scheduled and manual actions audited without script content')
        for id in [cid,grouped,retry,overlap,build,workflow_cron,revoked,missed,draining]:
            eventually(lambda:all(r['state'] not in ('queued','running') for r in runs(id)))
            request(owner,root+'/cron-jobs/'+id,method='DELETE',expected=204)
        check(request(owner,root+'/cron-jobs')['total']==0,'inactive jobs and histories can be deleted')
        print(f'{checks} checks passed',flush=True)
    except Exception:
        log.flush();print((data/'server.log').read_text(encoding='utf8',errors='replace')[-5000:],flush=True)
        raise
    finally:
        if runner: runner.terminate();runner.wait(timeout=15)
        if proc.poll() is None:
            if proc.stdin and not proc.stdin.closed: proc.stdin.close()
            proc.wait(timeout=30)
        log.close()
