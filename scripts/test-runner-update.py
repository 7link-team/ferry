#!/usr/bin/env python3
"""Verify runner update security with throwaway Minisign keys, only in CI."""
import json, os, subprocess, tempfile
from pathlib import Path

with tempfile.TemporaryDirectory(prefix='runner-update-') as directory:
    root = Path(directory)
    npm = 'npm.cmd' if os.name == 'nt' else 'npm'
    def signer(*args):
        subprocess.run([npm, 'exec', 'tauri', 'signer', *args], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    signer('generate', '--', '--ci', '--password', '', '--write-keys', str(root/'test.key'))
    signer('generate', '--', '--ci', '--password', '', '--write-keys', str(root/'other.key'))
    import hashlib
    payload = b'verified runner update fixture\n'
    (root/'payload.bin').write_bytes(payload)
    catalog = {'schema': 1, 'version': '9.9.9', 'protocol': 1,
               'targets': {'fixture-target': {'url': 'https://example.invalid/runner',
                  'size': len(payload), 'sha256': hashlib.sha256(payload).hexdigest()}}}
    for name, changes in [('valid', {}), ('wrong-protocol', {'protocol': 999}),
                          ('wrong-schema', {'schema': 999}), ('downgrade', {'version': '0.0.1'})]:
        path = root/(name+'.json')
        path.write_text(json.dumps(dict(catalog, **changes)))
        signer('sign', '--', '--private-key-path', str(root/'test.key'), '--password', '', str(path))
    subprocess.run(['cargo', 'test', '--locked', '-p', 'ferry-runner', 'signed_runner_update', '--', '--ignored'],
                   env=dict(os.environ, RUNNER_UPDATE_TEST_DIR=str(root)), check=True)
