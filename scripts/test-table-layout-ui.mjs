/** Real browser geometry: lists fill the viewport; only table data scrolls. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer as netServer } from 'node:net';
import { createServer as viteServer } from 'vite';
import { chromium } from 'playwright';
const work = await mkdtemp(join(tmpdir(), 'table-layout-'));
const listener = netServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const backend = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser, vite;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null);
    try { if ((await fetch(`${backend}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  const frontend = netServer();
  await new Promise(r => frontend.listen(0, '127.0.0.1', r));
  const frontendPort = frontend.address().port;
  await new Promise(r => frontend.close(r));
  vite = await viteServer({ server: { host: '127.0.0.1', port: frontendPort, strictPort: true, proxy: { '/api': { target: backend, ws: true } } } });
  await vite.listen();
  const origin = `http://127.0.0.1:${vite.httpServer.address().port}`;
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  let seedCookie = '';
  async function post(path, data) {
    // Seed through the backend directly; cookies remain shared with the UI on
    // 127.0.0.1. Layout assertions still exercise the real frontend/API proxy.
    const response = await fetch(`${backend}/api${path}`, { method: 'POST', headers: { 'X-Ferry-Request': '1', 'Content-Type': 'application/json', Cookie: seedCookie }, body: JSON.stringify(data), signal: AbortSignal.timeout(30000) });
    const cookies = response.headers.getSetCookie();
    if (cookies.length) {
      seedCookie = cookies.map(cookie => cookie.split(';')[0]).join('; ');
      await context.addCookies(cookies.map(cookie => {
        const value = cookie.split(';')[0], at = value.indexOf('=');
        return { name: value.slice(0, at), value: value.slice(at + 1), url: backend };
      }));
    }
    assert(response.ok, `${path}: ${!response.ok ? await response.text() : response.status}`);
    return response.json();
  }
  await post('/setup', { name: 'Layout Admin', email: 'layout@example.invalid', password: 'temporary-layout-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace_layout = (await post('/workspaces', { name: 'Layout', slug: 'layout' })).id;
  async function openList(path) {
    await page.goto(origin + path);
    await page.getByRole('navigation', { name: '分页', exact: true }).waitFor().catch(async error => {
      console.error('List did not load:', page.url(), await page.locator('body').innerText(), errors);
      throw error;
    });
    await page.getByRole('button', { name: '刷新', exact: true }).waitFor();
    await page.waitForFunction(() => !document.querySelector('button .animate-spin'));
    await page.evaluate(() => document.fonts.ready);
  }
  async function geometry() {
    return page.evaluate(() => {
      const table = document.querySelector('table');
      const scroll = table.parentElement;
      const footer = document.querySelector('nav[aria-label="分页"]').parentElement;
      const rect = e => { const r = (e || table).getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, height: r.height }; };
      const body = table.querySelector('tbody tr');
      return { viewport: innerHeight, width: innerWidth, footer: rect(footer), scroll: rect(scroll), header: rect(table.querySelector('thead')), first: rect(body.firstElementChild), last: rect(body.lastElementChild), data: rect(body.children[1]), scrollHeight: scroll.scrollHeight, clientHeight: scroll.clientHeight, scrollWidth: scroll.scrollWidth, clientWidth: scroll.clientWidth, pageHeight: document.documentElement.scrollHeight, pageWidth: document.documentElement.scrollWidth };
    });
  }
  function fills(g, label, padding = 0) {
    assert(Math.abs(g.footer.bottom - (g.viewport - padding)) < 2, `${label}: footer must stay at viewport bottom: ${JSON.stringify(g)}`);
    assert(g.scroll.height > 100, `${label}: data viewport collapsed`);
    assert(g.scroll.bottom <= g.footer.top, `${label}: footer overlaps data`);
    assert(g.pageHeight <= g.viewport + 1 && g.pageWidth <= g.width + 1, `${label}: outer page scrolls`);
  }
  await openList(`/workspaces/${workspace_layout}/credentials`);
  fills(await geometry(), 'empty list');
  console.log('PASS empty list fills available viewport');
  const seed = i => post(`/workspaces/${workspace_layout}/credentials`, { name: `Credential ${String(i).padStart(3, '0')}`, kind: 'token', username: 'user-' + 'x'.repeat(180), secret: 'fixture-only' });
  await seed(0);
  await openList(`/workspaces/${workspace_layout}/credentials`);
  fills(await geometry(), 'single row');
  await page.locator('tbody tr').first().locator('td').nth(1).click();
  assert.match(page.url(), /\/workspaces\/\d+\/credentials\/\d+\?scope=build$/);
  await page.getByText('Credential 000', { exact: true }).waitFor();
  await openList(`/workspaces/${workspace_layout}/credentials`);
  await page.locator('tbody tr').first().focus();
  await page.keyboard.press('Enter');
  assert.match(page.url(), /\/workspaces\/\d+\/credentials\/\d+\?scope=build$/);
  await openList(`/workspaces/${workspace_layout}/credentials`);
  await page.locator('tbody tr').first().getByRole('link', { name: '编辑' }).click();
  assert.match(page.url(), /\/workspaces\/\d+\/credentials\/\d+\/edit\?scope=build$/);
  await openList(`/workspaces/${workspace_layout}/credentials`);
  console.log('PASS single row keeps footer at bottom');
  for (let i = 1; i < 65; i++) await seed(i);
  for (const viewport of [{ width: 1280, height: 800 }, { width: 900, height: 600 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    await openList(`/workspaces/${workspace_layout}/credentials?page_size=100`);
    const before = await geometry();
    fills(before, `many rows ${viewport.width}`);
    assert(before.scrollHeight > before.clientHeight, 'vertical overflow is inside table');
    assert(before.scrollWidth > before.clientWidth, 'horizontal overflow is inside table');
    await page.locator('table').evaluate(table => { table.parentElement.scrollTop = 250; table.parentElement.scrollLeft = 350; });
    await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
    const after = await geometry();
    fills(after, `scrolled ${viewport.width}`);
    // Sticky cells, not the thead box itself, establish the fixed header frame.
    const headerTop = await page.locator('thead th').first().evaluate(e => e.getBoundingClientRect().top);
    assert(Math.abs(headerTop - before.scroll.top - 1) < 2, 'header remains at data viewport top');
    assert(Math.abs(after.first.left - before.first.left) < 2, 'first column remains fixed');
    assert(Math.abs(after.last.right - before.last.right) < 2, 'action column remains fixed');
    assert(after.data.left < before.data.left - 100 && after.data.top < before.data.top - 100, 'middle data scrolls both axes');
    assert(Math.abs(after.footer.top - before.footer.top) < 2, 'footer never moves with table rows');
    if (viewport.width === 1280) {
      const row = page.locator('tbody tr').nth(7);
      await row.locator('td').last().hover();
      for (const dark of [false, true]) {
        await page.evaluate(value => document.documentElement.classList.toggle('dark', value), dark);
        const colors = await row.locator('td').evaluateAll(cells => cells.map(cell => getComputedStyle(cell).backgroundColor));
        for (const color of colors) {
          assert(!color.includes('rgba') && !color.includes('/'), `hover cell must be opaque: ${color}`);
          assert.notEqual(color, 'transparent');
        }
        assert.equal(new Set(colors).size, 1, 'sticky and scrolling cells use the same opaque hover surface');
      }
      await page.evaluate(() => document.documentElement.classList.remove('dark'));
      console.log('PASS opaque sticky hover cells in light and dark themes');
    }
    console.log(`PASS ${viewport.width}x${viewport.height}: data scrolls both axes; header/footer/edge columns fixed`);
  }
  await page.setViewportSize({ width: 1280, height: 800 });
  await openList(`/workspaces/${workspace_layout}/members`);
  fills(await geometry(), 'members invitation');
  await page.getByRole('button', { name: '查看邀请码', exact: true }).click();
  await page.getByRole('button', { name: '复制邀请码', exact: true }).waitFor();
  fills(await geometry(), 'expanded invitation');
  await openList('/platform/users');
  fills(await geometry(), 'platform list', 20);
  // The heading renders before the settings request finishes, as on a busy CI runner.
  await page.route('**/api/platform/settings/1', async route => {
    await new Promise(done => setTimeout(done, 200));
    await route.continue();
  });
  await page.getByRole('link', { name: '平台设置', exact: true }).click();
  await page.getByRole('heading', { name: '平台设置', exact: true }).waitFor();
  await page.getByLabel('平台公开访问地址', { exact: true }).waitFor();
  const formScroll = await page.locator('main').evaluate(main => {
    const el = [...main.querySelectorAll('*'), main].find(el => el.scrollHeight > el.clientHeight && ['auto','scroll'].includes(getComputedStyle(el).overflowY));
    if (!el) return false;
    el.scrollTop = el.scrollHeight;
    return el.scrollTop > 0;
  });
  assert(formScroll, 'long platform form keeps its own scrolling');
  assert.deepEqual(errors, []);
  console.log('PASS member invitation, platform list, and long form scrolling');
} catch(error) {
  console.error(`::error::${String(error.stack || error).replaceAll('\r','').replaceAll('\n','%0A')}`);
  throw error;
} finally {
  await browser?.close();
  await vite?.close();
  server.kill('SIGTERM');
  await new Promise(r => { if (server.exitCode !== null) r(); else server.once('exit', r); });
  await log.close();
  await rm(work, { recursive: true, force: true });
}
