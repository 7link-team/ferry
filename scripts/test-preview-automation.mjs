#!/usr/bin/env node
// Live preview-automation check (docs/preview-automation-design.md section 9):
// an isolated server, a real Runner, real local services and a real browser
// drive ferry/preview@v1 (approve and reject), the host.preview workflow node
// with human review, and the host detail's preview tab.
// Build first: `cargo build -p ferry-server -p ferry-runner`, the Actions
// worker for this host (scripts/build-actions-workers.sh) and `npm run build`.
// Binaries: FERRY_BINARY, FERRY_RUNNER_BINARY, ACTIONS_WORKER_DIR (defaults:
// target/debug and the server's sibling actions-workers directory).
// Evidence lands in data/maintenance/preview-automation-<stamp>/.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { openSync } from 'node:fs';
import { createServer, connect } from 'node:net';
import { join, resolve } from 'node:path';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const exe = process.platform === 'win32' ? '.exe' : '';
const serverBinary = resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${exe}`);
const runnerBinary = resolve(process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${exe}`);
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const work = resolve(`data/maintenance/preview-automation-${stamp}`);
await mkdir(work, { recursive: true });

const processes = [];
const result = { started: new Date().toISOString(), checks: {}, screenshots: [], result: 'FAIL' };
let current = 'setup';
const pass = (name, detail = true) => {
  result.checks[name] = detail;
  console.log(`PASS ${name}`);
};
const step = (name) => {
  current = name;
  console.log(`---- ${name}`);
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

/** True when something accepts connections on 127.0.0.1:port. */
function listening(port) {
  return new Promise((done) => {
    const socket = connect({ host: '127.0.0.1', port });
    socket.once('connect', () => { socket.destroy(); done(true); });
    socket.once('error', () => done(false));
    socket.setTimeout(2000, () => { socket.destroy(); done(false); });
  });
}

function start(name, command, args, env = {}) {
  const log = openSync(join(work, `${name}.log`), 'a');
  const child = spawn(command, args, {
    env: { ...process.env, ...env },
    stdio: ['ignore', log, log],
    windowsHide: true,
  });
  processes.push(child);
  return child;
}

function stop(child) {
  if (child.exitCode === null && child.signalCode === null) child.kill();
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
    await new Promise((done) => setTimeout(done, 400));
  }
  throw new Error(`${label} timed out (${last ?? 'no value'})`);
}

/** Processes started for this run and their descendants, never unrelated users of the binaries. */
function leftovers() {
  if (process.platform !== 'win32') return Promise.resolve([]);
  const roots = processes.map(child=>child.pid).filter(pid=>Number.isInteger(pid));
  const needle = work.replaceAll("'", "''");
  const script = `$ids=@(${roots.join(',')});$work='${needle}';$all=@(Get-CimInstance Win32_Process);` +
    'do { $changed=$false; foreach($p in $all) { if(($ids -contains $p.ParentProcessId) -and ($ids -notcontains $p.ProcessId)) { $ids+= $p.ProcessId; $changed=$true } } } while($changed);' +
    '$all | Where-Object { (($_.CommandLine -and $_.CommandLine.Contains($work)) -or ($ids -contains $_.ProcessId)) -and $_.ProcessId -ne $PID } | ' +
    'ForEach-Object { "$($_.ProcessId) $($_.Name)" }';
  return new Promise((done) => {
    execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { windowsHide: true },
      (_, stdout) => done(String(stdout).split(/\r?\n/).map((line) => line.trim()).filter(Boolean)
        .filter((line) => !/powershell/i.test(line))));
  });
}

const serverPort = await freePort();
const ports = {
  server: serverPort,
  approve: await freePort(),
  reject: await freePort(),
  workflow: await freePort(),
  manual: await freePort(),
};
result.ports = ports;
const base = `http://127.0.0.1:${serverPort}`;
let cookie = '';

async function call(path, body, method = body === undefined ? 'GET' : 'POST') {
  const response = await fetch(base + path, {
    method,
    headers: { 'content-type': 'application/json', 'x-ferry-request': '1', 'accept-language': 'zh-CN', cookie },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const setCookie = response.headers.get('set-cookie');
  if (setCookie?.startsWith('ferry_session=')) cookie = setCookie.split(';')[0];
  const text = await response.text();
  return { status: response.status, text, json: () => (text ? JSON.parse(text) : null) };
}

async function api(path, body, method) {
  const response = await call(path, body, method);
  if (response.status < 200 || response.status >= 300) {
    throw new Error(`${method ?? (body === undefined ? 'GET' : 'POST')} ${path} -> ${response.status} ${response.text}`);
  }
  return response.json();
}

// The previewed service: one known page on every path, so the check sees the
// page whether or not the preview strips its prefix.
const serviceScript = join(work, 'service.cjs');
await writeFile(serviceScript, `
const http = require('node:http');
const port = Number(process.argv[2]);
const label = process.argv[3] || 'service';
http.createServer((request, response) => {
  response.setHeader('content-type', 'text/html; charset=utf-8');
  response.end('<!doctype html><html><body><h1 id="marker">PREVIEW_OK ' + label + ' ' + port + '</h1></body></html>');
}).listen(port, '127.0.0.1');
`);
const marker = (port, label) => `PREVIEW_OK ${label} ${port}`;

let browser;
let context;
let page;
let ws;
let root;
let host;
const overflow = {};

/** Viewport screenshots at the three widths; each must not scroll sideways. */
async function shots(page, name, ready) {
  for (const [width, height] of [[1440, 900], [900, 1100], [390, 844]]) {
    await page.setViewportSize({ width, height });
    await ready();
    await page.waitForTimeout(300);
    const extra = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    overflow[`${name}-${width}`] = extra;
    const file = `${name}-${width}.png`;
    await page.screenshot({ path: join(work, file), fullPage: true });
    result.screenshots.push(file);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
}

const finished = (state) => ['succeeded', 'failed', 'cancelled'].includes(state);

async function runLogs(run) {
  const rows = await api(`${root}/pipeline-runs/${run}/logs?after=0&page_size=500`);
  return rows.items.map((row) => row.chunk).join('');
}

/** A one-job pipeline: start the service, preview it with hold, then echo the link. */
async function previewPipeline(name, port) {
  const definition = {
    name,
    on: 'workflow_dispatch',
    jobs: {
      web: {
        'runs-on': 'self-hosted',
        'timeout-minutes': 15,
        steps: [
          {
            name: 'Start service',
            shell: 'powershell',
            env: { SERVICE_NODE: process.execPath, SERVICE_SCRIPT: serviceScript, SERVICE_PORT: String(port) },
            run: 'Start-Process -WindowStyle Hidden -FilePath $env:SERVICE_NODE -ArgumentList @($env:SERVICE_SCRIPT, $env:SERVICE_PORT, "pipeline")',
          },
          {
            id: 'preview',
            name: 'Preview web',
            uses: 'ferry/preview@v1',
            with: { port, name, hold: 'approval', 'timeout-minutes': 5, 'ready-timeout-seconds': 60 },
          },
          {
            name: 'After review',
            shell: 'powershell',
            run: 'Write-Output "AFTER_REVIEW url=${{ steps.preview.outputs.url }}"',
          },
        ],
      },
    },
  };
  const pipeline = await api(`${root}/pipelines`, {
    name,
    definition_format: 'github-actions',
    definition: JSON.stringify(definition),
    executor_bindings: [{ job_id: 'web', target_kind: 'host', target_ref: host, image: 'host' }],
  });
  return (await api(`${root}/pipelines/${pipeline.id}/runs`, {})).id;
}

async function waitingPreview(run) {
  return until('pipeline preview waiting for review', async () => {
    const detail = await api(`${root}/pipeline-runs/${run}`);
    if (finished(detail.state)) throw new Error(`run ended ${detail.state}: ${await runLogs(run)}`);
    const preview = detail.previews?.find((row) => row.review_state === 'waiting');
    return preview && { detail, preview };
  }, 240);
}

/** The page a preview link serves, once the local service answers through it. */
async function serves(url) {
  let last = '';
  return until(`preview ${url} serves the page`, async () => {
    const response = await fetch(base + url);
    last = `${response.status} ${await response.text()}`;
    if (response.status !== 200) throw new Error(last);
    return last.slice(4);
  }, 90);
}

async function hole(id) {
  return api(`${root}/holes/${id}`);
}

try {
  step('setup');
  const data = join(work, 'server-data');
  const serverEnv = {
    FERRY_DATA_DIR: data,
    FERRY_BIND: `127.0.0.1:${serverPort}`,
    FERRY_DATABASE_URL: '',
    FERRY_NATS_URL: '',
  };
  if (process.env.ACTIONS_WORKER_DIR) serverEnv.ACTIONS_WORKER_DIR = resolve(process.env.ACTIONS_WORKER_DIR);
  const server = start('server', serverBinary, [], serverEnv);
  await until('server ready', async () => {
    assert.equal(server.exitCode, null, 'server exited during startup');
    return (await api('/api/status')).ok;
  });
  const setupToken = (await readFile(join(data, 'setup-token'), 'utf8')).trim();
  await api('/api/setup', {
    email: 'preview-auto@example.invalid',
    name: 'Preview automation',
    password: 'isolated-preview-auto-password',
    token: setupToken,
  });
  if (!cookie) {
    await api('/api/sessions', { email: 'preview-auto@example.invalid', password: 'isolated-preview-auto-password' });
  }
  assert.ok(cookie, 'console session cookie');
  ws = (await api('/api/workspaces', { name: 'Preview automation', slug: 'preview-auto' })).id;
  root = `/api/workspaces/${ws}`;
  host = (await api(`${root}/nodes`, { name: 'Preview Runner', connection: 'runner' })).id;
  const token = (await api(`${root}/runner-tokens`, { host_id: host })).token;
  const runner = start('runner', runnerBinary, [
    '--server', base, '--token', token, '--state-dir', join(work, 'runner'),
  ]);
  await until('runner online', async () => {
    assert.equal(runner.exitCode, null, 'runner exited');
    return (await api(`${root}/nodes/${host}`)).state === 'online';
  });
  pass('real_runner_online');

  browser = await chromium.launch({
    headless: true,
    ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}),
  });
  context = await browser.newContext({ viewport: { width: 1440, height: 900 }, locale: 'zh-CN' });
  const [cookieName, cookieValue] = cookie.split('=');
  await context.addCookies([{ name: cookieName, value: cookieValue, domain: '127.0.0.1', path: '/', httpOnly: true, sameSite: 'Lax' }]);
  page = await context.newPage();
  const pageErrors = [];
  page.on('pageerror', (error) => pageErrors.push(String(error)));

  // 1. Pipeline: hold for approval, approve on the run page.
  step('pipeline_approve');
  const approveRun = await previewPipeline('Preview approve', ports.approve);
  result.approve_run = approveRun;
  const { preview: approvePreview } = await waitingPreview(approveRun);
  result.approve_preview = approvePreview;
  assert.equal(approvePreview.job, 'web');
  assert.equal(approvePreview.state, 'active');
  assert.match(approvePreview.url, /^\/holes\/[^/]+\/$/);
  pass('pipeline_preview_waiting_in_run_detail', { id: approvePreview.id, url: approvePreview.url });
  // The row exists from the request on; the step gets the link once the port answers.
  const servedText = await serves(approvePreview.url);
  assert.ok(servedText.includes(marker(ports.approve, 'pipeline')), servedText);
  pass('pipeline_preview_serves_page');
  const runUrl = `${base}/zh-CN/workspaces/${ws}/pipeline-runs/${approveRun}`;
  await page.goto(runUrl);
  const previews = page.getByRole('region', { name: '预览', exact: true });
  const approveButton = previews.getByRole('button', { name: '通过', exact: true });
  await approveButton.waitFor({ timeout: 30000 });
  assert.equal(await previews.getByRole('link', { name: '打开预览' }).count(), 1);
  await shots(page, 'pipeline-run-previews', () => approveButton.waitFor({ timeout: 30000 }));
  pass('pipeline_run_page_shows_previews_and_approve');
  await approveButton.click();
  const approved = await until('approved run finishes', async () => {
    const detail = await api(`${root}/pipeline-runs/${approveRun}`);
    return finished(detail.state) && detail;
  }, 180);
  const approveLogs = await runLogs(approveRun);
  await writeFile(join(work, 'pipeline-approve.log'), approveLogs);
  assert.equal(approved.state, 'succeeded', approveLogs);
  assert.ok(approveLogs.includes(`AFTER_REVIEW url=${approvePreview.url}`) ||
    new RegExp(`AFTER_REVIEW url=\\S*${approvePreview.url.replaceAll('/', '\\/')}`).test(approveLogs),
  `later step output missing: ${approveLogs}`);
  pass('pipeline_approve_succeeds_and_later_step_ran');
  const approvedHole = await until('approved preview revoked', async () => {
    const row = await hole(approvePreview.id);
    return row.state === 'revoked' && row;
  }, 30);
  assert.equal(approvedHole.review_state, 'approved');
  assert.equal((await fetch(base + approvePreview.url)).status, 404);
  pass('pipeline_approve_preview_revoked_and_404', { review_state: approvedHole.review_state });
  await page.goto(runUrl);
  await previews.getByText('已撤销').first().waitFor({ timeout: 30000 });
  await page.screenshot({ path: join(work, 'pipeline-run-approved-1440.png'), fullPage: true });
  result.screenshots.push('pipeline-run-approved-1440.png');

  // 2. Pipeline: reject on the run page.
  step('pipeline_reject');
  const rejectRun = await previewPipeline('Preview reject', ports.reject);
  result.reject_run = rejectRun;
  const { preview: rejectPreview } = await waitingPreview(rejectRun);
  assert.ok((await serves(rejectPreview.url)).includes(marker(ports.reject, 'pipeline')));
  await page.goto(`${base}/zh-CN/workspaces/${ws}/pipeline-runs/${rejectRun}`);
  const rejectButton = previews.getByRole('button', { name: '驳回', exact: true });
  await rejectButton.waitFor({ timeout: 30000 });
  await rejectButton.click();
  const rejected = await until('rejected run finishes', async () => {
    const detail = await api(`${root}/pipeline-runs/${rejectRun}`);
    return finished(detail.state) && detail;
  }, 180);
  const rejectLogs = await runLogs(rejectRun);
  await writeFile(join(work, 'pipeline-reject.log'), rejectLogs);
  assert.equal(rejected.state, 'failed', rejectLogs);
  assert.equal(rejected.jobs.find((job) => job.key === 'web')?.state, 'failed');
  assert.ok(!rejectLogs.includes('AFTER_REVIEW'), 'the step after a rejected review must not run');
  const rejectedHole = await until('rejected preview revoked', async () => {
    const row = await hole(rejectPreview.id);
    return row.state === 'revoked' && row;
  }, 30);
  assert.equal(rejectedHole.review_state, 'rejected');
  assert.equal((await fetch(base + rejectPreview.url)).status, 404);
  pass('pipeline_reject_fails_job_and_revokes', { run: rejected.state, review_state: rejectedHole.review_state });

  // 3. Workflow: host.preview starts the service and waits for a person.
  step('workflow_human_review');
  const node = (id, type, params, index) => ({ id, name: id, type, type_version: 1, params, position: [index * 320, 160] });
  const graph = {
    engine: 'ferry-graph',
    schema: 1,
    nodes: [
      node('start', 'core.trigger.manual', { input_schema: { type: 'object' } }, 0),
      node('preview', 'host.preview', {
        host_id: host,
        command: '"$FERRY_INPUT_NODE" "$FERRY_INPUT_SCRIPT" "$FERRY_INPUT_PORT" workflow',
        input_env: { NODE: process.execPath, SCRIPT: serviceScript, PORT: String(ports.workflow) },
        port: ports.workflow,
        name: 'Workflow preview',
        strip_prefix: true,
        ready_timeout_seconds: 60,
        review: 'human',
        review_timeout_minutes: 5,
      }, 1),
      node('done', 'core.no_op', {}, 2),
    ],
    connections: [
      { from: { node: 'start', port: 'main', index: 0 }, to: { node: 'preview', port: 'main', index: 0 } },
      { from: { node: 'preview', port: 'approved', index: 0 }, to: { node: 'done', port: 'main', index: 0 } },
    ],
  };
  const workflow = await api(`${root}/workflows`, { name: 'Preview workflow', definition: JSON.stringify(graph) });
  const execution = (await api(`${root}/workflows/${workflow.id}/executions`, {
    mode: 'manual', revision: 'draft', version: workflow.version, entry_node_id: 'start', inputs: {},
  })).id;
  result.workflow = { id: workflow.id, execution };
  const workflowPreview = await until('workflow preview waiting', async () => {
    const state = await api(`${root}/workflow-executions/${execution}`);
    if (finished(state.state)) throw new Error(`execution ended ${state.state}: ${JSON.stringify(state)}`);
    const rows = await api(`${root}/holes?source_kind=workflow&source_id=${execution}`);
    const row = rows.items.find((item) => item.review_state === 'waiting' && item.probe_state === 'reachable');
    const attempts = await api(`${root}/workflow-executions/${execution}/node-runs`);
    const parked = attempts.items.find((item) => item.node_id === 'preview')?.state === 'waiting';
    return row && parked && row;
  }, 180);
  assert.equal(workflowPreview.source_ref, 'preview');
  assert.ok(await listening(ports.workflow), 'service port open while waiting');
  const workflowServed = await fetch(base + workflowPreview.url);
  const workflowText = await workflowServed.text();
  assert.equal(workflowServed.status, 200, workflowText);
  assert.ok(workflowText.includes(marker(ports.workflow, 'workflow')), workflowText);
  pass('workflow_node_waits_and_preview_serves_page', { hole: workflowPreview.id, url: workflowPreview.url });
  await page.goto(`${base}/zh-CN/workspaces/${ws}/workflows/${workflow.id}?execution=${execution}`);
  const nodeSelect = page.getByRole('combobox', { name: '节点执行详情' });
  await nodeSelect.waitFor({ timeout: 30000 });
  const previewOption = await nodeSelect.locator('option').filter({ hasText: 'preview' }).getAttribute('value');
  await nodeSelect.selectOption(previewOption);
  const panel = page.getByRole('region', { name: '预览', exact: true });
  const workflowApprove = panel.getByRole('button', { name: '通过', exact: true });
  await workflowApprove.waitFor({ timeout: 30000 });
  assert.equal(await page.getByText('node_wait', { exact: true }).count(), 0, 'Waiting for review must not display an internal control code as an error');
  assert.equal(await panel.getByRole('link', { name: '打开预览' }).count(), 1);
  await shots(page, 'workflow-execution-preview', async () => {
    await nodeSelect.waitFor({ timeout: 30000 });
    if ((await nodeSelect.inputValue()) !== previewOption) await nodeSelect.selectOption(previewOption);
    await workflowApprove.waitFor({ timeout: 30000 });
  });
  pass('workflow_execution_view_shows_link_and_approve');
  await workflowApprove.click();
  const workflowDone = await until('workflow execution finishes', async () => {
    const state = await api(`${root}/workflow-executions/${execution}`);
    return finished(state.state) && state;
  }, 120);
  await writeFile(join(work, 'workflow-execution.json'), JSON.stringify(workflowDone, null, 2));
  assert.equal(workflowDone.state, 'succeeded', JSON.stringify(workflowDone));
  const attempts = (await api(`${root}/workflow-executions/${execution}/node-runs`)).items;
  await writeFile(join(work, 'workflow-node-runs.json'), JSON.stringify(attempts, null, 2));
  const doneAttempt = attempts.find((item) => item.node_id === 'done');
  assert.equal(doneAttempt?.state, 'succeeded', 'the approved port ran the next node');
  const previewAttempt = attempts.filter((item) => item.node_id === 'preview').at(-1);
  const previewOutput = await api(`${root}/workflow-executions/${execution}/node-runs/${previewAttempt.id}/output`);
  const approvedPort = previewOutput.ports.find((port) => port.name === 'approved');
  assert.equal(approvedPort?.skipped, false, JSON.stringify(previewOutput));
  assert.equal(approvedPort.items[0].json.review_state, 'approved');
  pass('workflow_approved_path_succeeds', { output: approvedPort.items[0].json });
  const workflowHole = await until('workflow preview revoked', async () => {
    const row = await hole(workflowPreview.id);
    return row.state === 'revoked' && row;
  }, 30);
  assert.equal((await fetch(base + workflowPreview.url)).status, 404);
  const closedAfter = Date.now();
  await until('workflow service port closed', async () => !(await listening(ports.workflow)), 60);
  pass('workflow_preview_revoked_and_service_stopped', {
    review_state: workflowHole.review_state, port_closed_ms: Date.now() - closedAfter,
  });

  // 4. Manual preview from the host detail's tab.
  step('host_tab');
  const tabUrl = `${base}/zh-CN/workspaces/${ws}/nodes/${host}?tab=holes`;
  await page.goto(tabUrl);
  const tabPanel = page.getByRole('tabpanel', { name: '预览穿透' });
  await tabPanel.getByText(`流水线运行 · web`).first().waitFor({ timeout: 30000 });
  await tabPanel.getByText(`工作流执行 · preview`).first().waitFor({ timeout: 30000 });
  assert.equal(await tabPanel.getByText('流水线运行 · web').count(), 2, 'both pipeline previews listed');
  // Desktop source links must remain visible beside the sticky actions.
  result.host_tab_source_cell = await tabPanel.getByText('工作流执行 · preview').first().evaluate((cell) => {
    const box = cell.getBoundingClientRect();
    let clip = { left: 0, right: window.innerWidth };
    for (let node = cell.parentElement; node; node = node.parentElement) {
      if (node.scrollWidth > node.clientWidth && getComputedStyle(node).overflowX !== 'visible') {
        const rect = node.getBoundingClientRect();
        clip = { left: Math.max(clip.left, rect.left), right: Math.min(clip.right, rect.right) };
      }
    }
    const atPoint = document.elementFromPoint(box.left + 2, box.top + box.height / 2);
    return { left: Math.round(box.left), right: Math.round(box.right), clip_right: Math.round(clip.right),
      within_clip: box.left >= clip.left && box.right <= clip.right, topmost: !!atPoint && cell.contains(atPoint) };
  });
  pass('host_tab_lists_sources');
  assert.ok(result.host_tab_source_cell.within_clip && result.host_tab_source_cell.topmost, 'Desktop source link must be visible and not hidden by sticky actions');
  await shots(page, 'host-holes-tab', async () => {
    await tabPanel.getByText('工作流执行 · preview').first().waitFor({ timeout: 30000 });
    const table=tabPanel.locator('table').first();
    assert(await table.evaluate(element=>element.parentElement.clientHeight>=120),'Preview table viewport must leave room to see and operate rows');
    assert(await table.evaluate(element => {
      const viewport = element.parentElement;
      if (viewport.scrollWidth <= viewport.clientWidth) return true;
      viewport.scrollLeft = viewport.scrollWidth;
      const reachable = viewport.scrollLeft > 0;
      viewport.scrollLeft = 0;
      return reachable;
    }), 'Wide preview table must remain scrollable inside its own viewport');
    await tabPanel.evaluate(element=>{element.scrollTop=element.scrollHeight;});
  });
  await tabPanel.getByRole('link', { name: '新建预览穿透' }).click();
  await page.waitForURL(/\/holes\/new\?/, { timeout: 30000 });
  const formUrl = new URL(page.url());
  assert.equal(formUrl.searchParams.get('host_id'), host);
  const hostField = page.getByLabel('主机', { exact: true });
  await page.getByText('Preview Runner').first().waitFor({ timeout: 30000 });
  // The menu group is also labelled 主机; only form controls count.
  const locked = await hostField.evaluateAll((elements) => elements
    .filter((element) => ['BUTTON', 'INPUT', 'SELECT', 'TEXTAREA'].includes(element.tagName) || element.getAttribute('role') === 'combobox')
    .map((element) => ({
    tag: element.tagName,
    disabled: element.disabled === true || element.getAttribute('aria-disabled') === 'true' || element.hasAttribute('data-disabled'),
    readonly: element.readOnly === true || element.getAttribute('aria-readonly') === 'true',
    text: element.textContent || element.value || '',
  })));
  result.checks.host_field = locked;
  assert.ok(locked.length > 0 && locked.every((field) => field.disabled || field.readonly), `host field editable: ${JSON.stringify(locked)}`);
  assert.ok(locked.some((field) => field.text.includes('Preview Runner')), `host not prefilled: ${JSON.stringify(locked)}`);
  await page.screenshot({ path: join(work, 'host-hole-form-1440.png'), fullPage: true });
  result.screenshots.push('host-hole-form-1440.png');
  await page.getByLabel('名称', { exact: true }).fill('Manual tab preview');
  await page.getByLabel('本地端口', { exact: true }).fill(String(ports.manual));
  await page.getByRole('button', { name: /^(创建|保存|提交)$/ }).first().click();
  const manual = await until('manual preview created', async () => {
    const rows = await api(`${root}/holes?host_id=${host}&q=Manual%20tab%20preview`);
    return rows.items.find((row) => row.name === 'Manual tab preview');
  }, 30);
  assert.equal(manual.host_id, host);
  assert.equal(manual.source_kind, 'manual');
  await page.goto(tabUrl);
  await tabPanel.getByText('Manual tab preview').first().waitFor({ timeout: 30000 });
  await tabPanel.getByText('手动').first().waitFor({ timeout: 30000 });
  pass('host_tab_manual_create_prefilled_locked_and_listed', { id: manual.id, host_field: locked });
  await page.screenshot({ path: join(work, 'host-holes-tab-after-create-1440.png'), fullPage: true });
  result.screenshots.push('host-holes-tab-after-create-1440.png');

  // 5. The left menu has no preview-tunnel entry.
  step('nav');
  await page.goto(`${base}/zh-CN/workspaces/${ws}/nodes`);
  const menu = page.getByRole('navigation').first();
  await menu.waitFor({ timeout: 30000 });
  await page.waitForLoadState('networkidle');
  const menuText = await page.getByRole('navigation').allTextContents();
  const menuLinks = await page.getByRole('navigation').locator('a').evaluateAll((links) => links.map((link) => link.getAttribute('href') || ''));
  assert.ok(menuText.join(' ').includes('主机'), `menu not rendered: ${menuText}`);
  assert.ok(!menuText.join(' ').includes('预览穿透'), `menu still shows preview tunnels: ${menuText}`);
  assert.ok(!menuLinks.some((href) => /\/holes(\/|\?|$)/.test(href)), `menu links to holes: ${menuLinks}`);
  pass('nav_has_no_preview_tunnels_entry');
  await page.screenshot({ path: join(work, 'nav-hosts-1440.png'), fullPage: true });
  result.screenshots.push('nav-hosts-1440.png');

  // The rejected outlet and cancellation must release the same real service.
  step('workflow_reject_and_cancel');
  const branchGraph = structuredClone(graph);
  branchGraph.nodes.push(node('rejected_done', 'core.no_op', {}, 3));
  branchGraph.connections.push({from:{node:'preview',port:'rejected',index:0},to:{node:'rejected_done',port:'main',index:0}});
  const branchWorkflow = await api(`${root}/workflows`, {name:'Preview reject and cancel',definition:JSON.stringify(branchGraph)});
  for (const decision of ['rejected','cancelled']) {
    const run = (await api(`${root}/workflows/${branchWorkflow.id}/executions`, {mode:'manual',revision:'draft',version:branchWorkflow.version,entry_node_id:'start',inputs:{}})).id;
    const opened = await until(`${decision} preview waiting`, async()=>{
      const rows = await api(`${root}/holes?source_kind=workflow&source_id=${run}`);
      return rows.items.find(row=>row.review_state==='waiting' && row.probe_state==='reachable');
    },180);
    assert.ok(await listening(ports.workflow));
    if (decision==='rejected') await api(`${root}/holes/${opened.id}/reviews`,{decision});
    else await api(`${root}/workflow-executions/${run}/cancellations`,{});
    const ended = await until(`${decision} execution ends`, async()=>{const row=await api(`${root}/workflow-executions/${run}`);return finished(row.state)&&row;},120);
    assert.equal(ended.state,decision==='rejected'?'succeeded':'cancelled');
    if (decision==='rejected') {
      const rows=(await api(`${root}/workflow-executions/${run}/node-runs`)).items;
      assert.equal(rows.find(row=>row.node_id==='rejected_done')?.state,'succeeded');
      assert.notEqual(rows.find(row=>row.node_id==='done')?.state,'succeeded');
    }
    await until(`${decision} preview revoked`,async()=> (await hole(opened.id)).state==='revoked',30);
    assert.equal((await fetch(base+opened.url)).status,404);
    await until(`${decision} service port closed`,async()=>!(await listening(ports.workflow)),60);
    pass(`workflow_${decision}_routes_and_releases_service`);
  }

  // 6. No page scrolls sideways at any width.
  step('overflow');
  result.checks.page_overflow_px = overflow;
  assert.ok(Object.values(overflow).every((px) => px <= 0), `page scrolls sideways: ${JSON.stringify(overflow)}`);
  pass('three_pages_three_widths_no_page_overflow', overflow);
  result.page_errors = pageErrors;
  assert.deepEqual(pageErrors, [], 'Preview pages must not raise browser script errors');

  result.result = 'PASS';
} catch (error) {
  result.failed_check = current;
  result.error = String(error.stack || error);
  console.error(`FAIL ${current}:`, error);
  process.exitCode = 1;
  // Evidence for diagnosis while the server is still up.
  await page?.screenshot({ path: join(work, 'failure.png'), fullPage: true }).catch(() => {});
  await page?.content().then((html) => writeFile(join(work, 'failure.html'), html)).catch(() => {});
  for (const key of ['approve_run', 'reject_run']) {
    if (!result[key]) continue;
    await runLogs(result[key]).then((text) => writeFile(join(work, `${key}-failure.log`), text)).catch(() => {});
    await api(`${root}/pipeline-runs/${result[key]}`)
      .then((detail) => writeFile(join(work, `${key}-failure.json`), JSON.stringify(detail, null, 2))).catch(() => {});
  }
} finally {
  // 7. Cleanup: every process this run started is gone and its ports closed.
  await browser?.close().catch(() => {});
  for (const child of processes) stop(child);
  await Promise.all(processes.map((child) => (child.exitCode !== null || child.signalCode !== null)
    ? null : new Promise((done) => { child.once('exit', done); setTimeout(done, 10000); })));
  let left = [];
  for (let attempt = 0; attempt < 20; attempt += 1) {
    left = await leftovers();
    if (!left.length) break;
    await new Promise((done) => setTimeout(done, 500));
  }
  if (left.length) {
    // Never leave them running, but report them.
    for (const line of left) {
      const pid = line.split(' ')[0];
      await new Promise((done) => execFile('taskkill', ['/PID', pid, '/T', '/F'], { windowsHide: true }, () => done()));
    }
  }
  const open = {};
  for (const [name, port] of Object.entries(ports)) open[name] = await listening(port);
  result.cleanup = { leftover_processes: left, ports_listening: open };
  const clean = !left.length && Object.values(open).every((value) => !value);
  if (clean) {
    pass('cleanup_no_processes_ports_closed', result.cleanup);
  } else {
    result.result = 'FAIL';
    result.failed_check ??= 'cleanup_no_processes_ports_closed';
    console.error(`FAIL cleanup: ${JSON.stringify(result.cleanup)}`);
    process.exitCode = 1;
  }
  result.finished = new Date().toISOString();
  await writeFile(join(work, 'result.json'), JSON.stringify(result, null, 2));
  console.log(`result: ${result.result}`);
  console.log(`evidence: ${work}`);
}
