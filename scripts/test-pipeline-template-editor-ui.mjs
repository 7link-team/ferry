/** Real browser + HTTP acceptance for the shared pipeline/template workspace. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, open, readFile, rm, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { parseDocument } from 'yaml';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'template-editor-'));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1280, height: 900 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const headers = { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' };
  const request = async (method, path, data) => {
    const response = await context.request.fetch(`${origin}/api${path}`, { method, headers, data });
    assert(response.ok(), `${method} ${path}: ${await response.text()}`);
    return response.status() === 204 ? undefined : response.json();
  };
  await request('POST', '/setup', { email: 'admin@example.invalid', name: 'Admin', password: 'temporary-admin-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const ws = (await request('POST', '/workspaces', { name: 'Template test', slug: 'template-test' })).id;
  const root = `/workspaces/${ws}`;
  const p = await context.newPage();
  const errors = [];
  p.on('pageerror', error => errors.push(error.message));
  const canvas = () => p.locator('[aria-label="流水线画布"]');
  const name = 'Visual template';
  await p.goto(`${origin}/zh-CN${root}/pipeline-templates/new`);
  await canvas().waitFor();
  await p.getByRole('textbox', { name: '模板名称', exact: true }).fill(name);
  await p.getByRole('button', { name: '配置', exact: true }).click();
  const parameters = await p.getByLabel('参数定义（JSON 数组）', { exact: false }).inputValue();
  await p.getByRole('button', { name: '配置', exact: true }).click();
  await p.locator('.react-flow__node').getByRole('button', { name: /步骤1/ }).click();
  await p.getByRole('textbox', { name: '执行命令', exact: true }).fill('echo {{ .Values.command }} ${{ github.ref }}');
  assert.equal(await p.getByRole('button', { name: '保存并运行', exact: true }).count(), 0, 'a template is not an executable pipeline');
  await p.getByRole('button', { name: '创建', exact: true }).click();
  await p.getByRole('cell', { name, exact: true }).waitFor();
  let row = (await request('GET', `${root}/pipeline-templates`)).items.find(row => row.name === name);
  assert(row, 'template created without a bound executor');
  assert.equal(row.parameters, parameters);
  assert.match(JSON.parse(row.content).workflow, /\$\{\{ github.ref \}\}/);
  console.log('PASS visual creation, template parameters and runtime expressions, no mandatory host');

  const host = await request('POST', `${root}/nodes`, { name: 'Template host', connection: 'runner' });
  const envelope = JSON.parse(row.content);
  const jobId = Object.keys(parseDocument(envelope.workflow).toJS().jobs)[0];
  envelope.executor_bindings = [{ job_id: jobId, target_kind: 'host', target_ref: host.id, image: 'host' }];
  envelope.graph_layout = { [jobId]: { x: 80, y: 120 } };
  row = await request('PATCH', `${root}/pipeline-templates/${row.id}`, { ...row, content: JSON.stringify(envelope) }).then(() => request('GET', `${root}/pipeline-templates/${row.id}`));
  await p.goto(`${origin}/zh-CN${root}/pipeline-templates/${row.id}/edit`);
  await canvas().waitFor();
  await p.locator('.react-flow__node').getByRole('button', { name: /步骤1/ }).click();
  assert.equal(await p.getByRole('textbox', { name: '执行命令', exact: true }).inputValue(), 'echo {{ .Values.command }} ${{ github.ref }}');
  await p.getByRole('textbox', { name: '执行命令', exact: true }).fill('printf {{ .Values.command }} ${{ github.ref }}');
  await p.getByRole('tab', { name: 'YAML', exact: true }).click();
  await p.locator('.monaco-editor .view-lines').click();
  await p.keyboard.press('ControlOrMeta+A');
  await p.keyboard.press('ControlOrMeta+C');
  assert.match(await p.evaluate(() => navigator.clipboard.readText()), /printf \{\{ \.Values.command \}\}/);
  await p.getByRole('tab', { name: '可视化', exact: true }).click();
  await p.getByRole('button', { name: '保存', exact: true }).click();
  await p.getByRole('cell', { name, exact: true }).waitFor();
  row = await request('GET', `${root}/pipeline-templates/${row.id}`);
  const saved = JSON.parse(row.content);
  assert.equal(row.parameters, parameters);
  assert.deepEqual(saved.executor_bindings, envelope.executor_bindings);
  assert.deepEqual(saved.graph_layout, envelope.graph_layout);
  assert.deepEqual(saved.checkout_bindings, envelope.checkout_bindings);
  assert.match(saved.workflow, /printf/);
  console.log('PASS edit, YAML/visual round trip, HTTP persistence and binding/layout preservation');

  await p.goto(`${origin}/zh-CN${root}/pipeline-templates/${row.id}`);
  await canvas().waitFor();
  assert.equal(await p.getByRole('textbox', { name: '模板名称', exact: true }).getAttribute('readonly'), '');
  assert.equal(await p.getByRole('button', { name: '保存', exact: true }).count(), 0);
  await p.locator('.react-flow__node').getByRole('button', { name: /步骤1/ }).click();
  assert.equal(await p.getByRole('textbox', { name: '执行命令', exact: true }).getAttribute('readonly'), '');
  assert.equal(await p.getByRole('link', { name: '编辑', exact: true }).count(), 1);
  await mkdir('data/maintenance/template-editor', { recursive: true });
  await p.screenshot({ path: 'data/maintenance/template-editor/detail.png' });
  await p.setViewportSize({ width: 390, height: 844 });
  assert(await p.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'mobile has no document overflow');
  await p.screenshot({ path: 'data/maintenance/template-editor/mobile.png' });
  console.log('PASS readonly visual detail, edit navigation, desktop/mobile layout');

  const rendered = await request('POST', `${root}/pipeline-templates/${row.id}/renderings`, { version: row.version, values: { command: 'hello' } });
  const generated = JSON.parse(rendered.definition);
  assert.match(generated.workflow, /printf hello \$\{\{ github.ref \}\}/);
  assert.deepEqual(generated.executor_bindings, envelope.executor_bindings);
  assert(!generated.workflow.includes('.Values.command'));
  console.log('PASS server template rendering preserves runtime expressions and executor bindings');
  assert.deepEqual(errors, []);
  console.log('PASS no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
