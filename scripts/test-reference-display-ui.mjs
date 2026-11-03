/** Production browser regression for optional references, valid zero values and stale IDs. */
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
const origin = process.env.REFERENCE_UI_URL || 'http://127.0.0.1:5194';
const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const browser = await chromium.launch({ headless: true, ...(process.platform === 'win32' && existsSync(chrome) ? { executablePath: chrome } : {}) });
try {
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1366, height: 900 } });
  const page = await context.newPage();
  const errors = []; const writes = [];
  page.on('pageerror', error => errors.push(error.message));
  const permissions = ['hosts.read', 'hosts.update', 'host-groups.read', 'host-groups.update', 'tags.read', 'credentials.read'];
  const host = { id: '17', name: 'SSH target', connection: 'ssh', state: 'offline', group_id: '0', jump_host_id: 0, credential_id: '8', address: 'target.example.invalid', port: 22, username: 'root', tag_ids: [], version: 1 };
  const group = { id: '2', name: 'Production group', parent_id: 0, description: '', version: 1 };
  await page.route('**/api/**', async route => {
    const request = route.request(); const url = new URL(request.url()); const path = url.pathname;
    let data = { items: [], total: 0, page: 1, page_size: 200 };
    if (path === '/api/setup') data = { required: false, has_users: true };
    else if (path === '/api/me') data = { user: { id: '1', name: 'Reviewer', email: 'reviewer@example.invalid' }, workspaces: [{ id: '2', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions }] };
    else if (path.endsWith('/preferences')) data = { default_page_size: 20, dictionaries: {} };
    else if (path === '/api/workspaces/2/nodes/17') {
      if (request.method() === 'PATCH') { const body = request.postDataJSON(); writes.push(body); Object.assign(host, body); data = { id: host.id, version: ++host.version }; }
      else data = { ...host };
    } else if (path === '/api/workspaces/2/nodes') data = { items: [{ id: '3', name: 'Jump server', connection: 'ssh', jump_host_id: '0' }], total: 1, page: 1, page_size: 200 };
    else if (path === '/api/workspaces/2/node-groups/2') data = { ...group };
    else if (path === '/api/workspaces/2/node-groups') data = { items: [{ ...group }, { id: '4', name: 'Parent group', parent_id: '0' }], total: 2, page: 1, page_size: 200 };
    else if (path === '/api/workspaces/2/credentials') data = { items: [{ id: '8', name: 'SSH login', kind: 'password' }], total: 1, page: 1, page_size: 200 };
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(data) });
  });
  const detail = `${origin}/zh-CN/workspaces/2/nodes/17?tab=info`;
  await page.goto(detail);
  try { await page.getByText('未分组', { exact: true }).waitFor(); }
  catch (error) { console.error('Host page diagnostic:', page.url(), await page.locator('body').innerText()); throw error; }
  await page.getByText('不使用跳板主机', { exact: true }).waitFor();
  assert.equal(writes.length, 0);
  assert.equal(host.group_id, '0'); assert.equal(host.jump_host_id, 0);
  await page.goto(`${origin}/zh-CN/workspaces/2/nodes/17/edit`);
  assert.equal(await page.getByRole('combobox', { name: '主机组', exact: true }).textContent(), '未分组');
  assert.equal(await page.getByRole('combobox', { name: '跳板主机', exact: true }).textContent(), '不使用跳板主机');
  await page.getByRole('combobox', { name: '主机组', exact: true }).click();
  await page.getByRole('option', { name: 'Production group', exact: true }).click();
  await page.getByRole('combobox', { name: '主机组', exact: true }).click();
  await page.getByRole('option', { name: '未分组', exact: true }).click();
  assert.equal(writes.length, 0);
  host.group_id = '2'; host.jump_host_id = '3';
  await page.goto(detail);
  await page.getByText('Production group', { exact: true }).waitFor();
  await page.getByText('Jump server', { exact: true }).waitFor();
  host.group_id = '999'; host.jump_host_id = '998';
  await page.goto(detail);
  await page.getByText('选项不可用（999）', { exact: true }).waitFor();
  await page.getByText('选项不可用（998）', { exact: true }).waitFor();
  const groupDetail = `${origin}/zh-CN/workspaces/2/nodes`;
  const openGroup = async () => {
    await page.goto(groupDetail);
    await page.getByRole('button', { name: '操作 · Production group', exact: true }).click();
    await page.getByRole('menuitem', { name: '编辑分组', exact: true }).click();
    return page.getByRole('dialog').getByRole('combobox', { name: '上级分组', exact: true });
  };
  for (const parent of [0, '0', '', null]) {
    group.parent_id = parent;
    const parentSelect = await openGroup();
    assert.equal(await parentSelect.textContent(), '顶级分组');
    assert.equal(group.parent_id, parent);
  }
  group.parent_id = '4';
  assert.equal(await (await openGroup()).textContent(), 'Parent group');
  group.parent_id = '999';
  assert.equal(await (await openGroup()).textContent(), '选项不可用（999）');
  assert.equal(writes.length, 0);
  assert.deepEqual(errors, []);
  console.log('PASS: host detail/edit and group parent zero/null/empty references, names, unavailable references and explicit clearing; presentation causes no writes');
} finally { await browser.close(); }
