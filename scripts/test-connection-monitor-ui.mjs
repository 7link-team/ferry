/** Real Chromium coverage of repository grouping and connection monitoring settings. */
import assert from 'node:assert/strict';
import { preview } from 'vite';
import { chromium } from 'playwright';
import { createServer as createNetServer } from 'node:net';
const listener = createNetServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
// Exercise the built, shipped UI without a development dependency optimizer.
const server = await preview({ preview: { host: '127.0.0.1', port, strictPort: true } });
let browser;
try {
  const origin = server.resolvedUrls.local[0].replace(/\/$/, '');
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  let saved;
  const monitor = { id: '42', provider: 'codeup', mode: 'push', poll_interval_minutes: 1, enabled: true, all_repositories: false, repositories: 'org/team/repo', events: ['push'], version: 2, has_secret: true, has_webhook: true, callback_path: '/api/repository-events/42' };
  await page.route('**/api/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    let json = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/setup') json = { required: false, has_users: true, registration_enabled: false };
    if (path === '/api/me') json = { user: { id: '1', name: 'Owner', email: 'owner@example.invalid' }, workspaces: [{ id: '2', name: 'Operations', slug: 'ops', role_name: 'Owner', permissions: ['repositories.read', 'repositories.update'] }] };
    if (path.endsWith('/preferences')) json = { default_page_size: 20, dictionaries: {} };
    if (path === '/api/workspaces/2/code-connections/42') json = { id: '42', provider: 'codeup', name: 'Team Codeup', version: 1 };
    if (path.endsWith('/42/repositories')) {
      const second = new URL(request.url()).searchParams.get('page') === '2';
      json = { items: (second ? ['org/other/second', 'root-repo'] : ['org/team/repo']).map(full_name => ({ full_name, name: full_name.split('/').at(-1), url: 'https://git.example.invalid/' + full_name, default_branch: 'main' })), has_more: !second };
    }
    if (path.endsWith('/code-connection-monitors/42')) {
      if (request.method() === 'PATCH') {
        saved = request.postDataJSON();
        Object.assign(monitor, saved, { version: monitor.version + 1 });
      }
      json = monitor;
    }
    await route.fulfill({ json });
  });
  await page.goto(origin + '/workspaces/2/code-connections/42?tab=repositories');
  await page.getByRole('cell', { name: 'org/team/repo', exact: true }).waitFor();
  await page.getByRole('button', { name: '分组', exact: true }).click();
  await page.getByRole('button', { name: 'org', exact: true }).click();
  await page.getByRole('cell', { name: 'org/team/repo', exact: true }).waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: 'team', exact: true }).click();
  await page.getByRole('cell', { name: 'repo', exact: true }).waitFor();
  assert.equal(await page.getByRole('cell', { name: 'second', exact: true }).count(), 0);
  await page.getByRole('button', { name: '上级分组', exact: true }).click();
  await page.getByRole('button', { name: 'other', exact: true }).click();
  await page.getByRole('cell', { name: 'second', exact: true }).waitFor();
  await page.reload();
  await page.getByRole('cell', { name: 'second', exact: true }).waitFor();
  await page.getByRole('button', { name: '全部仓库', exact: true }).click();
  await page.getByRole('cell', { name: 'root-repo', exact: true }).waitFor();
  await page.getByRole('button', { name: '列表', exact: true }).click();
  await page.getByRole('tab', { name: '监控通知', exact: true }).click();
  await page.getByRole('button', { name: '修改订阅配置', exact: true }).click();
  const dialog = page.getByRole('dialog');
  assert.equal(await dialog.locator('textarea').inputValue(), 'org/team/repo');
  assert.equal(await dialog.getByText('新增仓库', { exact: true }).count(), 0);
  await dialog.locator('button[type=submit]').click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(saved.version, 2);
  assert.deepEqual(saved.events, ['push']);
  assert.equal(saved.repositories, 'org/team/repo');
  assert.match(page.url(), /\/code-connections\/42\?/);
  await page.getByRole('button', { name: '修改订阅配置', exact: true }).click();
  await dialog.getByRole('combobox').click();
  await page.getByRole('option', { name: '主动轮询', exact: true }).click();
  assert.equal(await dialog.getByText('Webhook 验证密钥', { exact: true }).count(), 0);
  assert.equal(await dialog.getByText('新增仓库', { exact: true }).count(), 1);
  const interval = dialog.getByRole('spinbutton');
  assert.equal(await interval.inputValue(), '1');
  assert.equal(await interval.getAttribute('min'), '1');
  assert.equal(await dialog.locator('button[type=submit]').isDisabled(), true, 'mode switch clears prior selected events');
  await dialog.getByText('代码推送', { exact: true }).click();
  await interval.fill('0');
  assert.equal(await dialog.locator('button[type=submit]').isDisabled(), true);
  await interval.fill('2');
  await dialog.locator('button[type=submit]').click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(saved.mode, 'poll');
  assert.equal(saved.poll_interval_minutes, 2);
  assert.deepEqual(saved.events, ['push']);
  await page.getByText('/api/repository-events/42', { exact: true }).waitFor({ state: 'detached' });
  assert.equal(await page.getByText('/api/repository-events/42', { exact: true }).count(), 0, 'polling detail hides callback endpoint');
  assert.equal(await page.getByText('Webhook 验证密钥', { exact: true }).count(), 0, 'polling detail hides inbound secret');
  for (const width of [1440, 768, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const card = page.getByRole('region', { name: '订阅配置', exact: true });
    const box = await card.boundingBox();
    const formBox = await card.locator('..').boundingBox();
    assert(Math.abs(box.width - formBox.width) < 2, `subscription card fills form at ${width}`);
    const edit = card.getByRole('button', { name: '修改订阅配置', exact: true });
    const editBox = await edit.boundingBox();
    assert(editBox.x > box.x + box.width - 90, 'edit icon stays at right edge');
    assert.equal(await edit.getAttribute('title'), '修改订阅配置');
    assert.equal(await edit.innerText(), '', 'edit is icon-only');
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `page overflow at ${width}`);
  }
  assert.equal(await page.getByRole('region', { name: '仓库事件记录' }).count(), 0);
  await page.getByRole('tab', { name: '事件记录', exact: true }).click();
  await page.getByRole('region', { name: '仓库事件记录' }).waitFor();
  assert(new URL(page.url()).searchParams.get('tab') === 'events');
  await page.reload();
  await page.getByRole('region', { name: '仓库事件记录' }).waitFor();
  assert.deepEqual(errors, []);
  console.log('PASS: Chromium paginated namespace directory navigation/parent/breadcrumb/reload/list switch, monitoring modal saved values/provider capabilities/save in place, responsive widths 1440/768/390');
} finally { await browser?.close(); await new Promise(resolve => server.httpServer.close(resolve)); }
