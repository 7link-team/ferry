/** Isolated Server/Runner acceptance. Fake auth proves directory wiring only;
 * no inference, user login copying, system CLI install, or production writes. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { createServer } from 'node:net';
import { request } from 'playwright';
import { pathToFileURL } from 'node:url';

const dsh = process.env.AGENT_TEST_RUNTIME === 'dsh';

const work = await mkdtemp(join(tmpdir(), 'agent-runtime-'));
const state = join(work, 'runner');
await mkdir(state);
const policy = join(state, 'agent-runtime.json');
await writeFile(policy, JSON.stringify({ isolate_config: true }));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const bind = `127.0.0.1:${listener.address().port}`, origin = `http://${bind}`;
await new Promise(r => listener.close(r));
const env = { ...process.env, FERRY_BIND: bind, FERRY_DATA_DIR: join(work, 'server') };
// Windows environment names are case insensitive, but this copied JS object is
// not. Avoid passing both Path and PATH to the native Runner.
const runtimePath = process.env.PATH;
for (const key of Object.keys(env)) if (key.toUpperCase() === 'PATH') delete env[key];
env.PATH = runtimePath;
for (const key of ['FERRY_DATABASE_URL', 'FERRY_REDIS_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE']) delete env[key];
if (process.env.AGENT_TEST_EXECUTABLE_DIR) env.PATH = process.env.AGENT_TEST_EXECUTABLE_DIR + (process.platform === 'win32' ? ';' : ':') + env.PATH;
else if (process.platform === 'win32') {
  const require = createRequire(join(process.env.APPDATA, 'npm/node_modules/@openai/codex/package.json'));
  env.PATH = join(dirname(require.resolve('@openai/codex-win32-x64/package.json')), 'vendor/x86_64-pc-windows-msvc/bin') + ';' + env.PATH;
}
const suffix = process.platform === 'win32' ? '.exe' : '';
const serverLog = await open(join(work, 'server.log'), 'a'), runnerLog = await open(join(work, 'runner.log'), 'a');
let server, runner;
const contexts = [];
const report = { result: 'RUNNING', work, checks: [] };
const pass = name => { report.checks.push(name); console.log('PASS', name); };
async function wait(fn) {
  for (let i = 0; i < 200; i++) { if (await fn()) return; await new Promise(r => setTimeout(r, 150)); }
  throw Error('Acceptance condition timed out');
}
async function stop(proc) {
  if (!proc || proc.exitCode !== null || proc.signalCode !== null) return;
  const ended = once(proc, 'exit'); proc.kill(); await ended;
}
try {
  server = spawn(resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${suffix}`), [], { env, windowsHide: true, stdio: ['ignore', serverLog.fd, serverLog.fd] });
  await wait(async () => { assert.equal(server.exitCode, null); try { return (await fetch(origin + '/api/status')).ok; } catch { return false; } });
  const owner = await request.newContext(), outsider = await request.newContext(); contexts.push(owner, outsider);
  const api = context => async (path, data, method = data ? 'POST' : 'GET', expected) => {
    const response = await context.fetch(origin + '/api' + path, { method, headers: { 'X-Ferry-Request': '1', 'Accept-Language': 'en-US' }, ...(data ? { data } : {}) });
    assert(expected ? response.status() === expected : response.ok(), `${method} ${path}: ${response.status()} ${await response.text()}`);
    return response.status() === 204 ? null : response.json();
  };
  const admin = api(owner), other = api(outsider);
  await admin('/setup', { name: 'Owner', email: 'owner@example.invalid', password: 'temporary-runtime-acceptance', token: (await readFile(join(env.FERRY_DATA_DIR, 'setup-token'), 'utf8')).trim() });
  await other('/users', { name: 'Other', email: 'other@example.invalid', password: 'temporary-other-acceptance' });
  const workspace = await admin('/workspaces', { name: 'Runtime acceptance', slug: 'runtime-acceptance' });
  const root = `/workspaces/${workspace.id}`;
  const host = await admin(root + '/nodes', { name: 'Isolated runtime', connection: 'runner', runner_auto_update: 'off' });
  const token = await admin(root + '/runner-tokens', { host_id: host.id });
  runner = spawn(resolve(process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${suffix}`), ['--server', origin, '--token', token.token, '--state-dir', state], { env, windowsHide: true, stdio: ['ignore', runnerLog.fd, runnerLog.fd] });
  await wait(async () => { assert.equal(runner.exitCode, null); return (await admin(root + '/nodes/' + host.id)).state === 'online'; });
  const me = await admin('/me');
  const grants = ['hosts.read', 'host-groups.read', 'host-groups.create'];
  const role = dsh ? await admin(root + '/roles', { name: 'Runtime tool acceptance', permissions: grants }) : null;
  const body = { runtime: dsh ? 'dsh' : 'codex', host_id: host.id, owner_id: me.user.id, cwd: '', permission: 'read_only', visibility: 'workspace', state: 'enabled',
    ...(dsh ? { model: 'runtime-test/model-two', platform_role_id: role.id } : {}) };
  const a = await admin(root + '/ai-agents', { ...body, name: 'First Agent' });
  const b = await admin(root + '/ai-agents', { ...body, name: 'Second Agent' });
  const create = agent => ({ agent_id: agent.id, title: 'Configuration isolation acceptance' });
  if (dsh) {
    const inspected = await admin(root + '/ai-agents/' + a.id + '/probes', {});
    assert.equal(inspected.status.error, null, JSON.stringify(inspected));
    const conversation = await admin(root + '/conversations', create(a));
    const path = root + '/conversations/' + conversation.id;
    const cliRoot = process.env.AGENT_TEST_EXECUTABLE_DIR;
    assert(cliRoot, 'AGENT_TEST_EXECUTABLE_DIR must contain the installed official npm package');
    const home = join(state, 'agent-configs', a.id, '.dsh');
    const cli = join(cliRoot, 'node_modules/@deepseek-ai/dsh/lib/bin.js');
    const init = spawn(process.execPath, [cli, 'acp', '--dump-config'], { env: { ...env, DSH_HOME: home }, windowsHide: true, stdio: ['ignore', runnerLog.fd, runnerLog.fd] });
    assert.equal((await once(init, 'exit'))[0], 0);
    const provider = join(home, 'test-provider.mjs');
    const moduleUrl = pathToFileURL(join(cliRoot, 'node_modules/@deepseek-ai/dsh-llm/lib/index.js')).href;
    await writeFile(provider, (await readFile('scripts/fixtures/dsh-provider.mjs', 'utf8')).replace('__LLM_MODULE_URL__', moduleUrl));
    const patch = [
      ...['llm-deepseek', 'llm-deepseek-account', 'llm-pi-ai'].map(id => ({ id, disabled: true })),
      { id: 'agent-default-model', config: { provider: 'runtime-test', model: 'model-one' } },
      { id: 'acp', config: { provider: 'runtime-test', model: 'model-one' } },
      { insert: [{ id: 'runtime-test-provider', name: pathToFileURL(provider).href, inject: ['llm'] }] },
    ];
    await writeFile(join(home, 'profiles/acp/cordis.patch.yml'), JSON.stringify(patch));
    async function send(test, target = path) {
      return admin(target + '/turns', { prompt: 'DSH_TEST:' + JSON.stringify(test), attachments: [], client_id: crypto.randomUUID() });
    }
    async function completed(turn, target = path, expected = 'completed') {
      let detail;
      await wait(async () => {
        detail = await admin(target);
        const saved = detail.turns.find(item => item.id === turn.id);
        if (!saved || ['queued', 'running', 'stopping'].includes(saved.state)) return false;
        assert.equal(saved.state, expected, JSON.stringify(saved));
        return true;
      });
      return detail;
    }
    async function transcript(target = path) {
      let after = 0, rows = [];
      for (;;) {
        const page = await admin(target + '/events?after=' + after);
        rows.push(...page.items);
        if (!page.has_more) return JSON.stringify(rows);
        after = page.next;
      }
    }
    const first = await completed(await send({ action: 'text' }));
    assert.match(await transcript(), /DSH_TEST_RESULT model=model-two/);
    await completed(await send({ action: 'text' }));
    // The detail view omits native IDs; execution events retain the binding.
    const sessions = JSON.parse(await transcript()).filter(row => row.event.type === 'session');
    assert.equal(sessions.length, 2);
    assert(sessions[0].event.native_session_id);
    assert.equal(sessions[0].event.native_session_id, sessions[1].event.native_session_id);
    pass('official DSH process creates and resumes the same session with the selected model');
    await completed(await send({ action: 'mcp', path: '/nodes' }));
    assert((await transcript()).includes(host.id));
    pass('real DSH MCP process calls the current workspace API through the Runner');
    await completed(await send({ action: 'mcp', path: '/credentials' }));
    assert.match(await transcript(), /403/);
    const foreign = await admin('/workspaces', { name: 'Foreign workspace', slug: 'runtime-foreign' });
    const foreignHost = await admin(`/workspaces/${foreign.id}/nodes`, { name: 'Must stay private', connection: 'runner' });
    await completed(await send({ action: 'mcp', path: '/nodes/' + foreignHost.id }));
    const text = await transcript();
    assert.match(text, /404/);
    assert(!text.includes('Must stay private'));
    pass('DSH MCP rejects ungranted features and foreign workspace resources');
    const write = await send({ action: 'mcp', method: 'POST', path: '/node-groups', body: { name: 'DSH approved group' } });
    let decision;
    await wait(async () => {
      const turn = (await admin(path)).turns.find(item => item.id === write.id);
      decision = Object.entries(turn?.decisions || {}).find(([, item]) => item.tool === 'ferry_request' && item.decision === null);
      return !!decision;
    });
    assert(!(await admin(root + '/node-groups')).items.some(item => item.name === 'DSH approved group'));
    await admin(path + '/turns/' + write.id + '/decisions', { request_id: decision[0], allow: true });
    await completed(write);
    assert.equal((await admin(root + '/node-groups')).items.filter(item => item.name === 'DSH approved group').length, 1);
    pass('DSH platform writes require confirmation and create exactly one record');
    await completed(await send({ action: 'write' }));
    const directory = first.binding.canonical_directory.replace(/^\\\\\?\\/, '');
    await assert.rejects(readFile(join(directory, 'runtime-side-effect.txt')));
    pass('DSH read-only sandbox cannot mutate the conversation workspace');
    const failureConversation = await admin(root + '/conversations', create(a));
    const failurePath = root + '/conversations/' + failureConversation.id;
    await completed(await send({ action: 'fail' }, failurePath), failurePath, 'failed');
    pass('native DSH failure is recorded as failed');
    const stalled = await send({ action: 'wait' });
    await wait(async () => (await admin(path)).turns.find(item => item.id === stalled.id)?.state === 'running');
    await admin(path + '/turns/' + stalled.id + '/cancellations', {});
    await completed(stalled, path, 'cancelled');
    pass('DSH process cancellation terminates the active turn');
    await other(path, undefined, 'GET', 404);
    await other(root + '/ai-agents/' + a.id, undefined, 'GET', 404);
    await admin(root + '/conversations', { ...create(b), cwd: directory }, 'POST', 409);
    await assert.rejects(readFile(join(state, 'agent-configs', b.id, '.dsh/profiles/acp/cordis.patch.yml')));
    pass('DSH configuration and conversation resources remain isolated between Agents and accounts');
  } else {
  await admin(root + '/conversations', create(a), 'POST', 409);
  pass('isolated managed conversation refuses inherited native login');
  const home = join(state, 'agent-configs', a.id, '.codex');
  // Native login status checks presence, not provider validity. This value must
  // never be used for inference; the script never sends a conversation turn.
  await writeFile(join(home, 'auth.json'), JSON.stringify({ OPENAI_API_KEY: 'acceptance-placeholder-not-a-real-key' }));
  const conversation = await admin(root + '/conversations', create(a));
  const again = await admin(root + '/conversations', create(a));
  assert.notEqual(conversation.id, again.id);
  const first = await admin(root + '/conversations/' + conversation.id), second = await admin(root + '/conversations/' + again.id);
  assert.notEqual(first.binding.canonical_directory, second.binding.canonical_directory);
  assert.equal(first.binding.execution_identity, second.binding.execution_identity);
  pass('same Agent reuses isolated auth across distinct conversation workdirs');
  await admin(root + '/conversations', create(b), 'POST', 409);
  await assert.rejects(readFile(join(state, 'agent-configs', b.id, '.codex/auth.json')));
  pass('another Agent does not inherit the first Agent authentication file');
  const denied = await owner.fetch(origin + '/api' + root + '/conversations', { method: 'POST', headers: { 'X-Ferry-Request': '1' }, data: { ...create(b), cwd: first.binding.canonical_directory } });
  assert(!denied.ok(), 'manual cwd must not select another Agent home');
  pass('manual managed-path override cannot select another Agent configuration');
  await other(root + '/ai-agents/' + a.id, undefined, 'GET', 404);
  await other(root + '/conversations/' + conversation.id, undefined, 'GET', 404);
  pass('nonmember cannot read isolated Agent or conversation across workspace');
  await assert.rejects(readFile(join(state, 'agent-runtimes/codex/current.json')));
  pass('disabled automatic installation leaves no managed executable record');
  }
  report.result = 'PASS';
} catch (error) { report.result = 'FAIL'; report.error = String(error); throw error; }
finally {
  await stop(runner); await stop(server);
  for (const context of contexts) await context.dispose();
  await serverLog.close(); await runnerLog.close();
  await mkdir('data/maintenance', { recursive: true });
  await writeFile(`data/maintenance/${dsh ? 'dsh-runtime-http' : 'agent-runtime-http'}.json`, JSON.stringify(report, null, 2));
}
