#!/usr/bin/env python3
"""Real HTTP maintenance acceptance; optional empty FERRY_TEST_DATABASE_URL."""
from contextlib import closing
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
        settings=request(owner,root+'/settings')['items']
        check(len(settings)==5,'five typed settings seeded')
        import urllib.parse
        check(request(owner,root+'/settings?q='+urllib.parse.quote('默认每页条数'))['total']==1,
            'builtin settings searchable by displayed names')
        concurrent=next(r for r in settings if r['key']=='pipeline_concurrent_runs')
        check((concurrent['value'],concurrent['default'],concurrent['min'])==(10,10,0),'pipeline concurrency defaults to 10 runs, 0 meaning unlimited')
        event_retention=next(r for r in settings if r['key']=='repository_event_retention_days')
        check((event_retention['value'],event_retention['default'],event_retention['min'],event_retention['max'])==(0,0,0,36500),'repository event retention defaults to permanent with a bounded range')
        prefs=request(guest,root+'/preferences')
        check(prefs['default_page_size']==20 and len(prefs['dictionaries']['host-state'])==3,'members receive presentation preferences')
        setting=next(r for r in settings if r['key']=='default_page_size'); sid=setting['id']
        request(guest,root+'/settings/'+sid,{'version':1,'value':37},'PATCH',403)
        request(outsider,f'/workspaces/{workspace_other}/settings/'+sid,expected=404)
        request(owner,root+'/settings/'+sid,{'version':1,'value':201},'PATCH',400)
        request(owner,root+'/settings/'+sid,{'version':1,'value':37},'PATCH')
        request(owner,root+'/settings/'+sid,{'version':1,'value':38},'PATCH',409)
        check(request(guest,root+'/preferences')['default_page_size']==37,'updated page size is applied to preferences')
        check(request(outsider,f'/workspaces/{workspace_other}/preferences')['default_page_size']==20,'settings tenant isolation')
        body={'key':'custom','name':'Custom','items':json.dumps([{'value':'a','label':'Alpha'},{'value':'b','label':'Beta'}])}
        did=request(owner,root+'/dictionaries',body,expected=201)['id'];url=root+'/dictionaries/'+did
        request(guest,root+'/dictionaries',body,expected=403)
        request(owner,root+'/dictionaries',body,expected=409)
        request(owner,root+'/dictionaries',dict(body,key='invalid',items=json.dumps([{'value':'a','label':'A'},{'value':'a','label':'B'}])),expected=400)
        check(json.loads(request(owner,url)['items'])[0]['label']=='Alpha','dictionary items and order persisted')
        request(owner,url,dict(body,version=1,key='renamed'),'PATCH',400)
        request(owner,url,dict(body,version=1,items=json.dumps([{'value':'b','label':'Revised'}])),'PATCH')
        request(owner,url,dict(body,version=1),'PATCH',409)
        check(json.loads(request(owner,url)['items'])==[{'value':'b','label':'Revised'}],'dictionary update replaces children atomically')
        request(outsider,f'/workspaces/{workspace_other}/dictionaries/'+did,expected=404)
        check(request(owner,root+'/dictionaries?q=custom')['total']==1,'dictionary search works')
        builtin=next(r for r in request(owner,root+'/dictionaries')['items'] if r['key']=='host-state')
        built=request(owner,root+'/dictionaries/'+builtin['id'])
        changed=json.loads(built['items']);changed[0]['label']='Connected'
        request(owner,root+'/dictionaries/'+builtin['id'],dict(built,items=json.dumps(changed)),'PATCH')
        check(request(guest,root+'/preferences')['dictionaries']['host-state'][0]['label']=='Connected','dictionary labels consumed by member preferences')
        request(owner,root+'/dictionaries/'+builtin['id'],dict(built,version=2,items='[]'),'PATCH',400)
        request(owner,root+'/dictionaries/'+builtin['id'],method='DELETE',expected=400)
        request(owner,url,method='DELETE',expected=204)
        request(owner,url,expected=404)
        check(True,'builtin machine states protected and custom dictionaries deletable')
        events=request(owner,root+'/activity?resource=dictionaries')['items']
        check(len(events)==4,'dictionary writes audited')
        typed_samples = {
            'text': 'A typed value', 'num': 0, 'tab': [{'key':'a','label':'Alpha'}],
            'selection': [{'value':'a','label':'Alpha'}], 'image': [], 'video': [], 'file': [],
            'code': 'print(1)', 'markdown': '# Heading', 'html': '<p>HTML</p>',
            'richtext': '<p>Rich text</p>', 'json': {'enabled':True,'count':2},
        }
        typed_ids = {}
        for resource in ['dictionaries', 'settings']:
            for kind, value in typed_samples.items():
                typed_body = {'key':'typed_'+kind,'name':'Typed '+kind,'description':'Typed acceptance',
                    'category':'acceptance','value_type':kind,'value_json':json.dumps(value),
                    'schema_json':json.dumps({'min':0,'max':0,'decimals':0} if kind=='num' else {})}
                typed_id=request(owner,root+'/'+resource,typed_body,expected=201)['id']
                typed_url=root+'/'+resource+'/'+typed_id
                typed_ids[(resource,kind)]=typed_id
                row=request(owner,typed_url)
                check(row['value_type']==kind and row['category']=='acceptance'
                    and json.loads(row['value_json'])==value,resource+' '+kind+' typed value round trip')
                request(guest,typed_url,dict(typed_body,version=1),'PATCH',403)
                request(outsider,f'/workspaces/{workspace_other}/'+resource+'/'+typed_id,expected=404)
                request(owner,typed_url,dict(typed_body,version=1,description='Updated'),'PATCH')
                request(owner,typed_url,dict(typed_body,version=1),'PATCH',409)
            filtered=request(owner,root+'/'+resource+'?category=acceptance&value_type=json')
            check(filtered['total']==1 and filtered['items'][0]['key']=='typed_json',resource+' filters category and type')
            for sort_field in ['key','name','category']:
                for direction in ['','-']:
                    request(owner,root+'/'+resource+'?sort='+direction+sort_field)
            check(True,resource+' supports every frontend ascending/descending sort')
            invalid={'key':'invalid_typed','name':'Invalid','category':'acceptance','value_type':'num',
                'value_json':'-1','schema_json':'{"min":0}'}
            request(owner,root+'/'+resource,invalid,expected=400)
            request(owner,root+'/'+resource,dict(invalid,value_json='1',schema_json='{"max":0}'),expected=400)
            request(owner,root+'/'+resource,dict(invalid,value_json='1.25',schema_json='{"decimals":1}'),expected=400)
            request(owner,root+'/'+resource,dict(invalid,value_type='json',value_json='{broken'),expected=400)
            request(owner,root+'/'+resource,dict(invalid,value_type='image',value_json='[{"url":"javascript:alert(1)"}]',schema_json='{}'),expected=400)
            request(owner,root+'/'+resource,dict(invalid,value_type='file',value_json='[{"id":"fake","url":"/api/fake","size":1}]',schema_json='{}'),expected=400)
            check(True,resource+' validates zero bounds, decimals, JSON and media references')
            remove_url=root+'/'+resource+'/'+typed_ids[(resource,'text')]
            request(guest,remove_url,method='DELETE',expected=403)
            request(owner,remove_url,method='DELETE',expected=204)
            request(owner,remove_url,expected=404)
        current_setting=request(owner,root+'/settings/'+sid)
        request(owner,root+'/settings/'+sid,dict(current_setting,value_type='text',value_json='"unsafe"'),'PATCH',400)
        request(owner,root+'/settings/'+sid,dict(current_setting,value_json='999',schema_json='{}'),'PATCH',400)
        request(owner,root+'/settings/'+sid,dict(current_setting,name='Changed',category='Changed',description='Editable system description'),'PATCH')
        current_setting=request(owner,root+'/settings/'+sid)
        check(current_setting['name']=='Changed' and current_setting['category']=='Changed' and current_setting['description']=='Editable system description','builtin setting descriptive metadata is editable')
        request(owner,root+'/settings/'+sid,dict(current_setting,value_json='37',schema_json='{}'),'PATCH',400)
        request(owner,root+'/settings/'+sid,method='DELETE',expected=400)
        check(request(guest,root+'/preferences')['default_page_size']==37,'builtin setting values and constraints remain authoritative')
        check('typed_json' not in request(guest,root+'/preferences')['dictionaries'],'custom typed contents not exposed in member preferences')
        if not os.environ.get('FERRY_TEST_DATABASE_URL'):
            import sqlite3
            host=request(owner,root+'/nodes',{'name':'Offline','connection':'runner'},expected=201)['id']
            pid=request(owner,root+'/pipelines',{'name':'Retention','target_kind':'host','target_ref':host,'definition':json.dumps({'workdir':'/tmp/retention','steps':[{'name':'test','run':'true'}]})},expected=201)['id']
            rid=request(owner,root+'/pipelines/'+pid+'/runs',{},expected=201)['id']
            for _ in range(100):
                run=request(owner,root+'/pipeline-runs/'+rid)
                if run['state']=='failed': break
                time.sleep(.1)
            check(run['state']=='failed','real failed execution exists before retention')
            manifest=json.dumps({'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'nginx:alpine'}]})
            appbody={'name':'Retention app','mode':'compose','target_kind':'host','target_ref':host,'workdir':'/tmp/retention','manifest':manifest}
            app=request(owner,root+'/deployments',appbody,expected=201)['id']
            uncertain=request(owner,root+'/deployments',dict(appbody,name='Uncertain'),expected=201)['id']
            proc.terminate();proc.wait(timeout=20)
            with closing(sqlite3.connect(data/'ferry.sqlite')) as conn, conn:
                conn.execute('UPDATE pipeline_runs SET finished_at=? WHERE id=?',(int(time.time()*1000)-3*86400000,rid))
                workspace=conn.execute('SELECT workspace_id FROM deployments WHERE id=?',(app,)).fetchone()[0]
                old=int(time.time()*1000)-3*86400000
                columns=conn.execute('PRAGMA table_info(deployment_revisions)').fetchall()
                for revision,application,state in [(101,app,'failed'),(102,app,'succeeded'),(103,app,'needs-review'),(104,uncertain,'failed')]:
                    record={r[1]:None if r[1]=='variable_snapshot' else 0 if 'INT' in r[2] else '' for r in columns}
                    record.update(id=revision,workspace_id=workspace,deployment_id=int(application),number=revision,state=state,finished_at=old,created_at=old,updated_at=old,version=1,variable_snapshot='encrypted-fixture')
                    conn.execute('INSERT INTO deployment_revisions ('+','.join(record)+') VALUES ('+','.join('?' for _ in record)+')',list(record.values()))
                conn.execute('UPDATE deployments SET state=?,current_revision=? WHERE id=?',('succeeded',102,app))
                conn.execute('UPDATE deployments SET state=? WHERE id=?',('needs-review',uncertain))
            proc=launch()
            for resource in ['settings','dictionaries']:
                persisted=request(owner,root+'/'+resource+'/'+typed_ids[(resource,'json')])
                check(json.loads(persisted['value_json'])==typed_samples['json'],resource+' typed contents persist across restart')
            check(request(owner,root+'/pipeline-runs/'+rid)['state']=='failed','default retention keeps old history across restart')
            retention=next(r for r in request(owner,root+'/settings')['items'] if r['key']=='history_retention_days')
            request(owner,root+'/settings/'+retention['id'],{'version':1,'value':1},'PATCH')
            proc.terminate();proc.wait(timeout=20);proc=launch()
            for _ in range(100):
                status,_=request(owner,root+'/pipeline-runs/'+rid,expected=None)
                if status==404:break
                time.sleep(.1)
            check(status==404,'explicit retention removes expired completed history')
            with closing(sqlite3.connect(data/'ferry.sqlite')) as conn, conn:
                check(conn.execute('SELECT count(*) FROM pipeline_run_steps WHERE run_id=?',(rid,)).fetchone()[0]==0 and conn.execute('SELECT count(*) FROM pipeline_run_logs WHERE run_id=?',(rid,)).fetchone()[0]==0,'retention deletes child logs and steps')
            for _ in range(100):
                status,_=request(owner,root+'/deployment-revisions/101',expected=None)
                if status==404: break
                time.sleep(.1)
            check(status==404,'expired noncurrent deployment revision and encrypted snapshot removed')
            for revision in [102,103,104]: request(owner,root+'/deployment-revisions/'+str(revision))
            check(True,'current revision and uncertain deployments/history retained')
            check(request(owner,root+'/activity?action=retention-cleanup')['total']==2,'automatic cleanup is audited')
        print(f'{checks} checks passed',flush=True)
    finally:
        if proc.poll() is None: proc.terminate();proc.wait(timeout=20)
        log.close()
