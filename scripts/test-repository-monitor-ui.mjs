/** Production UI interaction acceptance; delivery/encryption are verified separately by HTTP tests. */
import assert from 'node:assert/strict';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
const workspace = '109124796398174208';
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const page = await browser.newPage({ locale: 'zh-CN', viewport: { width: 1280, height: 900 } });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  let saves = 0;
  let monitor = { id: 'c1', name: 'Notification test', provider: 'github', mode: 'poll', enabled: true, all_repositories: true, events: ['push'], poll_interval_minutes: 60, version: 1,
    notification_channels: [{ name: 'Operations', webhook: 'https://notify.example.invalid/ops', language: 'zh-CN', enabled: true }] };
  await page.route('**/api/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    let body;
    if (path === '/api/setup') body = { required: false };
    else if (path === '/api/me') body = { user: { id: 'u1', name: 'Tester', email: 'tester@example.invalid', is_superadmin: false }, workspaces: [{ id: workspace, slug: 'ops', name: 'Test workspace', permissions: ['repositories.read', 'repositories.update'] }] };
    else if (path.endsWith('/preferences')) body = { default_page_size: 20, dictionaries: {} };
    else if (path.endsWith('/code-connections')) body = { items: [{ id: 'c1', name: 'Connection', provider: 'github', state: 'connected', version: 1, recent_commit_event: { repository: 'team/repository', reference: 'feature/non-default', summary: 'Latest commit across branches', created_at: Date.now() } }], total: 1, page: 1, page_size: 20 };
    else if (path.endsWith('/code-connections/c1')) body = { id: 'c1', name: 'Connection', provider: 'github', state: 'connected', version: 1 };
    else if (path.endsWith('/code-connection-monitors/c1')) {
      if (request.method() === 'PATCH') { saves++; monitor = { ...monitor, ...request.postDataJSON(), version: monitor.version + 1 }; }
      body = monitor;
    } else body = { items: [], total: 0, page: 1, page_size: 20 };
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(`${origin}/zh-CN/workspaces/${workspace}/code-connections/c1?tab=monitoring`);
  await page.getByRole('button', { name: '修改订阅配置', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox', { name: 'Webhook URL 1', exact: true }).waitFor();
  assert.equal(await dialog.getByRole('textbox', { name: 'Webhook URL 1', exact: true }).inputValue(), monitor.notification_channels[0].webhook);
  await dialog.getByRole('button', { name: '添加', exact: true }).click();
  await dialog.getByRole('textbox', { name: '名称 2', exact: true }).fill('Development');
  await dialog.getByRole('textbox', { name: 'Webhook URL 2', exact: true }).fill('https://notify.example.invalid/development');
  await dialog.locator('select').filter({ has: page.locator('option[value="ja-JP"]') }).nth(1).selectOption('ja-JP');
  await dialog.getByRole('button', { name: '确认修改订阅配置', exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(saves, 1);
  assert.deepEqual(monitor.notification_channels.map(channel => [channel.name, channel.language]), [['Operations', 'zh-CN'], ['Development', 'ja-JP']]);
  await page.reload();
  await page.getByRole('button', { name: '修改订阅配置', exact: true }).click();
  assert.equal(await dialog.getByRole('textbox', { name: 'Webhook URL 2', exact: true }).inputValue(), 'https://notify.example.invalid/development');
  const languages = dialog.locator('select').filter({ has: page.locator('option[value="ja-JP"]') });
  assert.deepEqual(await languages.evaluateAll(selects => selects.map(select => select.value)), ['zh-CN', 'ja-JP']);
  await dialog.getByRole('button', { name: '删除 1', exact: true }).click();
  await dialog.getByRole('button', { name: '确认修改订阅配置', exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(monitor.notification_channels.length, 1);
  assert.equal(monitor.notification_channels[0].name, 'Development');
  await page.goto(`${origin}/zh-CN/workspaces/${workspace}/code-connections`);
  await page.getByRole('columnheader', { name: '最近提交事件', exact: true }).waitFor();
  const recent = page.getByRole('cell').filter({ hasText: 'Latest commit across branches' });
  assert.match(await recent.innerText(), /feature\/non-default/);
  assert.deepEqual(errors, []);
  console.log('PASS visible saved Webhook URLs, multiple independent language channels, save/reload and removal in production Chromium UI');
  console.log('PASS connection list renders latest cross-branch commit event summary, branch and timestamp');
} finally { await browser?.close(); await new Promise(done => server.httpServer.close(done)); }
