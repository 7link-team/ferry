#!/usr/bin/env python3
"""Privileged CI acceptance of real shell/PowerShell bootstrap and native services.
Uses existing target/debug binaries; never builds. Metadata is fixture-controlled:
server-side signed-catalog verification is covered by separate API tests.
"""
import hashlib
import http.cookiejar
import http.server
import json
import os
from pathlib import Path
import platform
import re
import selectors
import shutil
import signal
import socket
import socketserver
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import uuid

def token_allows_metadata(authorization, enrollment, status_fetch):
    """Keep the fixture's metadata eligibility identical to production tokens."""
    if authorization != ('Bearer ' + enrollment['token']).encode(): return False
    status = status_fetch(enrollment['id'])
    return status['state'] == 'pending' and status['used_at'] == 0 and status['expires_at'] > int(time.time() * 1000)


# Lightweight executable regression for this fixture's authentication boundary.
# Native runs below additionally query the real backend token status endpoint.
if sys.argv[1:] == ['--check-token-gate']:
    enrollment = {'id': 'fixture-token-id', 'token': 'fixture-secret'}
    pending = {'state': 'pending', 'used_at': 0, 'expires_at': int(time.time() * 1000) + 60000}
    for label, status, expected in [
        ('pending', pending, True),
        ('used', dict(pending, state='connected', used_at=1), False),
        ('expired', dict(pending, state='expired', expires_at=1), False),
        ('used-without-host', dict(pending, used_at=1), False),
    ]:
        def status_fetch(token_id):
            assert token_id == enrollment['id']
            return status
        assert token_allows_metadata(b'Bearer fixture-secret', enrollment, status_fetch) == expected, label
        print('PASS token metadata gate', label)
    assert not token_allows_metadata(b'Bearer wrong-token', enrollment, lambda _: pending)
    print('PASS token metadata gate invalid token')
    sys.exit(0)


sys.stdout.reconfigure(encoding='utf-8', errors='backslashreplace')
sys.stderr.reconfigure(encoding='utf-8', errors='backslashreplace')
repo = Path(__file__).resolve().parent.parent
system = platform.system()
windows = os.name == 'nt'
exe = '.exe' if windows else ''
runner = Path(os.environ.get('RUNNER_BINARY', repo / ('target/debug/ferry-runner' + exe))).resolve()
backend = Path(os.environ.get('FERRY_BINARY', repo / ('target/debug/ferry-server' + exe))).resolve()
name = 'runner-bootstrap-' + uuid.uuid4().hex
state = (Path(os.environ['ProgramData']) if windows else Path('/var/lib')) / name
process_options = {'creationflags': subprocess.CREATE_NO_WINDOW} if windows else {}

def command(*args, check=True, env=None, timeout=90):
    result = subprocess.run([str(arg) for arg in args], env=env, capture_output=True,
                            text=True, encoding='utf-8', errors='backslashreplace',
                            timeout=timeout, **process_options)
    if check:
        assert result.returncode == 0, (args, result.stdout, result.stderr)
    return result

def wait(predicate, label, seconds=45):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate(): return
        time.sleep(.2)
    raise AssertionError(label)

def service(action, check=True):
    return command(runner, 'service', action, '--name', name, check=check)

def service_pid():
    if system == 'Darwin':
        result = command('launchctl', 'print', 'system/' + name, check=False)
        found = re.search(r'\bpid = (\d+)', result.stdout)
    elif system == 'Linux':
        result = command('systemctl', 'show', name, '--property=MainPID', check=False)
        found = re.search(r'MainPID=(\d+)', result.stdout)
    else:
        result = command('sc.exe', 'queryex', name, check=False)
        found = re.search(r'PID\s*:\s*(\d+)', result.stdout)
    return int(found[1]) if found else 0

def interrupted(signum, frame):
    raise TimeoutError('bootstrap acceptance exceeded deadline')

if not windows:
    assert os.geteuid() == 0, 'Run this native CI fixture with sudo.'
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGALRM, interrupted)
    signal.alarm(420)
assert runner.is_file() and backend.is_file(), 'Build the native binaries in CI before this fixture.'
assert not state.exists(), 'Random fixture service state unexpectedly exists.'

with tempfile.TemporaryDirectory(prefix='runner-bootstrap-',
                                 dir=os.environ['ProgramFiles'] if windows else None) as temp:
    root = Path(temp)
    server = None
    proxy = None
    https = None
    trusted = False
    log = (root / 'server.log').open('w', encoding='utf-8')
    try:
        payload = runner.read_bytes()
        digest = hashlib.sha256(payload).hexdigest()
        # Preserve size but change contents, proving the independent digest gate.
        corrupt = bytes([payload[0] ^ 1]) + payload[1:]
        artifact = {'bytes': payload}
        version_match = re.search(r'\d+\.\d+\.\d+[^\s]*', command(runner, '--version').stdout)
        assert version_match, 'Runner version output missing'
        version = version_match[0]
        openssl = shutil.which('openssl') or 'C:/Program Files/Git/usr/bin/openssl.exe'
        (root / 'ca.cnf').write_text('[req]\ndistinguished_name=dn\nx509_extensions=ca\n[dn]\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n')
        command(openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-keyout', root/'ca.key',
                '-out', root/'ca.crt', '-days', '1', '-nodes', '-subj', '/CN=Bootstrap CI root', '-config', root/'ca.cnf')
        command(openssl, 'req', '-new', '-newkey', 'rsa:2048', '-keyout', root/'tls.key',
                '-out', root/'tls.csr', '-nodes', '-subj', '/CN=localhost')
        (root/'tls.ext').write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n')
        command(openssl, 'x509', '-req', '-in', root/'tls.csr', '-CA', root/'ca.crt',
                '-CAkey', root/'ca.key', '-CAcreateserial', '-out', root/'tls.crt', '-days', '1', '-extfile', root/'tls.ext')
        fingerprint = command(openssl, 'x509', '-in', root/'ca.crt', '-noout', '-fingerprint', '-sha1').stdout.strip().split('=')[1].replace(':', '')
        if windows:
            command('certutil', '-addstore', 'Root', root/'ca.crt')
            trusted = True

        class ArtifactHandler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                if self.path != '/runner':
                    self.send_error(404)
                    return
                body = artifact['bytes']
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                try: self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError): pass

        https = http.server.ThreadingHTTPServer(('127.0.0.1', 0), ArtifactHandler)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(root/'tls.crt', root/'tls.key')
        https.socket = tls.wrap_socket(https.socket, server_side=True)
        threading.Thread(target=https.serve_forever, daemon=True).start()
        binary_url = f'https://localhost:{https.server_port}/runner'
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            backend_port = probe.getsockname()[1]
        backend_origin = f'http://127.0.0.1:{backend_port}'
        env = dict(os.environ, FERRY_DATA_DIR=str(root/'server'), FERRY_BIND=f'127.0.0.1:{backend_port}')
        env.pop('FERRY_DATABASE_URL', None)
        server = subprocess.Popen([str(backend)], env=env, stdout=log, stderr=log, **process_options)
        jar = http.cookiejar.CookieJar()
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(jar))
        def api(path, body=None):
            request = urllib.request.Request(backend_origin + '/api' + path,
                data=json.dumps(body).encode() if body is not None else None,
                headers={'Content-Type':'application/json', 'X-Ferry-Request':'1'})
            with client.open(request, timeout=10) as response: return json.load(response)
        def ready():
            try: return api('/status')['ok']
            except OSError: return False
        wait(ready, 'backend startup')
        api('/setup', {'name':'Bootstrap Test','email':'bootstrap@example.invalid',
                      'password':'temporary-bootstrap-password','token':(root/'server/setup-token').read_text().strip()})
        api('/workspaces', {'name':'Bootstrap','slug':'bootstrap'})
        host = api('/workspaces/bootstrap/hosts', {'name':'Bootstrap runner','connection':'runner'})['id']
        enrollment = api('/workspaces/bootstrap/runner-tokens', {'host_id':host})
        token = enrollment['token']
        def token_status(token_id):
            return api('/workspaces/bootstrap/runner-tokens/' + token_id)
        expected_target = {
            ('Linux','x86_64'):'x86_64-unknown-linux-gnu', ('Linux','aarch64'):'aarch64-unknown-linux-gnu',
            ('Darwin','x86_64'):'x86_64-apple-darwin', ('Darwin','arm64'):'aarch64-apple-darwin',
            ('Windows','AMD64'):'x86_64-pc-windows-msvc',
        }[(system, platform.machine())]
        metadata_requests = []

        class ProxyHandler(socketserver.BaseRequestHandler):
            def handle(self):
                self.request.settimeout(20)
                header = bytearray()
                while b'\r\n\r\n' not in header:
                    chunk = self.request.recv(4096)
                    if not chunk: return
                    header.extend(chunk)
                    if len(header) > 65536: return
                first = bytes(header).split(b'\r\n', 1)[0].decode('ascii')
                path = first.split(' ')[1]
                if path.startswith('/api/runners/install/'):
                    fields = bytes(header).split(b'\r\n\r\n', 1)[0].split(b'\r\n')[1:]
                    headers = {k.lower():v.strip() for field in fields for k, v in [field.split(b':', 1)]}
                    accepted = path == '/api/runners/install/' + expected_target and token_allows_metadata(headers.get(b'authorization'), enrollment, token_status)
                    metadata_requests.append(accepted)
                    body = f'{version}\n{binary_url}\n{digest}\n{len(payload)}\n'.encode() if accepted else b'unauthorized'
                    status = b'200 OK' if accepted else b'401 Unauthorized'
                    self.request.sendall(b'HTTP/1.1 '+status+b'\r\nContent-Type: text/plain\r\nConnection: close\r\nContent-Length: '+str(len(body)).encode()+b'\r\n\r\n'+body)
                    return
                # Forward the original HTTP upgrade and any already-read bytes;
                # thereafter bridge raw WebSocket traffic without interpreting it.
                with socket.create_connection(('127.0.0.1', backend_port), timeout=20) as upstream:
                    upstream.sendall(header)
                    with selectors.DefaultSelector() as selector:
                        selector.register(self.request, selectors.EVENT_READ, upstream)
                        selector.register(upstream, selectors.EVENT_READ, self.request)
                        deadline = time.monotonic() + 400
                        while time.monotonic() < deadline:
                            for key, _ in selector.select(timeout=1):
                                try:
                                    chunk = key.fileobj.recv(65536)
                                    if not chunk: return
                                    key.data.sendall(chunk)
                                except OSError: return

        class Proxy(socketserver.ThreadingTCPServer):
            daemon_threads = True
            allow_reuse_address = True
        proxy = Proxy(('127.0.0.1', 0), ProxyHandler)
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        origin = f'http://127.0.0.1:{proxy.server_address[1]}'
        installer_env = dict(os.environ, FERRY_TOKEN=token, CURL_CA_BUNDLE=str(root/'ca.crt'))
        if windows:
            system_root = Path(os.environ['SystemRoot'])
            powershell = system_root/'System32/WindowsPowerShell/v1.0/powershell.exe'
            installer_env['PSModulePath'] = str(system_root/'System32/WindowsPowerShell/v1.0/Modules')
            def quote(value): return "'" + str(value).replace("'", "''") + "'"
            expression = "[Console]::OutputEncoding = [Text.UTF8Encoding]::new(); & " + quote(repo/'scripts/runner-install.ps1') + ' -Server ' + quote(origin) + ' -Name ' + quote(name)
            install_args = [powershell, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', expression]
        else:
            install_args = ['/bin/sh', repo/'scripts/runner-install.sh', '--server', origin, '--name', name]
        host_path = '/workspaces/bootstrap/hosts/' + host
        update_path = host_path + '/runner-update'
        if windows:
            # Existing unsafe state must never be promoted to trusted service
            # state. Keep its content and ACL intact when installation rejects it.
            state.mkdir()
            sentinel = state/'untrusted-marker'
            sentinel.write_bytes(b'preexisting untrusted content')
            command('icacls.exe', state, '/grant', '*S-1-5-32-545:(OI)(CI)M')
            acl_args = [powershell, '-NoProfile', '-NonInteractive', '-Command',
                        '(Get-Acl -LiteralPath ' + quote(state) + ').Sddl']
            unsafe_acl = command(*acl_args, env=installer_env).stdout
            unsafe = command(runner, 'service', 'install', '--server', origin, '--name', name,
                             env=installer_env, check=False)
            assert unsafe.returncode != 0, 'preexisting unsafe default state was accepted'
            assert 'Untrusted write access' in unsafe.stdout + unsafe.stderr, (unsafe.stdout, unsafe.stderr)
            assert sentinel.read_bytes() == b'preexisting untrusted content'
            assert command(*acl_args, env=installer_env).stdout == unsafe_acl, 'rejection changed unsafe ACL'
            assert not (state/'service.json').exists()
            assert not (state/'runner-service.exe').exists()
            shutil.rmtree(state)
            print('PASS unsafe existing default state rejected without changing ACL or files', flush=True)
        assert not state.exists(), 'bootstrap must create fresh default native state'
        installed = command(*install_args, env=installer_env, timeout=150)
        if windows:
            assert f'Installing Runner {version} as {name}...' in installed.stdout, installed.stdout
        wait(lambda: api(host_path)['state'] == 'online', 'bootstrap service enrols automatically')
        wait(lambda: api(update_path)['ready'], 'managed service authenticates worker readiness')
        assert api(update_path)['supported'] is True
        identity = (state/'runner.json').read_bytes()
        first_pid = service_pid()
        assert first_pid > 0
        assert metadata_requests == [True]
        print('PASS bootstrap verifies HTTPS artifact, installs native service and enrols intended host', flush=True)
        assert token_status(enrollment['id'])['state'] == 'connected'
        used_command = command(*install_args, env=installer_env, timeout=150, check=False)
        assert used_command.returncode != 0, 'consumed token must reject the old installation command'
        assert '401' in used_command.stdout + used_command.stderr, (used_command.stdout, used_command.stderr)
        assert metadata_requests == [True, False]
        assert service_pid() == first_pid
        assert (state/'runner.json').read_bytes() == identity
        assert api(host_path)['state'] == 'online' and api(update_path)['ready']
        print('PASS consumed installation command returns 401 and preserves the healthy native service', flush=True)
        # The explicit bootstrap rebind rotates credentials and restarts the
        # service, while keeping the same registered host.
        enrollment = api('/workspaces/bootstrap/runner-tokens', {'host_id':host})
        installer_env['FERRY_TOKEN'] = enrollment['token']
        command(*install_args, env=installer_env, timeout=150)
        wait(lambda: token_status(enrollment['id'])['state'] == 'connected', 'rebind consumes fresh token')
        wait(lambda: api(host_path)['state'] == 'online' and api(update_path)['ready'], 'rebound service is ready')
        assert service_pid() > 0 and service_pid() != first_pid, 'rebind must restart service'
        rebound_identity = (state/'runner.json').read_bytes()
        assert rebound_identity != identity, 'rebind must rotate the long-term key'
        assert json.loads(rebound_identity)['host_id'] == host, 'rebind must preserve host ID'
        assert token_status(enrollment['id'])['used_at'] > 0
        assert metadata_requests == [True, False, True]
        first_pid, identity = service_pid(), rebound_identity
        print('PASS fresh installation command rotates credentials, restarts native service and preserves host ID', flush=True)
        enrollment = api('/workspaces/bootstrap/runner-tokens', {'host_id':host})
        installer_env['FERRY_TOKEN'] = enrollment['token']
        artifact['bytes'] = corrupt
        rejected = command(*install_args, env=installer_env, timeout=150, check=False)
        assert rejected.returncode != 0, 'tampered executable was accepted'
        assert 'SHA256 mismatch' in rejected.stdout + rejected.stderr, (rejected.stdout, rejected.stderr)
        assert service_pid() == first_pid
        assert (state/'runner.json').read_bytes() == identity
        assert api(host_path)['state'] == 'online' and api(update_path)['ready']
        assert metadata_requests == [True, False, True, True]
        assert token_status(enrollment['id'])['state'] == 'pending'
        print('PASS tampered download rejected before execution; original native service remains healthy', flush=True)
    except Exception:
        log.flush()
        print((root/'server.log').read_text(encoding='utf-8', errors='replace')[-12000:], flush=True)
        if (state/'service.log').exists():
            print((state/'service.log').read_text(encoding='utf-8', errors='replace')[-12000:], flush=True)
        raise
    finally:
        # Cleanup each owned resource even if another cleanup action fails.
        cleanup_errors = []
        try:
            service('uninstall', check=False)
            wait(lambda: service_pid() == 0, 'bootstrap service cleanup', seconds=45)
            if state.exists(): shutil.rmtree(state)
        except Exception as error: cleanup_errors.append(error)
        if server:
            server.terminate()
            try: server.wait(timeout=20)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=10)
        for listener in (proxy, https):
            if listener:
                listener.shutdown()
                listener.server_close()
        if trusted:
            try: command('certutil', '-delstore', 'Root', fingerprint)
            except Exception as error: cleanup_errors.append(error)
        log.close()
        if cleanup_errors: raise RuntimeError('Bootstrap fixture cleanup failed', cleanup_errors)
        if not windows: signal.alarm(0)
