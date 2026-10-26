#!/usr/bin/env python3
"""Real Compose + runner/SSH acceptance. Owns all fixtures; no silent skips."""
import concurrent.futures
import http.cookiejar
import json
import os
from pathlib import Path
import platform
import random
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


allocated_ports = set()


def port():
    # Docker binds later, potentially in a Linux VM. An OS-selected ephemeral
    # port can be reused by an outgoing connection between preview and deploy.
    # Stay below the default Linux/macOS ephemeral ranges and check local use.
    for candidate in random.SystemRandom().sample(range(20000, 30000), 100):
        if candidate in allocated_ports:
            continue
        with socket.socket() as s:
            try:
                s.bind(('0.0.0.0', candidate))
            except OSError:
                continue
            allocated_ports.add(candidate)
            return candidate
    raise AssertionError('no free fixture port in 20000–29999')


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
    server_port, ssh_port, native_port = port(), port(), port()
    origin = f'http://127.0.0.1:{server_port}'
    processes, files, projects = [], [], []
    extra_containers = []
    checks = 0
    mode = os.environ.get("FERRY_TEST_DEPLOY_MODE", "compose")
    assert mode in ("compose", "docker", "native", "stack")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def check(label, condition):
        nonlocal checks
        if not condition and mode=='stack':
            result=subprocess.run(['docker','exec',name+'-manager','/usr/bin/docker','service','ls'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            (work/'swarm-services.log').write_bytes(result.stdout)
            result=subprocess.run(['docker','exec',name,'cat','/tmp/stack-cli-errors'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            (work/'swarm-cli-errors.log').write_bytes(result.stdout)
            for project in projects:
                result=subprocess.run(['docker','exec',name+'-manager','/usr/bin/docker','service','ps','--no-trunc',project+'_web'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
                (work/(project+'-tasks.log')).write_bytes(result.stdout)
        if not condition and mode=='native':
            debug=subprocess.run(['docker','exec',name,'journalctl','--no-pager','-n','120'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            (work/'native-journal.log').write_bytes(debug.stdout)
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
        result = action(id, 'rollback' if revision else 'apply', dict(version=p['version'], expected_hash=p['expected_hash'], preview_token=p['preview_token'], **({'target_hashes':p['target_hashes']} if 'target_hashes' in p else {}), **({'revision_id':revision} if revision else {})), status=202)['id']
        saved = api('/workspaces/ops/deployment-revisions/'+result)
        check('executed snapshot exactly matches preview', saved['artifact']==p['desired'])
        return result
    def done(id):
        def poll():
            a = app(id)
            return a if a['state'] not in ('queued','running') else None
        return eventually(poll, 180)
    def scheduled_restart(id,label):
        job=create('cron-jobs',{'name':label,'scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'restart','target_kind':'deployment','target_ref':id,'command':'','timeout_seconds':120,'max_retries':0,'retry_delay_seconds':1})
        run=api('/workspaces/ops/cron-jobs/'+job+'/actions/run-now',{},status=202)['id']
        def finished():
            row=api('/workspaces/ops/cron-runs/'+run)
            return row if row['state'] not in ('queued','running') else None
        result=eventually(finished,150)
        api('/workspaces/ops/cron-jobs/'+job,method='DELETE',status=204)
        return result
    def manifest(value, published):
        return {'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[
            {'name':'web','image':os.environ.get('FERRY_TEST_IMAGE','busybox:1.36'),'command':['sh','-c',f'mkdir -p /www; printf {value} > /www/index.html; exec httpd -f -p 8080 -h /www'],
             'health':{'readiness':{'httpGet':{'path':'/','port':8080},'periodSeconds':1},'liveness':{'httpGet':{'path':'/','port':8080},'periodSeconds':2}}, 'resources':{'limits':{'cpu':'500m','memory':'64Mi'},'requests':{'memory':'16Mi'}}, 'ports':[{'container':8080,'published':published}], 'env':[{'name':'LITERAL','value':'${NOT_INTERPOLATED}'}]}]}
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
        if mode=='native':
            dockerfile=dockerfile.replace(b'openssh-server docker.io',b'openssh-server systemd systemd-sysv git python3 curl docker.io')
            dockerfile=dockerfile.replace(b'CMD ["/usr/sbin/sshd","-D","-e"]',b'RUN systemctl enable ssh && systemctl disable docker.service docker.socket containerd.service\nCMD ["/sbin/init"]')
        (build_dir/'Dockerfile').write_bytes(dockerfile.replace(b'BASE_IMAGE',base_image.encode()))
        (build_dir/'docker-wrapper').write_text('#!/bin/sh\nwhile [ -f /tmp/deployment-pause ]; do sleep 0.1; done\nfor arg in "$@"; do if [ "$arg" = up ]; then while [ -f /tmp/deployment-up-pause ]; do sleep 0.1; done; fi; if [ "$arg" = up ] && [ -f /tmp/deployment-fail ]; then echo fixture-command-failed >&2; if [ -f /tmp/deployment-leak ]; then find /tmp/deployments -name \".variables-*.env\" -exec cat {} \\; >&2; fi; exit 42; fi; done\nexec /usr/bin/docker "$@"\n')
        command(['curl','-fL','--max-time','120','-o',str(build_dir/'docker-compose'),f'https://github.com/docker/compose/releases/download/v2.40.3/docker-compose-linux-{compose_arch}'])
        if mode=='stack':
            (build_dir/'nested-daemon').write_text('#!/bin/sh\nset -eu\nif [ -f /sys/fs/cgroup/cgroup.controllers ]; then\n mkdir -p /sys/fs/cgroup/init\n for pid in $(cat /sys/fs/cgroup/cgroup.procs); do echo "$pid" > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null || :; done\n sed -e "s/ / +/g" -e "s/^/+/" /sys/fs/cgroup/cgroup.controllers > /sys/fs/cgroup/cgroup.subtree_control\nfi\nexec dockerd "$@"\n')
            with (build_dir/'Dockerfile').open('ab') as docker_file:
                docker_file.write(b'\nCOPY nested-daemon /usr/local/bin/nested-daemon\nRUN chmod +x /usr/local/bin/nested-daemon\n')
            (build_dir/'docker-wrapper').write_text('#!/bin/sh\nerr=$(mktemp); /usr/bin/docker --host unix:///docker-socket/docker.sock "$@" 2>"$err"; result=$?; cat "$err" >&2; cat "$err" >> /tmp/stack-cli-errors; rm -f "$err"; exit $result\n')
        print('== preparing Linux Docker/SSH fixture', flush=True)
        with (work / 'docker-build.log').open('wb') as log:
            print('build log: '+str(work / 'docker-build.log'), flush=True)
            subprocess.run(['docker','build','--progress=plain','-t','deployment-live-test-'+mode,str(build_dir)],stdout=log,stderr=log,check=True,timeout=600)
        if mode=='stack':
            # This CLI reaches an owned nested daemon. Never initialize Swarm on the developer daemon.
            command(['docker','network','create',name+'-swarm'])
            command(['docker','volume','create',name+'-socket'])
            for node in ('manager','worker'):
                args=['docker','run','-d','--privileged','--cgroupns=private','--name',name+'-'+node,'--network',name+'-swarm','-e','DOCKER_TLS_CERTDIR=']
                if node=='manager': args+=['-v',name+'-socket:/docker-socket','-p',f'127.0.0.1:{native_port}:18081']
                args+=['deployment-live-test-'+mode,'nested-daemon','--host=unix:///var/run/docker.sock',*(['--host=unix:///docker-socket/docker.sock'] if node=='manager' else []),'--storage-driver=vfs']
                command(args)
            def engine(args,node='manager'):
                return command(['docker','exec',name+'-'+node,'/usr/bin/docker',*args])
            def daemon_ready(node):
                state=json.loads(command(['docker','inspect',name+'-'+node]))[0]['State']
                assert state['Running'], command(['docker','logs',name+'-'+node]).decode()
                return subprocess.run(['docker','exec',name+'-'+node,'/usr/bin/docker','info'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
            for node in ('manager','worker'): eventually(lambda node=node:daemon_ready(node),120)
            manager_ip=json.loads(command(['docker','inspect',name+'-manager']))[0]['NetworkSettings']['Networks'][name+'-swarm']['IPAddress']
            engine(['swarm','init','--advertise-addr',manager_ip])
            join=engine(['swarm','join-token','-q','worker']).decode().strip()
            engine(['swarm','join','--token',join,manager_ip+':2377'],'worker')
            for node in ('manager','worker'): engine(['pull',os.environ.get('FERRY_TEST_IMAGE','busybox:1.36')],node)
        volume=name+'-files'
        command(['docker','volume','create',volume])
        deployment_workdir=json.loads(command(['docker','volume','inspect',volume]))[0]['Mountpoint']
        # Sibling workload containers resolve bind sources on the daemon host.
        # Share the same named volume at that exact path in the SSH/runner host.
        command(['docker','run','-d','--name',name,*(['--privileged','--cgroupns=private','--tmpfs','/run','--tmpfs','/run/lock','-p',f'127.0.0.1:{native_port}:18081'] if mode=='native' else []),'--add-host','host.docker.internal:host-gateway','-p',f'127.0.0.1:{ssh_port}:22',*(['-v',name+'-socket:/docker-socket'] if mode=='stack' else ['-v','/var/run/docker.sock:/var/run/docker.sock'] if mode!='native' else []),'-v',volume+':/tmp/deployments','-v',volume+':'+deployment_workdir,'deployment-live-test-'+mode])
        if platform.system() == 'Linux':
            runner=Path('target/debug/ferry-runner').resolve()
        else:
            arch=command(['docker','info','--format','{{.Architecture}}']).decode().strip()
            target='aarch64-unknown-linux-gnu' if arch in ('aarch64','arm64') else 'x86_64-unknown-linux-gnu'
            runner=work/'runner'
            command(['gh','release','download','v0.1.0-alpha.1','--pattern',f'ferry-runner-{target}','--output',str(runner)])
        # systemd can mount /tmp during boot; keep the executable outside it.
        command(['docker','cp',str(runner),name+':/usr/local/bin/fixture-runner'])
        command(['docker','exec',name,'chmod','+x','/usr/local/bin/fixture-runner'])
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
        command(['docker','exec','-d',name,'/usr/local/bin/fixture-runner','--server',f'http://host.docker.internal:{server_port}','--token',token,'--state-dir','/tmp/runner-state'])
        eventually(lambda:api('/workspaces/ops/hosts/'+runner_id)['state']=='online')
        credential=create('credentials',{'name':'ssh','kind':'password','username':'root','secret':'fixture-pass'})
        ssh_id=create('hosts',{'name':'ssh','connection':'ssh','address':'127.0.0.1','port':ssh_port,'username':'root','credential_id':credential})
        action_path='/workspaces/ops/hosts/'+ssh_id+'/actions/test-connection'
        eventually(lambda:api(action_path,{},status=200)['ok'])
        if mode=='native' and os.environ.get('FERRY_TEST_GROUPS')=='1':
            import shlex
            import sqlite3
            group=create('host-groups',{'name':'Deployment group'})
            def membership(host,group_id):
                row=api('/workspaces/ops/hosts/'+host)
                api('/workspaces/ops/hosts/'+host,dict(row,group_id=group_id),method='PATCH')
            membership(runner_id,group)
            nodes=[(runner_id,name,native_port)]
            for index in range(2):
                child=name+'-group-'+str(index)
                extra_containers.append(child)
                ssh=port();web=port()
                command(['docker','run','-d','--name',child,'--privileged','--cgroupns=private','--tmpfs','/run','--tmpfs','/run/lock','-p',f'127.0.0.1:{ssh}:22','-p',f'127.0.0.1:{web}:18081','deployment-live-test-native'])
                eventually(lambda child=child:subprocess.run(['docker','exec',child,'systemctl','is-active','--quiet','ssh'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0)
                host=create('hosts',{'name':'group-'+str(index),'connection':'ssh','address':'127.0.0.1','port':ssh,'username':'root','credential_id':credential,'group_id':group})
                eventually(lambda host=host:api('/workspaces/ops/hosts/'+host+'/actions/test-connection',{},status=200)['ok'])
                nodes.append((host,child,web))
            def remote(index,script):return command(['docker','exec',nodes[index][1],'sh','-c',script]).decode()
            def executable(version):
                return '#!/usr/bin/python3\nimport http.server,os,time,sys\nwhile os.path.exists("/tmp/group-pause"): time.sleep(.1)\nif os.path.exists("/tmp/group-fail"): sys.exit(3)\nopen("/tmp/group-env","w").write(os.environ["TOKEN"])\nopen("/tmp/group-config","w").write(open("/etc/group.conf").read())\nclass Handler(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200); self.end_headers(); self.wfile.write('+repr(version)+'.encode())\nhttp.server.HTTPServer.allow_reuse_address=True\nhttp.server.HTTPServer(("0.0.0.0",18081),Handler).serve_forever()\n'
            definition={'workdir':'/tmp/group-build/{{run}}','steps':[{'name':'build','run':'printf %s '+shlex.quote(executable('group-v1'))+' > app; chmod 755 app'}],'artifacts':['app']}
            pipeline=create('pipelines',{'name':'group-build','target_kind':'host','target_ref':runner_id,'definition':json.dumps(definition)})
            def build(version):
                definition['steps'][0]['run']='printf %s '+shlex.quote(executable(version))+' > app; chmod 755 app'
                row=api('/workspaces/ops/pipelines/'+pipeline)
                api('/workspaces/ops/pipelines/'+pipeline,dict(row,definition=json.dumps(definition)),method='PATCH')
                rid=api('/workspaces/ops/pipelines/'+pipeline+'/actions/run',{},status=201)['id']
                def finished():
                    run=api('/workspaces/ops/pipeline-runs/'+rid)
                    return run if run['state'] not in ('queued','running') else None
                check('group executable archived',eventually(finished)['state']=='succeeded')
                return rid
            run1=build('group-v1')
            group_secret='group secret \"quote\" $dollar\nline2'
            variable=create('variables',{'name':'group-token','scope':'deploy','kind':'secret','value':group_secret})
            template=create('config-templates',{'name':'group-config','content':'token={{ .Values.token }}','parameters':json.dumps([{'name':'token','type':'string','secret':True,'default':'group-token'}])})
            source={'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'group-app'},'strategy':{'type':'recreate','healthTimeout':10},'services':[{'name':'web','artifact':{'source':'pipeline','pipeline':pipeline,'path':'app'},'health':{'readiness':{'httpGet':{'port':18081,'path':'/'},'periodSeconds':1}}}]}
            source['services'][0]['env']=[{'name':'TOKEN','valueFrom':'variable/group-token'}]
            source['services'][0]['files']=[{'path':'/etc/group.conf','fromTemplate':template}]
            id=create('deployments',{'name':'Group deployment','mode':'native','target_kind':'host-group','target_ref':group,'workdir':'/tmp/group-deploy','manifest':json.dumps(source)})
            first=start(id);row=done(id)
            check('group first deployment succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
            check('all three actual hosts serve first version',all(eventually(lambda web=web:response(web)=='group-v1') for _,_,web in nodes))
            check('group records ordered per-host success',[(r['host_id'],r['state']) for r in row['latest_revision']['targets']]==[(host,'succeeded') for host,_,_ in nodes])
            check('group saves current version on each host',len(row['current_targets'])==3 and all(r['current_revision']==first for r in row['current_targets']))
            check('group applies encrypted environment and config snapshot on every host',all(remote(i,'cat /tmp/group-env')==group_secret and remote(i,'cat /tmp/group-config')=='token='+group_secret for i in range(3)))
            check('group public histories never expose resolved secrets',group_secret not in json.dumps(row) and group_secret not in json.dumps(api('/workspaces/ops/deployment-revisions/'+first)))
            p=preview(id)
            api(f'/workspaces/ops/deployments/{id}/actions/apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=403,client=viewer)
            check('group execution requires apply permission',True)
            membership(nodes[2][0],'0')
            action(id,'apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=409)
            check('membership changes invalidate an old preview',True)
            membership(nodes[2][0],group)
            api('/workspaces/ops/host-groups/'+group,method='DELETE',status=409)
            api('/workspaces/ops/hosts/'+nodes[0][0],method='DELETE',status=409)
            check('group and active target host deletion protected',True)
            api('/workspaces/ops/variables/'+variable,{'name':'group-token','version':1,'value':'group-secret-v2'},method='PATCH')
            run2=build('group-v2')
            remote(1,'touch /tmp/group-fail')
            failed=start(id);row=done(id)
            check('group stops at first failure',row['state']=='failed' and [r['state'] for r in row['latest_revision']['targets']]==['succeeded','failed','skipped'])
            check('group does not automatically roll back completed hosts',response(nodes[0][2])=='group-v2' and response(nodes[2][2])=='group-v1')
            check('group partial failure preserves per-host secret versions',remote(0,'cat /tmp/group-env')=='group-secret-v2' and remote(2,'cat /tmp/group-env')==group_secret)
            print(command(['node','scripts/test-deployments-ui.mjs',origin,id,first,'groups']).decode(),flush=True)
            check('group actual browser acceptance',True)
            pointers={r['host_id']:r['current_revision'] for r in row['current_targets']}
            check('group partial success has independent current versions',pointers=={nodes[0][0]:failed,nodes[1][0]:first,nodes[2][0]:first})
            rejected=scheduled_restart(id,'partial-group-restart')
            check('group partial failure cannot be restarted as old successful configuration',rejected['state']=='failed' and app(id)['latest_revision']['id']==failed)
            unused=build('unused')
            # Prove configured cleanup actually ran while partial-success revisions remained usable.
            with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                old=int(time.time()*1000)-3*86400000
                db.execute('UPDATE deployment_revisions SET finished_at=? WHERE id IN (?,?)',(old,first,failed))
                db.execute('UPDATE pipeline_runs SET finished_at=? WHERE id IN (?,?,?)',(old,run1,run2,unused))
            retention=next(item for item in api('/workspaces/ops/settings')['items'] if item['key']=='history_retention_days')
            api('/workspaces/ops/settings/'+retention['id'],{'version':retention['version'],'value':1},method='PATCH')
            def pruned():
                with sqlite3.connect(work/'data'/'ferry.sqlite') as db:return db.execute('SELECT count(*) FROM pipeline_runs WHERE id=?',(unused,)).fetchone()[0]==0
            check('group unused build history is pruned',eventually(pruned,75))
            for rid in (first,failed):
                check('group retains each host current revision '+rid,len(api('/workspaces/ops/deployment-revisions/'+rid)['targets'])==3)
            for rid in (run1,run2):
                check('group retains referenced archived executable '+rid,len(api('/workspaces/ops/pipeline-runs/'+rid)['artifacts'])==1)
            retention=next(item for item in api('/workspaces/ops/settings')['items'] if item['key']=='history_retention_days')
            api('/workspaces/ops/settings/'+retention['id'],{'version':retention['version'],'value':0},method='PATCH')
            remote(1,'rm /tmp/group-fail')
            start(id,first);row=done(id)
            check('group explicit rollback restores all historical targets',row['state']=='succeeded' and all(eventually(lambda web=web:response(web)=='group-v1') for _,_,web in nodes))
            check('group rollback restores historical secret snapshot on every host',all(remote(i,'cat /tmp/group-env')==group_secret and remote(i,'cat /tmp/group-config')=='token='+group_secret for i in range(3)))
            build('group-v3')
            remote(0,'touch /tmp/group-pause')
            queued=start(id)
            eventually(lambda:app(id)['latest_revision']['targets'][0]['state']=='running')
            membership(nodes[2][0],'0')
            remote(0,'rm /tmp/group-pause')
            row=done(id)
            check('group queue freezes targets despite later membership changes',row['state']=='succeeded' and len(row['latest_revision']['targets'])==3 and response(nodes[2][2])=='group-v3')
            membership(nodes[2][0],group)
            p=preview(id)
            remote(2,f"printf '\\n' >> /tmp/group-deploy/{workspace['id']}/{id}/compose.json")
            action(id,'apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=202)
            row=done(id)
            check('group detects drift on an individual host',row['state']=='failed' and [r['state'] for r in row['latest_revision']['targets']]==['succeeded','succeeded','failed'])
            start(id,first);check('group recovers from drift by fresh rollback preview',done(id)['state']=='succeeded')
            role=api('/workspaces/ops/roles',{'name':'Group operator','permissions':['deployments.read','deployments.apply','pipelines.read','config-templates.read']})['id']
            member=next(m for m in api('/workspaces/ops/members')['items'] if m['name']=='viewer')
            api('/workspaces/ops/members/'+member['id'],{'role_id':role,'state':'active','version':member['version']},method='PATCH')
            remote(0,'touch /tmp/group-pause')
            p=api(f'/workspaces/ops/deployments/{id}/actions/preview',{},client=viewer)
            api(f'/workspaces/ops/deployments/{id}/actions/apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=202,client=viewer)
            eventually(lambda:app(id)['latest_revision']['targets'][0]['state']=='running')
            api('/workspaces/ops/roles/'+role,{'name':'Group operator','permissions':['deployments.read'],'version':1},method='PATCH')
            remote(0,'rm /tmp/group-pause')
            row=done(id)
            check('group rechecks actor permissions before each remaining host',row['state']=='failed' and [t['state'] for t in row['latest_revision']['targets']]==['succeeded','skipped','skipped'])
            start(id,first);check('owner can recover group after permission revocation',done(id)['state']=='succeeded')
            api('/workspaces/other/deployment-revisions/'+first,status=404)
            check('group target histories are workspace scoped',True)
            empty=create('host-groups',{'name':'Empty deployment group'})
            empty_app=create('deployments',{'name':'Empty group app','mode':'native','target_kind':'host-group','target_ref':empty,'workdir':'/tmp/empty-group','manifest':json.dumps(source)})
            action(empty_app,'preview',status=400)
            check('empty group cannot be previewed as successful deployment',True)
            api('/workspaces/ops/deployments/'+empty_app,method='DELETE',status=204)
            api('/workspaces/ops/host-groups/'+empty,method='DELETE',status=204)
            api('/workspaces/ops/deployments/'+id,method='DELETE',status=204)
            with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                check('group deletion releases target records and references',db.execute('SELECT count(*) FROM deployment_revision_targets').fetchone()[0]==0 and db.execute('SELECT count(*) FROM deployment_target_states').fetchone()[0]==0 and db.execute('SELECT count(*) FROM deployment_artifact_refs').fetchone()[0]==0)
            check('group check floor',checks>=30)
            print(f'{checks} passed, 0 failed, 0 skipped',flush=True)
            return
        for kind,host in [('runner',runner_id),('ssh',ssh_id)]:
            print('== '+kind,flush=True)
            web_port=port()
            if mode=='stack':
                web_port=native_port
                source=manifest('stack-v1',18081)
                source['services'][0]['replicas']=2
                source['strategy']={'type':'rolling','maxUnavailable':0,'healthTimeout':90}
                secret='stack secret \\slash \"quote\" $dollar\nsecond'
                key='stack-token-'+kind
                variable=create('variables',{'name':key,'scope':'deploy','kind':'secret','value':secret})
                template=create('config-templates',{'name':'stack-config-'+kind,'content':'token={{ .Values.token }}','parameters':json.dumps([{'name':'token','type':'string','secret':True,'default':key}])})
                source['services'][0]['env'].append({'name':'TOKEN','valueFrom':'variable/'+key})
                source['services'][0]['files']=[{'path':'/etc/runtime.conf','fromTemplate':template}]
                id=create('deployments',{'name':'Swarm '+kind,'mode':'stack','target_kind':'host','target_ref':host,'workdir':deployment_workdir,'manifest':json.dumps(source)})
                project=f"deployment-{workspace['id']}-{id}";projects.append(project)
                first=start(id);row=done(id)
                check(kind+' Swarm replicas converge: '+row['latest_revision']['output'],row['state']=='succeeded')
                check(kind+' Swarm ingress actual HTTP',eventually(lambda:response(web_port)=='stack-v1'))
                def task_containers(node): return engine(['ps','-q','--filter','label=com.docker.swarm.service.name='+project+'_web'],node).decode().split()
                check(kind+' Swarm schedules on manager and worker',bool(task_containers('manager')) and bool(task_containers('worker')))
                for node in ('manager','worker'):
                    container=task_containers(node)[0]
                    check(kind+' '+node+' receives config through Swarm secret',engine(['exec',container,'cat','/etc/runtime.conf'],node).decode()=='token='+secret)
                    check(kind+' '+node+' multiline environment preserved',engine(['exec',container,'printenv','TOKEN'],node).decode()==secret+'\n')
                    check(kind+' '+node+' literal environment preserved',engine(['exec',container,'printenv','LITERAL'],node).decode().strip()=='${NOT_INTERPOLATED}')
                saved=json.loads(row['latest_revision']['artifact'])
                check(kind+' Swarm history contains no secret or manager bind',secret not in json.dumps(row) and all(v['type']!='bind' for v in saved['services']['web'].get('volumes',[])))
                source['services'][0]['command'][2]=source['services'][0]['command'][2].replace('stack-v1','stack-v2')
                update(id,source);start(id);row=done(id)
                check(kind+' Swarm rolling update completes',row['state']=='succeeded' and eventually(lambda:response(web_port)=='stack-v2'))
                spec=json.loads(engine(['service','inspect',project+'_web']))[0]['Spec']
                check(kind+' Swarm start-first policy applied',spec['UpdateConfig']['Order']=='start-first')
                api('/workspaces/ops/variables/'+variable,{'name':key,'version':1,'value':'rotated'},method='PATCH')
                start(id,first);row=done(id)
                check(kind+' Swarm rollback restores historical image and config',row['state']=='succeeded' and eventually(lambda:response(web_port)=='stack-v1'))
                for node in ('manager','worker'):
                    for container in task_containers(node):
                        check(kind+' Swarm rollback restores historical variable',engine(['exec',container,'printenv','TOKEN'],node).decode()==secret+'\n')
                restarted=scheduled_restart(id,'swarm-restart-'+kind)
                check(kind+' Swarm scheduled restart converges and preserves historical variables',restarted['state']=='succeeded' and eventually(lambda:response(web_port)=='stack-v1') and all(engine(['exec',container,'printenv','TOKEN'],node).decode()==secret+'\n' for node in ('manager','worker') for container in task_containers(node)))
                source['strategy']['healthTimeout']=5
                source['services'][0]['health']['readiness']['httpGet']['path']='/not-ready'
                update(id,source);start(id);row=done(id)
                check(kind+' Swarm readiness failure cannot report success',row['state']=='failed')
                start(id,first);check(kind+' Swarm explicit rollback recovers',done(id)['state']=='succeeded')
                stage_left=command(['docker','exec',name,'find',deployment_workdir,'-name','stack-*']).decode()
                check(kind+' Swarm private runtime files cleaned',not stage_left.strip())
                engine(['stack','rm',project])
                eventually(lambda:not engine(['service','ls','-q','--filter','label=com.docker.stack.namespace='+project]).strip())
                # Wait for ingress port release before the next transport scenario.
                eventually(lambda:not task_containers('manager') and not task_containers('worker'))
                continue
            if mode=='native':
                import shlex
                web_port=native_port
                def remote(script): return command(['docker','exec',name,'sh','-c',script]).decode()
                repo_path='/tmp/native-repo-'+kind
                remote(f'mkdir -p {repo_path}; cd {repo_path}; git init -b main; git config user.email test@example.invalid; git config user.name test; echo seed > README; git add README; git commit -m seed; git update-server-info; systemd-run --unit=fixture-git /usr/bin/python3 -m http.server 9418 --bind 127.0.0.1 --directory /tmp' if kind=='runner' else f'mkdir -p {repo_path}; cd {repo_path}; git init -b main; git config user.email test@example.invalid; git config user.name test; echo seed > README; git add README; git commit -m seed; git update-server-info')
                repo=create('repositories',{'name':'native-'+kind,'url':'http://127.0.0.1:9418/native-repo-'+kind+'/.git','branch':'main'})
                def executable(version):
                    return '#!/usr/bin/python3\nimport http.server,os\nclass Handler(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200 if self.path == "/" else 404); self.end_headers(); self.wfile.write('+repr(version)+'.encode())\nopen("/tmp/native-env-'+kind+'","w").write(os.environ["TOKEN"])\nopen("/tmp/native-config-'+kind+'","w").write(open("/etc/native-runtime.conf").read())\nhttp.server.HTTPServer(("0.0.0.0",18081),Handler).serve_forever()\n'
                definition={'workdir':'/tmp/native-build-'+kind+'/{{run}}','steps':[{'name':'build','run':'printf %s '+shlex.quote(executable('native-v1'))+' > app; chmod 755 app'}],'artifacts':['app']}
                pipeline=create('pipelines',{'name':'native-build-'+kind,'target_kind':'host','target_ref':host,'repository_id':repo,'definition':json.dumps(definition)})
                def build():
                    rid=api('/workspaces/ops/pipelines/'+pipeline+'/actions/run',{},status=201)['id']
                    def finished():
                        run=api('/workspaces/ops/pipeline-runs/'+rid)
                        return run if run['state'] not in ('queued','running') else None
                    run=eventually(finished)
                    check(kind+' native executable archived',run['state']=='succeeded' and len(run['artifacts'])==1)
                    return rid
                build1=build()
                key='native-token-'+kind;secret='quote" slash\\ dollar$HOME percent% newline\nsecond'
                variable=create('variables',{'name':key,'scope':'deploy','kind':'secret','value':secret})
                template=create('config-templates',{'name':'native-config-'+kind,'content':'token={{ .Values.token }}','parameters':json.dumps([{'name':'token','type':'string','secret':True,'default':key}])})
                source={'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'native'},'services':[{'name':'web','artifact':{'source':'pipeline','pipeline':pipeline,'path':'app'},'env':[{'name':'TOKEN','valueFrom':'variable/'+key}],'files':[{'path':'/etc/native-runtime.conf','fromTemplate':template}],'ports':[{'container':18081}],'health':{'readiness':{'httpGet':{'port':18081,'path':'/'},'periodSeconds':1}},'resources':{'limits':{'cpu':'500m','memory':'64Mi'}}}]}
                id=create('deployments',{'name':'Native '+kind,'mode':'native','target_kind':'host','target_ref':host,'workdir':deployment_workdir,'manifest':json.dumps(source)})
                project=f"deployment-{workspace['id']}-{id}";projects.append(project)
                first=start(id);row=done(id)
                check(kind+' native systemd apply succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
                check(kind+' native actual HTTP response',eventually(lambda:response(web_port)=='native-v1'))
                check(kind+' native multiline environment intact',remote('cat /tmp/native-env-'+kind)==secret)
                check(kind+' native config mount resolved',remote('cat /tmp/native-config-'+kind)=='token='+secret)
                check(kind+' native unit enabled',remote('systemctl is-enabled '+project+'-web.service').strip()=='enabled')
                check(kind+' native actual resource limits',remote('systemctl show '+project+'-web.service -p MemoryMax --value').strip()=='67108864' and remote('systemctl show '+project+'-web.service -p CPUQuotaPerSecUSec --value').strip()=='500ms')
                check(kind+' native exact unit retained',bool(json.loads(row['latest_revision']['artifact'])['x-native-units']))
                remote('rm -rf /tmp/native-build-'+kind)
                definition['steps'][0]['run']='printf %s '+shlex.quote(executable('native-v2'))+' > app; chmod 755 app'
                pipe=api('/workspaces/ops/pipelines/'+pipeline);api('/workspaces/ops/pipelines/'+pipeline,dict(pipe,definition=json.dumps(definition)),method='PATCH')
                build2=build();start(id);row=done(id)
                check(kind+' native update uses latest successful archive',row['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v2'))
                api('/workspaces/ops/variables/'+variable,{'name':key,'version':1,'value':'rotated'},method='PATCH')
                remote('rm -rf /tmp/native-build-'+kind)
                start(id,first);row=done(id)
                check(kind+' native rollback restores archived executable',row['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v1'))
                check(kind+' native rollback restores old config and environment',remote('cat /tmp/native-env-'+kind)==secret and remote('cat /tmp/native-config-'+kind)=='token='+secret)
                remote('systemctl restart '+project+'-web.service')
                check(kind+' native host restart retains secret files',eventually(lambda:response(web_port)=='native-v1') and remote('cat /tmp/native-env-'+kind)==secret)
                source['strategy']={'type':'recreate','healthTimeout':3};source['services'][0]['health']['readiness']['httpGet']['path']='/not-ready'
                update(id,source);start(id);row=done(id)
                check(kind+' native failed readiness not accepted',row['state']=='failed')
                start(id,first);check(kind+' native explicit rollback recovers',done(id)['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v1'))
                api('/workspaces/other/deployments/'+id,status=404);check(kind+' native tenant isolation',True)
                api('/workspaces/ops/pipelines/'+pipeline,method='DELETE',status=409)
                check(kind+' native pipeline reference prevents deletion',True)
                if not os.environ.get('FERRY_TEST_DATABASE_URL'):
                    import sqlite3
                    unused=build()
                    with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                        old=int(time.time()*1000)-3*86400000
                        db.execute('UPDATE pipeline_runs SET finished_at=? WHERE id IN (?,?,?)',(old,build1,build2,unused))
                    retention=next(item for item in api('/workspaces/ops/settings')['items'] if item['key']=='history_retention_days')
                    api('/workspaces/ops/settings/'+retention['id'],{'version':retention['version'],'value':1},method='PATCH')
                    def pruned():
                        with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                            return db.execute('SELECT count(*) FROM pipeline_runs WHERE id=?',(unused,)).fetchone()[0]==0
                    check(kind+' native retention removes only unreferenced build',eventually(pruned,75))
                    for rid in (build1,build2):
                        check(kind+' native retained deployment preserves archive '+rid,len(api('/workspaces/ops/pipeline-runs/'+rid)['artifacts'])==1)
                    start(id,first);check(kind+' native rollback still works after history pruning',done(id)['state']=='succeeded')
                    retention=next(item for item in api('/workspaces/ops/settings')['items'] if item['key']=='history_retention_days')
                    api('/workspaces/ops/settings/'+retention['id'],{'version':retention['version'],'value':0},method='PATCH')
                remote('systemctl disable --now '+project+'-web.service')
                api('/workspaces/ops/deployments/'+id,method='DELETE',status=204)
                if not os.environ.get('FERRY_TEST_DATABASE_URL'):
                    with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                        check(kind+' native deletion releases archive references',db.execute('SELECT count(*) FROM deployment_artifact_refs WHERE revision_id=?',(first,)).fetchone()[0]==0)
                continue
            if mode=='docker':
                source=manifest('docker-v1',web_port)
                key='docker-token-'+kind
                secret="secret 'quote' $dollar\nsecond-line"
                variable=create('variables',{'name':key,'scope':'deploy','kind':'secret','value':secret})
                template=create('config-templates',{'name':'docker-config-'+kind,'content':'token={{ .Values.token }}','parameters':json.dumps([{'name':'token','type':'string','secret':True,'default':key}])})
                source['services'][0]['env'].append({'name':'TOKEN','valueFrom':'variable/'+key})
                source['services'][0]['files']=[{'path':'/etc/runtime.conf','fromTemplate':template}]
                body={'name':'Docker '+kind,'mode':'docker','target_kind':'host','target_ref':host,'workdir':deployment_workdir,'manifest':json.dumps(source)}
                id=create('deployments',body);project=f"deployment-{workspace['id']}-{id}";projects.append(project)
                rendered=action(id,'render')['artifact']
                check(kind+' Docker render deterministic',rendered==action(id,'render')['artifact'])
                check(kind+' Docker literal dollars remain literal',json.loads(rendered)['services']['web']['environment']['LITERAL']=='${NOT_INTERPOLATED}')
                first=start(id);row=done(id)
                check(kind+' Docker apply succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
                check(kind+' Docker actual HTTP response',eventually(lambda:response(web_port)=='docker-v1'))
                container=command(['docker','ps','-q','--filter','label=workspace.deployment='+project]).decode().strip()
                check(kind+' Docker multiline secret environment preserved',command(['docker','exec',container,'printenv','TOKEN']).decode()==secret+'\n')
                check(kind+' Docker config file mounted',command(['docker','exec',container,'cat','/etc/runtime.conf']).decode()=='token='+secret)
                inspected=json.loads(command(['docker','inspect',container]))[0]
                check(kind+' Docker limits and actual health applied',inspected['HostConfig']['Memory']==67108864 and inspected['HostConfig']['NanoCpus']==500000000 and inspected['State']['Health']['Status']=='healthy')
                check(kind+' Docker secret absent from public history',secret not in json.dumps(row))
                source['services'][0]['command'][2]=source['services'][0]['command'][2].replace('docker-v1','docker-v2')
                update(id,source);start(id);row=done(id)
                check(kind+' Docker update changes running service',row['state']=='succeeded' and eventually(lambda:response(web_port)=='docker-v2'))
                api('/workspaces/ops/variables/'+variable,{'name':key,'version':1,'value':'rotated'},method='PATCH')
                start(id,first);row=done(id)
                check(kind+' Docker rollback restores HTTP and historical secret',row['state']=='succeeded' and eventually(lambda:response(web_port)=='docker-v1') and command(['docker','exec',project+'-web','printenv','TOKEN']).decode()==secret+'\n')
                cron_id=create('cron-jobs',{'name':'docker-restart-'+kind,'scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'restart','target_kind':'deployment','target_ref':id,'command':'','timeout_seconds':60,'max_retries':0,'retry_delay_seconds':1})
                cron_run=api('/workspaces/ops/cron-jobs/'+cron_id+'/actions/run-now',{},status=202)['id']
                def restart_finished():
                    result=api('/workspaces/ops/cron-runs/'+cron_run)
                    return result if result['state'] not in ('queued','running') else None
                restarted=eventually(restart_finished,90)
                check(kind+' Docker scheduled restart succeeds',restarted['state']=='succeeded' and app(id)['latest_revision']['action']=='restart')
                api('/workspaces/ops/cron-jobs/'+cron_id,method='DELETE',status=204)
                invalid=manifest('bad',web_port);invalid['services'][0]['replicas']=2
                original=app(id);api('/workspaces/ops/deployments/'+id,dict(original,manifest=json.dumps(invalid)),method='PATCH',status=400)
                check(kind+' Docker rejects replicas before execution',True)
                source['strategy']={'type':'recreate','healthTimeout':3}
                source['services'][0]['health']['readiness']['httpGet']['path']='/not-ready'
                update(id,source);start(id);row=done(id)
                check(kind+' Docker readiness failure cannot report success',row['state']=='failed')
                start(id,first);check(kind+' Docker recovers by explicit rollback',done(id)['state']=='succeeded')
                api('/workspaces/other/deployments/'+id,status=404)
                check(kind+' Docker deployment tenant isolation',True)
                continue
            config_key='runtime-config-'+kind
            config_secret='config-secret-'+kind+'-quote\"-dollar$'
            config_variable=create('variables',{'name':config_key,'scope':'deploy','kind':'secret','value':config_secret})
            config_body={'name':'runtime-config-'+kind,'content':'listen=8080\nliteral=${HOME} $$ $plain\npassword={{ .Values.password }}\n','parameters':json.dumps([{'name':'password','type':'string','secret':True,'default':config_key}])}
            config_template=create('config-templates',config_body)
            source=manifest('v1',web_port)
            source['services'][0]['files']=[{'path':'/etc/runtime.conf','fromTemplate':config_template}]
            image_value=source['services'][0]['image']; source['services'][0]['image']='{{ .Values.image }}'
            template=create('app-templates',{'name':'live-'+kind,'content':json.dumps(source),'parameters':json.dumps([{'name':'image','type':'string'}])})
            generated=api('/workspaces/ops/app-templates/'+template+'/actions/render',{'version':1,'values':{'image':image_value}})['manifest']
            body={'name':kind,'mode':'compose','target_kind':'host','target_ref':host,'workdir':deployment_workdir,'manifest':generated}
            id=create('deployments',body)
            project=f"deployment-{workspace['id']}-{id}"; projects.append(project)
            check(kind+' render is deterministic',action(id,'render')['artifact']==action(id,'render')['artifact'])
            check(kind+' initial preview empty',preview(id)['current']=='')
            rev1=start(id); row=done(id)
            check(kind+' deploy succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
            check(kind+' HTTP serves v1',eventually(lambda:response(web_port)=='v1'))
            container=command(['docker','ps','-q','--filter','label=com.docker.compose.project='+project]).decode().strip()
            limits=json.loads(command(['docker','inspect',container]))[0]['HostConfig']
            running=json.loads(command(['docker','inspect',container]))[0]
            check(kind+' real container healthcheck is healthy',running['State']['Health']['Status']=='healthy' and running['Config']['Healthcheck']['Interval']==2000000000)
            check(kind+' actual container CPU and memory limits applied',limits['NanoCpus']==500000000 and limits['Memory']==67108864 and limits['MemoryReservation']==16777216)
            check(kind+' literal environment preserved',command(['docker','exec',container,'printenv','LITERAL']).decode().strip()=='${NOT_INTERPOLATED}')
            artifact=row['latest_revision']['artifact']; check(kind+' immutable image digest saved','@sha256:' in artifact)
            check(kind+' config file mounted with resolved secret',command(['docker','exec',container,'cat','/etc/runtime.conf']).decode()=='listen=8080\nliteral=${HOME} $$ $plain\npassword='+config_secret+'\n')
            check(kind+' config secret absent from preview and revision',config_secret not in json.dumps(row) and config_secret not in json.dumps(preview(id)))
            config_mount=next(v for v in json.loads(artifact)['services']['web']['volumes'] if v['target']=='/etc/runtime.conf')['source']
            config_remote=f"{deployment_workdir}/{workspace['id']}/{id}/{config_mount}"
            check(kind+' config file private permissions',command(['docker','exec',name,'stat','-c','%a',config_remote]).decode().strip()=='600')
            import sqlite3
            with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                sealed=db.execute('SELECT variable_snapshot FROM deployment_revisions WHERE id=?',(row['latest_revision']['id'],)).fetchone()[0]
                check(kind+' historical config encrypted at rest',sealed.startswith('v1.') and config_secret not in sealed)
            api('/workspaces/ops/variables/'+config_variable,{'name':config_key,'version':1,'value':'rotated-config-value'},method='PATCH')
            api('/workspaces/ops/config-templates/'+config_template,method='DELETE',status=409)
            check(kind+' referenced configuration template cannot be deleted',True)
            config_row=api('/workspaces/ops/config-templates/'+config_template)
            api('/workspaces/ops/config-templates/'+config_template,dict(config_body,version=config_row['version'],content='changed={{ .Values.password }}'),method='PATCH')
            update(id,manifest('v2',web_port)); start(id); row=done(id)
            check(kind+' update succeeds',row['state']=='succeeded')
            check(kind+' HTTP serves v2',eventually(lambda:response(web_port)=='v2'))
            start(id,rev1); row=done(id)
            check(kind+' rollback succeeds',row['state']=='succeeded')
            check(kind+' exact artifact reused',row['latest_revision']['artifact']==artifact)
            rollback_container=command(['docker','ps','-q','--filter','label=com.docker.compose.project='+project]).decode().strip()
            check(kind+' rollback restores original config despite template edits',command(['docker','exec',rollback_container,'cat','/etc/runtime.conf']).decode()=='listen=8080\nliteral=${HOME} $$ $plain\npassword='+config_secret+'\n')
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
            # Variables run through both real host channels and Docker Compose.
            key='secret-'+kind
            secret="  fixture-secret-v1 'quoted' \\slash\n$HOME ${UNSET} $$ \"double\"\t\r\x01  "
            global_id=create('variables',{'name':key,'scope':'global','kind':'secret','value':'global-fixture'})
            module_id=create('variables',{'name':key,'scope':'deploy','kind':'secret','value':'module-fixture'})
            empty_id=create('variables',{'name':'empty-'+kind,'scope':'deploy','kind':'plain','value':''})
            bound=manifest('v2',web_port)
            bound['services'][0]['env'] += [{'name':'BOUND_SECRET','valueFrom':'variable/'+key},{'name':'BOUND_EMPTY','valueFrom':'variable/empty-'+kind}]
            bound['services'][0]['env'][-2]['valueFrom']='{{ .Values.token }}'
            secret_template=create('app-templates',{'name':'secret-'+kind,'content':json.dumps(bound),'parameters':json.dumps([{'name':'token','type':'string','secret':True}])})
            bound=json.loads(api('/workspaces/ops/app-templates/'+secret_template+'/actions/render',{'version':1,'values':{'token':key}})['manifest'])
            update(id,bound)
            inherited=preview(id)
            check(kind+' module overrides global',next(v for v in inherited['variables'] if v['name']==key)['scope']=='deploy')
            object_id=create('variables',{'name':key,'scope':'deploy','scope_ref':id,'kind':'secret','value':secret})
            action(id,'apply',dict(version=inherited['version'],expected_hash=inherited['expected_hash'],preview_token=inherited['preview_token']),status=409)
            check(kind+' new object override invalidates preview',True)
            api('/workspaces/ops/variables/'+object_id,method='DELETE',status=409)
            check(kind+' active variable reference prevents deletion',True)
            p=preview(id)
            check(kind+' preview shows object override metadata', next(v for v in p['variables'] if v['name']==key)['scope_ref']==id)
            check(kind+' preview never exposes secret', 'fixture-secret-v1' not in json.dumps(p) and 'global-fixture' not in json.dumps(p))
            secret_rev=start(id); row=done(id)
            check(kind+' secret deployment succeeds',row['state']=='succeeded')
            def container_env():
                container=command(['docker','ps','-q','--filter','label=com.docker.compose.project='+project]).decode().strip()
                return dict(v.split('=',1) for v in command(['docker','exec',container,'env','-0']).decode().split('\0') if '=' in v)
            check(kind+' exact special characters and empty value reach container',container_env()['BOUND_SECRET']==secret and container_env()['BOUND_EMPTY']=='')
            check(kind+' revision has no secret or encrypted snapshot field', 'fixture-secret-v1' not in json.dumps(row) and 'variable_snapshot' not in json.dumps(row))
            folder=f"/tmp/deployments/{workspace['id']}/{id}"
            def env_paths(): return command(['docker','exec',name,'find',folder,'-name','.variables-*.env']).decode().splitlines()
            check(kind+' environment files cleaned after execution',env_paths()==[])
            if kind=='runner':
                print(command(['node','scripts/test-deployments-ui.mjs',origin,id,secret_rev,'variables']).decode(),flush=True)
                check('real variable binding UI',True)
            stale=preview(id)
            api('/workspaces/ops/variables/'+object_id,{'name':key,'version':1,'value':'fixture-secret-v2'},method='PATCH')
            action(id,'apply',dict(version=stale['version'],expected_hash=stale['expected_hash'],preview_token=stale['preview_token']),status=409)
            check(kind+' rotation invalidates old preview',True)
            fresh=preview(id)
            command(['docker','exec',name,'touch','/tmp/deployment-up-pause'])
            action(id,'apply',dict(version=fresh['version'],expected_hash=fresh['expected_hash'],preview_token=fresh['preview_token']),status=202)
            paths=eventually(env_paths)
            check(kind+' environment file permissions are private',all(command(['docker','exec',name,'stat','-c','%a',path]).strip()==b'600' for path in paths))
            api('/workspaces/ops/variables/'+object_id,{'name':key,'version':2,'value':'fixture-secret-v3'},method='PATCH')
            command(['docker','exec',name,'rm','/tmp/deployment-up-pause'])
            check(kind+' queued snapshot not changed by later rotation',done(id)['state']=='succeeded' and container_env()['BOUND_SECRET']=='fixture-secret-v2')
            start(id,secret_rev)
            check(kind+' rollback restores historical secret',done(id)['state']=='succeeded' and container_env()['BOUND_SECRET']==secret)
            alert_id=create('alert-rules',{'name':'Deployment failure '+kind,'target_kind':'deployment','target_ref':id,'metric':'execution-failed','enabled':True})
            def alert_state(state):
                return any(e['state']==state for e in api('/workspaces/ops/alert-events?rule_id='+alert_id)['items'])
            command(['docker','exec',name,'touch','/tmp/deployment-fail','/tmp/deployment-leak'])
            start(id); failed=done(id)
            check(kind+' secret-bearing command error output suppressed',failed['state']=='failed' and 'fixture-secret' not in json.dumps(failed) and 'fixture-command-failed' not in json.dumps(failed))
            command(['docker','exec',name,'rm','/tmp/deployment-fail','/tmp/deployment-leak'])
            check(kind+' failed execution cleans secret files',env_paths()==[])
            check(kind+' actual deployment failure triggers alert',eventually(lambda:alert_state('firing')))
            update(id,manifest('v2',web_port))
            for variable in [object_id,module_id,global_id,empty_id]:
                api('/workspaces/ops/variables/'+variable,method='DELETE',status=204)
            start(id,secret_rev)
            check(kind+' history remains usable after current variables deleted',done(id)['state']=='succeeded' and container_env()['BOUND_SECRET']==secret)
            check(kind+' historical files cleaned again',env_paths()==[])
            check(kind+' actual successful rollback recovers alert',eventually(lambda:alert_state('recovered')))
            api('/workspaces/ops/alert-rules/'+alert_id,method='DELETE',status=204)
            cron={'name':'scheduled-'+kind,'scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'restart','target_kind':'deployment','target_ref':id,'command':'','timeout_seconds':60,'max_retries':0,'retry_delay_seconds':1}
            cron_id=create('cron-jobs',cron)
            def scheduled():
                run_id=api('/workspaces/ops/cron-jobs/'+cron_id+'/actions/run-now',{},status=202)['id']
                def finished():
                    result=api('/workspaces/ops/cron-runs/'+run_id)
                    return result if result['state'] not in ('queued','running') else None
                return eventually(finished,90)
            result=scheduled()
            check(kind+' scheduled restart succeeds: '+result['output'],result['state']=='succeeded')
            check(kind+' restart keeps historical secret after current variable deletion',container_env()['BOUND_SECRET']==secret)
            check(kind+' restart revision explicitly recorded',app(id)['latest_revision']['action']=='restart')
            cron_row=api('/workspaces/ops/cron-jobs/'+cron_id)
            api('/workspaces/ops/cron-jobs/'+cron_id,dict(cron_row,action='redeploy'),method='PATCH')
            result=scheduled()
            check(kind+' scheduled redeployment succeeds: '+result['output'],result['state']=='succeeded' and response(web_port)=='v2')
            check(kind+' redeploy uses current manifest', 'BOUND_SECRET' not in container_env())
            api('/workspaces/ops/deployments/'+id,method='DELETE',status=409)
            api('/workspaces/ops/cron-jobs/'+cron_id,method='DELETE',status=204)
            # Delete metadata only: the existing v2 HTTP service keeps running.
            api('/workspaces/ops/deployments/'+id,method='DELETE',status=204)
            check(kind+' deletion keeps containers running',response(web_port)=='v2')
        check('check floor',checks>=(35 if mode=='compose' else 30))
        print(f'{checks} passed, 0 failed, 0 skipped',flush=True)
    finally:
        for project in projects:
            ids=command(['docker','ps','-aq','--filter',('label=com.docker.compose.project=' if mode=='compose' else 'label=workspace.deployment=')+project]).decode().split()
            if ids: subprocess.run(['docker','rm','-f',*ids],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            nets=command(['docker','network','ls','-q','--filter','label=com.docker.compose.project='+project]).decode().split()
            if nets: subprocess.run(['docker','network','rm',*nets],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for p in reversed(processes):
            if p.poll() is None: p.kill();p.wait()
        for child in extra_containers: subprocess.run(['docker','rm','-f',child],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','rm','-f',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','volume','rm',name+'-files'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        if mode=='stack':
            for node in ('manager','worker'):
                result=subprocess.run(['docker','logs',name+'-'+node],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
                (work/(node+'-daemon.log')).write_bytes(result.stdout)
                subprocess.run(['docker','rm','-fv',name+'-'+node],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            subprocess.run(['docker','volume','rm',name+'-socket'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            subprocess.run(['docker','network','rm',name+'-swarm'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for f in files:f.close()
        print('fixture logs: '+str(work),flush=True)


if __name__=='__main__': main()
