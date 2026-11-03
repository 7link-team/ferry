#!/usr/bin/env python3
"""Real server -> act -> Git collector -> ZIP artifact -> report acceptance on Windows.

Arguments: SERVER RUNNER WORKERS_DIR. Uses only disposable local data and Git history.
FERRY_TEST_GIT_AUTH=basic tests private HTTP; =ssh tests private key-only SSH.
FERRY_TEST_GIT_MONITOR=1 runs independent Runner refs monitoring without act/statistics.
FERRY_TEST_CODE_INSIGHTS=1 exercises standalone insights over the same real act path.
SSH needs rustc and Paramiko (global installation is unnecessary: pip --target
data/maintenance/ssh-test-deps paramiko is discovered automatically). Host trust
is pinned in a temporary Git transport SSH shim; user SSH settings stay untouched.
Monitor mode also needs rustc for its temporary process-launch audit shim.
"""
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from functools import partial
import threading
import io
import zipfile
import hashlib
import base64
import secrets
import shlex
import shutil
import sqlite3
from contextlib import closing


def monitor_git_environment(work, flags, ssh=None):
    """Test-only launch audit and isolated Runner host-key provisioning."""
    actual_git = Path(shutil.which('git')).as_posix()
    real_exec = subprocess.check_output(['git', '--exec-path'], **flags).decode().strip()
    tools = work/'monitor-tools'
    tools.mkdir()
    # Git for Windows initializes its own PATH before running the script. A
    # disposable installation layout lets us add the audit/host-trust shims
    # inside that shell, without altering the installed Git or user account.
    shell_bin = tools/'bin'
    shell_bin.mkdir()
    discovery_exec = tools/'mingw64/libexec/git-core'
    discovery_exec.mkdir(parents=True)
    actual_sh = Path(real_exec).parents[2]/'bin/sh.exe'
    shell_source = tools/'sh.rs'
    def msys_path(path):
        value = Path(path).as_posix()
        return '/' + value[0].lower() + value[2:]
    shell_prefix = msys_path(tools) + (':' + msys_path(ssh.exec_path) if ssh else '')
    shell_source.write_text('use std::{env,process::{Command,exit}};\n'
        'use std::os::windows::process::CommandExt;\nfn main() { '
        'let status=Command::new(' + json.dumps(actual_sh.as_posix()) + ')'
        '.args(["-c",' + json.dumps('export PATH=' + shlex.quote(shell_prefix) + ':"$PATH"; . "$1"')
        + ',"monitor-shell"]).args(env::args_os().skip(1))'
        '.stderr(std::process::Stdio::from(std::fs::OpenOptions::new().create(true).append(true).open('
        + json.dumps((work/'monitor-shell.log').as_posix()) + ').unwrap()))'
        '.creation_flags(0x08000000).status().expect("launch actual sh"); '
        'exit(status.code().unwrap_or(1)); }\n', encoding='utf8')
    subprocess.run(['rustc','--edition=2024',str(shell_source),'-o',str(shell_bin/'sh.exe')],
                   check=True,capture_output=True,**flags)
    launch_log = work/'git-launches.log'
    script = tools/'git.rs'
    script.write_text('use std::{env,fs::OpenOptions,io::Write,process::{Command,exit}};\n'
        'use std::os::windows::process::CommandExt;\nfn main() { '
        'let mut log=OpenOptions::new().create(true).append(true).open('
        + json.dumps(launch_log.as_posix()) + ').unwrap(); '
        'let args:Vec<_>=env::args_os().skip(1).collect(); '
        'let kind=if args.iter().any(|a| a=="ls-remote") { "ls-remote" } else { "discovery" }; '
        'writeln!(log,"{}:{}:ssh={}",env::var("FERRY_TEST_GIT_ORIGIN").unwrap_or_default(),kind,env::var("GIT_SSH_COMMAND").is_ok()).unwrap(); '
        'if args.len()==1 && args[0]=="--exec-path" { println!("{}",'
        + json.dumps(discovery_exec.as_posix()) + '); return; } '
        + ('if let Ok(command)=env::var("GIT_SSH_COMMAND") { '
           'unsafe { env::set_var("GIT_SSH_COMMAND",command.replacen("ssh ",'
           + json.dumps(shlex.quote((Path(ssh.exec_path)/'ssh.exe').as_posix()) + ' ')
           + ',1)); } } ' if ssh else '') +
        'let status=Command::new(' + json.dumps(actual_git) + ').args(args)'
        '.env("GIT_EXEC_PATH",' + json.dumps(ssh.exec_path if ssh else real_exec) + ')'
        '.creation_flags(0x08000000).status().expect("launch actual Git"); '
        'exit(status.code().unwrap_or(1)); }\n', encoding='utf8')
    subprocess.run(['rustc','--edition=2024',str(script),'-o',str(tools/'git.exe')],
                   check=True,capture_output=True,**flags)
    return {'PATH':str(tools)+os.pathsep+(ssh.path if ssh else os.environ['PATH'])}, launch_log


def exercise_monitor(work, api, eventually, git, repo, source, root, connection,
                     host, auth_mode, remote, processes, logs, environment,
                     server, flags, origin, launch_log, webhook, notices):
    """Drive only public APIs; SQL changes merely advance an idle test clock."""
    database = work/'server/ferry.sqlite'
    monitor = f'{root}/code-connection-monitors/{connection}'
    history = f'{root}/code-connections/{connection}/events'
    kinds = ['push','branch.created','branch.deleted','tag.created','tag.updated','tag.deleted']
    settings = {'mode':'poll','enabled':True,'all_repositories':False,
        'repositories':source,'events':kinds,'runner_id':host,'poll_interval_minutes':1,'version':0,
        'webhook':webhook}

    def sql(query, args=()):
        with closing(sqlite3.connect(database, timeout=15)) as db, db:
            return db.execute(query,args).fetchall()

    def state(identity=connection):
        rows=sql('SELECT snapshot,last_polled_at,next_poll_at,last_error FROM repository_poll_states WHERE id=?',(identity,))
        return rows[0] if rows else None

    def sample(identity=connection, previous=None):
        if previous is None:
            previous=(state(identity) or ('',0,0,''))[1]
        def due():
            with closing(sqlite3.connect(database, timeout=15)) as db, db:
                held=db.execute('SELECT 1 FROM locks WHERE name=? AND expires_at>?',
                    (f'repository-poll:{identity}',int(time.time()*1000))).fetchone()
                if held:
                    return False
                db.execute('UPDATE repository_poll_states SET next_poll_at=0 WHERE id=?',(identity,))
                return True
        eventually(due,'idle monitor lease',30)
        result=eventually(lambda: (row if (row:=state(identity)) and row[1]>previous else None),
                          'real Runner ref sample',45)
        return result

    def success(identity=connection):
        row=sample(identity)
        assert not row[3], row[3]
        assert row[0].startswith('v1.') and 'refs/heads' not in row[0], 'baseline must be encrypted'
        eventually(lambda: len(notices)==sql('SELECT COUNT(*) FROM repository_events')[0][0],
                   'real localhost webhook delivery',15)
        return row

    def events(identity=connection):
        return api(f'{root}/code-connections/{identity}/events?page_size=100')['items']

    def independent():
        for table in ('repository_analysis_configs','repository_analysis_jobs','pipeline_runs','pipelines','pipeline_artifacts'):
            assert sql(f'SELECT COUNT(*) FROM {table}')[0][0]==0, table
        assert not list((work/'server').rglob('.git')), 'server must not clone repositories'
        assert launch_log.exists(), 'process-launch audit must observe real Runner Git discovery'
        roles=launch_log.read_text().splitlines()
        assert any(row.startswith('runner:ls-remote:') for row in roles) and not any(row.startswith('server:') for row in roles), 'Git must execute on Runner, not server'

    # Configuration must reject invalid and foreign Runner bindings atomically.
    api(monitor,dict(settings,runner_id='999'),method='PATCH',status=(400,403,404))
    other=api('/workspaces',{'name':'Other','slug':'monitor-other'})['id']
    foreign=api(f'/workspaces/{other}/nodes',{'name':'foreign','connection':'runner'},status=201)['id']
    api(monitor,dict(settings,runner_id=foreign),method='PATCH',status=(400,403,404))
    api(f'/workspaces/{other}/code-connection-monitors/{connection}',settings,method='PATCH',status=404)
    anonymous=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    api(monitor,settings,method='PATCH',status=401,client=anonymous)
    assert state() is None
    if auth_mode == 'ssh':
        # SCP syntax can name a root-level repository without any slash.
        credential = api(f'{root}/code-connections/{connection}')['credential_id']
        scp = api(root+'/code-connections', {'provider':'git',
            'name':'SCP root repository', 'credential_id':credential}, status=201)['id']
        configured = api(f'{root}/code-connection-monitors/{scp}',
            dict(settings, enabled=False, repositories='git@git.example.test:project.git'),
            method='PATCH')
        assert configured['repositories'] == 'git@git.example.test:project.git'
        assert state(scp) is None, 'disabled SCP monitor must not schedule remote access'
        print('PASS SCP root repository monitoring configuration', flush=True)
    saved=api(monitor,settings,method='PATCH')
    first=eventually(lambda: (row if (row:=state()) and row[1]>0 else None),'first monitor baseline',45)
    assert not first[3], (first[3], getattr(remote,'counts',None),
        remote.debug_log.read_text(errors='replace') if auth_mode=='ssh' and remote.debug_log.exists() else '',
        launch_log.read_text() if launch_log.exists() else '',
        (work/'monitor-shell.log').read_text(errors='replace') if (work/'monitor-shell.log').exists() else '')
    assert first[0].startswith('v1.') and not events(), 'first baseline emits no historical events'
    assert sql('SELECT config FROM repository_monitors WHERE id=?',(connection,))[0][0].startswith('v1.')
    independent()
    print('PASS independent Runner Git monitor: encrypted first baseline, no analysis/act jobs, invalid bindings denied',flush=True)

    root_sha=git('rev-parse','shared')
    second_sha=git('rev-parse','main')
    (repo/'file.py').write_text('root\nnext\nmonitor\n',encoding='utf8')
    git('commit','-qam','fix: monitor push')
    git('branch','monitor/new')
    git('tag','-a','release','-m','annotated release')
    git('update-server-info')
    success()
    assert {(row['event'],row['reference']) for row in events()}=={
        ('push','main'),('branch.created','monitor/new'),('tag.created','release')}, events()
    assert all(not row['actor'] for row in events()), 'Git refs cannot establish a pusher'
    success()
    assert len(events())==3, 'unchanged sample must not duplicate notifications'
    git('update-ref','refs/heads/main',root_sha)
    git('branch','-D','monitor/new')
    git('tag','-f','release',second_sha)
    git('update-server-info')
    success()
    assert len(events())==6, events()
    assert {(row['event'],row['reference']) for row in events()} >= {
        ('branch.deleted','monitor/new'),('tag.updated','release')}, events()
    git('tag','-d','release')
    git('update-server-info')
    success()
    assert len(events())==7 and any(row['event']=='tag.deleted' for row in events()),events()
    assert len(notices)==7
    print('PASS Runner refs + webhook delivery: push/force-push, branch create/delete, annotated tag create/move/delete, duplicate suppression',flush=True)

    if auth_mode in ('basic','ssh'):
        baseline=state()[0]
        if auth_mode=='ssh':
            remote.enabled=False
        else:
            remote.auth_enabled=False
        git('branch','offline-created',second_sha)
        git('update-server-info')
        failed=sample()
        assert failed[3] and failed[0]==baseline and len(events())==7, 'read failure must preserve baseline, never emit false deletes'
        if auth_mode=='ssh':
            remote.enabled=True
        else:
            remote.auth_enabled=True
        success()
        assert len(events())==8 and any(row['reference']=='offline-created' and row['event']=='branch.created' for row in events())
        print('PASS revoked private read preserves baseline; restored access compares the last successful snapshot',flush=True)

    before=len(events())
    processes[0].terminate()
    processes[0].wait(30)
    processes[0]=subprocess.Popen([server],env=environment,stdout=logs[0],stderr=logs[0],**flags)
    eventually(lambda: api('/status')['ok'],'restarted server',30)
    eventually(lambda: api(f'{root}/nodes/{host}')['state']=='online','Runner reconnect',45)
    success()
    assert len(events())==before,'restart must reuse the encrypted persisted baseline'
    print('PASS server restart reuses persisted baseline without replay',flush=True)
    disabled=api(monitor,dict(settings,version=saved['version'],enabled=False),method='PATCH')
    assert state() is None
    git('branch','disabled-only',second_sha)
    git('update-server-info')
    time.sleep(2.5)
    assert state() is None and len(events())==before
    independent()
    enabled=api(monitor,dict(settings,version=disabled['version']),method='PATCH')
    first=eventually(lambda: (row if (row:=state()) and row[1]>0 else None),'reenabled baseline',45)
    assert not first[3] and len(events())==before
    api(monitor,dict(settings,version=enabled['version'],enabled=False),method='PATCH')
    print('PASS disabled independent monitor suppresses scheduling; reenable establishes baseline',flush=True)

    # Native Git protocol is a separate, real daemon; no HTTP or platform API.
    with socket.socket() as probe:
        probe.bind(('127.0.0.1',0))
        daemon_port=probe.getsockname()[1]
    daemon=subprocess.Popen(['git','daemon','--reuseaddr','--export-all',
        f'--base-path={work}', '--listen=127.0.0.1',f'--port={daemon_port}'],
        stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,**flags)
    processes.append(daemon)
    native_url=f'git://127.0.0.1:{daemon_port}/history'
    def daemon_ready():
        with socket.socket() as probe:
            return probe.connect_ex(('127.0.0.1',daemon_port))==0
    eventually(daemon_ready,'native Git daemon',15)
    readable=[('native Git',native_url)]
    public=os.environ.get('FERRY_TEST_PUBLIC_GIT_URL')
    if public:
        if public == 'local-native':
            readable.append(('local native Git marker',native_url))
        else:
            readable.append(('public OSS native Git' if public.startswith('git://') else 'public OSS HTTPS',public))
    for label,url in readable:
        identity=api(root+'/code-connections',{'provider':'git','name':label},status=201)['id']
        path=f'{root}/code-connection-monitors/{identity}'
        body=dict(settings,repositories=url,version=0)
        selected=api(path,body,method='PATCH')
        row=eventually(lambda: (value if (value:=state(identity)) and value[1]>0 else None),f'{label} baseline',45)
        assert not row[3] and not events(identity), (label,row[3])
        success(identity)
        assert not events(identity),label
        api(path,dict(body,version=selected['version'],enabled=False),method='PATCH')
        independent()
        print(f'PASS independent {label} refs monitor, first baseline and unchanged sample',flush=True)
    for suffix in ('server.log','runner.log'):
        output=(work/suffix).read_text(encoding='utf8',errors='replace')
        assert 'BEGIN RSA PRIVATE KEY' not in output
        if auth_mode=='ssh':
            assert all(line not in output for line in remote.secret.splitlines() if len(line)>30)
        if auth_mode=='basic':
            assert remote.auth_secret not in output
    print('PASS all independent Git monitor gates; no server clone or server-side Git launch',flush=True)
    # Finally enable the separate analytics subscription without an execution
    # pipeline. A changed ref must dirty its durable job, without starting act.
    selected=api(monitor,dict(settings,version=enabled['version']+1),method='PATCH')
    first=eventually(lambda: (row if (row:=state()) and row[1]>0 else None),'statistics subscription baseline',45)
    assert not first[3]
    analysis=f'{root}/code-connections/{connection}/analysis'
    api(analysis,{'version':0,'config':{'enabled':True,'all_repositories':False,
        'repositories':[source],'full_history':True}},method='PATCH')
    source_state=eventually(lambda: (rows[0] if (rows:=api(analysis)['sources']) else None),
                            'separate statistics source discovery',30)
    generation=source_state['job']['generation']
    git('branch','statistics-dirty',second_sha)
    git('update-server-info')
    success()
    assert api(analysis)['sources'][0]['job']['generation']>generation, 'monitor must dirty the separate statistics job'
    assert sql('SELECT COUNT(*) FROM pipeline_runs')[0][0]==0
    api(monitor,dict(settings,version=selected['version'],enabled=False),method='PATCH')
    print('PASS ref monitoring dirties the separate statistics subscription without launching act',flush=True)


class LoopbackSshGit:
    """Key-only SSH upload-pack fixture; serves exactly one disposable repository."""
    def __init__(self, work, repository, flags):
        try:
            import paramiko
        except ImportError:
            local = Path(__file__).resolve().parents[1]/'data/maintenance/ssh-test-deps'
            sys.path.insert(0, str(local))
            import paramiko
        self.enabled = True
        self.counts = {'allowed': 0, 'denied': 0}
        self.transports, self.processes, self.threads = [], [], []
        self.stopped = threading.Event()
        host_key, client_key = paramiko.RSAKey.generate(2048), paramiko.RSAKey.generate(2048)
        secret = io.StringIO()
        client_key.write_private_key(secret)
        self.secret = secret.getvalue()
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.listener.settimeout(.25)
        self.port = self.listener.getsockname()[1]
        self.url = f'ssh://reader@127.0.0.1:{self.port}/history.git'
        known_hosts = work/'known_hosts'
        known_hosts.write_text(f'[127.0.0.1]:{self.port} {host_key.get_name()} {host_key.get_base64()}\n', encoding='ascii')
        wrapper = work/'ssh-bin'
        wrapper.mkdir()
        # Native forwarding wrappers must use native OpenSSH. Re-entering the
        # MSYS SSH runtime through a native child loses Git's transport pipes.
        native_ssh = Path(os.environ['SystemRoot'])/'System32/OpenSSH/ssh.exe'
        actual_ssh = str(native_ssh) if native_ssh.is_file() else shutil.which('ssh')
        assert actual_ssh, 'OpenSSH client required'
        # Isolated equivalent of a Runner provisioned with this host key. Keep
        # the production credential/key flags intact and never touch ~/.ssh.
        native_wrapper = work/'ssh-wrapper.rs'
        self.debug_log = work/'ssh-client.log'
        native_wrapper.write_text('use std::{env,process::{Command,exit}};\n'
            'use std::os::windows::process::CommandExt;\n'
            'fn main() { let result = Command::new(' + json.dumps(Path(actual_ssh).as_posix()) + ')'
            '.args(["-o", "StrictHostKeyChecking=yes", "-o", '
            + json.dumps('UserKnownHostsFile="' + known_hosts.as_posix() + '"') + '])'
            '.args(env::args_os().skip(1)).stderr(std::process::Stdio::from('
            'std::fs::OpenOptions::new().create(true).append(true).open('
            + json.dumps(self.debug_log.as_posix()) + ').unwrap()))'
            '.creation_flags(0x08000000).status()'
            '.expect("launch pinned SSH client"); exit(result.code().unwrap_or(1)); }\n', encoding='utf8')
        subprocess.run(['rustc', '--edition=2024', str(native_wrapper), '-o', str(wrapper/'ssh.exe')],
                       check=True, capture_output=True, **flags)
        self.exec_path = str(wrapper)
        original_exec_path = subprocess.check_output(['git', '--exec-path'], **flags).decode().strip()
        # An explicit checkout PATH overrides act's discovered shell directory.
        # Keep Git for Windows' shell available alongside the pinned SSH shim.
        git_bin = Path(original_exec_path).parents[2]/'bin'
        assert (git_bin/'sh.exe').is_file(), 'Git for Windows shell required'
        self.path = os.pathsep.join((str(wrapper), original_exec_path, str(git_bin), os.environ['PATH']))
        fixture = self

        class Server(paramiko.ServerInterface):
            def __init__(self):
                self.command = threading.Event()

            def get_allowed_auths(self, username):
                return 'publickey'

            def check_auth_publickey(self, username, key):
                accepted = fixture.enabled and username == 'reader' and key == client_key
                fixture.counts['allowed' if accepted else 'denied'] += 1
                return paramiko.AUTH_SUCCESSFUL if accepted else paramiko.AUTH_FAILED

            def check_channel_request(self, kind, channel_id):
                return paramiko.OPEN_SUCCEEDED if kind == 'session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

            def check_channel_exec_request(self, channel, command):
                if shlex.split(command.decode()) != ['git-upload-pack', '/history.git']:
                    return False
                self.command.set()
                return True

        def handle(client):
            transport = paramiko.Transport(client)
            self.transports.append(transport)
            try:
                transport.add_server_key(host_key)
                policy = Server()
                transport.start_server(server=policy)
                channel = transport.accept(15)
                if channel is None or not policy.command.wait(15):
                    return
                process = subprocess.Popen(['git', 'upload-pack', str(repository/'.git')],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **flags)
                self.processes.append(process)

                def incoming():
                    try:
                        while data := channel.recv(65536):
                            process.stdin.write(data)
                            process.stdin.flush()
                    except (OSError, EOFError):
                        pass
                    finally:
                        process.stdin.close()

                def outgoing():
                    try:
                        while data := os.read(process.stdout.fileno(), 65536):
                            channel.sendall(data)
                    except (OSError, EOFError):
                        pass

                reader = threading.Thread(target=incoming, daemon=True)
                writer = threading.Thread(target=outgoing, daemon=True)
                reader.start()
                writer.start()
                result = process.wait(timeout=120)
                writer.join(5)
                channel.send_exit_status(result)
                channel.shutdown_write()
                channel.close()
                reader.join(5)
            except (OSError, EOFError, paramiko.SSHException):
                pass
            finally:
                transport.close()

        def accept():
            while not self.stopped.is_set():
                try:
                    client, _ = self.listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    break
                thread = threading.Thread(target=handle, args=(client,), daemon=True)
                self.threads.append(thread)
                thread.start()

        self.accept_thread = threading.Thread(target=accept, daemon=True)
        self.accept_thread.start()

    def shutdown(self):
        self.stopped.set()
        self.listener.close()
        self.accept_thread.join(5)
        for transport in self.transports:
            transport.close()
        for process in self.processes:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
            for pipe in (process.stdin, process.stdout, process.stderr):
                if not pipe.closed:
                    pipe.close()
        for thread in self.threads:
            thread.join(5)

    def server_close(self):
        pass


def main():
    assert os.name == 'nt', 'Windows integration; portable collector has a separate test'
    server, runner, workers = [str(Path(p).resolve()) for p in sys.argv[1:4]]
    collector = str(Path(__file__).with_name('collect-contributions.py').resolve())
    flags = {'creationflags': subprocess.CREATE_NO_WINDOW}
    monitor_mode = os.environ.get('FERRY_TEST_GIT_MONITOR') == '1'
    insight_mode = os.environ.get('FERRY_TEST_CODE_INSIGHTS') == '1'
    with tempfile.TemporaryDirectory(prefix='contribution-actions-') as directory:
        work = Path(directory)
        repo = work / 'history'
        repo.mkdir()

        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.STDOUT, **flags).decode().strip()

        git('init', '-q', '-b', 'main')
        git('config', 'user.name', 'Author')
        git('config', 'user.email', 'author@example.test')
        (repo / 'file.py').write_text('root\n', encoding='utf8')
        git('add', '.')
        git('commit', '-qm', 'feat: root')
        git('branch', 'shared')
        (repo / 'file.py').write_text('root\nnext\n', encoding='utf8')
        git('commit', '-qam', 'feat: second')
        git('update-server-info')
        auth_mode = os.environ.get('FERRY_TEST_GIT_AUTH', '')
        assert auth_mode in ('', 'basic', 'ssh'), 'FERRY_TEST_GIT_AUTH must be basic or ssh'
        private = auth_mode in ('basic', 'ssh')
        password = secrets.token_hex(20)
        auth_counts = {'allowed': 0, 'denied': 0}
        class QuietFiles(SimpleHTTPRequestHandler):
            def do_GET(self):
                expected = 'Basic ' + base64.b64encode(('reader:' + password).encode()).decode()
                if private and (not remote.auth_enabled or self.headers.get('Authorization') != expected):
                    auth_counts['denied'] += 1
                    self.send_response(401)
                    self.send_header('WWW-Authenticate', 'Basic realm="Git fixture"')
                    self.end_headers()
                    return
                auth_counts['allowed'] += 1
                super().do_GET()

            def log_message(self, *_):
                pass
        if auth_mode == 'ssh':
            remote = LoopbackSshGit(work, repo, flags)
            source, auth_counts = remote.url, remote.counts
        else:
            remote=ThreadingHTTPServer(('127.0.0.1',0), partial(QuietFiles,directory=directory))
            remote.auth_enabled = True
            remote.auth_secret = password
            threading.Thread(target=remote.serve_forever,daemon=True).start()
            source=f'http://127.0.0.1:{remote.server_port}/history/.git'
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            port = probe.getsockname()[1]
        origin = f'http://127.0.0.1:{port}'
        environment = dict(os.environ, FERRY_DATA_DIR=str(work / 'server'),
                           FERRY_BIND=f'127.0.0.1:{port}', ACTIONS_WORKER_DIR=workers)
        if auth_mode == 'ssh':
            environment['PATH'] = remote.path
        monitor_environment, launch_log = ({}, None)
        notifications, notices = None, []
        if monitor_mode:
            monitor_environment, launch_log = monitor_git_environment(work, flags,
                remote if auth_mode == 'ssh' else None)
            environment.update(monitor_environment, FERRY_TEST_GIT_ORIGIN='server')
            class Notices(SimpleHTTPRequestHandler):
                def do_POST(self):
                    notices.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                    self.send_response(200)
                    self.end_headers()
                def log_message(self, *_):
                    pass
            notifications=ThreadingHTTPServer(('127.0.0.1',0),Notices)
            threading.Thread(target=notifications.serve_forever,daemon=True).start()
        environment.pop('FERRY_DATABASE_URL', None)
        logs = [open(work / 'server.log', 'wb'), open(work / 'runner.log', 'wb')]
        processes = [subprocess.Popen([server], env=environment, stdout=logs[0], stderr=logs[0], **flags)]
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                             urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

        def api(path, body=None, method=None, status=200, client=None):
            request = urllib.request.Request(origin + '/api' + path,
                data=json.dumps(body).encode() if body is not None else None, method=method,
                headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN'})
            try:
                response = (client or opener).open(request, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            value = json.loads(response.read() or b'null')
            assert response.code in (status if isinstance(status,tuple) else (status,)), (path, response.code, value)
            return value

        def eventually(probe, label, seconds=180):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                try:
                    result = probe()
                    if result:
                        return result
                except OSError:
                    pass
                time.sleep(.25)
            raise AssertionError('Timed out: ' + label)

        try:
            eventually(lambda: api('/status')['ok'], 'server startup', 30)
            api('/setup', {'name': 'Analyst', 'email': 'analyst@example.test',
                'password': 'temporary-password', 'token': (work/'server'/'setup-token').read_text().strip()})
            workspace = api('/workspaces', {'name': 'Analytics', 'slug': 'analytics'})['id']
            root = f'/workspaces/{workspace}'
            host = api(root + '/nodes', {'name': 'collector-host', 'connection': 'runner'}, status=201)['id']
            token = api(root + '/runner-tokens', {'host_id': host}, status=201)['token']
            runner_environment = dict(os.environ,
                **({'PATH':remote.path} if auth_mode == 'ssh' else {}))
            runner_temp=work/'Runner temp with spaces'
            runner_temp.mkdir()
            runner_environment.update(TEMP=str(runner_temp), TMP=str(runner_temp))
            if monitor_mode:
                runner_environment.update(monitor_environment, FERRY_TEST_GIT_ORIGIN='runner')
                if auth_mode == 'ssh':
                    # Scope host trust to this disposable Runner process.
                    runner_environment['GIT_EXEC_PATH'] = remote.exec_path
            processes.append(subprocess.Popen([runner, '--server', origin, '--token', token,
                '--state-dir', str(work/'runner')], env=runner_environment,
                stdout=logs[1], stderr=logs[1], **flags))
            eventually(lambda: api(f'{root}/nodes/{host}')['state']=='online', 'runner online', 60)
            connection_body = {'provider': 'git', 'name': 'Git history'}
            if private:
                credential = api(root + '/credentials', {'name': 'Private Git reader',
                    'kind': 'ssh-key' if auth_mode == 'ssh' else 'password', 'username': 'reader',
                    'secret': remote.secret if auth_mode == 'ssh' else password}, status=201)['id']
                connection_body['credential_id'] = credential
            connection = api(root + '/code-connections', connection_body, status=201)['id']
            if os.environ.get('FERRY_TEST_CODE_REVIEWS') == '1':
                import runpy
                def create_review_resource(resource, body):
                    return api(root + '/' + resource, body, status=201)['id']
                review_checks = []
                def review_check(label, condition, detail=''):
                    assert condition, (label, detail)
                    review_checks.append(label)
                    print('PASS ' + label, flush=True)
                context = dict(api=api, create=create_review_resource, check=review_check,
                    workspace_ops=workspace, runner=host, cred=connection_body.get('credential_id',''),
                    repo_url=source, commit=git('rev-parse','HEAD'), all_runs=[], windows=True,
                    repository_file='file.py', repository_content='root\nnext')
                runpy.run_path(str(Path(__file__).with_name('test-code-reviews-execution.py')))['accept'](context)
                print(f'PASS: {len(review_checks)} actual Windows code review checks', flush=True)
                return
            if monitor_mode:
                exercise_monitor(work, api, eventually, git, repo, source, root, connection,
                    host, auth_mode, remote, processes, logs, environment, server, flags, origin, launch_log,
                    f'http://127.0.0.1:{notifications.server_port}/notify', notices)
                return
            specs = {name: {'type': 'string', 'required': True} for name in [
                'analysis_task_id', 'analysis_generation', 'analysis_repository',
                'analysis_coverage_start', 'analysis_known_complete']}
            # Run the real portable collector, using a local fixture as the execution
            # host's already fetched cache. No fixture repository code is executed.
            command = ('[IO.File]::WriteAllText((Join-Path $PWD "known.json"), $env:KNOWN, '
                '(New-Object Text.UTF8Encoding $false))\n'
                f'& "{sys.executable}" "{collector}" --repository source '
                '--task-id "$env:TASK" --generation "$env:GEN" --repository-id "$env:SOURCE" '
                '--coverage-start "$env:START" --known-complete known.json --output contribution-analysis.json\n'
                'if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }')
            workflow = {'on': {'workflow_dispatch': {'inputs': specs}}, 'jobs': {
                'collect': {'runs-on': 'self-hosted', 'steps': [
                    {'id':'source', 'uses':'actions/checkout@v4', 'with':{'path':'source'}},
                    {'run': command, 'env': {key: '${{ inputs.' + value + ' }}' for key, value in {
                        'TASK':'analysis_task_id', 'GEN':'analysis_generation', 'SOURCE':'analysis_repository',
                        'START':'analysis_coverage_start', 'KNOWN':'analysis_known_complete'}.items()}},
                    # Host jobs use the worker's built-in v4 helper and real
                    # authenticated artifact runtime, without a Node/action fetch.
                    {'uses': 'actions/upload-artifact@v4',
                     'with': {'name': 'contribution-analysis', 'path': 'contribution-analysis.json',
                              'if-no-files-found': 'error', 'compression-level': 6}},
                ]}}}
            if auth_mode == 'ssh':
                # Git prepends exec-path when spawning SSH. Restrict this shim
                # to checkout; server/Runner Git installation discovery stays native.
                workflow['jobs']['collect']['steps'][0]['env'] = {
                    'GIT_EXEC_PATH':remote.exec_path, 'PATH':remote.path}
            pipeline = api(root + '/pipelines', {'name': 'Git collector', 'definition_format': 'github-actions',
                'definition': json.dumps(workflow), 'checkout_bindings': [
                    {'job_id':'collect','step_id':'source','connection_id':connection,
                     'repository_url':source,'path':'source'}], 'executor_bindings': [
                    {'job_id':'collect', 'target_kind':'host', 'target_ref':host, 'image':'host'}]}, status=201)['id']
            def configure_analysis(url, name):
                if insight_mode:
                    insight = api(root + '/code-insights', {'name': name, 'enabled': True,
                        'full_history': True, 'initial_days': 90, 'reconcile_minutes': 1440,
                        'repositories': [{'connection_id': connection, 'repository': url,
                                          'pipeline_id': pipeline}]}, status=201)['id']
                    path = f'{root}/code-insights/{insight}'
                    detail = api(path)
                    assert detail['repositories'][0]['connection_id'] == connection, detail
                    assert detail['repositories'][0]['repository'] == url, detail
                    assert detail['repositories'][0]['pipeline_id'] == pipeline, detail
                    assert detail['update_mode'] == 'scheduled', detail
                    assert detail['schedule'] == '0 */4 * * *', detail
                    assert detail['next_run_at'] > int(time.time() * 1000), detail
                    return path, path + '/sync'
                path = f'{root}/code-connections/{connection}/analysis'
                api(path, {'version':0, 'config': {'enabled': True, 'all_repositories': False,
                    'repositories':[url], 'full_history': True, 'pipeline_id':pipeline}}, method='PATCH')
                return path, f'{root}/code-connections/{connection}/analysis-runs'

            config_path, retry_path = configure_analysis(source, 'Private Git insight' if private else 'Git insight')

            def report_url(kind='commits'):
                if insight_mode:
                    return f'{config_path}/reports/{kind}?isMerge=0,1'
                return f'{root}/code-contributions/{kind}?connection_id={connection}&isMerge=0,1'

            def verify_insight_metrics(expected):
                if not insight_mode:
                    return
                # This linear fixture appends one line per non-root commit.
                # Existing collector semantics assign zero line totals to roots.
                changed_lines = expected - 1
                for kind in ('overview', 'contributors', 'ranking', 'period', 'heatmap'):
                    report = api(report_url(kind))
                    values = report.get('series') if kind in ('period', 'heatmap') else report['items']
                    assert sum(row['commits'] for row in values) == expected, (kind, report)
                    assert sum(row['additions'] for row in values) == changed_lines, (kind, report)
                    assert sum(row['deletions'] for row in values) == 0, (kind, report)
                    assert sum(row['effectives'] for row in values) == changed_lines, (kind, report)
                    assert report['coverage']['complete'], (kind, report)
                branches = api(report_url('branches'))
                assert {row['branchName']: row['commits'] for row in branches['items']} == {
                    'main': expected, 'shared': 1}, branches

            def synced(expected):
                row = api(config_path)
                sources = row['sources']
                if sources and sources[0]['state']=='failed' and sources[0]['job']['state']=='failed':
                    runs = api(root + '/pipeline-runs')['items']
                    output = [api(f'{root}/pipeline-runs/{r["id"]}/logs?after=0') for r in runs]
                    raise AssertionError((sources, output))
                if (not sources or sources[0]['state']!='complete'
                    or sources[0]['job']['state']!='complete'
                    or sources[0]['job']['generation']!=sources[0]['job']['completed_generation']):
                    return False
                report = api(report_url())
                return report if report['total']==expected else False

            def verify_zip(run_count, fact_count, reachable_count, expected_refs=None):
                artifacts = api(root + '/pipeline-artifacts')['items']
                assert len(artifacts) == run_count, artifacts
                artifact = artifacts[0]
                assert artifact['path'].endswith('.zip'), artifact
                request = urllib.request.Request(origin + '/api' + root + '/pipeline-artifacts/' + artifact['id'] + '/content',
                                                 headers={'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN'})
                with opener.open(request, timeout=30) as response:
                    payload = response.read()
                assert len(payload) == artifact['size']
                assert hashlib.sha256(payload).hexdigest() == artifact['sha256']
                with zipfile.ZipFile(io.BytesIO(payload)) as archive:
                    assert archive.namelist() == ['contribution-analysis.json'], archive.namelist()
                    manifest = json.loads(archive.read('contribution-analysis.json'))
                assert len(manifest['commits']) == fact_count, manifest
                assert set(manifest['refs']) == set(expected_refs or ['main', 'shared']), manifest
                if expected_refs:
                    assert manifest['refs'] == expected_refs, manifest['refs']
                assert len(set().union(*map(set, manifest['ref_commits'].values()))) == reachable_count
                assert manifest['generation'] == api(config_path)['sources'][0]['job']['completed_generation']
                assert api(root + '/pipeline-runs/' + artifact['run_id'])['state'] == 'succeeded'

            first = eventually(lambda: synced(2), 'initial act artifact import')
            assert len({r['commitHash'] for r in first['items']}) == 2
            verify_zip(1, 2, 2)
            verify_insight_metrics(2)
            print('PASS real act ZIP artifact imported across both branches', flush=True)
            (repo / 'file.py').write_text('root\nnext\nthird\n', encoding='utf8')
            git('commit', '-qam', 'fix: third')
            git('update-server-info')
            if insight_mode:
                # Advance only this disposable insight's persisted clock. The
                # background scheduler must enqueue the real collector itself.
                with closing(sqlite3.connect(work/'server/ferry.sqlite', timeout=15)) as db, db:
                    changed = db.execute('UPDATE code_insights SET next_run_at=1 WHERE id=?',
                                         (config_path.rsplit('/', 1)[1],))
                    assert changed.rowcount == 1
            else:
                api(retry_path, {}, method='POST')
            second = eventually(lambda: synced(3), 'incremental act artifact import')
            assert second['total']==3
            verify_zip(2, 1, 3)
            verify_insight_metrics(3)
            if insight_mode:
                scheduled = api(config_path)
                assert scheduled['next_run_at'] > int(time.time() * 1000), scheduled
                print('PASS due schedule consumed by background worker through actual act ZIP; next run advanced', flush=True)
            print('PASS incremental act import reuses known complete SHAs without duplicate facts', flush=True)
            api(retry_path, {}, method='POST')
            eventually(lambda: api(config_path)['sources'][0]['job']['state']=='complete', 'idempotent retry')
            assert synced(3)
            verify_zip(3, 0, 3)
            verify_insight_metrics(3)
            print('PASS repeated act run leaves report unchanged', flush=True)
            if private:
                assert auth_counts['allowed'] > 0, auth_counts
                saved_password = password
                if auth_mode == 'ssh':
                    remote.enabled = False
                else:
                    password = secrets.token_hex(20)
                api(retry_path, {}, method='POST')
                eventually(lambda: api(config_path)['sources'][0]['job']['state']=='failed',
                           'revoked private Git access fails collection')
                assert api(report_url())['total'] == 3
                assert len(api(root + '/pipeline-artifacts')['items']) == 3
                assert auth_counts['denied'] > 0, auth_counts
                if auth_mode == 'ssh':
                    remote.enabled = True
                else:
                    password = saved_password
                api(retry_path, {}, method='POST')
                eventually(lambda: synced(3), 'restored private Git read access')
                verify_zip(4, 0, 3)
                verify_insight_metrics(3)
                for suffix in ('server.log', 'runner.log'):
                    output = (work/suffix).read_text(encoding='utf8', errors='replace')
                    assert password not in output
                    if auth_mode == 'ssh':
                        assert remote.secret not in output and 'BEGIN RSA PRIVATE KEY' not in output
                print(f'PASS private {"SSH key" if auth_mode == "ssh" else "HTTP"} Git credentials, denial, unchanged facts and restored access', flush=True)

            public_url = os.environ.get('FERRY_TEST_PUBLIC_GIT_URL')
            if public_url:
                if public_url == 'local-native':
                    with socket.socket() as probe:
                        probe.bind(('127.0.0.1', 0))
                        git_port = probe.getsockname()[1]
                    daemon_log = open(work/'git-daemon.log', 'wb')
                    logs.append(daemon_log)
                    daemon = subprocess.Popen(['git', 'daemon', '--export-all', '--reuseaddr',
                        '--listen=127.0.0.1', f'--port={git_port}',
                        f'--base-path={work.as_posix()}', '--timeout=60', '--init-timeout=15',
                        '--verbose', repo.as_posix()], stdout=daemon_log, stderr=daemon_log, **flags)
                    processes.append(daemon)

                    def native_ready():
                        assert daemon.poll() is None, 'Local Git daemon exited before readiness'
                        with socket.create_connection(('127.0.0.1', git_port), timeout=1):
                            return True

                    eventually(native_ready, 'local native Git daemon startup', 15)
                    public_url = f'git://127.0.0.1:{git_port}/history'
                # This mirror is an independent test oracle, never a service-side cache.
                reference = work/'public-oracle.git'
                subprocess.run(['git', 'clone', '--mirror', public_url, str(reference)],
                               check=True, capture_output=True, timeout=90, **flags)
                rows = subprocess.check_output(['git', '-C', str(reference), 'for-each-ref',
                    '--format=%(refname:strip=2) %(objectname)', 'refs/heads/'], **flags).decode()
                refs = dict(row.split() for row in rows.splitlines())
                expected = int(subprocess.check_output(['git', '-C', str(reference), 'rev-list',
                    '--count', *refs.values()], **flags).decode())
                # Use an anonymous connection even when the preceding fixture was private.
                connection = api(root + '/code-connections', {'provider':'git', 'name':'Public OSS Git'}, status=201)['id']
                pipeline = api(root + '/pipelines', {'name':'Public OSS collector',
                    'definition_format':'github-actions', 'definition':json.dumps(workflow),
                    'checkout_bindings':[{'job_id':'collect','step_id':'source',
                        'connection_id':connection,'repository_url':public_url,'path':'source'}],
                    'executor_bindings':[{'job_id':'collect','target_kind':'host',
                        'target_ref':host,'image':'host'}]}, status=201)['id']
                config_path, retry_path = configure_analysis(public_url, 'Native Git insight' if public_url.startswith('git://') else 'Public Git insight')
                eventually(lambda: synced(expected), 'public OSS Git full-history act import')
                run_count = 5 if private else 4
                verify_zip(run_count, expected, expected, refs)
                api(retry_path, {}, method='POST')
                eventually(lambda: synced(expected) and len(api(root+'/pipeline-artifacts')['items'])==run_count+1,
                           'public OSS Git unchanged incremental run')
                verify_zip(run_count+1, 0, expected, refs)
                label = 'local native Git' if os.environ.get('FERRY_TEST_PUBLIC_GIT_URL') == 'local-native' else f'public OSS {public_url}'
                print(f'PASS {label}: {len(refs)} branches, {expected} unique commits; repeat 0 new facts', flush=True)
        except BaseException:
            for log in logs:
                log.flush()
            if os.environ.get('FERRY_TEST_LOG_DIR'):
                retained = Path(os.environ['FERRY_TEST_LOG_DIR'])
                retained.mkdir(parents=True, exist_ok=True)
                for log in work.glob('*.log'):
                    shutil.copy2(log, retained/log.name)
            print((work/'server.log').read_text(encoding='utf8', errors='replace')[-6000:], file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    subprocess.run(['taskkill','/PID',str(process.pid),'/T','/F'],
                        stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,**flags)
                try:
                    process.wait(15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            for log in logs:
                log.close()
            remote.shutdown()
            remote.server_close()
            if notifications:
                notifications.shutdown()
                notifications.server_close()


if __name__ == '__main__':
    main()
