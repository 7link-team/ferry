#!/usr/bin/env python3
"""Repository authorization acceptance against isolated HTTP providers; optional empty FERRY_TEST_DATABASE_URL."""
from contextlib import closing
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

import base64,hashlib,uuid
from urllib.parse import urlparse,parse_qs,urlencode
codes={};refreshes=[];fail_refresh=False
class Receiver(BaseHTTPRequestHandler):
    def do_GET(self):
        parsed=urlparse(self.path);query=parse_qs(parsed.query)
        if parsed.path.endswith('/authorize'):
            code=uuid.uuid4().hex;codes[code]=query
            self.send_response(302);self.send_header('Location',query['redirect_uri'][0]+'?'+urlencode({'code':code,'state':query['state'][0]}));self.end_headers();return
        access=query.get('access_token',[None])[0] or self.headers.get('PRIVATE-TOKEN') or self.headers.get('x-yunxiao-token') or self.headers.get('Authorization','').removeprefix('Bearer ').removeprefix('token ')
        if not access or not any(v in access for v in ['initial-access','renewed-access','valid-personal-token']):
            self.send_response(401);self.end_headers();return
        listing=parsed.path.endswith('/user/repos') or parsed.path.endswith('/projects') or parsed.path.endswith('/repositories')
        if listing:
            if query.get('page',['1'])[0]=='2':value=[]
            else:value=[{'name':'repository','full_name':'owner/repository','clone_url':api_base+'/owner/repository.git','webUrl':api_base+'/owner/repository','default_branch':'develop'}]
            if '/codeup/' in parsed.path:
                for row in value:row.pop('clone_url',None)
            payload=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload);return
        if parsed.path.endswith('/branches'):
            payload=json.dumps([{'name':'develop'},{'name':'release/test'}]).encode();self.send_response(200);self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload);return
        if not parsed.path.endswith('/user'):assert 'renewed-access' in access, 'market fetch must use refreshed OAuth token'
        if parsed.path.endswith('/user'):value={'login':'authorized-user','username':'authorized-user'}
        else:
            content={'version':1,'templates':[{'key':'web','name':'Authorized app','content':{'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'busybox:1.36'}]},'parameters':[]}]}
            value={'content':base64.b64encode(json.dumps(content).encode()).decode(),'encoding':'base64'}
        payload=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
    def do_POST(self):
        form=parse_qs(self.rfile.read(int(self.headers['Content-Length'])).decode())
        provider=form['client_id'][0].split('-')[0]
        assert form['client_secret']==['fixture-client-secret']
        if form['grant_type']==['refresh_token']:
            refreshes.append(provider)
            if fail_refresh:self.send_response(401);self.end_headers();return
            value={'access_token':provider+'-renewed-access','expires_in':3600,'refresh_token':provider+'-rotated-refresh'}
        else:
            auth=codes.pop(form['code'][0]);assert auth['redirect_uri']==form['redirect_uri']
            if provider!='gitee':assert base64.urlsafe_b64encode(hashlib.sha256(form['code_verifier'][0].encode()).digest()).decode().rstrip('=')==auth['code_challenge'][0]
            value={'access_token':provider+'-initial-access','expires_in':10,'refresh_token':provider+'-initial-refresh'}
        payload=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
    def log_message(self,*args):pass
receiver=ThreadingHTTPServer(('127.0.0.1',0),Receiver)
threading.Thread(target=receiver.serve_forever,daemon=True).start()
api_base=f'http://127.0.0.1:{receiver.server_port}'
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args):return None
checks = 0
def check(condition, label):
    global checks
    assert condition, label
    checks += 1
    print('PASS', label, flush=True)

def client():
    return urllib.request.build_opener(NoRedirect(), urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

with tempfile.TemporaryDirectory(prefix='repository-auth-test-') as directory:
    data = Path(directory)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}/api'
    env = dict(os.environ, FERRY_DATA_DIR=directory, FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    if os.environ.get('FERRY_TEST_DATABASE_URL'): env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
    env['REPOSITORY_OAUTH_PUBLIC_URL']=base[:-4]
    for provider in ['github','gitlab','gitee']:
        prefix='REPOSITORY_OAUTH_'+provider.upper()
        env[prefix+'_CLIENT_ID']=provider+'-client'
        env[prefix+'_CLIENT_SECRET']='fixture-client-secret'
        env[prefix+'_BASE_URL']=api_base;env[prefix+'_API_URL']=api_base
    binary = os.environ.get('FERRY_BINARY', 'target/debug/ferry-server')
    def request(c, path, body=None, method=None, expected=200):
        req = urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None,
            method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
        try: response = c.open(req, timeout=30)
        except urllib.error.HTTPError as e: response = e
        if path.startswith("/repository-providers/callback"):assert response.headers["Referrer-Policy"]=="no-referrer"
        if path.startswith('/repository-providers/callback') and response.code == 303:
            assert urlparse(response.headers['Location']).path == f'/workspaces/{workspace_tenant}/code-connections/new'
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
        workspace_tenant = request(owner, '/workspaces', {'name':'Tenant','slug':'tenant'})['id']
        workspace_other = request(outsider, '/workspaces', {'name':'Other','slug':'other'})['id']
        root = f'/workspaces/{workspace_tenant}'
        invitation=request(owner,root+'/members/invitation')['code']
        request(guest,'/workspace-memberships',{'code':invitation})
        # Connection creation has no concrete repository or branch. Reuse vault references.
        shared=request(owner,root+'/credentials',{'name':'Reusable PAT','kind':'token','username':'authorized-user','secret':'valid-personal-token'},expected=201)['id']
        count=request(owner,root+'/credentials')['total']
        credential_before=request(owner,root+'/credentials/'+shared)
        connection_body={'name':'Team GitHub','provider':'github','credential_id':shared,'base_url':api_base,'api_url':api_base}
        linked=request(owner,root+'/code-connections',connection_body,expected=201)
        check(linked['credential_id']==shared and 'default_branch' not in linked and 'url' not in linked,'connection creation uses existing credential without repository/branch')
        check(request(owner,root+'/credentials')['total']==count,'connection does not duplicate the existing PAT')
        check(request(owner,root+'/credentials/'+shared)==credential_before,'connection creation does not rewrite shared credential metadata or version')
        linked2=request(owner,root+'/code-connections',dict(connection_body,name='Second GitHub'),expected=201)
        check(linked2['id']!=linked['id'],'same platform supports separate named connections')
        browse=request(owner,root+'/code-connections/'+linked['id']+'/repositories')
        check(browse['items'][0]['default_branch']=='develop','named connection browses visible repositories')
        branches=request(owner,root+'/code-connections/'+linked['id']+'/branches?'+urlencode({'url':browse['items'][0]['url']}))
        check([v['name'] for v in branches['items']]==['develop','release/test'],'selected repository exposes actual branches')
        request(owner,root+'/code-connections/'+linked['id']+'/branches?'+urlencode({'url':'https://attacker.example.invalid/repo.git'}),expected=400)
        request(owner,root+'/credentials/'+shared,method='DELETE',expected=409)
        request(owner,root+'/credentials/'+shared,{'name':'Rotated shared PAT','kind':'token','username':'authorized-user','secret':'valid-personal-token-rotated','version':credential_before['version']},method='PATCH')
        check(request(owner,root+'/code-connections/'+linked['id']+'/repositories')['items'],'referenced PAT can rotate through existing credential resource')
        check(request(guest,root+'/code-connections')['items']==[],'named connections remain private to authorizing user')
        request(guest,root+'/code-connections/'+linked['id'],expected=404)
        renamed=request(owner,root+'/code-connections/'+linked['id'],dict(connection_body,name='Renamed team',version=linked['version']),method='PATCH')
        check(renamed['name']=='Renamed team' and request(owner,root+'/code-connections/'+linked2['id'])['name']=='Second GitHub','update targets only selected named connection')
        request(owner,root+'/code-connections/'+linked['id'],dict(connection_body,version=linked['version']),method='PATCH',expected=409)
        request(outsider,f'/workspaces/{workspace_other}/code-connections',connection_body,expected=400)
        wrong=request(owner,root+'/credentials',{'name':'Password not PAT','kind':'password','username':'user','secret':'valid-personal-token'},expected=201)['id']
        request(owner,root+'/code-connections',dict(connection_body,credential_id=wrong),expected=400)
        invalid=request(owner,root+'/credentials',{'name':'Invalid PAT','kind':'token','secret':'rejected-secret'},expected=201)['id']
        request(owner,root+'/code-connections',dict(connection_body,credential_id=invalid),expected=400)
        check(request(owner,root+'/code-connections')['total']==2,'invalid authorization does not leave a connected resource')
        request(owner,root+'/credentials/'+invalid,method='DELETE',expected=204)
        plain=request(owner,root+'/code-connections',{'name':'Plain Git','provider':'git','credential_id':wrong,'url':api_base+'/owner/repository.git'},expected=201)
        check(plain['url']==api_base+'/owner/repository.git' and 'default_branch' not in plain,'plain Git connection stores URL without branch')
        request(owner,root+'/code-connections/'+plain['id'],method='DELETE',expected=204)
        for row in [linked,linked2]:request(owner,root+'/code-connections/'+row['id'],method='DELETE',expected=204)
        request(owner,root+'/credentials/'+shared,method='DELETE',expected=204)
        request(owner,root+'/credentials/'+wrong,method='DELETE',expected=204)
        def status(provider):return next(r for r in request(owner,root+'/repository-providers')['items'] if r['id']==provider)
        def authorize(provider,options=None):
            url=request(owner,root+'/repository-providers/'+provider+'/authorizations'+('?'+urlencode(options) if options else ''),{})['url']
            try:owner.open(url)
            except urllib.error.HTTPError as response:
                assert response.code==302;return response.headers['Location'][len(base):]
            raise AssertionError('provider did not redirect')
        request(guest,root+'/repository-providers/github/authorizations',{},expected=403)
        role=request(owner,root+'/roles',{'name':'Authorize only','permissions':['credentials.create']})['id']
        member=request(owner,root+'/members?q=guest')['items'][0]
        request(owner,root+'/members/'+member['id'],{'version':member['version'],'role_id':role},'PATCH')
        check(not status('codeup')['web_available'],'Codeup exposes API fallback instead of pretending web authorization')
        sources=[];repositories=[];credentials=[]
        for provider in ['github','gitlab','gitee']:
            callback=authorize(provider)
            request(guest,callback,expected=400)
            request(outsider,callback.replace(f"/workspaces/{workspace_tenant}/",f"/workspaces/{workspace_other}/"),expected=400)
            request(owner,callback,expected=303)
            request(owner,callback,expected=400)
            connected=status(provider)
            check(connected['account']=='authorized-user' and connected['credential_id'],provider+' code exchange validates state and records authorized account')
            check('access' not in json.dumps(connected) and 'refresh' not in connected['last_error'],provider+' connection metadata never exposes tokens')
            credential=connected['credential_id'];credentials.append(credential)
            request(owner,root+'/credentials/'+credential,{'name':'Manual override','kind':'token','secret':'replacement','version':1},'PATCH',400)
            request(owner,root+'/repositories',{'name':'Wrong origin','url':'https://attacker.example.invalid/repo.git','credential_id':credential},expected=400)
            repo=request(owner,root+'/repositories',{'name':provider,'url':api_base+'/owner/repository.git','credential_id':credential},expected=201)['id'];repositories.append(repo)
            sid=request(owner,root+'/marketplace-sources',{'name':provider,'repository_id':repo,'provider':provider,'api_base':api_base,'project':'owner/repository','organization':'','branch':'main','index_path':'marketplace.json'},expected=201)['id'];sources.append(sid)
            request(owner,root+'/marketplace-sources/'+sid+'/synchronizations',{})
            check(provider in refreshes and status(provider)['state']=='connected',provider+' expiring authorization refreshes before real market fetch')
            before=credential
            request(owner,authorize(provider),expected=303)
            check(status(provider)['credential_id']==before,provider+' reauthorization preserves credential references')
        check(len(request(owner,root+'/credentials')['items'])==3,'reauthorization never duplicates credentials')
        legacy_credential=status('github')['credential_id']
        for name in ['Named OAuth one','Named OAuth two']:
            request(owner,authorize('github',{'name':name}),expected=303)
        named=request(owner,root+'/code-connections')['items']
        check(len(named)==2 and len({r['credential_id'] for r in named})==2,'named OAuth creates independent connections and refresh owners')
        check(status('github')['credential_id']==legacy_credential,'named OAuth does not overwrite legacy authorization')
        first=named[0]
        request(owner,authorize('github',{'name':first['name'],'connection_id':first['id']}),expected=303)
        check(request(owner,root+'/code-connections/'+first['id'])['credential_id']==first['credential_id'],'explicit OAuth reauthorization preserves only selected identity')
        for row in named:
            request(owner,root+'/code-connections/'+row['id'],method='DELETE',expected=204)
            request(owner,root+'/credentials/'+row['credential_id'],method='DELETE',expected=204)

        callback=authorize('github');cancel=callback.split('?')[0]+'?'+urlencode({'state':parse_qs(urlparse(callback).query)['state'][0],'error':'access_denied'})
        request(owner,cancel,expected=303);request(owner,callback,expected=400)
        check(True,'cancelled authorization consumes single-use state')
        if not os.environ.get('FERRY_TEST_DATABASE_URL'):
            import sqlite3
            with closing(sqlite3.connect(data/'ferry.sqlite')) as conn, conn:
                stored=conn.execute('SELECT secret FROM credentials').fetchall()+conn.execute('SELECT refresh_secret FROM repository_connections').fetchall()
                check(all(r[0].startswith('v1.') for r in stored) and 'initial-access' not in str(stored),'access and refresh tokens encrypted at rest')
            expired_callback=authorize('github')
            expired_state=parse_qs(urlparse(expired_callback).query)['state'][0]
            with closing(sqlite3.connect(data/'ferry.sqlite')) as conn, conn:
                conn.execute('UPDATE repository_oauth_flows SET expires_at=0 WHERE state_hash=?', (hashlib.sha256(expired_state.encode()).hexdigest(),))
            request(owner,expired_callback,expected=400)
            check(True,'expired authorization state cannot exchange a code')
            before_refresh=len(refreshes)
            proc.terminate();proc.wait(timeout=30)
            with closing(sqlite3.connect(data/'ferry.sqlite')) as conn, conn:
                conn.execute("UPDATE repository_connections SET state='refreshing' WHERE provider='github'")
            proc=launch()
            check(status('github')['state']=='reauthorize' and len(refreshes)==before_refresh,'interrupted refresh requires reauthorization without replaying token exchange')
            request(owner,root+'/marketplace-sources/'+sources[0]+'/synchronizations',{},expected=400)
            check(len(refreshes)==before_refresh,'execution cannot reuse credentials with uncertain refresh outcome')
            request(owner,authorize('github'),expected=303)
            fail_refresh=True
            request(owner,root+'/marketplace-sources/'+sources[0]+'/synchronizations',{},expected=400)
            check(status('github')['state']=='reauthorize','refresh rejection reports reauthorization requirement')
            fail_refresh=False;request(owner,authorize('github'),expected=303)
            check(status('github')['state']=='connected','new web authorization clears rejected refresh state')
        # Changing a trusted deployment endpoint must not transfer an existing token.
        proc.terminate();proc.wait(timeout=30)
        env['REPOSITORY_OAUTH_GITHUB_API_URL']=api_base+'/changed-api'
        proc=launch()
        before_refresh=len(refreshes)
        request(owner,root+'/marketplace-sources/'+sources[0]+'/synchronizations',{},expected=400)
        check(len(refreshes)==before_refresh,'changed authorization endpoint rejects old credentials before token refresh or API fetch')
        env['REPOSITORY_OAUTH_GITHUB_API_URL']=api_base
        proc.terminate();proc.wait(timeout=30)
        proc=launch()
        # Dynamic platform configuration: permission, encryption, atomic validation, live changes.
        request(owner,'/platform/settings/1',expected=403)
        settings=request(admin,'/platform/settings/1')
        check('fixture-client-secret' not in json.dumps(settings) and settings['oauth_github_has_client_secret'],'platform settings report presence but never return application secret')
        request(owner,'/platform/settings/1',{'version':settings['version'],'registration_enabled':True},'PATCH',403)
        patch={'version':settings['version'],'registration_enabled':True,'oauth_public_url':base[:-4],
            'oauth_gitea_enabled':True,'oauth_gitea_client_id':'gitea-client','oauth_gitea_client_secret':'fixture-client-secret',
            'oauth_gitea_base_url':api_base,'oauth_gitea_api_url':api_base}
        request(admin,'/platform/settings/1',dict(patch,oauth_public_url='javascript:bad'),'PATCH',400)
        check(request(admin,'/platform/settings/1')['version']==settings['version'],'invalid platform settings do not change version or stored values')
        request(admin,'/platform/settings/1',patch,'PATCH')
        request(admin,'/platform/settings/1',patch,'PATCH',409)
        settings=request(admin,'/platform/settings/1')
        check(settings['oauth_source'].startswith('平台设置') and status('gitea')['web_available'],'new Gitea OAuth configuration is effective without restart')
        request(owner,authorize('gitea'),expected=303)
        browse=request(owner,root+'/repository-providers/gitea/repositories')
        check(browse['account']=='authorized-user' and browse['items'][0]['default_branch']=='develop','Gitea browser authorization returns selectable repositories')
        request(admin,'/platform/settings/1',{'version':settings['version'],'registration_enabled':True,'oauth_github_enabled':False,'oauth_github_client_secret':''},'PATCH')
        request(owner,root+'/repository-providers/github/authorizations',{},expected=400)
        check(not status('github')['web_available'],'disable applies immediately without falling back to environment variables')
        settings=request(admin,'/platform/settings/1')
        check(settings['oauth_github_has_client_secret'],'blank secret retains existing encrypted application secret')
        request(admin,'/platform/settings/1',{'version':settings['version'],'registration_enabled':True,'oauth_github_enabled':True},'PATCH')
        callback=authorize('github')
        settings=request(admin,'/platform/settings/1')
        request(admin,'/platform/settings/1',{'version':settings['version'],'registration_enabled':True,'oauth_github_client_id':'github-changed'},'PATCH')
        request(owner,callback,expected=400)
        check(True,'pending browser authorization rejects changed application configuration')
        # A stored setting remains authoritative after restart and environment changes.
        env['REPOSITORY_OAUTH_GITHUB_CLIENT_ID']='environment-must-not-win'
        proc.terminate();proc.wait(timeout=30);proc=launch()
        check(request(admin,'/platform/settings/1')['oauth_github_client_id']=='github-changed','saved platform settings persist and override environment after restart')
        created_repos=[]
        for provider in ['github','gitlab','gitee','gitea','codeup']:
            before=request(owner,root+'/credentials')['total']
            connection_body={'token':'invalid-token','base_url':api_base,'api_url':api_base,'organization':'test-org','username':'codeup-user'}
            request(owner,root+'/repository-providers/'+provider+'/connections',connection_body,expected=400)
            check(request(owner,root+'/credentials')['total']==before,provider+' rejects invalid token without saving a credential')
            connection_body['token']='valid-personal-token'
            result=request(owner,root+'/repository-providers/'+provider+'/connections',connection_body)
            check(result['credential_id'] and len(result['items'])==1,provider+' PAT verifies actual repository access and creates a connection')
            repo=result['items'][0]
            rid=request(owner,root+'/repositories',{'name':provider+' connected','provider':provider,'url':repo['url'],'default_branch':repo['default_branch'],'credential_id':result['credential_id']},expected=201)['id'];created_repos.append(rid)
            saved=request(owner,root+'/repositories/'+rid)
            check(saved['provider']==provider and saved['credential_id']==result['credential_id'],provider+' repository is saved with its connected authorization')
            request(owner,root+'/repositories/'+rid,{'name':saved['name'],'url':saved['url'],'credential_id':saved['credential_id'],'version':saved['version']},'PATCH')
            check(request(owner,root+'/repositories/'+rid)['provider']==provider,provider+' legacy update without provider preserves source binding')
            request(outsider,f'/workspaces/{workspace_other}/repositories',{'name':'Stolen','provider':provider,'url':repo['url'],'credential_id':result['credential_id']},expected=400)
            request(owner,root+'/repositories',{'name':'Wrong host','provider':provider,'url':'https://attacker.example.invalid/repo.git','credential_id':result['credential_id']},expected=400)
            check(request(owner,root+'/repository-providers/'+provider+'/repositories?page=2')['items']==[],provider+' pagination remains in the same connection')
        before=request(owner,root+'/credentials')['total']
        inline={'kind':'password','username':'git-user','secret':'inline-private-password'}
        request(owner,root+'/repositories',{'name':'','provider':'git','url':api_base+'/generic.git','inline_credential':inline},expected=400)
        check(request(owner,root+'/credentials')['total']==before,'failed generic Git creation rolls back inline credential')
        for kind,url in [('password',api_base+'/password.git'),('token',api_base+'/token.git'),('ssh-key','git@example.invalid:team/repository.git')]:
            rid=request(owner,root+'/repositories',{'name':'Generic '+kind,'provider':'git','url':url,'inline_credential':dict(inline,kind=kind)},expected=201)['id'];created_repos.append(rid)
            saved=request(owner,root+'/repositories/'+rid)
            check(saved['credential_id']!='0' and 'inline-private-password' not in json.dumps(saved),'generic Git '+kind+' saves encrypted authorization with repository')
        if not os.environ.get('FERRY_TEST_DATABASE_URL'):
            with closing(sqlite3.connect(data/'ferry.sqlite')) as conn, conn:
                stored=conn.execute('SELECT repository_oauth FROM platform_state').fetchone()[0]
                check(stored.startswith('v1.') and 'fixture-client-secret' not in stored,'platform OAuth application configuration encrypted at rest')
                check(all(v not in (data/'server.log').read_text() for v in ['fixture-client-secret','valid-personal-token','inline-private-password']),'secrets absent from server logs')
        for rid in created_repos:request(owner,root+'/repositories/'+rid,method='DELETE',expected=204)
        audit=request(owner,root+'/activity?page_size=200')

        check(all(secret not in json.dumps(audit) for secret in ['fixture-client-secret','initial-access','initial-refresh']),'authorization audit contains no client secret or tokens')
        for sid in sources:request(owner,root+'/marketplace-sources/'+sid,method='DELETE',expected=204)
        for repo in repositories:request(owner,root+'/repositories/'+repo,method='DELETE',expected=204)
        for credential in request(owner,root+'/credentials')['items']:request(owner,root+'/credentials/'+credential['id'],method='DELETE',expected=204)
        check(all(r['credential_id'] is None for r in request(owner,root+'/repository-providers')['items']),'credential deletion removes linked authorization secrets')
        print(f'{checks} checks passed (isolated OAuth provider, not live external account acceptance)',flush=True)
    except Exception:
        log.flush();print((data/'server.log').read_text()[-5000:],flush=True)
        raise
    finally:
        if proc.poll() is None:proc.terminate();proc.wait(timeout=30)
        log.close();receiver.shutdown();receiver.server_close()
