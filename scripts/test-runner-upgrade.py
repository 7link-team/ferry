#!/usr/bin/env python3
"""CI-only real signed service upgrade, rollback, and interrupted activation."""
import functools, hashlib, http.server, json, os, platform, re, shutil, ssl, subprocess, tempfile, threading
from pathlib import Path

repo = Path.cwd()
npm = 'npm.cmd' if os.name == 'nt' else 'npm'
exe = '.exe' if os.name == 'nt' else ''
openssl = shutil.which('openssl') or 'C:/Program Files/Git/usr/bin/openssl.exe'
system = platform.system()
def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)
def privileged(*args):
    return run(*((['sudo', '-n'] if os.name != 'nt' else []) + list(args)))

with tempfile.TemporaryDirectory(prefix='runner-upgrade-') as temp:
    root = Path(temp)
    def sign(*args):
        run(npm, 'exec', 'tauri', 'signer', *args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    sign('generate', '--', '--ci', '--password', '', '--write-keys', root/'key')
    env = dict(os.environ, RUNNER_UPDATE_PUBLIC_KEY=(root/'key.pub').read_text(encoding='utf-8').strip())
    run('cargo', 'build', '--locked', '-p', 'ferry-runner', env=env)
    baseline = root/('baseline'+exe)
    built = repo/'target/debug'/('ferry-runner'+exe)
    shutil.copy2(built, baseline)
    # Only source is copied; cached build products remain in the CI target dir.
    source = root/'source'
    for crate in ['runner', 'protocol']:
        shutil.copytree(repo/'crates'/crate, source/'crates'/crate)
    (source/'desktop').mkdir(parents=True)
    shutil.copy2(repo/'desktop/tauri.conf.json', source/'desktop/tauri.conf.json')
    shutil.copy2(repo/'Cargo.lock', source/'Cargo.lock')
    manifest = (repo/'Cargo.toml').read_text(encoding='utf-8').replace('"crates/server", ', '')
    original_main = (source/'crates/runner/src/main.rs').read_text(encoding='utf-8')
    for version, filename, stall in [('9.9.9', 'candidate', False), ('9.9.12', 'stalled', True)]:
        (source/'Cargo.toml').write_text(re.sub(r'^version = "[^"]+"$', f'version = "{version}"', manifest, count=1, flags=re.M))
        (source/'crates/runner/src/main.rs').write_text(original_main.replace('fn main() -> anyhow::Result<()> {', 'fn main() -> anyhow::Result<()> { std::thread::sleep(std::time::Duration::from_secs(120));') if stall else original_main, encoding='utf-8')
        run('cargo', 'build', '--offline', '--manifest-path', source/'Cargo.toml', '--target-dir', repo/'target', '-p', 'ferry-runner', env=env)
        shutil.copy2(built, root/filename)
    shutil.copy2(baseline, built)
    (root/'broken').write_bytes(b'not an executable\n')
    shutil.copy2(baseline, root/'wrong-version')
    run(openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-keyout', root/'ca.key', '-out', root/'ca.crt', '-days', '1', '-nodes', '-subj', '/CN=Runner CI root', '-addext', 'basicConstraints=critical,CA:TRUE', stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    run(openssl, 'req', '-new', '-newkey', 'rsa:2048', '-keyout', root/'tls.key', '-out', root/'tls.csr', '-nodes', '-subj', '/CN=localhost', stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    (root/'tls.ext').write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n', encoding='utf-8')
    run(openssl, 'x509', '-req', '-in', root/'tls.csr', '-CA', root/'ca.crt', '-CAkey', root/'ca.key', '-CAcreateserial', '-out', root/'tls.crt', '-days', '1', '-extfile', root/'tls.ext', stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    fingerprint = subprocess.check_output([openssl, 'x509', '-in', str(root/'ca.crt'), '-noout', '-fingerprint', '-sha1'], text=True).strip().split('=')[1].replace(':', '')
    ca_path = Path('/usr/local/share/ca-certificates')/(root.name+'.crt')
    def trust(add):
        if system == 'Linux':
            if add: privileged('cp', root/'ca.crt', ca_path)
            else: privileged('rm', '-f', ca_path)
            privileged('update-ca-certificates')
        elif system == 'Darwin':
            if add: privileged('security', 'add-trusted-cert', '-d', '-r', 'trustRoot', '-k', '/Library/Keychains/System.keychain', root/'ca.crt')
            else: privileged('security', 'delete-certificate', '-Z', fingerprint, '/Library/Keychains/System.keychain')
        elif add: run('certutil', '-addstore', 'Root', root/'ca.crt')
        else: run('certutil', '-delstore', 'Root', fingerprint)
    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args): pass
    http = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(root)))
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(root/'tls.crt', root/'tls.key')
    http.socket = tls.wrap_socket(http.socket, server_side=True)
    thread = threading.Thread(target=http.serve_forever, daemon=True)
    thread.start()
    target = next(line.split(': ', 1)[1] for line in subprocess.check_output(['rustc', '-vV'], text=True).splitlines() if line.startswith('host: '))
    protocol = int(re.search(r'pub const VERSION: u32 = (\d+);', (repo/'crates/protocol/src/lib.rs').read_text(encoding='utf-8'))[1])
    for version, filename in [('9.9.9','candidate'), ('9.9.10','broken'), ('9.9.11','wrong-version'), ('9.9.12','stalled')]:
        payload = (root/filename).read_bytes()
        catalog = {'schema':1, 'protocol':protocol, 'version':version, 'targets':{target:{'url':f'https://localhost:{http.server_port}/{filename}', 'size':len(payload), 'sha256':hashlib.sha256(payload).hexdigest()}}}
        path = root/(version+'.json')
        path.write_text(json.dumps(catalog))
        sign('sign', '--', '--private-key-path', root/'key', '--password', '', path)
    try:
        trust(True)
        if system == 'Darwin':
            privileged('security', 'dump-trust-settings', '-d')
            privileged('security', 'verify-cert', '-c', root/'tls.crt', '-p', 'ssl', '-s', 'localhost')
        args = [str(Path(shutil.which('python') or shutil.which('python3'))), 'scripts/test-runner-service.py']
        if os.name != 'nt':
            run('sudo', '-n', 'env', 'RUNNER_TEST_SYSTEM_SERVICE=1', f'RUNNER_BINARY={baseline}', f'RUNNER_UPGRADE_FIXTURES={root}', *args)
        else:
            run(*args, env=dict(os.environ, RUNNER_BINARY=str(baseline), RUNNER_UPGRADE_FIXTURES=str(root)))
    finally:
        http.shutdown()
        trust(False)
