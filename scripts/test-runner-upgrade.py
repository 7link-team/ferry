#!/usr/bin/env python3
"""Real signed service upgrade, rollback, and automatic activation acceptance.
Fixture CA trust is embedded only in copied test sources; OS trust is never changed.
RUNNER_TEST_AUTO_ONLY=1 reruns automatic scenarios after separate manual acceptance.
"""
import functools, hashlib, http.server, json, os, platform, re, shutil, ssl, subprocess, sys, tempfile, threading, time
from pathlib import Path

repo = Path.cwd()
npm = 'npm.cmd' if os.name == 'nt' else 'npm'
exe = '.exe' if os.name == 'nt' else ''
openssl = shutil.which('openssl') or 'C:/Program Files/Git/usr/bin/openssl.exe'
system = platform.system()
def run(*args, **kwargs):
    if os.name == 'nt': kwargs.setdefault('creationflags', subprocess.CREATE_NO_WINDOW)
    kwargs.setdefault('stdout', sys.stdout)
    kwargs.setdefault('stderr', sys.stderr)
    return subprocess.run([str(a) for a in args], check=True, **kwargs)
with tempfile.TemporaryDirectory(prefix='runner-upgrade-') as temp:
    root = Path(temp)
    def sign(*args):
        run(npm, 'exec', 'tauri', 'signer', *args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    sign('generate', '--', '--ci', '--password', '', '--write-keys', root/'key')
    env = dict(os.environ, RUNNER_UPDATE_PUBLIC_KEY=(root/'key.pub').read_text(encoding='utf-8').strip())
    # OpenSSL 1.1 appends -addext to configured v3_ca extensions, producing
    # duplicate basicConstraints. Use a fixture-owned config on every OS.
    (root/'ca.cnf').write_text('[req]\ndistinguished_name=dn\nx509_extensions=ca\n[dn]\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n', encoding='utf-8')
    run(openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-keyout', root/'ca.key', '-out', root/'ca.crt', '-days', '1', '-nodes', '-subj', '/CN=Runner CI root', '-config', root/'ca.cnf', stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    run(openssl, 'req', '-new', '-newkey', 'rsa:2048', '-keyout', root/'tls.key', '-out', root/'tls.csr', '-nodes', '-subj', '/CN=localhost', stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    (root/'tls.ext').write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n', encoding='utf-8')
    run(openssl, 'x509', '-req', '-in', root/'tls.csr', '-CA', root/'ca.crt', '-CAkey', root/'ca.key', '-CAcreateserial', '-out', root/'tls.crt', '-days', '1', '-extfile', root/'tls.ext', stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    # Source and binaries are isolated; production executables are never overwritten.
    fixture_target = repo/'target/runner-upgrade-fixture'
    source = root/'source'
    for crate in ['runner', 'protocol']:
        shutil.copytree(repo/'crates'/crate, source/'crates'/crate)
    (source/'desktop').mkdir(parents=True)
    shutil.copy2(repo/'desktop/tauri.conf.json', source/'desktop/tauri.conf.json')
    shutil.copy2(repo/'Cargo.lock', source/'Cargo.lock')
    manifest = (repo/'Cargo.toml').read_text(encoding='utf-8').replace('"crates/server", ', '').replace(', "crates/desktop-updater"', '')
    update_source = source/'crates/runner/src/update.rs'
    update_text = update_source.read_text(encoding='utf-8')
    builder = 'reqwest::Client::builder()'
    assert builder in update_text
    certificate = '.add_root_certificate(reqwest::Certificate::from_pem(include_bytes!("fixture-ca.crt"))?)'
    shutil.copy2(root/'ca.crt', update_source.parent/'fixture-ca.crt')
    update_source.write_text(update_text.replace(builder, builder + certificate), encoding='utf-8')
    (source/'Cargo.toml').write_text(manifest, encoding='utf-8')
    run('cargo', 'build', '--offline', '--manifest-path', source/'Cargo.toml', '--target-dir', fixture_target, '-p', 'ferry-runner', env=env)
    built = fixture_target/'debug'/('ferry-runner'+exe)
    baseline = root/('baseline'+exe)
    shutil.copy2(built, baseline)
    original_main = (source/'crates/runner/src/main.rs').read_text(encoding='utf-8')
    for version, filename, stall in [('9.9.9', 'candidate', False), ('9.9.12', 'stalled', True)]:
        (source/'Cargo.toml').write_text(re.sub(r'^version = "[^"]+"$', f'version = "{version}"', manifest, count=1, flags=re.M))
        (source/'crates/runner/src/main.rs').write_text(original_main.replace('fn main() -> anyhow::Result<()> {', 'fn main() -> anyhow::Result<()> { std::thread::sleep(std::time::Duration::from_secs(120));') if stall else original_main, encoding='utf-8')
        run('cargo', 'build', '--offline', '--manifest-path', source/'Cargo.toml', '--target-dir', fixture_target, '-p', 'ferry-runner', env=env)
        shutil.copy2(built, root/filename)
    shutil.copy2(baseline, built)
    (root/'broken').write_bytes(b'not an executable\n')
    shutil.copy2(baseline, root/'wrong-version')
    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_GET(self):
            if self.path.endswith('.json.sig') and (root/'invalid-auto-signature').exists():
                payload = b'invalid fixture signature'
                self.send_response(200)
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                return
            if self.path == '/slow':
                (root/'slow.started').touch()
                deadline = time.monotonic() + 90
                while not (root/'slow.release').exists() and time.monotonic() < deadline:
                    time.sleep(.1)
                self.path = '/candidate'
            try: super().do_GET()
            except (BrokenPipeError, ConnectionResetError, ssl.SSLError): pass
    http = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(root)))
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(root/'tls.crt', root/'tls.key')
    http.socket = tls.wrap_socket(http.socket, server_side=True)
    thread = threading.Thread(target=http.serve_forever, daemon=True)
    thread.start()
    target = next(line.split(': ', 1)[1] for line in subprocess.check_output(['rustc', '-vV'], text=True, **({'creationflags':subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {})).splitlines() if line.startswith('host: '))
    protocol = int(re.search(r'pub const VERSION: u32 = (\d+);', (repo/'crates/protocol/src/lib.rs').read_text(encoding='utf-8'))[1])
    for version, filename in [('9.9.9','candidate'), ('9.9.10','broken'), ('9.9.11','wrong-version'), ('9.9.12','stalled'), ('9.9.13','candidate')]:
        payload = (root/filename).read_bytes()
        catalog = {'schema':1, 'protocol':protocol, 'version':version, 'targets':{target:{'url':f'https://localhost:{http.server_port}/{"slow" if version=="9.9.13" else filename}', 'size':len(payload), 'sha256':hashlib.sha256(payload).hexdigest()}}}
        path = root/(version+'.json')
        path.write_text(json.dumps(catalog))
        sign('sign', '--', '--private-key-path', root/'key', '--password', '', path)
    run(openssl, 'verify', '-CAfile', root/'ca.crt', root/'tls.crt')
    try:
        args = [str(Path(shutil.which('python') or shutil.which('python3'))), 'scripts/test-runner-service.py']
        if os.environ.get('RUNNER_TEST_AUTO_ONLY') == '1':
            pass  # The manual suite can be verified separately before automatic acceptance.
        elif os.name != 'nt':
            run('sudo', '-n', *(['timeout', '-k', '15s', '300s'] if system=='Linux' else []), 'env', 'RUNNER_TEST_SYSTEM_SERVICE=1', f'RUNNER_TEST_ACTIONS={os.environ.get("RUNNER_TEST_ACTIONS", "")}', f'RUNNER_BINARY={baseline}', f'RUNNER_UPGRADE_FIXTURES={root}', *args)
        else:
            run(*args, env=dict(os.environ, RUNNER_BINARY=str(baseline), RUNNER_UPGRADE_FIXTURES=str(root)))
        # The production server has no alternate trust/key/URL configuration.
        # Compile a throwaway source copy against the fixture's signed TLS catalog.
        shutil.copytree(repo/'crates/server', source/'crates/server')
        shutil.copytree(repo/'dist', source/'dist')
        (source/'scripts').mkdir()
        for script in ['runner-install.sh', 'runner-install.ps1']:
            shutil.copy2(repo/'scripts'/script, source/'scripts'/script)
        config = json.loads((source/'desktop/tauri.conf.json').read_text(encoding='utf-8'))
        config['plugins']['updater']['pubkey'] = (root/'key.pub').read_text(encoding='utf-8').strip()
        (source/'desktop/tauri.conf.json').write_text(json.dumps(config), encoding='utf-8')
        catalog_source = source/'crates/server/src/hosts/runner/install.rs'
        text = catalog_source.read_text(encoding='utf-8')
        production_url = 'format!("https://github.com/{repository}/releases/download/v{version}/runner-update.json")'
        assert production_url in text, 'fixture must replace the exact release URL, never weaken signature verification'
        shutil.copy2(root/'ca.crt', catalog_source.parent/'fixture-ca.crt')
        text = text.replace('reqwest::Client::builder()', 'reqwest::Client::builder().add_root_certificate(reqwest::Certificate::from_pem(include_bytes!("fixture-ca.crt")).expect("fixture certificate"))')
        text = text.replace(production_url, f'format!("https://localhost:{http.server_port}/{{version}}.json")', 1)
        catalog_source.write_text(text, encoding='utf-8')
        server_manifest = (repo/'Cargo.toml').read_text(encoding='utf-8').replace(', "crates/desktop-updater"', '')
        for version, mode in [('9.9.9', 'success'), ('9.9.10', 'rollback')]:
            (source/'Cargo.toml').write_text(re.sub(r'^version = "[^"]+"$', f'version = "{version}"', server_manifest, count=1, flags=re.M), encoding='utf-8')
            # Runner-only resolution prunes server dependencies, including locked yanked versions.
            shutil.copy2(repo/'Cargo.lock', source/'Cargo.lock')
            run('cargo', 'build', '--offline', '--manifest-path', source/'Cargo.toml', '--target-dir', fixture_target, '-p', 'ferry-server')
            fixture_server = root/('server-'+version+exe)
            shutil.copy2(fixture_target/'debug'/('ferry-server'+exe), fixture_server)
            auto_env = dict(os.environ, RUNNER_BINARY=str(baseline), FERRY_BINARY=str(fixture_server), RUNNER_UPGRADE_FIXTURES=str(root), RUNNER_TEST_AUTO_UPDATE=mode)
            if os.name != 'nt':
                run('sudo', '-n', *(['timeout', '-k', '15s', '420s'] if system=='Linux' else []), 'env', 'RUNNER_TEST_SYSTEM_SERVICE=1', f'RUNNER_TEST_ACTIONS={os.environ.get("RUNNER_TEST_ACTIONS", "")}', f'RUNNER_BINARY={baseline}', f'FERRY_BINARY={fixture_server}', f'RUNNER_UPGRADE_FIXTURES={root}', f'RUNNER_TEST_AUTO_UPDATE={mode}', *args)
            else:
                run(*args, env=auto_env)
    finally:
        http.shutdown()
