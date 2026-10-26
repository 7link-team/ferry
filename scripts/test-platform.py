#!/usr/bin/env python3
"""Real HTTP platform acceptance; optional empty FERRY_TEST_DATABASE_URL."""
import concurrent.futures
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
            method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
        try: response = c.open(req, timeout=30)
        except urllib.error.HTTPError as e: response = e
        value = json.load(response)
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
        status = request(anonymous, '/setup')
        check(status == {'required': True, 'has_users': False, 'registration_enabled': False}, 'fresh install requires initialization')
        token = (data / 'setup-token').read_text().strip()
        check(len(token) >= 32, 'operator proof has random secret length')
        if os.name != 'nt': check((data / 'setup-token').stat().st_mode & 0o777 == 0o600, 'operator proof file is private')
        password = 'correct-horse-battery'
        admin_body = {'email': 'admin@example.invalid', 'name': 'Admin', 'password': password, 'token': token}
        request(anonymous, '/users', admin_body, expected=403)
        request(anonymous, '/setup', dict(admin_body, token='wrong'), expected=403)
        request(anonymous, '/setup', dict(admin_body, password='short'), expected=400)
        check(request(anonymous, '/setup')['required'], 'bad token/password cannot initialize or register')
        first, second = client(), client()
        with concurrent.futures.ThreadPoolExecutor() as pool:
            results = list(pool.map(lambda c: request(c, '/setup', admin_body, expected=None), [first, second]))
        check(sorted(x[0] for x in results) == [200, 409], 'concurrent initialization has exactly one winner')
        admin = first if results[0][0] == 200 else second
        me = request(admin, '/me')
        admin_id = me['user']['id']
        check(me['user']['is_superadmin'] and me['workspaces'] == [], 'initialization signs in a global admin without workspace')
        request(admin, '/setup', admin_body, expected=409)
        check(not (data / 'setup-token').exists(), 'successful setup removes one-time proof')
        proc.terminate(); proc.wait(timeout=20); proc = launch()
        check(not request(anonymous, '/setup')['required'] and not (data / 'setup-token').exists(), 'restart does not reinitialize or regenerate token')
        check(request(admin, '/me')['user']['is_superadmin'], 'session survives restart')
        owner, outsider = client(), client()
        owner_id = request(owner, '/users', {'name':'Owner','email':'owner@example.invalid','password':password})['id']
        outsider_id = request(outsider, '/users', {'name':'Outsider','email':'outsider@example.invalid','password':password})['id']
        ws = request(owner, '/workspaces', {'name':'Tenant','slug':'tenant'})
        request(outsider, '/workspaces/tenant/hosts', expected=404)
        for path in ['/platform/users', '/platform/workspaces', '/platform/settings/1']:
            request(owner, path, expected=403)
            request(anonymous, path, expected=401)
        request(owner, '/platform/users/'+owner_id, {'version':1,'active':True,'is_superadmin':True}, 'PATCH', 403)
        check(not request(owner, '/me')['user']['is_superadmin'], 'workspace owner cannot self-elevate or access platform')
        check(request(admin, '/workspaces/tenant/hosts')['total'] == 0, 'superadmin can enter an unjoined workspace')
        check(request(admin, '/me')['workspaces'][0]['slug'] == 'tenant', 'superadmin identity includes all workspaces')
        users = request(admin, '/platform/users?q=owner&page_size=1')
        check(users['total'] == 1 and len(users['items']) == 1, 'global list uses search and pagination')
        check('password' not in json.dumps(users) and 'token' not in json.dumps(users), 'global user projection excludes credentials')
        request(admin, '/platform/users?sort=password_hash', expected=400)
        request(admin, '/platform/users?workspace_id=1', expected=400)
        created = request(admin, '/platform/workspaces', {'name':'New workspace','slug':'platform'})
        check(created['slug'] == 'platform', 'legacy-valid platform workspace slug remains available')
        row = request(admin, '/platform/workspaces/'+ws['id'])
        request(admin, '/platform/workspaces/'+ws['id'], {'name':'Renamed','version':row['version']}, 'PATCH')
        request(admin, '/platform/workspaces/'+ws['id'], {'name':'Stale','version':row['version']}, 'PATCH', 409)
        check(request(admin, '/platform/workspaces/'+ws['id'])['name'] == 'Renamed', 'workspace rename protects stale writes')
        for active, superadmin in [(False, True), (True, False)]:
            request(admin, '/platform/users/'+admin_id, {'version':1, 'active':active, 'is_superadmin':superadmin}, 'PATCH', 409)
        check(request(admin, '/me')['user']['is_superadmin'], 'last active admin cannot be disabled or demoted')
        settings = request(admin, '/platform/settings/1')
        request(admin, '/platform/settings/1', {'version':settings['version'],'registration_enabled':False}, 'PATCH')
        request(admin, '/platform/settings/1', {'version':settings['version'],'registration_enabled':True}, 'PATCH', 409)
        request(anonymous, '/users', {'email':'closed@example.invalid','name':'Closed','password':password}, expected=403)
        request(owner, '/sessions', {'email':'owner@example.invalid','password':password})
        check(not request(anonymous, '/setup')['registration_enabled'], 'registration closes but existing accounts can sign in')
        row = request(admin, '/platform/users/'+outsider_id)
        request(admin, '/platform/users/'+outsider_id, {'version':row['version'],'active':False,'is_superadmin':False}, 'PATCH')
        request(outsider, '/me', expected=401)
        request(outsider, '/sessions', {'email':'outsider@example.invalid','password':password}, expected=401)
        check(True, 'disabled account loses existing session and cannot sign in')
        row = request(admin, '/platform/users/'+owner_id)
        request(admin, '/platform/users/'+owner_id, {'version':row['version'],'active':True,'is_superadmin':True}, 'PATCH')
        request(owner, '/me', expected=401)
        request(owner, '/sessions', {'email':'owner@example.invalid','password':password})
        check(request(owner, '/me')['user']['is_superadmin'], 'promotion invalidates old sessions and takes effect after login')
        admin_row = request(admin, '/platform/users/'+admin_id)
        owner_row = request(owner, '/platform/users/'+owner_id)
        with concurrent.futures.ThreadPoolExecutor() as pool:
            jobs = [pool.submit(request, c, '/platform/users/'+r['id'], {'version':r['version'],'active':True,'is_superadmin':False}, 'PATCH', None)
                    for c, r in [(admin, admin_row), (owner, owner_row)]]
            results = [j.result() for j in jobs]
        check(sorted(x[0] for x in results) == [200,409], 'concurrent admin demotion retains one active admin')
        survivor = owner if results[0][0] == 200 else admin
        check(sum(r['is_superadmin'] and r['active'] for r in request(survivor, '/platform/users')['items']) == 1, 'exactly one active admin remains')
        logs = (data / 'server.log').read_text()
        check(token not in logs and password not in logs, 'logs contain no initialization token or password')
        print(f'PASS: {checks} platform acceptance checks')
    finally:
        if proc.poll() is None: proc.terminate(); proc.wait(timeout=20)
        log.close()
