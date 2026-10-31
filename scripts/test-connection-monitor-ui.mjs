/** Real Chromium coverage of repository grouping and connection monitoring settings. */
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { createServer as createNetServer } from 'node:net';
const listener = createNetServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
const server = await createServer({ server: { host: '127.0.0.1', port, strictPort: true } });
let browser;
try {
  await server.listen();
  const origin = server.resolvedUrls.local[0].replace(/\/$/, '');
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  let saved;
  const monitor = { id: '42', provider: 'codeup', enabled: true, all_repositories: false, repositories: 'org/team/repo', events: ['push'], version: 2, has_secret: true, has_webhook: true, callback_path: '/api/repository-events/42' };
  await page.route('**/api/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    let json = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/setup') json = { required: false, has_users: true, registration_enabled: false };
    if (path === '/api/me') json = { user: { id: '1', name: 'Owner', email: 'owner@example.invalid' }, workspaces: [{ id: '2', name: 'Operations', slug: 'ops', role_name: 'Owner', permissions: ['repositories.read', 'repositories.update'] }] };
    if (path.endsWith('/preferences')) json = { default_page_size: 20, dictionaries: {} };
    if (path === '/api/workspaces/2/code-connections/42') json = { id: '42', provider: 'codeup', name: 'Team Codeup', version: 1 };
    if (path.endsWith('/42/repositories')) json = { items: ['org/team/repo', 'org/other/second'].map(full_name => ({ full_name, name: full_name.split('/').at(-1), url: 'https://git.example.invalid/' + full_name, default_branch: 'main' })), has_more: false };
    if (path.endsWith('/code-connection-monitors/42')) {
      if (request.method() === 'PATCH') saved = request.postDataJSON();
      json = monitor;
    }
    await route.fulfill({ json });
  });
  await page.goto(origin + '/workspaces/2/code-connections/42?tab=repositories');
  await page.getByRole('cell', { name: 'org/team/repo', exact: true }).waitFor();
  await page.getByRole('button', { name: '分组', exact: true }).click();
  const group = page.getByRole('button', { name: /org\/team.*当前页/ });
  await group.click();
  await page.getByRole('cell', { name: 'org/team/repo', exact: true }).waitFor({ state: 'hidden' });
  await group.click();
  await page.getByRole('cell', { name: 'org/team/repo', exact: true }).waitFor();
  await page.getByRole('button', { name: '列表', exact: true }).click();
  await page.getByRole('tab', { name: '监控通知', exact: true }).click();
  await page.getByRole('button', { name: '配置监控', exact: true }).click();
  const dialog = page.getByRole('dialog');
  assert.equal(await dialog.locator('textarea').inputValue(), 'org/team/repo');
  assert.equal(await dialog.getByText('新增仓库', { exact: true }).count(), 0);
  await dialog.locator('button[type=submit]').click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(saved.version, 2);
  assert.deepEqual(saved.events, ['push']);
  assert.equal(saved.repositories, 'org/team/repo');
  assert.match(page.url(), /\/code-connections\/42\?/);
  for (const width of [1440, 768, 390]) {
    await page.setViewportSize({ width, height: 900 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `page overflow at ${width}`);
  }
  assert.deepEqual(errors, []);
  console.log('PASS: Chromium namespace grouping/collapse/list switch, monitoring modal saved values/provider capabilities/save in place, responsive widths 1440/768/390');
} finally { await browser?.close(); await server.close(); }
