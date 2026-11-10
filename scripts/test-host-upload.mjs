/** Isolated real Server -> temporary Runner -> filesystem/browser acceptance. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, open, readFile, writeFile, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';
const evidence = resolve('data/maintenance/host-upload'); await mkdir(evidence, { recursive: true });
const work = await mkdtemp(join(tmpdir(), 'host-upload-'));
const probe = createServer(); await new Promise(r => probe.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${probe.address().port}`; await new Promise(r => probe.close(r));
const env = { ...process.env, FERRY_DATA_DIR: join(work, 'server'), FERRY_BIND: origin.slice(7) };
for (const key of ['FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE']) delete env[key];
const log = await open(join(work, 'server.log'), 'a'), rlog = await open(join(work, 'runner.log'), 'a');
let server, runner, browser;
const report = { work, checks: [], screenshots: [], production_host_tested: false, native_posix_tested: false };
const wait = async fn => { for (let n = 0; n < 240; n++) { if (await fn()) return; await new Promise(r => setTimeout(r, 250)); } throw Error('Acceptance timeout'); };
try {
  const suffix = process.platform === 'win32' ? '.exe' : '';
  server = spawn(resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${suffix}`), [], { env, windowsHide: true, stdio: ['ignore', log.fd, log.fd] });
  await wait(async () => { assert.equal(server.exitCode, null); try { return (await fetch(origin + '/api/status')).ok; } catch { return false; } });
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const context = await browser.newContext(), page = await context.newPage(), errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const api = async (path, data, method = data ? 'POST' : 'GET', status) => {
    const r = await context.request.fetch(origin + '/api' + path, { method, headers: { 'X-Ferry-Request': '1', 'Accept-Language': 'en-US' }, ...(data ? { data } : {}) });
    assert(status ? r.status() === status : r.ok(), `${method} ${path}: ${r.status()} ${await r.text()}`);
    return r.status() === 204 ? null : r.json();
  };
  await api('/setup', { name: 'Upload fixture owner', email: 'upload@example.invalid', password: 'temporary-upload-fixture-password', token: (await readFile(join(env.FERRY_DATA_DIR, 'setup-token'), 'utf8')).trim() });
  const space = await api('/workspaces', { name: 'Upload acceptance', slug: 'upload-acceptance' });
  const other = await api('/workspaces', { name: 'Other workspace', slug: 'other-upload' });
  const root = `/workspaces/${space.id}`, host = await api(root + '/nodes', { name: 'Temporary file runner', connection: 'runner', state: 'offline' });
  const enrollment = await api(root + '/runner-tokens', { host_id: host.id });
  runner = spawn(resolve(process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${suffix}`), ['--server', origin, '--token', enrollment.token, '--state-dir', join(work, 'runner')], { env, windowsHide: true, stdio: ['ignore', rlog.fd, rlog.fd] });
  await wait(async () => { assert.equal(runner.exitCode, null); return (await api(root + '/nodes/' + host.id)).state === 'online'; });
  const target = join(work, 'uploads'); await mkdir(target); const directory = target.replaceAll('\\', '/');
  const files = root + `/nodes/${host.id}/files`;
  const limited = await browser.newContext();
  const limitedRequest = async (path, data, method = 'POST') => limited.request.fetch(origin + '/api' + path, { method, headers: { 'X-Ferry-Request': '1' }, data });
  assert((await limitedRequest('/users', { name: 'Limited member', email: 'limited@example.invalid', password: 'temporary-limited-password' })).ok());
  assert((await limitedRequest('/workspace-memberships', { code: space.join_code })).ok());
  const member = (await api(root + '/members')).items.find(m => m.email === 'limited@example.invalid');
  const role = await api(root + '/roles', { name: 'Host reader without file access', permissions: ['hosts.read'] });
  await api(root + '/members/' + member.id, { role_id: role.id, version: member.version }, 'PATCH');
  assert.equal((await limitedRequest(files + '/permissions', { path: directory + '/absent', mode: 420 }, 'PATCH')).status(), 403);
  assert.equal((await limitedRequest(files + '/content?path=' + encodeURIComponent(directory + '/denied.txt'), Buffer.from('denied'), 'PUT')).status(), 403);
  report.checks.push('Member without hosts.files cannot upload or change file permissions');
  await api(files + '/permissions', { path: directory + '/absent', mode: 512 }, 'PATCH', 400);
  await api(`/workspaces/${other.id}/nodes/${host.id}/files/permissions`, { path: directory + '/absent', mode: 420 }, 'PATCH', 404);
  if (process.platform === 'win32') await api(files + '/permissions', { path: directory + '/absent', mode: 420 }, 'PATCH', 409);
  report.checks.push('Permissions API: invalid mode, cross-workspace and unsupported Windows rejection');
  const url = origin + `/en-US/workspaces/${space.id}/nodes/${host.id}/workspace?tool=files&directory=${encodeURIComponent(directory)}`;
  await page.goto(url); await page.getByRole('button', { name: 'Upload file', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Upload file', exact: true }).click();
  const dialog = page.getByRole('dialog'); await dialog.waitFor();
  await dialog.locator('input[type=file]:not([webkitdirectory])').setInputFiles([
    { name: 'one.txt', mimeType: 'text/plain', buffer: Buffer.from('first real upload') },
    { name: 'empty.txt', mimeType: 'text/plain', buffer: Buffer.alloc(0) },
    { name: 'second.txt', mimeType: 'text/plain', buffer: Buffer.from('second real upload') },
  ]);
  assert.equal((await api(files + '?path=' + encodeURIComponent(directory))).items.length, 0);
  for (const width of [1440, 900, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const path = join(evidence, `upload-selection-${width}.png`); await page.screenshot({ path }); report.screenshots.push(path);
  }
  await dialog.getByRole('button', { name: 'Confirm upload', exact: true }).click();
  await wait(async () => (await dialog.getByText('Host confirmed', { exact: true }).count()) === 3);
  assert.equal(await readFile(join(target, 'one.txt'), 'utf8'), 'first real upload');
  assert.equal((await stat(join(target, 'empty.txt'))).size, 0);
  report.checks.push('Multi-file confirmation gate, actual Runner writes and zero-byte ACK; four-width dialog');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  const folder = join(work, 'source-folder'); await mkdir(join(folder, 'nested'), { recursive: true });
  await writeFile(join(folder, 'nested', 'inside.txt'), 'folder structure');
  await page.getByRole('button', { name: 'Upload file', exact: true }).click();
  await dialog.locator('input[webkitdirectory]').setInputFiles(folder);
  await dialog.getByRole('button', { name: 'Confirm upload', exact: true }).click();
  await wait(async () => (await dialog.getByText('Host confirmed', { exact: true }).count()) > 0);
  assert.equal(await readFile(join(target, 'source-folder', 'nested', 'inside.txt'), 'utf8'), 'folder structure');
  report.checks.push('Native folder picker preserves directory hierarchy through real Runner');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  const network = await context.newCDPSession(page);
  await network.send('Network.enable');
  await network.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: 256 * 1024 });
  await page.getByRole('button', { name: 'Upload file', exact: true }).click();
  await dialog.locator('input[type=file]:not([webkitdirectory])').setInputFiles({ name: 'progress.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(2 * 1024 * 1024, 77) });
  await dialog.getByRole('button', { name: 'Confirm upload', exact: true }).click();
  await wait(async () => { const value = Number(await dialog.locator('li [role=progressbar]').getAttribute('aria-valuenow')); return value > 0 && value < 100; });
  assert(await page.evaluate(() => { const event = new Event('beforeunload', { cancelable: true }); window.dispatchEvent(event); return event.defaultPrevented; }));
  await page.screenshot({ path: join(evidence, 'upload-active-320.png') });
  await wait(async () => (await dialog.getByText('Host confirmed', { exact: true }).count()) === 1);
  assert.equal((await readFile(join(target, 'progress.bin'))).length, 2 * 1024 * 1024);
  await network.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  await network.detach();
  report.checks.push('Throttled real 2MiB transfer shows partial per-file progress and beforeunload guard, then host confirms exact bytes');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  // A browser-side 403 fixture checks visible per-file failure and deliberate retry.
  await page.route('**/files/content?**', async route => {
    if (route.request().method() === 'PUT') await route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: 'Fixture: destination permission denied' }) });
    else await route.continue();
  });
  await page.getByRole('button', { name: 'Upload file', exact: true }).click();
  await dialog.locator('input[type=file]:not([webkitdirectory])').setInputFiles({ name: 'retry.txt', mimeType: 'text/plain', buffer: Buffer.from('retry success') });
  await dialog.getByRole('button', { name: 'Confirm upload', exact: true }).click();
  await dialog.getByRole('alert').filter({ hasText: 'Fixture: destination permission denied' }).waitFor();
  await page.screenshot({ path: join(evidence, 'upload-failure-320.png') });
  await page.unroute('**/files/content?**'); await dialog.getByRole('button', { name: 'Retry unfinished files', exact: true }).click();
  await wait(async () => (await dialog.getByText('Host confirmed', { exact: true }).count()) === 1);
  assert.equal(await readFile(join(target, 'retry.txt'), 'utf8'), 'retry success');
  report.checks.push('Injected HTTP403 displays on file and succeeds only after explicit retry');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  const downloaded = await context.request.get(origin + '/api' + files + '/content?path=' + encodeURIComponent(directory + '/one.txt'));
  assert(downloaded.ok()); assert.equal((await downloaded.body()).toString(), 'first real upload');
  // Brand assets are served by the embedded application, not an API JSON fallback.
  for (const asset of ['logo.png', 'logo-256.png', 'logo-32.png', 'logo.ico']) {
    const response = await context.request.get(origin + '/brand/' + asset); assert(response.ok()); assert(!response.headers()['content-type'].includes('text/html'));
    assert.deepEqual(await response.body(), await readFile(resolve('public/brand', asset)));
  }
  await page.goto(origin + `/en-US/workspaces/${space.id}/nodes`);
  await page.locator('header img[src="/brand/logo-256.png"]').waitFor();
  assert.equal(await page.locator('header img[src="/brand/logo-256.png"]').count(), 1);
  const login = await browser.newContext(), lp = await login.newPage();
  for (const theme of ['light', 'dark']) for (const width of [1440, 900, 390, 320]) {
    await lp.emulateMedia({ colorScheme: theme }); await lp.setViewportSize({ width, height: 900 });
    await lp.goto(origin + '/en-US/login');
    await lp.locator('img[src="/brand/logo-256.png"]').waitFor();
    await lp.waitForFunction(() => document.querySelector('img[src="/brand/logo-256.png"]')?.naturalWidth > 0);
    assert.equal(await lp.evaluate(() => document.documentElement.classList.contains('dark')), theme === 'dark');
    assert(await lp.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const path = join(evidence, `brand-login-${theme}-${width}.png`); await lp.screenshot({ path }); report.screenshots.push(path);
  }
  report.checks.push('Actual download byte equality; embedded logo assets and login/navigation across four widths and light/dark media');
  assert.deepEqual(errors, []); report.result = 'PASS';
} catch (error) { report.result = 'FAIL'; report.error = error.stack; process.exitCode = 1; }
finally {
  await writeFile(join(evidence, 'acceptance.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
  await browser?.close(); runner?.kill(); server?.kill(); await log.close(); await rlog.close();
}
