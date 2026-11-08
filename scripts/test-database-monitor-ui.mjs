/** Real SQLite metadata and shared ResourceTable browser acceptance, isolated from local services. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, open, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const work = await mkdtemp(join(tmpdir(), 'database-monitor-ui-'));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
const checks = [];
let browser;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 300, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN', 'X-Ferry-Request': '1' } });
  const setup = await context.request.post(`${origin}/api/setup`, { data: {
    token: (await readFile(join(work, 'setup-token'), 'utf8')).trim(),
    name: 'Monitor Admin', email: 'monitor@example.invalid', password: 'temporary-monitor-password',
  } });
  assert.equal(setup.status(), 200, await setup.text());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const path = `${origin}/zh-CN/platform/monitor/database?tab=tables`;
  const tableResponse = response => response.url().includes('/api/platform/monitor/database/tables?') && response.ok();
  await page.goto(path);
  await page.getByRole('cell', { name: /KiB/ }).first().waitFor();
  assert.equal(await page.locator('[data-list-toolbar]').count(), 1);
  assert.match(await page.locator('tbody').innerText(), /KiB/);
  checks.push('real SQLite sizes and shared toolbar');
  const search = page.locator('[data-list-toolbar] input');
  await search.fill('users');
  const searched = page.waitForResponse(tableResponse);
  await page.getByRole('button', { name: '搜索', exact: true }).click();
  const result = await (await searched).json();
  assert(result.items.every(row => row.name.includes('users')));
  assert.equal(new URL(page.url()).searchParams.get('db_tables.q'), 'users');
  await page.reload();
  await page.getByRole('cell', { name: 'users', exact: true }).waitFor();
  assert.equal(await search.inputValue(), 'users');
  checks.push('search, URL persistence and reload');
  await page.getByRole('combobox', { name: '类型', exact: true }).click();
  await page.getByRole('option', { name: '表', exact: true }).click();
  const typed = page.waitForResponse(tableResponse);
  await page.getByRole('button', { name: '搜索', exact: true }).click();
  assert((await (await typed).json()).items.every(row => row.kind === 'table'));
  checks.push('type selection');
  await search.fill('');
  const cleared = page.waitForResponse(tableResponse);
  await page.getByRole('button', { name: '搜索', exact: true }).click();
  await cleared;
  for (const sort of ['bytes', '-bytes']) {
    const sorted = page.waitForResponse(tableResponse);
    await page.getByRole('columnheader').getByRole('button', { name: /^占用空间/ }).click();
    const response = await sorted;
    assert.equal(new URL(response.url()).searchParams.get('sort'), sort);
    const values = (await response.json()).items.map(row => row.bytes).filter(value => value != null);
    assert.deepEqual(values, [...values].sort((a, b) => sort === 'bytes' ? a - b : b - a));
  }
  checks.push('numeric sorting in both directions');
  const paged = page.waitForResponse(tableResponse);
  await page.locator('nav button').last().click();
  assert.equal((await (await paged).json()).page, 2);
  checks.push('shared pagination');
  for (const width of [1440, 900, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(path);
    await page.getByRole('cell', { name: /KiB/ }).first().waitFor();
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${width}: page overflow`);
    await page.getByRole('button', { name: '搜索', exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: join(work, `tables-${width}.png`), fullPage: true });
    await page.locator('[data-auto-table-footer]').scrollIntoViewIfNeeded();
    assert(await page.locator('[data-auto-table-footer]').isVisible());
    await page.getByRole('cell', { name: /KiB/ }).first().scrollIntoViewIfNeeded();
    await page.screenshot({ path: join(work, `table-content-${width}.png`), fullPage: true });
    checks.push(`${width}: toolbar reachable, no page overflow`);
  }
  await search.fill('nonexistent_catalog_fixture');
  const empty = page.waitForResponse(tableResponse);
  await page.getByRole('button', { name: '搜索', exact: true }).click();
  assert.equal((await (await empty).json()).total, 0);
  await page.screenshot({ path: join(work, 'empty-320.png'), fullPage: true });
  checks.push('empty search');
  await page.route('**/api/platform/monitor/database/tables?**', route => route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'metadata fixture unavailable' }) }));
  await page.goto(path);
  await page.getByText('metadata fixture unavailable', { exact: false }).waitFor();
  await page.screenshot({ path: join(work, 'error-320.png'), fullPage: true });
  checks.push('controlled metadata error');
  await page.unroute('**/api/platform/monitor/database/tables?**');
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/api/platform/monitor/database/tables?**', async route => { await gate; await route.continue(); });
  await page.goto(path);
  await page.locator('[data-auto-table-root] [aria-busy="true"]').waitFor();
  await page.screenshot({ path: join(work, 'loading-320.png'), fullPage: true });
  release();
  await page.getByRole('cell', { name: /KiB/ }).first().waitFor();
  checks.push('loading skeleton and recovery to real data');
  assert.deepEqual(errors, []);
  await writeFile(join(work, 'acceptance.json'), JSON.stringify({ checks, errors, boundary: 'real local SQLite; error response uses a controlled fixture; PostgreSQL/MySQL and production not exercised' }, null, 2));
  console.log(JSON.stringify({ result: 'PASS', checks: checks.length, evidence: work }));
} finally {
  if (browser) await browser.close();
  server.kill();
  await log.close();
  console.log(`Evidence: ${work}`);
}
