/** Real HTTP/browser acceptance against an owned disposable database only. */
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'login-sessions-'));
const evidence = join(work, 'evidence'); await mkdir(evidence);
const listener = createServer();
await new Promise(done => listener.listen(0, '127.0.0.1', done));
const bind = `127.0.0.1:${listener.address().port}`, origin = `http://${bind}`;
await new Promise(done => listener.close(done));
const env = { ...process.env, FERRY_DATA_DIR: join(work, 'server'), FERRY_BIND: bind };
for (const key of ['FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE']) delete env[key];
const log = await open(join(work, 'server.log'), 'a');
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server.exe'), [], {
  env, windowsHide: true, stdio: ['ignore', log.fd, log.fd],
});
const report = { work, origin, checks: [], status: 'RUNNING' };
let browser, page;
const pass = name => { report.checks.push(name); console.log('PASS', name); };
const wait = async fn => {
  for (let i = 0; i < 150; i++) { if (await fn()) return; await new Promise(done => setTimeout(done, 200)); }
  throw Error('Acceptance timed out');
};
const sql = (query, params = []) => {
  const result = spawnSync('python', ['-c',
    'import sys,json,sqlite3; p=json.load(sys.stdin); c=sqlite3.connect(p["db"],timeout=10); r=c.execute(p["query"],p["params"]); rows=r.fetchall(); c.commit(); print(json.dumps(rows))'], {
    input: JSON.stringify({ db: join(env.FERRY_DATA_DIR, 'ferry.sqlite'), query, params }),
    encoding: 'utf8', windowsHide: true,
  });
  assert.equal(result.status, 0, result.stderr); return JSON.parse(result.stdout);
};
const hash = value => createHash('sha256').update(value).digest('hex');
const rawCookies = cookies => cookies.map(item => `${item.name}=${item.value}`).join('; ');
const request = context => async (path, body, method = body ? 'POST' : 'GET', expected = 200) => {
  const result = await context.request.fetch(origin + '/api' + path, {
    method, headers: { 'X-Ferry-Request': '1', 'Accept-Language': 'en-US' }, ...(body ? { data: body } : {}),
  });
  assert.equal(result.status(), expected, `${method} ${path}: ${await result.text()}`);
  return result;
};
const explicit = async (path, cookies, method = 'POST') => fetch(origin + '/api' + path, {
  method, headers: { Cookie: rawCookies(cookies), 'X-Ferry-Request': '1' },
});
const expire = cookies => {
  const token = cookies.find(item => item.name === 'ferry_session'); assert(token);
  sql('UPDATE sessions SET expires_at=? WHERE token_hash=?', [Date.now() - 1, hash(token.value)]);
};
try {
  await wait(async () => { assert.equal(server.exitCode, null); try { return (await fetch(origin + '/api/status')).ok; } catch { return false; } });
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const admin = await browser.newContext(), user = await browser.newContext();
  const a = request(admin), u = request(user);
  const credentials = { email: 'sessions@example.invalid', password: 'temporary-session-acceptance', name: 'Session owner' };
  await a('/setup', { ...credentials, token: (await readFile(join(env.FERRY_DATA_DIR, 'setup-token'), 'utf8')).trim() });
  const workspace = await (await a('/workspaces', { name: 'Session acceptance', slug: 'session-acceptance' })).json();
  const config = await (await a('/platform/settings/1')).json();
  assert.equal(config.session_access_minutes, 30); assert.equal(config.session_refresh_days, 14); assert.equal(config.session_auto_renew, true);
  const initial = await admin.cookies();
  const access = initial.find(item => item.name === 'ferry_session'), refresh = initial.find(item => item.name === 'session_refresh');
  assert(access?.httpOnly && refresh?.httpOnly); assert.equal(refresh.path, '/api/sessions');
  const metadata = (await (await a('/me')).json()).session;
  assert.equal(metadata.access_ttl_ms, 1_800_000);
  assert(Math.abs(metadata.access_expires_at_ms - metadata.server_time_ms - 1_800_000) < 15_000);
  assert(Math.abs(metadata.refresh_expires_at_ms - metadata.server_time_ms - 1_209_600_000) < 15_000);
  const rows = sql('SELECT s.token_hash,r.token_hash,r.expires_at FROM sessions s JOIN session_refresh_tokens r ON r.session_id=s.id WHERE s.token_hash=?', [hash(access.value)]);
  assert.equal(rows[0][0], hash(access.value)); assert.equal(rows[0][1], hash(refresh.value));
  pass('setup issues two HttpOnly opaque cookies; only hashes persisted; default short access and fixed refresh deadlines');

  // A foreground interaction renews an expired access grant without dropping the current form draft.
  const target = origin + '/en-US/platform/settings/1/edit?section=' + encodeURIComponent('Login sessions');
  page = await admin.newPage(); const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto(target);
  const input = page.locator('input[name="session_access_minutes"]'); await input.waitFor();
  assert.equal(await input.inputValue(), '30');
  await input.fill('17');
  const second = await admin.newPage(); await second.goto(target); await second.locator('input[name="session_access_minutes"]').waitFor();
  const workspaceDraft = await admin.newPage();
  await workspaceDraft.goto(origin + `/en-US/workspaces/${workspace.id}/variables/new`);
  const description = workspaceDraft.locator('textarea[name="description"]');
  await description.waitFor(); await description.fill('unsaved workspace draft');
  // Both pages have real near-expiry hints, while the actual server row is expired afterward.
  const before = await admin.cookies();
  sql('UPDATE sessions SET expires_at=? WHERE token_hash=?', [Date.now() + 20_000, hash(before.find(item => item.name === 'ferry_session').value)]);
  await Promise.all([page, second, workspaceDraft].map(tab => tab.evaluate(async () => {
    const response = await fetch('/api/me'); const identity = await response.json();
    // The normal App reload path observes the new metadata through its shared API.
    window.dispatchEvent(new Event('session:renewed'));
    return identity.user.id;
  })));
  await new Promise(done => setTimeout(done, 500));
  expire(before);
  let rotations = 0;
  const count = response => { if (response.url() === origin + '/api/sessions/refresh' && response.status() === 200) rotations++; };
  page.on('response', count); second.on('response', count); workspaceDraft.on('response', count);
  const idle = await page.evaluate(async () => (await fetch('/api/platform/settings/1')).status);
  assert.equal(idle, 401);
  await new Promise(done => setTimeout(done, 500));
  assert.equal(rotations, 0); assert.equal(await input.inputValue(), '17');
  await Promise.all([page.locator('input[name="session_access_minutes"]').click(),
    second.locator('input[name="session_access_minutes"]').click(), description.click()]);
  await wait(async () => rotations === 1);
  await a('/me'); assert.equal(await input.inputValue(), '17'); assert.equal(rotations, 1);
  await new Promise(done => setTimeout(done, 250));
  assert.equal(await description.inputValue(), 'unsaved workspace draft');
  pass('idle expired read does not renew; real activity across three tabs rotates once and retains platform and workspace drafts');
  await second.close(); await workspaceDraft.close();

  const stored = await (await a('/platform/settings/1')).json();
  await a('/platform/settings/1', { ...stored, session_access_minutes: 5, session_refresh_days: 2 }, 'PATCH');
  const still = (await (await a('/me')).json()).session;
  assert.equal(still.refresh_expires_at_ms, metadata.refresh_expires_at_ms);
  await a('/sessions', credentials);
  const changed = (await (await a('/me')).json()).session;
  assert(Math.abs(changed.access_expires_at_ms - changed.server_time_ms - 300_000) < 10_000);
  assert(Math.abs(changed.refresh_expires_at_ms - changed.server_time_ms - 172_800_000) < 10_000);
  pass('duration changes apply to new logins; existing refresh deadline stays fixed');

  const previous = await admin.cookies(); expire(previous);
  assert.equal((await explicit('/me', previous, 'GET')).status, 401);
  const rotated = await a('/sessions/refresh', undefined, 'POST');
  const rotatedMeta = await rotated.json();
  assert.equal(rotatedMeta.refresh_expires_at_ms, changed.refresh_expires_at_ms);
  assert.equal((await explicit('/sessions/refresh', previous)).status, 401);
  const current = await admin.cookies();
  assert.notEqual(current.find(item => item.name === 'ferry_session').value, previous.find(item => item.name === 'ferry_session').value);
  assert.equal((await explicit('/sessions', previous, 'DELETE')).status, 200);
  await a('/me', undefined, 'GET', 401);
  await a('/sessions/refresh', undefined, 'POST', 401);
  pass('expired access refreshes with rotation and fixed deadline; replay rejected; stale-cookie logout revokes rotated pair');

  await a('/sessions', credentials);
  const competing = await admin.cookies();
  const competition = await Promise.all([explicit('/sessions/refresh', competing), explicit('/sessions/refresh', competing)]);
  assert.deepEqual(competition.map(result => result.status).sort(), [200, 401]);
  assert.equal(competition.find(result => result.status === 401).headers.get('set-cookie'), null);
  // The explicit client intentionally did not replace the browser's cookies. The old pair is still logout-only.
  await a('/sessions', undefined, 'DELETE'); await a('/sessions', credentials);
  const disable = await (await a('/platform/settings/1')).json();
  await a('/platform/settings/1', { ...disable, session_auto_renew: false }, 'PATCH');
  await a('/sessions/refresh', undefined, 'POST', 403); await a('/me');
  const enable = await (await a('/platform/settings/1')).json();
  await a('/platform/settings/1', { ...enable, session_auto_renew: true }, 'PATCH');
  pass('concurrent refresh has one winner with no loser cookies; disabling renewal blocks refresh immediately without revoking access');

  await u('/users', { email: 'member@example.invalid', password: 'temporary-member-password', name: 'Restricted member' });
  await u('/platform/settings/1', undefined, 'GET', 403);
  await u(`/workspaces/${workspace.id}/workflows`, undefined, 'GET', 404);
  await u('/sessions/refresh', undefined, 'POST');
  await u(`/workspaces/${workspace.id}/workflows`, undefined, 'GET', 404);
  pass('refresh does not grant administrator or other-workspace access');

  const shortConfig = await (await a('/platform/settings/1')).json();
  await a('/platform/settings/1', { ...shortConfig, session_access_minutes: 1 }, 'PATCH');
  const restoring = await browser.newContext(); const restoreRequest = request(restoring);
  await restoreRequest('/sessions', credentials);
  const restoredPage = await restoring.newPage();
  let shortRotations = 0;
  restoredPage.on('request', request => { if (request.url() === origin + '/api/sessions/refresh') shortRotations++; });
  await restoredPage.goto(target);
  await restoredPage.locator('input[name="session_access_minutes"]').waitFor();
  await restoredPage.locator('input[name="session_access_minutes"]').click();
  await new Promise(done => setTimeout(done, 250));
  assert.equal(shortRotations, 0);
  expire(await restoring.cookies());
  await restoredPage.goto(target);
  await restoredPage.locator('input[name="session_access_minutes"]').waitFor();
  assert(!restoredPage.url().includes('/login'));
  await restoreRequest('/me'); await restoring.close();
  assert.equal(shortRotations, 1);
  pass('one-minute access does not rotate immediately; a fresh foreground load restores expired access once through the refresh cookie');

  for (const locale of ['en-US', 'zh-CN', 'ms-MY']) {
    await page.goto(origin + `/${locale}/platform/settings/1/edit`);
    // English group identifiers remain stable URL identities; localized menu selection resolves on load.
    await page.goto(origin + `/${locale}/platform/settings/1/edit?section=` + encodeURIComponent('Login sessions'));
    await page.locator('input[name="session_access_minutes"]').waitFor();
    for (const width of [1440, 900, 390, 320]) {
      await page.setViewportSize({ width, height: 900 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.screenshot({ path: join(evidence, `${locale}-${width}.png`) });
    }
  }
  assert.deepEqual(errors, []);
  pass('shared session settings render in three required locales and four widths without horizontal overflow or page errors');
  report.status = 'PASS';
} catch (error) {
  report.status = 'FAIL'; report.error = error.stack;
  if (page) { await page.screenshot({ path: join(evidence, 'failure.png') }).catch(() => {}); await writeFile(join(evidence, 'failure.txt'), await page.locator('body').innerText().catch(() => '')); }
  throw error;
} finally {
  await writeFile(join(evidence, 'result.json'), JSON.stringify(report, null, 2));
  await browser?.close();
  if (server.exitCode === null) { const ended = once(server, 'exit'); server.kill(); await ended; }
  await log.close(); console.log(JSON.stringify(report));
}
