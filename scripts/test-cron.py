#!/usr/bin/env python3
"""Real HTTP and runner scheduled-task acceptance; optional empty FERRY_TEST_DATABASE_URL."""
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import shlex
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
        proc = subprocess.Popen([binary,"--desktop-control"], env=env, stdout=log, stderr=log, stdin=subprocess.PIPE)
        for _ in range(150):
            if proc.poll() is not None: raise AssertionError((data / 'server.log').read_text())
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
        request(owner, '/workspaces', {'name':'Tenant','slug':'tenant'})
        request(outsider, '/workspaces', {'name':'Other','slug':'other'})
        root = '/workspaces/tenant'
        invitation=request(owner,root+'/members/invitation')['code']
        request(guest,'/workspaces/actions/join',{'code':invitation})
        group=request(owner,root+'/host-groups',{'name':'Cron group'},expected=201)['id']
        host=request(owner,root+'/hosts',{'name':'Cron runner','connection':'runner','group_id':group},expected=201)['id']
        token=request(owner,root+'/runner-tokens',{'host_id':host},expected=201)['token']
        runner=subprocess.Popen(['target/debug/ferry-runner','--server',base[:-4],'--token',token,'--state-dir',str(data/'runner')],stdout=log,stderr=log)
        def eventually(fn,timeout=30):
            until=time.monotonic()+timeout
            while time.monotonic()<until:
                value=fn()
                if value:return value
                time.sleep(.1)
            raise AssertionError('timed out waiting for expected state')
        eventually(lambda:request(owner,root+'/hosts/'+host)['state']=='online')
        eventually(lambda:(result[0]==200 and result[1].get('ok')) if (result:=request(owner,root+'/hosts/'+host+'/actions/test-connection',{},expected=None)) else False)
        def create(body,c=owner): return request(c,root+'/cron-jobs',body,expected=201)['id']
        def job(id):return request(owner,root+'/cron-jobs/'+id)
        def runs(id):return request(owner,root+'/cron-jobs/'+id+'/runs')['items']
        def execute(id):return request(owner,root+'/cron-jobs/'+id+'/actions/run-now',{},expected=202)['id']
        def run(id):return request(owner,root+'/cron-runs/'+id)
        def done(id):return eventually(lambda: (r if r['state'] not in ('queued','running') else None) if (r:=run(id)) else None)
        def update(id,**values):return request(owner,root+'/cron-jobs/'+id,dict(job(id),**values),'PATCH')
        body={'name':'Runner command','scope':'host','schedule':'* * * * *','timezone':'Asia/Shanghai','enabled':False,'action':'command','target_kind':'host','target_ref':host,'command':'printf CRON_OK','timeout_seconds':10,'max_retries':0,'retry_delay_seconds':1}
        cid=create(body)
        check(job(cid)['next_run_at']==0 and job(cid)['run_as_name']=='Owner','disabled job and execution identity persisted')
        request(guest,root+'/cron-jobs',body,expected=403)
        request(guest,root+'/cron-jobs/'+cid+'/actions/run-now',{},expected=403)
        request(outsider,'/workspaces/other/cron-jobs/'+cid,expected=404)
        request(outsider,'/workspaces/other/cron-jobs/'+cid+'/runs',expected=404)
        request(owner,root+'/hosts/'+host,method='DELETE',expected=409)
        check(True,'tenant, permission and target reference guards enforced')
        for changes in [{'schedule':'bad'},{'timezone':'Bad/Zone'},{'scope':'build'},{'timeout_seconds':0},{'max_retries':11}]:
            request(owner,root+'/cron-jobs',dict(body,**changes),expected=400)
        request(owner,root+'/cron-jobs/'+cid,dict(body,version=0),'PATCH',409)
        check(True,'schedule/action/timeout/retry validation and CAS enforced')
        first=execute(cid);finished=done(first)
        check(finished['state']=='succeeded' and 'CRON_OK' in finished['output'],'real runner command succeeds and records output')
        request(outsider,'/workspaces/other/cron-runs/'+first,expected=404)
        check('printf CRON_OK' in finished['config'],'run records immutable configuration snapshot')
        update(cid,command='printf UPDATED')
        check('printf CRON_OK' in run(first)['config'],'editing job does not alter old snapshot')
        grouped=create(dict(body,name='Group command',target_kind='host-group',target_ref=group))
        request(owner,root+'/host-groups/'+group,method='DELETE',expected=409)
        check(done(execute(grouped))['state']=='succeeded','real group command uses member HostChannel')
        counter=shlex.quote(str(data/'attempts'))
        retry=create(dict(body,name='Retry',command=f'n=$(cat {counter} 2>/dev/null || echo 0); n=$((n+1)); echo "$n" > {counter}; [ "$n" -ge 3 ]',max_retries=2))
        result=done(execute(retry))
        check(result['state']=='succeeded' and result['attempt']==3 and (data/'attempts').read_text().strip()=='3','configured retry succeeds on third real attempt')
        uncertain=create(dict(body,name='Timeout',command='sleep 3; echo LATE',timeout_seconds=1,max_retries=2))
        pending=execute(uncertain)
        request(owner,root+'/cron-jobs/'+uncertain,method='DELETE',expected=409)
        result=done(pending)
        check(result['state']=='needs-review' and result['attempt']==1,'uncertain timeout never retries despite configured retries')
        request(owner,root+'/cron-jobs/'+uncertain+'/actions/run-now',{},expected=409)
        request(owner,root+'/cron-runs/'+pending+'/actions/acknowledge',{'version':result['version'],'note':''},expected=400)
        time.sleep(3) # The remote sleep must finish before the operator confirms it.
        request(owner,root+'/cron-runs/'+pending+'/actions/acknowledge',{'version':result['version'],'note':'Remote command has finished.'})
        check(run(pending)['state']=='failed','operator acknowledgement resolves uncertain execution')
        request(owner,root+'/cron-jobs/'+uncertain,method='DELETE',expected=204)
        overlap=create(dict(body,name='Overlap',schedule='* * * * * *',enabled=True,command='sleep 2; echo SCHEDULED'))
        eventually(lambda:any(r['state']=='running' for r in runs(overlap)))
        request(owner,root+'/cron-jobs/'+overlap+'/actions/run-now',{},expected=409)
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
        # A scheduled identity must not retain permissions after its role is revoked.
        from datetime import datetime,timezone,timedelta
        role=request(owner,root+'/roles',{'name':'Scheduler','permissions':['cron-jobs.create','cron-jobs.update','cron-jobs.run','hosts.terminal']})['id']
        member=request(owner,root+'/members?q=guest')['items'][0]
        request(owner,root+'/members/'+member['id'],{'version':member['version'],'role_id':role},'PATCH')
        instant=datetime.now(timezone.utc)+timedelta(seconds=5)
        denied_marker=data/'must-not-run'
        revoked=create(dict(body,name='Revoked identity',enabled=True,schedule=f'{instant.second} {instant.minute} {instant.hour} * * *',timezone='UTC',command=f'touch {shlex.quote(str(denied_marker))}'),guest)
        viewer=next(r for r in request(owner,root+'/roles')['items'] if r['name']=='只读')
        request(owner,root+'/members/'+member['id'],{'version':member['version']+1,'role_id':viewer['id']},'PATCH')
        eventually(lambda:any(r['state']=='failed' for r in runs(revoked)))
        check(not denied_marker.exists(),'automatic trigger rechecks revoked actor permissions before executing')
        # Persist a frequent enabled schedule, stop the service while no command is active, and restart.
        missed=create(dict(body,name='Missed',schedule='* * * * * *',enabled=True,command='true'))
        proc.stdin.close();proc.wait(timeout=30)
        time.sleep(2)
        proc=launch()
        eventually(lambda:any(r['state']=='skipped' for r in runs(missed)))
        update(missed,enabled=False)
        check(any('停机' in run(r['id'])['output'] for r in runs(missed) if r['state']=='skipped'),'downtime occurrence recorded as skipped, not replayed')
        eventually(lambda:request(owner,root+'/hosts/'+host)['state']=='online')
        eventually(lambda:(result[0]==200 and result[1].get('ok')) if (result:=request(owner,root+'/hosts/'+host+'/actions/test-connection',{},expected=None)) else False)
        # Desktop-owned drain must wait for the active command and preserve its final result.
        marker=data/'drain-started';release=data/'drain-release'
        draining=create(dict(body,name='Drain',command=f'touch {shlex.quote(str(marker))}; while [ ! -f {shlex.quote(str(release))} ]; do sleep .1; done; echo DRAIN_DONE'))
        active=execute(draining);eventually(marker.exists)
        proc.stdin.close();time.sleep(.4)
        check(proc.poll() is None,'desktop drain waits for active scheduled command')
        release.write_text('release');proc.wait(timeout=30);proc=launch()
        check(run(active)['state']=='succeeded' and 'DRAIN_DONE' in run(active)['output'],'graceful shutdown preserves command completion before exit')
        events=request(owner,root+'/activity?resource=cron-jobs&page_size=200')['items']
        check(any(e['action']=='scheduled' for e in events) and all('command' not in e['details'] and 'CRON_OK' not in json.dumps(e['details']) for e in events),'scheduled and manual actions audited without script content')
        for id in [cid,grouped,retry,overlap,build,revoked,missed,draining]:
            eventually(lambda:all(r['state'] not in ('queued','running') for r in runs(id)))
            request(owner,root+'/cron-jobs/'+id,method='DELETE',expected=204)
        check(request(owner,root+'/cron-jobs')['total']==0,'inactive jobs and histories can be deleted')
        print(f'{checks} checks passed',flush=True)
    except Exception:
        log.flush();print((data/'server.log').read_text()[-5000:],flush=True)
        raise
    finally:
        if runner: runner.terminate();runner.wait(timeout=15)
        if proc.poll() is None:proc.terminate();proc.wait(timeout=30)
        log.close()
