"""Sentinel protocol checks for a template deployed through Ferry's real API."""
import json
import time
import uuid


def exercise(api, create, start, done, check, command, mode, workspace, host, transport,
             deployment_workdir, templates, projects, engine, swarm_nodes, swarm_ids, fixture, work, allocate_port):
    """Use the harness-owned infrastructure; all mutations go through its API."""
    template = next(t for t in templates if json.loads(t['content'])['metadata']['name'] == 'example-redis-sentinel')
    startup = next(t for t in api(f'/workspaces/{workspace}/config-templates')['items'] if t['name'] == 'example-redis-start')
    password = '--Redis-' + uuid.uuid4().hex + ' "\\$#'
    variable = 'redis-password-' + transport
    create('variables', {'name': variable, 'scope': 'deploy', 'kind': 'secret', 'value': password})
    values = {'password': variable}
    endpoints, sentinels = {}, {}
    if mode == 'compose':
        address = json.loads(command(['docker', 'network', 'inspect', 'bridge']))[0]['IPAM']['Config'][0]['Gateway']
    for index, suffix in enumerate(['a', 'b', 'c']):
        if mode == 'stack':
            node = swarm_nodes[index]
            address = json.loads(command(['docker', 'inspect', fixture + '-' + node]))[0]['NetworkSettings']['Networks'][fixture + '-swarm']['IPAddress']
            engine(['node', 'update', '--label-add', 'ferry-redis=' + suffix, swarm_ids[node]])
            engine(['pull', json.loads(template['content'])['services'][0]['image']], node)
        # Harness port allocation checks the daemon host for Compose. Distinct
        # fixed ports on owned nested nodes avoid requiring host NAT mappings.
        if mode == 'compose':
            data_port, sentinel_port = allocate_port(), allocate_port()
        else:
            data_port, sentinel_port = 16379 + index, 26379 + index
        values.update({'ip_' + suffix: address, 'data_port_' + suffix: data_port, 'sentinel_port_' + suffix: sentinel_port})
        endpoints['redis-' + suffix] = (address, data_port)
        sentinels['sentinel-' + suffix] = (address, sentinel_port)
    generated = api(f'/workspaces/{workspace}/app-templates/{template["id"]}/renderings', {'version': 1, 'values': values})['manifest']
    sample = create('deployments', {'name': 'Redis Sentinel example ' + transport, 'mode': mode,
                    'target_kind': 'host', 'target_ref': host, 'workdir': deployment_workdir, 'manifest': generated})
    project = f'deployment-{workspace}-{sample}'
    projects.append(project)

    def containers(member):
        result = []
        for node in (swarm_nodes if mode == 'stack' else ['host']):
            prefix = ['docker', 'exec', fixture + '-' + node, '/usr/bin/docker'] if mode == 'stack' else ['docker']
            label = ('com.docker.swarm.service.name=' + project + '_' + member if mode == 'stack'
                     else 'com.docker.compose.project=' + project)
            args = prefix + ['ps', '-q', '--filter', 'label=' + label]
            if mode == 'compose':
                args += ['--filter', 'label=com.docker.compose.service=' + member]
            result += [(prefix, item) for item in command(args).decode().split()]
        return result

    def wait_for(fn, label):
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if fn():
                return
            time.sleep(.5)
        raise AssertionError(label)

    def stop(member):
        if mode == 'stack':
            engine(['service', 'scale', '--detach', project + '_' + member + '=0'])
            wait_for(lambda: not containers(member), 'Redis member did not stop')
        else:
            prefix, container = containers(member)[0]
            compose_members[member] = (prefix, container)
            command(prefix + ['stop', '-t', '5', container])

    def restore(member):
        if mode == 'stack':
            engine(['service', 'scale', '--detach', project + '_' + member + '=1'])
        else:
            prefix, container = compose_members[member]
            command(prefix + ['start', container])

    def persisted(member):
        candidates = containers(member)
        if len(candidates) != 1:
            return ''
        prefix, container = candidates[0]
        config = '/data/sentinel.conf' if member.startswith('sentinel-') else '/data/redis.conf'
        # Deliberately select role lines only; runtime files also contain auth.
        return command(prefix + ['exec', container, 'sh', '-c', "grep -E '^(replicaof |sentinel monitor )' " + config + " || :"]).decode()

    def diagnostics():
        result = []
        for node in (swarm_nodes if mode == 'stack' else ['host']):
            prefix = ['docker', 'exec', fixture + '-' + node, '/usr/bin/docker'] if mode == 'stack' else ['docker']
            label = ('com.docker.stack.namespace=' if mode == 'stack' else 'com.docker.compose.project=') + project
            for container in command(prefix + ['ps', '-aq', '--filter', 'label=' + label]).decode().split():
                result.append(command(prefix + ['inspect', '--format', '{{.Name}} {{json .State}}', container]).decode())
                result.append(command(prefix + ['logs', '--tail', '60', container]).decode())
        text = '\n'.join(result).replace(password, '[REDACTED]').replace(json.dumps(password)[1:-1], '[REDACTED]')
        (work / ('redis-' + transport + '-diagnostics.log')).write_text(text, encoding='utf8')
        print(text[-7000:], flush=True)

    compose_members = {}
    try:
        revision = start(sample)
        row = done(sample)
        check(transport + ' Redis Sentinel template deployment succeeds: ' + row['latest_revision']['output'], row['state'] == 'succeeded')
        check(transport + ' Redis secret absent from public deployment history', json.dumps(password)[1:-1] not in json.dumps(row))
        if mode == 'stack':
            for index, suffix in enumerate(['a', 'b', 'c']):
                for prefix in ['redis-', 'sentinel-']:
                    member = prefix + suffix
                    task = json.loads(engine(['service', 'ps', '--filter', 'desired-state=running', '--format', '{{json .}}', project + '_' + member]).decode().splitlines()[0])
                    check(transport + ' ' + member + ' placed on assigned node', task['Node'] == engine(['info', '--format', '{{.Name}}'], swarm_nodes[index]).decode().strip())
        protocol = verify(endpoints, sentinels, password,
                          lambda label, result: check(transport + ' ' + label, result), stop, restore, persisted)
    except Exception:
        diagnostics()
        raise
    if mode == 'stack':
        engine(['stack', 'rm', project])
        wait_for(lambda: not engine(['service', 'ls', '-q', '--filter', 'label=com.docker.stack.namespace=' + project]).decode().strip(), 'Redis stack cleanup')
    else:
        for member in [*endpoints, *sentinels]:
            for prefix, container in containers(member):
                command(prefix + ['rm', '-f', container])
    api(f'/workspaces/{workspace}/deployments/{sample}', method='DELETE', status=204)
    return {'templates': {'application': template, 'startup': startup}, 'transport': transport,
            'deployment': sample, 'revision': revision, 'protocol': protocol}


def verify(data_endpoints, sentinel_endpoints, password, check, stop_member, restore_member, persisted_role):
    import redis
    from redis.sentinel import Sentinel

    def retry(fn, label, seconds=100):
        deadline = time.monotonic() + seconds
        last = None
        while time.monotonic() < deadline:
            try:
                value = fn()
                if value:
                    return value
            except (redis.RedisError, OSError) as error:
                last = type(error).__name__
            time.sleep(.5)
        raise AssertionError(label + ': ' + str(last))

    def client(endpoint, authenticated=True):
        return redis.Redis(host=endpoint[0], port=endpoint[1], password=password if authenticated else None,
                           socket_timeout=3, socket_connect_timeout=3, decode_responses=True)

    members = {name: client(endpoint) for name, endpoint in data_endpoints.items()}
    voters = {name: client(endpoint) for name, endpoint in sentinel_endpoints.items()}
    discovery = Sentinel(list(sentinel_endpoints.values()), socket_timeout=3,
                         sentinel_kwargs={'password': password}, password=password)
    stopped = set()
    try:
        for label, endpoint in [('data', next(iter(data_endpoints.values()))),
                                ('Sentinel', next(iter(sentinel_endpoints.values())))]:
            anonymous = client(endpoint, False)
            try:
                try:
                    anonymous.ping()
                except redis.AuthenticationError:
                    rejected = True
                else:
                    rejected = False
                check('Redis ' + label + ' rejects unauthenticated clients', rejected)
            finally:
                anonymous.close()
        retry(lambda: all(v.ping() for v in voters.values()), 'authenticated Sentinel health')
        retry(lambda: all(len(v.sentinel_sentinels('example')) == 2 for v in voters.values()), 'three independent Sentinel voters')
        check('all three Sentinels authorize quorum', all(v.sentinel_ckquorum('example') for v in voters.values()))
        original = retry(lambda: discovery.discover_master('example'), 'initial master discovery')
        check('Sentinel discovery returns a configured reachable data endpoint', original in data_endpoints.values())
        original_name = next(name for name, endpoint in data_endpoints.items() if endpoint == original)

        def write(endpoint, key, value, replicas):
            with client(endpoint) as pool:
                with pool.client() as connection:
                    connection.set(key, value)
                    return connection.wait(replicas, 5000) >= replicas

        retry(lambda: write(original, 'proof:baseline', 'before', 2), 'baseline replicated to two replicas')
        retry(lambda: all(c.get('proof:baseline') == 'before' for c in members.values()), 'baseline visible on all three data members')
        check('baseline reaches both asynchronous replicas before fault injection', True)
        stop_member(original_name)
        stopped.add(original_name)

        def agreed(excluded=None):
            addresses = [v.sentinel_get_master_addr_by_name('example') for v in voters.values()]
            if len(set(addresses)) != 1 or not addresses[0] or addresses[0] == excluded:
                return None
            endpoint = addresses[0]
            assert endpoint in data_endpoints.values(), 'Sentinel advertised an unconfigured data endpoint'
            with client(endpoint) as connection:
                return endpoint if connection.info('replication')['role'] == 'master' else None

        promoted = retry(lambda: agreed(original), 'all Sentinels agree on a new writable master')
        retry(lambda: write(promoted, 'proof:after', 'after', 1), 'new master has a writable replica')
        with client(promoted) as connection:
            check('promoted master retains replicated baseline', connection.get('proof:baseline') == 'before')
        check('Sentinel promotes a different configured member', promoted != original)
        restore_member(original_name)
        stopped.remove(original_name)
        retry(lambda: members[original_name].info('replication')['role'] == 'slave'
              and members[original_name].get('proof:after') == 'after', 'old master rejoins as a caught-up replica')
        retry(lambda: all(c.get('proof:after') == 'after' for c in members.values()), 'all members converge after failover')
        expected_replica = 'replicaof ' + promoted[0] + ' ' + str(promoted[1])
        retry(lambda: expected_replica in persisted_role(original_name), 'old master persists replica role to its original volume')
        for name in voters:
            expected_monitor = 'sentinel monitor example ' + promoted[0] + ' ' + str(promoted[1]) + ' 2'
            retry(lambda name=name: expected_monitor in persisted_role(name), 'Sentinel persists current master on its own volume')
            stop_member(name)
            stopped.add(name)
            restore_member(name)
            stopped.remove(name)
            retry(lambda: agreed() == promoted, 'restarted Sentinel preserves elected master')
        stop_member(original_name)
        stopped.add(original_name)
        restore_member(original_name)
        stopped.remove(original_name)
        retry(lambda: members[original_name].info('replication')['role'] == 'slave'
              and members[original_name].get('proof:after') == 'after', 'restarted old master remains a replica')
        check('Sentinel and old-master restarts preserve dynamic configuration and data', True)

        for name in list(voters)[1:]:
            stop_member(name)
            stopped.add(name)
        survivor = next(iter(voters.values()))

        def no_quorum():
            try:
                survivor.sentinel_ckquorum('example')
            except redis.ResponseError as error:
                return 'NOQUORUM' in str(error)
            return False

        retry(no_quorum, 'one Sentinel cannot authorize failover', 40)
        check('lost Sentinel majority reports NOQUORUM', True)
        promoted_name = next(name for name, endpoint in data_endpoints.items() if endpoint == promoted)
        stop_member(promoted_name)
        stopped.add(promoted_name)
        time.sleep(8)  # Exceeds the template's 5-second subjective-down threshold.
        check('without quorum a failed master is not replaced during the observation window',
              survivor.sentinel_get_master_addr_by_name('example') == promoted
              and all(c.info('replication')['role'] == 'slave' for name, c in members.items() if name != promoted_name))
        restore_member(promoted_name)
        stopped.remove(promoted_name)
        for name in list(stopped):
            restore_member(name)
            stopped.remove(name)
        current = retry(agreed, 'master discovery after voter restoration')
        retry(lambda: all(v.sentinel_ckquorum('example') for v in voters.values()), 'Sentinel quorum restored')
        retry(lambda: write(current, 'proof:recovered', 'recovered', 2), 'write after quorum restoration')
        check('quorum restoration retains baseline and post-failover data',
              all(c.get('proof:baseline') == 'before' and c.get('proof:after') == 'after' for c in members.values()))
        return {'initial_master': original, 'promoted_master': promoted, 'current_master': current,
                'redis_version': members[original_name].info('server')['redis_version'],
                'dynamic_configuration_preserved': True, 'quorum_recovered': True,
                'replication': 'Asynchronous; verified acknowledged baseline, not RPO=0 or network-partition fencing'}
    finally:
        for name in stopped:
            restore_member(name)
        for connection in [*members.values(), *voters.values()]:
            connection.close()
        for connection in discovery.sentinels:
            connection.close()
