/** Browser acceptance against the production frontend, with isolated API fixtures. */
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.FERRY_UI_ORIGIN || `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, viewport: { width: Number(process.env.FERRY_UI_VIEWPORT_WIDTH || 1280), height: 800 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  let permissions = ['hosts.read', 'hosts.create', 'hosts.enroll', 'hosts.update'];
  let connection = 'runner', listReads = 0, creates = 0, resets = 0, online = false;
  let currentToken = 'fixture-current-token';
  let updates = 0;
  let updateState = null;
  const host = () => ({ id: 'h1', name: 'Fixture host', address: 'fixture.invalid', connection, state: online ? 'online' : 'offline', port: 22, username: 'root', tags: [], tag_ids: [], group_id: '', credential_id: '', description: '', agent_version: '0.1.0-alpha.38', agent_update_available: true, agent_update_state: updateState?.state, execution: { enabled: true, shared: true, capacity: 2 }, execution_status: { active: 0, capacity: 2, state: 'available', running_jobs: [] }, version: 1 });
  await page.route('**/api/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    let body;
    if (path === '/api/me') body = { user: { id: 'u1', name: 'Tester', email: 'tester@example.invalid', is_superadmin: false }, workspaces: [{ id: '109124796398174208', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions }] };
    else if (path === '/api/setup') body = { required: false };
    else if (path.endsWith('/preferences')) body = { default_page_size: 20, dictionaries: {} };
    else if (path === '/api/workspaces/109124796398174208/nodes') { listReads++; body = new URL(request.url()).searchParams.get('connection') === 'ssh' ? { items: [host(), { ...host(), id: 'h2', name: 'Valid jump host', jump_host_id: '0' }], total: 2, page: 1, page_size: 200 } : { items: [host()], total: 1, page: 1, page_size: 20 }; }
    else if (path === '/api/workspaces/109124796398174208/nodes/h1/runner-updates') {
      if (request.method() === 'POST') { assert.deepEqual(request.postDataJSON(), {}); updates++; updateState = { state: 'draining', version: '0.2.0-alpha.1', message: 'Waiting for tasks' }; }
      body = { available: true, agent_version: '0.1.0-alpha.38', target_version: '0.2.0-alpha.1', supported: true, ready: online, enabled: true, auto_update: 'inherit', update: updateState };
    }
    else if (path === '/api/workspaces/109124796398174208/nodes/h1') body = host();
    else if (path === '/api/workspaces/109124796398174208/nodes/h1/metrics') body = { cpu_percent: 12.5, load1: 0.2, memory_used: 1024, memory_total: 4096, disks: [{ name: 'system', mount: '/', used: 1024, total: 4096 }], networks: [{ name: 'eth0', received: 10, transmitted: 20 }] };
    else if (path === '/api/workspaces/109124796398174208/nodes/h1/connection-credentials') {
      if (request.method() === 'POST') { resets++; currentToken = `fixture-reset-${resets}`; online = false; }
      body = { token: currentToken, server: 'https://fixture.example:8443', state: online ? 'online' : 'offline', os: 'linux', installation_scope: 'system' };
    }
    else if (path === '/api/workspaces/109124796398174208/runner-tokens' && request.method() === 'POST') {
      creates++;
      const input = request.postDataJSON();
      assert.equal(input.ttl_minutes, 15);
      assert.equal(input.host_id, new URL(page.url()).pathname.endsWith('/h1') ? 'h1' : '');
      body = { id: `token-${creates}`, token: `fixture-secret-${creates}`, expires_at: Date.now() + 15 * 60000, server: 'https://fixture.example:8443' };
    } else if (/\/runner-tokens\/token-\d+$/.test(path)) body = { id: path.split('/').at(-1), state: 'pending', expires_at: Date.now() + 15 * 60000, used_at: 0, host_id: '', host_state: '' };
    else body = { items: [], total: 0, page: 1, page_size: 20 };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const exactButton = name => page.getByRole('button', { name, exact: true });
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes/new`);
  const command = page.getByRole('textbox', { name: '安装命令' });
  await command.waitFor({ timeout: 10000 });
  assert.match(await command.inputValue(), /fixture-secret-1/);
  assert.equal(creates, 1, 'new Runner host generates a token without saving a host');
  assert.equal(await exactButton('生成安装命令').count(), 0);
  await page.getByRole('tab', { name: 'Windows', exact: true }).click();
  assert.match(await command.inputValue(), /install.ps1/);
  await page.getByRole('tab', { name: 'Linux', exact: true }).click();
  assert.match(await command.inputValue(), /install.sh/);
  console.log('PASS new Runner host shows a ready-to-copy Linux/Windows command without saving');

  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes`);
  await page.getByRole('table').waitFor();
  await page.locator('tbody').getByText('主机代理', { exact: true }).waitFor();
  assert.equal(await page.getByText('主机代理（HostAgent）', { exact: true }).count(), 0);
  assert.equal(await exactButton('连接主机').count(), 0);
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes/h1`);
  const current = page.getByRole('textbox', { name: '当前连接命令' });
  await current.waitFor();
  // Visibility precedes the credential response; wait for the ready command.
  await page.waitForFunction(() => document.querySelector('textarea[aria-label="当前连接命令"]')?.value.includes('fixture-current-token'));
  assert.match(await current.inputValue(), /fixture-current-token/);
  if (process.env.FERRY_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.FERRY_SCREENSHOT_DIR, 'host-detail-info.png'), fullPage: true });
  await page.getByRole('tab', { name: '监控', exact: true }).click();
  await page.getByText('12.5%').waitFor();
  if (process.env.FERRY_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.FERRY_SCREENSHOT_DIR, 'host-detail-monitor.png'), fullPage: true });
  await exactButton('定时刷新').click();
  await page.getByRole('menuitem', { name: '每 15 秒', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '15s');
  await exactButton('刷新').click();
  await page.getByRole('tab', { name: '基本信息' }).click();
  assert.equal(await page.locator('[data-slot=refresh-control]').count(), 0);
  await page.getByRole('tab', { name: 'Windows', exact: true }).click();
  assert.match(await current.inputValue(), /powershell.exe/);
  await page.getByRole('tab', { name: 'Linux', exact: true }).click();
  await exactButton('复制命令').click();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), await current.inputValue());
  await page.reload();
  // The command fills in after the credential request resolves, not on render.
  await page.waitForFunction(() => document.querySelector('textarea[aria-label="当前连接命令"]')?.value.includes('fixture-current-token'));
  assert.match(await current.inputValue(), /fixture-current-token/);
  await exactButton('重置连接').click();
  assert.equal(resets, 0);
  await page.getByRole('dialog').getByRole('button', { name: '确认重置' }).click();
  await page.waitForFunction(() => document.querySelector('textarea[aria-label="当前连接命令"]')?.value.includes('fixture-reset-1'));
  assert.equal(resets, 1);
  await page.getByText('已断开连接', { exact: true }).waitFor();
  console.log('PASS stable detail command, confirmed reset, disconnected status and clipboard');

  connection = 'ssh';
  await page.reload();
  assert.equal(await page.getByRole('textbox', { name: '当前连接命令' }).count(), 0);
  console.log('PASS SSH detail has no Agent command');

  permissions = ['hosts.read', 'hosts.update']; connection = 'runner';
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes`);
  await page.getByRole('table').waitFor();
  await page.getByText('HostAgent 0.1.0-alpha.38', { exact: true }).waitFor();
  await page.getByText('共享', { exact: true }).waitFor();
  const updateNow = page.getByRole('button', { name: '立即更新', exact: true });
  assert.equal(await updateNow.isDisabled(), true);
  online = true;
  await page.reload();
  await updateNow.click();
  await page.getByRole('status').filter({ hasText: '等待任务结束' }).waitFor();
  assert.equal(updates, 1);
  updateState = null;
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes/h1?tab=connect`);
  await page.getByText('HostAgent 0.1.0-alpha.38', { exact: true }).waitFor();
  await Promise.all([page.waitForResponse(response => response.url().endsWith('/runner-updates') && response.request().method() === 'POST'), updateNow.click()]);
  assert.equal(updates, 2);
  console.log('PASS list/detail version, shared badge, offline guard and signed-update requests');
  connection = 'ssh';
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes/h1/edit`);
  const jump = page.getByRole('combobox', { name: '跳板主机', exact: true });
  await jump.click();
  await page.getByRole('option', { name: 'Valid jump host', exact: true }).waitFor();
  assert.equal(await page.getByRole('option', { name: 'Fixture host', exact: true }).count(), 0);
  await page.getByRole('option', { name: 'Valid jump host', exact: true }).click();
  console.log('PASS edited node is excluded from jump-host dropdown, valid peer remains selectable');
  connection = 'runner';
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes/h1`);

  permissions = ['hosts.read'];
  await page.reload();
  await page.getByText('HostAgent 0.1.0-alpha.38', { exact: true }).waitFor();
  assert.equal(await updateNow.count(), 0);

  assert.equal(await page.getByRole('textbox', { name: '当前连接命令' }).count(), 0);
  assert.equal(await exactButton('重置连接').count(), 0);
  permissions = ['hosts.read', 'hosts.create', 'hosts.enroll'];
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/nodes/new`);
  await command.waitFor();
  await page.getByRole('tab', { name: 'SSH', exact: true }).click();
  assert.equal(await page.getByRole('tab', { name: '主机代理' }).count(), 1);
  assert.equal(await page.getByRole('tab', { name: 'SSH', exact: true }).getAttribute('aria-selected'), 'true');
  await page.getByRole('textbox', { name: /主机地址/ }).waitFor();
  await page.getByRole('tab', { name: '主机代理' }).click();
  await command.waitFor();
  assert.equal(await page.getByRole('tab', { name: '主机代理' }).getAttribute('aria-selected'), 'true');
  console.log('PASS new host switches Runner to SSH and back with the tabs retained');

  permissions = ['pipelines.read', 'pipelines.create', 'pipeline-groups.read'];
  await page.goto(`${origin}/zh-CN/workspaces/109124796398174208/pipelines/new`);
  await page.getByRole('tab', { name: '高级 YAML', exact: true }).click();
  await page.locator('.monaco-editor').waitFor({ timeout: 20000 });
  assert.equal(await page.getByRole('textbox', { name: 'GitHub Actions YAML' }).count(), 1);
  console.log('PASS production pipeline editor loads local Monaco YAML');
  assert.deepEqual(errors, []);
  console.log('PASS list/detail permissions and no browser runtime errors');
} finally {
  await browser?.close();
  await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
