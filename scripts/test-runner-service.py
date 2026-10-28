#!/usr/bin/env python3
"""Native service acceptance. Uses a uniquely named service; preserves no test services.
macOS uses a per-user LaunchAgent. Linux and Windows require service-install privileges.
Run after cargo build -p ferry-server -p ferry-runner.
"""
import json, os, platform, re, socket, subprocess, sys, tempfile, time, urllib.request, http.cookiejar

sys.stdout.reconfigure(encoding="utf-8", errors="backslashreplace")
sys.stderr.reconfigure(encoding="utf-8", errors="backslashreplace")
from pathlib import Path

runner = Path(os.environ.get('RUNNER_BINARY', 'target/debug/ferry-runner' + ('.exe' if os.name == 'nt' else ''))).resolve()
server_binary = Path(os.environ.get('FERRY_BINARY', 'target/debug/ferry-server' + ('.exe' if os.name == 'nt' else ''))).resolve()
system = platform.system()
name = 'runner-test-' + str(os.getpid())
user = ['--user'] if system == 'Darwin' and not os.environ.get('RUNNER_TEST_SYSTEM_SERVICE') else []

def command(*args, check=True):
    # Rust emits UTF-8; native service tools can emit a legacy code page.
    # Preserve undecodable bytes in diagnostics instead of losing stderr.
    result = subprocess.run([str(a) for a in args], capture_output=True,
                            text=True, encoding='utf-8', errors='backslashreplace')
    if check: assert result.returncode == 0, (args, result.stdout, result.stderr)
    return result

def service(action, *args, check=True):
    return command(runner, 'service', action, '--name', name, *user, *args, check=check)

def wait(test, label, seconds=35):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = test()
        if value: return value
        time.sleep(.15)
    raise AssertionError(label)

def pid():
    if system == 'Darwin':
        text = command('launchctl', 'print', f'gui/{os.getuid()}/{name}' if user else f'system/{name}', check=False).stdout
        found = re.search(r'\bpid = (\d+)', text)
    elif system == 'Linux':
        text = command('systemctl', 'show', name, '--property=MainPID', check=False).stdout
        found = re.search(r'MainPID=(\d+)', text)
    else:
        text = command('sc.exe', 'queryex', name, check=False).stdout
        found = re.search(r'PID\s*:\s*(\d+)', text)
    return int(found[1]) if found else 0

# Hosted Windows runners grant other accounts write access to their Temp tree.
# A privileged service must live under an administrator-controlled parent.
with tempfile.TemporaryDirectory(prefix='runner-service-',
                                 dir=os.environ['ProgramFiles'] if system == 'Windows' else None) as tmp:
    root = Path(tmp)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env = dict(os.environ, FERRY_DATA_DIR=str(root/'server'), FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    log = (root/'server.log').open('w')
    server = subprocess.Popen([str(server_binary)], env=env, stdout=log, stderr=log)
    jar = http.cookiejar.CookieJar()
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(jar))
    def api(path, body=None):
        request = urllib.request.Request(origin+'/api'+path, data=json.dumps(body).encode() if body is not None else None, headers={'Content-Type':'application/json','X-Ferry-Request':'1'})
        with client.open(request, timeout=10) as response: return json.load(response)
    try:
        def ready():
            try: return api('/status')['ok']
            except OSError: return False
        wait(ready, 'server startup')
        api('/setup', {'name':'Service Test','email':'service@example.invalid','password':'temporary-service-password','token':(root/'server/setup-token').read_text().strip()})
        api('/workspaces', {'name':'Service','slug':'service'})
        host = api('/workspaces/service/hosts', {'name':'Managed runner','connection':'runner'})['id']
        token = api('/workspaces/service/runner-tokens', {'host_id':host})['token']
        state = root/'state with spaces'
        if system == 'Windows':
            unsafe = root/'unsafe-state'
            unsafe.mkdir()
            command('icacls.exe', unsafe, '/grant', '*S-1-5-32-545:(OI)(CI)M')
            rejected = service('install', '--server', origin, '--token', token,
                               '--state-dir', unsafe, check=False)
            assert rejected.returncode != 0, 'must reject user-writable service state'
            assert not (unsafe/'service.json').exists(), 'unsafe install must not persist credentials'
        service('install', '--server', origin, '--token', token, '--state-dir', state)
        host_path = '/workspaces/service/hosts/'+host
        wait(lambda: api(host_path)['state']=='online', 'service should enrol automatically')
        update_path = host_path+'/runner-update'
        wait(lambda: api(update_path)['ready'], 'supervisor must confirm authenticated worker readiness')
        assert api(update_path)['supported'] is True
        invalid_update = api(update_path, {'version':'9.9.9','manifest':'{}','signature':'invalid'})
        assert invalid_update['state'] == 'draining'
        wait(lambda: api(update_path)['update']['state']=='failed', 'invalid signature must reject update')
        assert api(host_path)['state']=='online', 'invalid update must preserve current runner'
        print('PASS authenticated readiness and rejected update preserve live service', flush=True)
        original = (state/'runner.json').read_bytes()
        original_version = api(update_path)['agent_version']
        if os.environ.get('RUNNER_UPGRADE_FIXTURES'):
            fixtures = Path(os.environ['RUNNER_UPGRADE_FIXTURES'])
            def upgrade(version):
                body = {'version':version, 'manifest':(fixtures/(version+'.json')).read_text(), 'signature':(fixtures/(version+'.json.sig')).read_text().strip()}
                result = api(update_path, body)
                assert result['state']=='draining'
                return result
            import runpy
            ws_module = runpy.run_path(str(Path(__file__).with_name('ws-probe.py')))
            terminal = ws_module['Ws'](origin.replace('http://','ws://')+'/api'+host_path+'/terminal', '; '.join(f'{cookie.name}={cookie.value}' for cookie in jar))
            def terminal_output(seconds):
                output = bytearray()
                for opcode, data in terminal.frames(seconds):
                    if opcode == ws_module['BINARY']:
                        output.extend(data)
                        # ConPTY asks its terminal emulator for the cursor position.
                        # Unlike xterm.js, this raw WebSocket client must reply itself.
                        if b'\x1b[6n' in output:
                            terminal.send(ws_module['BINARY'], b'\x1b[1;1R')
                    elif opcode == ws_module['TEXT']:
                        print('Terminal control:', repr(data), flush=True)
                return bytes(output)
            initial_output = terminal_output(2)
            print('Terminal initial output:', repr(initial_output), flush=True)
            upgrade('9.9.9')
            time.sleep(.5)
            assert api(update_path)['update']['state']=='draining', 'upgrade must wait for existing terminal'
            terminal.send(ws_module['BINARY'], b'echo DRAIN-CONTINUES\r\n')
            output = terminal_output(3)
            print('Terminal drain output:', repr(output), flush=True)
            assert b'DRAIN-CONTINUES' in output, 'active terminal must remain usable during drain'
            assert api(update_path)['update']['state']=='draining'
            terminal.close()
            print('PASS upgrade waits for active terminal without interrupting output', flush=True)
            wait(lambda: api(update_path)['update']['state']=='succeeded', 'signed version upgrade', seconds=120)
            assert api(update_path)['agent_version']=='9.9.9'
            journal = json.loads((state/'versions.json').read_text())
            assert journal['current']['version']=='9.9.9' and journal['pending'] is None
            assert (state/'runner.json').read_bytes()==original
            print('PASS signed update downloads over TLS and confirms real new-version handshake', flush=True)
            for version in ['9.9.10', '9.9.11']:
                upgrade(version)
                wait(lambda: api(update_path)['update']['state']=='rolled_back', 'failed candidate must roll back', seconds=120)
                assert api(update_path)['agent_version']=='9.9.9'
                journal = json.loads((state/'versions.json').read_text())
                assert journal['current']['version']=='9.9.9' and journal['failed_version']==version
                assert (state/'runner.json').read_bytes()==original
            print('PASS invalid executable and wrong-version health check restore confirmed version', flush=True)
            upgrade('9.9.12')
            wait(lambda: json.loads((state/'versions.json').read_text()).get('pending') is not None, 'pending activation journal', seconds=120)
            interrupted = pid()
            if system=='Windows': command('taskkill','/PID',str(interrupted),'/F')
            else: os.kill(interrupted, 9)
            wait(lambda: pid() not in (0, interrupted), 'native manager restarts interrupted supervisor')
            wait(lambda: api(update_path)['update']['state']=='rolled_back', 'interrupted switch must recover old version', seconds=120)
            journal = json.loads((state/'versions.json').read_text())
            assert journal['current']['version']=='9.9.9' and journal['pending'] is None
            assert journal['failed_version']=='9.9.12'
            assert (state/'runner.json').read_bytes()==original
            print('PASS supervisor crash during unconfirmed activation rolls back after native restart', flush=True)
            # Fault injection after stopping the actual service: a confirmed
            # installation can be damaged between boots, not only while pending.
            saved_journal = (state/'versions.json').read_bytes()
            installed = state/'versions'/journal['current']['digest']/('runner.exe' if system=='Windows' else 'runner')
            service('stop')
            wait(lambda: pid()==0, 'stop before damaging confirmed version')
            saved_binary = installed.read_bytes()
            saved_mode = installed.stat().st_mode
            for fault in ['missing', 'corrupt'] + (['nonexecutable'] if system!='Windows' else []):
                (state/'versions.json').write_bytes(saved_journal)
                installed.write_bytes(saved_binary)
                installed.chmod(saved_mode)
                if fault=='missing': installed.unlink()
                elif fault=='corrupt': installed.write_bytes(b'corrupt confirmed payload')
                else: installed.chmod(0o600)
                service('start')
                wait(lambda: api(update_path)['ready'] and api(update_path)['agent_version']==original_version,
                     'damaged confirmed version must recover bootstrap: '+fault, seconds=60)
                recovered = json.loads((state/'versions.json').read_text())
                assert recovered['current'] is None and recovered['pending'] is None
                assert recovered['failed_version']=='9.9.9'
                assert (state/'runner.json').read_bytes()==original
                service('stop')
                wait(lambda: pid()==0, 'stop recovered bootstrap')
            installed.write_bytes(saved_binary)
            installed.chmod(saved_mode)
            service('start')
            wait(lambda: api(update_path)['ready'], 'bootstrap restarts after fault tests')
            print('PASS missing/corrupt/unlaunchable confirmed version recovers bootstrap identity', flush=True)

        first = wait(pid, 'service PID')
        # Reinstall must not restart a running agent or change its identity.
        service('install', '--server', origin, '--token', 'unused-token', '--state-dir', state)
        assert pid()==first, 'reinstall interrupted running service'
        assert (state/'runner.json').read_bytes()==original
        collision = service('install', '--server', origin, '--token', 'unused-token', '--state-dir', root/'other-state', check=False)
        assert collision.returncode != 0, 'same service name must not be redirected to a new state directory'
        assert pid()==first, 'rejected install must leave current service running'
        print('PASS native service starts, enrols and safely reinstalls', flush=True)
        if system=='Windows': command('taskkill','/PID',str(first),'/F')
        else: os.kill(first, 9)
        wait(lambda: pid() not in (0,first), 'service manager should restart crashed agent')
        wait(lambda: api(host_path)['state']=='online', 'restarted agent should reconnect')
        assert (state/'runner.json').read_bytes()==original
        print('PASS crash recovery retains host identity', flush=True)
        service('stop')
        wait(lambda: pid()==0, 'service stops')
        time.sleep(6)
        assert pid()==0, 'intentional stop must not auto-restart'
        service('start')
        wait(pid, 'service starts again after stop')
        wait(lambda: api(host_path)['state']=='online', 'restarted service reconnects')
        assert (state/'runner.json').read_bytes()==original
        print('PASS explicit stop/start preserves identity', flush=True)
        service('uninstall')
        wait(lambda: pid()==0, 'uninstalled service should finish stopping')
        assert (state/'runner.json').read_bytes()==original, 'uninstall must preserve identity data'
        print('PASS uninstall removes only service registration', flush=True)
    except Exception:
        try:
            if 'update_path' in locals(): print('Upgrade status:', json.dumps(api(update_path)), flush=True)
            for filename in ['versions.json', 'service.log']:
                path = state/filename
                if path.exists(): print(filename, path.read_text(encoding='utf-8', errors='replace')[-12000:], flush=True)
            if system == 'Linux': print(command('journalctl', '-u', name, '--no-pager', '-n', '60', check=False).stdout, flush=True)
        except Exception as diagnostic: print('Diagnostic error:', type(diagnostic).__name__, flush=True)
        raise
    finally:
        if 'terminal' in locals(): terminal.close()
        service('uninstall', check=False)
        wait(lambda: pid()==0, 'test service cleanup stops supervisor')
        server.terminate()
        server.wait(timeout=30)
        log.close()
