/** Real Chromium: superadmin adds a platform template source in the UI, syncs it, and every workspace market shows it. */
import assert from 'node:assert/strict';
import { createServer as createHttpServer } from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(process.env.FERRY_MARKET_UI_OUTPUT || join(repo, 'data', 'maintenance', 'marketplace-p1'));
await mkdir(output, { recursive: true });
const work = await mkdtemp(join(tmpdir(), 'platform-market-ui-'));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
const bundle = JSON.parse(await readFile(join(repo, 'marketplace', 'index.json'), 'utf8'));
const fixture = createHttpServer((request, response) => {
  response.setHeader('Content-Type', 'application/json');
  response.end(JSON.stringify({ encoding: 'base64', content: Buffer.from(JSON.stringify(bundle)).toString('base64') }));
});
await new Promise(r => fixture.listen(0, '127.0.0.1', r));
const fixtureOrigin = `http://127.0.0.1:${fixture.address().port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}`, FERRY_TEST_MARKETPLACE_HTTP: '1' };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser;
const checks = [];
const check = (condition, label) => { assert(condition, label); checks.push(label); console.log('PASS', label); };
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const headers = { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' };
  const admin = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, viewport: { width: 1440, height: 900 } });
  const post = async (ctx, path, data) => {
    const response = await ctx.request.post(`${origin}/api${path}`, { headers, data });
    assert(response.ok(), `${path}: ${await response.text()}`); return response.json();
  };
  await post(admin, '/setup', { email: 'admin@example.invalid', name: 'Admin', password: 'temporary-admin-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace = (await post(admin, '/workspaces', { name: 'Tenant', slug: 'tenant' })).id;
  const page = await admin.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('dialog', d => d.accept());

  await page.goto(`${origin}/zh-CN/platform/marketplace-sources`);
  await page.getByText('Ferry official templates').first().waitFor();
  check(true, 'built-in official source is listed on a fresh install');
  await page.getByRole('button', { name: '添加来源', exact: true }).click();
  await page.getByRole('textbox', { name: /^名称/ }).fill('Ferry 官方模板');
  await page.getByRole('textbox', { name: /^API 接入点/ }).fill(fixtureOrigin);
  await page.getByRole('textbox', { name: /^项目路径/ }).fill('ferry/ferry');
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.waitForURL(url => !url.pathname.endsWith('/new'));
  const listed = await (await admin.request.get(`${origin}/api/platform/marketplace-sources`)).json();
  const source = listed.items.find(row => row.name === 'Ferry 官方模板');
  check(source && source.trust === 'official' && source.sync_interval_hours === 6 && source.index_path === 'marketplace/index.json', 'platform source created through the form with defaults');

  await page.goto(`${origin}/zh-CN/platform/marketplace-sources`);
  await page.getByRole('row', { name: /Ferry 官方模板/ }).getByRole('button', { name: '立即同步' }).click().catch(async () => {
    await page.getByRole('row', { name: /Ferry 官方模板/ }).getByRole('button').last().click();
    await page.getByRole('menuitem', { name: '立即同步' }).click();
  });
  await page.waitForFunction(async () => {
    const r = await fetch('/api/platform/marketplace-sources', { headers: { 'X-Ferry-Request': '1' } });
    return (await r.json()).items.some(i => i.entry_count > 0 && i.state === 'idle');
  });
  check(true, 'sync now fills the platform catalog');

  for (const width of [1440, 900, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`${origin}/zh-CN/platform/marketplace-sources`);
    await page.getByText('Ferry 官方模板').first().waitFor();
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `platform sources page overflows at ${width}`);
    await page.screenshot({ path: join(output, `platform-sources-${width}.png`), fullPage: true });
    await page.goto(`${origin}/zh-CN/workspaces/${workspace}/marketplace`);
    await page.getByText('HTTP 示例服务').first().waitFor();
    await page.locator('span:text-is("官方"):visible').first().waitFor();
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `marketplace page overflows at ${width}`);
    await page.screenshot({ path: join(output, `marketplace-${width}.png`), fullPage: true });
  }
  check(true, 'platform sources and workspace market fit 1440/900/390/320 without page overflow, official badge shown');
  assert.deepEqual(errors, [], 'page errors');
  check(true, 'no page errors');
  console.log(`${checks.length} checks passed; screenshots in ${output}`);
} catch (error) {
  console.error(error);
  console.error((await readFile(join(work, 'server.log'), 'utf8')).slice(-4000));
  process.exitCode = 1;
} finally {
  await browser?.close();
  server.kill();
  fixture.close();
  await log.close();
}
