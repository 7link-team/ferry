#!/usr/bin/env node
import {previewFetch as fetch, previewResolverRules} from './preview-test-network.mjs';
// Live preview-hole check: an isolated server, a real Runner, a real Vite dev
// server and a real browser (docs/preview-holes-design.md section 14).
// Build first: `cargo build -p ferry-server -p ferry-runner` and `npm run build`.
// Evidence lands in data/maintenance/preview-holes-<stamp>/.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import http from 'node:http';
import { join, resolve } from 'node:path';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const exe = process.platform === 'win32' ? '.exe' : '';
const serverBinary = resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${exe}`);
const runnerBinary = resolve(process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${exe}`);
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const work = resolve(`data/maintenance/preview-holes-${stamp}`);
await mkdir(work, { recursive: true });

const processes = [];
const result = { started: new Date().toISOString(), checks: {}, result: 'FAIL' };
const pass = (name, detail = true) => {
  result.checks[name] = detail;
  console.log(`PASS ${name}`);
};

function freePort() {
  return new Promise((done) => {
    const probe = createServer();
    probe.listen(0, '127.0.0.1', () => {
      const { port } = probe.address();
      probe.close(() => done(port));
    });
  });
}

async function start(name, command, args, env = {}) {
  const log = await import('node:fs').then((fs) => fs.openSync(join(work, `${name}.log`), 'a'));
  const child = spawn(command, args, {
    env: { ...process.env, ...env },
    stdio: ['ignore', log, log],
    windowsHide: true,
  });
  processes.push(child);
  return child;
}

function stop(child) {
  if (child.exitCode === null) child.kill();
}

async function until(label, check, seconds = 60) {
  const deadline = Date.now() + seconds * 1000;
  let last;
  while (Date.now() < deadline) {
    try {
      const value = await check();
      if (value) return value;
    } catch (error) {
      last = error.message;
    }
    await new Promise((done) => setTimeout(done, 300));
  }
  throw new Error(`${label} timed out (${last ?? 'no value'})`);
}

const serverPort = await freePort();
const vitePort = await freePort();
const echoPort = await freePort();
const base = `http://127.0.0.1:${serverPort}`;
let cookie = '';

async function api(path, body, method = body === undefined ? 'GET' : 'POST') {
  const response = await fetch(base + path, {
    method,
    headers: { 'content-type': 'application/json', 'x-ferry-request': '1', cookie },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const setCookie = response.headers.get('set-cookie');
  if (setCookie?.startsWith('ferry_session=')) cookie = setCookie.split(';')[0];
  const text = await response.text();
  if (!response.ok) throw new Error(`${method} ${path} -> ${response.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

let browser;
let echo;
try {
  const data = join(work, 'server-data');
  await start('server', serverBinary, [], {
    FERRY_DATA_DIR: data,
    FERRY_BIND: `127.0.0.1:${serverPort}`,
    FERRY_DATABASE_URL: '',
    FERRY_NATS_URL: '',
  });
  await until('server ready', async () => (await api('/api/status')).ok);
  const setupToken = (await readFile(join(data, 'setup-token'), 'utf8')).trim();
  await api('/api/setup', {
    email: 'preview@example.invalid',
    name: 'Preview test',
    password: 'isolated-preview-password',
    token: setupToken,
  });
  if (!cookie) {
    await api('/api/sessions', { email: 'preview@example.invalid', password: 'isolated-preview-password' });
  }
  assert.ok(cookie, 'console session cookie');
  const access = await api('/api/platform/settings/1');
  const ws = (await api('/api/workspaces', { name: 'Preview test', slug: 'preview' })).id;
  const root = `/api/workspaces/${ws}`;
  const host = (await api(`${root}/nodes`, { name: 'Preview Runner', connection: 'runner' })).id;
  const token = (await api(`${root}/runner-tokens`, { host_id: host })).token;
  const runner = await start('runner', runnerBinary, [
    '--server', base, '--token', token, '--state-dir', join(work, 'runner'),
  ]);
  await until('runner online', async () => (await api(`${root}/nodes/${host}`)).state === 'online');
  pass('real_runner_online');

  const configure = async (domain) => {
    const settings = await api('/api/platform/settings/1');
    return api('/api/platform/settings/1', {version:settings.version, registration_enabled:settings.registration_enabled, public_url:base, preview_domain:domain}, 'PATCH');
  };
  const createBody = {name:'Domain check',host_id:host,target_port:echoPort,ttl_hours:1};
  const createStatus = async () => (await fetch(base+root+'/holes', {method:'POST',headers:{'content-type':'application/json','x-ferry-request':'1',cookie},body:JSON.stringify(createBody)})).status;
  assert.equal(await createStatus(),409);
  assert.equal((await fetch(base+'/holes/'+'a'.repeat(64)+'/')).status,404);
  for (const alias of ['localhost:'+serverPort, 'lan-alias.test:'+serverPort]) {
    assert.equal((await fetch(base+'/api/status', {headers:{host:alias}})).status,200);
  }
  pass('unconfigured_preview_disabled_and_console_aliases_allowed');
  await configure('preview.test');
  assert.equal((await fetch(`http://unknown.preview.test:${serverPort}/api/me`)).status,404);
  pass('unknown_preview_host_never_reaches_console');

  // A Vite app served at the root of its token domain.
  const vite = await api(`${root}/holes`, {
    name: 'Vite preview', host_id: host, target_port: vitePort, strip_prefix: false, ttl_hours: 1,
  });
  const app = join(work, 'app');
  await mkdir(app, { recursive: true });
  await writeFile(join(app, 'api.json'), JSON.stringify({ from: 'relative api' }));
  await writeFile(join(app, 'sw.js'), 'self.addEventListener("fetch", () => {});');
  await writeFile(join(app, 'index.html'),
    '<!doctype html><html><body><h1 id="version">v1</h1><pre id="report">pending</pre>' +
    '<script type="module" src="./main.js"></script></body></html>');
  const mainSource = (version) => `
document.getElementById('version').textContent = ${JSON.stringify(version)};
const report = {};
const attempt = async (name, run) => {
  try { report[name] = { ok: true, value: await run() }; }
  catch (error) { report[name] = { ok: false, error: String(error && error.name || error) }; }
};
await attempt('relative_api', async () => (await fetch('./api.json')).json());
await attempt('console_me', async () => {
  const response = await fetch('/api/me', { credentials: 'include' });
  return { status: response.status, body: await response.text() };
});
await attempt('console_holes', async () => {
  const response = await fetch('/api/workspaces/${ws}/holes', { credentials: 'include' });
  return { status: response.status, body: await response.text() };
});
await attempt('local_storage', async () => { localStorage.setItem('k', 'v'); return localStorage.getItem('k'); });
await attempt('cookie_read', async () => document.cookie);
await attempt('service_worker', async () => {
  if (!('serviceWorker' in navigator)) throw new Error('unavailable');
  await navigator.serviceWorker.register('./sw.js');
  return 'registered';
});
await attempt('console_dom', async () => {
  const frame = document.createElement('iframe');
  frame.src = '/';
  document.body.append(frame);
  await new Promise((done) => frame.addEventListener('load', done, { once: true }));
  return frame.contentDocument ? frame.contentDocument.title : null;
});
document.getElementById('report').textContent = JSON.stringify(report);
`;
  await writeFile(join(app, 'main.js'), mainSource('v1'));
  const viteProcess = () => start('vite', process.execPath, [
    resolve('node_modules/vite/bin/vite.js'), app,
    '--base', '/', '--port', String(vitePort), '--strictPort', '--host', '127.0.0.1',
  ]);
  let viteServer = await viteProcess();
  await until('vite ready', async () => (await fetch(`http://127.0.0.1:${vitePort}/`)).ok);

  // A local JSON service that reports original paths and received headers.
  echo = http.createServer((request, response) => {
    response.setHeader('content-type', 'application/json');
    response.setHeader('set-cookie', 'upstream=1; Path=/');
    response.end(JSON.stringify({ path: request.url, headers: request.headers }));
  });
  await new Promise((done) => echo.listen(echoPort, '127.0.0.1', done));
  const echoHole = await api(`${root}/holes`, {
    name: 'Echo API', host_id: host, target_port: echoPort, strip_prefix: true, ttl_hours: 1,
  });

  assert.equal((await fetch(vite.url)).status,200);
  assert.equal((await fetch(echoHole.url)).status,200);
  const probed = await api(`${root}/holes/${vite.id}/probes`, {});
  assert.equal(probed.probe_state, 'reachable');
  pass('explicit_probe_reachable');

  browser = await chromium.launch({
    headless: true,
    args: previewResolverRules ? [`--host-resolver-rules=${previewResolverRules}`] : [],
    ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}),
  });
  // The visitor is logged in to the console in the same browser: the case the
  // sandbox exists for.
  const context = await browser.newContext();
  const [name, value] = cookie.split('=');
  await context.addCookies([{ name, value, domain: '127.0.0.1', path: '/', httpOnly: true, sameSite: 'Lax' }]);
  const page = await context.newPage();
  const consoleLines = [];
  page.on('console', (message) => consoleLines.push(message.text()));
  await page.goto(`${vite.url}`);
  await page.waitForFunction(() => document.getElementById('report')?.textContent !== 'pending', null, { timeout: 30000 });
  const report = JSON.parse(await page.locator('#report').textContent());
  result.checks.sandbox_report = report;
  assert.deepEqual(report.relative_api, { ok: true, value: { from: 'relative api' } });
  pass('previewed_app_reaches_its_own_api');
  for (const probe of ['console_me', 'console_holes']) {
    // Either the read is blocked outright or it carries no session.
    const leaked = report[probe].ok && report[probe].value.status === 200;
    assert.ok(!leaked, `${probe} leaked console data: ${JSON.stringify(report[probe])}`);
  }
  pass('console_api_not_readable_from_preview');
  assert.equal(report.local_storage.ok, false, 'storage must be unavailable');
  assert.ok(!report.cookie_read.ok || report.cookie_read.value === '', 'cookies must be unreadable');
  assert.equal(report.service_worker.ok, false, 'service worker must not register');
  assert.ok(!report.console_dom.ok || report.console_dom.value === null, 'console DOM must be unreadable');
  pass('sandbox_blocks_storage_cookie_service_worker_and_dom');

  // HMR over the proxied WebSocket.
  await writeFile(join(app, 'main.js'), mainSource('v2'));
  await page.waitForFunction(() => document.getElementById('version')?.textContent === 'v2', null, { timeout: 30000 });
  pass('vite_hmr_through_hole');

  // The console cookie never reaches the previewed service.
  const echoPage = await context.newPage();
  await echoPage.goto(`${echoHole.url}data?x=1`);
  const echoed = JSON.parse(await echoPage.locator('body').textContent());
  assert.equal(echoed.path, '/data?x=1');
  assert.equal(echoed.headers.cookie, undefined, 'console cookie forwarded');
  assert.equal(echoed.headers.host, `localhost:${echoPort}`);
  pass('upstream_never_sees_console_cookie');
  const upstreamApi = await fetch(echoHole.url+'api/me?scope=preview');
  assert.equal((await upstreamApi.json()).path,'/api/me?scope=preview');
  const another = await api('/api/workspaces',{name:'Other preview workspace',slug:'other-preview'});
  assert.equal((await fetch(`${base}/api/workspaces/${another.id}/holes/${echoHole.id}`,{headers:{cookie}})).status,404);
  pass('preview_api_is_upstream_and_workspace_details_remain_isolated');
  await echoPage.close();

  // Monitoring: the list follows the local service going down and up.
  const stateOf = async (id) => (await api(`${root}/holes/${id}`)).probe_state;
  stop(viteServer);
  let since = Date.now();
  await until('unreachable reported', async () => (await stateOf(vite.id)) === 'unreachable', 40);
  result.checks.down_detected_ms = Date.now() - since;
  viteServer = await viteProcess();
  since = Date.now();
  await until('reachable reported', async () => (await stateOf(vite.id)) === 'reachable', 40);
  result.checks.up_detected_ms = Date.now() - since;
  pass('monitor_tracks_local_service', { down_ms: result.checks.down_detected_ms, up_ms: result.checks.up_detected_ms });

  // UI screenshots across widths and languages.
  const ui = await context.newPage();
  const overflow = {};
  for (const [width, height] of [[1440, 900], [900, 1100], [390, 844], [320, 720]]) {
    await ui.setViewportSize({ width, height });
    for (const locale of width === 390 ? ['zh-CN', 'en-US', 'ms-MY'] : ['zh-CN']) {
      await ui.goto(`${base}/${locale}/workspaces/${ws}/holes`);
      await ui.getByText('Vite preview').first().waitFor({ timeout: 30000 });
      overflow[`${locale}-${width}`] = await ui.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
      await ui.screenshot({ path: join(work, `list-${locale}-${width}.png`), fullPage: true });
    }
    await ui.goto(`${base}/zh-CN/workspaces/${ws}/holes/${vite.id}`);
    await ui.getByText(vite.url).first().waitFor({ timeout: 30000 });
    await ui.screenshot({ path: join(work, `detail-zh-CN-${width}.png`), fullPage: true });
    await ui.goto(`${base}/zh-CN/workspaces/${ws}/holes/new`);
    await ui.waitForLoadState('networkidle');
    await ui.screenshot({ path: join(work, `form-zh-CN-${width}.png`), fullPage: true });
  }
  result.checks.page_overflow_px = overflow;
  assert.ok(Object.values(overflow).every((px) => px <= 0), `page scrolls sideways: ${JSON.stringify(overflow)}`);
  pass('list_detail_form_four_widths_no_page_overflow');

  // Revocation cuts an open HMR socket and the link stops working. A fresh
  // page: the earlier one already lost its socket when Vite was restarted.
  const live = await context.newPage();
  const liveLines = [];
  live.on('console', (message) => liveLines.push(message.text()));
  await live.goto(`${vite.url}`);
  await until('HMR connected', async () => liveLines.some((line) => /\[vite\] connected/i.test(line)), 30);
  assert.ok((await api(`${root}/holes/${vite.id}`)).active_connections > 0);
  const revokedAt = Date.now();
  await api(`${root}/holes/${vite.id}/revocations`, {});
  await until('HMR socket cut', async () => liveLines.some((line) => /connection lost/i.test(line)), 40);
  result.checks.revoke_cut_ms = Date.now() - revokedAt;
  const after = await fetch(`${vite.url}`);
  assert.equal(after.status, 404);
  pass('revoke_cuts_live_socket_and_link');
  assert.equal((await api(`${root}/holes/${vite.id}`)).active_connections, 0);
  pass('no_connections_left_after_revoke');

  // Changing the root keeps the record/token, but does not expose the old host.
  const oldUrl = echoHole.url;
  await configure('nested.preview.test');
  const changed = await api(`${root}/holes/${echoHole.id}`);
  assert.ok(new URL(changed.url).hostname.endsWith('.nested.preview.test'));
  assert.equal((await fetch(changed.url+'api/me')).status,200);
  assert.equal((await fetch(oldUrl+'api/me')).status,404);
  const label = new URL(changed.url).hostname.split('.')[0];
  const alphabet = 'abcdefghijklmnopqrstuvwxyz234567';
  const bits = [...label].map(c=>alphabet.indexOf(c).toString(2).padStart(5,'0')).join('').slice(0,256);
  const hex = Buffer.from(bits.match(/.{8}/g).map(b=>parseInt(b,2))).toString('hex');
  const legacy = await fetch(`${base}/holes/${hex}/api/me?x=1`, {redirect:'manual'});
  assert.equal(legacy.status,308);
  assert.equal(legacy.headers.get('location'),changed.url+'api/me?x=1');
  pass('nested_preview_domain_and_legacy_redirect_preserve_path_query');
  await configure('');
  assert.equal(await createStatus(),409);
  assert.equal((await fetch(changed.url)).status,404);
  assert.equal((await fetch(`${base}/holes/${hex}/api/me`,{redirect:'manual'})).status,404);
  pass('clearing_domain_disables_existing_and_new_links');
  await configure('preview.test');

  // Runner offline is reported.
  stop(runner);
  await until('runner offline reported', async () => (await stateOf(echoHole.id)) === 'runner_offline', 60);
  pass('monitor_reports_runner_offline');

  result.result = 'PASS';
} catch (error) {
  result.error = String(error.stack || error);
  console.error(error);
  process.exitCode = 1;
} finally {
  await browser?.close().catch(() => {});
  echo?.close();
  echo?.closeAllConnections?.();
  for (const child of processes) stop(child);
  result.finished = new Date().toISOString();
  await writeFile(join(work, 'result.json'), JSON.stringify(result, null, 2));
  console.log(`evidence: ${work}`);
}
