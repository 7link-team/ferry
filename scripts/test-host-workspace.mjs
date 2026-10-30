// Real, isolated Runner + browser sessions. Never installs or changes a service.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer, createConnection } from 'node:net';
import { randomBytes } from 'node:crypto';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'host-workspace-'));
const listener = createServer();
await new Promise(done => listener.listen(0, '127.0.0.1', done));
const port = listener.address().port;
await new Promise(done => listener.close(done));
const origin = `http://127.0.0.1:${port}`;
const env = { ...process.env }; delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), ['--data-dir', work, '--bind', `127.0.0.1:${port}`], { env, stdio: 'ignore', windowsHide: true });
let runner, browser, heartbeatSocket;
let cookie = '';
const api = async (path, body) => {
  const response = await fetch(origin + '/api' + path, { method: body === undefined ? 'GET' : 'POST', headers: { 'Content-Type': 'application/json', 'X-Ferry-Request': '1', Cookie: cookie }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
  if (response.headers.getSetCookie().length) cookie = response.headers.getSetCookie().map(value => value.split(';')[0]).join('; ');
  const value = await response.json(); assert(response.ok, `${path}: ${JSON.stringify(value)}`); return value;
};
const until = async (check, label) => {
  for (let i = 0; i < 150; i++) { try { if (await check()) return; } catch {} await new Promise(done => setTimeout(done, 100)); }
  throw new Error(`Timed out: ${label}`);
};
try {
  await until(async () => (await fetch(origin + '/api/status')).ok, 'backend');
  await api('/setup', { name: 'Workspace test', email: 'workspace@example.invalid', password: 'temporary-workspace-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace_tools = (await api('/workspaces', { name: 'Tools', slug: 'tools' })).id;
  const host = await api(`/workspaces/${workspace_tools}/nodes`, { name: 'Workspace Runner', connection: 'runner' });
  const { token } = await api(`/workspaces/${workspace_tools}/runner-tokens`, { host_id: host.id });
  runner = spawn(resolve(process.env.RUNNER_BINARY || 'target/debug/ferry-runner'), ['--server', origin, '--token', token, '--state-dir', join(work, 'runner')], { env, stdio: 'ignore', windowsHide: true });
  await until(async () => (await api(`/workspaces/${workspace_tools}/nodes/${host.id}`)).state === 'online', 'runner');
  browser = await chromium.launch({ headless: true, ...(process.env.CHROME_BINARY ? { executablePath: process.env.CHROME_BINARY } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await context.addCookies(cookie.split('; ').map(value => { const at = value.indexOf('='); return { name: value.slice(0, at), value: value.slice(at + 1), url: origin }; }));
  const page = await context.newPage();
  const errors = []; const sessions = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('websocket', ws => { if (ws.url().includes('/terminal-sessions?')) sessions.push({ ws, closed: false }); ws.on('close', () => { const item = sessions.find(value => value.ws === ws); if (item) item.closed = true; }); });
  await page.goto(`${origin}/tools/host/hosts/${host.id}/workspace?tool=terminal`);
  await page.waitForURL(`${origin}/workspaces/${workspace_tools}/nodes/${host.id}/workspace?tool=terminal`);
  await page.getByLabel('实时主机监控').waitFor({ timeout: 20_000 });
  await page.getByLabel(/主机连接延迟：\d+ ms/).waitFor({ timeout: 20_000 });
  await until(() => sessions.length === 1, 'original terminal session');
  await page.getByRole('button', { name: '左右分屏', exact: true }).first().click();
  await page.getByRole('button', { name: '上下分屏', exact: true }).first().click();
  await page.getByRole('button', { name: '上下分屏', exact: true }).last().click();
  await page.getByRole('button', { name: '切换为文件', exact: true }).last().click();
  await until(() => sessions.length >= 4, 'split terminal sessions');
  assert.equal(sessions[0].closed, false, 'splitting must preserve the original live terminal');
  const checkLayout = async () => {
    const boxes = await page.locator('section[aria-label$="分屏"]').evaluateAll(elements => elements.filter(element => element.getBoundingClientRect().width > 0).map(element => {
      const { x, y, width, height } = element.getBoundingClientRect(); return { x, y, width, height };
    }));
    assert.equal(boxes.length, 4);
    const viewport = page.viewportSize();
    for (const box of boxes) {
      assert(box.width > 0 && box.height > 0);
      assert(box.x >= 0 && box.y >= 88 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1, 'pane outside viewport');
    }
    for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i], b = boxes[j];
      const overlap = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x) > 1 && Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y) > 1;
      assert.equal(overlap, false, 'panes overlap');
    }
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'workspace overflows horizontally');
  };
  await checkLayout();
  if (process.env.HOST_WORKSPACE_SCREENSHOT) await page.screenshot({ path: process.env.HOST_WORKSPACE_SCREENSHOT });
  await page.setViewportSize({ width: 900, height: 650 }); await checkLayout();
  await page.setViewportSize({ width: 600, height: 800 }); await checkLayout();
  assert.equal(sessions[0].closed, false);
  assert.deepEqual(errors, []);
  console.log('PASS mixed terminal/file split geometry at 1440, 900 and 600px; original terminal preserved; live metrics and host RTT');

  // Read actual control frames: browser devtools omit protocol-level Ping/Pong.
  let pings = 0, opened = false, ended = false, output = '', buffered = Buffer.alloc(0);
  heartbeatSocket = createConnection({ host: '127.0.0.1', port });
  heartbeatSocket.on('connect', () => heartbeatSocket.write(
    `GET /api/workspaces/${workspace_tools}/nodes/${host.id}/terminal-sessions?cols=80&rows=24 HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ${randomBytes(16).toString('base64')}\r\nSec-WebSocket-Version: 13\r\nOrigin: ${origin}\r\nCookie: ${cookie}\r\n\r\n`,
  ));
  heartbeatSocket.on('close', () => { ended = true; });
  heartbeatSocket.on('data', data => {
    buffered = Buffer.concat([buffered, data]);
    if (!opened) {
      const at = buffered.indexOf('\r\n\r\n'); if (at < 0) return;
      assert.match(buffered.subarray(0, at).toString(), /^HTTP\/1\.1 101 /);
      opened = true; buffered = buffered.subarray(at + 4);
    }
    while (buffered.length >= 2) {
      const opcode = buffered[0] & 15; let length = buffered[1] & 127; let offset = 2;
      if (length === 126) { if (buffered.length < 4) return; length = buffered.readUInt16BE(2); offset = 4; }
      else if (length === 127) { if (buffered.length < 10) return; length = Number(buffered.readBigUInt64BE(2)); offset = 10; }
      if (buffered.length < offset + length) return;
      const payload = buffered.subarray(offset, offset + length); buffered = buffered.subarray(offset + length);
      if (opcode === 2) output += payload.toString();
      if (opcode === 8) ended = true;
      if (opcode === 9) {
        pings++;
        const mask = randomBytes(4), frame = Buffer.alloc(6 + payload.length);
        frame[0] = 0x8a; frame[1] = 0x80 | payload.length; mask.copy(frame, 2);
        for (let i = 0; i < payload.length; i++) frame[6 + i] = payload[i] ^ mask[i % 4];
        heartbeatSocket.write(frame);
      }
    }
  });
  await until(() => opened && output.length > 0, 'PTY initial output');
  await new Promise(done => setTimeout(done, 1_000));
  const before = output;
  await new Promise(done => setTimeout(done, 22_000));
  assert(pings >= 2, 'idle terminal must receive recurring protocol heartbeats');
  assert.equal(ended, false, 'idle terminal disconnected');
  assert.equal(output, before, 'heartbeat injected data into shell');
  console.log('PASS idle terminal heartbeats without shell input');
} finally {
  heartbeatSocket?.destroy(); await browser?.close();
  for (const child of [runner, server]) if (child && child.exitCode === null) { child.kill(); await new Promise(done => child.once('exit', done)); }
  await rm(work, { recursive: true, force: true });
}
