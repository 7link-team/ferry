/** Isolated real-server/Runner/browser acceptance for Agent management (no model calls). */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'ai-agents-'));
const output = resolve('data/maintenance/agent-p1-validation');
await mkdir(output, { recursive: true });
const report = { work, checks: [] };
const pass = name => { report.checks.push(name); console.log('PASS', name); };
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const bind = `127.0.0.1:${listener.address().port}`, origin = `http://${bind}`;
await new Promise(r => listener.close(r));
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: bind };
for (const key of ['FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE']) delete env[key];
// The caller must supply a dedicated empty database when checking another engine.
if (process.env.FERRY_TEST_DATABASE_URL) env.FERRY_DATABASE_URL = process.env.FERRY_TEST_DATABASE_URL;
const suffix = process.platform === 'win32' ? '.exe' : '';
const log = await open(join(work, 'server.log'), 'a');
const runnerLog = await open(join(work, 'runner.log'), 'a');
let server, runner, browser, legacy;
async function wait(fn) {
  for (let i = 0; i < 200; i++) { if (await fn()) return; await new Promise(r => setTimeout(r, 150)); }
  throw Error('Acceptance condition timed out');
}
async function stop(proc) {
  if (!proc || proc.exitCode !== null || proc.signalCode !== null) return;
  const ended = once(proc, 'exit');
  proc.kill('SIGTERM'); await ended;
}
async function launch() {
  server = spawn(resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${suffix}`), [], { env, windowsHide: true, stdio: ['ignore', log.fd, log.fd] });
  await wait(async () => { assert.equal(server.exitCode, null); try { return (await fetch(origin + '/api/status')).ok; } catch { return false; } });
}
try {
  await launch();
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const owner = await browser.newContext({ viewport: { width: 1366, height: 900 } });
  const reader = await browser.newContext();
  const api = context => async (path, data, method = data ? 'POST' : 'GET', status) => {
    const response = await context.request.fetch(origin + '/api' + path, {
      method, headers: { 'X-Ferry-Request': '1', 'Accept-Language': 'en-US' }, ...(data ? { data } : {})
    });
    assert(status ? response.status() === status : response.ok(), `${method} ${path}: ${response.status()} ${await response.text()}`);
    return response.status() === 204 ? null : response.json();
  };
  const admin = api(owner), member = api(reader);
  await admin('/setup', { name: 'Owner', email: 'owner@example.invalid', password: 'temporary-agent-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const ws = await admin('/workspaces', { name: 'Agent acceptance', slug: 'agents' });
  const other = await admin('/workspaces', { name: 'Other', slug: 'other' });
  const root = `/workspaces/${ws.id}`, collection = root + '/ai-agents';
  await member('/users', { name: 'Reader', email: 'reader@example.invalid', password: 'temporary-reader-password' });
  await member('/workspace-memberships', { code: (await admin(root + '/members/invitation')).code });
  const actor = (await admin(root + '/members')).items.find(r => r.email === 'reader@example.invalid');
  const human = (await admin(root + '/members')).items.find(r => r.email === 'owner@example.invalid');
  const role = await admin(root + '/roles', { name: 'Agent user', permissions: ['ai-agents.read', 'ai-agents.use'] });
  await admin(root + '/members/' + actor.id, { role_id: role.id, version: actor.version }, 'PATCH');
  const host = await admin(root + '/nodes', { name: 'Agent Runner', connection: 'runner', runner_auto_update: 'off' });
  const profile=await admin(root+'/ai-models',{name:'Configured Claude',kind:'cli',runtime:'claude',models:['catalog-a','catalog-b'],default_model:'',state:'enabled'});
  const token = await admin(root + '/runner-tokens', { host_id: host.id });
  runner = spawn(resolve(process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${suffix}`), ['--server', origin, '--token', token.token, '--state-dir', join(work, 'runner')], { windowsHide: true, stdio: ['ignore', runnerLog.fd, runnerLog.fd] });
  await wait(async () => { assert.equal(runner.exitCode, null); return (await admin(root + '/nodes/' + host.id)).state === 'online'; });
  const errors = [];
  const page = await owner.newPage(); page.on('pageerror', e => errors.push(e.message));
  await page.goto(`${origin}/en-US${collection}/new`);
  await page.getByLabel('Name', { exact: true }).fill('Review Agent');
  await page.getByRole('combobox', { name: 'Human owner', exact: true }).click();
  await page.getByRole('option', { name: 'Owner', exact: true }).click();
  await page.getByRole('combobox', { name: 'Model profile', exact: true }).click();
  await page.getByRole('option', { name: 'Configured Claude', exact: true }).click();
  await page.getByRole('combobox', { name: 'Model', exact: true }).click();
  await page.getByRole('option', { name: 'catalog-b', exact: true }).click();
  for(const name of ['Runner host','Runtime'])assert.equal(await page.getByRole('combobox',{name,exact:true}).count(),0);
  for (const name of ['Working directory', 'Description', 'Tool permission']) assert.equal(await page.getByLabel(name, { exact: true }).count(), 0);
  await page.getByLabel('Agent instructions', { exact: true }).fill('Review the current working directory.');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await page.waitForURL(`**${collection}`);
  const created = (await admin(collection)).items.find(r => r.name === 'Review Agent');
  assert(created); const url = collection + '/' + created.id;
  assert.equal(created.owner_id, human.user_id);
  assert.equal(created.model_profile_id,profile.id);assert.equal(created.model,'catalog-b');assert.equal(created.host_id,'0');
  const probeAgent=await admin(collection,{name:'Probe Agent',runtime:'codex',host_id:host.id,owner_id:human.user_id,visibility:'owner',state:'enabled'});
  const probeUrl=collection+'/'+probeAgent.id;
  pass('real browser selects configured model profile/catalog and persists Agent without Runner or runtime fields');
  await member(url, undefined, 'GET', 404);
  assert.equal((await member(collection)).total, 0);
  await admin(`/workspaces/${other.id}/ai-agents/${created.id}`, undefined, 'GET', 404);
  await member(url + '/probes', {}, 'POST', 403);
  await member(collection, created, 'POST', 403);
  pass('private Agent, cross-workspace access, create/probe permissions enforced');
  await page.goto(`${origin}/en-US${url}/edit`);
  await page.getByRole('combobox', { name: 'Who can use', exact: true }).click();
  await page.getByRole('option', { name: 'Workspace members', exact: true }).click();
  await page.getByRole('combobox', { name: 'State', exact: true }).click();
  await page.getByRole('option', { name: 'Disabled', exact: true }).click();
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await page.waitForURL(`**${collection}`);
  let saved = await admin(url);
  assert.equal(saved.state, 'disabled'); assert.equal(saved.visibility, 'workspace');
  await admin(url, { ...created, name: 'Stale overwrite' }, 'PATCH', 409);
  const history = await admin(url + '/revisions'); assert.equal(history.total, 2);
  assert.notEqual(created.revision_id, saved.revision_id);
  pass('browser edit, disable, immutable revisions and stale-update conflict');
  const limited = await reader.newPage(); limited.on('pageerror', e => errors.push(e.message));
  const denied = [];
  limited.on('response', r => { if (r.url().includes('/api/') && r.status() === 403) denied.push(new URL(r.url()).pathname); });
  await limited.goto(`${origin}/en-US${url}`);
  await limited.getByRole('heading', { name: 'Review Agent', exact: true }).waitFor();
  assert.equal(await limited.getByText('Configuration history', { exact: true }).count(),0);
  assert.equal(await limited.getByRole('button', { name: 'Check environment', exact: true }).count(), 0);
  await limited.waitForLoadState('networkidle'); assert.deepEqual(denied, []);
  assert.equal((await member(collection)).total, 1);
  await member(root + '/nodes', undefined, 'GET', 403);
  pass('shared Agent is readable without host/member options requests or terminal grant');
  for (const runtime of ['codex', 'claude']) {
    const probeSaved = await admin(probeUrl);
    await admin(probeUrl, { ...probeSaved, runtime }, 'PATCH');
    const result = await admin(probeUrl + '/probes', {});
    assert.equal(result.status.runtime, runtime);
    assert.equal(result.status.directory_exists, false);
    assert(result.status.execution_user);
    assert(['logged_in', 'logged_out', 'unknown'].includes(result.status.authentication));
    report[runtime] = result.status;
    assert(!JSON.stringify(result).includes('access_token'));
  }
  pass('real Server → Runner → Codex/Claude environment probes, no model usage');
  const probeSaved = await admin(probeUrl); await admin(probeUrl, { ...probeSaved, cwd: join(work, 'missing') }, 'PATCH');
  assert.equal((await admin(probeUrl + '/probes', {})).status.directory_exists, false);
  await page.goto(`${origin}/en-US${url}`);
  await page.getByRole('heading', { name: 'Identity', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Check environment', exact: true }).count(),0);
  assert.equal(await page.getByText('Directory missing or inaccessible', { exact: true }).count(), 0);
  await page.screenshot({ path: join(output, 'agent-environment.png'), fullPage: true });
  const translations = JSON.parse(await readFile('src/shared/i18n/locales/agents.json', 'utf8'));
  for (const [i, locale] of ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'].entries()) {
    await page.goto(`${origin}/${locale}${url}`);
    await page.getByRole('heading', { name: translations.identity[i], exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: translations.probe[i], exact: true }).count(),0);
    assert.equal(await page.getByRole('heading', { name: translations.revisions[i], exact: true }).count(),0);
    await page.getByRole('heading', { name: 'Review Agent', exact: true }).waitFor();
  }
  pass('six locale detail labels omit environment diagnostics and revision sections; user content preserved');
  await page.setViewportSize({ width: 390, height: 844 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({ path: join(output, 'agent-mobile.png'), fullPage: true });
  pass('Agent management omits session directory/permission/description and technical sections; API environment probe and mobile view work');
  await stop(runner);
  await wait(async () => (await admin(root + '/nodes/' + host.id)).state !== 'online');
  runner = spawn(resolve(process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${suffix}`), ['--server', origin, '--state-dir', join(work, 'runner')], { env: { ...process.env, PATH: join(work, 'empty-path') }, windowsHide: true, stdio: ['ignore', runnerLog.fd, runnerLog.fd] });
  await wait(async () => { assert.equal(runner.exitCode, null); return (await admin(root + '/nodes/' + host.id)).state === 'online'; });
  const missingCli = await admin(probeUrl + '/probes', {});
  assert.equal(missingCli.status.error, 'missing'); assert.equal(missingCli.status.executable, null);
  pass('Runner with no CLI on PATH returns explicit missing status');
  await stop(runner);
  await wait(async () => (await admin(root + '/nodes/' + host.id)).state !== 'online');
  const offline = await owner.request.post(origin + '/api' + probeUrl + '/probes', { headers: { 'X-Ferry-Request': '1' }, data: {} });
  assert(!offline.ok()); pass('offline Runner probe fails explicitly');
  const legacyHost = await admin(root + '/nodes', { name: 'Legacy Runner', connection: 'runner', runner_auto_update: 'off' });
  const legacyToken = await admin(root + '/runner-tokens', { host_id: legacyHost.id });
  legacy = new WebSocket(origin.replace('http', 'ws') + '/api/runners/connect');
  legacy.addEventListener('open', () => legacy.send(JSON.stringify({ version: 1, auth: { type: 'enroll', token: legacyToken.token }, hostname: 'legacy', os: process.platform, arch: process.arch, agent_version: 'test' })));
  legacy.addEventListener('message', e => { if (JSON.parse(e.data).type === 'ping') legacy.send(JSON.stringify({ type: 'pong' })); });
  await wait(async () => (await admin(root + '/nodes/' + legacyHost.id)).state === 'online');
  const legacyAgent = await admin(collection, { ...await admin(probeUrl), name: 'Legacy Agent', host_id: legacyHost.id }, 'POST', 201);
  await admin(collection + '/' + legacyAgent.id + '/probes', {}, 'POST', 409);
  legacy.close();
  pass('legacy Runner without capability bit receives explicit upgrade error');
  const before = await admin(url);
  await stop(server); await launch();
  assert.deepEqual(await admin(url), before);
  assert.equal((await admin(url + '/revisions')).total, 2);
  const probeHistory=await admin(probeUrl + '/revisions');
  assert.equal(probeHistory.total, 3); // initial Codex, Claude change, missing-directory change
  assert.equal((await member(collection)).total, 1);
  assert.deepEqual(errors, []);
  pass('server restart preserves Agent, revisions and member visibility; no browser errors');
  report.status = 'PASS';
} catch (error) {
  if (browser) for (const [i, context] of browser.contexts().entries()) for (const [j, page] of context.pages().entries()) {
    await page.screenshot({ path: join(output, `failure-${i}-${j}.png`), fullPage: true }).catch(() => {});
    await writeFile(join(output, `failure-${i}-${j}.txt`), page.url() + '\n' + await page.locator('body').innerText()).catch(() => {});
  }
  report.status = 'FAIL'; report.error = String(error); throw error;
} finally {
  if (browser) await browser.close();
  if (legacy) legacy.close();
  await stop(runner); await stop(server);
  await log.close(); await runnerLog.close();
  await writeFile(join(output, 'result.json'), JSON.stringify(report, null, 2) + '\n');
}
