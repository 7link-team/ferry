#!/usr/bin/env python3
"""Owned PXC/Swarm laboratory. Never initializes the caller's Docker Swarm."""
import concurrent.futures
import json
import os
from pathlib import Path
import secrets
import subprocess
import time
import uuid

# Each pinned image is qualified independently. The previous 8.4.11 candidate
# failed SST; its failed runs remain evidence, not support for this candidate.
IMAGE = 'percona/percona-xtradb-cluster@sha256:ca15bf7f3b53464a546b57a2a2a3e4cd67cedc6ad25c079e1461128465b649ff'
ENGINE = 'docker:27.5.1-dind'
NODES = ('a', 'b', 'c')
STATUS_SQL = "SHOW GLOBAL STATUS WHERE Variable_name IN ('wsrep_cluster_size','wsrep_cluster_status','wsrep_connected','wsrep_ready','wsrep_local_state_comment','wsrep_cluster_state_uuid')"


def main():
    if os.environ.get('FERRY_PXC_LAB') != '1':
        raise SystemExit('Set FERRY_PXC_LAB=1 only on an isolated laboratory Docker host')
    output = Path(os.environ.get('FERRY_PXC_OUTPUT', 'pxc-evidence')).resolve()
    output.mkdir(parents=True, exist_ok=True)
    prefix = 'fpxc-' + uuid.uuid4().hex[:10]
    password, sst_password = secrets.token_hex(24), secrets.token_hex(24)
    if os.environ.get('GITHUB_ACTIONS') == 'true':
        print('::add-mask::' + password, flush=True)
        print('::add-mask::' + sst_password, flush=True)
    report = {'result': 'RUNNING', 'source': os.environ.get('GITHUB_SHA', 'local'),
              'prefix': prefix, 'image': IMAGE, 'engine': ENGINE, 'checks': [], 'snapshots': [],
              'boundary': 'Three nested daemons on one shared kernel; direct Swarm experiment, not Ferry API or physical-host certification',
              'uncovered': ['production TLS', 'proxy routing/HA', 'Ferry API/SSH/Agent integration',
                            'backup restore', 'rolling version upgrade', 'physical-host quorum', 'cross-host volume migration']}
    created, node_ids, ips = [], {}, {}
    expected_rows = 0

    def redact(value):
        return value.replace(password, '[REDACTED]').replace(sst_password, '[REDACTED]')

    def run(args, check=True, timeout=120):
        result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=timeout)
        if check and result.returncode:
            raise RuntimeError(redact(f'{args[0]} failed ({result.returncode}): {result.stderr[-5000:]}'))
        return result

    def docker(*args, **kw):
        return run(['docker', *args], **kw)

    def engine(node, *args, **kw):
        return docker('exec', prefix + '-' + node, 'docker', *args, **kw)

    def manager(*args, **kw):
        return engine('a', *args, **kw)

    def check(name, condition, detail=None):
        report['checks'].append({'name': name, 'passed': bool(condition), 'detail': detail})
        assert condition, name
        print('PASS ' + name, flush=True)

    def eventually(fn, label, timeout=240):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                last = fn()
                if last:
                    return last
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                last = redact(str(error))
            time.sleep(2)
        raise AssertionError(f'{label} timed out: {str(last)[-1500:]}')

    def task(node):
        result = engine(node, 'ps', '-q', '--filter', 'label=com.docker.swarm.service.name=' + prefix + '-db-' + node,
                        check=False, timeout=15)
        ids = result.stdout.split()
        return ids[0] if result.returncode == 0 and len(ids) == 1 else None

    def sql(node, query):
        container = task(node)
        if not container:
            return None
        result = engine(node, 'exec', '-e', 'MYSQL_PWD=' + password, container, 'mysql',
                        '--protocol=socket', '-uroot', '--batch', '--raw', '--skip-column-names',
                        '--socket=/tmp/mysql.sock', '--connect-timeout=5', '-e', query, check=False, timeout=20)
        return result.stdout.strip() if result.returncode == 0 else None

    def status(node):
        raw = sql(node, STATUS_SQL)
        return dict(line.split('\t', 1) for line in raw.splitlines() if '\t' in line) if raw else {}

    def ready(node, size):
        state = status(node)
        return (state.get('wsrep_cluster_size') == str(size) and state.get('wsrep_cluster_status') == 'Primary'
                and state.get('wsrep_connected') == 'ON' and state.get('wsrep_ready') == 'ON'
                and state.get('wsrep_local_state_comment') == 'Synced')

    def cluster(size=3, nodes=NODES):
        return all(ready(node, size) for node in nodes)

    def snapshot(stage):
        states = {}
        for node in NODES:
            try:
                states[node] = {'task': task(node), 'wsrep': status(node)}
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                states[node] = {'unavailable': redact(str(error))[-500:]}
        report['snapshots'].append({'stage': stage, 'time': time.time(), 'states': states})

    def write(node):
        nonlocal expected_rows
        value = expected_rows + 1
        query = f"INSERT INTO ferry_pxc.checks VALUES ({value},'from-{node}-{value}') ON DUPLICATE KEY UPDATE payload='from-{node}-{value}'; SELECT 'written'"
        started = time.monotonic()
        result = eventually(lambda: sql(node, query) == 'written', 'write availability on ' + node, timeout=90)
        check(f'write {value} accepted on {node}', result, {'seconds': round(time.monotonic() - started, 3)})
        expected_rows = value

    def data_consistent(nodes=NODES):
        query = "SET SESSION wsrep_sync_wait=1; SELECT COUNT(*),SHA2(GROUP_CONCAT(CONCAT(id,':',payload) ORDER BY id SEPARATOR '|'),256) FROM ferry_pxc.checks"
        results = {node: sql(node, query) for node in nodes}
        return (all(value and value.startswith(str(expected_rows) + '\t') for value in results.values())
                and len(set(results.values())) == 1)

    def service(node, join):
        args = ['service', 'create', '--detach=true', '--name', prefix + '-db-' + node,
                '--hostname', prefix + '-db-' + node, '--replicas', '1', '--endpoint-mode', 'dnsrr',
                '--constraint', 'node.labels.pxc.slot==' + node, '--network', prefix + '-cluster',
                '--mount', 'type=volume,source=' + prefix + '-data-' + node + ',target=/var/lib/mysql',
                '--env', 'MYSQL_ROOT_PASSWORD=' + password, '--env', 'MYSQL_ROOT_HOST=localhost',
                '--env', 'XTRABACKUP_PASSWORD=' + sst_password, '--env', 'CLUSTER_NAME=' + prefix,
                '--env', 'PERCONA_TELEMETRY_DISABLE=1',
                '--health-cmd', 'MYSQL_PWD="$MYSQL_ROOT_PASSWORD" mysqladmin --protocol=socket --socket=/tmp/mysql.sock -uroot --silent ping',
                '--health-interval', '10s', '--health-timeout', '5s', '--health-retries', '6',
                '--health-start-period', '180s', '--restart-delay', '10s', '--stop-grace-period', '60s',
                '--limit-memory', '2G']
        if join:
            args += ['--env', 'CLUSTER_JOIN=' + join]
        manager(*args, IMAGE, 'mysqld', '--defaults-file=/etc/mysql/node.cnf',
                '--pxc-encrypt-cluster-traffic=OFF',
                '--innodb-buffer-pool-size=128M', '--max-connections=40',
                '--log-error=/var/lib/mysql/lab-error.log', timeout=120)

    def diagnostics():
        for node in NODES:
            if node not in created:
                continue
            result = docker('logs', '--tail', '150', prefix + '-' + node, check=False, timeout=20)
            (output / f'engine-{node}.log').write_text(redact(result.stdout + result.stderr))
            result = manager('service', 'logs', '--raw', '--tail', '200', prefix + '-db-' + node,
                             check=False, timeout=30)
            (output / f'pxc-{node}.log').write_text(redact(result.stdout + result.stderr))
            result = manager('service', 'ps', '--no-trunc', prefix + '-db-' + node, check=False, timeout=15)
            (output / f'tasks-{node}.log').write_text(redact(result.stdout + result.stderr))
            # Server startup errors are written inside the data volume, not always
            # to service stdout. Read only this laboratory's existing volume.
            volume = prefix + '-data-' + node
            if engine(node, 'volume', 'inspect', volume, check=False).returncode == 0:
                result = engine(node, 'run', '--rm', '--network', 'none', '--entrypoint', 'sh',
                                '--mount', 'type=volume,source=' + volume + ',target=/evidence,readonly',
                                IMAGE, '-c', 'tail -n 100 /evidence/lab-error.log /evidence/*.err 2>/dev/null',
                                check=False, timeout=30)
                (output / f'mysql-{node}.log').write_text(redact(result.stdout + result.stderr))

    try:
        check('Linux Docker laboratory', os.name == 'posix' and docker('info').returncode == 0)
        docker('network', 'create', prefix + '-underlay')
        for node in NODES:
            docker('volume', 'create', prefix + '-engine-' + node)
            docker('run', '-d', '--privileged', '--tmpfs', '/run', '--name', prefix + '-' + node,
                   '--network', prefix + '-underlay', '--env', 'DOCKER_TLS_CERTDIR=',
                   '--volume', prefix + '-engine-' + node + ':/var/lib/docker', ENGINE,
                   '--storage-driver=overlay2', timeout=180)
            created.append(node)
        for node in NODES:
            eventually(lambda node=node: engine(node, 'info', check=False, timeout=15).returncode == 0,
                       'Docker daemon ' + node, timeout=90)
            data = json.loads(docker('inspect', prefix + '-' + node).stdout)[0]
            ips[node] = data['NetworkSettings']['Networks'][prefix + '-underlay']['IPAddress']
        manager('swarm', 'init', '--advertise-addr', ips['a'])
        token = manager('swarm', 'join-token', '-q', 'manager').stdout.strip()
        for node in NODES[1:]:
            engine(node, 'swarm', 'join', '--token', token, ips['a'] + ':2377')
        for node in NODES:
            node_ids[node] = engine(node, 'info', '--format', '{{.Swarm.NodeID}}').stdout.strip()
            manager('node', 'update', '--label-add', 'pxc.slot=' + node, node_ids[node])
        check('PXC-01 three independent Swarm node identities', len(set(node_ids.values())) == 3)
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
            list(pool.map(lambda node: engine(node, 'pull', IMAGE, timeout=600), NODES))
        manager('network', 'create', '--driver', 'overlay', '--attachable', prefix + '-cluster')
        service('a', '')
        eventually(lambda: ready('a', 1), 'first bootstrap', timeout=300)
        service('b', prefix + '-db-a')
        eventually(lambda: cluster(2, ('a', 'b')), 'second member SST', timeout=300)
        service('c', prefix + '-db-a,' + prefix + '-db-b')
        eventually(cluster, 'third member SST', timeout=300)
        bootstrap_task = task('a')
        manager('service', 'update', '--detach=true', '--env-add',
                'CLUSTER_JOIN=' + prefix + '-db-b,' + prefix + '-db-c', prefix + '-db-a')
        eventually(lambda: task('a') not in (None, bootstrap_task) and cluster(),
                   'bootstrap member becomes ordinary joiner', timeout=300)
        initial_uuid = status('a')['wsrep_cluster_state_uuid']
        check('PXC-01 all members share a nonzero cluster UUID', initial_uuid != '00000000-0000-0000-0000-000000000000'
              and all(status(node).get('wsrep_cluster_state_uuid') == initial_uuid for node in NODES))
        for node in NODES:
            info = json.loads(engine(node, 'inspect', task(node)).stdout)[0]
            check('PXC-01 member ' + node + ' owns its data volume', any(
                mount.get('Name') == prefix + '-data-' + node and mount['Destination'] == '/var/lib/mysql'
                for mount in info['Mounts']))
        snapshot('initial-three-member-cluster')
        check('PXC-02 create replicated InnoDB table', sql('a',
              'CREATE DATABASE ferry_pxc; CREATE TABLE ferry_pxc.checks(id INT PRIMARY KEY,payload VARCHAR(100) NOT NULL) ENGINE=InnoDB; SELECT 1') == '1')
        for node in NODES:
            write(node)
        eventually(data_consistent, 'initial multiwriter consistency')
        check('PXC-02 all three members contain identical multiwriter data', data_consistent())

        before = task('b')
        engine('b', 'kill', '--signal=KILL', before)
        write('a')
        eventually(lambda: task('b') not in (None, before) and cluster(), 'crashed task replacement', timeout=300)
        eventually(data_consistent, 'crash data consistency')
        check('PXC-03 task replaced and original data preserved', task('b') != before and data_consistent())
        snapshot('task-crash-recovered')

        docker('network', 'disconnect', prefix + '-underlay', prefix + '-c')
        eventually(lambda: cluster(2, ('a', 'b')), 'majority after partition', timeout=120)
        eventually(lambda: bool(status('c')) and status('c').get('wsrep_cluster_status') != 'Primary',
                   'isolated minority loses Primary', timeout=120)
        check('PXC-04 isolated member refuses writes', sql('c', "INSERT INTO ferry_pxc.checks VALUES (9001,'must-not-commit'); SELECT 'written'") is None)
        write('a')
        check('PXC-04 majority remains writable and consistent', data_consistent(('a', 'b')))
        snapshot('network-partition')
        docker('network', 'connect', '--ip', ips['c'], prefix + '-underlay', prefix + '-c')
        eventually(cluster, 'partition heals', timeout=300)
        eventually(data_consistent, 'partition data consistency')
        check('PXC-04 healed cluster retains original UUID and data', all(
            status(node).get('wsrep_cluster_state_uuid') == initial_uuid for node in NODES) and data_consistent())
        snapshot('partition-healed')

        docker('stop', '-t', '60', prefix + '-c', timeout=90)
        eventually(lambda: cluster(2, ('a', 'b')), 'majority after node outage', timeout=120)
        eventually(lambda: json.loads(manager('node', 'inspect', node_ids['c']).stdout)[0]['Status']['State'] == 'down',
                   'scheduler observes node down', timeout=120)
        write('b')
        def pending_member():
            ids = manager('service', 'ps', '-q', '--filter', 'desired-state=running', prefix + '-db-c').stdout.split()
            tasks = json.loads(manager('inspect', *ids).stdout) if ids else []
            return tasks if any(value['Status']['State'] == 'pending' for value in tasks) else None
        tasks = eventually(pending_member, 'pinned member stays pending without its volume owner', timeout=120)
        check('PXC-05 no empty-volume migration onto surviving nodes', all(
            value.get('NodeID') not in (node_ids['a'], node_ids['b']) for value in tasks))
        snapshot('daemon-offline')
        docker('start', prefix + '-c')
        eventually(cluster, 'original node and volume return', timeout=300)
        eventually(data_consistent, 'node return data consistency')
        check('PXC-05 original-volume rejoin preserves all writes', data_consistent())
        snapshot('daemon-returned')

        manager('service', 'scale', '--detach=true', prefix + '-db-c=0')
        eventually(lambda: task('c') is None, 'stop member before volume loss', timeout=120)
        stopped = engine('c', 'ps', '-aq', '--filter', 'label=com.docker.swarm.service.name=' + prefix + '-db-c').stdout.split()
        if stopped:
            engine('c', 'rm', *stopped)
        engine('c', 'volume', 'rm', prefix + '-data-c')
        check('PXC-06 only stopped experimental member data volume removed', engine(
            'c', 'volume', 'inspect', prefix + '-data-c', check=False).returncode != 0)
        write('a')
        manager('service', 'scale', '--detach=true', prefix + '-db-c=1')
        eventually(cluster, 'fresh data-volume SST', timeout=360)
        eventually(data_consistent, 'fresh volume data consistency')
        check('PXC-06 fresh joiner restores all data from healthy cluster', data_consistent())
        snapshot('fresh-volume-rebuilt')

        specs = json.loads(manager('service', 'inspect', *(prefix + '-db-' + node for node in NODES)).stdout)
        check('PXC-07 every normal service uses peers without bootstrap flag', all(
            any(value.startswith('CLUSTER_JOIN=') and value != 'CLUSTER_JOIN=' for value in spec['Spec']['TaskTemplate']['ContainerSpec']['Env'])
            and '--wsrep-new-cluster' not in spec['Spec']['TaskTemplate']['ContainerSpec'].get('Args', []) for spec in specs))
        for node in NODES:
            manager('service', 'scale', '--detach=true', prefix + '-db-' + node + '=0')
        eventually(lambda: all(task(node) is None for node in NODES), 'all database members stopped', timeout=180)
        snapshot('full-outage')
        for node in NODES:
            manager('service', 'scale', '--detach=true', prefix + '-db-' + node + '=1')
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline and not cluster():
            time.sleep(3)
        if cluster():
            eventually(data_consistent, 'full outage data consistency')
            check('PXC-07 same-group recovery preserves UUID and complete data', all(
                status(node).get('wsrep_cluster_state_uuid') == initial_uuid for node in NODES) and data_consistent())
            report['full_outage_recovery'] = 'GALERA_SAME_GROUP_RECOVERED'
        else:
            states = {node: status(node) for node in NODES}
            check('PXC-07 no isolated Primary or forced bootstrap after full outage', all(
                value.get('wsrep_cluster_status') != 'Primary' for value in states.values()))
            report['full_outage_recovery'] = 'OPERATOR_REQUIRED; automatic full-outage recovery NOT verified'
        snapshot('full-outage-restart')
        report['result'] = 'PASS'
    except Exception as error:
        report['result'] = 'FAIL'
        report['error'] = redact(str(error))
        print('FAIL ' + report['error'], flush=True)
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            escaped = report['error'].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print('::error title=PXC laboratory failure::' + escaped, flush=True)
        raise
    finally:
        try:
            diagnostics()
        except Exception as error:
            report['diagnostic_error'] = redact(str(error))[-1000:]
        (output / 'acceptance.json').write_text(json.dumps(report, indent=2), encoding='utf8')
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            # GitHub check annotations truncate messages at 4096 characters.
            # Keep each chunk bounded and leave the complete report in artifact.
            evidence = json.dumps(report, separators=(',', ':'))
            for index, offset in enumerate(range(0, len(evidence), 3000)):
                chunk = evidence[offset:offset + 3000].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
                print(f'::notice title=PXC laboratory evidence chunk {index:03d}::' + chunk, flush=True)
            if report['result'] == 'FAIL':
                for path in sorted(output.glob('*.log')):
                    if path.name.startswith(('mysql-', 'pxc-', 'tasks-')):
                        diagnostic = path.read_text(errors='replace')[-3500:]
                        diagnostic = diagnostic.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
                        print('::notice title=PXC diagnostic ' + path.name + '::' + diagnostic, flush=True)
        (output / 'summary.md').write_text('\n'.join([
            f"Result: {report['result']}", f"Checks: {len(report['checks'])}",
            f"Full outage: {report.get('full_outage_recovery', 'NOT_REACHED')}",
            report['boundary'], 'Uncovered: ' + ', '.join(report['uncovered']),
            *[f"- {'PASS' if item['passed'] else 'FAIL'} {item['name']}" for item in report['checks']],
        ]) + '\n', encoding='utf8')
        for node in reversed(created):
            docker('rm', '-f', '-v', prefix + '-' + node, check=False, timeout=90)
            docker('volume', 'rm', prefix + '-engine-' + node, check=False, timeout=90)
        docker('network', 'rm', prefix + '-underlay', check=False, timeout=30)


if __name__ == '__main__':
    main()
