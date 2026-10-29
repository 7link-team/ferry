/** Browser acceptance against the production frontend, with isolated API fixtures. */
import assert from 'node:assert/strict';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  let permissions = ['hosts.read', 'hosts.create', 'hosts.enroll'];
  let connection = 'runner', listReads = 0, creates = 0, tokenState = 'pending', online = false, consumed = false;
  const host = () => ({ id: 'h1', name: 'Fixture host', address: 'fixture.invalid', connection, state: online ? 'online' : 'offline', port: 22, username: 'root', tags: [], tag_ids: [], group_id: '', credential_id: '', description: '', version: 1 });
  await page.route('**/api/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    let body;
    if (path === '/api/me') body = { user: { id: 'u1', name: 'Tester', email: 'tester@example.invalid', is_superadmin: false }, workspaces: [{ id: 'w1', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions }] };
    else if (path === '/api/setup') body = { required: false };
    else if (path.endsWith('/preferences')) body = { default_page_size: 20, dictionaries: {} };
    else if (path === '/api/workspaces/ops/hosts') { listReads++; body = { items: [host()], total: 1, page: 1, page_size: 20 }; }
    else if (path === '/api/workspaces/ops/hosts/h1') body = host();
    else if (path === '/api/workspaces/ops/runner-tokens' && request.method() === 'POST') {
      creates++; tokenState = 'pending'; consumed = false;
      const input = request.postDataJSON();
      assert.equal(input.ttl_minutes, 15);
      assert.equal(input.host_id, new URL(page.url()).pathname.endsWith('/h1') ? 'h1' : '');
      body = { id: `token-${creates}`, token: `fixture-secret-${creates}`, expires_at: Date.now() + 15 * 60000, server: 'https://fixture.example:8443' };
    } else if (/\/runner-tokens\/token-\d+$/.test(path)) body = { id: path.split('/').at(-1), state: tokenState, expires_at: Date.now() + 15 * 60000, used_at: consumed || tokenState === 'connected' ? Date.now() : 0, host_id: tokenState === 'connected' ? 'h1' : '', host_state: online ? 'online' : '' };
    else body = { items: [], total: 0, page: 1, page_size: 20 };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const exactButton = name => page.getByRole('button', { name, exact: true });
  await page.goto(`${origin}/ops/host/hosts/new`);
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

  await page.goto(`${origin}/ops/host/hosts`);
  await exactButton('连接主机').click({ timeout: 10000 });
  await exactButton('生成安装命令').click();
  await command.waitFor();
  assert.match(await command.inputValue(), /mktemp -d/);
  const countdown = await page.getByText(/一次性命令，剩余/).textContent();
  await page.waitForFunction(previous => [...document.querySelectorAll('p')].some(p => p.textContent.includes('一次性命令，剩余') && p.textContent !== previous), countdown);

  assert.match(await command.inputValue(), /--server 'https:\/\/fixture.example:8443'/);
  await exactButton('复制命令').click();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), await command.inputValue());
  await page.getByRole('tab', { name: 'Windows', exact: true }).click();
  assert.match(await command.inputValue(), /powershell.exe/);
  assert.match(await command.inputValue(), /-Server ''https:\/\/fixture.example:8443''/);
  assert.match(await command.inputValue(), /-MaximumRedirection 0/);
  assert.match(await command.inputValue(), /-EncodedCommand/);
  assert.doesNotMatch(await command.inputValue(), /-OutFile/);
  await page.getByText('支持 x64；请以管理员身份打开 PowerShell。', { exact: true }).waitFor();
  await page.getByRole('tab', { name: 'macOS', exact: true }).click();
  assert.match(await command.inputValue(), /install.sh/);
  const address = page.getByLabel('2. 目标主机可访问的服务地址');
  await address.fill('https://fixture.example/path');
  await command.waitFor({ state: 'detached' });
  await address.fill('http://remote.example:4290');
  await page.getByText('远程安装请使用 HTTPS，HTTP 仅支持本机测试', { exact: true }).waitFor();
  assert.equal(await command.count(), 0);
  await address.fill('https://reachable.example:9443');
  await command.waitFor();
  assert.match(await command.inputValue(), /https:\/\/reachable.example:9443/);
  console.log('PASS OS choices, origin validation, live countdown, command generation and actual clipboard copy');

  // Small-screen dialog stays within the viewport with a scrollable interior.
  await page.setViewportSize({ width: 390, height: 844 });
  const bounds = await page.getByRole('dialog').boundingBox();
  assert(bounds.x >= 0 && bounds.x + bounds.width <= 391 && bounds.y >= 0 && bounds.y + bounds.height <= 845);
  await page.setViewportSize({ width: 1280, height: 800 });
  consumed = true;
  await page.getByText('已注册，等待主机上线…', { exact: true }).waitFor({ timeout: 10000 });
  tokenState = 'expired';
  await page.getByText('命令已过期', { exact: true }).waitFor({ timeout: 10000 });
  assert.equal(await command.count(), 0);
  await exactButton('重新生成命令').click();
  await command.waitFor();
  assert.equal(creates, 3);
  const before = listReads;
  tokenState = 'connected'; online = true;
  await page.getByText('主机已连接', { exact: true }).waitFor({ timeout: 10000 });
  await page.waitForFunction(() => document.querySelector('table')?.textContent?.includes('在线'));
  assert(listReads > before, 'successful enrolment refreshes the existing shared list');
  assert.equal(await page.getByRole('link', { name: '查看主机详情' }).getAttribute('href'), '/ops/host/hosts/h1');
  assert.equal(await page.evaluate(() => Object.values(localStorage).some(v => v.includes('fixture-secret'))), false);
  console.log('PASS expired/regenerate, live connection, list refresh, host link and memory-only credential');

  await page.goto(`${origin}/ops/host/hosts/h1?tab=connect`);
  await page.getByRole('tab', { name: '接入 Agent', exact: true }).waitFor();
  await exactButton('生成安装命令').click();
  await command.waitFor();
  assert.equal(creates, 4);
  console.log('PASS Agent detail enrolment with scoped host ID');

  connection = 'ssh';
  await page.reload();
  await page.getByRole('tab', { name: '信息', exact: true }).waitFor();
  assert.equal(await page.getByRole('tab', { name: '接入 Agent', exact: true }).count(), 0);
  assert.equal(await exactButton('生成安装命令').count(), 0);
  assert.equal(creates, 4);
  console.log('PASS SSH detail never offers Agent enrolment, including a direct tab URL');

  permissions = ['hosts.read']; connection = 'runner';
  await page.goto(`${origin}/ops/host/hosts`);
  await page.getByRole('table').waitFor();
  assert.equal(await exactButton('连接主机').count(), 0);
  await page.goto(`${origin}/ops/host/hosts/h1?tab=connect`);
  await page.getByRole('tab', { name: '信息', exact: true }).waitFor();
  assert.equal(await page.getByRole('tab', { name: '接入 Agent', exact: true }).count(), 0);
  assert.equal(await exactButton('生成安装命令').count(), 0);
  assert.equal(creates, 4);
  permissions = ['hosts.read', 'hosts.create', 'hosts.enroll'];
  await page.goto(`${origin}/ops/host/hosts/new`);
  await command.waitFor();
  await page.getByRole('tab', { name: 'SSH', exact: true }).click();
  assert.equal(await page.getByRole('tab', { name: '代理 Runner' }).count(), 1);
  assert.equal(await page.getByRole('tab', { name: 'SSH', exact: true }).getAttribute('aria-selected'), 'true');
  await page.getByRole('textbox', { name: /主机地址/ }).waitFor();
  await page.getByRole('tab', { name: '代理 Runner' }).click();
  await command.waitFor();
  assert.equal(await page.getByRole('tab', { name: '代理 Runner' }).getAttribute('aria-selected'), 'true');
  console.log('PASS new host switches Runner to SSH and back with the tabs retained');

  permissions = ['pipelines.read', 'pipeline-groups.read', 'pipeline-groups.create'];
  await page.goto(`${origin}/ops/build/pipelines`);
  const directory = page.getByRole('complementary', { name: '流水线分组' });
  await directory.getByRole('button', { name: '管理分组' }).click();
  assert.equal(new URL(page.url()).pathname, '/ops/build/pipelines/groups');
  assert.match(await page.getByRole('navigation', { name: '面包屑' }).textContent(), /构建\s*\/\s*流水线\s*\/\s*分组/);
  console.log('PASS pipeline group management uses the nested route and breadcrumb');

  permissions = ['pipelines.read', 'pipelines.create', 'pipeline-groups.read'];
  await page.goto(`${origin}/ops/build/pipelines/new`);
  await page.getByRole('tab', { name: 'YAML', exact: true }).click();
  await page.locator('.monaco-editor').waitFor({ timeout: 20000 });
  assert.equal(await page.getByRole('textbox', { name: 'GitHub Actions YAML' }).count(), 1);
  console.log('PASS production pipeline editor loads local Monaco YAML');
  assert.deepEqual(errors, []);
  console.log('PASS list/detail permissions and no browser runtime errors');
} finally {
  await browser?.close();
  await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
