/**
 * Real WKWebView simulator acceptance; no remote IPC, delegate mocks or TLS bypass.
 * Usage (fresh CI simulator, do NOT preinstall the app):
 * node .github/scripts/test-ios-client.mjs --udid UUID --app-path desktop/gen/apple/build/arm64-sim/App.app
 * --bundle-id defaults to desktop/tauri.conf.json; an override must match that source.
 * --source-check is portable and DOES NOT claim native acceptance.
 *
 * Only an installation created by this invocation is seeded/terminated/uninstalled.
 * Simulator itself, unrelated applications and user certificates are never modified.
 * A disposable XcodeGen/XCTest host tests real typing and keyboard reachability.
 * --skip-ui is an explicit diagnostic fallback, never full native acceptance.
 */
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createServer as createHttpsServer } from 'node:https';
import { spawn, spawnSync } from 'node:child_process';
import { closeSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir, homedir } from 'node:os';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const { values: args } = parseArgs({ options: {
  udid: { type: 'string' }, 'app-path': { type: 'string' }, 'bundle-id': { type: 'string' },
  output: { type: 'string' }, 'source-check': { type: 'boolean' },
  'skip-ui': { type: 'boolean' },
} });
const config = JSON.parse(readFileSync(join(root, 'desktop/tauri.conf.json'), 'utf8'));
const bundle = args['bundle-id'] ?? config.identifier;
assert.equal(bundle, config.identifier, 'Only the source-configured application may be tested');
assert.match(bundle, /^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/);
assert.equal(config.app.appDirectoriesOverride, undefined, 'Re-derive sandbox path if directory overrides change');
for (const name of readdirSync(join(root, 'desktop')).filter((name) => /^tauri\..*\.conf\.json$/.test(name))) {
  assert.equal(JSON.parse(readFileSync(join(root, 'desktop', name), 'utf8')).app?.appDirectoriesOverride,
    undefined, `Re-derive sandbox path for ${name} override`);
}
const lock = readFileSync(join(root, 'desktop/Cargo.lock'), 'utf8');
assert.match(lock, /name = "tauri"\nversion = "2\.12\.5"/);
assert.match(lock, /name = "dirs"\nversion = "7\.0\.0"/);
const registry = join(process.env.CARGO_HOME || join(homedir(), '.cargo'), 'registry/src');
function dependencySource(name, path) {
  for (const entry of readdirSync(registry)) {
    const candidate = join(registry, entry, name, path);
    if (existsSync(candidate)) return readFileSync(candidate, 'utf8');
  }
  throw new Error(`Pinned source unavailable: ${name}/${path}; run cargo fetch --manifest-path desktop/Cargo.toml`);
}
// Tauri's "desktop" path implementation also serves iOS. Fail closed on source drift.
assert.match(dependencySource('tauri-2.12.5', 'src/path/mod.rs'), /#\[cfg\(not\(target_os = "android"\)\)\]\s*mod desktop/);
assert.match(dependencySource('tauri-2.12.5', 'src/path/desktop.rs'), /pub fn app_local_data_dir[\s\S]*?dirs::data_local_dir\(\)[\s\S]*?dir\.join\(&self\.0\.config\(\)\.identifier\)/);
assert.match(dependencySource('dirs-7.0.0', 'src/lib.rs'), /#\[cfg\(any\(target_os = "macos", target_os = "ios"\)\)\]\s*mod mac/);
assert.match(dependencySource('dirs-7.0.0', 'src/mac.rs'), /h\.join\("Library\/Application Support"\)/);
assert.match(dependencySource('dirs-7.0.0', 'src/mac.rs'), /pub fn data_local_dir\(\)\s*-> Option<PathBuf>\s*\{ app_support_dir\(\) \}/);
const preferencesRelative = join('Library', 'Application Support', bundle, 'connection.json');
assert.match(readFileSync(join(root, 'desktop/src/connection.rs'), 'utf8'), /pub server_url: String/);
if (args['source-check']) {
  console.log(JSON.stringify({ sourceCheck: 'PASS', nativeAcceptance: 'NOT_RUN', bundle, preferencesRelative }, null, 2));
  process.exit(0);
}
assert.equal(process.platform, 'darwin', 'Native acceptance requires macOS/Xcode; use --source-check elsewhere');
assert.match(args.udid || '', /^[A-Fa-f0-9-]{36}$/, 'Supply an explicit simulator UDID');
assert.ok(args['app-path'], 'Supply the built simulator .app path; do not preinstall');
const app = realpathSync(resolve(args['app-path']));
const buildRoot = realpathSync(join(root, 'desktop/gen/apple/build/arm64-sim'));
assert.ok(app.startsWith(buildRoot + sep) && app.endsWith('.app'), 'Only the local simulator build may be installed');
const output = resolve(args.output || join(root, 'build-artifacts/mobile-ios/native-acceptance'));
mkdirSync(output, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), 'ios-client-acceptance-'));
const report = { nativeAcceptance: 'RUNNING', bundle, udid: args.udid, preferencesRelative,
  uiAcceptance: args['skip-ui'] ? 'NOT_RUN' : 'PENDING',
  limitations: ['Cancellation is exercised by real superseding navigation; NSError -999 itself is not directly instrumented.'], cases: [] };
const events = [];
const commands = [];
const servers = [];
let installed = false;
let container;
let prefs;
let ownsUIHelpers = false;
const helperIDs = [`${bundle}.acceptancehost`, `${bundle}.acceptanceuitests.xctrunner`];
let mode = 'stable';
let command = '';
let reloaded = false;
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
const nativeBuildTimeout = 600000;
function run(binary, argv, allowFailure = false, timeout = 60000) {
  const result = spawnSync(binary, argv, { encoding: 'utf8', windowsHide: true, timeout, maxBuffer: 8 * 1024 * 1024 });
  commands.push({ at: Date.now(), binary, argv, status: result.status,
    stdout: result.stdout, stderr: result.stderr, error: result.error?.message });
  if (!allowFailure) assert.equal(result.status, 0, `${binary} ${argv.join(' ')}\n${result.stderr || result.stdout || result.error}`);
  return result;
}
function captureNativeFailure() {
  // A launch timeout can also leave simulator control commands unresponsive.
  // Collect independent, bounded evidence before cleanup; preserve the failure.
  run('/bin/ps', ['-axo', 'pid,ppid,state,etime,comm'], true, 5000);
  run('/usr/bin/log', ['show', '--last', '5m', '--style', 'compact', '--predicate',
    'process == "CoreSimulatorService" OR process == "SpringBoard" OR process == "runningboardd"'], true, 10000);
  run('xcrun', ['simctl', 'list', 'devices', 'available', '-j'], true, 10000);
  run('xcrun', ['simctl', 'io', args.udid, 'screenshot', join(output, 'failure-native.png')], true, 10000);
}
const sim = (...argv) => run('xcrun', ['simctl', ...argv]);
function applicationInventory() {
  // listapps emits a property list, unlike `list devices -j`.
  const inventory = join(scratch, 'applications.plist');
  writeFileSync(inventory, sim('listapps', args.udid).stdout);
  return JSON.parse(run('/usr/bin/plutil', ['-convert', 'json', '-o', '-', inventory]).stdout);
}
const saveReport = () => {
  writeFileSync(join(output, 'results.json'), JSON.stringify(report, null, 2) + '\n');
  writeFileSync(join(output, 'fixture-events.json'), JSON.stringify(events, null, 2) + '\n');
  writeFileSync(join(output, 'command-log.json'), JSON.stringify(commands, null, 2) + '\n');
};
function record(name, evidence) {
  report.cases.push({ name, status: 'PASS', evidence });
  console.log(`PASS ${name}`); saveReport();
}
let shotIndex = 0;
async function observe(label) {
  const filename = join(output, `${String(++shotIndex).padStart(3, '0')}-${label}.png`);
  sim('io', args.udid, 'screenshot', filename);
  const lines = JSON.parse(run(join(scratch, 'ocr'), [filename]).stdout);
  writeFileSync(filename.replace(/\.png$/, '.json'), JSON.stringify(lines, null, 2));
  // OCR line boundaries reflect wrapping, not changes to the expected UI text.
  return lines.join(' ').replace(/\s+/g, ' ').trim();
}
async function waitText(label, predicate, timeout = 45000) {
  const deadline = Date.now() + timeout;
  let text = '';
  do {
    text = await observe(label);
    if (predicate(text)) return text;
    await delay(1000);
  } while (Date.now() < deadline);
  assert.fail(`${label}: required native UI missing after ${timeout}ms; last OCR:\n${text}`);
}
const isSettings = (text) => /Server address/i.test(text) && /Open workspace/i.test(text);
const isFailure = (text) => isSettings(text) && /Cannot open the server page/i.test(text);
function verifyOwnedContainer() {
  assert.ok(installed && container, 'No mutation without installation ownership');
  const live = realpathSync(sim('get_app_container', args.udid, bundle, 'data').stdout.trim());
  assert.equal(live, container, 'Application container changed unexpectedly');
  assert.match(live, new RegExp(`[\\/]${args.udid}[\\/]data[\\/]Containers[\\/]Data[\\/]Application[\\/][A-Fa-f0-9-]+$`, 'i'));
  const relativePrefs = relative(live, prefs);
  assert.ok(!isAbsolute(relativePrefs) && !relativePrefs.startsWith('..' + sep));
  // Verify existing ancestors have no symlink escape before writing one known file.
  let ancestor = dirname(prefs);
  while (!existsSync(ancestor)) ancestor = dirname(ancestor);
  const resolved = realpathSync(ancestor);
  assert.ok(resolved === live || resolved.startsWith(live + sep));
}
function stop() {
  // XCTest may already have terminated a launched application at session end.
  const result = run('xcrun', ['simctl', 'terminate', args.udid, bundle], true);
  if (result.status !== 0) assert.match(`${result.stderr}\n${result.stdout}`, /not running|found nothing to terminate|No such process/i);
}
function launch() { sim('launch', args.udid, bundle, '-AppleLanguages', '(en)', '-AppleLocale', 'en_US'); }
async function restartFixture(nextMode = 'stable') {
  stop(); mode = nextMode; command = ''; reloaded = false;
  verifyOwnedContainer();
  mkdirSync(dirname(prefs), { recursive: true });
  writeFileSync(prefs, JSON.stringify({ server_url: base }));
  launch();
  await waitText(`${nextMode}-loaded`, (text) => /NATIVE FIXTURE READY/i.test(text));
}
async function listen(server) {
  servers.push(server);
  server.on('connection', (socket) => { socket.on('error', () => {}); });
  await new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
  return server.address().port;
}
let base;
async function testNativeUI(secondURL, phase = 'first') {
  // XcodeGen's documented bundle.ui-testing target and scheme test environment:
  // https://github.com/yonaskolb/XcodeGen/blob/master/Docs/ProjectSpec.md
  // A scratch host provides the normal UI-test target dependency. The product
  // app is launched independently by bundle ID and is never a build dependency.
  const project = join(scratch, `ui-project-${phase}`);
  mkdirSync(project);
  writeFileSync(join(project, 'Host.swift'), `import UIKit\n@main final class AcceptanceHost: UIResponder, UIApplicationDelegate {\nvar window: UIWindow?\nfunc application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {\nwindow = UIWindow(frame: UIScreen.main.bounds)\nwindow?.rootViewController = UIViewController()\nwindow?.makeKeyAndVisible()\nreturn true\n}\n}\n`);
  copyFileSync(join(root, '.github/scripts/ios-client-ui-tests.swift'), join(project, 'ClientAcceptanceTests.swift'));
  const spec = {
    name: 'ClientAcceptance', options: { deploymentTarget: { iOS: '15.0' } },
    settings: { base: { SWIFT_VERSION: '5.0', CODE_SIGNING_ALLOWED: 'NO', TARGETED_DEVICE_FAMILY: '1,2' } },
    targets: {
      AcceptanceHost: { type: 'application', platform: 'iOS', sources: ['Host.swift'],
        settings: { base: { PRODUCT_BUNDLE_IDENTIFIER: helperIDs[0], GENERATE_INFOPLIST_FILE: 'YES' } } },
      ClientAcceptanceTests: { type: 'bundle.ui-testing', platform: 'iOS', sources: ['ClientAcceptanceTests.swift'],
        dependencies: [{ target: 'AcceptanceHost' }], settings: { base: {
          PRODUCT_BUNDLE_IDENTIFIER: `${bundle}.acceptanceuitests`, TEST_TARGET_NAME: 'AcceptanceHost', GENERATE_INFOPLIST_FILE: 'YES',
        } } },
    },
    schemes: { ClientAcceptance: { build: { targets: { ClientAcceptanceTests: ['test'] } },
      test: { targets: ['ClientAcceptanceTests'], captureScreenshotsAutomatically: true,
        deleteScreenshotsWhenEachTestSucceeds: false, environmentVariables: {
          CLIENT_ACCEPTANCE_BUNDLE_ID: bundle, CLIENT_ACCEPTANCE_URL_A: base, CLIENT_ACCEPTANCE_URL_B: secondURL,
        } } } },
  };
  const specification = join(project, 'project.json');
  writeFileSync(specification, JSON.stringify(spec, null, 2));
  // Keep the exact generated contract with the artifacts after scratch cleanup.
  copyFileSync(specification, join(output, `ui-${phase}-project-spec.json`));
  run('xcodegen', ['generate', '--spec', specification, '--project', project]);
  const applications = applicationInventory();
  if (!ownsUIHelpers) for (const id of helperIDs) assert.ok(!Object.hasOwn(applications, id), `Refusing an existing helper installation: ${id}`);
  const resultBundle = `ui-${phase}.xcresult`;
  const logName = `ui-${phase}.log`;
  assert.ok(!existsSync(join(output, resultBundle)), 'Use a fresh --output directory; preserving earlier XCTest results');
  ownsUIHelpers = true;
  const argv = ['test', '-project', join(project, 'ClientAcceptance.xcodeproj'), '-scheme', 'ClientAcceptance',
    '-destination', `platform=iOS Simulator,id=${args.udid}`, '-derivedDataPath', join(scratch, `derived-data-${phase}`),
    '-resultBundlePath', join(output, resultBundle), '-parallel-testing-enabled', 'NO',
    '-maximum-concurrent-test-simulator-destinations', '1', '-disable-concurrent-destination-testing',
    `-only-testing:ClientAcceptanceTests/ClientAcceptanceTests/${phase === 'first' ? 'testFirstConfigurationKeyboardServerChangeAndRestart' : 'testOfflineRestartAllowsAddressChangeAndReconnect'}`,
    'CODE_SIGNING_ALLOWED=NO'];
  const log = openSync(join(output, logName), 'w');
  try {
    const code = await new Promise((done, reject) => {
      // Async is essential: localhost fixture HTTP must keep serving during XCTest.
      const child = spawn('xcodebuild', argv, { stdio: ['ignore', log, log], windowsHide: true, timeout: nativeBuildTimeout });
      child.once('error', reject); child.once('exit', (code, signal) => done({ code, signal }));
    });
    commands.push({ at: Date.now(), binary: 'xcodebuild', argv, ...code, log: logName });
    assert.equal(code.code, 0, `XCTest failed (${JSON.stringify(code)}); inspect ${logName} and ${resultBundle}`);
  } finally {
    closeSync(log);
    // Preserve directly viewable screenshots and accessibility attachments on failure.
    run('xcrun', ['xcresulttool', 'export', 'attachments', '--path', join(output, resultBundle),
      '--output-path', join(output, `ui-${phase}-attachments`)], true);
  }
  assert.equal(new URL(JSON.parse(readFileSync(prefs, 'utf8')).server_url).href, new URL(secondURL).href,
    'Actual UI server change did not save the second fixture address');
  report.uiAcceptance = 'PASS';
  record(phase === 'first' ? 'real-first-configuration-keyboard-server-change-and-process-restore' : 'real-offline-address-edit-and-reconnect',
    { phase, xcresult: resultBundle, log: logName, savedAddress: secondURL });
}
try {
  assert.equal(run('/usr/libexec/PlistBuddy', ['-c', 'Print CFBundleIdentifier', join(app, 'Info.plist')]).stdout.trim(), bundle);
  const devices = JSON.parse(sim('list', 'devices', 'available', '-j').stdout);
  const device = Object.values(devices.devices).flat().find((device) => device.udid === args.udid);
  assert.equal(device?.state, 'Booted', 'Explicit simulator must already be booted');
  const applications = applicationInventory();
  assert.ok(!Object.hasOwn(applications, bundle), 'Refusing an existing application from the installed-app inventory');
  const existing = run('xcrun', ['simctl', 'get_app_container', args.udid, bundle, 'data'], true);
  assert.notEqual(existing.status, 0, 'Refusing to overwrite an existing app; use a fresh disposable simulator');
  // Cold macOS framework compilation uses the existing native build budget.
  run('xcrun', ['swiftc', join(root, '.github/scripts/ios-client-ocr.swift'), '-o', join(scratch, 'ocr')], false, nativeBuildTimeout);
  run('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', join(scratch, 'key.pem'),
    '-out', join(scratch, 'cert.pem'), '-days', '1', '-subj', '/CN=localhost']);
  const tls = createHttpsServer({ key: readFileSync(join(scratch, 'key.pem')), cert: readFileSync(join(scratch, 'cert.pem')) }, (_req, res) => res.end('UNTRUSTED TLS'));
  tls.on('tlsClientError', (error) => events.push({ at: Date.now(), type: 'tls-client-error', message: error.message }));
  const tlsPort = await listen(tls);
  const closed = createServer();
  const closedPort = await listen(closed);
  await new Promise((done) => closed.close(done));
  const handleRequest = (req, res) => {
    const path = req.url.split('?')[0];
    events.push({ at: Date.now(), type: 'request', path, host: req.headers.host, cookie: req.headers.cookie || '', mode });
    res.setHeader('Cache-Control', 'no-store');
    if (path === '/api/status') { res.setHeader('Content-Type', 'application/json'); res.end('{"ok":true}'); return; }
    if (path === '/command') { const value = command; command = ''; res.end(value); return; }
    if (path === '/session') { res.setHeader('Set-Cookie', 'native_fixture=session; HttpOnly; SameSite=Lax; Path=/'); res.end('OK'); return; }
    if (path === '/cookie') { res.end((req.headers.cookie || '').includes('native_fixture=session') ? 'COOKIE VERIFIED' : 'COOKIE MISSING'); return; }
    if (path === '/hang') {
      res.on('close', () => events.push({ at: Date.now(), type: 'pending-closed' }));
      return; // No headers: actual provisional navigation, not a mocked delegate.
    }
    if (path === '/abort-frame') { res.destroy(); return; }
    if (path === '/late') {
      res.on('close', () => events.push({ at: Date.now(), type: 'superseded-closed' }));
      setTimeout(() => { events.push({ at: Date.now(), type: 'stale-abort' }); res.destroy(); }, 6000).unref();
      return;
    }
    if (path === '/' && mode === 'reload' && reloaded) { res.destroy(); return; }
    if (path === '/') reloaded = true;
    const marker = path === '/current' ? 'NATIVE CURRENT PAGE' : 'NATIVE FIXTURE READY';
    res.setHeader('Content-Type', 'text/html; charset=utf-8');
    res.end(`<!doctype html><html lang="en-US"><meta name="viewport" content="width=device-width,initial-scale=1"><style>body{font:24px system-ui;margin:24px;overflow-wrap:anywhere}a{display:block;min-height:44px}</style><body><h1>${marker}</h1><p id="cookie">Checking cookie</p><a href="app-settings://connection" data-client-connection-entry>Connection settings</a><script>
      (async()=>{await fetch('/session',{method:'POST'});document.querySelector('#cookie').textContent=await(await fetch('/cookie')).text()})();
      const tick=setInterval(async()=>{const command=await(await fetch('/command')).text();
        if(command==='refused')location.href='http://127.0.0.1:${closedPort}/unavailable';
        if(command==='dns')location.href='http://native-acceptance.invalid/unavailable';
        if(command==='tls')location.href='https://127.0.0.1:${tlsPort}/unavailable';
        if(command==='reload')location.reload();
        if(command==='timeout')location.href='/hang';
        if(command==='cancel'){location.href='/late';setTimeout(()=>location.href='/current',1000)}
        if(command==='iframe'){const frame=document.createElement('iframe');frame.src='/abort-frame';document.body.append(frame)}
      },250);
      </script></body></html>`);
  };
  const http = createServer(handleRequest);
  base = `http://127.0.0.1:${await listen(http)}`;
  const secondHttp = createServer(handleRequest);
  const secondURL = `http://127.0.0.1:${await listen(secondHttp)}`;
  sim('install', args.udid, app); installed = true;
  container = realpathSync(sim('get_app_container', args.udid, bundle, 'data').stdout.trim());
  prefs = join(container, preferencesRelative);
  verifyOwnedContainer();
  assert.ok(!existsSync(prefs), 'Fresh install unexpectedly contains saved configuration');
  launch();
  const first = await waitText('first-launch', isSettings);
  assert.match(first, /This\s+device\s+is\s+a\s+client/i);
  assert.ok(!/127\.0\.0\.1|4290|Local service mode|Start service/i.test(first));
  assert.ok(!existsSync(prefs));
  assert.equal(events.filter((event) => event.type === 'request').length, 0, 'Blank launcher contacted a fixture');
  record('first-launch-client-only-blank-selection', 'Real settings screenshot, no preference file, no server requests');
  if (args['skip-ui']) {
    report.limitations.push('XCTest explicitly skipped: address typing/change and keyboard acceptance NOT_RUN.');
  } else {
    try { await testNativeUI(secondURL); }
    catch (error) { report.uiAcceptance = 'FAIL'; throw error; }
  }
  await restartFixture();
  await waitText('cookie', (text) => /COOKIE VERIFIED/i.test(text));
  await delay(34000);
  assert.match(await observe('stable-after-watchdog'), /NATIVE FIXTURE READY/i);
  record('saved-selection-connect-cookie-and-stable-over-30s', 'Actual HTTP status, HttpOnly cookie round trip, visible page after 34 seconds');
  for (const failure of ['refused', 'reload', 'dns', 'tls', 'timeout']) {
    await restartFixture(failure);
    const start = Date.now(); const index = events.length;
    command = failure;
    await waitText(`${failure}-recovered`, isFailure, 50000);
    if (failure === 'timeout') assert.ok(Date.now() - start >= 28000, 'Timeout case did not exercise the provisional deadline');
    if (failure === 'tls') assert.ok(events.slice(index).some((event) => event.type === 'tls-client-error'), 'No actual TLS handshake observed');
    const healthy = await fetch(`${base}/api/status`);
    assert.equal(healthy.status, 200); assert.deepEqual(await healthy.json(), { ok: true });
    assert.equal(new URL(JSON.parse(readFileSync(prefs, 'utf8')).server_url).href, new URL(base).href);
    record(`${failure}-real-navigation-recovery`, { elapsedMs: Date.now() - start, savedOriginHealthy: true, savedSelectionRetained: true });
  }
  await restartFixture();
  let index = events.length; command = 'cancel';
  await delay(9000);
  assert.ok(events.slice(index).some((event) => event.path === '/late'));
  assert.ok(events.slice(index).some((event) => event.path === '/current'));
  assert.ok(events.slice(index).some((event) => event.type === 'superseded-closed'));
  assert.ok(events.slice(index).some((event) => event.type === 'stale-abort'));
  await delay(26000);
  assert.match(await observe('canceled-and-stale-stable'), /NATIVE CURRENT PAGE/i);
  record('real-superseded-and-late-aborted-navigation-preserves-current-page', 'Pending request superseded by /current, late abort, visible page beyond old 30-second deadline; no NSError instrumentation');
  await restartFixture();
  index = events.length; command = 'iframe';
  await delay(35000);
  assert.ok(events.slice(index).some((event) => event.path === '/abort-frame'));
  assert.match(await observe('iframe-failure-stable'), /NATIVE FIXTURE READY/i);
  record('actual-child-frame-failure-preserves-top-document', 'Aborted iframe request; top document visible after 35 seconds');
  stop();
  http.closeAllConnections(); await new Promise((done) => http.close(done));
  launch();
  const offline = await waitText('offline-process-restart', (text) => isSettings(text) && /Cannot connect to the server/i.test(text));
  assert.ok(offline.replace(/\s/g, '').includes(`127.0.0.1:${new URL(base).port}`), 'Saved address not visible after offline restart');
  assert.equal(new URL(JSON.parse(readFileSync(prefs, 'utf8')).server_url).href, new URL(base).href);
  record('offline-process-restart-retains-address-and-settings', 'Real process restart with fixture stopped; localized error and retained address visible');
  if (!args['skip-ui']) {
    try { await testNativeUI(secondURL, 'offline'); }
    catch (error) { report.uiAcceptance = 'FAIL'; throw error; }
  }
  report.nativeAcceptance = args['skip-ui'] ? 'INCOMPLETE' : 'PASS';
  if (args['skip-ui']) process.exitCode = 1;
} catch (error) {
  report.nativeAcceptance = 'FAIL'; report.error = error.stack;
  console.error(error); process.exitCode = 1;
  captureNativeFailure();
} finally {
  saveReport();
  for (const server of servers) { server.closeAllConnections?.(); server.close(); }
  if (installed) {
    run('xcrun', ['simctl', 'terminate', args.udid, bundle], true);
    // No app-container deletion or simulator erasure: uninstall only our own install.
    const cleanup = run('xcrun', ['simctl', 'uninstall', args.udid, bundle], true);
    report.cleanup = cleanup.status === 0 ? 'PASS' : 'FAIL';
    if (cleanup.status !== 0) { report.nativeAcceptance = 'FAIL'; process.exitCode = 1; }
  }
  if (ownsUIHelpers) {
    try {
      const applications = applicationInventory();
      for (const id of helperIDs.filter((id) => Object.hasOwn(applications, id))) {
        const cleanup = run('xcrun', ['simctl', 'uninstall', args.udid, id], true);
        if (cleanup.status !== 0) { report.nativeAcceptance = 'FAIL'; report.helperCleanup = 'FAIL'; process.exitCode = 1; }
      }
    } catch (error) { report.nativeAcceptance = 'FAIL'; report.helperCleanup = 'FAIL'; report.helperCleanupError = error.message; process.exitCode = 1; }
  }
  rmSync(scratch, { recursive: true, force: true });
  saveReport();
}
