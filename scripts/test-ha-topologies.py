#!/usr/bin/env python3
"""Protocol-specific HA experiments; no production deployments or user fixtures."""
import asyncio
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request
import uuid

IMAGES = {
    'redis': 'redis@sha256:02419de7eddf55aa5bcf49efb74e88fa8d931b4d77c07eff8a6b2144472b6952',
    'nats': 'nats@sha256:d69eb29526c1d98afdfb2e2434763bef77b5f3c83e2e24769c13a4d104be475e',
    'nginx': 'nginx@sha256:4ff102c5d78d254a6f0da062b3cf39eaf07f01eec0927fd21e219d0af8bc0591',
}


class Lab:
    def __init__(self, kind):
        assert os.environ.get('FERRY_HA_LAB') == '1' and os.name == 'posix', 'isolated Linux laboratory required'
        self.kind = kind
        self.prefix = 'fha-' + uuid.uuid4().hex[:10]
        self.output = Path(os.environ.get('FERRY_HA_OUTPUT', 'ha-evidence')).resolve()
        self.output.mkdir(parents=True, exist_ok=True)
        self.containers, self.volumes = [], []
        self.report = {'result': 'RUNNING', 'kind': kind, 'prefix': self.prefix,
                       'source': os.environ.get('GITHUB_SHA', 'local'), 'checks': [], 'observations': {},
                       'boundary': 'Direct Docker protocol experiment, one host; not Ferry deployment-driver or production HA certification',
                       'uncovered': ['physical-host faults', 'production TLS/auth', 'NAT/remote client discovery',
                                     'Ferry API/SSH/Agent integration', 'backup restore', 'cross-host volume migration']}

    def command(self, *args, check=True, timeout=90):
        result = subprocess.run(['docker', *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=timeout)
        if check and result.returncode:
            raise RuntimeError(result.stderr[-4000:])
        return result

    def check(self, name, condition, detail=None):
        self.report['checks'].append({'name': name, 'passed': bool(condition), 'detail': detail})
        assert condition, name
        print('PASS ' + name, flush=True)

    def eventually(self, fn, label, timeout=90):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                last = fn()
                if last:
                    return last
            except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
                last = str(error)
            time.sleep(.5)
        raise AssertionError(label + ' timed out: ' + str(last)[-1000:])

    def name(self, node):
        return self.prefix + '-' + node

    def start(self, node, image, args, port=None, env=None, data=False, shell=False):
        name = self.name(node)
        options = ['run', '-d', '--name', name, '--network', self.prefix]
        if port:
            options += ['-p', f'127.0.0.1::{port}']
        if data:
            volume = self.name(node) + '-data'
            self.command('volume', 'create', volume)
            self.volumes.append(volume)
            options += ['-v', volume + ':/data']
        for key, value in (env or {}).items():
            options += ['-e', key + '=' + value]
        if shell:
            options += ['--entrypoint', 'sh']
        self.command(*options, IMAGES[image], *args, timeout=180)
        self.containers.append(name)

    def ip(self, node):
        return json.loads(self.command('inspect', self.name(node)).stdout)[0]['NetworkSettings']['Networks'][self.prefix]['IPAddress']

    def port(self, node, port):
        return int(json.loads(self.command('inspect', self.name(node)).stdout)[0]['NetworkSettings']['Ports'][f'{port}/tcp'][0]['HostPort'])

    def finish(self, error=None):
        self.report['result'] = 'FAIL' if error else 'PASS'
        if error:
            self.report['error'] = str(error)
        for name in self.containers:
            result = self.command('logs', '--tail', '150', name, check=False, timeout=20)
            (self.output / (name + '.log')).write_text(result.stdout + result.stderr, encoding='utf8')
        (self.output / 'acceptance.json').write_text(json.dumps(self.report, indent=2), encoding='utf8')
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            message = json.dumps(self.report, separators=(',', ':')).replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print('::notice title=HA topology evidence::' + message, flush=True)
            if error:
                print('::error title=HA topology failure::' + str(error).replace('%', '%25').replace('\n', '%0A'), flush=True)
        for name in reversed(self.containers):
            self.command('rm', '-f', name, check=False)
        for volume in self.volumes:
            self.command('volume', 'rm', volume, check=False)
        self.command('network', 'rm', self.prefix, check=False)


def redis_sentinel(lab):
    def cli(node, *args, address=None):
        target = ['-h', address, '-p', '6379'] if address else ['-p', '26379' if node.startswith('s') else '6379']
        result = lab.command('exec', lab.name(node), 'redis-cli', '--raw', *target, *args, check=False, timeout=15)
        return (result.stdout + result.stderr).strip()

    lab.start('r0', 'redis', ['redis-server', '--appendonly', 'yes', '--save', '', '--protected-mode', 'no'], data=True)
    lab.eventually(lambda: cli('r0', 'PING') == 'PONG', 'Redis initial master')
    original = lab.ip('r0')
    for node in ('r1', 'r2'):
        lab.start(node, 'redis', ['redis-server', '--appendonly', 'yes', '--save', '', '--protected-mode', 'no',
                                 '--replicaof', original, '6379'], data=True)
    for node in ('s0', 's1', 's2'):
        config = f'port 26379\nbind 0.0.0.0\nprotected-mode no\nsentinel monitor proof {original} 6379 2\nsentinel down-after-milliseconds proof 1000\nsentinel failover-timeout proof 10000\nsentinel parallel-syncs proof 1\n'
        lab.start(node, 'redis', ['-c', 'printf \'%s\' "$LAB_CONFIG" > /tmp/sentinel.conf; exec redis-server /tmp/sentinel.conf --sentinel'],
                  env={'LAB_CONFIG': config}, shell=True)
    lab.eventually(lambda: 'OK' in cli('s0', 'SENTINEL', 'CKQUORUM', 'proof'), 'Sentinel discovers quorum')
    lab.check('Sentinel topology uses explicit reachable IP and three independent voters', all(
        cli(node, 'SENTINEL', 'get-master-addr-by-name', 'proof').splitlines() == [original, '6379'] for node in ('s0', 's1', 's2')))
    lab.check('initial master accepts write', cli('r0', 'SET', 'proof', 'before') == 'OK')
    lab.eventually(lambda: all(cli(node, 'GET', 'proof') == 'before' for node in ('r1', 'r2')), 'both replicas receive baseline')
    lab.command('stop', '-t', '10', lab.name('r0'))

    def discovered():
        addresses = [cli(node, 'SENTINEL', 'get-master-addr-by-name', 'proof').splitlines() for node in ('s0', 's1', 's2')]
        if all(len(value) == 2 for value in addresses) and len({value[0] for value in addresses}) == 1 and addresses[0][0] != original:
            candidate = addresses[0][0]
            if cli('s0', 'ROLE', address=candidate).splitlines()[0] == 'master':
                return candidate
        return None
    promoted = lab.eventually(discovered, 'majority agrees on promoted Redis master', timeout=90)
    lab.check('Sentinel resolves a new writable master with replicated baseline', cli('s0', 'GET', 'proof', address=promoted) == 'before'
              and cli('s0', 'SET', 'proof', 'after', address=promoted) == 'OK')
    lab.command('start', lab.name('r0'))
    lab.eventually(lambda: cli('r0', 'ROLE').splitlines()[0] == 'slave' and cli('r0', 'GET', 'proof') == 'after',
                   'old master rejoins as replica', timeout=90)
    lab.check('old master is demoted and converges to current data', cli('r0', 'ROLE').splitlines()[0] == 'slave')
    for node in ('s1', 's2'):
        lab.command('stop', '-t', '10', lab.name(node))
    lab.eventually(lambda: 'NOQUORUM' in cli('s0', 'SENTINEL', 'CKQUORUM', 'proof'), 'single Sentinel cannot authorize failover', timeout=30)
    lab.check('lost Sentinel quorum is reported without claiming a new leader', 'NOQUORUM' in cli('s0', 'SENTINEL', 'CKQUORUM', 'proof'))
    lab.report['observations'] = {'original_master': original, 'promoted_master': promoted, 'replication': 'asynchronous; this test does not establish RPO=0'}


async def nats_jetstream(lab):
    import nats
    from nats.js.api import StreamConfig, StorageType, ConsumerConfig, AckPolicy
    routes = ','.join('nats://' + lab.name(node) + ':6222' for node in ('n0', 'n1', 'n2'))
    for node in ('n0', 'n1', 'n2'):
        lab.start(node, 'nats', ['--name', lab.name(node), '--jetstream', '--store_dir', '/data',
                                 '--cluster_name', lab.prefix, '--cluster', 'nats://0.0.0.0:6222', '--routes', routes], port=4222, data=True)
    urls = ['nats://127.0.0.1:' + str(lab.port(node, 4222)) for node in ('n0', 'n1', 'n2')]
    async def retry(fn, label, timeout=90):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                result = await fn()
                if result:
                    return result
            except Exception as error:
                last = str(error)
            await asyncio.sleep(.5)
        raise AssertionError(label + ': ' + str(last))
    nc = await retry(lambda: nats.connect(servers=urls, connect_timeout=2, max_reconnect_attempts=100, reconnect_time_wait=.2), 'NATS endpoints')
    try:
        js = nc.jetstream(timeout=5)
        await retry(lambda: js.add_stream(config=StreamConfig(name='PROOF', subjects=['proof'], storage=StorageType.FILE, num_replicas=3)), 'create replicated JetStream stream')
        async def replicas_current():
            info = await js.stream_info('PROOF')
            return info if info.cluster and info.cluster.replicas and len(info.cluster.replicas) == 2 and all(member.current for member in info.cluster.replicas) else None
        info = await retry(replicas_current, 'three current JetStream replicas')
        lab.check('JetStream uses three replicas with DNS routes and explicit client endpoints', info.config.num_replicas == 3)
        consumer = await js.pull_subscribe('proof', durable='proof-consumer', stream='PROOF',
                                          config=ConsumerConfig(ack_policy=AckPolicy.EXPLICIT,
                                                                ack_wait=2, num_replicas=3))
        await js.publish('proof', b'before', headers={'Nats-Msg-Id': 'before'})
        first = (await consumer.fetch(1, timeout=5))[0]
        lab.check('durable consumer receives first message', first.data == b'before')
        await first.ack_sync(timeout=5)
        await js.publish('proof', b'pending', headers={'Nats-Msg-Id': 'pending'})
        pending = (await consumer.fetch(1, timeout=5))[0]
        lab.check('second message is pending before leader failure', pending.data == b'pending')
        previous = (await js.stream_info('PROOF')).cluster.leader
        lab.command('stop', '-t', '10', previous)
        async def leader_changed():
            current = await js.stream_info('PROOF')
            return current if current.cluster.leader and current.cluster.leader != previous else None
        changed = await retry(leader_changed, 'JetStream leader election')
        await retry(lambda: js.publish('proof', b'after', headers={'Nats-Msg-Id': 'after'}), 'publish after leader failure')
        lab.check('JetStream leader changes and committed messages survive', (await js.get_msg('PROOF', seq=1)).data == b'before'
                  and (await js.get_msg('PROOF', seq=3)).data == b'after')
        await consumer.unsubscribe()
        consumer = await retry(lambda: js.pull_subscribe('proof', durable='proof-consumer', stream='PROOF'),
                               'rebind existing durable consumer after leader failure')
        redelivered = (await retry(lambda: consumer.fetch(1, timeout=3), 'pending message redelivery'))[0]
        lab.check('durable consumer retains ack floor and redelivers unacknowledged message',
                  redelivered.data == b'pending' and redelivered.metadata.sequence.stream == 2
                  and redelivered.metadata.num_delivered >= 2)
        await redelivered.ack_sync(timeout=5)
        following = (await consumer.fetch(1, timeout=5))[0]
        lab.check('durable consumer continues with next message without replaying acknowledged data',
                  following.data == b'after' and following.metadata.sequence.stream == 3)
        await following.ack_sync(timeout=5)
        lab.command('start', previous)
        await retry(replicas_current, 'returning replica catches up')
        info = await js.stream_info('PROOF')
        lab.check('returning JetStream member restores three current replicas', info.config.num_replicas == 3 and all(member.current for member in info.cluster.replicas))
        async def consumer_caught_up():
            state = await js.consumer_info('PROOF', 'proof-consumer')
            return state if (state.ack_floor.stream_seq == 3 and state.num_ack_pending == 0
                             and state.cluster and state.cluster.replicas
                             and len(state.cluster.replicas) == 2
                             and all(member.current for member in state.cluster.replicas)) else None
        state = await retry(consumer_caught_up, 'consumer replicas and committed ack floor recover')
        lab.check('three durable consumer replicas converge with no outstanding acknowledgements',
                  state.config.num_replicas == 3 and state.ack_floor.stream_seq == 3)
        lab.report['observations'] = {'previous_leader': previous, 'new_leader': changed.cluster.leader, 'messages': info.state.messages,
                                       'scope': 'stream and durable explicit-ack consumer recovery; application exactly-once behavior not certified'}
    finally:
        await nc.close()


def nginx_stateless(lab):
    def config(body):
        return 'worker_processes 1; events { worker_connections 128; } http { ' + body + ' }'
    command = ['-c', 'printf \'%s\' "$LAB_CONFIG" > /etc/nginx/nginx.conf; exec nginx -g "daemon off;"']
    for node in ('b0', 'b1'):
        lab.start(node, 'nginx', command, shell=True, env={'LAB_CONFIG': config(
            'server { listen 80; location / { default_type text/plain; return 200 "' + node + '"; } }')})
    upstream = 'upstream proof { server ' + lab.name('b0') + ':80 max_fails=1 fail_timeout=2s; server ' + lab.name('b1') + ':80 max_fails=1 fail_timeout=2s; }'
    front_config = config(upstream + ' server { listen 80; add_header X-Lab-Version v1; location / { proxy_connect_timeout 1s; proxy_read_timeout 2s; proxy_next_upstream error timeout http_502 http_503 http_504; proxy_pass http://proof; } }')
    for node in ('f0', 'f1'):
        lab.start(node, 'nginx', command, port=80, shell=True, env={'LAB_CONFIG': front_config})
    urls = {node: 'http://127.0.0.1:' + str(lab.port(node, 80)) for node in ('f0', 'f1')}
    def get(node):
        with urllib.request.urlopen(urls[node], timeout=4) as response:
            return response.read().decode()
    for node in ('f0', 'f1'):
        lab.eventually(lambda node=node: get(node) in ('b0', 'b1'), 'frontend startup')
        lab.check(node + ' balances across service-name upstreams', {get(node) for _ in range(8)} == {'b0', 'b1'})
    lab.command('stop', '-t', '10', lab.name('b0'))
    for node in ('f0', 'f1'):
        lab.check(node + ' switches away from failed backend', all(get(node) == 'b1' for _ in range(4)))
    invalid = lab.command('exec', lab.name('f1'), 'sh', '-c',
                          'printf \'invalid config\' > /tmp/invalid.conf; nginx -t -c /tmp/invalid.conf', check=False)
    lab.check('invalid replacement configuration is rejected while current service remains live', invalid.returncode != 0 and get('f1') == 'b1')
    lab.command('stop', '-t', '10', lab.name('f0'))
    lab.check('second independent Nginx frontend remains available', get('f1') == 'b1')
    lab.report['observations'] = {'backend_addressing': 'Docker service DNS', 'client_addressing': 'explicit loopback published ports',
                                   'entrypoint_ha': 'NOT_VERIFIED; a shared external LB/VIP is not provided by two Nginx replicas'}


def main():
    kind = sys.argv[1]
    assert kind in ('redis-sentinel', 'nats-jetstream', 'nginx')
    lab = Lab(kind)
    error = None
    try:
        lab.command('network', 'create', lab.prefix)
        if kind == 'redis-sentinel':
            redis_sentinel(lab)
        elif kind == 'nats-jetstream':
            asyncio.run(nats_jetstream(lab))
        else:
            nginx_stateless(lab)
    except Exception as caught:
        error = caught
        raise
    finally:
        lab.finish(error)


if __name__ == '__main__':
    main()
