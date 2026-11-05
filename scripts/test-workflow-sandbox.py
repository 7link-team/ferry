#!/usr/bin/env python3
"""Real V13 subprocess checks; always retain the original inputs and outputs."""
import ctypes
import copy
from ctypes import wintypes
import hashlib
import json
import os
import select
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
work = Path(tempfile.mkdtemp(prefix='workflow-sandbox-'))
checks, timings = [], {}


def request(expression='42', **options):
    return dict({'version': 1, 'timeout_ms': 1000, 'memory_bytes': 16 * 1024 * 1024,
        'deterministic': True, 'evaluations': [{'expression': expression, 'context': {}}]}, **options)


def invoke(body, name, args=(), raw=None):
    data = raw if raw is not None else json.dumps(body, ensure_ascii=False).encode()
    (work / (name + '.request.json')).write_bytes(data)
    started = time.monotonic()
    env = dict(os.environ, FERRY_DATABASE_URL='PRIVATE_DATABASE_CANARY',
        FERRY_VAULT_KEY='PRIVATE_VAULT_CANARY', SANDBOX_CANARY_SECRET='PRIVATE_ENV_CANARY')
    result = subprocess.run([str(binary), *args], input=data, capture_output=True, timeout=12,
        env=env, creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    timings[name] = time.monotonic() - started
    (work / (name + '.stdout.json')).write_bytes(result.stdout)
    (work / (name + '.stderr.log')).write_bytes(result.stderr)
    assert result.returncode == 0, (name, result.returncode, result.stderr.decode(errors='replace'))
    assert len(result.stdout) <= 8 * 1024 * 1024, name
    return json.loads(result.stdout)


def error(body, code, name, **options):
    reply = invoke(body, name, **options)
    assert reply['status'] == 'error' and reply['code'] == code, (name, reply)
    assert 'PRIVATE_' not in json.dumps(reply), (name, reply)
    return reply


def passed(label):
    checks.append(label)
    print('PASS', label, flush=True)


def windows_children(parent):
    class Entry(ctypes.Structure):
        _fields_ = [('size', wintypes.DWORD), ('usage', wintypes.DWORD), ('pid', wintypes.DWORD),
            ('heap', ctypes.c_size_t), ('module', wintypes.DWORD), ('threads', wintypes.DWORD),
            ('parent', wintypes.DWORD), ('priority', wintypes.LONG), ('flags', wintypes.DWORD),
            ('exe', wintypes.WCHAR * 260)]
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Entry)]
    kernel.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Entry)]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.CreateToolhelp32Snapshot(2, 0)
    assert handle != ctypes.c_void_p(-1).value
    try:
        entry = Entry(); entry.size = ctypes.sizeof(entry)
        result = []
        valid = kernel.Process32FirstW(handle, ctypes.byref(entry))
        while valid:
            if entry.parent == parent:
                result.append(entry.pid)
            valid = kernel.Process32NextW(handle, ctypes.byref(entry))
        return result
    finally:
        kernel.CloseHandle(handle)


try:
    frozen = {'$now': {'iso': '2026-11-02T00:30:12.345+08:00', 'zone': 'Asia/Kuala_Lumpur'},
        '$today': {'iso': '2026-11-02T00:00:00.000+08:00', 'zone': 'Asia/Kuala_Lumpur'}}
    dates = [
        ('={{$now}}', frozen['$now']['iso']),
        ('=at {{$now}}', 'at ' + frozen['$now']['iso']),
        ('={{({date:$today.toISODate(),zone:$now.zoneName,offset:$now.offset,fields:[$now.year,$now.month,$now.day,$now.hour,$now.minute,$now.second,$now.millisecond],elapsed:$now-$today,clock:typeof Date,same:$now.toMillis()==+$now,json:JSON.stringify($today)})}}',
         {'date':'2026-11-02','zone':'Asia/Kuala_Lumpur','offset':480,'fields':[2026,11,2,0,30,12,345],
          'elapsed':1812345,'clock':'undefined','same':True,'json':'"2026-11-02T00:00:00.000+08:00"'}),
        ('={{Object.isFrozen($now)&&Object.isFrozen($now.constructor)&&Object.isFrozen(Object.getPrototypeOf($now))&&Object.isFrozen($now.toISO)&&typeof $now.constructor.now==="undefined"}}', True),
        ('={{String($now)}}', frozen['$now']['iso'])]
    date_body = request(context=frozen, evaluations=[{'expression':source,'template':True,'context':{}}
        for source, _ in dates] * 2)
    assert invoke(date_body, 'frozen-dates')['values'] == [value for _, value in dates] * 2
    for index, source in enumerate(('$now.year=1', '$now.constructor.prototype.toISO=()=>"bad"',
            '$now.toISO.carry=1', '__context.$now.iso="bad"')):
        error(request(source, context=frozen), 'evaluation', f'frozen-date-mutation-{index}')
    invalid = copy.deepcopy(frozen)
    invalid['$now']['iso'] = '2026-02-30T00:00:00.000Z'
    error(request('$now.toISO()', context=invalid), 'evaluation', 'invalid-frozen-date')
    passed('frozen date helpers preserve ISO types and interpolation, batch consistency, timezone fields and immutable prototypes without exposing a native clock')

    batch = request('({answer:$json.value*2, label:$vars.label, valid:true, empty:null})')
    batch['evaluations'][0]['context'] = {'$json': {'value': 21}, '$vars': {'label': '汉字_日本語'}}
    expected = {'answer': 42, 'label': '汉字_日本語', 'valid': True, 'empty': None}
    assert invoke(batch, 'typed-context')['values'] == [expected]
    passed('isolated QuickJS preserves JSON types, Unicode and the explicit plain-data context')

    syntax = request(syntax_only=True, evaluations=[
        {'expression': source, 'template': True, 'context': {}}
        for source in ('={{(()=>{while(true){}})()}}',
            '=text {{(()=>{throw new Error("PRIVATE_COMPILE_ONLY")})()}}',
            '={{missing.upstream.value}}', '={{globalThis.leak=1}}')])
    assert invoke(syntax, 'compile-only-no-execution')['values'] == []
    escape = '=0)); }); (()=>{while(true){}})(); (function(){return ((0'
    bad = request(syntax_only=True, evaluations=[
        {'expression': '=42', 'template': True, 'context': {}},
        {'expression': escape, 'template': True, 'context': {}}])
    assert error(bad, 'syntax', 'compile-only-wrapper-escape')['index'] == 1
    passed('confined compile-only accepts unresolved, throwing and infinite-loop expressions without executing them and rejects wrapper breakout')

    templates = [
        ('=$json.value*2', 42), ('={{$json.value*2}}', 42),
        ('={{{a:1}}}', {'a': 1}), ('=answer={{$json.value*2}}!', 'answer=42!'),
        ("={{ '}}' }}", '}}'), ("={{ /}}/.test('}}') }}", True),
        ('={{ `x${({a:1}).a}` }}', 'x1'),
        ('={{ 1 // ignored }}\n + 2 }}', 3),
        ('=data={{({a:[true,null]})}}', 'data={"a":[true,null]}')]
    body = request(evaluations=[{'expression': source, 'template': True,
        'context': {'$json': {'value': 21}}} for source, _ in templates])
    assert invoke(body, 'ferry-templates')['values'] == [value for _, value in templates]
    error(request(evaluations=[{'expression': '=text {{ Promise.resolve(1) }}',
        'template': True, 'context': {}}]), 'evaluation', 'template-non-json')
    passed('confined Ferry templates preserve sole-expression types and parse strings, regex, comments, object and template-literal delimiters')

    identity = 'a' * 64
    def pair(node, index=0, iteration_hash=identity):
        return {'node': node, 'iteration_hash': iteration_hash, 'index': index}
    data = {'items': [{'json': {'value': 22}, 'paired': [pair('middle')]}],
        'upstream': [
            {'node': 'source', 'name': 'Source', 'iteration_hash': identity,
                'items': [{'json': {'value': 1}}, {'json': {'value': 2}}]},
            {'node': 'source', 'name': 'Source', 'iteration_hash': 'b' * 64,
                'items': [{'json': {'value': 99}}]},
            {'node': 'middle', 'name': 'Middle', 'iteration_hash': identity,
                'items': [{'json': {}, 'paired': [pair('source', 1)]}]}]}
    def scoped(source, snapshot=data, index=0):
        return request(data=snapshot, evaluations=[{'expression': source,
            'item_index': index, 'context': {'$json': {'value': 999}}}])
    result = invoke(scoped("({current:$json.value,item:$input.item.json.value,first:$input.first().json.value,count:$input.all().length,source:$('Source').item.json.value,all:$('Source').all().map(x=>x.json.value)})"), 'lineage-typed')
    assert result['values'] == [{'current': 22, 'item': 22, 'first': 22,
        'count': 1, 'source': 2, 'all': [1, 2, 99]}], result
    diamond = copy.deepcopy(data)
    diamond['upstream'][2]['items'][0]['paired'].append(pair('source', 1))
    diamond['items'][0]['paired'].append(pair('branch'))
    diamond['upstream'].append({'node': 'branch', 'name': 'Branch', 'iteration_hash': identity,
        'items': [{'json': {}, 'paired': [pair('source', 1)]}]})
    assert invoke(scoped("$('Source').item.json.value", diamond), 'lineage-diamond')['values'] == [2]
    merged = copy.deepcopy(data)
    merged['upstream'][2]['items'][0]['paired'].append(pair('source', 0))
    error(scoped("$('Source').item", merged), 'ambiguous_item', 'lineage-ambiguous')
    iterations = copy.deepcopy(data)
    iterations['upstream'][2]['items'][0]['paired'].append(pair('source', 0, 'b' * 64))
    error(scoped("$('Source').item", iterations), 'ambiguous_item', 'lineage-ambiguous-iterations')
    assert invoke(scoped("$('Source').first().json.value", merged), 'lineage-first')['values'] == [1]
    cycle = copy.deepcopy(data)
    cycle['upstream'][2]['items'][0]['paired'] = [pair('middle')]
    error(scoped("$('Source').item", cycle), 'missing_item', 'lineage-cycle')
    error(scoped("$('Unknown').item"), 'missing_item', 'lineage-missing')
    error(scoped('$input.item', index=None), 'missing_item', 'lineage-no-selection')
    passed('shared input helpers follow exact iteration identities, deduplicate diamonds and explicitly reject ambiguous, missing and cyclic lineage')
    for source in ('($input.item.json.value=99)', "($('Source').item.json.value=99)",
                   '(__scope.lookup.secret=99)', '$input.all().push({json:{}})'):
        error(scoped(source), 'evaluation', 'lineage-frozen-' + str(len(timings)))
    error(scoped('$input.item', index=1), 'invalid_request', 'lineage-index-range')
    invalid_snapshot = copy.deepcopy(data)
    invalid_snapshot['upstream'].append(copy.deepcopy(data['upstream'][0]))
    error(scoped('$input.item', invalid_snapshot), 'invalid_request', 'lineage-duplicate-identity')
    passed('shared execution snapshots stay immutable and reject out-of-range item selection and duplicate output identities')

    shared = copy.deepcopy(data)
    shared['items'] = [{'json': {'value': value}, 'paired': [pair('source', value % 2)]}
        for value in range(1000)]
    shared_expression = "(()=>{const helper=$('Source').all;const previous=helper.counter||0;helper.counter=previous+1;return [$json.value,$('Source').item.json.value,previous]})()"
    shared_batch = request(data=shared, memory_bytes=64*1024*1024,
        evaluations=[{'expression': shared_expression, 'item_index': value, 'context': {}}
            for value in range(1000)])
    assert invoke(shared_batch, 'lineage-shared-batch')['values'] == [
        [value, value % 2 + 1, 0] for value in range(1000)]
    passed('1000 evaluations share one snapshot while retaining each selected input, lineage and fresh helper state')
    metadata = request(data=shared, context={'$vars': {'factor': 3, 'large': 'x' * 20000}},
        memory_bytes=64*1024*1024,
        evaluations=[{'expression': '$vars.factor * $json.value', 'item_index': value, 'context': {}}
            for value in range(1000)])
    assert invoke(metadata, 'shared-metadata')['values'] == [value*3 for value in range(1000)]
    frozen = request(context={'$vars': {'nested': {'value': 1}}},
        evaluations=[{'expression': '($vars.nested.value=99)', 'context': {}}])
    error(frozen, 'evaluation', 'shared-metadata-frozen')
    override = copy.deepcopy(frozen)
    override['evaluations'][0]['context'] = {'$vars': {'nested': {'value': 999}}}
    error(override, 'invalid_request', 'shared-metadata-override')
    forbidden = copy.deepcopy(frozen); forbidden['context'] = {'$env': {'secret': 'PRIVATE'}}
    error(forbidden, 'invalid_request', 'shared-metadata-forbidden')
    passed('large metadata is sent once per batch, stays deeply immutable and rejects per-item overrides and host namespaces')

    host = request('[typeof process,typeof require,typeof fetch,typeof $env,typeof secrets,typeof std,typeof os]')
    assert invoke(host, 'no-host-globals')['values'] == [['undefined'] * 7]
    for source in ('Date.now()', 'Math.random()', '(globalThis.leak=1)', '(Array.prototype.push=()=>0)'):
        error(request(source), 'evaluation', 'frozen-' + str(len(timings)))
    assert invoke(request('typeof leak'), 'fresh-global')['values'] == ['undefined']
    passed('deterministic mode blocks clock, random and global/intrinsic mutation and exposes no host API')

    error(request(), 'isolation', 'direct-inner-bypass', args=('--inner',))
    if os.name == 'nt' or sys.platform == 'linux':
        canary = work / 'private.txt'; canary.write_text('PRIVATE_HOST_FILE_CANARY', encoding='utf-8')
        assert canary.read_text() == 'PRIVATE_HOST_FILE_CANARY'
        if sys.platform == 'linux':
            # The uid change alone must not make the read/write probes pass.
            # Ordinary low-privilege code could read this file and write here;
            # seccomp must prevent both even on a permissive directory.
            work.chmod(0o777); canary.chmod(0o644)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0)); listener.listen()
            probe = {'path': str(canary), 'address': '127.0.0.1:' + str(listener.getsockname()[1])}
            reply = invoke(probe, 'native-isolation', args=('--isolation-probe',))
            assert reply['status'] == 'ok' and reply['probe'] and all(reply['probe'].values()), reply
        passed('real OS confinement denies native file, write, network, child-process and memory escapes; business environment is absent')

    error(request('1 +'), 'syntax', 'syntax')
    for source in ('NaN', 'Infinity', 'undefined', '1n', 'Promise.resolve(1)',
                   "(()=>{throw new Error('PRIVATE_USER_EXCEPTION')})()"):
        body = request(source)
        body['evaluations'].insert(0, {'expression': '42', 'context': {}})
        assert error(body, 'evaluation', 'non-json-' + str(len(timings)))['index'] == 1
    passed('invalid syntax and non-JSON/async/error results fail the batch without partial output or exception-text leakage')

    error(request('(()=>{while(true){}})()', timeout_ms=30), 'timeout', 'infinite-loop')
    error(request("/(a+)+$/.test('a'.repeat(10000)+'!')", timeout_ms=30), 'timeout', 'catastrophic-regex')
    assert timings['infinite-loop'] < 5 and timings['catastrophic-regex'] < 5
    assert invoke(request('7*6'), 'after-timeouts')['values'] == [42]
    passed('infinite loops and catastrophic regex are interrupted; a fresh request succeeds after timeouts')

    error(request("(()=>{const a=[];while(true){a.push(new Array(10000).fill('x'));}})()",
        memory_bytes=1024*1024), 'memory', 'memory-limit')
    assert invoke(request('6*7'), 'after-memory')['values'] == [42]
    error(request("'x'.repeat(9*1024*1024)", memory_bytes=64*1024*1024), 'result_limit', 'result-limit')
    passed('VM memory and response-size limits are enforced; memory failure cannot poison a later request')

    for name, patch in [('unknown-version', {'version': 2}), ('zero-timeout', {'timeout_ms': 0}),
                        ('large-timeout', {'timeout_ms': 5001}), ('small-memory', {'memory_bytes': 1048575}),
                        ('large-memory', {'memory_bytes': 67108865}), ('unknown-option', {'database_url': 'PRIVATE_INVALID'})]:
        error(request(**patch), 'invalid_request', name)
    error(request(evaluations=[{'expression': '1', 'context': {'$env': {'key': 'PRIVATE_INVALID'}}}]),
        'invalid_request', 'forbidden-context')
    error(request(evaluations=[{'expression': '1', 'context': {}}] * 20001), 'invalid_request', 'many-evaluations')
    error(None, 'invalid_request', 'request-limit', raw=b' ' * (8 * 1024 * 1024 + 1))
    passed('protocol rejects out-of-budget requests, forbidden contexts and arbitrary launch/configuration options')

    if os.name == 'nt':
        parent = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=subprocess.CREATE_NO_WINDOW)
        child_pid = None
        try:
            parent.stdin.write(json.dumps(request('(()=>{while(true){}})()', timeout_ms=5000)).encode())
            parent.stdin.close()
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                assert parent.poll() is None
                children = windows_children(parent.pid)
                if children:
                    assert len(children) == 1
                    child_pid = children[0]
                    break
                time.sleep(0.02)
            assert child_pid is not None, 'confined child was not observed'
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            kernel.OpenProcess.restype = wintypes.HANDLE
            kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            child_handle = kernel.OpenProcess(0x100000, False, child_pid)
            assert child_handle, ('OpenProcess', ctypes.get_last_error())
            try:
                parent.kill(); parent.wait(timeout=5)
                assert kernel.WaitForSingleObject(child_handle, 3000) == 0, 'confined child survived parent death'
            finally:
                kernel.CloseHandle(child_handle)
        finally:
            if parent.poll() is None:
                parent.kill(); parent.wait(timeout=5)
            parent.stdout.close(); parent.stderr.close()
        passed('killing the launcher closes its job and terminates the exact observed confined child')

    if sys.platform == 'linux':
        parent = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE)
        child_fd = None
        try:
            parent.stdin.write(json.dumps(request('(()=>{while(true){}})()', timeout_ms=5000)).encode())
            parent.stdin.close()
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                assert parent.poll() is None
                children = Path(f'/proc/{parent.pid}/task/{parent.pid}/children').read_text().split()
                if children:
                    assert len(children) == 1
                    state = Path(f'/proc/{children[0]}/status').read_text()
                    if any(line.split() == ['Seccomp:', '2'] for line in state.splitlines()):
                        child_fd = os.pidfd_open(int(children[0]))
                        break
                time.sleep(0.02)
            assert child_fd is not None, 'confined child was not observed'
            parent.kill(); parent.wait(timeout=5)
            assert select.select([child_fd], [], [], 3)[0], 'confined child survived parent death'
        finally:
            if child_fd is not None:
                os.close(child_fd)
            if parent.poll() is None:
                parent.kill(); parent.wait(timeout=5)
            parent.stdout.close(); parent.stderr.close()
        passed('killing the launcher terminates the exact pidfd-observed Linux worker')

    count = 10000
    batch = request(timeout_ms=5000, memory_bytes=64*1024*1024,
        evaluations=[{'expression': '$json.value*2', 'context': {'$json': {'value': value}}} for value in range(count)])
    result = invoke(batch, 'throughput')
    assert result.get('values') == [value*2 for value in range(count)], result
    throughput = count / timings['throughput']
    assert throughput >= 10000, ('V13 throughput gate', throughput, timings['throughput'])
    passed(f'cold confined batch evaluates {count} distinct items at {throughput:.0f} expressions/second')

    result = {'result': 'PASS', 'checks': checks, 'timings_seconds': timings,
        'throughput_per_second': throughput, 'binary': str(binary),
        'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'platform': sys.platform,
        'boundaries': ['This result covers only the platform where this harness ran; other OS runtime confinement remains unverified',
            'Covers raw JS, Ferry templates, compile-only, input, lineage and frozen date helpers; graph execution remains unverified; graph HTTP acceptance uses a separate harness']}
    (work / 'results.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(f'PASS: {len(checks)} sandbox checks; {throughput:.0f} expressions/second', flush=True)
except BaseException as failure:
    (work / 'results.json').write_text(json.dumps({
        'result': 'FAIL', 'checks': checks, 'timings_seconds': timings,
        'failure': str(failure), 'platform': sys.platform, 'binary': str(binary),
        'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    }, indent=2), encoding='utf-8')
    raise
finally:
    print('Sandbox evidence:', work, flush=True)
