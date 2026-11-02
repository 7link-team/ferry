#!/usr/bin/env python3
"""Six-language acceptance against an isolated real server and its SQLite DB.

Usage: python scripts/test-i18n-http.py SERVER_BINARY
No existing instance, credentials, services or external providers are used.
"""
import http.cookiejar
from contextlib import closing
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


def main():
    binary = str(Path(sys.argv[1]).resolve())
    catalogs = Path(__file__).resolve().parents[1] / 'crates/server/src/i18n'
    locales = ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY']
    messages = {}
    for locale in locales:
        messages[locale] = {}
        for prefix in ['', 'auth-', 'repository-']:
            messages[locale].update(json.loads((catalogs / f'{prefix}{locale}.json').read_text(encoding='utf8')))
    with tempfile.TemporaryDirectory(prefix='locale-http-') as directory:
        data = Path(directory)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        origin = f'http://127.0.0.1:{port}/api'
        environment = dict(os.environ, FERRY_DATA_DIR=directory, FERRY_BIND=f'127.0.0.1:{port}')
        environment.pop('FERRY_DATABASE_URL', None)
        environment.pop('FERRY_NATS_URL', None)
        flags = {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        anonymous = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def request(path, body=None, language=None, expected=200, session=client):
            headers = {'Content-Type': 'application/json', 'X-Ferry-Request': '1'}
            if language is not None:
                headers['Accept-Language'] = language
            req = urllib.request.Request(origin + path, data=json.dumps(body).encode() if body is not None else None, headers=headers)
            try:
                response = session.open(req, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                value = json.load(response)
                assert response.code == expected, (path, response.code, value)
                assert 'accept-language' in response.headers.get('Vary', '').lower()
                return value, response.headers['Content-Language']

        with (data / 'server.log').open('wb') as log:
            process = subprocess.Popen([binary], env=environment, stdout=log, stderr=log, **flags)
            try:
                for _ in range(200):
                    if process.poll() is not None:
                        raise AssertionError('Isolated server exited before startup')
                    try:
                        request('/status')
                        break
                    except OSError:
                        time.sleep(.1)
                else:
                    raise AssertionError('Isolated server did not start')
                request('/setup', {'name': 'Locale fixture', 'email': 'locale@example.invalid', 'password': 'temporary-locale-password', 'token': (data / 'setup-token').read_text().strip()})
                workspace = request('/workspaces', {'name': 'User workspace / 用户', 'slug': 'locale-fixture'})[0]['id']
                root = f'/workspaces/{workspace}/credentials'
                label = 'User-owned / 用户 / 日本語'
                request(root, {'name': label, 'kind': 'password', 'scope': 'host', 'secret': 'temporary-local-secret'}, expected=201)
                db_files = list(data.glob('*.db')) + list(data.glob('*.sqlite'))
                assert len(db_files) == 1
                with closing(sqlite3.connect(db_files[0])) as connection:
                    original = connection.execute('SELECT id,name,secret FROM credentials').fetchall()
                for locale in locales:
                    value, chosen = request('/workspaces/123/nodes', language=locale, expected=401, session=anonymous)
                    assert chosen == locale
                    assert value['error'] == messages[locale]['error.unauthorized']
                    value, chosen = request(root + '?page=0', language=locale, expected=400)
                    assert chosen == locale
                    assert value['error'] == messages[locale]['resource.positive_integer'].replace('{field}', 'page')
                    value, _ = request(root, {'name': label, 'kind': 'password', 'scope': 'invalid'}, language=locale, expected=400)
                    assert value['error'] == messages[locale]['credentials.scopeInvalid']
                    value, _ = request(root, language=locale)
                    assert value['total'] == 1 and value['items'][0]['name'] == label
                for language, expected in [(None, 'en-US'), ('fr-FR', 'en-US'), ('zh-TW;q=0.5,ja-JP;q=0.9', 'ja-JP')]:
                    value, chosen = request(root + '?page=0', language=language, expected=400)
                    assert chosen == expected
                    assert value['error'] == messages[expected]['resource.positive_integer'].replace('{field}', 'page')
                with closing(sqlite3.connect(db_files[0])) as connection:
                    assert connection.execute('SELECT id,name,secret FROM credentials').fetchall() == original
                print('PASS: actual six-language auth/validation/reader HTTP boundaries, English fallback, quality negotiation, Vary headers, original user names and unchanged encrypted credentials')
            finally:
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)


if __name__ == '__main__':
    main()
