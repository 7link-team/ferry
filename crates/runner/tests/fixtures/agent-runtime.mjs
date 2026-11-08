// Native-protocol fixture; Node is already required by the repository toolchain.
import readline from 'node:readline';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
const mode = process.argv[2];
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
const pending = setInterval(() => {}, 1000);
if (mode === 'eof') process.exit(0);
if (mode === 'malformed') { process.stdout.write('not-json\n'); process.exit(0); }
const lines = readline.createInterface({ input: process.stdin });
lines.on('close', () => { clearInterval(pending); process.exit(0); });
lines.on('line', line => {
  const value = JSON.parse(line);
  if (value.method === 'initialize') send({ id: 0, result: {} });
  if (value.method === 'thread/start') {
    assert.equal(value.params.approvalPolicy, 'untrusted');
    assert.equal(value.params.sandbox, 'workspace-write');
    send({ id: 1, result: { thread: { id: 'fixture-thread' } } });
  }
  if (value.method === 'turn/start') {
    assert.equal(value.params.threadId, 'fixture-thread');
    assert.equal(value.params.input[0].text, 'hello');
    send({ id: 2, result: { turn: { id: 'fixture-turn' } } });
    send({ method: 'turn/started', params: { threadId: 'fixture-thread', turn: { id: 'fixture-turn' } } });
    if (mode === 'descendant') {
      const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { windowsHide: true, stdio: 'ignore' });
      send({ method: 'item/agentMessage/delta', params: { threadId: 'fixture-thread', turnId: 'fixture-turn', itemId: 'child', delta: String(child.pid) } });
    }
    if (mode === 'complete') send({ id: 90, method: 'item/fileChange/requestApproval', params: { threadId: 'fixture-thread', turnId: 'fixture-turn' } });
    if (mode === 'flood') {
      const flood = () => {
        const more = process.stdout.write(JSON.stringify({ method: 'item/agentMessage/delta', params: { threadId: 'fixture-thread', turnId: 'fixture-turn', itemId: 'a', delta: 'x'.repeat(10000) } }) + '\n');
        if (more) setImmediate(flood); else process.stdout.once('drain', flood);
      };
      flood();
    }
  }
  if (value.id === 90) {
    assert.deepEqual(value.result, { decision: 'accept' });
    send({ method: 'item/completed', params: { threadId: 'fixture-thread', turnId: 'fixture-turn', item: { id: 'answer', type: 'agentMessage', text: 'fixture reply' } } });
    send({ method: 'turn/completed', params: { threadId: 'fixture-thread', turn: { id: 'fixture-turn', status: 'completed' } } });
  }
});
