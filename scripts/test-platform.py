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
            method=method, headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN'})
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
        check(status == {'required': True, 'has_users': False, 'registration_enabled': False,
                         'email_verification_required': False}, 'fresh install requires initialization')
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
        workspace_tenant = ws['id']
        request(outsider, f'/workspaces/{workspace_tenant}/nodes', expected=404)
        for path in ['/platform/users', '/platform/workspaces', '/platform/settings/1']:
            request(owner, path, expected=403)
            request(anonymous, path, expected=401)
        request(owner, '/platform/users/'+owner_id, {'version':1,'active':True,'is_superadmin':True}, 'PATCH', 403)
        check(not request(owner, '/me')['user']['is_superadmin'], 'workspace owner cannot self-elevate or access platform')
        check(request(admin, f'/workspaces/{workspace_tenant}/nodes')['total'] == 0, 'superadmin can enter an unjoined workspace')
        check(request(admin, '/me')['workspaces'][0]['slug'] == 'tenant', 'superadmin identity includes all workspaces')
        users = request(admin, '/platform/users?q=owner&page_size=1')
        check(users['total'] == 1 and len(users['items']) == 1, 'global list uses search and pagination')
        check('password' not in json.dumps(users) and 'token' not in json.dumps(users), 'global user projection excludes credentials')
        request(admin, '/platform/users?sort=password_hash', expected=400)
        request(admin, '/platform/users?workspace_id=1', expected=400)
        # Self-service profile: the name freely; email and password only with the current password.
        profile = request(owner, '/profile/1')
        check(profile['email'] == 'owner@example.invalid' and profile['name'] == 'Owner' and profile['new_password'] == ''
              and 'hash' not in json.dumps(profile), 'profile shows own name and email, never a password')
        request(anonymous, '/profile/1', expected=401)
        owner_elsewhere = client()
        request(owner_elsewhere, '/sessions', {'email':'owner@example.invalid','password':password})
        request(owner, '/profile/1', {**profile, 'name':'Owner Renamed'}, 'PATCH')
        check(request(owner, '/me')['user']['name'] == 'Owner Renamed' and request(owner_elsewhere, '/me')['user']['name'] == 'Owner Renamed',
              'renaming needs no password and keeps other sessions')
        request(owner, '/profile/1', {**profile, 'name':'Stale'}, 'PATCH', 409)
        current = request(owner, '/profile/1')
        for body, status in [({'email':'new-owner@example.invalid'}, 400),
                             ({'email':'new-owner@example.invalid','current_password':'wrong-password'}, 400),
                             ({'new_password':password+'-x','current_password':'wrong-password'}, 400),
                             ({'email':'Outsider@Example.invalid','current_password':password}, 409),
                             ({'email':'not-an-email','current_password':password}, 400),
                             ({'name':'  '}, 400),
                             ({'new_password':'short','current_password':password}, 400)]:
            request(owner, '/profile/1', {**current, **body}, 'PATCH', status)
        check(request(owner, '/profile/1') == current, 'refuses a missing or wrong current password, a taken or invalid email, an empty name and a short password')
        new_password = password + '-new'
        request(owner, '/profile/1', {**current, 'email':' New-Owner@Example.invalid ', 'new_password':new_password, 'current_password':password}, 'PATCH')
        check(request(owner, '/me')['user']['email'] == 'new-owner@example.invalid', 'email is normalized and the editing session stays signed in')
        request(owner_elsewhere, '/me', expected=401)
        check(True, 'changing email or password signs out every other session')
        for email, secret in [('owner@example.invalid', password), ('owner@example.invalid', new_password), ('new-owner@example.invalid', password)]:
            request(client(), '/sessions', {'email':email,'password':secret}, expected=401)
        again = client()
        request(again, '/sessions', {'email':'new-owner@example.invalid','password':new_password})
        check(request(again, '/me')['user']['id'] == owner_id, 'only the new email with the new password signs in')
        check(request(admin, '/platform/users/'+owner_id)['email'] == 'new-owner@example.invalid', 'administrators see the changed email')
        back = request(owner, '/profile/1')
        request(owner, '/profile/1', {**back, 'name':'Owner', 'email':'owner@example.invalid', 'new_password':password, 'current_password':new_password}, 'PATCH')
        check(request(owner, '/me')['user']['email'] == 'owner@example.invalid', 'the original email and password can be restored')
        created = request(admin, '/platform/workspaces', {'name':'New workspace','slug':'platform'})
        workspace_platform = created['id']
        check(created['slug'] == 'platform', 'legacy-valid platform workspace slug remains available')
        row = request(admin, '/platform/workspaces/'+ws['id'])
        request(admin, '/platform/workspaces/'+ws['id'], {'name':'Renamed','version':row['version']}, 'PATCH')
        request(admin, '/platform/workspaces/'+ws['id'], {'name':'Stale','version':row['version']}, 'PATCH', 409)
        check(request(admin, '/platform/workspaces/'+ws['id'])['name'] == 'Renamed', 'workspace rename protects stale writes')
        check(request(owner, f'/workspaces/{workspace_tenant}/nodes')['total'] == 0,
              'workspace rename keeps existing ID-based resource URLs valid')
        # Each workspace says whether the user created it, joined it, or only administers it.
        listed = {w['slug']: (w['relation'], w['my_role']) for w in request(admin, '/platform/workspaces')['items']}
        check(listed == {'platform': ('owner', '拥有者'), 'tenant': ('none', '')}, 'platform list marks the workspace the admin created and the one not joined')
        mine = {w['slug']: (w['relation'], w['role_name']) for w in request(admin, '/me')['workspaces']}
        check(mine == {'platform': ('owner', '拥有者'), 'tenant': ('none', '平台超级管理员')}, 'admin identity keeps real roles instead of a blanket administrator role')
        check([(w['slug'], w['relation']) for w in request(owner, '/me')['workspaces']] == [('tenant', 'owner')], 'a member identity marks the workspace they created')
        invitation = request(owner, f'/workspaces/{workspace_tenant}/members/invitation')['code']
        request(admin, '/workspace-memberships', {'code': invitation})
        relation, role = {w['slug']: (w['relation'], w['my_role']) for w in request(admin, '/platform/workspaces')['items']}['tenant']
        check(relation == 'member' and role not in ('', '拥有者'), 'after joining, the workspace shows the joined role')
        check({w['slug']: w['relation'] for w in request(admin, '/me')['workspaces']}['tenant'] == 'member', 'identity shows the joined workspace as a membership')
        for active, superadmin in [(False, True), (True, False)]:
            request(admin, '/platform/users/'+admin_id, {'version':1, 'active':active, 'is_superadmin':superadmin}, 'PATCH', 409)
        check(request(admin, '/me')['user']['is_superadmin'], 'last active admin cannot be disabled or demoted')
        settings = request(admin, '/platform/settings/1')
        check(settings['runner_auto_update'] is True, 'automatic Runner updates default on globally')
        request(owner, '/platform/settings/1', expected=403)
        request(admin, '/platform/settings/1', {'version':settings['version'], 'registration_enabled':True, 'runner_auto_update':False}, 'PATCH')
        settings = request(admin, '/platform/settings/1')
        check(settings['runner_auto_update'] is False, 'platform administrator can disable the automatic upgrade default')
        request(admin, '/platform/settings/1', {'version':settings['version'], 'registration_enabled':True}, 'PATCH')
        settings = request(admin, '/platform/settings/1')
        check(settings['runner_auto_update'] is False, 'legacy platform edits preserve the automatic upgrade policy')
        host_path = f'/workspaces/{workspace_tenant}/nodes'
        upgrade_host = request(admin, host_path, {'name':'automatic policy acceptance', 'connection':'runner'}, expected=201)['id']
        host_path += '/' + upgrade_host
        host = request(admin, host_path)
        check(host['runner_auto_update'] == 'inherit', 'existing host create requests inherit the platform default')
        for policy, enabled in [('on', True), ('off', False), ('inherit', False)]:
            request(admin, host_path, {'name':host['name'], 'version':host['version'], 'runner_auto_update':policy}, 'PATCH')
            host = request(admin, host_path)
            status = request(admin, host_path+'/runner-updates')
            check(host['runner_auto_update'] == policy and status['enabled'] is enabled, f'host policy {policy} resolves against disabled global default')
        request(admin, host_path, {'name':host['name'], 'version':host['version'], 'runner_auto_update':'invalid'}, 'PATCH', 400)
        request(admin, host_path, {'name':host['name'], 'version':host['version'], 'runner_auto_update':'off'}, 'PATCH')
        host = request(admin, host_path)
        request(admin, host_path, {'name':host['name'], 'version':host['version']}, 'PATCH')
        check(request(admin, host_path)['runner_auto_update'] == 'off', 'legacy host edits preserve explicit automatic update override')
        request(admin, '/platform/settings/1', {'version':settings['version'], 'registration_enabled':True, 'runner_auto_update':True}, 'PATCH')
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
        logs = (data / 'server.log').read_text(encoding='utf-8')
        check(token not in logs and password not in logs and new_password not in logs, 'logs contain no initialization token or password')
        print(f'PASS: {checks} platform acceptance checks')
    finally:
        if proc.poll() is None: proc.terminate(); proc.wait(timeout=20)
        log.close()
