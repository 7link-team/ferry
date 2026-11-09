#!/usr/bin/env python3
"""Opt-in live Cloudflare quick-tunnel acceptance; never uses the user's server.

Run after building server and Runner. CLOUDFlared must be installed or supplied
with CLOUDFLARED_BINARY. Requires outbound Cloudflare HTTPS/QUIC connectivity.
Creates disposable data, enrolls a real Runner over public WSS, executes a
connection test, then restarts the origin and proves authenticated reconnect.
Logs stay in data/maintenance/cloudflared-<unique>/; no tokens are printed.
"""
import http.cookiejar
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import time
import urllib.request
import urllib.error
import urllib.parse
import uuid


def main():
    suffix = '.exe' if os.name == 'nt' else ''
    server = str(Path(os.environ.get('FERRY_BINARY', 'target/debug/ferry-server'+suffix)).resolve())
    runner = str(Path(os.environ.get('FERRY_RUNNER_BINARY', 'target/debug/ferry-runner'+suffix)).resolve())
    cloud = os.environ.get('CLOUDFLARED_BINARY') or shutil.which('cloudflared')
    if not cloud and os.name == 'nt':
        cloud = 'C:/Program Files (x86)/cloudflared/cloudflared.exe'
    assert cloud and Path(cloud).is_file(), 'cloudflared is required (no silent skip)'
    work = Path('data/maintenance')/('cloudflared-'+uuid.uuid4().hex[:10])
    work.mkdir(parents=True)
    data = work/'server'
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env = dict(os.environ, FERRY_DATA_DIR=str(data.resolve()), FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL', None)
    env.pop('FERRY_NATS_URL', None)
    flags = {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}
    processes, logs = [], []
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}),
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def start(args, name, environment=None):
        log = (work/(name+'.log')).open('ab')
        logs.append(log)
        process = subprocess.Popen(args, env=environment, stdout=log, stderr=log, **flags)
        processes.append(process)
        return process

    def request(path, body=None, base=origin):
        req = urllib.request.Request(base+path,
            data=json.dumps(body).encode() if body is not None else None,
            headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
        with client.open(req, timeout=15) as response:
            return json.load(response)

    def until(probe, label, seconds=60):
        deadline = time.monotonic()+seconds
        last = None
        while time.monotonic() < deadline:
            try:
                value = probe()
                if value:
                    return value
            except urllib.error.HTTPError as error:
                last = f'HTTP {error.code}'
            except (OSError, ValueError) as error:
                last = type(error).__name__
            time.sleep(.5)
        raise AssertionError(f'{label} timed out ({last}); inspect {work}')

    evidence = {'scope': 'live credentialless quick tunnel; not a named tunnel', 'result': 'FAIL'}
    try:
        service = start([server], 'server', env)
        until(lambda: request('/api/status').get('ok'), 'origin ready')
        request('/api/setup', {'email': 'tunnel@example.invalid', 'name': 'Tunnel test',
            'password': 'isolated-tunnel-password', 'token': (data/'setup-token').read_text().strip()})
        ws = request('/api/workspaces', {'name': 'Tunnel test', 'slug': 'tunnel'})['id']
        root = f'/api/workspaces/{ws}'
        host = request(root+'/nodes', {'name': 'Tunnel Runner', 'connection': 'runner'})['id']
        token = request(root+'/runner-tokens', {'host_id': host})['token']
        # HTTP/2 also works on networks that block outbound QUIC/UDP. Callers
        # can explicitly select quic to exercise that transport separately.
        protocol = os.environ.get('CLOUDFLARED_PROTOCOL', 'http2')
        extra = ['--no-prechecks'] if os.environ.get('CLOUDFLARED_NO_PRECHECKS') == '1' else []
        tunnel = start([cloud, 'tunnel', '--no-autoupdate', *extra, '--protocol', protocol, '--url', origin], 'tunnel')

        def endpoint():
            assert tunnel.poll() is None, 'cloudflared exited; inspect tunnel.log'
            match = re.search(r'https://[a-z0-9-]+\.trycloudflare\.com',
                (work/'tunnel.log').read_text(encoding='utf8', errors='replace'))
            return match.group() if match else None

        public = until(endpoint, 'quick tunnel URL', 90)
        # Public deployments must explicitly allow the published hostname. Keep
        # the normal Host defence enabled instead of rewriting forwarded Host.
        service.terminate()
        service.wait(timeout=30)
        service = start([server], 'server', env)
        until(lambda: request('/api/status').get('ok'), 'origin with allowed tunnel host')
        until(lambda: request('/api/status', base=public).get('ok'), 'public HTTPS ready', 90)
        evidence['public_https'] = True
        print('PASS public HTTPS reaches isolated origin', flush=True)
        spec = importlib.util.spec_from_file_location('ws_probe', Path(__file__).with_name('ws-probe.py'))
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        socket_probe = probe.Ws(public.replace('https://', 'wss://')+'/api/runners/connect')
        try:
            socket_probe.text(json.dumps({'version': 99, 'auth': {'type': 'enroll', 'token': 'invalid'},
                'hostname': 'probe', 'os': 'linux', 'arch': 'x86_64', 'agent_version': '0.0.0'}))
            reply = next(json.loads(payload) for opcode, payload in socket_probe.frames(15) if opcode == probe.TEXT)
            assert reply.get('type') == 'rejected', reply
            evidence['public_wss_rejection'] = True
        finally:
            socket_probe.close()
        agent = start([runner, '--server', public, '--token', token,
            '--state-dir', str((work/'runner').resolve())], 'runner')
        until(lambda: request(root+'/nodes/'+host).get('state') == 'online', 'Runner enroll over WSS', 90)
        assert agent.poll() is None
        evidence['runner_wss_enrollment'] = True
        assert request(root+'/nodes/'+host+'/connection-tests', {})['ok']
        evidence['exec_roundtrip'] = True
        print('PASS real Runner WSS enrollment and remote exec round trip', flush=True)
        service.terminate()
        service.wait(timeout=30)
        # Keep the tunnel and Runner alive: only the disposable origin restarts.
        service = start([server], 'server', env)
        until(lambda: request('/api/status').get('ok'), 'restarted origin')
        until(lambda: request(root+'/nodes/'+host).get('state') == 'online', 'authenticated WSS reconnect', 90)
        # The persisted host can still say online before its new control socket
        # registers; only successful execution proves authenticated reconnection.
        until(lambda: request(root+'/nodes/'+host+'/connection-tests', {}).get('ok'),
            'remote execution after authenticated WSS reconnect', 90)
        assert agent.poll() is None and tunnel.poll() is None
        evidence.update(authenticated_reconnect=True, second_exec_roundtrip=True, result='PASS')
        print('PASS origin restart, authenticated WSS reconnect and second exec round trip', flush=True)
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
        for log in logs:
            log.close()
        with socket.socket() as sock:
            evidence['origin_port_closed'] = sock.connect_ex(('127.0.0.1', port)) != 0
        (work/'result.json').write_text(json.dumps(evidence, indent=2), encoding='utf8')
        print('Evidence:', work/'result.json', flush=True)


if __name__ == '__main__':
    main()
