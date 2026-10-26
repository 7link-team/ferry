/** Real browser acceptance for the shared variable form, using a disposable server. */
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, open, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const directory = await mkdtemp(join(tmpdir(), 'variables-ui-'));
const log = await open(join(directory, 'server.log'), 'w');
const port = await new Promise(resolvePort => {
  const socket = createServer(); socket.listen(0, '127.0.0.1', () => {
    const chosen = socket.address().port; socket.close(() => resolvePort(chosen));
  });
});
const origin = `http://127.0.0.1:${port}`;
const env = { ...process.env, FERRY_DATA_DIR: directory, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve('target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 150; i++) {
    assert.equal(server.exitCode, null, 'test server exited early');
    try { ready = (await fetch(`${origin}/api/status`)).ok; } catch {}
    if (ready) break;
    await new Promise(resolveWait => setTimeout(resolveWait, 200));
  }
  assert.ok(ready, 'test server ready');
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const api = async (path, data, method = data === undefined ? 'GET' : 'POST') => {
    const response = await context.request.fetch(`${origin}/api${path}`, { method, data, headers: { 'X-Ferry-Request': '1' } });
    assert.ok(response.ok(), `${path}: ${response.status()}`);
    return response.status() === 204 ? undefined : response.json();
  };
  execFileSync('python3', ['scripts/test-setup.py', `${origin}/api`, directory], { windowsHide: true });
  await api('/users', { email: 'variables@example.invalid', name: 'Variable acceptance', password: 'temporary-variable-password' });
  await api('/workspaces', { name: 'Variables', slug: 'ops' });
  const page = await context.newPage();
  const errors = [], reveals = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (request.url().endsWith('/actions/reveal')) reveals.push(request.url()); });
  const secret = "  ui-secret\n$literal 'quoted' \\tail  ";
  let globalId;
  for (const [group, scope] of [['ops', 'global'], ['deploy', 'deploy'], ['build', 'build']]) {
    await page.goto(`${origin}/ops/${group}/variables/new`);
    await page.getByLabel('名称', { exact: false }).fill('shared-url');
    await page.getByLabel('密钥值', { exact: true }).fill(secret);
    await page.getByRole('button', { name: '创建', exact: true }).click();
    await page.getByRole('cell', { name: 'shared-url', exact: true }).waitFor();
    const result = await api(`/workspaces/ops/variables?scope=${scope}`);
    assert.equal(result.total, 1, `form uses ${scope} default`);
    assert.equal(result.items[0].scope, scope);
    assert.equal(result.items[0].scope_ref, '0');
    assert.equal(result.items[0].value, undefined);
    assert.ok(!(await page.locator('body').textContent()).includes('ui-secret'));
    if (scope === 'global') globalId = result.items[0].id;
  }
  assert.equal(reveals.length, 0);
  await page.goto(`${origin}/ops/ops/variables/${globalId}`);
  await page.getByRole('button', { name: '查看明文（记录审计）', exact: true }).click();
  await page.getByLabel('变量明文').waitFor();
  assert.equal(await page.getByLabel('变量明文').textContent(), secret);
  await page.getByRole('button', { name: '隐藏明文', exact: true }).click();
  assert.equal(await page.getByLabel('变量明文').count(), 0);
  await page.getByRole('button', { name: '查看明文（记录审计）', exact: true }).click();
  await page.getByLabel('变量明文').waitFor();
  await page.goto(`${origin}/ops/deploy/variables`);
  await page.getByRole('cell', { name: 'shared-url', exact: true }).waitFor();
  assert.ok(!(await page.locator('body').textContent()).includes('ui-secret'));
  await page.goto(`${origin}/ops/ops/variables/${globalId}/edit`);
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('cell', { name: 'shared-url', exact: true }).waitFor();
  assert.equal((await api(`/workspaces/ops/variables/${globalId}/actions/reveal`, {})).value, secret);
  await page.goto(`${origin}/ops/ops/variables/${globalId}/edit`);
  await page.getByRole('checkbox', { name: /替换密钥值/ }).check();
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('cell', { name: 'shared-url', exact: true }).waitFor();
  assert.equal((await api(`/workspaces/ops/variables/${globalId}/actions/reveal`, {})).value, '');
  const audit = await api('/workspaces/ops/activity?resource=variables&action=reveal');
  assert.equal(audit.total, 4);
  assert.ok(!JSON.stringify(audit).includes('ui-secret'));
  assert.ok(!(await readFile(join(directory, 'server.log'), 'utf8')).includes('ui-secret'));
  assert.deepEqual(errors, []);
  console.log('PASS: three scope defaults, shared forms, secret never auto-fetched, explicit reveal/hide/audit, navigation clears value, omitted vs empty edits');
} finally {
  await browser?.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await once(server, 'exit'); }
  await log.close();
  await rm(directory, { recursive: true, force: true });
}
