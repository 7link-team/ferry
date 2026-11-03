#!/usr/bin/env python3
"""Real Docker acceptance: fresh directories, legacy migration, failure and relocation.

Runs ONLY against a disposable Compose project and temporary directory. Requires
Linux, Docker Compose and sudo (to archive UID-preserving database directories).
Usage: python3 scripts/test-compose-storage.py postgres|mysql
"""
import base64
import http.cookiejar
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import traceback
import urllib.request
import uuid


def main():
    engine = sys.argv[1]
    assert engine in ('postgres', 'mysql')
    repo = Path(__file__).resolve().parents[1]
    docker = shutil.which('docker')
    assert docker and sys.platform == 'linux', 'Requires a real Linux Docker host'
    project = 'storage_' + uuid.uuid4().hex[:10]
    with tempfile.TemporaryDirectory(prefix='ferry-storage-') as temporary:
        root = Path(temporary)
        deployment = root / 'ferry'
        binary = root / 'bin'
        binary.mkdir()
        # Serve this checkout's Compose files without release/API dependencies.
        (binary / 'curl').write_text('''#!/usr/bin/env python3
import os, shutil, sys
from pathlib import Path
args = sys.argv[1:]
url = args[-1]
relative = url.split("raw.githubusercontent.com/", 1)[1].split("/", 3)[3]
shutil.copyfile(Path(os.environ['STORAGE_TEST_REPO']) / relative, args[args.index('-o') + 1])
''')
        (binary / 'docker').write_text('''#!/bin/sh
case "${STORAGE_TEST_FAILURE:-}:$*" in
    copy:*'/backup'*' sh nats') echo 'Injected copy failure' >&2; exit 77 ;;
    cutover:*' up -d --wait'*)
        if grep -q '^FERRY_STORAGE_LAYOUT=directory$' .env.production; then exit 77; fi ;;
esac
exec "$STORAGE_TEST_DOCKER" "$@"
''')
        for file in binary.iterdir():
            file.chmod(0o755)
        environment = dict(os.environ, PATH=str(binary) + ':' + os.environ['PATH'],
                           STORAGE_TEST_DOCKER=docker, STORAGE_TEST_REPO=str(repo))

        def run(*args, cwd=None, env=None, check=True):
            result = subprocess.run(args, cwd=cwd or deployment, env=env or environment,
                                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if check and result.returncode:
                # Do not dump Compose configuration, env files or seeded secrets.
                raise AssertionError(f'{args[0]} failed ({result.returncode}):\n{result.stdout[-1800:]}')
            return result

        def quick(*args, failure=None, check=True):
            env = dict(environment, STORAGE_TEST_FAILURE=failure or '')
            result = run('sh', str(repo / 'scripts/quickstart.sh'), '--dir', str(deployment),
                         *args, cwd=root, env=env, check=False)
            if check and result.returncode:
                logs = ''
                if (deployment / 'compose.storage.yaml').exists():
                    logs = compose('logs', '--no-color', '--tail', '8', 'ferry', engine, check=False).stdout
                raise AssertionError(f'quickstart failed ({result.returncode}):\n{result.stdout[-700:]}\n{logs[-1300:]}')
            return result

        def settings():
            return dict(line.split('=', 1) for line in (deployment / '.env.production').read_text().splitlines()
                        if line and not line.startswith('#'))

        def set_value(key, value):
            file = deployment / '.env.production'
            lines = [line for line in file.read_text().splitlines() if not line.startswith(key + '=')]
            if value is not None:
                lines.append(f'{key}={value}')
            file.write_text('\n'.join(lines) + '\n')

        def compose(*args, check=True):
            return run(docker, 'compose', '--env-file', '.env.production', '-f', 'compose.production.yaml',
                       '-f', f'compose.{engine}.yaml', '--profile', 'redis',
                       '-f', 'compose.storage.yaml', *args, check=check)

        def container(service):
            return compose('ps', '-a', '-q', service).stdout.strip()

        def port(service, target):
            return int(run(docker, 'port', container(service), str(target)).stdout.strip().rsplit(':', 1)[1])

        client = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

        def api(path, body=None):
            request = urllib.request.Request(f'http://127.0.0.1:{port("ferry", 4290)}/api' + path,
                data=json.dumps(body).encode() if body is not None else None,
                headers={'Content-Type': 'application/json', 'X-Ferry-Request': '1'})
            with client.open(request, timeout=30) as response:
                return json.load(response)

        def nats(subject, body):
            payload = json.dumps(body).encode()
            with socket.create_connection(('127.0.0.1', port('nats', 4222)), timeout=10) as connection:
                authentication = json.dumps({'user': 'ferry', 'pass': settings()['NATS_PASSWORD']})
                connection.sendall(f'CONNECT {authentication}\r\nSUB _storage_reply 1\r\n'.encode()
                    + f'PUB {subject} _storage_reply {len(payload)}\r\n'.encode() + payload + b'\r\nPING\r\n')
                reader = connection.makefile('rb')
                while line := reader.readline():
                    if line.startswith(b'MSG '):
                        data = json.loads(reader.read(int(line.split()[-1])))
                        assert 'error' not in data, data
                        return data
                    assert not line.startswith(b'-ERR'), line
                raise AssertionError('No NATS response')

        def sql(query):
            if engine == 'postgres':
                return compose('exec', '-T', engine, 'psql', '-U', 'postgres', '-d', 'ferry', '-Atc', query).stdout
            return compose('exec', '-T', engine, 'sh', '-ec',
                'MYSQL_PWD="$MYSQL_ROOT_PASSWORD" exec mysql -uroot -N -B ferry -e "$1"', 'sh', query).stdout

        try:
            quick('--init-only', '--database', engine, '--version', 'v0.1.0-alpha.38')
            set_value('COMPOSE_PROJECT_NAME', project)
            set_value('FERRY_BIND_ADDRESS', '127.0.0.1')
            set_value('FERRY_PORT', '0')  # Compose chooses a free published port.
            # Ephemeral, loopback-only test access to NATS; never part of the deployment defaults.
            file = deployment / 'compose.production.yaml'
            before, nats_and_rest = file.read_text().split('  nats:\n', 1)
            # Docker does not publish ports on an internal-only network.
            nats_and_rest = nats_and_rest.replace('    networks: [backend]', '    networks: [backend, egress]', 1)
            file.write_text(before + '  nats:\n    ports: ["127.0.0.1:0:4222"]\n' + nats_and_rest)
            quick()
            mounts = json.loads(run(docker, 'inspect', container('ferry')).stdout)[0]['Mounts']
            assert any(m['Type'] == 'bind' and m['Source'] == str(deployment / 'data/ferry') for m in mounts)
            token = compose('exec', '-T', 'ferry', 'cat', '/data/setup-token').stdout.strip()
            api('/setup', {'name': 'Storage fixture', 'email': 'storage@example.invalid',
                          'password': 'temporary-storage-password', 'token': token})
            workspace = api('/workspaces', {'name': 'Storage fixture', 'slug': 'storage-fixture'})['id']
            prefix = f'/workspaces/{workspace}'
            credential = api(prefix + '/credentials', {'name': 'Persistent credential', 'kind': 'password',
                'scope': 'host', 'secret': 'temporary-storage-secret'})['id']
            variable = api(prefix + '/variables', {'name': 'STORAGE_SECRET', 'scope': 'global',
                'scope_ref': '0', 'kind': 'secret', 'value': 'temporary-encrypted-value'})['id']
            original_ciphertext = sql('SELECT id,secret FROM credentials ORDER BY id')
            original_settings = settings()
            nats('$JS.API.STREAM.CREATE.STORAGE_FIXTURE', {'name': 'STORAGE_FIXTURE',
                'subjects': ['storage.fixture'], 'storage': 'file'})
            nats('storage.fixture', {'persisted': True})
            compose('exec', '-T', 'redis', 'sh', '-ec',
                'REDISCLI_AUTH="$REDIS_PASSWORD" redis-cli SET storage-fixture persisted')
            compose('exec', '-T', 'ferry', 'sh', '-ec',
                'printf persistent > /data/storage-fixture; chmod 640 /data/storage-fixture; ln -s storage-fixture /data/storage-link')
            run(docker, 'run', '--rm', '--network', 'none', '--mount',
                f'type=bind,src={deployment}/data,dst=/target', 'alpine:3.22', 'sh', '-ec',
                'printf certificate-fixture > /target/caddy-data/certificate.fixture; '
                'printf configuration-fixture > /target/caddy-config/configuration.fixture')
            print('PASS fresh directory installation and non-root Ferry writes', flush=True)

            def verify():
                assert api(prefix + f'/credentials/{credential}')['name'] == 'Persistent credential'
                assert api(prefix + f'/variables/{variable}/disclosures', {})['value'] == 'temporary-encrypted-value'
                assert sql('SELECT id,secret FROM credentials ORDER BY id') == original_ciphertext
                assert compose('exec', '-T', 'ferry', 'cat', '/data/storage-link').stdout == 'persistent'
                assert compose('exec', '-T', 'redis', 'sh', '-ec',
                    'REDISCLI_AUTH="$REDIS_PASSWORD" redis-cli GET storage-fixture').stdout.strip() == 'persisted'
                message = nats('$JS.API.STREAM.MSG.GET.STORAGE_FIXTURE', {'last_by_subj': 'storage.fixture'})
                assert json.loads(base64.b64decode(message['message']['data'])) == {'persisted': True}
                for key, value in original_settings.items():
                    if key != 'FERRY_STORAGE_LAYOUT':
                        assert settings()[key] == value, key
                if settings().get('FERRY_STORAGE_LAYOUT') == 'directory':
                    for item, name, value in [('caddy-data', 'certificate', 'certificate-fixture'),
                                               ('caddy-config', 'configuration', 'configuration-fixture')]:
                        assert run(docker, 'run', '--rm', '--network', 'none', '--mount',
                            f'type=bind,src={deployment}/data/{item},dst=/source,readonly',
                            'alpine:3.22', 'cat', f'/source/{name}.fixture').stdout == value

            verify()
            compose('down')
            # Recreate an old deployment from its actual persisted bytes, including
            # database UID/mode/symlinks; no artificial empty-volume migration.
            items = ('ferry', engine, 'nats', 'redis', 'caddy-data', 'caddy-config')
            for item in items:
                suffix = item if item.startswith('caddy-') else item + '-data'
                volume = f'{project}_{suffix}'
                run(docker, 'volume', 'create', '--label', f'com.docker.compose.project={project}',
                    '--label', f'com.docker.compose.volume={suffix}', volume)
                run(docker, 'run', '--rm', '--network', 'none', '--mount',
                    f'type=bind,src={deployment}/data/{item},dst=/source,readonly', '--mount',
                    f'type=volume,src={volume},dst=/target', 'alpine:3.22', 'sh', '-ec', 'cp -a /source/. /target/')
            run('sudo', 'mv', str(deployment / 'data'), str(root / 'initial-directories'))
            set_value('FERRY_STORAGE_LAYOUT', None)
            quick()
            verify()
            assert 'ferry-data:/data' in (deployment / 'compose.storage.yaml').read_text()
            print('PASS legacy rerun retains named volumes and all data', flush=True)

            failure = quick('--migrate-storage', failure='copy', check=False)
            assert failure.returncode and 'Injected copy failure' in failure.stdout, failure.stdout[-1800:]
            assert settings().get('FERRY_STORAGE_LAYOUT') is None
            assert not (deployment / 'data/ferry').exists()
            verify()
            print('PASS failed copy restores original service and leaves source volumes intact', flush=True)
            # New backup directory must have a unique name even on quick reruns.
            failure = quick('--migrate-storage', failure='cutover', check=False)
            assert failure.returncode and settings()['FERRY_STORAGE_LAYOUT'] == 'directory'
            assert 'Do NOT restore stale volumes' in failure.stdout
            quick()
            verify()
            assert 'already uses' in quick('--migrate-storage').stdout
            for item in items:
                suffix = item if item.startswith('caddy-') else item + '-data'
                run(docker, 'volume', 'inspect', f'{project}_{suffix}')
            print('PASS cutover recovery, secret decryption, IDs, SQL, NATS, Redis, Caddy files and retained source volumes', flush=True)

            compose('down')
            archive = root / 'deployment.tar'
            run('sudo', 'tar', '-cpf', str(archive), '-C', str(root), 'ferry')
            relocated = root / 'relocated'
            relocated.mkdir()
            run('sudo', 'tar', '-xpf', str(archive), '-C', str(relocated))
            deployment = relocated / 'ferry'
            quick()
            verify()
            print(f'PASS {engine}: complete tar relocation and healthy restart with original secrets and facts', flush=True)
        finally:
            if (deployment / 'compose.storage.yaml').exists():
                compose('down', check=False)
            # Only this test's unique project and verified temporary directory.
            volumes = run(docker, 'volume', 'ls', '-q', '--filter', f'label=com.docker.compose.project={project}').stdout.split()
            if volumes:
                run(docker, 'volume', 'rm', *volumes, check=False)
            assert root.parent == Path(tempfile.gettempdir()) and root.name.startswith('ferry-storage-')
            run('sudo', 'rm', '-rf', '--', str(root), cwd=repo)


if __name__ == '__main__':
    try:
        main()
    except Exception:
        detail = traceback.format_exc()
        print('::error::' + detail.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A'), flush=True)
        raise
