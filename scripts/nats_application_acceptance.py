"""Protocol acceptance for applications created through Ferry's deployment API."""
import asyncio
import time
from urllib.parse import urlparse


async def verify(urls, token, check, stop_member, restore_member):
    import nats
    from nats.js.api import AckPolicy, ConsumerConfig, StorageType, StreamConfig

    async def quiet_error(_error):
        pass

    async def retry(fn, label, timeout=75):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                value = await fn()
                if value:
                    return value
            except Exception as error:
                last = type(error).__name__
            await asyncio.sleep(.5)
        raise AssertionError(label + ': ' + str(last))

    rejected = False
    try:
        unauthorized = await nats.connect(servers=urls[:1], connect_timeout=2, allow_reconnect=False,
                                          max_reconnect_attempts=0, error_cb=quiet_error)
    except Exception:
        rejected = True
    else:
        await unauthorized.close()
    check('NATS application rejects an unauthenticated client', rejected)
    client = await retry(lambda: nats.connect(servers=urls, token=token, connect_timeout=2,
        reconnect_time_wait=.2, max_reconnect_attempts=100, error_cb=quiet_error), 'authenticated NATS client')
    stopped = set()
    try:
        expected = {(urlparse(url).hostname, urlparse(url).port) for url in urls}
        check('NATS client discovery returns only configured reachable endpoints', bool(client.discovered_servers)
              and all((server.hostname, server.port) in expected for server in client.discovered_servers))
        js = client.jetstream(timeout=3)
        await retry(lambda: js.add_stream(config=StreamConfig(name='PROOF', subjects=['proof'],
                    storage=StorageType.FILE, num_replicas=3)), 'replicated stream')
        async def current():
            info = await js.stream_info('PROOF')
            return info if info.cluster and len(info.cluster.replicas or []) == 2 and all(r.current for r in info.cluster.replicas) else None
        baseline = await retry(current, 'three current members')
        check('NATS application provides three current persistent stream replicas', baseline.config.num_replicas == 3)
        consumer = await js.pull_subscribe('proof', durable='proof-consumer', stream='PROOF',
            config=ConsumerConfig(ack_policy=AckPolicy.EXPLICIT, ack_wait=2, num_replicas=3))
        await js.publish('proof', b'before')
        first = (await consumer.fetch(1, timeout=5))[0]
        await first.ack_sync(timeout=5)
        await js.publish('proof', b'pending')
        pending = (await consumer.fetch(1, timeout=5))[0]
        check('NATS durable consumer has an acknowledged and a pending message', first.data == b'before' and pending.data == b'pending')
        leader = baseline.cluster.leader
        stop_member(leader)
        stopped.add(leader)
        async def changed():
            info = await js.stream_info('PROOF')
            return info if info.cluster and info.cluster.leader and info.cluster.leader != leader else None
        promoted = await retry(changed, 'leader election after member loss')
        await retry(lambda: js.publish('proof', b'after'), 'publish after election')
        check('NATS committed data survives leader loss', (await js.get_msg('PROOF', seq=1)).data == b'before')
        await consumer.unsubscribe()
        consumer = await retry(lambda: js.pull_subscribe('proof', durable='proof-consumer', stream='PROOF'), 'durable consumer recovery')
        received = {}
        delivery_deadline = time.monotonic() + 90
        while len(received) < 2:
            assert time.monotonic() < delivery_deadline, 'durable consumer recovery exceeded deadline'
            message = (await retry(lambda: consumer.fetch(1, timeout=3), 'pending delivery'))[0]
            sequence = message.metadata.sequence.stream
            assert sequence in (2, 3), 'already acknowledged message replayed'
            received[sequence] = (message.data, message.metadata.num_delivered)
            await message.ack_sync(timeout=5)
        check('NATS durable consumer resumes pending messages without replaying acknowledged data',
              received[2][0] == b'pending' and received[2][1] >= 2 and received[3][0] == b'after')
        restore_member(leader)
        stopped.remove(leader)
        await retry(current, 'returning member catches up using its original volume')
        check('NATS returning member restores three current replicas', True)
        live = (await js.stream_info('PROOF')).cluster.leader
        for member in ['nats-a', 'nats-b', 'nats-c']:
            if member != live:
                stop_member(member)
                stopped.add(member)
        failed = False
        try:
            await js.publish('proof', b'without-quorum', timeout=3)
        except Exception:
            failed = True
        check('NATS lost quorum does not acknowledge a successful write', failed)
        for member in list(stopped):
            restore_member(member)
            stopped.remove(member)
        await retry(current, 'quorum restoration')
        await retry(lambda: js.publish('proof', b'recovered'), 'publish after quorum restoration')
        check('NATS quorum restoration preserves committed baseline', (await js.get_msg('PROOF', seq=1)).data == b'before')
        return {'original_leader': leader, 'promoted_leader': promoted.cluster.leader,
                'stream_replicas': 3, 'consumer_pending_sequence': 2, 'quorum_recovered': True}
    finally:
        for member in stopped:
            restore_member(member)
        await client.close()
