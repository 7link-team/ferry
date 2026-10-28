#!/usr/bin/env python3
"""Native service acceptance. Uses a uniquely named service; preserves no test services.
macOS uses a per-user LaunchAgent. Linux and Windows require service-install privileges.
Run after cargo build -p ferry-server -p ferry-runner.
"""
import json, os, platform, re, socket, subprocess, tempfile, time, urllib.request, http.cookiejar
from pathlib import Path

runner = Path(os.environ.get('RUNNER_BINARY', 'target/debug/ferry-runner' + ('.exe' if os.name == 'nt' else ''))).resolve()
server_binary = Path(os.environ.get('FERRY_BINARY', 'target/debug/ferry-server' + ('.exe' if os.name == 'nt' else ''))).resolve()
system = platform.system()
name = 'runner-test-' + str(os.getpid())
user = ['--user'] if system == 'Darwin' and not os.environ.get('RUNNER_TEST_SYSTEM_SERVICE') else []

def command(*args, check=True):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True)
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

with tempfile.TemporaryDirectory(prefix='runner-service-') as tmp:
    root = Path(tmp)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env = dict(os.environ, FERRY_DATA_DIR=str(root/'server'), FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    log = (root/'server.log').open('w')
    server = subprocess.Popen([str(server_binary)], env=env, stdout=log, stderr=log)
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
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
        service('install', '--server', origin, '--token', token, '--state-dir', state)
        host_path = '/workspaces/service/hosts/'+host
        wait(lambda: api(host_path)['state']=='online', 'service should enrol automatically')
        original = (state/'runner.json').read_bytes()
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
    finally:
        service('uninstall', check=False)
        server.terminate()
        server.wait(timeout=30)
        log.close()
