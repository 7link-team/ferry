#!/usr/bin/env python3
"""Real Compose + runner/SSH acceptance. Owns all fixtures; no silent skips."""
import concurrent.futures
import http.cookiejar
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid


def command(args, **kw):
    try:
        return subprocess.run(args, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw).stdout
    except subprocess.CalledProcessError as error:
        print(error.stderr.decode(errors='replace')[-6000:], flush=True)
        raise


def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def eventually(fn, seconds=90):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = fn()
        if result:
            return result
        time.sleep(.2)
    raise AssertionError('condition timed out')


def main():
    work = Path(tempfile.mkdtemp(prefix='compose-acceptance-'))
    name = 'compose-check-' + uuid.uuid4().hex[:10]
    server_port, ssh_port = port(), port()
    origin = f'http://127.0.0.1:{server_port}'
    processes, files, projects = [], [], []
    checks = 0
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def check(label, condition):
        nonlocal checks
        assert condition, label
        checks += 1
        print('  ok ' + label, flush=True)

    def api(path, body=None, method=None, status=200, client=None):
        req = urllib.request.Request(origin + '/api' + path, data=None if body is None else json.dumps(body).encode(),
            headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'}, method=method or ('POST' if body is not None else 'GET'))
        try:
            with (client or opener).open(req, timeout=150) as r:
                code, data = r.status, r.read()
        except urllib.error.HTTPError as e:
            code, data = e.code, e.read()
        assert code == status, (path, code, status, data[:1000])
        return json.loads(data) if data else None

    def launch_server():
        env = dict(os.environ, FERRY_DATA_DIR=str(work / 'data'), FERRY_BIND=f'0.0.0.0:{server_port}', FERRY_ALLOWED_HOSTS='host.docker.internal')
        env.pop('FERRY_DATABASE_URL', None)
        if os.environ.get('FERRY_TEST_DATABASE_URL'):
            env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
        log = open(work / f'server-{len(processes)}.log', 'wb'); files.append(log)
        p = subprocess.Popen(['target/debug/ferry-server'], env=env, stdout=log, stderr=log); processes.append(p)
        def ready():
            assert p.poll() is None, (work / f'server-{len(processes)-1}.log').read_text()
            try: return api('/status')['ok']
            except OSError: return False
        eventually(ready)
        return p

    def create(resource, body, ws='ops'):
        return api(f'/workspaces/{ws}/{resource}', body, status=201)['id']

    def app(id): return api('/workspaces/ops/deployments/' + id)
    def action(id, verb, body=None, status=200): return api(f'/workspaces/ops/deployments/{id}/actions/{verb}', body or {}, status=status)
    def preview(id, revision=None): return action(id, 'preview', {'revision_id':revision} if revision else {})
    def start(id, revision=None):
        p = preview(id,revision)
        result = action(id, 'rollback' if revision else 'apply', dict(version=p['version'], expected_hash=p['expected_hash'], preview_token=p['preview_token'], **({'revision_id':revision} if revision else {})), status=202)['id']
        saved = api('/workspaces/ops/deployment-revisions/'+result)
        check('executed snapshot exactly matches preview', saved['artifact']==p['desired'])
        return result
    def done(id):
        def poll():
            a = app(id)
            return a if a['state'] not in ('queued','running') else None
        return eventually(poll, 180)
    def manifest(value, published):
        return {'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[
            {'name':'web','image':os.environ.get('FERRY_TEST_IMAGE','busybox:1.36'),'command':['sh','-c',f'mkdir -p /www; printf {value} > /www/index.html; exec httpd -f -p 8080 -h /www'],
             'ports':[{'container':8080,'published':published}], 'env':[{'name':'LITERAL','value':'${NOT_INTERPOLATED}'}]}]}
    def update(id, value):
        row=app(id); row['manifest']=json.dumps(value)
        return api('/workspaces/ops/deployments/'+id,row,method='PATCH')
    def response(p):
        try:
            with urllib.request.urlopen(f'http://127.0.0.1:{p}', timeout=2) as r: return r.read().decode()
        except OSError: return None

    try:
        command(['docker','info'])
        dockerfile=b'''FROM BASE_IMAGE
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server docker.io util-linux ca-certificates && rm -rf /var/lib/apt/lists/* && mkdir /run/sshd && echo 'root:fixture-pass' | chpasswd && printf '%s\\n' 'PermitRootLogin yes' 'PasswordAuthentication yes' >> /etc/ssh/sshd_config
COPY docker-wrapper /usr/local/bin/docker
COPY docker-compose /usr/local/lib/docker/cli-plugins/docker-compose
RUN chmod +x /usr/local/bin/docker /usr/local/lib/docker/cli-plugins/docker-compose
CMD ["/usr/sbin/sshd","-D","-e"]
'''
        base_image = 'ubuntu:24.04' if platform.system() == 'Linux' else 'debian:bookworm-slim'
        arch=command(['docker','info','--format','{{.Architecture}}']).decode().strip()
        compose_arch='aarch64' if arch in ('aarch64','arm64') else 'x86_64'
        build_dir=work/'image';build_dir.mkdir()
        (build_dir/'Dockerfile').write_bytes(dockerfile.replace(b'BASE_IMAGE',base_image.encode()))
        (build_dir/'docker-wrapper').write_text('#!/bin/sh\nwhile [ -f /tmp/deployment-pause ]; do sleep 0.1; done\nfor arg in "$@"; do if [ "$arg" = up ] && [ -f /tmp/deployment-fail ]; then echo fixture-command-failed >&2; exit 42; fi; done\nexec /usr/bin/docker "$@"\n')
        command(['curl','-fL','--max-time','120','-o',str(build_dir/'docker-compose'),f'https://github.com/docker/compose/releases/download/v2.40.3/docker-compose-linux-{compose_arch}'])
        print('== preparing Linux Docker/SSH fixture', flush=True)
        with (work / 'docker-build.log').open('wb') as log:
            print('build log: '+str(work / 'docker-build.log'), flush=True)
            subprocess.run(['docker','build','--progress=plain','-t','deployment-live-test',str(build_dir)],stdout=log,stderr=log,check=True,timeout=600)
        command(['docker','run','-d','--name',name,'--add-host','host.docker.internal:host-gateway','-p',f'127.0.0.1:{ssh_port}:22','-v','/var/run/docker.sock:/var/run/docker.sock','deployment-live-test'])
        if platform.system() == 'Linux':
            runner=Path('target/debug/ferry-runner').resolve()
        else:
            arch=command(['docker','info','--format','{{.Architecture}}']).decode().strip()
            target='aarch64-unknown-linux-gnu' if arch in ('aarch64','arm64') else 'x86_64-unknown-linux-gnu'
            runner=work/'runner'
            command(['gh','release','download','v0.1.0-alpha.1','--pattern',f'ferry-runner-{target}','--output',str(runner)])
        command(['docker','cp',str(runner),name+':/tmp/runner'])
        command(['docker','exec',name,'chmod','+x','/tmp/runner'])
        server=launch_server()
        command(['python3', 'scripts/test-setup.py', origin + '/api', str(work / 'data')])
        api('/users',{'email':'deploy@example.invalid','password':'correct-horse-battery','name':'deploy'})
        workspace=api('/workspaces',{'name':'ops','slug':'ops'})
        api('/workspaces',{'name':'other','slug':'other'})
        viewer=urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        api('/users',{'email':'viewer@example.invalid','password':'correct-horse-battery','name':'viewer'},client=viewer)
        api('/workspaces/actions/join',{'code':workspace['join_code']},client=viewer)
        runner_id=create('hosts',{'name':'runner','connection':'runner'})
        token=api('/workspaces/ops/runner-tokens',{'host_id':runner_id},status=201)['token']
        command(['docker','exec','-d',name,'/tmp/runner','--server',f'http://host.docker.internal:{server_port}','--token',token,'--state-dir','/tmp/runner-state'])
        eventually(lambda:api('/workspaces/ops/hosts/'+runner_id)['state']=='online')
        credential=create('credentials',{'name':'ssh','kind':'password','username':'root','secret':'fixture-pass'})
        ssh_id=create('hosts',{'name':'ssh','connection':'ssh','address':'127.0.0.1','port':ssh_port,'username':'root','credential_id':credential})
        action_path='/workspaces/ops/hosts/'+ssh_id+'/actions/test-connection'
        eventually(lambda:api(action_path,{},status=200)['ok'])
        for kind,host in [('runner',runner_id),('ssh',ssh_id)]:
            print('== '+kind,flush=True)
            web_port=port()
            body={'name':kind,'mode':'compose','target_kind':'host','target_ref':host,'workdir':'/tmp/deployments','manifest':json.dumps(manifest('v1',web_port))}
            id=create('deployments',body)
            project=f"deployment-{workspace['id']}-{id}"; projects.append(project)
            check(kind+' render is deterministic',action(id,'render')['artifact']==action(id,'render')['artifact'])
            check(kind+' initial preview empty',preview(id)['current']=='')
            rev1=start(id); row=done(id)
            check(kind+' deploy succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
            check(kind+' HTTP serves v1',eventually(lambda:response(web_port)=='v1'))
            container=command(['docker','ps','-q','--filter','label=com.docker.compose.project='+project]).decode().strip()
            check(kind+' literal environment preserved',command(['docker','exec',container,'printenv','LITERAL']).decode().strip()=='${NOT_INTERPOLATED}')
            artifact=row['latest_revision']['artifact']; check(kind+' immutable image digest saved','@sha256:' in artifact)
            update(id,manifest('v2',web_port)); start(id); row=done(id)
            check(kind+' update succeeds',row['state']=='succeeded')
            check(kind+' HTTP serves v2',eventually(lambda:response(web_port)=='v2'))
            start(id,rev1); row=done(id)
            check(kind+' rollback succeeds',row['state']=='succeeded')
            check(kind+' exact artifact reused',row['latest_revision']['artifact']==artifact)
            check(kind+' HTTP returns v1',eventually(lambda:response(web_port)=='v1'))
            check(kind+' nested history',api('/workspaces/ops/deployments/'+id+'/revisions')['total']==3)
            api('/workspaces/other/deployments/'+id+'/revisions',status=404)
            api('/workspaces/other/deployments/'+id,status=404); check(kind+' tenant isolation',True)
            api('/workspaces/other/deployment-revisions/'+rev1,status=404); check(kind+' revision isolation',True)
            stale=app(id); stale['version']=0;api('/workspaces/ops/deployments/'+id,stale,method='PATCH',status=409);check(kind+' stale update rejected',True)
            changed=app(id);changed['target_ref']=ssh_id if host==runner_id else runner_id
            api('/workspaces/ops/deployments/'+id,changed,method='PATCH',status=400);check(kind+' immutable host',True)
            p=preview(id); bad=dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'])
            command(['docker','exec',name,'sh','-c',f"printf '\\n' >> /tmp/deployments/{workspace['id']}/{id}/compose.json"])
            action(id,'apply',bad,status=202);row=done(id);check(kind+' remote drift rejected',row['state']=='failed' and '已变化' in row['latest_revision']['output'])
            p=preview(id); payload=dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'])
            # Requests share one version: exactly one may claim the deployment.
            action(id,'apply',payload,status=202);action(id,'apply',payload,status=409);done(id);check(kind+' duplicate trigger rejected',True)
            if kind=='runner':
                # A fixture-only barrier holds the prerequisite command until the server is killed.
                p=preview(id);paused_payload=dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'])
                command(['docker','exec',name,'touch','/tmp/deployment-pause'])
                action(id,'apply',paused_payload,status=202)
                eventually(lambda:app(id)['state']=='running')
                server.kill();server.wait()
                command(['docker','exec',name,'rm','/tmp/deployment-pause'])
                server=launch_server()
                row=app(id);check('restart marks uncertain deployment',row['state']=='needs-review')
                # Reconnection is required for a fresh, valid preview token.
                def reconnected():
                    try: return api('/workspaces/ops/hosts/'+runner_id+'/actions/test-connection',{})['ok']
                    except AssertionError as error:
                        if isinstance(error.args[0],tuple) and error.args[0][1]==503: return False
                        raise
                eventually(reconnected)
                p=preview(id)
                action(id,'apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token']),status=409);check('uncertain execution blocks new apply',True)
                action(id,'acknowledge',{'version':row['version'],'note':'Fixture: checked remote process; cleanup follows.'});check('manual acknowledgement clears block',app(id)['state']=='failed')
            if kind=='runner':
                result=command(['node','scripts/test-deployments-ui.mjs',origin,id,rev1])
                print(result.decode(),flush=True);check('real browser deployment UI',True)
            p=preview(id)
            payload=dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'])
            api(f'/workspaces/ops/deployments/{id}/actions/apply',payload,status=403,client=viewer)
            check(kind+' read-only user cannot deploy',True)
            update(id,manifest('v2',web_port))
            action(id,'apply',payload,status=409);check(kind+' changed manifest invalidates preview',True)
            p=preview(id)
            command(['docker','exec',name,'touch','/tmp/deployment-fail'])
            action(id,'apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token']),status=202)
            row=done(id)
            check(kind+' command failure is retained',row['state']=='failed' and 'fixture-command-failed' in row['latest_revision']['output'])
            command(['docker','exec',name,'rm','/tmp/deployment-fail'])
            command(['docker','exec',name,'mv','/usr/local/lib/docker/cli-plugins/docker-compose','/tmp/compose-disabled'])
            action(id,'preview',{},status=400);check(kind+' missing Compose is explicit',True)
            command(['docker','exec',name,'mv','/tmp/compose-disabled','/usr/local/lib/docker/cli-plugins/docker-compose'])
            invalid=manifest('v2',web_port);invalid['services'][0]['image']='127.0.0.1:1/unavailable:missing'
            update(id,invalid)
            action(id,'preview',{},status=400);check(kind+' unavailable image cannot be previewed',True)
            update(id,manifest('v2',web_port))
            # Delete metadata only: the existing v2 HTTP service keeps running.
            api('/workspaces/ops/deployments/'+id,method='DELETE',status=204)
            check(kind+' deletion keeps containers running',response(web_port)=='v2')
        check('check floor',checks>=35)
        print(f'{checks} passed, 0 failed, 0 skipped',flush=True)
    finally:
        for project in projects:
            ids=command(['docker','ps','-aq','--filter','label=com.docker.compose.project='+project]).decode().split()
            if ids: subprocess.run(['docker','rm','-f',*ids],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            nets=command(['docker','network','ls','-q','--filter','label=com.docker.compose.project='+project]).decode().split()
            if nets: subprocess.run(['docker','network','rm',*nets],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for p in reversed(processes):
            if p.poll() is None: p.kill();p.wait()
        subprocess.run(['docker','rm','-f',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for f in files:f.close()
        print('fixture logs: '+str(work),flush=True)


if __name__=='__main__': main()
