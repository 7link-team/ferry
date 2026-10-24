/** Real browser -> server -> runner acceptance. No project commits or persistent data.
 * Run after npm run build and cargo build -p ferry-server -p ferry-runner.
 * PLAYWRIGHT_MODULE may point to an existing Playwright index.mjs installation.
 */
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, rm, mkdir, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const port = () => new Promise((resolve, reject) => {
  const s = createServer(); s.on('error', reject);
  s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolve(p)); });
});
const work = await mkdtemp(join(tmpdir(), 'build-ui-'));
const children = [], files = [];
let browser, page;
const launch = async (command, args, env = process.env) => {
  const file = await open(join(work, `process-${children.length}.log`), 'w'); files.push(file);
  const child = spawn(command, args, { env, stdio: ['ignore', file.fd, file.fd], windowsHide: true });
  children.push(child); return child;
};
const until = async (fn, seconds = 30) => {
  const deadline = Date.now() + seconds * 1000;
  while (Date.now() < deadline) { if (await fn()) return; await new Promise((r) => setTimeout(r, 100)); }
  throw new Error('Timed out waiting for live condition');
};
try {
  const serverPort = await port(), gitPort = await port();
  const origin = `http://127.0.0.1:${serverPort}`;
  await mkdir(join(work, 'repos'));
  // Clone existing history; never create a commit, even in the fixture.
  execFileSync('git', ['clone', '--bare', '--quiet', '.', join(work, 'repos/source.git')], { windowsHide: true });
  const branch = execFileSync('git', ['branch', '--show-current'], { encoding: 'utf8' }).trim();
  execFileSync('git', ['--git-dir=' + join(work, 'repos/source.git'), 'update-server-info'], { windowsHide: true });
  await launch('python3', ['-m', 'http.server', String(gitPort), '--bind', '127.0.0.1', '--directory', join(work, 'repos')]);
  const env = { ...process.env, FERRY_DATA_DIR: join(work, 'data'), FERRY_BIND: `127.0.0.1:${serverPort}` };
  delete env.FERRY_DATABASE_URL;
  const server = await launch(resolve('target/debug/ferry-server'), [], env);
  await until(async () => {
    assert.equal(server.exitCode, null, 'server exited early');
    try { return (await fetch(`${origin}/api/status`)).ok; } catch { return false; }
  });
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const api = async (path, body) => {
    const response = await context.request.fetch(`${origin}/api${path}`, {
      method: body === undefined ? 'GET' : 'POST', data: body,
      headers: { 'X-Ferry-Request': '1' },
    });
    assert(response.ok(), `${path}: ${response.status()} ${await response.text()}`);
    return response.json();
  };
  await api('/users', { email: 'ui@example.invalid', name: 'UI acceptance', password: 'temporary-ui-password' });
  await api('/workspaces', { name: 'UI acceptance', slug: 'ops' });
  const group = await api('/workspaces/ops/host-groups', { name: 'UI group' });
  const host = await api('/workspaces/ops/hosts', { name: 'UI runner', connection: 'runner', group_id: group.id });
  const token = await api('/workspaces/ops/runner-tokens', { host_id: host.id });
  await launch(resolve('target/debug/ferry-runner'), ['--server', origin, '--token', token.token, '--state-dir', join(work, 'runner')]);
  await until(async () => (await api(`/workspaces/ops/hosts/${host.id}`)).state === 'online');
  page = await context.newPage();
  const errors = []; page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(`${origin}/ops/build/repositories`);
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByLabel('名称', { exact: false }).fill('UI repository');
  await page.getByLabel('URL', { exact: false }).fill(`http://127.0.0.1:${gitPort}/source.git`);
  await page.getByLabel('默认分支', { exact: false }).fill(branch);
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByRole('cell', { name: 'UI repository', exact: true }).waitFor();
  await page.getByRole('link', { name: '流水线', exact: true }).click();
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByLabel('名称', { exact: false }).fill('UI pipeline');
  await page.getByRole('combobox').nth(0).click(); await page.getByRole('option', { name: 'UI repository', exact: true }).click();
  await page.getByRole('combobox').nth(2).click(); await page.getByRole('option', { name: 'UI runner', exact: true }).click();
  await page.getByRole('combobox').nth(1).click(); await page.getByRole('option', { name: '主机组', exact: true }).click();
  assert.equal(await page.getByRole('combobox').nth(2).innerText(), '请选择');
  await page.getByRole('combobox').nth(2).click(); await page.getByRole('option', { name: 'UI group', exact: true }).click();
  await page.getByRole('combobox').nth(1).click(); await page.getByRole('option', { name: '主机', exact: true }).click();
  assert.equal(await page.getByRole('combobox').nth(2).innerText(), '请选择');
  await page.getByRole('combobox').nth(2).click(); await page.getByRole('option', { name: 'UI runner', exact: true }).click();
  const definition = {
    workdir: join(work, 'build-{{run}}'), steps: [
      { name: '克隆', run: 'git clone -b {{branch}} {{repo_url}} source && echo CLONE_OK; sleep 3', timeout_seconds: 60 },
      { name: '构建', run: 'test -f source/package.json && echo BUILD_OK; sleep 4', timeout_seconds: 60 },
      { name: '完成', run: 'echo FINISH_OK; sleep 3', timeout_seconds: 60 },
    ],
  };
  await page.getByLabel('定义（JSON）', { exact: false }).fill(JSON.stringify({ ...definition, steps: [{ ...definition.steps[0], timeout_seconds: 0 }] }));
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByText(/第 1 步.*超时时间/).waitFor();
  console.log('PASS: repository/pipeline shared forms, dependent target reset, verbatim server validation');
  await page.getByLabel('定义（JSON）', { exact: false }).fill(JSON.stringify(definition, null, 2));
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByRole('cell', { name: 'UI pipeline', exact: true }).waitFor();
  const requests = [];
  page.on('request', (request) => { if (/pipeline-runs\/[^/]+(?:\/logs\?|$)/.test(request.url())) requests.push(request.url()); });
  await page.getByRole('button', { name: '触发', exact: true }).click();
  await page.waitForURL(/\/ops\/build\/runs\/\d+/);
  await page.getByLabel('步骤', { exact: true }).getByText('运行中', { exact: true }).waitFor();
  await page.getByLabel('执行日志', { exact: true }).getByText(/CLONE_OK/).waitFor();
  assert((await page.getByLabel('步骤', { exact: true }).innerText()).includes('运行中'));
  const firstCount = await page.locator('[data-seq]').count();
  await page.getByLabel('执行日志', { exact: true }).getByText(/BUILD_OK/).waitFor();
  const secondCount = await page.locator('[data-seq]').count(); assert(secondCount > firstCount);
  await page.getByText('日志已读取完毕，轮询已停止', { exact: true }).waitFor();
  assert((await page.getByLabel('执行日志', { exact: true }).innerText()).includes('FINISH_OK'));
  assert.equal(await page.getByLabel('步骤', { exact: true }).getByText('成功', { exact: true }).count(), 3);
  if (process.env.UI_SCREENSHOT) await page.screenshot({ path: process.env.UI_SCREENSHOT });
  const finalCount = await page.locator('[data-seq]').count();
  const requestCount = requests.length;
  await page.waitForTimeout(3500); assert.equal(requests.length, requestCount, 'polling continued after completion');
  assert(requests.some((url) => /after=[1-9]/.test(url)), 'no incremental cursor');
  console.log(`PASS: real Git clone -> runner; steps advance to 3 successes; log chunks ${firstCount} -> ${secondCount} -> ${finalCount}; after cursor advances; ${requestCount} requests unchanged for 3.5s after completion`);
  await page.getByRole('link', { name: '执行历史', exact: true }).first().click();
  await page.getByRole('cell', { name: '#1', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '触发', exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  console.log('PASS: execution history, no trigger action in history, no browser exceptions');
} catch (error) {
  if (page) console.error(await page.locator("body").innerText());
  throw error;
} finally {
  await browser?.close();
  for (const child of children.reverse()) {
    if (child.exitCode === null) {
      child.kill('SIGTERM');
      await Promise.race([new Promise((r) => child.once('exit', r)), new Promise((r) => setTimeout(() => { child.kill('SIGKILL'); r(); }, 3000).unref())]);
    }
  }
  for (const file of files) await file.close();
  await rm(work, { recursive: true, force: true });
}
