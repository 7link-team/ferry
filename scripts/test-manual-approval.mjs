import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium, request } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'ferry-approval-'));
const listener = createServer();
await new Promise(done => listener.listen(0, '127.0.0.1', done));
const port = listener.address().port;
await new Promise(done => listener.close(done));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve('target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let client;
try {
  for (let i = 0; i < 200; i++) {
    assert.equal(server.exitCode, null, 'server exited during startup');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 199, 'server startup timed out');
    await new Promise(done => setTimeout(done, 100));
  }
  client = await request.newContext({ baseURL: origin, extraHTTPHeaders: { 'X-Ferry-Request': '1' } });
  const post = async (path, data = {}) => {
    const response = await client.post(`/api${path}`, { data });
    assert(response.ok(), `${path}: ${await response.text()}`);
    return response.json();
  };
  await post('/setup', { name: 'Approval Admin', email: 'approval@example.invalid', password: 'temporary-approval-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace_approval = (await post('/workspaces', { name: 'Approval', slug: 'approval' })).id;
  const workflow = `name: Approval test\non: workflow_dispatch\njobs:\n  first:\n    name: First gate\n    ferry-approval: true\n  second:\n    name: Second gate\n    needs: first\n    ferry-approval: true\n`;
  const pipeline = await post(`/workspaces/${workspace_approval}/pipelines`, { name: 'Gated pipeline', definition_format: 'github-actions', definition: workflow });
  const run = await post(`/workspaces/${workspace_approval}/pipelines/${pipeline.id}/runs`);
  const detail = async () => (await client.get(`/api/workspaces/${workspace_approval}/pipeline-runs/${run.id}`)).json();
  const waitFor = async (predicate) => {
    for (let i = 0; i < 100; i++) {
      const current = await detail();
      if (predicate(current)) return current;
      await new Promise(done => setTimeout(done, 100));
    }
    throw new Error('approval state timed out');
  };
  let current = await waitFor(value => value.jobs?.find(job => job.key === 'first')?.state === 'waiting');
  assert.equal(current.jobs.find(job => job.key === 'second').state, 'queued');
  assert.equal(current.jobs.find(job => job.key === 'first').host_id, '0');
  if (process.env.FERRY_APPROVAL_SCREENSHOT) {
    const browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
    try {
      const context = await browser.newContext({ storageState: await client.storageState(), viewport: { width: 1440, height: 900 } });
      const page = await context.newPage();
      await page.goto(`${origin}/workspaces/${workspace_approval}/pipeline-runs/${run.id}`);
      await page.getByRole('button', { name: '确认继续' }).waitFor();
      await page.screenshot({ path: process.env.FERRY_APPROVAL_SCREENSHOT, fullPage: true });
    } finally { await browser.close(); }
  }
  await post(`/workspaces/${workspace_approval}/pipeline-runs/${run.id}/jobs/first/approvals`);
  current = await waitFor(value => value.jobs?.find(job => job.key === 'second')?.state === 'waiting');
  assert.equal(current.jobs.find(job => job.key === 'first').state, 'succeeded');
  const duplicate = await client.post(`/api/workspaces/${workspace_approval}/pipeline-runs/${run.id}/jobs/first/approvals`, { data: {} });
  assert.equal(duplicate.status(), 400);
  await post(`/workspaces/${workspace_approval}/pipeline-runs/${run.id}/jobs/second/approvals`);
  current = await waitFor(value => value.state === 'succeeded');
  assert(current.jobs.every(job => job.state === 'succeeded'));
  console.log('PASS manual approval waits, respects dependencies, rejects duplicate approval, and finishes without hosts');
} finally {
  await client?.dispose();
  server.kill('SIGTERM');
  await new Promise(done => { if (server.exitCode !== null) done(); else server.once('exit', done); });
  await log.close();
  await rm(work, { recursive: true, force: true });
}
