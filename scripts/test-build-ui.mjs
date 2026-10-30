/** Real browser -> server -> runner acceptance. No project commits or persistent data.
 * Run after npm run build and cargo build -p ferry-server -p ferry-runner.
 * PLAYWRIGHT_MODULE may point to an existing Playwright index.mjs installation.
 */
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, rm, mkdir, open, writeFile } from 'node:fs/promises';
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
let passed = 0;
const pass = (label) => { passed += 1; console.log(`PASS: ${label}`); };
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
  // A self-contained fixture repository, built in the scratch directory.
  //
  // This used to clone the project repo itself. That passed locally and failed
  // on CI: actions/checkout@v4 fetches depth 1, so the bare clone had an
  // incomplete object graph and the dumb-HTTP fetch could not resolve HEAD's
  // commit. A fixture must not depend on how deep the outer checkout happens
  // to be — nor on the project having any history at all. Committing inside a
  // throwaway directory touches nothing outside it.
  const branch = 'ui-fixture';
  const seed = join(work, 'seed');
  await mkdir(seed);
  await writeFile(join(seed, 'README.md'), 'ferry ui fixture\n');
  const git = (...args) =>
    execFileSync('git', ['-C', seed, ...args], {
      windowsHide: true,
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: 'ferry', GIT_AUTHOR_EMAIL: 'ferry@example.invalid',
        GIT_COMMITTER_NAME: 'ferry', GIT_COMMITTER_EMAIL: 'ferry@example.invalid',
      },
    });
  git('init', '--quiet', '--initial-branch', branch);
  git('add', 'README.md');
  git('commit', '--quiet', '-m', 'fixture');
  execFileSync('git', ['clone', '--bare', '--quiet', seed, join(work, 'repos/source.git')], { windowsHide: true });
  execFileSync('git', ['--git-dir=' + join(work, 'repos/source.git'), 'update-server-info'], { windowsHide: true });
  await launch(process.env.PYTHON_BINARY || 'python3', ['-m', 'http.server', String(gitPort), '--bind', '127.0.0.1', '--directory', join(work, 'repos')]);
  const env = { ...process.env, FERRY_DATA_DIR: join(work, 'data'), FERRY_BIND: `127.0.0.1:${serverPort}` };
  delete env.FERRY_DATABASE_URL;
  const server = await launch(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], env);
  await until(async () => {
    assert.equal(server.exitCode, null, 'server exited early');
    try { return (await fetch(`${origin}/api/status`)).ok; } catch { return false; }
  });
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const api = async (path, body) => {
    const response = await context.request.fetch(`${origin}/api${path}`, {
      method: body === undefined ? 'GET' : 'POST', data: body,
      headers: { 'X-Ferry-Request': '1' },
    });
    assert(response.ok(), `${path}: ${response.status()} ${await response.text()}`);
    return response.json();
  };
  execFileSync(process.env.PYTHON_BINARY || 'python3', ['scripts/test-setup.py', `${origin}/api`, join(work, 'data')], { windowsHide: true });
  await api('/users', { email: 'ui@example.invalid', name: 'UI acceptance', password: 'temporary-ui-password' });
  const workspace_ops = (await api('/workspaces', { name: 'UI acceptance', slug: 'ops' })).id;
  await api(`/workspaces/${workspace_ops}/variables`, { name: 'build-ui-secret', scope: 'build', kind: 'secret', value: 'browser-managed-build-secret' });
  const group = await api(`/workspaces/${workspace_ops}/node-groups`, { name: 'UI group' });
  const host = await api(`/workspaces/${workspace_ops}/nodes`, { name: 'UI runner', connection: 'runner', group_id: group.id });
  const token = await api(`/workspaces/${workspace_ops}/runner-tokens`, { host_id: host.id });
  await launch(resolve(process.env.RUNNER_BINARY || 'target/debug/ferry-runner'), ['--server', origin, '--token', token.token, '--state-dir', join(work, 'runner')]);
  await until(async () => (await api(`/workspaces/${workspace_ops}/nodes/${host.id}`)).state === 'online');
  page = await context.newPage();
  const errors = []; page.on('pageerror', (error) => errors.push(error.message));
  const assets = []; context.on('request', (request) => assets.push(request.url()));
  await page.goto(`${origin}/workspaces/${workspace_ops}/code-connections`);
  await page.getByRole('button', { name: '创建连接', exact: true }).waitFor();
  let releasePreferences;
  const preferencesGate = new Promise(resolve => { releasePreferences = resolve; });
  await page.route('**/preferences', async route => { await preferencesGate; await route.continue(); });
  await page.getByRole('button', { name: '创建连接', exact: true }).click();
  await page.getByLabel('名称', { exact: false }).fill('UI repository');
  await page.getByRole('combobox', {name:'代码平台',exact:true}).selectOption('git');
  await page.getByLabel('Git 仓库地址（可选）', { exact: true }).fill(`http://127.0.0.1:${gitPort}/source.git`);
  assert.equal(await page.getByLabel('默认分支', { exact: false }).count(),0);
  const preferencesLoaded = page.waitForResponse(response => response.url().endsWith('/preferences'));
  releasePreferences(); await preferencesLoaded;
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await page.getByLabel('名称', { exact: false }).inputValue(), 'UI repository', 'background preferences refresh discarded the form draft');
  assert.equal(await page.getByLabel('Git 仓库地址（可选）', { exact: true }).inputValue(), `http://127.0.0.1:${gitPort}/source.git`);
  await page.unroute('**/preferences');
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByRole('cell', { name: 'UI repository', exact: true }).waitFor();
  // New workflows use Actions; retain this acceptance fixture for editing and
  // executing pipelines saved by earlier releases, with the original assertions.
  // Seed a concrete repository as an older release would have persisted it;
  // new connection/source-node checkout is covered by repository-auth-ui.
  await api(`/workspaces/${workspace_ops}/repositories`,{name:'UI repository',provider:'git',url:`http://127.0.0.1:${gitPort}/source.git`,default_branch:branch});
  const legacy=await api(`/workspaces/${workspace_ops}/pipelines`,{name:'Legacy seed',target_kind:'host',target_ref:host.id,definition:JSON.stringify({workdir:join(work,'build-{{run}}'),steps:[{name:'Seed',run:'true'}]})});
  await page.goto(`${origin}/workspaces/${workspace_ops}/pipelines/${legacy.id}/edit`);
  await page.getByLabel('名称', { exact: false }).fill('UI pipeline');
  await page.getByRole('button', { name: '流水线设置', exact: true }).click();
  await page.getByRole('combobox', { name: '仓库',exact:true }).selectOption({label:'UI repository'});
  await page.getByRole('combobox', { name: /^目标主机(?!组)/ }).click(); await page.getByRole('option', { name: 'UI runner', exact: true }).click();
  await page.getByRole('combobox', { name: /^目标类型/ }).click(); await page.getByRole('option', { name: '主机组', exact: true }).click();
  assert.equal(await page.getByRole('combobox', { name: /^目标主机组/ }).innerText(), '请选择');
  await page.getByRole('combobox', { name: /^目标主机组/ }).click(); await page.getByRole('option', { name: 'UI group', exact: true }).click();
  await page.getByRole('combobox', { name: /^目标类型/ }).click(); await page.getByRole('option', { name: '主机', exact: true }).click();
  assert.equal(await page.getByRole('combobox', { name: /^目标主机(?!组)/ }).innerText(), '请选择');
  await page.getByRole('combobox', { name: /^目标主机(?!组)/ }).click(); await page.getByRole('option', { name: 'UI runner', exact: true }).click();
  const definition = {
    workdir: join(work, 'build-{{run}}'),
    // Hold each following step until the browser observes the preceding log.
    // Fixed sleeps raced independent detail/log polling on fast or busy hosts.
    env: [{ name: 'UI_BARRIER_DIR', value: work }, { name: 'MANAGED_SECRET', valueFrom: 'variable/build-ui-secret' }],
    steps: [
      { name: '克隆', run: 'git clone -b {{branch}} {{repo_url}} source && echo CLONE_OK', timeout_seconds: 60 },
      { name: '构建', run: 'while [ ! -f "$UI_BARRIER_DIR/allow-build" ]; do sleep 0.1; done; test -f source/README.md && echo BUILD_OK', timeout_seconds: 60 },
      { name: '完成', run: 'while [ ! -f "$UI_BARRIER_DIR/allow-finish" ]; do sleep 0.1; done; echo FINISH_OK', timeout_seconds: 60 },
    ],
  };
  await page.getByLabel('定义（JSON）', { exact: false }).fill(JSON.stringify({ ...definition, steps: [{ ...definition.steps[0], timeout_seconds: 0 }] }));
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByText(/第 1 步.*超时时间/).waitFor();
  pass('repository/pipeline shared forms, dependent target reset, verbatim server validation');
  await page.getByLabel('定义（JSON）', { exact: false }).fill(JSON.stringify(definition, null, 2));
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('cell', { name: 'UI pipeline', exact: true }).waitFor();
  const requests = [];
  page.on('request', (request) => { if (/pipeline-runs\/[^/]+(?:\/logs\?|$)/.test(request.url())) requests.push(request.url()); });
  await page.getByRole('button', { name: '触发', exact: true }).click();
  await page.waitForURL(/\/workspaces\/\d+\/pipeline-runs\/\d+/);
  await page.getByLabel('任务步骤和日志', { exact: true }).getByText('运行中', { exact: true }).waitFor();
  await page.getByLabel('任务步骤和日志', { exact: true }).getByText(/CLONE_OK/).waitFor();
  await page.getByLabel('任务步骤和日志', { exact: true }).getByText('运行中', { exact: true }).waitFor();
  assert((await page.getByLabel('任务步骤和日志', { exact: true }).innerText()).includes('运行中'));
  assert((await page.getByLabel('执行变量', { exact: true }).innerText()).includes('MANAGED_SECRET ← build-ui-secret · 构建模块 · v1 · 密钥'));
  assert(!(await page.locator('body').innerText()).includes('browser-managed-build-secret'));
  const firstCount = await page.locator('[data-seq]').count();
  await writeFile(join(work, 'allow-build'), '');
  await page.getByLabel('任务步骤和日志', { exact: true }).getByText(/BUILD_OK/).waitFor();
  const secondCount = await page.locator('[data-seq]').count(); assert(secondCount > firstCount);
  await writeFile(join(work, 'allow-finish'), '');
  await page.getByText('日志已加载', { exact: true }).waitFor();
  assert((await page.getByLabel('任务步骤和日志', { exact: true }).innerText()).includes('FINISH_OK'));
  assert.equal(await page.getByLabel('任务步骤和日志', { exact: true }).getByText('成功', { exact: true }).count(), 3);
  if (process.env.UI_SCREENSHOT) await page.screenshot({ path: process.env.UI_SCREENSHOT });
  const finalCount = await page.locator('[data-seq]').count();
  const requestCount = requests.length;
  await page.waitForTimeout(3500); assert.equal(requests.length, requestCount, 'polling continued after completion');
  assert(requests.some((url) => /after=[1-9]/.test(url)), 'no incremental cursor');
  pass(`real Git clone -> runner; steps advance to 3 successes; log chunks ${firstCount} -> ${secondCount} -> ${finalCount}; after cursor advances; ${requestCount} requests unchanged for 3.5s after completion`);
  await page.getByRole('link', { name: '执行历史', exact: true }).first().click();
  await page.getByRole('cell', { name: '#1', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '触发', exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  pass('execution history, no trigger action in history, no browser exceptions');
  assert(!assets.some((url) => /\/assets\/xterm-.*\.js/.test(url)), 'xterm loaded before terminal selection');
  await page.goto(`${origin}/workspaces/${workspace_ops}/nodes/${host.id}`);
  const terminalPage = context.waitForEvent('page');
  await page.getByRole('link', { name: '打开终端', exact: true }).click();
  page = await terminalPage;
  page.on('pageerror', error => errors.push(error.message));
  await page.getByText('已连接', { exact: true }).waitFor();
  await page.locator('.xterm-screen').waitFor();
  assert(assets.some((url) => /\/assets\/xterm-.*\.js/.test(url)), 'terminal did not load xterm chunk');
  assert.deepEqual(errors, []);
  pass('xterm excluded from initial pages, lazy terminal loads and connects to real runner');
  await page.getByRole('button', { name: '切换为文件', exact: true }).click();
  const directory = '/tmp';
  // Initial directory loading normalizes the path input when it finishes.
  // Wait for that load before entering a different directory.
  await page.getByLabel('目录路径').waitFor();
  await page.getByRole('button', { name: '刷新', exact: true }).locator('.animate-spin').waitFor({ state: 'hidden' });
  await page.getByLabel('目录路径').fill(directory);
  await Promise.all([
    page.waitForResponse(response => new URL(response.url()).searchParams.get('path') === directory && response.ok()),
    page.getByRole('button', { name: '转到目录', exact: true }).click(),
  ]);
  await until(async () => (await page.getByRole('button', { name: '在当前目录打开终端' }).getAttribute('title')) === `在 ${directory} 打开终端`);
  await page.getByRole('button', { name: '在当前目录打开终端' }).click();
  const directoryPanel = page.locator('section').filter({ has: page.locator('.xterm-screen') });
  await directoryPanel.getByText('已连接', { exact: true }).waitFor();
  await directoryPanel.locator('.xterm-screen').click();
  await page.keyboard.type('pwd');
  await page.keyboard.press('Enter');
  await until(async () => (await directoryPanel.locator('.xterm-screen').innerText()).includes(directory));
  pass('file directory terminal opens a real shell in the selected directory');
  assert(passed >= 5, `expected at least 5 acceptance groups, got ${passed}`);
  console.log(`${passed} passed, 0 failed, 0 skipped`);
} catch (error) {
  if (process.env.GITHUB_ACTIONS) console.error(`::error::${String(error.stack || error).replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A')}`);
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
