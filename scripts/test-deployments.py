#!/usr/bin/env python3
"""Real Compose + runner/SSH acceptance. Owns all fixtures; no silent skips."""
import concurrent.futures
import http.cookiejar
import hashlib
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


def report_log_tail(path, title, lines=80):
    """Print a log's tail and emit it as a GitHub annotation, so a CI failure is readable without downloading artifacts."""
    try:
        tail = Path(path).read_text(encoding='utf-8', errors='replace').splitlines()[-lines:]
    except OSError as error:
        tail = [f'cannot read {path}: {error}']
    text = '\n'.join(tail)
    print(f'== {title}: last {len(tail)} lines of {path}\n{text}', flush=True)
    if os.environ.get('GITHUB_ACTIONS') == 'true':
        escaped = text[-60000:].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::error title={title}::{escaped}', flush=True)


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


def artifact_workflow(workdir, command, path='app'):
    """Native Actions fixture: publish a ZIP while deployments keep logical paths."""
    workdir=workdir.replace('{{run}}', '${{ github.run_number }}')
    return {'jobs': {'build': {'runs-on': 'self-hosted', 'steps': [
        {'name':'Prepare directory','run':'mkdir -p "'+workdir+'"','shell':'sh'},
        {'name':'build','run':command,'shell':'sh','working-directory':workdir},
        {'name':'Publish executable','uses':'actions/upload-artifact@v4','if':'success()',
         'with':{'name':'executable','path':workdir+'/'+path,'if-no-files-found':'error'}}]}}}


def main():
    work = Path(tempfile.mkdtemp(prefix='compose-acceptance-', dir=os.environ.get('FERRY_TEST_OUTPUT_DIR')))
    name = 'compose-check-' + uuid.uuid4().hex[:10]
    server_port, ssh_port, native_port = port(), port(), port()
    nats_host_ports = [port(), port(), port()]
    origin = f'http://127.0.0.1:{server_port}'
    processes, files, projects = [], [], []
    extra_containers = []
    checks = 0
    mode = os.environ.get("FERRY_TEST_DEPLOY_MODE", "compose")
    assert mode in ("compose", "docker", "native", "stack")
    swarm_ha = os.environ.get('FERRY_TEST_SWARM_HA') == '1'
    test_nats = os.environ.get('FERRY_TEST_NATS_TEMPLATES') == '1' and mode in ('compose', 'stack')
    test_blueprint = os.environ.get('FERRY_TEST_BLUEPRINT') == '1'
    assert not test_blueprint or mode == 'docker' or (mode == 'stack' and swarm_ha), 'Blueprint acceptance runs in docker mode or on the three-node Swarm'
    assert not swarm_ha or mode == 'stack', 'Swarm HA acceptance requires stack mode'
    swarm_nodes = ('manager', 'worker', 'worker2') if swarm_ha else ('manager', 'worker')
    report = {'mode': mode, 'swarm_nodes': list(swarm_nodes) if mode == 'stack' else [],
              'boundary': 'isolated Docker daemons on one host; not physical-host or manager-quorum HA',
              'checks': [], 'snapshots': [], 'result': 'RUNNING'}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def check(label, condition):
        nonlocal checks
        report['checks'].append({'name': label, 'passed': bool(condition)})
        if not condition and mode=='stack':
            result=subprocess.run(['docker','exec',name+'-manager','/usr/bin/docker','service','ls'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            (work/'swarm-services.log').write_bytes(result.stdout)
            result=subprocess.run(['docker','exec',name,'cat','/tmp/stack-cli-errors'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            (work/'swarm-cli-errors.log').write_bytes(result.stdout)
            for project in projects:
                result=subprocess.run(['docker','exec',name+'-manager','/usr/bin/docker','service','ps','--no-trunc',project+'_web'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
                (work/(project+'-tasks.log')).write_bytes(result.stdout)
                print(result.stdout.decode(errors='replace'), flush=True)
                result=subprocess.run(['docker','exec',name+'-manager','/usr/bin/docker','service','inspect','--format','{{json .UpdateStatus}}',project+'_web'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
                (work/(project+'-update.log')).write_bytes(result.stdout)
                print('Swarm update status: '+result.stdout.decode(errors='replace'), flush=True)
        if not condition and mode=='native':
            debug=subprocess.run(['docker','exec',name,'journalctl','--no-pager','-n','120'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            (work/'native-journal.log').write_bytes(debug.stdout)
            print(debug.stdout.decode(errors='replace'), flush=True)
        assert condition, label
        checks += 1
        print('  ok ' + label, flush=True)

    def api(path, body=None, method=None, status=200, client=None):
        req = urllib.request.Request(origin + '/api' + path, data=None if body is None else json.dumps(body).encode(),
            headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN'}, method=method or ('POST' if body is not None else 'GET'))
        try:
            with (client or opener).open(req, timeout=150) as r:
                code, data = r.status, r.read()
        except urllib.error.HTTPError as e:
            code, data = e.code, e.read()
        assert code == status, (path, code, status, data[:1000])
        return json.loads(data) if data else None

    def launch_server():
        env = dict(os.environ, FERRY_DATA_DIR=str(work / 'data'), FERRY_BIND=f'0.0.0.0:{server_port}')
        env.pop('FERRY_DATABASE_URL', None)
        if os.environ.get('FERRY_TEST_DATABASE_URL'):
            env['FERRY_DATABASE_URL'] = os.environ['FERRY_TEST_DATABASE_URL']
        log = open(work / f'server-{len(processes)}.log', 'wb'); files.append(log)
        p = subprocess.Popen(['target/debug/ferry-server'], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0); processes.append(p)
        def ready():
            assert p.poll() is None, (work / f'server-{len(processes)-1}.log').read_text()
            try: return api('/status')['ok']
            except OSError: return False
        eventually(ready)
        return p

    def create(resource, body, ws='ops'):
        resource = {'hosts':'nodes', 'host-groups':'node-groups'}.get(resource, resource)
        return api(f'/workspaces/{workspace_ids[ws]}/{resource}', body, status=201)['id']

    def app(id): return api(f'/workspaces/{workspace_ops}/deployments/' + id)
    def action(id, verb, body=None, status=200):
        operation = dict(render='renderings', preview='previews', apply='executions', rollback='rollbacks', acknowledge='acknowledgements')[verb]
        return api(f'/workspaces/{workspace_ops}/deployments/{id}/{operation}', body or {}, status=status)
    def preview(id, revision=None): return action(id, 'preview', {'revision_id':revision} if revision else {})
    def start(id, revision=None):
        p = preview(id,revision)
        result = action(id, 'rollback' if revision else 'apply', dict(version=p['version'], expected_hash=p['expected_hash'], preview_token=p['preview_token'], **({'target_hashes':p['target_hashes']} if 'target_hashes' in p else {}), **({'revision_id':revision} if revision else {})), status=202)['id']
        saved = api(f'/workspaces/{workspace_ops}/deployment-revisions/'+result)
        check('executed snapshot exactly matches preview', saved['artifact']==p['desired'])
        current = app(id)
        listed = next(r for r in api(f'/workspaces/{workspace_ops}/deployment-revisions?deployment_id={id}&page_size=200')['items'] if r['id']==result)
        check('revision list names the application and the trigger user next to their IDs', listed['deployment_name']==current['name'] and listed['triggered_by_name']=='deploy')
        deployed = next(r for r in api(f'/workspaces/{workspace_ops}/deployments?page_size=200')['items'] if r['id']==id)
        if current['target_kind']=='blueprint':
            # A blueprint targets the hosts named by its binding (a stack only its Swarm manager);
            # there is no single target record.
            bound_hosts={i['hostId'] for rows in current['blueprint_binding']['roles'].values() for i in rows}
            stack=bool(current['blueprint_binding'].get('manager'))
            check('stack blueprint revision targets only the Swarm manager' if stack else 'blueprint revision records every bound host as a target',
                  deployed['target_ref']==current['target_ref'] and len(saved['targets'])==(1 if stack else len(bound_hosts)))
            return result
        target = api(f'/workspaces/{workspace_ops}/'+('nodes' if current['target_kind']=='host' else 'node-groups')+'/'+current['target_ref'])
        check('deployment list names its target host or host group', deployed['target_ref']==current['target_ref'] and deployed['target_name']==target['name'])
        return result
    def done(id):
        def poll():
            a = app(id)
            return a if a['state'] not in ('queued','running') else None
        return eventually(poll, 180)
    def scheduled_restart(id,label):
        job=create('cron-jobs',{'name':label,'scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'restart','target_kind':'deployment','target_ref':id,'command':'','timeout_seconds':120,'max_retries':0,'retry_delay_seconds':1})
        run=api(f'/workspaces/{workspace_ops}/cron-jobs/'+job+'/runs',{},status=202)['id']
        def finished():
            row=api(f'/workspaces/{workspace_ops}/cron-runs/'+run)
            return row if row['state'] not in ('queued','running') else None
        result=eventually(finished,150)
        api(f'/workspaces/{workspace_ops}/cron-jobs/'+job,method='DELETE',status=204)
        return result
    http_template = None

    def manifest(value, published):
        if http_template is not None:
            generated = api(f'/workspaces/{workspace_ops}/app-templates/'+http_template['id']+'/renderings',
                            {'version':http_template['version'],'values':{'port':published,'message':value}})['manifest']
            result = json.loads(generated)
            if 'FERRY_TEST_IMAGE' in os.environ:
                result['services'][0]['image'] = os.environ['FERRY_TEST_IMAGE']
            result['services'][0]['env'].append({'name':'LITERAL','value':'${NOT_INTERPOLATED}'})
            return result
        return {'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[
            {'name':'web','image':os.environ.get('FERRY_TEST_IMAGE','busybox:1.36'),'command':['sh','-c',f'mkdir -p /www; printf {value} > /www/index.html; exec httpd -f -p 8080 -h /www'],
             'health':{'readiness':{'httpGet':{'path':'/','port':8080},'periodSeconds':1},'liveness':{'httpGet':{'path':'/','port':8080},'periodSeconds':2}}, 'resources':{'limits':{'cpu':'500m','memory':'64Mi'},'requests':{'memory':'16Mi'}}, 'ports':[{'container':8080,'published':published}], 'env':[{'name':'LITERAL','value':'${NOT_INTERPOLATED}'}]}]}
    def update(id, value):
        row=app(id); row['manifest']=json.dumps(value)
        return api(f'/workspaces/{workspace_ops}/deployments/'+id,row,method='PATCH')
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
        compose_binary = os.environ.get('FERRY_TEST_COMPOSE_BINARY')
        if compose_binary:
            compose_source = Path(compose_binary).resolve(strict=True)
            assert compose_source.is_file(), 'provided Compose plugin must be a file'
            compose_version = command([str(compose_source),'version','--short']).decode().strip()
            assert compose_version.lstrip('v').startswith('2.'), 'acceptance requires Compose v2'
            shutil.copyfile(compose_source,build_dir/'docker-compose')
        else:
            command(['curl','-fL','--retry','3','--retry-delay','2','--retry-max-time','180','--max-time','120','-o',str(build_dir/'docker-compose'),f'https://github.com/docker/compose/releases/download/v2.40.3/docker-compose-linux-{compose_arch}'])
            compose_version = '2.40.3'
        report['compose'] = {'version':compose_version,'source':'provided' if compose_binary else 'release',
                             'sha256':hashlib.sha256((build_dir/'docker-compose').read_bytes()).hexdigest()}
        if mode=='stack' or test_blueprint:
            (build_dir/'nested-daemon').write_text('#!/bin/sh\nset -eu\nif [ -f /sys/fs/cgroup/cgroup.controllers ]; then\n mkdir -p /sys/fs/cgroup/init\n for pid in $(cat /sys/fs/cgroup/cgroup.procs); do echo "$pid" > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null || :; done\n sed -e "s/ / +/g" -e "s/^/+/" /sys/fs/cgroup/cgroup.controllers > /sys/fs/cgroup/cgroup.subtree_control\nfi\nexec dockerd "$@"\n')
            with (build_dir/'Dockerfile').open('ab') as docker_file:
                docker_file.write(b'\nCOPY nested-daemon /usr/local/bin/nested-daemon\nRUN chmod +x /usr/local/bin/nested-daemon\n')
        if mode=='stack':
            (build_dir/'docker-wrapper').write_text('#!/bin/sh\nerr=$(mktemp); /usr/bin/docker --host unix:///docker-socket/docker.sock "$@" 2>"$err"; result=$?; cat "$err" >&2; cat "$err" >> /tmp/stack-cli-errors; rm -f "$err"; exit $result\n')
        print('== preparing Linux Docker/SSH fixture', flush=True)
        with (work / 'docker-build.log').open('wb') as log:
            print('build log: '+str(work / 'docker-build.log'), flush=True)
            try:
                subprocess.run(['docker','build','--progress=plain','-t','deployment-live-test-'+mode,str(build_dir)],stdout=log,stderr=log,check=True,timeout=600)
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                log.flush()
                report_log_tail(work / 'docker-build.log', 'fixture docker build failed')
                raise
        if mode=='stack':
            # This CLI reaches an owned nested daemon. Never initialize Swarm on the developer daemon.
            command(['docker','network','create',name+'-swarm'])
            command(['docker','volume','create',name+'-socket'])
            for node in swarm_nodes:
                # A node reboot clears runtime sockets/PIDs but preserves /var/lib/docker.
                # Reusing /run from the container layer can mistake a reused PID for containerd.
                args=['docker','run','-d','--privileged','--cgroupns=private','--tmpfs','/run','--name',name+'-'+node,'--network',name+'-swarm','-e','DOCKER_TLS_CERTDIR=']
                if node=='manager': args+=['-v',name+'-socket:/docker-socket','-p',f'127.0.0.1:{native_port}:18081']
                if node=='manager' and test_nats:
                    for published, internal in zip(nats_host_ports, [18082,18083,18084]):
                        args+=['-p',f'127.0.0.1:{published}:{internal}']
                args+=['deployment-live-test-'+mode,'nested-daemon','--host=unix:///var/run/docker.sock',*(['--host=unix:///docker-socket/docker.sock'] if node=='manager' else []),'--storage-driver=vfs']
                command(args)
            def engine(args,node='manager'):
                return command(['docker','exec',name+'-'+node,'/usr/bin/docker',*args])
            def daemon_ready(node):
                state=json.loads(command(['docker','inspect',name+'-'+node]))[0]['State']
                assert state['Running'], command(['docker','logs',name+'-'+node]).decode()
                return subprocess.run(['docker','exec',name+'-'+node,'/usr/bin/docker','info'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
            for node in swarm_nodes: eventually(lambda node=node:daemon_ready(node),120)
            if test_blueprint:
                for node in swarm_nodes:
                    # Ferry runs `docker` on each node over SSH: it must reach that node's own
                    # daemon (the stack wrapper targets the manager socket, absent on workers).
                    command(['docker','exec','-i',name+'-'+node,'sh','-c','cat > /usr/local/bin/docker'],input=b'#!/bin/sh\nerr=$(mktemp); /usr/bin/docker "$@" 2>"$err"; result=$?; cat "$err" >&2; cat "$err" >> /tmp/stack-cli-errors; rm -f "$err"; exit $result\n')
                    # The node runs dockerd instead of the image's sshd command; /run is a tmpfs.
                    command(['docker','exec','-d',name+'-'+node,'sh','-c','mkdir -p /run/sshd && exec /usr/sbin/sshd -D -e'])
            manager_ip=json.loads(command(['docker','inspect',name+'-manager']))[0]['NetworkSettings']['Networks'][name+'-swarm']['IPAddress']
            engine(['swarm','init','--advertise-addr',manager_ip])
            join=engine(['swarm','join-token','-q','worker']).decode().strip()
            for node in swarm_nodes[1:]:
                engine(['swarm','join','--token',join,manager_ip+':2377'],node)
            for node in swarm_nodes: engine(['pull',os.environ.get('FERRY_TEST_IMAGE','busybox:1.36')],node)
            swarm_ids = {node: engine(['info','--format','{{.Swarm.NodeID}}'],node).decode().strip() for node in swarm_nodes}
            if swarm_ha:
                for node_id in swarm_ids.values():
                    engine(['node','update','--label-add','ferry-test=eligible',node_id])
                eventually(lambda: all(row['Status']['State'] == 'ready' for row in json.loads(engine(['node','inspect',*swarm_ids.values()]))))
                check('three isolated Swarm nodes registered and ready', len(set(swarm_ids.values())) == 3)

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
        workspace_ops = workspace['id']
        workspace_other = api('/workspaces',{'name':'other','slug':'other'})['id']
        native_template = native_build_template = None
        if os.environ.get('FERRY_TEST_SCENARIO_TEMPLATES') == '1':
            templates = api(f'/workspaces/{workspace_ops}/app-templates')['items']
            check('new workspace contains all four application examples', len(templates) == 4)
            http_template = next(t for t in templates if json.loads(t['content'])['metadata']['name'] == 'example-http')
            native_template = next(t for t in templates if json.loads(t['content'])['metadata']['name'] == 'example-native-http')
            pipelines = api(f'/workspaces/{workspace_ops}/pipeline-templates?workflow_kind=actions')['items']
            native_build_template = next(t for t in pipelines if 'Native HTTP artifact build' in t['content'])
            api(f'/workspaces/{workspace_other}/app-templates/'+http_template['id'],status=404)
            api(f'/workspaces/{workspace_other}/app-templates/'+native_template['id'],status=404)
            api(f'/workspaces/{workspace_other}/pipeline-templates/'+native_build_template['id'],status=404)
            check('application example cannot be read across workspaces', True)
            check('example initialization starts no deployments', api(f'/workspaces/{workspace_ops}/deployments')['total'] == 0)
            report['scenario_template'] = {'id':http_template['id'],'version':http_template['version'],
                                         'content':http_template['content'],'parameters':http_template['parameters']}
            report['native_scenario_templates'] = {'application':native_template,'pipeline':native_build_template}
            if mode == 'native':
                http_template = None
        workspace_ids = dict(ops=workspace_ops, other=workspace_other)
        viewer=urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        api('/users',{'email':'viewer@example.invalid','password':'correct-horse-battery','name':'viewer'},client=viewer)
        api('/workspace-memberships',{'code':workspace['join_code']},client=viewer)
        runner_id=create('hosts',{'name':'runner','connection':'runner'})
        token=api(f'/workspaces/{workspace_ops}/runner-tokens',{'host_id':runner_id},status=201)['token']
        command(['docker','exec','-d',name,'/usr/local/bin/fixture-runner','--server',f'http://host.docker.internal:{server_port}','--token',token,'--state-dir','/tmp/runner-state'])
        eventually(lambda:api(f'/workspaces/{workspace_ops}/nodes/'+runner_id)['state']=='online')
        credential=create('credentials',{'name':'ssh','kind':'password','username':'root','secret':'fixture-pass'})
        ssh_id=create('hosts',{'name':'ssh','connection':'ssh','address':'127.0.0.1','port':ssh_port,'username':'root','credential_id':credential})
        action_path=f'/workspaces/{workspace_ops}/nodes/'+ssh_id+'/connection-tests'
        eventually(lambda:api(action_path,{},status=200)['ok'])
        if test_blueprint:
            # Official marketplace blueprints on three Linux fixture hosts. Each fixture runs
            # its own Docker daemon, so a host only ever sees its own instances and files;
            # members address each other by fixture IP on one private network.
            fixtures=[]
            plain=urllib.request.build_opener(urllib.request.ProxyHandler({}))
            def fetch(url):
                try:
                    with plain.open(url,timeout=3) as r: return r.read().decode()
                except OSError: return None
            def fixture_docker(child,*args):
                return subprocess.run(['docker','exec',child,'/usr/bin/docker',*args],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            def diagnostics(label,row=None):
                output=[]
                # The executor's own stderr and exit code per target (the revision summary alone
                # only says which host failed).
                for target in (row or {}).get('current_targets') or []:
                    output.append('### target '+str(target.get('host_id'))+' '+str(target.get('state'))+'\n'+str(target.get('output')))
                for _,child,_ in fixtures:
                    output.append('### '+child+'\n'+fixture_docker(child,'ps','-a').stdout.decode(errors='replace'))
                    for container in fixture_docker(child,'ps','-aq').stdout.decode(errors='replace').split():
                        output.append(fixture_docker(child,'logs','--tail','80',container).stdout.decode(errors='replace'))
                if mode=='stack':
                    # Last, so task states and scheduler errors land in the annotated tail.
                    manager=name+'-manager'
                    output.append('### stack CLI errors\n'+subprocess.run(['docker','exec',manager,'sh','-c','tail -40 /tmp/stack-cli-errors'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT).stdout.decode(errors='replace'))
                    output.append('### swarm services\n'+fixture_docker(manager,'service','ls').stdout.decode(errors='replace'))
                    # Exit code and health log of every stack task container: a task stopped for
                    # failing its healthcheck exits 0 and Swarm drops the reason.
                    for _,child,_ in fixtures:
                        for container in fixture_docker(child,'ps','-aq','--filter','label=com.docker.stack.namespace').stdout.decode(errors='replace').split():
                            output.append(fixture_docker(child,'inspect','--format','{{.Name}} exit={{.State.ExitCode}} health={{json .State.Health}}',container).stdout.decode(errors='replace'))
                    for service in fixture_docker(manager,'service','ls','-q').stdout.decode(errors='replace').split():
                        output.append(fixture_docker(manager,'service','ps','--no-trunc','--format','{{.Name}} {{.Node}} {{.DesiredState}} {{.CurrentState}} {{.Error}}',service).stdout.decode(errors='replace'))
                path=work/('blueprint-'+label+'-diagnostics.log')
                path.write_text('\n'.join(output),encoding='utf8')
                report_log_tail(path,'blueprint '+label+' diagnostics',120)
            if mode=='stack':
                # The three Swarm nodes are the blueprint hosts, each reached over SSH and running
                # its own daemon; node 0 is the manager. Members address each other by node IP.
                # A spec places instances at their host's address, so each host is registered at
                # its node IP (the Linux test host routes to the fixture network directly).
                for node in swarm_nodes:
                    child=name+'-'+node
                    ip=json.loads(command(['docker','inspect',child]))[0]['NetworkSettings']['Networks'][name+'-swarm']['IPAddress']
                    host=create('hosts',{'name':'swarm-'+node,'connection':'ssh','address':ip,'port':22,'username':'root','credential_id':credential})
                    eventually(lambda host=host:api(f'/workspaces/{workspace_ops}/nodes/'+host+'/connection-tests',{},status=200)['ok'])
                    fixtures.append((host,child,ip))
                check('three Swarm nodes registered as SSH hosts with distinct addresses',len({ip for _,_,ip in fixtures})==3)
            else:
                network=name+'-blueprint'
                command(['docker','network','create',network])
                # The host as seen from this network. Reaching host.docker.internal (docker0) from a
                # user-defined bridge crosses bridges and can be filtered; the gateway of the fixtures'
                # own network is the host itself, and the server listens on 0.0.0.0.
                gateway=json.loads(command(['docker','network','inspect',network]))[0]['IPAM']['Config'][0]['Gateway']
                for index in range(4):
                    child=f'{name}-bp-{index}'
                    extra_containers.append(child)
                    command(['docker','run','-d','--name',child,'--privileged','--cgroupns=private','--network',network,'--add-host','host.docker.internal:host-gateway','deployment-live-test-'+mode])
                    command(['docker','exec','-d',child,'nested-daemon','--host=unix:///var/run/docker.sock','--storage-driver=vfs'])
                    eventually(lambda child=child:fixture_docker(child,'info').returncode==0,120)
                    ip=json.loads(command(['docker','inspect',child]))[0]['NetworkSettings']['Networks'][network]['IPAddress']
                    if index==0:
                        host=create('hosts',{'name':'blueprint-runner','connection':'runner'})
                        host_token=api(f'/workspaces/{workspace_ops}/runner-tokens',{'host_id':host},status=201)['token']
                        command(['docker','cp',str(runner),child+':/usr/local/bin/fixture-runner'])
                        command(['docker','exec',child,'chmod','+x','/usr/local/bin/fixture-runner'])
                        server_url=f'http://{gateway}:{server_port}'
                        command(['docker','exec','-d',child,'sh','-c',f'exec /usr/local/bin/fixture-runner --server {server_url} --token {host_token} --state-dir /tmp/runner-state >/tmp/fixture-runner.log 2>&1'])
                        try:
                            eventually(lambda host=host:api(f'/workspaces/{workspace_ops}/nodes/'+host)['state']=='online')
                        except AssertionError:
                            probe=subprocess.run(['docker','exec',child,'sh','-c',
                                f'echo "## runner log"; cat /tmp/fixture-runner.log 2>&1 | tail -60; '
                                f'echo "## processes"; ps -eo pid,stat,args | grep -v "ps -eo" | tail -30; '
                                f'echo "## server status from fixture ({server_url})"; curl -sS -m 5 {server_url}/api/status 2>&1; echo; '
                                f'echo "## host.docker.internal"; getent hosts host.docker.internal 2>&1; '
                                f'echo "## routes"; ip route 2>&1'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
                            (work/'blueprint-runner-diagnostics.log').write_bytes(probe.stdout)
                            report_log_tail(work/'blueprint-runner-diagnostics.log','blueprint fixture runner never came online',120)
                            raise
                    else:
                        # Registered at its fixture IP: a spec places instances at their host's address.
                        host=create('hosts',{'name':f'blueprint-ssh-{index}','connection':'ssh','address':ip,'port':22,'username':'root','credential_id':credential})
                        eventually(lambda host=host:api(f'/workspaces/{workspace_ops}/nodes/'+host+'/connection-tests',{},status=200)['ok'])
                    fixtures.append((host,child,ip))
                check('four blueprint hosts with independent Docker daemons and distinct addresses',len({ip for _,_,ip in fixtures})==4)
                # The runner reports its own address; placement uses it for the instances there.
                recorded={h:api(f'/workspaces/{workspace_ops}/nodes/'+h)['address'] for h,_,_ in fixtures}
                if any(recorded[h]!=ip for h,_,ip in fixtures):
                    (work/'blueprint-host-addresses.log').write_text(json.dumps({'recorded':recorded,'fixture':{h:ip for h,_,ip in fixtures}},indent=2),encoding='utf8')
                    report_log_tail(work/'blueprint-host-addresses.log','blueprint host addresses differ from fixture IPs')
                check('every blueprint host is recorded at its fixture IP',all(recorded[h]==ip for h,_,ip in fixtures))
            report['blueprint']={'hosts':[{'host_id':h,'container':c,'address':ip} for h,c,ip in fixtures],'deployments':{}}
            # Import the official packages exactly as bundled: package files become config
            # templates under their own names, the blueprint becomes an application template.
            bundle=json.loads(Path('marketplace/index.json').read_text(encoding='utf8'))
            def install(key):
                version=next(p for p in bundle['packages'] if p['key']==key)['versions'][-1]
                for item in version['files']:
                    create('config-templates',{'name':item['name'],'content':item['content'],'parameters':json.dumps(item['parameters'])})
                template=create('app-templates',{'name':key,'content':json.dumps(version['blueprint']),'parameters':json.dumps(version['parameters'])})
                return api(f'/workspaces/{workspace_ops}/app-templates/'+template)
            def render(template,values):
                return api(f'/workspaces/{workspace_ops}/app-templates/'+template['id']+'/renderings',{'version':template['version'],'values':values})['manifest']
            def binding(driver,roles,hosts,manager=None):
                # Instances are assigned to hosts round-robin in role order; a stack also names
                # its Swarm manager host.
                result,n={},0
                for role,count in roles:
                    result[role]=[]
                    for _ in range(count):
                        host,_,ip=hosts[n%len(hosts)];n+=1
                        result[role].append({'hostId':host,'address':ip})
                return {'driver':driver,'roles':result,**({'manager':manager} if manager else {})}
            def deploy(label,template,values,bound,title=None):
                # Deployment names are free text; compiled applications take the blueprint's name.
                id=create('deployments',{'name':title or label,'mode':bound['driver'],'target_kind':'blueprint','target_ref':'0','workdir':'/tmp/blueprint-deployments','manifest':render(template,values),'blueprint_binding':bound})
                revision=start(id);row=done(id)
                if row['state']!='succeeded': diagnostics(label)
                check(label+' blueprint deployment succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
                report['blueprint']['deployments'][label]={'deployment':id,'revisions':[revision],'binding':bound}
                return id,revision
            def tag(name,members):
                # A host tag carried by exactly `members`; selectors match it by name.
                tag_id=create('tags',{'name':name})
                for host in members:
                    row=api(f'/workspaces/{workspace_ops}/nodes/'+host)
                    api(f'/workspaces/{workspace_ops}/nodes/'+host,dict(row,tag_ids=[t['id'] for t in row.get('tags') or []]+[tag_id]),method='PATCH')
                return name
            def tag_label(name):
                # The Swarm node label standing for a host tag of this workspace.
                return f'ferry.w{workspace_ops}.tag.'+hashlib.sha256(name.encode()).hexdigest()[:12]
            def spec_text(template,driver,values,roles,manager=None,failover=None):
                # A ferry/deploy-v1 spec (JSON is YAML) whose roles select hosts by tag only.
                return json.dumps({'apiVersion':'ferry/deploy-v1','template':{'id':template['id'],'version':template['version']},'driver':driver,
                                   **({'manager':manager} if manager else {}),**({'failover':failover} if failover else {}),'values':values,
                                   'roles':{role:{'instances':count,'hosts':{'tags':tags}} for role,count,tags in roles}})
            def deploy_from_spec(label,text,title,tagged):
                # The server renders the template and places the instances; the stored binding must
                # use exactly the hosts tagged for each role, as the placement preview showed.
                previewed=api(f'/workspaces/{workspace_ops}/deployment-placements',{'spec':text})
                id=create('deployments',{'name':title,'workdir':'/tmp/blueprint-deployments','deploy_spec':text})
                bound=app(id)['blueprint_binding']
                check(label+' stored binding equals the placement preview',bound==previewed['binding'])
                for role,hosts in tagged.items():
                    placed=[item['hostId'] for item in bound['roles'][role]]
                    check(f'{label} {role} instances run on a subset of the hosts tagged for it',set(placed)<=set(hosts))
                revision=start(id);row=done(id)
                if row['state']!='succeeded': diagnostics(label)
                check(label+' blueprint deployment from a spec succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
                report['blueprint']['deployments'][label]={'deployment':id,'revisions':[revision],'binding':bound,'spec':json.loads(text)}
                return id,revision,bound
            def respec(id,text):
                # A values change on a deployed spec: an ordinary upgrade, the binding stays.
                row=app(id)
                return api(f'/workspaces/{workspace_ops}/deployments/'+id,dict(row,deploy_spec=text),method='PATCH')
            def owned(child,id):
                prefix=f"deployment-{workspace['id']}-{id}"
                return sorted(n[len(prefix)+1:] for n in fixture_docker(child,'ps','--filter','label=workspace.deployment='+prefix,'--format','{{.Names}}').stdout.decode().split())
            def monitor(ip,port_number,path):
                text=fetch(f'http://{ip}:{port_number}{path}')
                try: return json.loads(text) if text else None
                except ValueError: return None
            def two_routes(ip,port_number):
                routes=(monitor(ip,port_number,'/routez') or {}).get('routes') or []
                return len({r.get('remote_name') or r.get('remote_id') for r in routes})==2
            def meta_cluster(ip,port_number):
                meta=(monitor(ip,port_number,'/jsz') or {}).get('meta_cluster') or {}
                return meta if meta.get('cluster_size')==3 and meta.get('leader') else None
            def nats_request(ip,port_number,subject,payload):
                # Minimal NATS client: one request/reply over the text protocol.
                with socket.create_connection((ip,port_number),timeout=10) as conn:
                    reader=conn.makefile('rb')
                    assert reader.readline().startswith(b'INFO '),'NATS INFO expected'
                    body=json.dumps(payload).encode()
                    conn.sendall(b'CONNECT {"verbose":false,"pedantic":false}\r\nSUB _INBOX.ferry 1\r\nPUB '+subject.encode()+b' _INBOX.ferry '+str(len(body)).encode()+b'\r\n'+body+b'\r\n')
                    while True:
                        line=reader.readline()
                        if not line or line.startswith(b'-ERR'): raise OSError(line.decode(errors='replace'))
                        if line.startswith(b'PING'): conn.sendall(b'PONG\r\n')
                        elif line.startswith(b'MSG '):
                            size=int(line.split()[-1])
                            return json.loads(reader.read(size+2)[:size])
            def replicated_stream(ip,port_number,stream):
                try:
                    reply=nats_request(ip,port_number,'$JS.API.STREAM.CREATE.'+stream,{'name':stream,'subjects':[stream.lower()+'.>'],'num_replicas':3,'storage':'file'})
                except OSError: return None
                if reply.get('error') and reply['error'].get('err_code')==10058:
                    # Stream already exists from an earlier attempt; read it back.
                    try: reply=nats_request(ip,port_number,'$JS.API.STREAM.INFO.'+stream,{})
                    except OSError: return None
                ok=not reply.get('error') and reply['config']['num_replicas']==3 and len(reply.get('cluster',{}).get('replicas') or [])==2
                return reply if ok else None
            def balanced(url,message):
                bodies=[fetch(url) for _ in range(12)]
                if any(b is None or not b.startswith(message+' from ') for b in bodies): return None
                served={b.strip().split(' from ',1)[1] for b in bodies}
                return served if served=={'web-0','web-1'} else None
            try:
                nats_template=install('nats-cluster')
                nginx_template=install('nginx-ha')
                if mode=='stack':
                    print('== blueprint stack driver on the three-node Swarm',flush=True)
                    manager_host,manager,_=fixtures[0]
                    def stack_tasks(child,id):
                        # Services of this deployment with a task container on this node.
                        prefix=f"deployment-{workspace['id']}-{id}"
                        names=fixture_docker(child,'ps','--filter','label=com.docker.stack.namespace='+prefix,'--format','{{.Label "com.docker.swarm.service.name"}}').stdout.decode().split()
                        return sorted(n[len(prefix)+1:] for n in names)
                    def service_spec(id,service):
                        prefix=f"deployment-{workspace['id']}-{id}"
                        return json.loads(command(['docker','exec',manager,'/usr/bin/docker','service','inspect',prefix+'_'+service]))[0]['Spec']
                    # Hosts are chosen by tag: NATS on all three nodes, the web replicas on the
                    # manager and the first worker, the balancer on the second worker.
                    host_ids=[h for h,_,_ in fixtures]
                    nats_tag,web_tag,lb_tag=tag('bp-nats',host_ids),tag('bp-web',host_ids[:2]),tag('bp-lb',host_ids[2:])
                    fixture_of={h:(node,child,ip) for node,(h,child,ip) in zip(swarm_nodes,fixtures)}
                    nats_values={'client_port':14222,'cluster_port':16222,'monitor_port':18222}
                    nats_id,_,nats_bound=deploy_from_spec('nats-stack',spec_text(nats_template,'stack',nats_values,[('nats',3,[nats_tag])],manager_host),'NATS cluster (stack)',{'nats':host_ids})
                    for i,item in enumerate(nats_bound['roles']['nats']):
                        node,child,ip=fixture_of[item['hostId']]
                        spec=service_spec(nats_id,f'nats-{i}')
                        constraints=[c.replace(' ','') for c in spec['TaskTemplate'].get('Placement',{}).get('Constraints') or []]
                        # A stable member is pinned by node ID only; tag labels constrain stateless roles.
                        check(f'stack nats-{i} service is pinned to Swarm node {node}',constraints==[f'node.id=={swarm_ids[node]}'])
                        endpoint=spec.get('EndpointSpec') or {}
                        check(f'stack nats-{i} uses dnsrr and host-mode ports',endpoint.get('Mode')=='dnsrr' and {p.get('PublishMode') for p in endpoint.get('Ports') or []}=={'host'})
                        check(f'stack nats-{i} updates stop-first (its host ports are taken until the old task stops)',(spec.get('UpdateConfig') or {}).get('Order')=='stop-first')
                        check(f'stack node {node} runs only nats-{i}',eventually(lambda child=child,i=i:stack_tasks(child,nats_id)==[f'nats-{i}'],120))
                        check(f'stack nats-{i} has routes to the two other members',eventually(lambda ip=ip,i=i:two_routes(ip,18222+i),120))
                        meta=eventually(lambda ip=ip,i=i:meta_cluster(ip,18222+i),120)
                        check(f'stack nats-{i} sees a JetStream meta cluster of 3 with a leader',meta['cluster_size']==3)
                    first_ip=fixture_of[nats_bound['roles']['nats'][0]['hostId']][2]
                    stream=eventually(lambda:replicated_stream(first_ip,14222,'FERRY_STACK'),120)
                    check('stack NATS creates a JetStream stream with 3 replicas',stream['config']['num_replicas']==3)
                    report['blueprint']['nats_stack_stream']=stream.get('cluster')
                    nginx_values={'message':'stack-v1','web_port':18080,'lb_port':18000}
                    nginx_spec=lambda values:spec_text(nginx_template,'stack',values,[('web',2,[web_tag]),('lb',1,[lb_tag])],manager_host)
                    nginx_id,first,_=deploy_from_spec('nginx-stack',nginx_spec(nginx_values),'Nginx HA web (stack)',{'web':host_ids[:2],'lb':host_ids[2:]})
                    for service,replicas,selected in (('web',2,web_tag),('lb',1,lb_tag)):
                        spec=service_spec(nginx_id,service)
                        check(f'stack nginx {service} is one service with {replicas} replica(s) on the ingress mesh',spec['Mode'].get('Replicated',{}).get('Replicas')==replicas and {p.get('PublishMode') for p in spec['EndpointSpec']['Ports']}=={'ingress'})
                        check(f'stack nginx {service} runs at most one replica per node and updates stop-first',spec['TaskTemplate'].get('Placement',{}).get('MaxReplicas')==1 and (spec.get('UpdateConfig') or {}).get('Order')=='stop-first')
                        constraints=[c.replace(' ','') for c in spec['TaskTemplate'].get('Placement',{}).get('Constraints') or []]
                        check(f'stateless stack nginx {service} is constrained to the workspace-scoped label of its tag',f'node.labels.{tag_label(selected)}==true' in constraints)
                    labels={node:json.loads(engine(['node','inspect',swarm_ids[node]]))[0]['Spec'].get('Labels') or {} for node in swarm_nodes}
                    check('apply synchronized the tag labels onto exactly the tagged Swarm nodes',
                          all((labels[node].get(tag_label(name))=='true')==(h in members) for name,members in ((web_tag,host_ids[:2]),(lb_tag,host_ids[2:])) for node,(h,_,_) in zip(swarm_nodes,fixtures)))
                    for members,service in ((host_ids[:2],'web'),(host_ids[2:],'lb')):
                        for h in host_ids:
                            node,child,_=fixture_of[h]
                            where='runs' if h in members else 'does not run'
                            check(f'stack nginx {service} {where} on Swarm node {node}',
                                  eventually(lambda child=child,h=h,members=members,service=service:(service in stack_tasks(child,nginx_id))==(h in members),120))
                    prefix=f"deployment-{workspace['id']}-{nginx_id}"
                    lb_url=f'http://{fixtures[0][2]}:18000/'
                    def stack_balanced(message):
                        # Every replica answers '<message> from web'; tell them apart by the task
                        # whose access log records this round's user agent.
                        agent='ferry-balance-'+uuid.uuid4().hex[:12]
                        bodies=[]
                        for _ in range(12):
                            try:
                                with plain.open(urllib.request.Request(lb_url,headers={'User-Agent':agent}),timeout=3) as r: bodies.append(r.read().decode())
                            except OSError: return None
                        if any(b.strip()!=message+' from web' for b in bodies): return None
                        logs=fixture_docker(manager,'service','logs',prefix+'_web').stdout.decode(errors='replace')
                        tasks={line.split('|',1)[0].strip() for line in logs.splitlines() if agent in line and '|' in line}
                        return tasks if len(tasks)==2 else None
                    check('stack nginx load balancer serves both web replicas through the service VIP',eventually(lambda:stack_balanced('stack-v1'),120))
                    respec(nginx_id,nginx_spec(dict(nginx_values,message='stack-v2')))
                    second=start(nginx_id);row=done(nginx_id)
                    if row['state']!='succeeded': diagnostics('nginx-stack-upgrade',row)
                    check('stack nginx parameter upgrade applies a new revision: '+row['latest_revision']['output'],row['state']=='succeeded')
                    check('stack nginx serves the upgraded parameter through the load balancer',eventually(lambda:stack_balanced('stack-v2'),120))
                    start(nginx_id,first);row=done(nginx_id)
                    if row['state']!='succeeded': diagnostics('nginx-stack-rollback',row)
                    check('stack nginx rollback to revision 1 succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
                    check('stack nginx serves the revision 1 parameter after rollback',eventually(lambda:stack_balanced('stack-v1'),120))
                    check('stack rollback revision is current on the Swarm manager',[t['current_revision'] for t in row['current_targets']]==[row['latest_revision']['id']])
                    report['blueprint']['deployments']['nginx-stack']['revisions']+=[second,row['latest_revision']['id']]
                else:
                    print('== blueprint docker driver on four hosts (one spare)',flush=True)
                    # F0/F1/F2 carry NATS; F1/F2 carry web; F0 carries the balancer.
                    # F3 is eligible but initially spare for both replaceable roles.
                    host_ids=[h for h,_,_ in fixtures]
                    nats_tag,web_tag,lb_tag=tag('bp-nats',host_ids),tag('bp-web',host_ids[1:]),tag('bp-lb',host_ids[:1])
                    failover={'mode':'auto','hostDownAfter':30}
                    fixture_of={h:(child,ip) for h,child,ip in fixtures}
                    nats_values={'client_port':14222,'cluster_port':16222,'monitor_port':18222}
                    nats_id,_,nats_bound=deploy_from_spec('nats-docker',spec_text(nats_template,'docker',nats_values,[('nats',3,[nats_tag])],failover=failover),'NATS cluster',{'nats':host_ids})
                    check('NATS starts on F0/F1/F2 with F3 spare',[item['hostId'] for item in nats_bound['roles']['nats']]==host_ids[:3])
                    for i,item in enumerate(nats_bound['roles']['nats']):
                        child,ip=fixture_of[item['hostId']]
                        check(f'docker host of nats-{i} runs only that NATS member',owned(child,nats_id)==[f'nats-{i}'])
                        check(f'docker nats-{i} has routes to the two other members',eventually(lambda ip=ip,i=i:two_routes(ip,18222+i),120))
                        meta=eventually(lambda ip=ip,i=i:meta_cluster(ip,18222+i),120)
                        check(f'docker nats-{i} sees a JetStream meta cluster of 3 with a leader',meta['cluster_size']==3)
                    first_ip=fixture_of[nats_bound['roles']['nats'][0]['hostId']][1]
                    stream=eventually(lambda:replicated_stream(first_ip,14222,'FERRY_DOCKER'),120)
                    check('docker NATS creates a JetStream stream with 3 replicas',stream['config']['num_replicas']==3)
                    report['blueprint']['nats_docker_stream']=stream.get('cluster')
                    nginx_values={'message':'blueprint-v1','web_port':18080,'lb_port':18000}
                    nginx_spec=lambda values:spec_text(nginx_template,'docker',values,[('web',2,[web_tag]),('lb',1,[lb_tag])],failover=failover)
                    nginx_id,first,nginx_bound=deploy_from_spec('nginx-docker',nginx_spec(nginx_values),'Nginx HA web (docker)',{'web':host_ids[1:],'lb':host_ids[:1]})
                    check('nginx starts web on F1/F2 and lb on F0',[item['hostId'] for item in nginx_bound['roles']['web']]==host_ids[1:3] and nginx_bound['roles']['lb'][0]['hostId']==host_ids[0])
                    expected={h:[] for h in host_ids}
                    for role,items in nginx_bound['roles'].items():
                        for i,item in enumerate(items): expected[item['hostId']].append(f'{role}-{i}')
                    check('docker nginx instances land on their bound hosts',[owned(c,nginx_id) for h,c,_ in fixtures]==[sorted(expected[h]) for h,_,_ in fixtures])
                    lb_url=f"http://{fixture_of[nginx_bound['roles']['lb'][0]['hostId']][1]}:18000/"
                    check('docker nginx load balancer serves both web instances',eventually(lambda:balanced(lb_url,'blueprint-v1'),120))
                    row=app(nginx_id)
                    def messages(key):
                        return {json.loads(f.read_text(encoding='utf8'))[key] for f in Path('crates/server/src/i18n').glob('*.json') if key in json.loads(f.read_text(encoding='utf8'))}
                    moved=json.loads(nginx_spec(nginx_values))
                    moved['placement']={'lb':[{'hostId':host_ids[2],'address':fixtures[2][2]}]}
                    rejected=api(f'/workspaces/{workspace_ops}/deployments/'+nginx_id,dict(row,deploy_spec=json.dumps(moved)),method='PATCH',status=400)
                    check('placement change in the spec after the first revision is rejected as a topology change',rejected['error'] in messages('blueprint.topologyChangeRequiresPlan'))
                    moved_binding=json.loads(json.dumps(row['blueprint_binding']))
                    moved_binding['roles']['lb'][0]={'hostId':host_ids[2],'address':fixtures[2][2]}
                    rejected=api(f'/workspaces/{workspace_ops}/deployments/'+nginx_id,dict(row,blueprint_binding=moved_binding,deploy_spec=None),method='PATCH',status=409)
                    check('a spec-managed deployment rejects a binding edit outside its spec',rejected['error'] in messages('deployments.specManaged'))
                    check('binding is frozen after the first revision',app(nginx_id)['blueprint_binding']==row['blueprint_binding'])
                    respec(nginx_id,nginx_spec(dict(nginx_values,message='blueprint-v2')))
                    check('a values change keeps the frozen binding',app(nginx_id)['blueprint_binding']==row['blueprint_binding'])
                    second=start(nginx_id);row=done(nginx_id)
                    check('blueprint deployment keeps its free-text name across revisions',row['name']=='Nginx HA web (docker)')
                    if row['state']!='succeeded': diagnostics('nginx-docker-upgrade')
                    check('docker nginx parameter upgrade applies a new revision: '+row['latest_revision']['output'],row['state']=='succeeded')
                    check('docker nginx serves the upgraded parameter through the load balancer',eventually(lambda:balanced(lb_url,'blueprint-v2'),120))
                    start(nginx_id,first);row=done(nginx_id)
                    if row['state']!='succeeded': diagnostics('nginx-docker-rollback')
                    check('docker nginx rollback to revision 1 succeeds: '+row['latest_revision']['output'],row['state']=='succeeded')
                    check('docker nginx serves the revision 1 parameter after rollback',eventually(lambda:balanced(lb_url,'blueprint-v1'),120))
                    check('rollback revision is current on every bound host',len(row['current_targets'])==3 and all(t['current_revision']==row['latest_revision']['id'] for t in row['current_targets']))
                    report['blueprint']['deployments']['nginx-docker']['revisions']+=[second,row['latest_revision']['id']]
                    # Rollback restores the running revision, not the desired spec.
                    # Replacement previews that spec; align it with the verified v1
                    # baseline so this fault scenario does not request a v2 upgrade.
                    respec(nginx_id,nginx_spec(nginx_values))
                    print('== blueprint compose driver on one host',flush=True)
                    single=[fixtures[0]];ip=fixtures[0][2]
                    deploy('nats-compose',nats_template,{'client_port':24222,'cluster_port':26222,'monitor_port':28222},binding('compose',[('nats',3)],single))
                    for i in range(3):
                        check(f'compose nats-{i} has routes to the two other members',eventually(lambda i=i:two_routes(ip,28222+i),120))
                        check(f'compose nats-{i} sees a JetStream meta cluster of 3 with a leader',eventually(lambda i=i:meta_cluster(ip,28222+i),120)['cluster_size']==3)
                    check('compose NATS creates a JetStream stream with 3 replicas',eventually(lambda:replicated_stream(ip,24222,'FERRY_COMPOSE'),120)['config']['num_replicas']==3)
                    deploy('nginx-compose',nginx_template,{'message':'compose-v1','web_port':28080,'lb_port':28000},binding('compose',[('web',2),('lb',1)],single))
                    check('compose nginx load balancer serves both web instances',eventually(lambda:balanced(f'http://{ip}:28000/','compose-v1'),120))
                    print('== blueprint failover: kill F1, rebuild on F3, then recover F1',flush=True)
                    alert_web_tag=tag('bp-alert-web',host_ids[1:3])
                    alert_lb_tag=tag('bp-alert-lb',[host_ids[2]])
                    alert_values={'message':'alert-only','web_port':19080,'lb_port':19000}
                    alert_id,_,alert_bound=deploy_from_spec('nginx-alert',spec_text(nginx_template,'docker',alert_values,[('web',2,[alert_web_tag]),('lb',1,[alert_lb_tag])],failover={'mode':'alert','hostDownAfter':30}),'Nginx alert only',{'web':host_ids[1:3],'lb':[host_ids[2]]})
                    check('alert-only web-0 starts on the failed fixture',alert_bound['roles']['web'][0]['hostId']==host_ids[1])
                    tools=fixture_docker(fixtures[3][1],'run','--rm','nats:2.10-alpine','sh','-c','command -v wget; command -v nats || echo no-nats-cli')
                    tool_output=tools.stdout.decode(errors='replace')
                    (work/'nats-image-tools.log').write_text(tool_output,encoding='utf8')
                    check('NATS image contains wget and has no nats CLI',tools.returncode==0 and '/wget' in tool_output and 'no-nats-cli' in tool_output)
                    stream=eventually(lambda:replicated_stream(first_ip,14222,'FERRY_FAILOVER'),120)
                    check('failover stream starts with three replicas',stream['config']['num_replicas']==3)
                    for sequence in range(1,21):
                        published=nats_request(first_ip,14222,'ferry_failover.messages',{'sequence':sequence})
                        assert published.get('seq')==sequence and not published.get('error'),published
                    check('twenty failover messages acknowledged',published['seq']==20)
                    def timeline(id):
                        return api(f'/workspaces/{workspace_ops}/deployments/{id}/events?page_size=100')['items']
                    command(['docker','stop',fixtures[1][1]])
                    for id,instance in [(nginx_id,'web-0'),(nats_id,'nats-1')]:
                        def replacement_complete(id=id,instance=instance):
                            events=timeline(id)
                            return events if any(e['kind']=='replaceSucceeded' and e['instance']==instance for e in events) else None
                        events=eventually(replacement_complete,300)
                        chronological=[e['kind'] for e in reversed(events) if e['instance']==instance or (e['kind']=='hostConfirmedDown' and e['host_id']==host_ids[1])]
                        # Acceptance is durable before remote preparation; the binding/fence
                        # claim still precedes any destructive stop or new-host execution.
                        ordered=['hostConfirmedDown','replaceStarted','fenced','replaceSucceeded']
                        check(instance+' records confirmation, accepted operation, fencing and success in order',all(k in chronological for k in ordered) and [chronological.index(k) for k in ordered]==sorted(chronological.index(k) for k in ordered))
                        def successful_revision(id=id):
                            row=app(id)
                            revision=row.get('latest_revision') or {}
                            return row if revision.get('state')=='succeeded' and revision.get('action')=='auto-replace' else None
                        row=eventually(successful_revision,300)
                        check(instance+' latest revision is a successful automatic replacement',row['latest_revision']['action']=='auto-replace')
                        role,index=instance.rsplit('-',1)
                        check(instance+' moves to F3',row['blueprint_binding']['roles'][role][int(index)]['hostId']==host_ids[3])
                    check('nginx balances the original message after failover',eventually(lambda:balanced(lb_url,'blueprint-v1'),300))
                    check('F3 runs exactly the replacement web instance',owned(fixtures[3][1],nginx_id)==['web-0'])
                    moved_nats=app(nats_id)['blueprint_binding']['roles']['nats']
                    for i,item in enumerate(moved_nats):
                        member_ip=fixture_of[item['hostId']][1]
                        check(f'after failover nats-{i} sees three metadata members',eventually(lambda member_ip=member_ip,i=i:meta_cluster(member_ip,18222+i),300)['cluster_size']==3)
                    def caught_up():
                        # CREATE can reply from a follower with its local peer view;
                        # INFO reports the stream leader's current replication state.
                        try: state=nats_request(fixtures[3][2],14223,'$JS.API.STREAM.INFO.FERRY_FAILOVER',{})
                        except OSError: return None
                        report['blueprint']['failover_stream']=state
                        replicas=state.get('cluster',{}).get('replicas') or []
                        return state if not state.get('error') and state.get('config',{}).get('num_replicas')==3 and state.get('state',{}).get('messages')==20 and len(replicas)==2 and all(r.get('current') is True for r in replicas) else None
                    recovered=eventually(caught_up,300)
                    check('replacement member retains twenty messages and both replicas are current',recovered['state']['messages']==20)
                    nats_events=timeline(nats_id)
                    operation=next(e['id'] for e in nats_events if e['kind']=='replaceStarted' and e['instance']=='nats-1')
                    check('replacement healthy hook exits zero for its operation',any(e['kind']=='hookOutput' and e['instance']=='nats-1' and e['detail'].get('hook')=='healthy' and e['detail'].get('code')==0 and e['detail'].get('operation_id')==operation for e in nats_events))
                    def alert_recorded():
                        events=timeline(alert_id)
                        return events if any(e['kind']=='alertOnly' and e['host_id']==host_ids[1] for e in events) else None
                    alert_events=eventually(alert_recorded,300)
                    check('alert-only deployment never replaces a member',not any(e['kind']=='replaceStarted' for e in alert_events))
                    check('alert-only deployment binding stays unchanged',app(alert_id)['blueprint_binding']==alert_bound)
                    command(['docker','start',fixtures[1][1]])
                    command(['docker','exec','-d',fixtures[1][1],'nested-daemon','--host=unix:///var/run/docker.sock','--storage-driver=vfs'])
                    eventually(lambda:fixture_docker(fixtures[1][1],'info').returncode==0,120)
                    eventually(lambda:api(f'/workspaces/{workspace_ops}/nodes/{host_ids[1]}/connection-tests',{},status=200)['ok'],120)
                    for id in [nginx_id,nats_id]:
                        check('recovered host records unfencing '+id,eventually(lambda id=id:any(e['kind']=='unfenced' and e['host_id']==host_ids[1] for e in timeline(id)),300))
                        prefix=f"deployment-{workspace['id']}-{id}"
                        old=fixture_docker(fixtures[1][1],'ps','-a','--filter','label=workspace.deployment='+prefix,'--format','{{.Names}}')
                        check('recovered host removes old deployment containers '+id,old.returncode==0 and not old.stdout.strip())
                        check('recovered deployment has no remaining fences '+id,not app(id)['failover']['fences'])
            except Exception:
                import traceback
                failure=work/'blueprint-failure.log'
                failure.write_text(traceback.format_exc(),encoding='utf8')
                report_log_tail(failure,'blueprint acceptance failure')
                if mode=='docker':
                    event_logs={}
                    for label in ['nginx-docker','nats-docker','nginx-alert']:
                        deployed=report['blueprint']['deployments'].get(label)
                        if deployed:
                            try: event_logs[label]=api(f"/workspaces/{workspace_ops}/deployments/{deployed['deployment']}/events?page_size=100")
                            except Exception as error: event_logs[label]={'error':str(error)}
                    path=work/'failover-events.log'
                    path.write_text(json.dumps(event_logs,indent=2),encoding='utf8')
                    report_log_tail(path,'blueprint failover events',120)
                diagnostics('failure')
                raise
            check('blueprint check floor',checks>=(65 if mode=='docker' else 30))
            report['result']='PASS'
            print(f'{checks} passed, 0 failed, 0 skipped',flush=True)
            return
        if mode=='native' and os.environ.get('FERRY_TEST_GROUPS')=='1':
            import shlex
            import sqlite3
            group=create('host-groups',{'name':'Deployment group'})
            def membership(host,group_id):
                row=api(f'/workspaces/{workspace_ops}/nodes/'+host)
                api(f'/workspaces/{workspace_ops}/nodes/'+host,dict(row,group_id=group_id),method='PATCH')
            membership(runner_id,group)
            nodes=[(runner_id,name,native_port)]
            for index in range(2):
                child=name+'-group-'+str(index)
                extra_containers.append(child)
                ssh=port();web=port()
                command(['docker','run','-d','--name',child,'--privileged','--cgroupns=private','--tmpfs','/run','--tmpfs','/run/lock','-p',f'127.0.0.1:{ssh}:22','-p',f'127.0.0.1:{web}:18081','deployment-live-test-native'])
                eventually(lambda child=child:subprocess.run(['docker','exec',child,'systemctl','is-active','--quiet','ssh'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0)
                host=create('hosts',{'name':'group-'+str(index),'connection':'ssh','address':'127.0.0.1','port':ssh,'username':'root','credential_id':credential,'group_id':group})
                eventually(lambda host=host:api(f'/workspaces/{workspace_ops}/nodes/'+host+'/connection-tests',{},status=200)['ok'])
                nodes.append((host,child,web))
            def remote(index,script):return command(['docker','exec',nodes[index][1],'sh','-c',script]).decode()
            def executable(version):
                return '#!/usr/bin/python3\nimport http.server,os,time,sys\nwhile os.path.exists("/tmp/group-pause"): time.sleep(.1)\nif os.path.exists("/tmp/group-fail"): sys.exit(3)\nopen("/tmp/group-env","w").write(os.environ["TOKEN"])\nopen("/tmp/group-config","w").write(open("/etc/group.conf").read())\nclass Handler(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200); self.end_headers(); self.wfile.write('+repr(version)+'.encode())\nhttp.server.HTTPServer.allow_reuse_address=True\nhttp.server.HTTPServer(("0.0.0.0",18081),Handler).serve_forever()\n'
            definition=artifact_workflow('/tmp/group-build/{{run}}','printf %s '+shlex.quote(executable('group-v1'))+' > app; chmod 755 app')
            pipeline=create('pipelines',{'name':'group-build','target_kind':'host','target_ref':runner_id,'definition_format':'github-actions','definition':json.dumps(definition)})
            def build(version):
                definition['jobs']['build']['steps'][1]['run']='printf %s '+shlex.quote(executable(version))+' > app; chmod 755 app'
                row=api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline)
                api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline,dict(row,definition=json.dumps(definition)),method='PATCH')
                rid=api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline+'/runs',{},status=201)['id']
                def finished():
                    run=api(f'/workspaces/{workspace_ops}/pipeline-runs/'+rid)
                    return run if run['state'] not in ('queued','running') else None
                state=eventually(finished)['state']
                if state!='succeeded':
                    text=''.join(item['chunk'] for item in api(f'/workspaces/{workspace_ops}/pipeline-runs/'+rid+'/logs?after=0')['items'])
                    print(chr(10).join('RUNLOG| '+line for line in text.splitlines() if 'unable to get git' not in line)[-8000:], flush=True)
                check('group executable archived',state=='succeeded')
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
            check('group public histories never expose resolved secrets',group_secret not in json.dumps(row) and group_secret not in json.dumps(api(f'/workspaces/{workspace_ops}/deployment-revisions/'+first)))
            p=preview(id)
            api(f'/workspaces/{workspace_ops}/deployments/{id}/executions',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=403,client=viewer)
            check('group execution requires apply permission',True)
            membership(nodes[2][0],'0')
            action(id,'apply',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=409)
            check('membership changes invalidate an old preview',True)
            membership(nodes[2][0],group)
            api(f'/workspaces/{workspace_ops}/node-groups/'+group,method='DELETE',status=409)
            api(f'/workspaces/{workspace_ops}/nodes/'+nodes[0][0],method='DELETE',status=409)
            check('group and active target host deletion protected',True)
            api(f'/workspaces/{workspace_ops}/variables/'+variable,{'name':'group-token','version':1,'value':'group-secret-v2'},method='PATCH')
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
            retention=next(item for item in api(f'/workspaces/{workspace_ops}/settings')['items'] if item['key']=='history_retention_days')
            api(f'/workspaces/{workspace_ops}/settings/'+retention['id'],{'version':retention['version'],'value':1},method='PATCH')
            def pruned():
                with sqlite3.connect(work/'data'/'ferry.sqlite') as db:return db.execute('SELECT count(*) FROM pipeline_runs WHERE id=?',(unused,)).fetchone()[0]==0
            check('group unused build history is pruned',eventually(pruned,75))
            for rid in (first,failed):
                check('group retains each host current revision '+rid,len(api(f'/workspaces/{workspace_ops}/deployment-revisions/'+rid)['targets'])==3)
            for rid in (run1,run2):
                check('group retains referenced archived executable '+rid,len(api(f'/workspaces/{workspace_ops}/pipeline-runs/'+rid)['artifacts'])==1)
            retention=next(item for item in api(f'/workspaces/{workspace_ops}/settings')['items'] if item['key']=='history_retention_days')
            api(f'/workspaces/{workspace_ops}/settings/'+retention['id'],{'version':retention['version'],'value':0},method='PATCH')
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
            role=api(f'/workspaces/{workspace_ops}/roles',{'name':'Group operator','permissions':['deployments.read','deployments.apply','pipelines.read','config-templates.read']})['id']
            member=next(m for m in api(f'/workspaces/{workspace_ops}/members')['items'] if m['name']=='viewer')
            api(f'/workspaces/{workspace_ops}/members/'+member['id'],{'role_id':role,'state':'active','version':member['version']},method='PATCH')
            remote(0,'touch /tmp/group-pause')
            p=api(f'/workspaces/{workspace_ops}/deployments/{id}/previews',{},client=viewer)
            api(f'/workspaces/{workspace_ops}/deployments/{id}/executions',dict(version=p['version'],expected_hash=p['expected_hash'],preview_token=p['preview_token'],target_hashes=p['target_hashes']),status=202,client=viewer)
            eventually(lambda:app(id)['latest_revision']['targets'][0]['state']=='running')
            api(f'/workspaces/{workspace_ops}/roles/'+role,{'name':'Group operator','permissions':['deployments.read'],'version':1},method='PATCH')
            remote(0,'rm /tmp/group-pause')
            row=done(id)
            check('group rechecks actor permissions before each remaining host',row['state']=='failed' and [t['state'] for t in row['latest_revision']['targets']]==['succeeded','skipped','skipped'])
            start(id,first);check('owner can recover group after permission revocation',done(id)['state']=='succeeded')
            api(f'/workspaces/{workspace_other}/deployment-revisions/'+first,status=404)
            check('group target histories are workspace scoped',True)
            empty=create('host-groups',{'name':'Empty deployment group'})
            empty_app=create('deployments',{'name':'Empty group app','mode':'native','target_kind':'host-group','target_ref':empty,'workdir':'/tmp/empty-group','manifest':json.dumps(source)})
            action(empty_app,'preview',status=400)
            check('empty group cannot be previewed as successful deployment',True)
            api(f'/workspaces/{workspace_ops}/deployments/'+empty_app,method='DELETE',status=204)
            api(f'/workspaces/{workspace_ops}/node-groups/'+empty,method='DELETE',status=204)
            api(f'/workspaces/{workspace_ops}/deployments/'+id,method='DELETE',status=204)
            with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                check('group deletion releases target records and references',db.execute('SELECT count(*) FROM deployment_revision_targets').fetchone()[0]==0 and db.execute('SELECT count(*) FROM deployment_target_states').fetchone()[0]==0 and db.execute('SELECT count(*) FROM deployment_artifact_refs').fetchone()[0]==0)
            check('group check floor',checks>=30)
            report['result']='PASS'
            print(f'{checks} passed, 0 failed, 0 skipped',flush=True)
            return
        for kind,host in [('runner',runner_id),('ssh',ssh_id)]:
            print('== '+kind,flush=True)
            web_port=port()
            if os.environ.get('FERRY_TEST_REDIS_TEMPLATES') == '1' and mode in ('compose', 'stack'):
                import importlib.util
                redis_spec=importlib.util.spec_from_file_location('redis_application_acceptance',Path(__file__).with_name('redis_application_acceptance.py'))
                redis_module=importlib.util.module_from_spec(redis_spec)
                redis_spec.loader.exec_module(redis_module)
                result=redis_module.exercise(api,create,start,done,check,command,mode,workspace_ops,host,kind,
                    deployment_workdir,templates,projects,engine if mode=='stack' else None,swarm_nodes,
                    swarm_ids if mode=='stack' else {},name,work,port)
                report.setdefault('redis_example_runs',[]).append(result)
            if test_nats:
                assert os.environ.get('FERRY_TEST_SCENARIO_TEMPLATES') == '1', 'NATS acceptance requires initialized scenario templates'
                import asyncio
                import importlib.util
                protocol_spec=importlib.util.spec_from_file_location('nats_application_acceptance',Path(__file__).with_name('nats_application_acceptance.py'))
                protocol_module=importlib.util.module_from_spec(protocol_spec)
                protocol_spec.loader.exec_module(protocol_module)
                verify_nats=protocol_module.verify
                nats_template = next(t for t in templates if json.loads(t['content'])['metadata']['name'] == 'example-nats')
                nats_config = next(t for t in api(f'/workspaces/{workspace_ops}/config-templates')['items'] if t['name']=='example-nats-config')
                report['nats_scenario_templates']={'application':nats_template,'configuration':nats_config}
                token_key='nats-token-'+kind
                # NATS parses environment values as configuration literals. A
                # letter prefix keeps random hex from being parsed as a number.
                token='nats-'+uuid.uuid4().hex+uuid.uuid4().hex
                create('variables',{'name':token_key,'scope':'deploy','kind':'secret','value':token})
                published_ports = [18082,18083,18084] if mode=='stack' else nats_host_ports
                values={'token':token_key}
                for suffix, published, advertised in zip(['a','b','c'],published_ports,nats_host_ports):
                    values['port_'+suffix]=published
                    values['advertise_'+suffix]='127.0.0.1:'+str(advertised)
                generated=api(f'/workspaces/{workspace_ops}/app-templates/'+nats_template['id']+'/renderings',{'version':1,'values':values})['manifest']
                if mode=='stack':
                    assert len(swarm_nodes)==3, 'NATS Stack acceptance requires three owned daemons'
                    for suffix,node in zip(['a','b','c'],swarm_nodes):
                        engine(['node','update','--label-add','ferry-nats='+suffix,swarm_ids[node]])
                        engine(['pull',json.loads(generated)['services'][0]['image']],node)
                sample=create('deployments',{'name':'NATS example '+kind,'mode':mode,'target_kind':'host','target_ref':host,'workdir':deployment_workdir,'manifest':generated})
                sample_project=f"deployment-{workspace['id']}-{sample}";projects.append(sample_project)
                def nats_diagnostics():
                    output=[]
                    def capture(args):
                        result=subprocess.run(args,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=30)
                        output.append(result.stdout.decode(errors='replace')[-24000:].replace(token,'[REDACTED]'))
                        return result.stdout.decode(errors='replace').split()
                    try:
                        for node in (swarm_nodes if mode=='stack' else ['host']):
                            prefix=['docker','exec',name+'-'+node,'/usr/bin/docker'] if mode=='stack' else ['docker']
                            label=('com.docker.stack.namespace=' if mode=='stack' else 'com.docker.compose.project=')+sample_project
                            ids=capture(prefix+['ps','-aq','--filter','label='+label])
                            for container in ids:
                                capture(prefix+['inspect','--format','{{.Name}} {{json .State}}',container])
                                capture(prefix+['logs','--tail','100',container])
                        if mode=='stack':
                            for member in ['nats-a','nats-b','nats-c']:
                                capture(['docker','exec',name+'-manager','/usr/bin/docker','service','ps','--no-trunc',sample_project+'_'+member])
                    except Exception as cause: output.append('Diagnostic collection: '+type(cause).__name__)
                    (work/('nats-'+kind+'-diagnostics.log')).write_text('\n'.join(output),encoding='utf8')
                    print('\n'.join(output)[-6000:],flush=True)
                sample_revision=start(sample);sample_row=done(sample)
                if sample_row['state']!='succeeded': nats_diagnostics()
                check(kind+' NATS template deployment succeeds: '+sample_row['latest_revision']['output'],sample_row['state']=='succeeded')
                check(kind+' NATS secret absent from public deployment and history',token not in json.dumps(sample_row))
                if mode=='stack':
                    for suffix,node in zip(['a','b','c'],swarm_nodes):
                        tasks=json.loads(engine(['service','ps','--no-trunc','--filter','desired-state=running','--format','{{json .}}',sample_project+'_nats-'+suffix]).decode().splitlines()[0])
                        check(kind+' NATS member '+suffix+' placed on assigned node',tasks['Node']==engine(['info','--format','{{.Name}}'],node).decode().strip())
                    def stop_member(member):
                        service=sample_project+'_'+member
                        engine(['service','scale','--detach',service+'=0'])
                        eventually(lambda:all(not engine(['ps','-q','--filter','label=com.docker.swarm.service.name='+service],node).decode().strip() for node in swarm_nodes))
                    def restore_member(member): engine(['service','scale','--detach',sample_project+'_'+member+'=1'])
                else:
                    members={member:command(['docker','ps','-q','--filter','label=com.docker.compose.project='+sample_project,'--filter','label=com.docker.compose.service='+member]).decode().strip() for member in ['nats-a','nats-b','nats-c']}
                    check(kind+' NATS members have independent containers',len(set(members.values()))==3 and all(members.values()))
                    def stop_member(member): command(['docker','stop','-t','5',members[member]])
                    def restore_member(member): command(['docker','start',members[member]])
                try:
                    protocol=asyncio.run(verify_nats(['nats://127.0.0.1:'+str(p) for p in nats_host_ports],token,
                        lambda label,result:check(kind+' '+label,result),stop_member,restore_member))
                except Exception:
                    nats_diagnostics()
                    raise
                report.setdefault('nats_example_runs',[]).append({'transport':kind,'deployment':sample,'revision':sample_revision,'protocol':protocol})
                if mode=='stack':
                    engine(['stack','rm',sample_project])
                    eventually(lambda:not engine(['service','ls','-q','--filter','label=com.docker.stack.namespace='+sample_project]).decode().strip())
                else:
                    command(['docker','rm','-f',*members.values()])
                api(f'/workspaces/{workspace_ops}/deployments/'+sample,method='DELETE',status=204)
            if mode=='stack':
                web_port=native_port
                source=manifest('stack-v1',18081)
                source['services'][0]['replicas']=2
                if swarm_ha:
                    source['apiVersion']='ferry/v2'
                    source['services'][0]['placement']={'constraints':['node.labels.ferry-test == eligible'], 'maxReplicasPerNode':1}

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
                if swarm_ha:
                    distribution={node:task_containers(node) for node in swarm_nodes}
                    check(kind+' Swarm distributes two replicas on distinct nodes',sum(map(len,distribution.values())) == 2 and sum(bool(tasks) for tasks in distribution.values()) == 2)
                    spec=json.loads(engine(['service','inspect',project+'_web']))[0]['Spec']
                    check(kind+' v2 placement reaches the actual scheduler',spec['TaskTemplate']['Placement']['Constraints'] == ['node.labels.ferry-test == eligible'] and spec['TaskTemplate']['Placement']['MaxReplicas'] == 1)
                else:
                    check(kind+' Swarm schedules on manager and worker',bool(task_containers('manager')) and bool(task_containers('worker')))
                for node in swarm_nodes:
                    containers=task_containers(node)
                    if swarm_ha and not containers: continue
                    container=containers[0]
                    check(kind+' '+node+' receives config through Swarm secret',engine(['exec',container,'cat','/etc/runtime.conf'],node).decode()=='token='+secret)
                    check(kind+' '+node+' multiline environment preserved',engine(['exec',container,'printenv','TOKEN'],node).decode()==secret+'\n')
                    check(kind+' '+node+' literal environment preserved',engine(['exec',container,'printenv','LITERAL'],node).decode().strip()=='${NOT_INTERPOLATED}')
                saved=json.loads(row['latest_revision']['artifact'])
                check(kind+' Swarm history contains no secret or manager bind',secret not in json.dumps(row) and all(v['type']!='bind' for v in saved['services']['web'].get('volumes',[])))
                if swarm_ha:
                    service_name=project+'_web'
                    by_id={node_id: node for node,node_id in swarm_ids.items()}
                    def healthy_tasks():
                        ids=engine(['service','ps','-q','--filter','desired-state=running',service_name]).decode().split()
                        if len(ids) != 2: return None
                        tasks=json.loads(engine(['inspect',*ids]))
                        if any(task['Status']['State'] != 'running' for task in tasks): return None
                        result=[]
                        for task in tasks:
                            node=by_id[task['NodeID']]
                            container_id=task['Status']['ContainerStatus']['ContainerID']
                            inspected=subprocess.run(['docker','exec',name+'-'+node,'/usr/bin/docker','inspect',container_id],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                            if inspected.returncode: return None
                            container=json.loads(inspected.stdout)[0]
                            if container['State'].get('Health',{}).get('Status') != 'healthy': return None
                            result.append({'task_id':task['ID'], 'node':node, 'container_id':container_id, 'state':task['Status']['State']})
                        return result if len({task['node'] for task in result}) == 2 else None
                    def capture(stage, tasks):
                        report['snapshots'].append({'transport':kind, 'stage':stage, 'tasks':tasks})
                    before=eventually(healthy_tasks,120);capture('before-task-exit',before)
                    victim=before[0]
                    engine(['kill',victim['container_id']],victim['node'])
                    def replaced():
                        tasks=healthy_tasks()
                        return tasks if tasks and victim['task_id'] not in {task['task_id'] for task in tasks} else None
                    after=eventually(replaced,120);capture('after-task-exit',after)
                    check(kind+' Swarm replaces an exited task and restores healthy replicas',eventually(lambda:response(web_port)=='stack-v1'))
                    victim_node=next(task['node'] for task in after if task['node'] != 'manager')
                    command(['docker','stop','-t','5',name+'-'+victim_node])
                    eventually(lambda: json.loads(engine(['node','inspect',swarm_ids[victim_node]]))[0]['Status']['State'] == 'down',120)
                    def rescheduled():
                        tasks=healthy_tasks()
                        return tasks if tasks and all(task['node'] != victim_node for task in tasks) else None
                    recovered=eventually(rescheduled,150);capture('after-worker-down',recovered)
                    check(kind+' Swarm reschedules from an offline worker onto remaining nodes',eventually(lambda:response(web_port)=='stack-v1'))
                    command(['docker','start',name+'-'+victim_node])
                    eventually(lambda:daemon_ready(victim_node),120)
                    eventually(lambda:json.loads(engine(['node','inspect',swarm_ids[victim_node]]))[0]['Status']['State'] == 'ready',120)
                    capture('after-worker-return',eventually(healthy_tasks,120))
                    check(kind+' returning worker rejoins without a Ferry redeploy',eventually(lambda:response(web_port)=='stack-v1') and app(id)['latest_revision']['id']==first)
                if http_template is not None:
                    next(item for item in source['services'][0]['env'] if item['name']=='WELCOME_MESSAGE')['value']='stack-v2'
                else:
                    source['services'][0]['command'][2]=source['services'][0]['command'][2].replace('stack-v1','stack-v2')
                update(id,source);start(id);row=done(id)
                check(kind+' Swarm rolling update completes',row['state']=='succeeded' and eventually(lambda:response(web_port)=='stack-v2'))
                spec=json.loads(engine(['service','inspect',project+'_web']))[0]['Spec']
                check(kind+' Swarm start-first policy applied',spec['UpdateConfig']['Order']=='start-first')
                api(f'/workspaces/{workspace_ops}/variables/'+variable,{'name':key,'version':1,'value':'rotated'},method='PATCH')
                start(id,first);row=done(id)
                check(kind+' Swarm rollback restores historical image and config',row['state']=='succeeded' and eventually(lambda:response(web_port)=='stack-v1'))
                for node in swarm_nodes:
                    for container in task_containers(node):
                        check(kind+' Swarm rollback restores historical variable',engine(['exec',container,'printenv','TOKEN'],node).decode()==secret+'\n')
                restarted=scheduled_restart(id,'swarm-restart-'+kind)
                check(kind+' Swarm scheduled restart converges and preserves historical variables',restarted['state']=='succeeded' and eventually(lambda:response(web_port)=='stack-v1') and all(engine(['exec',container,'printenv','TOKEN'],node).decode()==secret+'\n' for node in swarm_nodes for container in task_containers(node)))
                if swarm_ha:
                    unavailable=json.loads(json.dumps(source))
                    unavailable['strategy']['healthTimeout']=5
                    unavailable['services'][0]['placement']['constraints']=['node.labels.ferry-test == unavailable']
                    update(id,unavailable);start(id);unavailable_row=done(id)
                    check(kind+' insufficient eligible nodes cannot report deployment success',unavailable_row['state']=='failed')
                    spec=json.loads(engine(['service','inspect',project+'_web']))[0]['Spec']
                    check(kind+' scheduler does not silently remove unmet placement constraints',spec['TaskTemplate']['Placement']['Constraints'] == ['node.labels.ferry-test == unavailable'])
                    start(id,first);check(kind+' rollback restores schedulable placement',done(id)['state']=='succeeded')
                source['strategy']['healthTimeout']=5
                source['services'][0]['health']['readiness']['httpGet']['path']='/not-ready'
                update(id,source);start(id);row=done(id)
                check(kind+' Swarm readiness failure cannot report success',row['state']=='failed')
                start(id,first);row=done(id)
                check(kind+' Swarm explicit rollback recovers: '+row['latest_revision']['output'],row['state']=='succeeded')
                check(kind+' Swarm explicit rollback restores historical HTTP',eventually(lambda:response(web_port)=='stack-v1'))
                spec=json.loads(engine(['service','inspect',project+'_web']))[0]['Spec']
                check(kind+' Swarm explicit rollback preserves start-first updates',spec['UpdateConfig']['Order']=='start-first')
                if swarm_ha:
                    check(kind+' Swarm rollback retains placement limits',spec['TaskTemplate']['Placement']['Constraints']==['node.labels.ferry-test == eligible'] and spec['TaskTemplate']['Placement']['MaxReplicas']==1)
                    check(kind+' Swarm rollback leaves exactly two healthy tasks',len(eventually(healthy_tasks,120))==2)
                stage_left=command(['docker','exec',name,'find',deployment_workdir,'-name','stack-*']).decode()
                check(kind+' Swarm private runtime files cleaned',not stage_left.strip())
                engine(['stack','rm',project])
                eventually(lambda:not engine(['service','ls','-q','--filter','label=com.docker.stack.namespace='+project]).strip())
                # Wait for ingress port release before the next transport scenario.
                eventually(lambda:all(not task_containers(node) for node in swarm_nodes))
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
                definition=artifact_workflow('/tmp/native-build-'+kind+'/{{run}}','printf %s '+shlex.quote(executable('native-v1'))+' > app; chmod 755 app')
                pipeline=create('pipelines',{'name':'native-build-'+kind,'target_kind':'host','target_ref':host,'repository_id':repo,'definition_format':'github-actions','definition':json.dumps(definition)})
                def build(pipeline_id=pipeline):
                    rid=api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline_id+'/runs',{},status=201)['id']
                    def finished():
                        run=api(f'/workspaces/{workspace_ops}/pipeline-runs/'+rid)
                        return run if run['state'] not in ('queued','running') else None
                    run=eventually(finished)
                    check(kind+' native executable archived',run['state']=='succeeded' and len(run['artifacts'])==1)
                    return rid
                if native_template is not None:
                    generated = api(f'/workspaces/{workspace_ops}/pipeline-templates/'+native_build_template['id']+'/renderings',
                                    {'version':native_build_template['version'],'values':{}})['definition']
                    sample_pipeline = create('pipelines',{'name':'sample-native-'+kind,'target_kind':'host','target_ref':host,
                        'repository_id':repo,'definition_format':'github-actions','definition':json.loads(generated)['workflow']})
                    sample_run = build(sample_pipeline)
                    message = "Native example 'quote' $HOME %\nsecond line 你好"
                    generated = api(f'/workspaces/{workspace_ops}/app-templates/'+native_template['id']+'/renderings',
                                    {'version':native_template['version'],'values':{'pipeline':sample_pipeline,'port':18081,'message':message}})['manifest']
                    sample = create('deployments',{'name':'Native example '+kind,'mode':'native','target_kind':'host','target_ref':host,
                        'workdir':deployment_workdir,'manifest':generated})
                    sample_project=f"deployment-{workspace['id']}-{sample}";projects.append(sample_project)
                    sample_revision=start(sample);sample_row=done(sample)
                    check(kind+' initialized Native example deploys: '+sample_row['latest_revision']['output'],sample_row['state']=='succeeded')
                    check(kind+' initialized Native example literal HTTP body',eventually(lambda:response(web_port)==message))
                    check(kind+' initialized Native example readiness endpoint',remote('curl -fsS http://127.0.0.1:18081/health')=='ok')
                    check(kind+' initialized Native example rejects unknown paths',remote("curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:18081/missing")=='404')
                    check(kind+' initialized Native example persists enabled unit',remote('systemctl is-enabled '+sample_project+'-web.service').strip()=='enabled')
                    report.setdefault('native_example_runs',[]).append({'transport':kind,'pipeline':sample_pipeline,'run':sample_run,'deployment':sample,'revision':sample_revision})
                    remote('systemctl disable --now '+sample_project+'-web.service')
                    api(f'/workspaces/{workspace_ops}/deployments/'+sample,method='DELETE',status=204)
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
                definition['jobs']['build']['steps'][1]['run']='printf %s '+shlex.quote(executable('native-v2'))+' > app; chmod 755 app'
                pipe=api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline);api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline,dict(pipe,definition=json.dumps(definition)),method='PATCH')
                build2=build();start(id);row=done(id)
                check(kind+' native update uses latest successful archive',row['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v2'))
                api(f'/workspaces/{workspace_ops}/variables/'+variable,{'name':key,'version':1,'value':'rotated'},method='PATCH')
                remote('rm -rf /tmp/native-build-'+kind)
                start(id,first);row=done(id)
                check(kind+' native rollback restores archived executable',row['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v1'))
                check(kind+' native rollback restores old config and environment',remote('cat /tmp/native-env-'+kind)==secret and remote('cat /tmp/native-config-'+kind)=='token='+secret)
                remote('systemctl restart '+project+'-web.service')
                check(kind+' native host restart retains secret files',eventually(lambda:response(web_port)=='native-v1') and remote('cat /tmp/native-env-'+kind)==secret)
                source['strategy']={'type':'recreate','healthTimeout':3};source['services'][0]['health']['readiness']['httpGet']['path']='/not-ready'
                update(id,source);start(id);row=done(id)
                check(kind+' native failed readiness not accepted',row['state']=='failed')
                start(id,first);row=done(id);check(kind+' native explicit rollback recovers: '+row['latest_revision']['output'],row['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v1'))
                # Keep automatic recovery pending after the fifth start fails.
                # A manual rollback must clear the counter even in ActiveState=activating.
                unit=project+'-web.service'
                remote('mkdir -p /etc/systemd/system/'+unit+'.d; printf "[Unit]\\nStartLimitIntervalSec=120\\nStartLimitBurst=5\\n[Service]\\nRestartSec=30\\n" > /etc/systemd/system/'+unit+'.d/start-limit.conf; systemctl daemon-reload; systemctl reset-failed '+unit)
                for attempt in range(4): remote('systemctl restart '+unit)
                start(id);row=done(id)
                check(kind+' native repeated failed readiness not accepted',row['state']=='failed')
                check(kind+' native automatic recovery is pending',remote('systemctl show '+unit+' -p ActiveState --value').strip()=='activating')
                start(id,first);row=done(id)
                check(kind+' native rollback during automatic recovery: '+row['latest_revision']['output'],row['state']=='succeeded' and eventually(lambda:response(web_port)=='native-v1'))
                remote('rm -f /etc/systemd/system/'+unit+'.d/start-limit.conf; systemctl daemon-reload; systemctl reset-failed '+unit)
                api(f'/workspaces/{workspace_other}/deployments/'+id,status=404);check(kind+' native tenant isolation',True)
                api(f'/workspaces/{workspace_ops}/pipelines/'+pipeline,method='DELETE',status=409)
                check(kind+' native pipeline reference prevents deletion',True)
                if not os.environ.get('FERRY_TEST_DATABASE_URL'):
                    import sqlite3
                    unused=build()
                    with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                        old=int(time.time()*1000)-3*86400000
                        db.execute('UPDATE pipeline_runs SET finished_at=? WHERE id IN (?,?,?)',(old,build1,build2,unused))
                    retention=next(item for item in api(f'/workspaces/{workspace_ops}/settings')['items'] if item['key']=='history_retention_days')
                    api(f'/workspaces/{workspace_ops}/settings/'+retention['id'],{'version':retention['version'],'value':1},method='PATCH')
                    def pruned():
                        with sqlite3.connect(work/'data'/'ferry.sqlite') as db:
                            return db.execute('SELECT count(*) FROM pipeline_runs WHERE id=?',(unused,)).fetchone()[0]==0
                    check(kind+' native retention removes only unreferenced build',eventually(pruned,75))
                    for rid in (build1,build2):
                        check(kind+' native retained deployment preserves archive '+rid,len(api(f'/workspaces/{workspace_ops}/pipeline-runs/'+rid)['artifacts'])==1)
                    start(id,first);check(kind+' native rollback still works after history pruning',done(id)['state']=='succeeded')
                    retention=next(item for item in api(f'/workspaces/{workspace_ops}/settings')['items'] if item['key']=='history_retention_days')
                    api(f'/workspaces/{workspace_ops}/settings/'+retention['id'],{'version':retention['version'],'value':0},method='PATCH')
                remote('systemctl disable --now '+project+'-web.service')
                api(f'/workspaces/{workspace_ops}/deployments/'+id,method='DELETE',status=204)
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
                if http_template is not None:
                    next(item for item in source['services'][0]['env'] if item['name']=='WELCOME_MESSAGE')['value']='docker-v2'
                else:
                    source['services'][0]['command'][2]=source['services'][0]['command'][2].replace('docker-v1','docker-v2')
                update(id,source);start(id);row=done(id)
                check(kind+' Docker update changes running service',row['state']=='succeeded' and eventually(lambda:response(web_port)=='docker-v2'))
                api(f'/workspaces/{workspace_ops}/variables/'+variable,{'name':key,'version':1,'value':'rotated'},method='PATCH')
                start(id,first);row=done(id)
                check(kind+' Docker rollback restores HTTP and historical secret',row['state']=='succeeded' and eventually(lambda:response(web_port)=='docker-v1') and command(['docker','exec',project+'-web','printenv','TOKEN']).decode()==secret+'\n')
                cron_id=create('cron-jobs',{'name':'docker-restart-'+kind,'scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'restart','target_kind':'deployment','target_ref':id,'command':'','timeout_seconds':60,'max_retries':0,'retry_delay_seconds':1})
                cron_run=api(f'/workspaces/{workspace_ops}/cron-jobs/'+cron_id+'/runs',{},status=202)['id']
                def restart_finished():
                    result=api(f'/workspaces/{workspace_ops}/cron-runs/'+cron_run)
                    return result if result['state'] not in ('queued','running') else None
                restarted=eventually(restart_finished,90)
                check(kind+' Docker scheduled restart succeeds',restarted['state']=='succeeded' and app(id)['latest_revision']['action']=='restart')
                api(f'/workspaces/{workspace_ops}/cron-jobs/'+cron_id,method='DELETE',status=204)
                invalid=manifest('bad',web_port);invalid['services'][0]['replicas']=2
                original=app(id);api(f'/workspaces/{workspace_ops}/deployments/'+id,dict(original,manifest=json.dumps(invalid)),method='PATCH',status=400)
                check(kind+' Docker rejects replicas before execution',True)
                source['strategy']={'type':'recreate','healthTimeout':3}
                source['services'][0]['health']['readiness']['httpGet']['path']='/not-ready'
                update(id,source);start(id);row=done(id)
                check(kind+' Docker readiness failure cannot report success',row['state']=='failed')
                start(id,first);check(kind+' Docker recovers by explicit rollback',done(id)['state']=='succeeded')
                api(f'/workspaces/{workspace_other}/deployments/'+id,status=404)
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
            generated=api(f'/workspaces/{workspace_ops}/app-templates/'+template+'/renderings',{'version':1,'values':{'image':image_value}})['manifest']
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
            api(f'/workspaces/{workspace_ops}/variables/'+config_variable,{'name':config_key,'version':1,'value':'rotated-config-value'},method='PATCH')
            api(f'/workspaces/{workspace_ops}/config-templates/'+config_template,method='DELETE',status=409)
            check(kind+' referenced configuration template cannot be deleted',True)
            config_row=api(f'/workspaces/{workspace_ops}/config-templates/'+config_template)
            api(f'/workspaces/{workspace_ops}/config-templates/'+config_template,dict(config_body,version=config_row['version'],content='changed={{ .Values.password }}'),method='PATCH')
            update(id,manifest('v2',web_port)); start(id); row=done(id)
            check(kind+' update succeeds',row['state']=='succeeded')
            check(kind+' HTTP serves v2',eventually(lambda:response(web_port)=='v2'))
            start(id,rev1); row=done(id)
            check(kind+' rollback succeeds',row['state']=='succeeded')
            check(kind+' exact artifact reused',row['latest_revision']['artifact']==artifact)
            rollback_container=command(['docker','ps','-q','--filter','label=com.docker.compose.project='+project]).decode().strip()
            check(kind+' rollback restores original config despite template edits',command(['docker','exec',rollback_container,'cat','/etc/runtime.conf']).decode()=='listen=8080\nliteral=${HOME} $$ $plain\npassword='+config_secret+'\n')
            check(kind+' HTTP returns v1',eventually(lambda:response(web_port)=='v1'))
            check(kind+' nested history',api(f'/workspaces/{workspace_ops}/deployments/'+id+'/revisions')['total']==3)
            api(f'/workspaces/{workspace_other}/deployments/'+id+'/revisions',status=404)
            api(f'/workspaces/{workspace_other}/deployments/'+id,status=404); check(kind+' tenant isolation',True)
            api(f'/workspaces/{workspace_other}/deployment-revisions/'+rev1,status=404); check(kind+' revision isolation',True)
            stale=app(id); stale['version']=0;api(f'/workspaces/{workspace_ops}/deployments/'+id,stale,method='PATCH',status=409);check(kind+' stale update rejected',True)
            changed=app(id);changed['target_ref']=ssh_id if host==runner_id else runner_id
            api(f'/workspaces/{workspace_ops}/deployments/'+id,changed,method='PATCH',status=400);check(kind+' immutable host',True)
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
                    try: return api(f'/workspaces/{workspace_ops}/nodes/'+runner_id+'/connection-tests',{})['ok']
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
            api(f'/workspaces/{workspace_ops}/deployments/{id}/executions',payload,status=403,client=viewer)
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
            bound=json.loads(api(f'/workspaces/{workspace_ops}/app-templates/'+secret_template+'/renderings',{'version':1,'values':{'token':key}})['manifest'])
            update(id,bound)
            inherited=preview(id)
            check(kind+' module overrides global',next(v for v in inherited['variables'] if v['name']==key)['scope']=='deploy')
            object_id=create('variables',{'name':key,'scope':'deploy','scope_ref':id,'kind':'secret','value':secret})
            action(id,'apply',dict(version=inherited['version'],expected_hash=inherited['expected_hash'],preview_token=inherited['preview_token']),status=409)
            check(kind+' new object override invalidates preview',True)
            api(f'/workspaces/{workspace_ops}/variables/'+object_id,method='DELETE',status=409)
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
            api(f'/workspaces/{workspace_ops}/variables/'+object_id,{'name':key,'version':1,'value':'fixture-secret-v2'},method='PATCH')
            action(id,'apply',dict(version=stale['version'],expected_hash=stale['expected_hash'],preview_token=stale['preview_token']),status=409)
            check(kind+' rotation invalidates old preview',True)
            fresh=preview(id)
            command(['docker','exec',name,'touch','/tmp/deployment-up-pause'])
            action(id,'apply',dict(version=fresh['version'],expected_hash=fresh['expected_hash'],preview_token=fresh['preview_token']),status=202)
            paths=eventually(env_paths)
            check(kind+' environment file permissions are private',all(command(['docker','exec',name,'stat','-c','%a',path]).strip()==b'600' for path in paths))
            api(f'/workspaces/{workspace_ops}/variables/'+object_id,{'name':key,'version':2,'value':'fixture-secret-v3'},method='PATCH')
            command(['docker','exec',name,'rm','/tmp/deployment-up-pause'])
            check(kind+' queued snapshot not changed by later rotation',done(id)['state']=='succeeded' and container_env()['BOUND_SECRET']=='fixture-secret-v2')
            start(id,secret_rev)
            check(kind+' rollback restores historical secret',done(id)['state']=='succeeded' and container_env()['BOUND_SECRET']==secret)
            alert_id=create('alert-rules',{'name':'Deployment failure '+kind,'target_kind':'deployment','target_ref':id,'metric':'execution-failed','enabled':True})
            def alert_state(state):
                return any(e['state']==state for e in api(f'/workspaces/{workspace_ops}/alert-events?rule_id='+alert_id)['items'])
            command(['docker','exec',name,'touch','/tmp/deployment-fail','/tmp/deployment-leak'])
            start(id); failed=done(id)
            check(kind+' secret-bearing command error output suppressed',failed['state']=='failed' and 'fixture-secret' not in json.dumps(failed) and 'fixture-command-failed' not in json.dumps(failed))
            command(['docker','exec',name,'rm','/tmp/deployment-fail','/tmp/deployment-leak'])
            check(kind+' failed execution cleans secret files',env_paths()==[])
            check(kind+' actual deployment failure triggers alert',eventually(lambda:alert_state('firing')))
            update(id,manifest('v2',web_port))
            for variable in [object_id,module_id,global_id,empty_id]:
                api(f'/workspaces/{workspace_ops}/variables/'+variable,method='DELETE',status=204)
            start(id,secret_rev)
            check(kind+' history remains usable after current variables deleted',done(id)['state']=='succeeded' and container_env()['BOUND_SECRET']==secret)
            check(kind+' historical files cleaned again',env_paths()==[])
            check(kind+' actual successful rollback recovers alert',eventually(lambda:alert_state('recovered')))
            api(f'/workspaces/{workspace_ops}/alert-rules/'+alert_id,method='DELETE',status=204)
            cron={'name':'scheduled-'+kind,'scope':'deploy','schedule':'0 * * * *','timezone':'UTC','enabled':False,'action':'restart','target_kind':'deployment','target_ref':id,'command':'','timeout_seconds':60,'max_retries':0,'retry_delay_seconds':1}
            cron_id=create('cron-jobs',cron)
            def scheduled():
                run_id=api(f'/workspaces/{workspace_ops}/cron-jobs/'+cron_id+'/runs',{},status=202)['id']
                def finished():
                    result=api(f'/workspaces/{workspace_ops}/cron-runs/'+run_id)
                    return result if result['state'] not in ('queued','running') else None
                return eventually(finished,90)
            result=scheduled()
            check(kind+' scheduled restart succeeds: '+result['output'],result['state']=='succeeded')
            check(kind+' restart keeps historical secret after current variable deletion',container_env()['BOUND_SECRET']==secret)
            check(kind+' restart revision explicitly recorded',app(id)['latest_revision']['action']=='restart')
            cron_row=api(f'/workspaces/{workspace_ops}/cron-jobs/'+cron_id)
            api(f'/workspaces/{workspace_ops}/cron-jobs/'+cron_id,dict(cron_row,action='redeploy'),method='PATCH')
            result=scheduled()
            check(kind+' scheduled redeployment succeeds: '+result['output'],result['state']=='succeeded' and response(web_port)=='v2')
            check(kind+' redeploy uses current manifest', 'BOUND_SECRET' not in container_env())
            api(f'/workspaces/{workspace_ops}/deployments/'+id,method='DELETE',status=409)
            api(f'/workspaces/{workspace_ops}/cron-jobs/'+cron_id,method='DELETE',status=204)
            # Delete metadata only: the existing v2 HTTP service keeps running.
            api(f'/workspaces/{workspace_ops}/deployments/'+id,method='DELETE',status=204)
            check(kind+' deletion keeps containers running',response(web_port)=='v2')
        if swarm_ha:
            expected={(transport,stage) for transport in ('runner','ssh') for stage in ('before-task-exit','after-task-exit','after-worker-down','after-worker-return')}
            check('fault recovery snapshots cover both deployment transports', {(row['transport'],row['stage']) for row in report['snapshots']} == expected)
        check('check floor',checks>=(35 if mode=='compose' else 30))
        report['result']='PASS'
        print(f'{checks} passed, 0 failed, 0 skipped',flush=True)
    finally:
        if report['result'] == 'RUNNING': report['result']='FAIL'
        (work/'acceptance.json').write_text(json.dumps(report,indent=2),encoding='utf8')
        if os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('FERRY_TEST_SCENARIO_TEMPLATES') == '1':
            summary = {key:report[key] for key in ('mode','boundary','result')}
            if 'compose' in report: summary['compose'] = report['compose']
            summary['source_commit'] = os.environ.get('GITHUB_SHA','')
            summary['template_sha256'] = hashlib.sha256(Path('crates/server/src/templates/examples/http-service.json').read_bytes()).hexdigest()
            summary['checks'] = [{'name':row['name'][:180],'passed':row['passed']} for row in report['checks']]
            summary['check_count'] = len(report['checks'])
            summary['scenario_initialized'] = 'scenario_template' in report
            summary['full_report'] = 'acceptance.json'
            # GitHub truncates annotations at 4096 bytes. Keep a valid summary
            # with the final checks; the artifact contains the complete report.
            while len(json.dumps(summary,separators=(',',':')).encode()) > 3800:
                summary['checks'].pop(0)
            notice = json.dumps(summary,separators=(',',':')).replace('%','%25').replace('\r','%0D').replace('\n','%0A')
            print('::notice title=Application scenario evidence::'+notice,flush=True)
        for project in projects:
            ids=command(['docker','ps','-aq','--filter',('label=com.docker.compose.project=' if mode=='compose' else 'label=workspace.deployment=')+project]).decode().split()
            if ids: subprocess.run(['docker','rm','-f',*ids],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            nets=command(['docker','network','ls','-q','--filter','label=com.docker.compose.project='+project]).decode().split()
            if nets: subprocess.run(['docker','network','rm',*nets],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            if mode=='compose':
                owned_volumes=command(['docker','volume','ls','-q','--filter','label=com.docker.compose.project='+project]).decode().split()
                if owned_volumes: subprocess.run(['docker','volume','rm',*owned_volumes],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for p in reversed(processes):
            if p.poll() is None: p.kill();p.wait()
        for child in extra_containers: subprocess.run(['docker','rm','-f',child],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        if test_blueprint: subprocess.run(['docker','network','rm',name+'-blueprint'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','rm','-f',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','volume','rm',name+'-files'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        if mode=='stack':
            for node in swarm_nodes:
                result=subprocess.run(['docker','logs',name+'-'+node],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
                (work/(node+'-daemon.log')).write_bytes(result.stdout)
                subprocess.run(['docker','rm','-fv',name+'-'+node],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            subprocess.run(['docker','volume','rm',name+'-socket'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            subprocess.run(['docker','network','rm',name+'-swarm'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for f in files:f.close()
        print('fixture logs: '+str(work),flush=True)


if __name__=='__main__': main()
