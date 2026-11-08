#!/usr/bin/env python3
"""Disposable Linux SQL/S3 dependencies for the real Ferry cluster harness.

Requires already built server/Runner/Actions worker and native nats-server.
Only containers carrying this run's ownership label can be stopped or started.
"""
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid


def docker(*args, check=True, timeout=180):
    result = subprocess.run(['docker', *args], capture_output=True, text=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError('Docker operation failed: ' + result.stderr[-1500:])
    return result.stdout.strip()


def until(check, label, seconds=120):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            if check():
                return
        except (OSError, RuntimeError, ValueError):
            pass
        time.sleep(.5)
    raise AssertionError(label + ' timed out')


def free_port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def ready(state, component):
    name = state['containers'][component]
    if component == 'database':
        if state['engine'] == 'postgres':
            command = ['pg_isready', '-h', '127.0.0.1', '-U', 'postgres', '-d', 'ferry_ha']
        else:
            # TCP excludes the official image's temporary initialization server.
            command = ['mysqladmin', '--protocol=tcp', '-h127.0.0.1', '-uroot', 'ping']
        return subprocess.run(['docker', 'exec', name, *command], capture_output=True, timeout=10).returncode == 0
    try:
        with urllib.request.urlopen(state['s3_endpoint'] + '/' + state['bucket'], timeout=2) as response:
            return response.status == 200
    except urllib.error.HTTPError as error:
        return error.code == 403


def control(path, component, operation):
    assert component in ('database', 's3') and operation in ('stop', 'start')
    state = json.loads(Path(path).read_text())
    name = state['containers'][component]
    assert name.startswith(state['prefix'] + '-') and state['prefix'].startswith('ferry-deps-')
    info = json.loads(docker('inspect', name))[0]
    assert info['Config']['Labels'].get('ferry.ha.lab') == state['prefix'], 'dependency ownership mismatch'
    docker(operation, *(['-t', '10'] if operation == 'stop' else []), name)
    if operation == 'start':
        until(lambda: ready(state, component), component + ' recovery')
    print('DEPENDENCY', component, operation, 'PASS', flush=True)


def main():
    assert os.name == 'posix' and sys.platform == 'linux' and os.environ.get('FERRY_HA_LAB') == '1', 'explicit disposable Linux lab required'
    engine = sys.argv[1]
    assert engine in ('postgres', 'mysql')
    repo = Path(__file__).resolve().parents[1]
    work = Path(os.environ['FERRY_HA_OUTPUT']).resolve()
    work.mkdir(parents=True, exist_ok=True)
    prefix = 'ferry-deps-' + uuid.uuid4().hex[:10]
    password, access, secret = (uuid.uuid4().hex for _ in range(3))
    redactions = [password, access, secret]
    def redact(text):
        for value in redactions:
            text = text.replace(value, '[REDACTED]')
        return text
    if os.environ.get('GITHUB_ACTIONS') == 'true':
        for value in redactions:
            print('::add-mask::' + value, flush=True)
    ports = {'database': free_port(), 's3': free_port()}
    state = {'prefix': prefix, 'engine': engine, 'containers': {kind: prefix + '-' + kind for kind in ports},
             's3_endpoint': 'http://127.0.0.1:' + str(ports['s3']), 'bucket': prefix}
    state_file = work / 'dependency-control.json'
    state_file.write_text(json.dumps(state), encoding='utf8')
    images = {'database': 'postgres:16.8-alpine' if engine == 'postgres' else 'mysql@sha256:7dcddc01f13bab2f15cde676d44d01f61fc9f99fe7785e86196dfc07d358ae2b',
              's3': 'chrislusf/seaweedfs:4.52'}
    report = {'source': os.environ.get('GITHUB_SHA', 'local'), 'result': 'RUNNING', 'engine': engine,
              'boundary': 'same-host native Ferry/NATS processes + disposable SQL/S3 containers; dependency restart, not SQL/S3 HA or physical-host certification',
              'images': {}, 'checks': []}
    created, volumes, child = [], [], None
    result_code = 1
    try:
        for kind, image in images.items():
            docker('pull', image, timeout=300)
            info = json.loads(docker('image', 'inspect', image))[0]
            digest = info['RepoDigests'][0]
            report['images'][kind] = digest
            volume = state['containers'][kind] + '-data'
            docker('volume', 'create', '--label', 'ferry.ha.lab=' + prefix, volume)
            volumes.append(volume)
            options = ['run', '-d', '--name', state['containers'][kind], '--label', 'ferry.ha.lab=' + prefix]
            if kind == 'database':
                internal = 5432 if engine == 'postgres' else 3306
                options += ['-p', f'127.0.0.1:{ports[kind]}:{internal}']
                if engine == 'postgres':
                    options += ['-v', volume + ':/var/lib/postgresql/data', '-e', 'POSTGRES_DB=ferry_ha', '-e', 'POSTGRES_PASSWORD=' + password]
                    args = []
                else:
                    options += ['-v', volume + ':/var/lib/mysql', '-e', 'MYSQL_DATABASE=ferry_ha', '-e', 'MYSQL_ROOT_HOST=%', '-e', 'MYSQL_ROOT_PASSWORD=' + password]
                    args = ['--max-connections=160', '--innodb-buffer-pool-size=128M']
            else:
                options += ['-p', f'127.0.0.1:{ports[kind]}:8333', '-v', volume + ':/data', '-e', 'S3_ACCESS_KEY=' + access, '-e', 'S3_SECRET_KEY=' + secret]
                args = ['mini', '-dir=/data', '-ip=127.0.0.1', '-ip.bind=0.0.0.0', '-s3.port=8333', '-bucket=' + state['bucket'], '-admin.ui=false', '-master.volumeSizeLimitMB=32', '-volume.max=2']
            # Track names before launch so even a partially successful CLI call
            # can be cleaned up, without enumerating unrelated containers.
            created.append(state['containers'][kind])
            docker(*options, digest, *args)
            until(lambda kind=kind: ready(state, kind), kind + ' initial readiness', 180)
        environment = dict(os.environ,
            FERRY_TEST_DATABASE_URL=f'{engine}://{("postgres" if engine == "postgres" else "root")}:{password}@127.0.0.1:{ports["database"]}/ferry_ha',
            FERRY_TEST_NATS_NODES='3', FERRY_TEST_CLUSTER_EXECUTIONS='1', FERRY_TEST_CLUSTER_DEPENDENCIES='1',
            FERRY_TEST_DEPENDENCY_CONTROL=json.dumps([sys.executable, str(Path(__file__).resolve()), '--control', str(state_file)]),
            FERRY_ARTIFACT_STORE='s3', FERRY_ARTIFACT_S3_ENDPOINT=state['s3_endpoint'], FERRY_ARTIFACT_S3_BUCKET=state['bucket'],
            FERRY_ARTIFACT_S3_REGION='us-east-1', AWS_ACCESS_KEY_ID=access, AWS_SECRET_ACCESS_KEY=secret,
            AWS_EC2_METADATA_DISABLED='true', KEEP_CLUSTER_TEST_DATA='1')
        report['binaries'] = {}
        for key in ('FERRY_BINARY', 'RUNNER_BINARY', 'NATS_SERVER_BINARY'):
            binary = Path(environment[key]).resolve(strict=True)
            report['binaries'][key] = hashlib.sha256(binary.read_bytes()).hexdigest()
        worker = Path(environment['ACTIONS_WORKER_DIR']) / 'actions-worker-linux-amd64'
        report['binaries']['ACTIONS_WORKER'] = hashlib.sha256(worker.read_bytes()).hexdigest()
        report['script_sha256'] = hashlib.sha256((repo / 'scripts/test-cluster.mjs').read_bytes()).hexdigest()
        with (work / 'cluster.log').open('wb') as log:
            child = subprocess.Popen(['node', 'scripts/test-cluster.mjs'], cwd=repo, env=environment,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            result_code = child.wait(timeout=900)
        text = (work / 'cluster.log').read_text(errors='replace')
        report['checks'] = [line for line in text.splitlines() if line.startswith('PASS ')]
        report['result'] = 'PASS' if result_code == 0 else 'FAIL'
        if result_code:
            report['error'] = redact(text[-2200:])
    except Exception as error:
        report['result'], report['error'] = 'FAIL', redact(str(error))
        raise
    finally:
        if child:
            # All native children and browser helpers belong to this new group.
            try:
                os.killpg(child.pid, signal.SIGTERM)
                time.sleep(1)
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for name in reversed(created):
            log_result = subprocess.run(['docker', 'logs', '--tail', '200', name], capture_output=True, text=True, timeout=30)
            logs = log_result.stdout + log_result.stderr
            (work / (name + '.log')).write_text(redact(logs), encoding='utf8')
            docker('rm', '-f', name, check=False)
        for volume in volumes:
            docker('volume', 'rm', volume, check=False)
        log_file = work / 'cluster.log'
        if log_file.exists():
            raw = log_file.read_text(errors='replace')
            log_file.write_text(redact(raw), encoding='utf8')
            # Collect only the harness's own preserved directory, never host data.
            import re
            matches = re.findall(r'Cluster acceptance evidence: (.+)', raw)
            if matches:
                evidence = Path(matches[-1].strip()).resolve()
                if evidence.name.startswith('cluster-acceptance-') and evidence.parent == Path('/tmp'):
                    destination = work / 'native-logs'
                    destination.mkdir(exist_ok=True)
                    for file in evidence.glob('*.log'):
                        (destination / file.name).write_text(redact(file.read_text(errors='replace')), encoding='utf8')
        (work / 'acceptance.json').write_text(json.dumps(report, indent=2), encoding='utf8')
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            # Check annotations truncate at 4096 bytes. Keep the complete report
            # in the artifact and an independently parseable failure summary.
            summary = dict(report, full_report='acceptance.json')
            if summary.get('error'):
                summary['error'] = summary['error'][-600:]
            for key in ('binaries', 'images', 'error'):
                if len(json.dumps(summary, separators=(',', ':')).encode()) <= 3900:
                    break
                summary.pop(key, None)
            annotation = json.dumps(summary, separators=(',', ':')).replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print('::notice title=Ferry dependency evidence::' + annotation, flush=True)
        print(report['result'], engine, len(report['checks']), 'checks;', work, flush=True)
    return result_code


if __name__ == '__main__':
    if len(sys.argv) == 5 and sys.argv[1] == '--control':
        control(*sys.argv[2:])
    else:
        raise SystemExit(main())
