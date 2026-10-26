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

requests=[]
index={'version':1,'templates':[{'key':'web','name':'Imported app','description':'fixture','content':{'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'busybox:1.36'}]},'parameters':[]}]}
fixture_status=200
class Receiver(BaseHTTPRequestHandler):
    def do_GET(self):
        import base64
        requests.append((self.path,{k.lower():v for k,v in self.headers.items()}))
        payload=json.dumps({'content':base64.b64encode(json.dumps(index).encode()).decode(),'encoding':'base64'}).encode()
        self.send_response(fixture_status);self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
    def log_message(self,*args):pass
receiver=ThreadingHTTPServer(('127.0.0.1',0),Receiver)
threading.Thread(target=receiver.serve_forever,daemon=True).start()
api_base=f'http://127.0.0.1:{receiver.server_port}'
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
        credential=request(owner,root+'/credentials',{'name':'Index token','kind':'token','secret':'fixture-provider-token'},expected=201)['id']
        repo=request(owner,root+'/repositories',{'name':'External repository','url':api_base+'/group/repository.git','credential_id':credential},expected=201)['id']
        def eventually(fn,timeout=30):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                value=fn()
                if value:return value
                time.sleep(.1)
            raise AssertionError('expected condition not reached')
        def source(id):return request(owner,root+'/marketplace-sources/'+id)
        def entries(id):return request(owner,root+'/marketplace?source_id='+id)['items']
        sources=[]
        body={'name':'Source','repository_id':repo,'provider':'github','api_base':api_base,'project':'group/repository','organization':'org','branch':'main','index_path':'marketplace.json'}
        for provider in ['github','gitlab','gitee','codeup']:
            sid=request(owner,root+'/marketplace-sources',dict(body,name=provider,provider=provider),expected=201)['id'];sources.append(sid)
            result=request(owner,root+'/marketplace-sources/'+sid+'/actions/sync',{})
            check(result['templates']==1 and source(sid)['state']=='idle',provider+' source sync validates and stores directory')
            path,headers=requests[-1]
            token=(headers.get('authorization')=='Bearer fixture-provider-token') if provider=='github' else headers.get('private-token')=='fixture-provider-token' if provider=='gitlab' else 'access_token=fixture-provider-token' in path if provider=='gitee' else headers.get('x-yunxiao-token')=='fixture-provider-token'
            check(token,provider+' transport uses documented authentication')
        sid=sources[0];entry=entries(sid)[0]
        request(guest,root+'/marketplace-sources',body,expected=403)
        request(guest,root+'/marketplace-sources/'+sid+'/actions/sync',{},expected=403)
        request(outsider,'/workspaces/other/marketplace/'+entry['id'],expected=404)
        request(owner,root+'/repositories/'+repo,method='DELETE',expected=409)
        request(owner,root+'/marketplace-sources',dict(body,api_base='http://example.invalid'),expected=400)
        check(True,'tenant, permission, repository references and credential-origin guards enforced')
        imported=request(owner,root+'/marketplace/'+entry['id']+'/actions/install',{'version':entry['version']},expected=201)['id']
        request(owner,root+'/marketplace/'+entry['id']+'/actions/install',{'version':0},expected=409)
        template=request(owner,root+'/app-templates/'+imported)
        rendered=request(owner,root+'/app-templates/'+imported+'/actions/render',{'version':1,'values':{}})
        check(json.loads(rendered['manifest'])['services'][0]['image']=='busybox:1.36','imported template is valid and uses real template renderer')
        index['templates'][0]['name']='Changed upstream'
        request(owner,root+'/marketplace-sources/'+sid+'/actions/sync',{})
        check(entries(sid)[0]['name']=='Changed upstream' and request(owner,root+'/app-templates/'+imported)==template,'sync atomically replaces directory without altering imported template')
        previous=entries(sid)
        index['templates'].append(dict(index['templates'][0]))
        request(owner,root+'/marketplace-sources/'+sid+'/actions/sync',{},expected=400)
        check(entries(sid)==previous and source(sid)['state']=='failed','invalid duplicate index preserves previous successful directory')
        index['templates'].pop();fixture_status=503
        request(owner,root+'/marketplace-sources/'+sid+'/actions/sync',{},expected=400)
        check('503' in source(sid)['last_error'] and entries(sid)==previous,'provider failure is visible and keeps previous directory')
        fixture_status=200
        job=request(owner,root+'/cron-jobs',{'name':'Market sync','scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'sync-marketplace','target_kind':'marketplace-source','target_ref':sid,'timeout_seconds':30},expected=201)['id']
        run=request(owner,root+'/cron-jobs/'+job+'/actions/run-now',{},expected=202)['id']
        eventually(lambda:request(owner,root+'/cron-runs/'+run)['state']=='succeeded')
        check(source(sid)['state']=='idle','shared scheduled task executor actually synchronizes market source')
        request(owner,root+'/marketplace-sources/'+sid,method='DELETE',expected=409)
        request(owner,root+'/cron-jobs/'+job,method='DELETE',expected=204)
        check('fixture-provider-token' not in json.dumps(request(owner,root+'/activity?page_size=200')) and 'fixture-provider-token' not in json.dumps(source(sid)),'source metadata and audit never reveal provider token')
        for source_id in sources:request(owner,root+'/marketplace-sources/'+source_id,method='DELETE',expected=204)
        check(request(owner,root+'/marketplace')['total']==0 and request(owner,root+'/app-templates/'+imported)==template,'source deletion removes catalog but preserves imported templates')
        request(owner,root+'/repositories/'+repo,method='DELETE',expected=204)
        print(f'{checks} checks passed (local provider fixtures; not live private-platform acceptance)',flush=True)
    except Exception:
        log.flush();print((data/'server.log').read_text()[-5000:],flush=True)
        raise
    finally:
        if proc.poll() is None:proc.terminate();proc.wait(timeout=30)
        log.close();receiver.shutdown();receiver.server_close()
