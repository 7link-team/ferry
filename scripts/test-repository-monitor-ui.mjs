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
  for (const width of [320, 375, 768, 1280]) {
    await page.setViewportSize({ width, height: 900 });
    const geometry = await dialog.evaluate(element => {
      const bounds = element.getBoundingClientRect();
      return { width: bounds.width, left: bounds.left, right: bounds.right, overflow: element.scrollWidth - element.clientWidth,
        controls: [...element.querySelectorAll('[data-notification-channel] input, [data-notification-channel] select, [data-notification-channel] button')].map(control => { const box = control.getBoundingClientRect(); return { left: box.left, right: box.right }; }) };
    });
    assert.ok(geometry.left >= 0 && geometry.right <= width, JSON.stringify({ width, geometry }));
    assert.ok(geometry.overflow <= 1, `dialog overflows at ${width}: ${JSON.stringify(geometry)}`);
    assert.ok(geometry.controls.every(box => box.left >= geometry.left && box.right <= geometry.right), `channel controls escape dialog at ${width}`);
    if (width === 1280) assert.ok(geometry.width >= 700, 'subscription dialog should use its wider layout');
    if (process.env.FERRY_UI_SCREENSHOT_DIR && [375, 1280].includes(width)) await page.screenshot({ path: `${process.env.FERRY_UI_SCREENSHOT_DIR}/subscription-${width}.png` });
  }
  assert.equal(await dialog.getByRole('textbox', { name: '名称 1', exact: true }).count(), 0);
  await dialog.getByRole('button', { name: '添加', exact: true }).click();
  await dialog.getByRole('textbox', { name: 'Webhook URL 2', exact: true }).fill('https://open.feishu.cn/open-apis/bot/v2/hook/example');
  await dialog.getByText('Feishu · 2', { exact: true }).waitFor();
  await dialog.locator('select').filter({ has: page.locator('option[value="ja-JP"]') }).nth(1).selectOption('ja-JP');
  await dialog.getByRole('button', { name: '添加', exact: true }).click();
  await dialog.getByRole('textbox', { name: 'Webhook URL 3', exact: true }).fill('https://open.feishu.cn/open-apis/bot/v2/hook/second');
  await dialog.getByRole('button', { name: '确认修改订阅配置', exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(saves, 1);
  assert.deepEqual(monitor.notification_channels.map(channel => [channel.name, channel.language]), [['Operations', 'zh-CN'], ['Feishu', 'ja-JP'], ['Feishu 2', 'en-US']]);
  await page.reload();
  await page.getByRole('button', { name: '修改订阅配置', exact: true }).click();
  assert.equal(await dialog.getByRole('textbox', { name: 'Webhook URL 2', exact: true }).inputValue(), 'https://open.feishu.cn/open-apis/bot/v2/hook/example');
  const languages = dialog.locator('select').filter({ has: page.locator('option[value="ja-JP"]') });
  assert.deepEqual(await languages.evaluateAll(selects => selects.map(select => select.value)), ['zh-CN', 'ja-JP', 'en-US']);
  await dialog.getByRole('button', { name: '删除 1', exact: true }).click();
  await dialog.getByRole('button', { name: '确认修改订阅配置', exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  assert.equal(monitor.notification_channels.length, 2);
  assert.equal(monitor.notification_channels[0].name, 'Feishu');
  await page.goto(`${origin}/zh-CN/workspaces/${workspace}/code-connections`);
  await page.getByRole('columnheader', { name: '最近提交事件', exact: true }).waitFor();
  const recent = page.getByRole('cell').filter({ hasText: 'Latest commit across branches' });
  assert.match(await recent.innerText(), /feature\/non-default/);
  assert.deepEqual(errors, []);
  console.log('PASS visible saved Webhook URLs, multiple independent language channels, save/reload and removal in production Chromium UI');
  console.log('PASS connection list renders latest cross-branch commit event summary, branch and timestamp');
  console.log('PASS subscription dialog and channel controls stay within 320/375/768/1280px screens; URL auto-detection requires no name input');
} finally { await browser?.close(); await new Promise(done => server.httpServer.close(done)); }
