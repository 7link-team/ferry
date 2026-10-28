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
    target = next(line.split(': ', 1)[1] for line in subprocess.check_output(['rustc', '-vV'], text=True).splitlines() if line.startswith('host: '))
    catalog = {'schema': 1, 'version': '9.9.9', 'protocol': 1,
               'targets': {'fixture-target': {'url': 'https://example.invalid/runner',
                  'size': len(payload), 'sha256': hashlib.sha256(payload).hexdigest()}}}
    catalog['targets'][target] = dict(catalog['targets']['fixture-target'])
    for name, changes in [('valid', {}), ('wrong-protocol', {'protocol': 999}),
                          ('wrong-schema', {'schema': 999}), ('downgrade', {'version': '0.0.1'})]:
        path = root/(name+'.json')
        path.write_text(json.dumps(dict(catalog, **changes)))
        signer('sign', '--', '--private-key-path', str(root/'test.key'), '--password', '', str(path))
    for name, changes in [('bad-url', {'url': 'http://example.invalid/runner'}),
                          ('credential-url', {'url': 'https://user:secret@example.invalid/runner'}),
                          ('bad-size', {'size': 0}), ('oversize', {'size': 268435457}),
                          ('bad-hash', {'sha256': 'not-a-sha256'})]:
        invalid = json.loads(json.dumps(catalog))
        for artifact in invalid['targets'].values(): artifact.update(changes)
        path = root/(name+'.json')
        path.write_text(json.dumps(invalid))
        signer('sign', '--', '--private-key-path', str(root/'test.key'), '--password', '', str(path))
    env = dict(os.environ, RUNNER_UPDATE_PUBLIC_KEY=(root/'test.key.pub').read_text().strip(), RUNNER_UPDATE_TEST_DIR=str(root))
    # Native CI only: this fixture-key binary is never uploaded as a release asset.
    subprocess.run(['cargo', 'build', '--locked', '-p', 'ferry-runner'], env=env, check=True)
    runner = str(Path('target/debug/ferry-runner' + ('.exe' if os.name == 'nt' else '')).resolve())
    command = [runner, 'service', 'verify-update', '--manifest', str(root/'valid.json'),
               '--signature', str(root/'valid.json.sig'), '--artifact', str(root/'payload.bin'), '--version', '9.9.9']
    subprocess.run(command, env=dict(os.environ, RUNNER_UPDATE_PUBLIC_KEY=(root/'other.key.pub').read_text().strip()), check=True)
    (root/'payload.bin').write_bytes(b'X' + payload[1:])
    assert subprocess.run(command, capture_output=True).returncode != 0, 'CLI must reject tampered artifact'
    (root/'payload.bin').write_bytes(payload)
    # Exercise the independent release-side verifier with a real signed fixture.
    subprocess.run(['node', '--input-type=module', '-e',
        "import {readFileSync} from 'node:fs'; import {verifySignature} from './.github/scripts/runner-manifest.mjs'; "
        "const p=process.argv[1]; verifySignature(readFileSync(p+'/valid.json'),readFileSync(p+'/valid.json.sig','utf8'),readFileSync(p+'/test.key.pub','utf8')); "
        "try {verifySignature(Buffer.from('tampered'),readFileSync(p+'/valid.json.sig','utf8'),readFileSync(p+'/test.key.pub','utf8')); process.exit(1)} catch {}", str(root)], check=True)
    subprocess.run(['cargo', 'test' , '--locked', '-p', 'ferry-runner', 'signed_runner_update', '--', '--ignored'],
                   env=env, check=True)
    subprocess.run(['cargo', 'test', '--locked', '-p', 'ferry-server', 'runner_install::tests', '--', '--ignored'], env=env, check=True)
