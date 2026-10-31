// Real, isolated Runner + browser sessions. Never installs or changes a service.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, mkdir, writeFile } from 'node:fs/promises';
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
  const hostRoot = `/workspaces/${workspace_tools}/nodes/${host.id}`;
  const volumes = await api(hostRoot + '/file-systems');
  assert(volumes.items.length > 0, 'actual mounted partitions are enumerated');
  assert.equal(volumes.platform, process.platform === 'win32' ? 'windows' : 'unix');
  const fixtureDir = join(work, "files ' % &");
  await mkdir(fixtureDir);
  await writeFile(join(fixtureDir, 'visible.txt'), 'hello');
  await writeFile(join(fixtureDir, '.secret'), 'hidden');
  if (process.platform === 'win32') {
    await writeFile(join(fixtureDir, 'hidden.txt'), 'hidden');
    await writeFile(join(fixtureDir, 'system.sys'), 'system');
    execFileSync('attrib.exe', ['+h', join(fixtureDir, 'hidden.txt')], { windowsHide: true });
    execFileSync('attrib.exe', ['+s', join(fixtureDir, 'system.sys')], { windowsHide: true });
  }
  const folderQuery = encodeURIComponent(fixtureDir.replaceAll('\\', '/'));
  const visible = await api(hostRoot + '/files?path=' + folderQuery);
  assert.deepEqual(visible.items.map(row => row.name), ['visible.txt']);
  const all = await api(hostRoot + '/files?path=' + folderQuery + '&show_hidden=true');
  assert(all.items.some(row => row.name === '.secret'));
  if (process.platform === 'win32') {
    assert(all.items.some(row => row.name === 'hidden.txt'));
    assert(all.items.some(row => row.name === 'system.sys'));
    const backslash = await api(hostRoot + '/files?path=' + encodeURIComponent(fixtureDir));
    assert.deepEqual(backslash.items.map(row => row.name), ['visible.txt']);
  }
  console.log('PASS real mounted partitions and hidden-file filtering, including Windows attributes and quoted paths');
  browser = await chromium.launch({ headless: true, ...(process.env.CHROME_BINARY ? { executablePath: process.env.CHROME_BINARY } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await context.addCookies(cookie.split('; ').map(value => { const at = value.indexOf('='); return { name: value.slice(0, at), value: value.slice(at + 1), url: origin }; }));
  const page = await context.newPage();
  // ConPTY asks for cursor position before node metadata may arrive.
  await page.route(`${origin}/api${hostRoot}`, async route => {
    await new Promise(done => setTimeout(done, 1000));
    await route.continue();
  });
  const errors = []; const sessions = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('websocket', ws => {
    if (!ws.url().includes('/terminal-sessions?')) return;
    const item = { ws, closed: false, output: '' }; sessions.push(item);
    ws.on('framereceived', event => { item.output += Buffer.from(event.payload).toString('utf8'); });
    ws.on('close', () => { item.closed = true; });
  });
  await page.goto(`${origin}/tools/host/hosts/${host.id}/workspace?tool=terminal`);
  await page.waitForURL(`${origin}/workspaces/${workspace_tools}/nodes/${host.id}/workspace?tool=terminal`);
  await page.getByLabel('实时主机监控').waitFor({ timeout: 20_000 });
  await page.getByLabel(/主机连接延迟：\d+ ms/).waitFor({ timeout: 20_000 });
  await until(() => sessions.length === 1, 'original terminal session');
  const originalPane = page.locator('section[aria-label$="分屏"]').first();
  await originalPane.getByRole('tab', { name: '文件', exact: true }).click();
  if (process.platform === 'win32') await originalPane.getByText('当前位置：此电脑', { exact: true }).waitFor();
  else await originalPane.getByText('当前位置：/', { exact: true }).waitFor();
  await originalPane.getByRole('textbox', { name: '目录路径', exact: true }).fill(fixtureDir.replaceAll('\\', '/'));
  await originalPane.getByRole('textbox', { name: '目录路径', exact: true }).press('Enter');
  await originalPane.getByRole('button', { name: '预览 visible.txt', exact: true }).waitFor();
  await originalPane.getByRole('button', { name: '显示隐藏文件', exact: true }).click();
  await originalPane.getByRole('button', { name: '预览 .secret', exact: true }).waitFor();
  await originalPane.getByRole('tab', { name: '终端', exact: true }).click();
  assert.equal(sessions.length, 1, 'tool switch must reuse the terminal connection');
  assert.equal(sessions[0].closed, false);
  await originalPane.getByRole('tab', { name: '文件', exact: true }).click();
  await originalPane.getByRole('button', { name: '预览 .secret', exact: true }).waitFor();
  assert.equal(await originalPane.getByRole('textbox', { name: '目录路径', exact: true }).inputValue(), fixtureDir.replaceAll('\\', '/'));
  await originalPane.getByRole('tab', { name: '终端', exact: true }).click();
  console.log('PASS tool tabs preserve live terminal, file directory, and hidden-file preference');
  await page.getByRole('button', { name: '左右分屏', exact: true }).first().click();
  await page.getByRole('button', { name: '上下分屏', exact: true }).first().click();
  await page.getByRole('button', { name: '上下分屏', exact: true }).last().click();
  await page.getByRole('tab', { name: '文件', exact: true }).last().click();
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
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${origin}${hostRoot}/workspace?tool=files&directory=${folderQuery}`);
  await page.getByRole('button', { name: '预览 visible.txt', exact: true }).waitFor();
  await page.getByRole('button', { name: '在当前目录打开终端', exact: true }).click();
  await page.getByText('已连接', { exact: true }).waitFor();
  const folderSession = sessions.at(-1);
  await until(() => folderSession.output.includes(fixtureDir), 'file shortcut PTY enters exact quoted directory');
  console.log('PASS file shortcut opens a native Windows PTY in the exact selected directory');

  // Read actual control frames: browser devtools omit protocol-level Ping/Pong.
  let pings = 0, opened = false, ended = false, output = '', buffered = Buffer.alloc(0);
  heartbeatSocket = createConnection({ host: '127.0.0.1', port });
  const sendFrame = (opcode, payload) => {
    const mask = randomBytes(4), frame = Buffer.alloc(6 + payload.length);
    frame[0] = 0x80 | opcode; frame[1] = 0x80 | payload.length; mask.copy(frame, 2);
    for (let i = 0; i < payload.length; i++) frame[6 + i] = payload[i] ^ mask[i % 4];
    heartbeatSocket.write(frame);
  };
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
      if (opcode === 2) {
        output += payload.toString();
        // ConPTY waits for the terminal's cursor-position reply before cmd.exe
        // emits its prompt. A real xterm client supplies this automatically.
        if (process.platform === 'win32' && output.endsWith('\x1b[6n')) sendFrame(2, Buffer.from('\x1b[1;1R'));
      }
      if (opcode === 8) ended = true;
      if (opcode === 9) {
        pings++;
        sendFrame(10, payload);
      }
    }
  });
  await until(() => opened && output.length > 0, 'PTY initial output');
  if (process.platform === 'win32') {
    await until(() => output.toLowerCase().includes(process.env.USERPROFILE.toLowerCase()), 'Windows PTY user home prompt');
    assert(!output.toLowerCase().includes('system32\\config\\systemprofile'), 'terminal must not start in a service profile');
    console.log('PASS Windows PTY starts in the user home directory');
  }
  await new Promise(done => setTimeout(done, 1_000));
  const before = output;
  await new Promise(done => setTimeout(done, 22_000));
  assert(pings >= 2, 'idle terminal must receive recurring protocol heartbeats');
  assert.equal(ended, false, 'idle terminal disconnected');
  assert.equal(output, before, 'heartbeat injected data into shell');
  console.log('PASS idle terminal heartbeats without shell input');
} catch (error) {
  if (browser) {
    for (const context of browser.contexts()) for (const page of context.pages()) console.error('Workspace failure state:', await page.locator('body').innerText().catch(() => 'unavailable'));
  }
  throw error;
} finally {
  heartbeatSocket?.destroy(); await browser?.close();
  for (const child of [runner, server]) if (child && child.exitCode === null) { child.kill(); await new Promise(done => child.once('exit', done)); }
  await rm(work, { recursive: true, force: true });
}
