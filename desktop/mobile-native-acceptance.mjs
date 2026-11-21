// Native-shell acceptance with disposable HTTP fixtures. Requires an Android
// emulator, a debug APK and Playwright; no production server or credentials.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { chromium } from 'playwright';

const apk = process.argv[2];
if (!apk) throw new Error('Usage: node desktop/mobile-native-acceptance.mjs <debug.apk>');
const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
const adbPath = process.env.ADB || (sdk ? join(sdk, 'platform-tools', process.platform === 'win32' ? 'adb.exe' : 'adb') : 'adb');
const config = JSON.parse(readFileSync(new URL('./tauri.conf.json', import.meta.url), 'utf8'));
const packageId = process.env.MOBILE_PACKAGE_ID || config.identifier;
const artifactDir = resolve('desktop/target-mobile/native-acceptance');
mkdirSync(artifactDir, { recursive: true });
const adb = (...args) => {
  const result = spawnSync(adbPath, ['-s', process.env.ANDROID_SERIAL || 'emulator-5554', ...args], { windowsHide: true, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr || result.stdout);
  return result.stdout.trim();
};
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
async function waitForNativeSettings(page, timeout = 45000) {
  const deadline = Date.now() + timeout;
  while (!page.isClosed() && Date.now() < deadline) {
    if (/#connection-settings\/navigation-failed/.test(page.url())) return;
    // Recovery intentionally cancels the failed navigation. CDP's load-bound
    // waitForURL can reject that intermediate abort before the local commit.
    await delay(100);
  }
  assert.fail(`Native failure recovery did not reach local settings: ${page.url()}`);
}
const originalSize = adb('shell', 'wm', 'size').match(/Override size: (\S+)/)?.[1];
const originalDensity = adb('shell', 'wm', 'density').match(/Override density: (\d+)/)?.[1];
function nativeScreenshot(name) {
  const result = spawnSync(adbPath, ['-s', process.env.ANDROID_SERIAL || 'emulator-5554', 'exec-out', 'screencap', '-p'], { windowsHide: true });
  assert.equal(result.status, 0);
  writeFileSync(`${artifactDir}/${name}.png`, result.stdout);
}
let browser;
const results = [];
const record = (name, evidence) => { results.push({ name, evidence }); console.log(`PASS ${name}`); };
function pickerNodes() {
  adb('shell', 'uiautomator', 'dump', '/sdcard/client-shell-picker.xml');
  const xml = adb('shell', 'cat', '/sdcard/client-shell-picker.xml');
  return [...xml.matchAll(/<node\b[^>]*>/g)].map(([node]) => Object.fromEntries([...node.matchAll(/([\w-]+)="([^"]*)"/g)].map((match) => [match[1], match[2]])));
}
function tapPickerNode(predicate) {
  const node = pickerNodes().find(predicate);
  assert.ok(node, 'Required native document-picker control was not found');
  const bounds = node.bounds.match(/\[(\d+),(\d+)\]\[(\d+),(\d+)\]/).slice(1).map(Number);
  adb('shell', 'input', 'tap', String(Math.round((bounds[0] + bounds[2]) / 2)), String(Math.round((bounds[1] + bounds[3]) / 2)));
}
async function fixture(name) {
  let failPages = false;
  const server = createServer((request, response) => {
    if (request.url === '/api/status') {
      response.writeHead(200, { 'Content-Type': 'application/json' });
      response.end('{"ok":true}'); return;
    }
    if (failPages) { response.destroy(); return; }
    if (request.url === '/session') {
      response.writeHead(200, { 'Set-Cookie': 'demo_session=fixture; HttpOnly; SameSite=Lax; Path=/' }); response.end('ok'); return;
    }
    response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    response.end(`<!doctype html><html lang="en-US"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${name}</title><style>*{box-sizing:border-box}body{margin:16px;font:16px system-ui}input,button,a{min-height:44px;max-width:100%}input{display:block;width:100%}button{margin-top:12px}#result{overflow-wrap:anywhere}</style><body><h1>${name}</h1><label>Demo name<input id="demo-name"></label><button id="login">Demo login</button><p id="result"></p><label>Attachment<input id="attachment" type="file"></label><script>document.getElementById('login').onclick=async()=>{await fetch('/session',{method:'POST'});document.getElementById('result').textContent='Session saved';};</script></body></html>`);
  });
  await new Promise((done) => server.listen(0, '0.0.0.0', done));
  return { server, url: `http://10.0.2.2:${server.address().port}`, failPages: (value) => { failPages = value; }, close: async () => { server.closeAllConnections(); await new Promise((done) => server.close(done)); } };
}
async function connect() {
  let last;
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      const pid = adb('shell', 'pidof', packageId).split(' ')[0];
      if (!pid) throw new Error('app has not started');
      adb('forward', 'tcp:9224', `localabstract:webview_devtools_remote_${pid}`);
      browser = await chromium.connectOverCDP('http://127.0.0.1:9224');
      const pages = browser.contexts().flatMap((context) => context.pages());
      assert.equal(pages.length, 1);
      return pages[0];
    } catch (error) { last = error; await delay(300); }
  }
  throw last;
}
async function restart() {
  if (browser) await browser.close().catch(() => {});
  adb('shell', 'am', 'force-stop', packageId);
  adb('shell', 'am', 'start', '-n', `${packageId}/.MainActivity`);
  return connect();
}
async function settings(page) {
  await page.locator('#native-connection-settings').click();
  await page.waitForURL(/#connection-settings/);
  await page.locator('#open').waitFor();
  await page.waitForFunction(() => document.getElementById('url')?.value.startsWith('http'));
}
const first = await fixture('Disposable server one');
const second = await fixture('Disposable server two');
let firstClosed = false;
let completed = false;
try {
  assert.equal(adb('shell', 'pm', 'list', 'packages', packageId), '', 'Use an emulator without an existing app; the acceptance script will not clear someone else\'s data.');
  adb('shell', 'wm', 'size', '640x1280');
  adb('shell', 'wm', 'density', '320');
  adb('install', resolve(apk));
  let page = await restart();
  await page.locator('#url').waitFor();
  await page.waitForFunction(() => document.getElementById('url')?.getAttribute('placeholder') === 'https://');
  assert.equal(await page.locator('#url').inputValue(), '');
  assert.equal(await page.locator('.updates').isVisible(), false);
  await page.locator('#open').click();
  assert.notEqual(await page.locator('#connection-error').textContent(), '');
  assert.equal(await page.evaluate(() => innerWidth), 320);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  nativeScreenshot('first-launch-320');
  record('first launch has no default server or local-service path', page.url());
  await page.locator('#url').fill(first.url);
  await page.locator('#url').click();
  await delay(600);
  const keyboard = adb('shell', 'dumpsys', 'input_method');
  assert.match(keyboard, /mInputShown=true|mIsInputViewShown=true|isInputViewShown=true/);
  const focusedVisible = await page.evaluate(() => {
    const box = document.activeElement.getBoundingClientRect();
    return box.top >= 0 && box.bottom <= (visualViewport?.height || innerHeight);
  });
  assert.equal(focusedVisible, true);
  nativeScreenshot('keyboard-320');
  adb('shell', 'input', 'keyevent', '4');
  record('native keyboard leaves focused server input visible at 320 CSS pixels', 'Android IME visibility and focused-input geometry');
  await page.locator('#open').click();
  await page.waitForURL(`${first.url}/`);
  await page.locator('#native-connection-settings').waitFor();
  await page.locator('#demo-name').fill('Disposable demo');
  await page.locator('#login').click();
  await page.getByText('Session saved', { exact: true }).waitFor();
  assert.equal((await browser.contexts()[0].cookies(first.url)).some((cookie) => cookie.name === 'demo_session'), true);
  const access = await page.evaluate(async (serverUrl) => {
    const invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) return { bridgeAvailable: false, read: 'bridge_unavailable', write: 'bridge_unavailable' };
    const attempt = async (command, args) => {
      try { await invoke(command, args); return 'ALLOWED'; }
      catch (error) { return String(error); }
    };
    return { bridgeAvailable: true, read: await attempt('connection_settings'), write: await attempt('open_workspace', { serverUrl }) };
  }, second.url);
  if (access.bridgeAvailable) {
    assert.match(access.read, /settings_unavailable|not allowed|denied|forbidden/i);
    assert.match(access.write, /settings_unavailable|not allowed|denied|forbidden/i);
  }
  record('single native webview connects and remote page cannot write settings', first.url);
  await page.screenshot({ path: `${artifactDir}/connected.png` });
  await delay(34000);
  assert.equal(page.url(), `${first.url}/`);
  assert.equal(await page.locator('#native-connection-settings').isVisible(), true);
  record('healthy remote page stays connected beyond the finite watchdog deadline', page.url());
  const unavailable = createServer();
  await new Promise((done) => unavailable.listen(0, '0.0.0.0', done));
  const badOrigin = `http://10.0.2.2:${unavailable.address().port}`;
  await new Promise((done) => unavailable.close(done));
  await page.goto(badOrigin, { waitUntil: 'commit', timeout: 10000 }).catch(() => {});
  await waitForNativeSettings(page);
  await page.waitForFunction((expected) => document.getElementById('url')?.value === expected, `${first.url}/`);
  assert.equal((await fetch(`${first.url.replace('10.0.2.2', '127.0.0.1')}/api/status`)).status, 200);
  nativeScreenshot('bad-origin-recovery');
  record('unavailable second origin recovers while saved server remains healthy', page.url());
  await page.locator('#open').click();
  await page.waitForURL(`${first.url}/`);
  await page.locator('#native-connection-settings').waitFor();
  await delay(500);
  first.failPages(true);
  await page.reload({ waitUntil: 'commit', timeout: 10000 }).catch(() => {});
  await waitForNativeSettings(page);
  await page.waitForFunction((expected) => document.getElementById('url')?.value === expected, `${first.url}/`);
  assert.equal((await fetch(`${first.url.replace('10.0.2.2', '127.0.0.1')}/api/status`)).status, 200);
  nativeScreenshot('same-url-failed-reload');
  record('failed same-URL reload rejects old document readiness while status remains healthy', page.url());
  first.failPages(false);
  await page.locator('#open').click();
  await page.waitForURL(`${first.url}/`);
  await settings(page);
  assert.equal(await page.locator('#url').inputValue(), `${first.url}/`);
  await delay(500);
  assert.match(page.url(), /#connection-settings/);
  record('settings sentinel returns locally without reconnect loop', page.url());
  await page.locator('#open').click();
  await page.waitForURL(`${first.url}/`);
  page = await restart();
  await page.waitForURL(`${first.url}/`);
  record('fresh process restores persisted server selection', page.url());
  adb('shell', 'input', 'keyevent', '4');
  await page.waitForURL(/#connection-settings/);
  await delay(500);
  assert.match(page.url(), /#connection-settings/);
  record('Android Back returns to editable settings without reconnect loop', page.url());
  await page.locator('#url').fill(second.url);
  await page.locator('#open').click();
  await page.waitForURL(`${second.url}/`);
  await settings(page);
  assert.equal(await page.locator('#url').inputValue(), `${second.url}/`);
  await page.locator('#url').fill(first.url);
  await page.locator('#open').click();
  await page.waitForURL(`${first.url}/`);
  record('changing server replaces the persisted selection', second.url);
  await first.close(); firstClosed = true;
  await page.reload({ waitUntil: 'commit', timeout: 12000 }).catch(() => {});
  await waitForNativeSettings(page, 15000);
  await page.locator('#connection-error').filter({ hasText: /./ }).waitFor();
  await page.screenshot({ path: `${artifactDir}/offline-recovery.png` });
  record('real server shutdown and failed page load recover to local settings', page.url());
  page = await restart();
  await page.locator('#url').waitFor();
  await page.locator('#connection-error').filter({ hasText: /./ }).waitFor();
  assert.equal(await page.locator('#url').inputValue(), `${first.url}/`);
  assert.equal(await page.locator('#url').isEditable(), true);
  record('offline restart retains editable server address', page.url());
  await page.locator('#url').fill(second.url);
  await page.locator('#open').click();
  await page.waitForURL(`${second.url}/`);
  record('offline recovery can change server and reconnect', page.url());
  const attachmentName = 'client-shell-attachment.txt';
  const attachmentPath = `${artifactDir}/${attachmentName}`;
  writeFileSync(attachmentPath, 'Disposable native file-selection fixture.\n');
  adb('push', attachmentPath, `/sdcard/Download/${attachmentName}`);
  await page.locator('#attachment').click();
  tapPickerNode((node) => node['content-desc'] === 'Show roots');
  tapPickerNode((node) => node['resource-id'] === 'android:id/title' && node.text === 'Downloads');
  tapPickerNode((node) => node['resource-id'] === 'android:id/title' && node.text === attachmentName);
  await page.waitForFunction(() => document.getElementById('attachment')?.files?.length === 1, null, { timeout: 15000 });
  assert.equal(await page.locator('#attachment').evaluate((input) => input.files[0].name), attachmentName);
  nativeScreenshot('selected-attachment');
  record('native document picker returns the selected file to the remote WebView', attachmentName);
  adb('shell', 'rm', `/sdcard/Download/${attachmentName}`);
  completed = true;
} finally {
  writeFileSync(`${artifactDir}/results.json`, JSON.stringify({ completed, results }, null, 2));
  if (browser) await browser.close().catch(() => {});
  if (!firstClosed) await first.close();
  await second.close();
  for (const args of [
    ['shell', 'wm', 'size', originalSize || 'reset'],
    ['shell', 'wm', 'density', originalDensity || 'reset'],
    ['forward', '--remove', 'tcp:9224'],
    ['shell', 'rm', '-f', '/sdcard/client-shell-picker.xml'],
  ]) {
    try { adb(...args); } catch (error) { console.error(`Cleanup: ${String(error)}`); }
  }
  // Keep the disposable installed app and artifact directory for review.
}
