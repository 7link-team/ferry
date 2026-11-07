/** Isolated server + real Agent: cancellation, permission loss, timeout and disconnect. */
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import assert from 'node:assert/strict';

const work = await mkdtemp(join(tmpdir(), 'workflow-host-boundaries-'));
const report = { work, mode: process.env.FERRY_BOUNDARY_SSH_PORT ? 'ssh' : process.env.FERRY_BOUNDARY_CAPACITY ? 'capacity' : 'agent', checks: [] };
const delay = ms => new Promise(r => setTimeout(r, ms));
async function wait(fn, seconds = 30) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) { const value = await fn(); if (value) return value; await delay(150); }
  throw Error('Timed out waiting for acceptance condition');
}
const sockets = new Set();
let connections = 0;
const ssh = createServer(socket => {
  connections++; sockets.add(socket);
  socket.on('error', () => {});
  socket.on('data', () => {});
  socket.on('close', () => sockets.delete(socket));
  socket.write('SSH-2.0-AcceptanceFixture\r\n');
});
await new Promise(r => ssh.listen(0, '127.0.0.1', r));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${listener.address().port}`;
await new Promise(r => listener.close(r));
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: origin.replace('http://', ''),
  FERRY_SANDBOX_PATH: resolve('target/debug/ferry-sandbox.exe') };
for (const key of ['FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE', 'FERRY_ARTIFACT_STORE']) delete env[key];
const log = await open(join(work, 'server.log'), 'w');
const server = spawn(resolve('target/debug/ferry-server.exe'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser, runner;
const agentLogs = [];
const pass = name => { report.checks.push(name); console.log('PASS', name); };
async function stopRunner() {
  if (!runner || runner.exitCode !== null) return;
  const ended = once(runner, 'exit');
  if (process.platform === 'win32') {
    const kill = spawn('taskkill', ['/PID', String(runner.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
    await once(kill, 'exit');
  } else runner.kill();
  await ended;
}
try {
  await wait(async () => { assert.equal(server.exitCode, null); try { return (await fetch(origin + '/api/status')).ok; } catch { return false; } });
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const owner = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const user = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const api = context => async (path, data, method = data ? 'POST' : 'GET', expected) => {
    const response = await context.request.fetch(origin + '/api' + path, {
      method, timeout: 10000, headers: { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' }, ...(data ? { data } : {})
    });
    assert(expected ? response.status() === expected : response.ok(), `${method} ${path}: HTTP ${response.status()}`);
    return response.status() === 204 ? null : response.json();
  };
  const admin = api(owner), actor = api(user);
  await admin('/setup', { name: 'Host acceptance', email: 'owner@example.invalid', password: 'temporary-host-acceptance', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace = await admin('/workspaces', { name: 'Host boundary acceptance', slug: 'host-boundaries' });
  const root = '/workspaces/' + workspace.id;
  await actor('/users', { name: 'Run actor', email: 'actor@example.invalid', password: 'temporary-actor-acceptance' });
  await actor('/workspace-memberships', { code: workspace.join_code });
  const member = (await admin(root + '/members')).items.find(m => m.email === 'actor@example.invalid');
  const permissions = ['workflows.read', 'workflows.create', 'workflows.update', 'workflows.run', 'hosts.read', 'hosts.terminal'];
  let role = await admin(root + '/roles', { name: 'Host workflow operator', permissions });
  await admin(root + '/members/' + member.id, { role_id: role.id, version: member.version }, 'PATCH');
  const group = process.env.FERRY_BOUNDARY_CAPACITY ? await admin(root + '/node-groups', { name: 'Workflow capacity group' }) : undefined;
  const host = await admin(root + '/nodes', { name: 'Boundary fixture Agent', connection: 'runner', ...(group ? { group_id: group.id } : {}) });
  async function startRunner() {
    const token = await admin(root + '/runner-tokens', { host_id: host.id });
    const output = await open(join(work, `agent-${Date.now()}.log`), 'w');
    agentLogs.push(output);
    runner = spawn(resolve('target/debug/ferry-runner.exe'), ['--server', origin, '--token', token.token, '--state-dir', join(work, 'agent-' + Date.now())], { windowsHide: true, stdio: ['ignore', output.fd, output.fd] });
    await wait(async () => { assert.equal(runner.exitCode, null); return (await admin(root + '/nodes/' + host.id)).state === 'online'; });
  }
  await startRunner();
  const node = (id, type, params, i) => ({ id, name: id, type, type_version: 1, params, position: [320 * i, 160] });
  const graph = {
    engine: 'ferry-graph', schema: 1,
    nodes: [node('start', 'core.trigger.manual', { input_schema: { type: 'object' } }, 0),
      node('remote', 'integration.git.repository.remote_refs', { url: `ssh://127.0.0.1:${ssh.address().port}/repository` }, 1)],
    connections: [{ from: { node: 'start', port: 'main', index: 0 }, to: { node: 'remote', port: 'main', index: 0 } }],
    settings: { execution: { runtime: 'host', target_kind: 'host', target_ref: host.id } }
  };
  const flow = await actor(root + '/workflows', { name: 'Host boundary fixture', definition: JSON.stringify(graph) });
  const runs = root + '/workflow-executions/';
  const begin = async () => actor(root + '/workflows/' + flow.id + '/executions', { mode: 'manual', revision: 'draft', version: flow.version, entry_node_id: 'start', inputs: {} });
  const terminal = async (id, seconds = 35) => wait(async () => { const row = await admin(runs + id); return ['succeeded', 'failed', 'cancelled'].includes(row.state) && row; }, seconds);
  async function blockingRun() {
    const before = connections;
    const run = await begin();
    await wait(() => connections > before);
    return run;
  }
  if (process.env.FERRY_BOUNDARY_SSH_PORT) {
    const credential = await admin(root + '/credentials', { name: 'Isolated Linux SSH', kind: 'password', username: 'root', secret: 'fixture-ssh-password' });
    const sshHost = await admin(root + '/nodes', { name: 'Ephemeral real Linux SSH', connection: 'ssh', address: '127.0.0.1', port: Number(process.env.FERRY_BOUNDARY_SSH_PORT), username: 'root', credential_id: credential.id });
    assert((await admin(root + '/nodes/' + sshHost.id + '/connection-tests', {}, 'POST')).ok, 'SSH connection test');
    const sshGraph = structuredClone(graph);
    sshGraph.settings.execution.target_ref = sshHost.id;
    sshGraph.nodes[1] = node('remote', 'integration.git.repository.log', { directory: '/tmp/repository', limit: 5 }, 1);
    const sshFlow = await actor(root + '/workflows', { name: 'Real Linux SSH Git log', definition: JSON.stringify(sshGraph) });
    const launch = flow => actor(root + '/workflows/' + flow.id + '/executions', { mode: 'manual', revision: 'draft', version: flow.version, entry_node_id: 'start', inputs: {} });
    const real = await launch(sshFlow);
    assert.equal((await terminal(real.id)).state, 'succeeded');
    const attempts = (await admin(runs + real.id + '/node-runs')).items;
    const output = await admin(runs + real.id + '/node-runs/' + attempts.find(n => n.node_id === 'remote').id + '/output');
    assert.equal(output.ports[0].items[0].json.execution_host.connection, 'ssh');
    assert.match(output.ports[0].items[0].json.stdout, /SSH_WORKFLOW_REAL/);
    pass('real Linux SSH host runs Git log through SFTP/private scripts and persists actual SSH host identity');
    sshGraph.nodes[1] = node('remote', 'integration.git.repository.remote_refs', { url: `ssh://10.0.2.2:${ssh.address().port}/repository` }, 1);
    const blocked = await actor(root + '/workflows', { name: 'Real Linux SSH cancellation', definition: JSON.stringify(sshGraph) });
    const before = connections;
    const remote = await launch(blocked);
    await wait(() => connections > before);
    await actor(runs + remote.id + '/cancellations', {}, 'POST');
    assert.equal((await terminal(remote.id, 95)).state, 'cancelled');
    await wait(() => sockets.size === 0);
    pass('real Linux SSH cancellation stops the Git/SSH descendants before publishing cancelled');
    const next = await launch(sshFlow);
    assert.equal((await terminal(next.id)).state, 'succeeded');
    pass('real Linux SSH host slot is reusable after cancellation');
  } else if (group) {
    const grouped = structuredClone(graph);
    grouped.settings.execution = { runtime: 'host', target_kind: 'host_group', target_ref: group.id };
    const groupedFlow = await actor(root + '/workflows', { name: 'Capacity queue through host group', definition: JSON.stringify(grouped) });
    const launch = flow => actor(root + '/workflows/' + flow.id + '/executions', { mode: 'manual', revision: 'draft', version: flow.version, entry_node_id: 'start', inputs: {} });
    const activated = async id => wait(async () => (await admin(runs + id + '/node-runs')).items.find(n => n.node_id === 'remote' && n.state === 'running'));
    const holder = await blockingRun();
    const before = connections;
    const queued = await launch(groupedFlow);
    await activated(queued.id);
    await delay(600);
    assert.equal(connections, before, 'capacity-one host cannot execute two remote commands');
    assert.equal((await admin(runs + holder.id)).state, 'running');
    await actor(runs + queued.id + '/cancellations', {}, 'POST');
    assert.equal((await terminal(queued.id, 5)).state, 'cancelled');
    assert.equal(connections, before, 'cancelled queued work must never start');
    assert.equal((await admin(runs + holder.id)).state, 'running', 'cancelling a waiter must not stop its slot owner');
    pass('real Agent capacity-one queue cancels promptly through a host group without running or interrupting the active owner');
    const resumed = await launch(groupedFlow);
    await activated(resumed.id);
    assert.equal(connections, before);
    await actor(runs + holder.id + '/cancellations', {}, 'POST');
    assert.equal((await terminal(holder.id)).state, 'cancelled');
    await wait(() => connections > before);
    assert.equal(connections, before + 1, 'one queued task must acquire the freed slot once');
    await actor(runs + resumed.id + '/cancellations', {}, 'POST');
    assert.equal((await terminal(resumed.id)).state, 'cancelled');
    await wait(() => sockets.size === 0);
    pass('host-group waiter resumes exactly once after its active owner releases capacity');
    const foreign = await admin('/workspaces', { name: 'Foreign host group workspace', slug: 'foreign-host-group' });
    const foreignGroup = await admin('/workspaces/' + foreign.id + '/node-groups', { name: 'Foreign group' });
    const isolated = structuredClone(grouped);
    isolated.settings.execution.target_ref = foreignGroup.id;
    const isolatedFlow = await actor(root + '/workflows', { name: 'Foreign group must not run', definition: JSON.stringify(isolated) });
    const denied = await launch(isolatedFlow);
    const deniedConnections = connections;
    assert.equal((await terminal(denied.id)).state, 'failed');
    assert.equal(connections, deniedConnections);
    pass('host group references remain scoped to the execution workspace');
    const repository = join(work, 'group-repository'); await mkdir(repository);
    const git = spawn('git', ['init', repository], { windowsHide: true, stdio: 'ignore' });
    assert.equal((await once(git, 'exit'))[0], 0);
    grouped.nodes[1] = node('remote', 'integration.git.repository.status', { directory: repository.replaceAll('\\', '/') }, 1);
    const recovery = await actor(root + '/workflows', { name: 'Host group recovery', definition: JSON.stringify(grouped) });
    const recovered = await launch(recovery);
    assert.equal((await terminal(recovered.id)).state, 'succeeded');
    const attempts = (await admin(runs + recovered.id + '/node-runs')).items;
    const output = await admin(runs + recovered.id + '/node-runs/' + attempts.find(n => n.node_id === 'remote').id + '/output');
    assert.equal(output.ports[0].items[0].json.execution_host.id, host.id);
    pass('real host-group selection persists the chosen Agent identity and executes again after queue cancellation');
  } else {
  const cancelled = await blockingRun();
  const page = await user.newPage();
  const pageErrors = []; page.on('pageerror', e => pageErrors.push(e.message));
  await page.goto(origin + '/zh-CN' + root + '/workflows/' + flow.id);
  await page.getByRole('tab', { name: '执行记录', exact: true }).click();
  await page.getByRole('button', { name: '查看详情', exact: true }).first().click();
  await page.getByRole('button', { name: '取消执行', exact: true }).click();
  assert.equal((await terminal(cancelled.id)).state, 'cancelled');
  await wait(() => sockets.size === 0);
  await page.getByRole('button', { name: '取消中…', exact: true }).waitFor({ state: 'hidden' });
  await page.locator('h3').filter({ hasText: /已取消/ }).waitFor();
  const attempts = (await admin(runs + cancelled.id + '/node-runs')).items;
  assert.equal(attempts.find(n => n.node_id === 'start').state, 'succeeded');
  assert.equal(attempts.find(n => n.node_id === 'remote').state, 'cancelled');
  await actor(runs + cancelled.id + '/cancellations', {}, 'POST');
  const other = await admin('/workspaces', { name: 'Other workspace', slug: 'other-workspace' });
  await admin('/workspaces/' + other.id + '/workflow-executions/' + cancelled.id + '/cancellations', {}, 'POST', 404);
  assert.deepEqual(pageErrors, []);
  await page.screenshot({ path: join(work, 'cancelled.png') });
  pass('real browser cancels Agent Git; remote process stops, completed history survives, repeated cancellation is idempotent');

  // Revoke the actor's host execution permission while its remote process is alive.
  const revoked = await blockingRun();
  role = await admin(root + '/roles/' + role.id);
  await admin(root + '/roles/' + role.id, { name: role.name, version: role.version, permissions: permissions.filter(p => p !== 'hosts.terminal') }, 'PATCH');
  const revokedResult = await terminal(revoked.id);
  assert.equal(revokedResult.state, 'failed'); assert.equal(revokedResult.end_reason, 'permission_revoked');
  await wait(() => sockets.size === 0);
  const deniedConnections = connections;
  const denied = await begin();
  assert.equal((await terminal(denied.id)).end_reason, 'permission_revoked');
  assert.equal(connections, deniedConnections);
  // Cancellation itself still requires current run permission, including for the owner of the definition.
  role = await admin(root + '/roles/' + role.id);
  await admin(root + '/roles/' + role.id, { name: role.name, version: role.version, permissions: permissions.filter(p => p !== 'workflows.run') }, 'PATCH');
  await actor(runs + cancelled.id + '/cancellations', {}, 'POST', 403);
  role = await admin(root + '/roles/' + role.id);
  await admin(root + '/roles/' + role.id, { name: role.name, version: role.version, permissions }, 'PATCH');
  pass('revoking hosts.terminal stops the live Agent operation; a subsequent run cannot send a remote command');

  const disconnected = await blockingRun();
  await stopRunner();
  assert.equal((await terminal(disconnected.id)).state, 'failed');
  await wait(() => sockets.size === 0);
  await startRunner();
  pass('abrupt Agent process-tree termination ends the execution without a success output; Agent reconnects');

  const timed = await blockingRun();
  const started = Date.now();
  assert.equal((await terminal(timed.id, 225)).state, 'failed');
  assert(Date.now() - started >= 100000, 'real 120-second execution deadline must be exercised');
  await wait(() => sockets.size === 0);
  pass('real Agent 120-second command timeout kills the blocked Git/SSH process');

  // An isolated local repository proves released host capacity can execute again.
  const repository = join(work, 'repository'); await mkdir(repository);
  const git = spawn('git', ['init', repository], { windowsHide: true, stdio: 'ignore' });
  assert.equal((await once(git, 'exit'))[0], 0);
  const next = structuredClone(graph);
  next.nodes[1] = node('remote', 'integration.git.repository.status', { directory: repository.replaceAll('\\', '/') }, 1);
  const recovery = await actor(root + '/workflows', { name: 'Host capacity recovery', definition: JSON.stringify(next) });
  const recovered = await actor(root + '/workflows/' + recovery.id + '/executions', { mode: 'manual', revision: 'draft', version: recovery.version, entry_node_id: 'start', inputs: {} });
  assert.equal((await terminal(recovered.id)).state, 'succeeded');
  const recoveredNodes = (await admin(runs + recovered.id + '/node-runs')).items;
  const output = await admin(runs + recovered.id + '/node-runs/' + recoveredNodes.find(n => n.node_id === 'remote').id + '/output');
  assert.equal(output.ports[0].items[0].json.execution_host.id, host.id);
  pass('host capacity is released after failures and cancellation; subsequent real Agent Git status succeeds');
  }
  report.result = 'PASS';
} catch (error) {
  report.result = 'FAIL'; report.error = error.name + ': ' + error.message.split('\n')[0];
  console.error(report.error); process.exitCode = 1;
} finally {
  await stopRunner();
  for (const socket of sockets) socket.destroy();
  await new Promise(r => ssh.close(r));
  await browser?.close();
  for (const output of agentLogs) await output.close();
  await log.close();
  if (server.exitCode === null) { const ended = once(server, 'exit'); server.kill(); await ended; }
  await writeFile(join(work, 'acceptance.json'), JSON.stringify(report, null, 2));
  await mkdir('data/maintenance', { recursive: true });
  await writeFile('data/maintenance/workflow-host-boundaries-latest.json', JSON.stringify(report, null, 2));
  await writeFile(`data/maintenance/workflow-host-boundaries-${report.mode}.json`, JSON.stringify(report, null, 2));
  console.log('Acceptance evidence:', work);
}
