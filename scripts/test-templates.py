#!/usr/bin/env python3
"""Real HTTP template acceptance; optional empty FERRY_TEST_DATABASE_URL."""
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
        workspace_tenant = request(owner, '/workspaces', {'name':'Tenant','slug':'tenant'})['id']
        workspace_other = request(outsider, '/workspaces', {'name':'Other','slug':'other'})['id']
        root = f'/workspaces/{workspace_tenant}'
        invitation=request(owner,root+'/members/invitation')['code']
        request(guest,'/workspace-memberships',{'code':invitation})
        host=request(owner,root+'/nodes',{'name':'Fixture','connection':'runner'},expected=201)['id']
        request(owner,root+'/variables',{'name':'token','scope':'deploy','kind':'secret','value':'never-copy-this-secret'},expected=201)
        for resource, field in [('app-templates','manifest'),('pipeline-templates','definition')]:
            app=field=='manifest'
            content=({'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'nginx:{{ .Values.tag }}','ports':[{'container':'{{ .Values.port }}'}],'env':[{'name':'TOKEN','valueFrom':'{{ .Values.token }}'}]}]} if app else {'workdir':'/tmp/build/{{run}}','steps':[{'name':'build','run':'{{ .Values.command }}','timeout_seconds':'{{ .Values.timeout }}'}]})
            params=([{'name':'tag','type':'string','default':'alpine','enum':['alpine','latest']},{'name':'port','type':'integer','default':8080,'min':1,'max':65535},{'name':'token','type':'string','secret':True}] if app else [{'name':'command','type':'string','min_length':1},{'name':'timeout','type':'integer','default':30,'min':1,'max':3600}])
            values={'token':'token'} if app else {'command':'printf \"hello\\n\"'}
            body={'name':'Reusable '+resource,'description':'searchable fixture','content':json.dumps(content),'parameters':json.dumps(params)}
            path=root+'/'+resource
            tid=request(owner,path,body,expected=201)['id']; url=path+'/'+tid
            row=request(owner,url)
            check(row['version']==1 and json.loads(row['content'])==content,resource+' create/read')
            listing=request(owner,path+'?q=searchable&page_size=1')
            check(listing['total']==1 and len(listing['items'])==1,resource+' search/pagination')
            request(guest,path,body,expected=403)
            request(guest,url,dict(body,version=1),method='PATCH',expected=403)
            request(guest,url,method='DELETE',expected=403)
            foreign=f'/workspaces/{workspace_other}/'+resource+'/'+tid
            for method,payload in [('GET',None),('PATCH',dict(body,version=1)),('DELETE',None)]:
                request(outsider,foreign,payload,method,404)
            request(outsider,foreign+'/renderings',{'version':1,'values':values},expected=404)
            rendered=request(guest,url+'/renderings',{'version':1,'values':values})[field]
            result=json.loads(rendered)
            if app:
                check(result['services'][0]['ports'][0]['container']==8080 and result['services'][0]['env'][0]['valueFrom']=='variable/token','typed defaults and secret references')
                obj={'name':'Generated app','mode':'compose','target_kind':'host','target_ref':host,'workdir':'/tmp/deployments','manifest':rendered}
                target='deployments'
            else:
                check(result['steps'][0]['run']==values['command'] and result['workdir'].endswith('{{run}}'),'escaped input and runtime placeholder retained')
                obj={'name':'Generated pipeline','target_kind':'host','target_ref':host,'definition':rendered}
                target='pipelines'
            oid=request(owner,root+'/'+target,obj,expected=201)['id']
            saved=request(owner,root+'/'+target+'/'+oid)[field]
            persisted=json.loads(saved)
            check(persisted['services'][0]['image']==result['services'][0]['image'] if app else persisted==result,resource+' generated configuration creates real resource')
            for invalid in [{},dict(values,unknown='x'),dict(values,**({'port':'80'} if app else {'timeout':'30'})),dict(values,**({'tag':'bad'} if app else {'command':''})),dict(values,**({'port':0} if app else {'timeout':0}))]:
                request(owner,url+'/renderings',{'version':1,'values':invalid},expected=400)
            check(True,resource+' missing/unknown/type/range validation')
            for invalid_params in [params+[{'name':'unused','type':'string'}],params+[params[0]], [dict(p,default=None) for p in params]]:
                request(owner,path,dict(body,parameters=json.dumps(invalid_params)),expected=400)
            bad_content=json.loads(json.dumps(content).replace('{{ .Values.'+('tag' if app else 'command')+' }}','{{ unsupported }}'))
            request(owner,path,dict(body,content=json.dumps(bad_content)),expected=400)
            if app:
                bad=json.loads(json.dumps(content)); bad['services'][0]['env'][0]={'name':'TOKEN','value':'{{ .Values.token }}'}
                request(owner,path,dict(body,content=json.dumps(bad)),expected=400)
            else:
                request(owner,path,dict(body,parameters=json.dumps([dict(p,secret=True) for p in params])),expected=400)
            check(True,resource+' invalid declarations and secret locations rejected')
            copy=request(owner,path,dict(body,name='Copy'),expected=201)['id']
            request(owner,path+'/'+copy,method='DELETE',expected=204)
            request(owner,url,dict(body,version=0),method='PATCH',expected=409)
            request(owner,url,dict(body,version=1,name='Revised'),method='PATCH')
            request(owner,url+'/renderings',{'version':1,'values':values},expected=409)
            request(owner,url+'/renderings',{'version':2,'values':values})
            request(owner,url,method='DELETE',expected=204)
            request(owner,url,expected=404)
            check(request(owner,root+'/'+target+'/'+oid)[field]==saved,resource+' update/delete leaves created object independent')
        member=request(owner,root+'/members?q=guest')['items'][0]
        role=request(owner,root+'/roles',{'name':'No template access','permissions':['dashboard.read']})['id']
        request(owner,root+'/members/'+member['id'],{'version':member['version'],'role_id':role},method='PATCH')
        for resource in ['app-templates','pipeline-templates']:
            request(guest,root+'/'+resource,expected=403)
            request(guest,root+'/'+resource+'/1/renderings',{'version':1},expected=403)
        check(True,'read permission required for list/render')
        activities=request(owner,root+'/activity?page_size=200')['items']
        logs=[r for r in activities if r['resource'] in ('app-templates','pipeline-templates')]
        check(len(logs)==10 and all('content' not in json.dumps(r) and 'never-copy-this-secret' not in json.dumps(r) for r in logs),'template CRUD audited without configuration or secret values')
        check(request(owner,root+'/pipeline-runs')['total']==0,'render/create does not execute pipeline')
        config_body={'name':'Runtime config','content':'port={{ .Values.port }}\nsecret={{ .Values.secret }}','parameters':json.dumps([{'name':'port','type':'integer','default':8080},{'name':'secret','type':'string','secret':True,'default':'token'}])}
        config_path=root+'/config-templates'
        config_id=request(owner,config_path,config_body,expected=201)['id']
        rendered=request(owner,config_path+'/'+config_id+'/renderings',{'version':1})['content']
        check(rendered=='port=8080\nsecret=<variable/token>','config render exposes reference, never secret plaintext')
        request(guest,config_path,config_body,expected=403)
        request(outsider,f'/workspaces/{workspace_other}/config-templates/'+config_id,expected=404)
        check(True,'config templates enforce role and workspace boundaries')
        config_manifest={'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'configured'},'services':[{'name':'web','image':'busybox:1.36','files':[{'path':'/etc/app.conf','fromTemplate':config_id}]}]}
        config_app=request(owner,root+'/deployments',{'name':'Configured app','mode':'compose','target_kind':'host','target_ref':host,'workdir':'/tmp/config-app','manifest':json.dumps(config_manifest)},expected=201)['id']
        request(owner,config_path+'/'+config_id,method='DELETE',expected=409)
        request(owner,config_path+'/'+config_id,dict(config_body,name='Renamed',version=1),method='PATCH',expected=409)
        check(True,'referenced config templates cannot be removed or renamed')
        request(owner,config_path+'/'+config_id,dict(config_body,version=1,content='new={{ .Values.port }}:{{ .Values.secret }}'),method='PATCH')
        request(owner,config_path+'/'+config_id,dict(config_body,version=1),method='PATCH',expected=409)
        check(True,'config content updates use optimistic version checks')
        request(owner,root+'/deployments/'+config_app,method='DELETE',expected=204)
        request(owner,config_path+'/'+config_id,method='DELETE',expected=204)
        check(True,'config template removable after current references are removed')
        print(f'{checks} checks passed',flush=True)
    finally:
        proc.terminate(); proc.wait(timeout=20); log.close()
