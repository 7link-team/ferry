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
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

notifications=[]
slow_started=threading.Event()
slow_release=threading.Event()
class Receiver(BaseHTTPRequestHandler):
    def do_POST(self):
        payload=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        notifications.append((self.path,payload))
        if self.path=='/slow':
            slow_started.set();slow_release.wait(8)
        self.send_response(503 if self.path=='/fail' else 204);self.end_headers()
    def log_message(self,*args):pass
receiver=ThreadingHTTPServer(('127.0.0.1',0),Receiver)
threading.Thread(target=receiver.serve_forever,daemon=True).start()
webhook=f'http://127.0.0.1:{receiver.server_port}/receive?token=fixture-private-token'
checks = 0
def check(condition, label):
    global checks
    assert condition, label
    checks += 1
    print('PASS', label, flush=True)

def client():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

with tempfile.TemporaryDirectory(prefix='alert-test-') as directory:
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
        def eventually(fn,timeout=30):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                value=fn()
                if value:return value
                time.sleep(.1)
            raise AssertionError('expected condition not reached')
        def create(body,c=owner):return request(c,root+'/alert-rules',body,expected=201)['id']
        def rule(id):return request(owner,root+'/alert-rules/'+id)
        def events(id):return request(owner,root+'/alert-events?rule_id='+id)['items']
        def update(id,**values):return request(owner,root+'/alert-rules/'+id,dict(rule(id),**values),'PATCH')
        body={'name':'Offline','target_kind':'host','target_ref':host,'metric':'offline','threshold':0,'enabled':True,'webhook':webhook}
        rid=create(body)
        request(guest,root+'/alert-rules',body,expected=403)
        request(outsider,'/workspaces/other/alert-rules/'+rid,expected=404)
        request(owner,root+'/hosts/'+host,method='DELETE',expected=409)
        check(True,'tenant, permission and target reference protection')
        for change in [{'metric':'bad'},{'webhook':'file:///etc/passwd'},{'webhook':'https://user:password@example.invalid'},{'metric':'memory-percent','threshold':101}]:
            request(owner,root+'/alert-rules',dict(body,**change),expected=400)
        request(owner,root+'/alert-rules/'+rid,dict(body,version=0),'PATCH',409)
        check(True,'validation and optimistic concurrency enforced')
        check(rule(rid)['has_webhook'] and 'fixture-private-token' not in json.dumps(rule(rid)),'webhook address never returned')
        if not os.environ.get('FERRY_TEST_DATABASE_URL'):
            import sqlite3
            with sqlite3.connect(data/'ferry.sqlite') as conn:
                stored=conn.execute('SELECT webhook FROM alert_rules WHERE id=?',(rid,)).fetchone()[0]
                check(stored.startswith('v1.') and 'fixture-private-token' not in stored,'webhook stored as authenticated ciphertext')
        event=eventually(lambda:next((e for e in events(rid) if 'HTTP 204' in e['trigger_delivery']),None))
        check(event['state']=='firing','offline host triggers a real event and webhook')
        request(outsider,'/workspaces/other/alert-events/'+event['id'],expected=404)
        time.sleep(4.5)
        check(len(events(rid))==1 and len(notifications)==1,'continuous abnormality produces one event and one notification')
        runner=subprocess.Popen(['target/debug/ferry-runner','--server',base[:-4],'--token',token,'--state-dir',str(data/'runner')],stdout=log,stderr=log)
        event=eventually(lambda:next((e for e in events(rid) if e['state']=='recovered' and 'HTTP 204' in e['recovery_delivery']),None))
        check(event['recovered_at']>=event['triggered_at'] and len(notifications)==2,'real runner connection recovers event and sends one recovery notification')
        check([n[1]['event'] for n in notifications]==['triggered','recovered'],'webhook payload distinguishes trigger and recovery')
        check('fixture-private-token' not in json.dumps([n[1] for n in notifications]),'notification payload contains no webhook credentials')
        update(rid,name='Edited',webhook='')
        check(rule(rid)['has_webhook'],'blank edit preserves encrypted webhook')
        update(rid,clear_webhook=True)
        check(not rule(rid)['has_webhook'],'explicit clear removes webhook')
        metric=create(dict(body,name='Memory',metric='memory-percent',threshold=0,webhook=webhook.replace('/receive?token=fixture-private-token','/fail')))
        event=eventually(lambda:next((e for e in events(metric) if 'HTTP 503' in e['trigger_delivery']),None),45)
        before=len(notifications);time.sleep(4.5)
        check(len(notifications)==before and event['state']=='firing','actual runner memory metrics trigger; webhook failure is recorded without retry')
        update(metric,threshold=100)
        eventually(lambda:any(e['state']=='recovered' and 'HTTP 503' in e['recovery_delivery'] for e in events(metric)))
        check(True,'threshold recovery sends exactly one failed recovery attempt')
        job=request(owner,root+'/cron-jobs',{'name':'Failing job','scope':'host','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'command','target_kind':'host','target_ref':host,'command':'exit 7','timeout_seconds':10},expected=201)['id']
        execution=create(dict(body,name='Job failed',target_kind='cron-job',target_ref=job,metric='execution-failed',webhook=''))
        request(owner,root+'/cron-jobs/'+job+'/actions/run-now',{},expected=202)
        eventually(lambda:any(e['state']=='firing' for e in events(execution)))
        check(True,'real scheduled command failure triggers execution alert')
        old=request(owner,root+'/cron-jobs/'+job)
        request(owner,root+'/cron-jobs/'+job,dict(old,command='true'),'PATCH')
        request(owner,root+'/cron-jobs/'+job+'/actions/run-now',{},expected=202)
        eventually(lambda:any(e['state']=='recovered' for e in events(execution)))
        check(True,'subsequent actual success recovers execution alert')
        request(owner,root+'/cron-jobs/'+job,method='DELETE',expected=409)
        pipeline=request(owner,root+'/pipelines',{'name':'Monitored pipeline','target_kind':'host','target_ref':host,'definition':json.dumps({'workdir':str(data/'pipeline'),'steps':[{'name':'Failure','run':'exit 9'}]})},expected=201)['id']
        pipeline_rule=create(dict(body,name='Pipeline failure',target_kind='pipeline',target_ref=pipeline,metric='execution-failed',webhook=''))
        request(owner,root+'/pipelines/'+pipeline+'/actions/run',{},expected=201)
        eventually(lambda:any(e['state']=='firing' for e in events(pipeline_rule)))
        old=request(owner,root+'/pipelines/'+pipeline)
        request(owner,root+'/pipelines/'+pipeline,dict(old,definition=json.dumps({'workdir':str(data/'pipeline'),'steps':[{'name':'Success','run':'true'}]})),'PATCH')
        request(owner,root+'/pipelines/'+pipeline+'/actions/run',{},expected=201)
        eventually(lambda:any(e['state']=='recovered' for e in events(pipeline_rule)))
        check(True,'real pipeline failure and success drive alert transitions')
        dashboard=request(owner,root+'/dashboard')
        check(dashboard['counts']['alert-rules']==4 and dashboard['states']['alert-events']['recovered']==4,'dashboard includes real authorized alert counts and states')
        audit=request(owner,root+'/activity?page_size=200')['items']
        check('fixture-private-token' not in json.dumps(audit) and any(e['action']=='alert-triggered' for e in audit),'transitions audited without webhook address')
        for id in [rid,metric,execution,pipeline_rule]:request(owner,root+'/alert-rules/'+id,method='DELETE',expected=204)
        check(request(owner,root+'/alert-events')['total']==4,'deleting rules preserves event history')
        role=request(owner,root+'/roles',{'name':'Alert editor','permissions':['alerts.create','hosts.read']})['id']
        member=request(owner,root+'/members?q=guest')['items'][0]
        request(owner,root+'/members/'+member['id'],{'version':member['version'],'role_id':role},'PATCH')
        revoked=create(dict(body,name='Revoked monitor',webhook=''),guest)
        request(owner,root+'/roles/'+role,{'name':'Alert editor','version':1,'permissions':[]},'PATCH')
        eventually(lambda:rule(revoked)['last_error'])
        check(events(revoked)==[],'monitoring rechecks revoked actor permissions and reports configuration error')
        request(owner,root+'/alert-rules/'+revoked,method='DELETE',expected=204)
        if not os.environ.get('FERRY_TEST_DATABASE_URL'):
            proc.stdin.close();proc.wait(timeout=30)
            with sqlite3.connect(data/'ferry.sqlite') as conn:
                old=int(time.time()*1000)-3*86400000
                conn.execute('UPDATE alert_events SET recovered_at=?',(old,))
                retained=conn.execute('SELECT id FROM alert_events ORDER BY id LIMIT 1').fetchone()[0]
                conn.execute("UPDATE alert_events SET state='firing',trigger_delivery='投递中' WHERE id=?",(retained,))
            count=len(notifications);proc=launch()
            eventually(lambda:'不确定' in request(owner,root+'/alert-events/'+str(retained))['trigger_delivery'])
            check(request(owner,root+'/alert-events')['total']==4,'default retention preserves old event history')
            time.sleep(2.5)
            check(len(notifications)==count,'restart marks interrupted webhook uncertain without replay')
            setting=next(r for r in request(owner,root+'/settings')['items'] if r['key']=='history_retention_days')
            request(owner,root+'/settings/'+setting['id'],{'version':setting['version'],'value':1},'PATCH')
            proc.stdin.close();proc.wait(timeout=30);proc=launch()
            eventually(lambda:request(owner,root+'/alert-events')['total']==1)
            check(request(owner,root+'/alert-events/'+str(retained))['state']=='firing','configured retention removes expired recovered events and protects active event')
        drain_host=request(owner,root+'/hosts',{'name':'Drain monitor target','connection':'runner'},expected=201)['id']
        drain_rule=create(dict(body,name='Drain delivery',target_ref=drain_host,webhook=f'http://127.0.0.1:{receiver.server_port}/slow'))
        eventually(slow_started.is_set)
        proc.stdin.close();time.sleep(.4)
        check(proc.poll() is None,'desktop shutdown waits for in-flight webhook delivery')
        slow_release.set();proc.wait(timeout=30);proc=launch()
        eventually(lambda:any('HTTP 204' in e['trigger_delivery'] for e in events(drain_rule)))
        check(len(events(drain_rule))==1,'graceful webhook completion persists without duplicate event after restart')
        print(f'{checks} checks passed',flush=True)
    except Exception:
        log.flush();print((data/'server.log').read_text()[-5000:],flush=True)
        raise
    finally:
        if runner:runner.terminate();runner.wait(timeout=15)
        if proc.poll() is None:proc.terminate();proc.wait(timeout=30)
        log.close()
        slow_release.set()
        receiver.shutdown();receiver.server_close()
