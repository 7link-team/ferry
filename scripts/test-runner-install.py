#!/usr/bin/env python3
"""Executable Unix bootstrap checks without root or real service changes.
Native CI additionally invokes the installers from test-runner-service.py.
"""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

repo = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix='runner-bootstrap-test-') as temp:
    root = Path(temp)
    tools = root / 'tools'
    tools.mkdir()
    marker = root / 'executed'
    payload = b'#!/bin/sh\nprintf "%s\\n" "$@" > "$TEST_MARKER"\nprintf "%s" "$FERRY_TOKEN" > "$TEST_MARKER.token"\n'
    (root / 'payload').write_bytes(payload)
    (tools / 'id').write_text('#!/bin/sh\necho 0\n')
    # Mock the transport boundary, while running the complete installer and
    # downloaded executable as subprocesses. Reject credentials on curl argv.
    (tools / 'curl').write_text('''#!/usr/bin/env python3
import os, pathlib, sys
args = sys.argv[1:]
assert os.environ['FERRY_TOKEN'] not in ' '.join(args), 'token leaked to argv'
assert '--max-time' not in args and '--connect-timeout' not in args, 'hidden installer deadline'
out = pathlib.Path(args[args.index('--output') + 1])
if '/api/runners/install/' in args[-1]:
    header = args[args.index('--header') + 1]
    assert header.startswith('@')
    assert pathlib.Path(header[1:]).read_text().strip() == 'Authorization: Bearer ' + os.environ['FERRY_TOKEN']
    out.write_bytes(pathlib.Path(os.environ['TEST_METADATA']).read_bytes())
else:
    import shutil
    with pathlib.Path(os.environ['TEST_PAYLOAD']).open('rb') as source, out.open('wb') as target:
        shutil.copyfileobj(source, target)
''')
    for path in tools.iterdir(): path.chmod(0o755)
    env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
               FERRY_TOKEN='single-use-test-secret', TEST_MARKER=str(marker),
               TEST_PAYLOAD=str(root / 'payload'), TEST_METADATA=str(root / 'metadata'))
    digest = hashlib.sha256(payload).hexdigest()
    valid = f'1.2.3\nhttps://example.invalid/runner\n{digest}\n{len(payload)}\n'
    cases = [('valid', valid, True), ('tamper', valid.replace(digest, '0' * 64), False),
             ('truncated', valid.replace(str(len(payload))+'\n', str(len(payload)+1)+'\n'), False),
             ('extra-bytes', valid.replace(str(len(payload))+'\n', str(len(payload)-1)+'\n'), False),
             ('http-artifact', valid.replace('https:', 'http:'), False),
             ('extra-line', valid + 'extra\n', False),
             ('partial-extra-line', valid + 'extra', False),
             ('missing-newline', valid.rstrip('\n'), False),
             ('zero-size', valid.replace(str(len(payload))+'\n', '0\n'), False),
             ('negative-size', valid.replace(str(len(payload))+'\n', '-1\n'), False),
             ('uppercase-hash', valid.replace(digest, digest.upper()), False),
             ('unrepresentable-size', valid.replace(str(len(payload))+'\n', '9223372036854775808\n'), False)]
    server_cases = [('remote-http', 'http://example.invalid', False),
                    ('remote-https', 'https://example.invalid', True),
                    ('loopback-ipv4', 'http://127.0.0.1:1234', True),
                    ('loopback-ipv6', 'http://[::1]:1234', True),
                    ('loopback-prefix-attack', 'http://localhost.evil.invalid', False),
                    ('loopback-port-attack', 'http://localhost:1234.evil.invalid', False)]
    cases += [(label, valid, success) for label, _, success in server_cases]
    servers = {label: url for label, url, _ in server_cases}
    for label, metadata, success in cases:
        server = servers.get(label, 'http://localhost:1234')
        marker.unlink(missing_ok=True)
        (root / 'metadata').write_text(metadata)
        result = subprocess.run(['/bin/sh', str(repo / 'scripts/runner-install.sh'), '--server',
                                 server, '--name', 'installer-test'],
                                env=env, capture_output=True, text=True)
        assert (result.returncode == 0) == success, (label, result.stdout, result.stderr)
        assert marker.exists() == success, (label, 'unverified artifact executed')
        if success:
            assert marker.read_text().splitlines() == ['service', 'install', '--server', server, '--name', 'installer-test', '--rebind']
            assert Path(str(marker)+'.token').read_text() == env['FERRY_TOKEN']
        print('PASS', label)
    # Stream a real executable across the former cutoff, without a large in-memory fixture.
    large_size = 268435457
    large_payload = payload + b'exit 0\n'
    with (root / 'payload').open('wb') as output:
        output.write(large_payload)
        output.write(b'#')
        block = b' ' * (1024 * 1024)
        remaining = large_size - len(large_payload) - 2
        while remaining:
            count = min(remaining, len(block))
            output.write(block[:count])
            remaining -= count
        output.write(b'\n')
    with (root / 'payload').open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    (root / 'metadata').write_text(f'1.2.3\nhttps://example.invalid/runner\n{digest}\n{large_size}\n')
    marker.unlink(missing_ok=True)
    result = subprocess.run(['/bin/sh', str(repo / 'scripts/runner-install.sh'), '--server',
                             'http://localhost:1234', '--name', 'installer-test'],
                            env=env, capture_output=True, text=True)
    assert result.returncode == 0, ('large-payload', result.stdout, result.stderr)
    assert marker.exists(), 'verified large artifact was not executed'
    print('PASS large-payload', large_size)
