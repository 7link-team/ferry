#!/usr/bin/env python3
"""Verify the deployment policy and the actual non-root container sandbox."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[2]
POLICY = ROOT / 'sandbox-seccomp.json'
USERNS = {'names': ['unshare'], 'action': 'SCMP_ACT_ALLOW',
          'args': [{'index': 0, 'value': 268435456, 'op': 'SCMP_CMP_EQ'}]}


def run(*args, **kwargs):
    kwargs.setdefault('check', True)
    kwargs.setdefault('text', True)
    return subprocess.run(list(args), **kwargs)


def policy_check():
    policy = json.loads(POLICY.read_text(encoding='utf-8'))
    # Removing our one exact-flag exception must recover the pinned Moby policy.
    # This guards against accidentally broadening any other container syscall.
    assert policy['syscalls'].pop() == USERNS
    digest = hashlib.sha256(json.dumps(policy, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    assert digest == '8fbd0c8cf3e4bfe63e36a3584d2b5682e3a2aeade5986f1d699577edb4b14a9e', digest
    assert policy['defaultAction'] == 'SCMP_ACT_ERRNO'
    print('PASS Docker policy preserves the pinned baseline with one exact NEWUSER exception', flush=True)


def main():
    policy_check()
    if sys.argv[1:] == ['--check-profile']:
        return
    image, = sys.argv[1:]
    options = ['--security-opt', 'no-new-privileges:true', '--security-opt', f'seccomp={POLICY}']
    batch = {'version': 1, 'timeout_ms': 1000, 'memory_bytes': 16777216,
             'deterministic': True, 'evaluations': [{'expression': '1+1', 'context': {}}]}
    result = run('docker', 'run', '--rm', '--init', '-i', *options,
                 '--entrypoint', '/usr/local/bin/ferry-sandbox', image,
                 input=json.dumps(batch), capture_output=True)
    assert json.loads(result.stdout) == {'status': 'ok', 'version': 1, 'values': [2]}, result.stdout
    print('PASS non-root container evaluates expressions with no-new-privileges', flush=True)
    environment = dict(os.environ, FERRY_IMAGE=image, FERRY_DATABASE_URL='', FERRY_REDIS_URL='', FERRY_TRUSTED_PROXIES='')
    compose = ['docker', 'compose', '-p', 'sandbox-acceptance-' + uuid.uuid4().hex[:12], '-f', str(ROOT / 'compose.yaml')]
    try:
        run(*compose, 'up', '-d', '--wait', '--wait-timeout', '120', 'ferry', env=environment)
        result = run(*compose, 'exec', '-T', 'ferry', '/usr/local/bin/ferry-sandbox',
                     env=environment, input=json.dumps(batch), capture_output=True)
        assert json.loads(result.stdout) == {'status': 'ok', 'version': 1, 'values': [2]}, result.stdout
        print('PASS actual Compose deployment evaluates expressions with the referenced policy', flush=True)
    finally:
        run(*compose, 'down', '--volumes', '--remove-orphans', env=environment, check=False)
    # The test driver needs Python; it is added to an ephemeral test image only.
    # The production image, runtime user and sandbox executable remain unchanged.
    with tempfile.TemporaryDirectory(prefix='sandbox-container-') as directory:
        dockerfile = Path(directory) / 'Dockerfile'
        dockerfile.write_text(f'FROM {image}\nUSER root\nRUN apt-get update -qq && apt-get install -y -qq --no-install-recommends python3 && rm -rf /var/lib/apt/lists/*\nUSER 10001:10001\n', encoding='utf-8')
        name = 'sandbox-container-acceptance:' + Path(directory).name.lower()
        run('docker', 'build', '-t', name, directory)
        try:
            run('docker', 'run', '--rm', '--init', *options, '-v',
                f'{ROOT / "scripts/test-workflow-sandbox.py"}:/test-workflow-sandbox.py:ro',
                '--entrypoint', '/usr/bin/python3', name,
                '/test-workflow-sandbox.py', '/usr/local/bin/ferry-sandbox')
        finally:
            run('docker', 'image', 'rm', name, check=False)


if __name__ == '__main__':
    main()
