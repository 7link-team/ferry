import assert from 'node:assert/strict';
import { join } from 'node:path';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  let saved = '';
  let createdEntry;
  let deletedPath;
  const longName = 'deployment-artifact-with-a-very-long-filename-that-must-remain-left-aligned-and-truncated-in-the-file-list.log';
  let groups = [
    { id: 'g1', name: 'Alpha', description: '', parent_id: '0', position: 0, version: 1 },
    { id: 'g2', name: 'Beta', description: '', parent_id: '0', position: 1, version: 1 },
  ];
  let reordered = false;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    if (path === '/api/me') return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ user: { id: 'u1', name: 'Tester', email: 'test@example.invalid', is_superadmin: false }, workspaces: [{ id: 'w1', slug: 'ops', name: 'Ops', role_name: 'Owner', permissions: ['hosts.read', 'hosts.update', 'hosts.delete', 'hosts.enroll', 'hosts.files', 'hosts.terminal', 'host-groups.read', 'host-groups.update', 'host-groups.delete'] }] }) });
    if (path === '/api/setup') return route.fulfill({ contentType: 'application/json', body: '{"required":false}' });
    if (path === '/api/workspaces/ops/preferences') return route.fulfill({ contentType: 'application/json', body: '{"default_page_size":20,"dictionaries":{}}' });
    if (path === '/api/workspaces/ops/tags') return route.fulfill({ contentType: 'application/json', body: '{"items":[],"total":0,"page":1,"page_size":200}' });
    if (path === '/api/workspaces/ops/host-groups/actions/reorder' && request.method() === 'POST') {
      const body = request.postDataJSON();
      assert.deepEqual(body, { parent_id: '0', ids: ['g2', 'g1'] });
      groups = body.ids.map((id, position) => ({ ...groups.find(group => group.id === id), position, version: 2 }));
      reordered = true;
      return route.fulfill({ contentType: 'application/json', body: '{"ok":true}' });
    }
    if (path === '/api/workspaces/ops/host-groups') return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ items: groups, total: groups.length, page: 1, page_size: 200 }) });
    if (path === '/api/workspaces/ops/hosts') return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ items: [{ id: 'h1', name: 'build-01', address: '192.168.0.130', connection: 'runner', state: 'online', tags: [], tag_ids: [], group_id: 'g1', seen_at: Date.now(), version: 1 }], total: 1, page: 1, page_size: 20 }) });
    if (path === '/api/workspaces/ops/hosts/h1') return route.fulfill({ contentType: 'application/json', body: '{"id":"h1","name":"build-01","connection":"runner"}' });
    if (path === '/api/workspaces/ops/hosts/h1/files') {
      if (request.method() === 'POST') { createdEntry = request.postDataJSON(); return route.fulfill({ status: 201, body: '' }); }
      if (request.method() === 'DELETE') { deletedPath = url.searchParams.get('path'); return route.fulfill({ status: 204, body: '' }); }
      return route.fulfill({ contentType: 'application/json', body: JSON.stringify({ path: url.searchParams.get('path'), items: [{ name: 'logs', kind: 'directory', size: 0, mode: 493, modified_at: 0 }, { name: 'note.txt', kind: 'file', size: 12, mode: 420, modified_at: 0 }, { name: longName, kind: 'file', size: 12, mode: 420, modified_at: 0 }] }) });
    }
    if (path === '/api/workspaces/ops/hosts/h1/files/content') {
      if (request.method() === 'PUT') { saved = request.postData() ?? ''; return route.fulfill({ status: 204, body: '' }); }
      return route.fulfill({ status: 200, contentType: 'application/octet-stream', body: url.searchParams.get('path') === '/new.txt' ? '' : 'hello world\n' });
    }
    return route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: `fixture missing ${path}` }) });
  });
  await page.goto(`${origin}/ops/host/nodes/h1/workspace?tool=files`);
  await page.getByRole('heading', { name: 'build-01' }).waitFor();
  await page.setViewportSize({ width: 900, height: 800 });
  const longLabel = page.locator('table tbody span[title]').filter({ hasText: longName }).first();
  await longLabel.waitFor();
  assert.equal(await longLabel.getAttribute('title'), longName);
  assert.equal(await longLabel.evaluate(node => getComputedStyle(node).textOverflow), 'ellipsis');
  assert.equal(await longLabel.evaluate(node => node.scrollWidth > node.clientWidth), true, 'long file names truncate instead of wrapping');
  await page.setViewportSize({ width: 1280, height: 800 });
  assert.equal(await page.getByRole('navigation').count(), 0, 'workspace has no application navigation');
  await page.getByRole('button', { name: '预览 note.txt' }).click();
  const editor = page.getByRole('textbox', { name: '文件内容' });
  assert.equal(await editor.inputValue(), 'hello world\n');
  await editor.fill('updated note\n');
  await page.getByRole('button', { name: '保存', exact: true }).click();
  assert.equal(saved, 'updated note\n');
  await page.getByRole('button', { name: '关闭预览' }).click();
  await page.getByRole('button', { name: '新建文件夹' }).click();
  await page.getByRole('dialog').getByRole('textbox', { name: '名称' }).fill('archive');
  await page.getByRole('dialog').getByRole('button', { name: '创建' }).click();
  assert.deepEqual(createdEntry, { path: '/archive', kind: 'directory' });
  await page.getByRole('tabpanel').getByRole('button', { name: '新建文件', exact: true }).click();
  await page.getByRole('dialog').getByRole('textbox', { name: '名称' }).fill('new.txt');
  await page.getByRole('dialog').getByRole('button', { name: '创建' }).click();
  assert.deepEqual(createdEntry, { path: '/new.txt', kind: 'file' });
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal(await page.getByRole('textbox', { name: '文件内容' }).count(), 0, 'creation leaves preview closed');
  await page.getByRole('button', { name: '删除 note.txt' }).click();
  await page.getByRole('dialog').getByText('确认删除“/note.txt”？').waitFor();
  assert.equal(deletedPath, undefined);
  await page.getByRole('dialog').getByRole('button', { name: '确认删除' }).click();
  assert.equal(deletedPath, '/note.txt');
  await page.getByRole('button', { name: '在 logs 打开终端' }).click();
  assert.equal(await page.getByRole('button', { name: '关闭分屏' }).count(), 2);
  await page.getByRole('button', { name: '上下分屏' }).first().click();
  assert.equal(await page.getByRole('button', { name: '关闭分屏' }).count(), 3);
  await page.getByRole('button', { name: '新建终端' }).click();
  assert.equal(await page.getByRole('tab').count(), 2);
  await page.getByRole('tab', { name: '文件 1' }).click();
  assert.equal(await page.getByRole('button', { name: '关闭分屏' }).count(), 3);
  if (process.env.FERRY_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.FERRY_SCREENSHOT_DIR, 'ferry-host-workspace-desktop.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'workspace fits the mobile viewport');
  const mobilePanes = await page.locator('[role="tabpanel"]:visible section').evaluateAll(nodes => nodes.map(node => {
    const rect = node.getBoundingClientRect();
    return { top: rect.top, bottom: rect.bottom };
  }));
  assert.equal(mobilePanes.length, 3);
  for (let index = 1; index < mobilePanes.length; index++) assert(mobilePanes[index].top >= mobilePanes[index - 1].bottom - 1, 'mobile panes do not overlap');
  if (process.env.FERRY_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.FERRY_SCREENSHOT_DIR, 'ferry-host-workspace-mobile.png'), fullPage: true });
  await page.goto(`${origin}/ops/host/nodes`);
  const terminalLink = page.getByRole('link', { name: '打开终端' });
  await terminalLink.waitFor();
  const actions = await page.locator('table tbody tr').first().locator('td').last().locator('a,button').evaluateAll(nodes => nodes.map(node => node.getAttribute('aria-label')));
  assert.deepEqual(actions, ['打开终端', '打开文件', '测试连接', '查看', '编辑', '删除']);
  await page.getByRole('button', { name: '删除', exact: true }).click();
  await page.getByRole('dialog').getByText('确认删除“build-01”？').waitFor();
  await page.getByRole('dialog').getByRole('button', { name: '取消' }).click();
  assert.equal(await terminalLink.getAttribute('target'), '_blank');
  assert.equal(new URL(await terminalLink.getAttribute('href'), origin).pathname, '/ops/host/nodes/h1/workspace');
  await page.getByRole('button', { name: '上移分组 Beta' }).click();
  assert.equal(reordered, true);
  await page.waitForFunction(() => {
    const labels = [...document.querySelectorAll('aside[aria-label="主机分组"] button')].map(button => button.textContent.trim());
    return labels.indexOf('Beta') < labels.indexOf('Alpha');
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  if (process.env.FERRY_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.FERRY_SCREENSHOT_DIR, 'ferry-host-groups.png'), fullPage: true });
  await page.setViewportSize({ width: 1920, height: 900 });
  await page.goto(`${origin}/ops/host/nodes/h1/edit`);
  const form = page.locator('main form');
  await form.waitFor();
  const bounds = await form.boundingBox();
  assert(bounds.width >= 1000, 'host form uses the wide content area');
  assert(Math.abs(bounds.x - (1920 - bounds.x - bounds.width)) < 200, 'host form is centered on wide screens');
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'host form fits the mobile viewport');
  assert.deepEqual(errors, []);
  console.log('PASS host workspace, splits, file edit, list shortcuts and persisted group ordering');
} finally {
  await browser?.close();
  await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
