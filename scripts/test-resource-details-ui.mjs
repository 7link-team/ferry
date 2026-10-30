import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { preview } from 'vite';
import { chromium } from 'playwright';

// Read-only details exercise the shared schema renderer, including asynchronous
// option labels, secrets, full-span content and long unbroken values.
const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
const workspace = '109124796398174208';
const longAddress = 'node-' + 'long-hostname-'.repeat(16) + '.example.invalid';
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    const list = items => ({ items, total: items.length, page: 1, page_size: 200 });
    const rows = {
      nodes: [{ id: 'n1', name: 'Production node', connection: 'ssh', address: longAddress, port: 22, username: 'operator', credential_id: 'c1', group_id: 'g1', tag_ids: ['t1'], version: 1 }],
      'node-groups': [{ id: 'g1', name: 'Production', parent_id: '0' }],
      tags: [{ id: 't1', name: 'Critical', color: '#2563eb' }],
      credentials: [{ id: 'c1', name: 'SSH login', scope: 'build', kind: 'password', username: 'operator', description: 'Operations credential', has_secret: true }],
      members: [{ id: 'm1', name: 'Workspace owner', email: 'owner@example.invalid', role_name: 'Owner', owner: true }],
    };
    let value;
    if (path === '/api/setup') value = { required: false };
    else if (path === `/api/workspaces/${workspace}/preferences`) value = { default_page_size: 20, dictionaries: {} };
    else if (path === '/api/me') value = { user: { id: 'u1', name: 'Tester', email: 'test@example.invalid', is_superadmin: false }, workspaces: [{ id: workspace, name: 'Operations', slug: 'ops', role_name: 'Owner', permissions: ['hosts.read', 'hosts.update', 'host-groups.read', 'tags.read', 'credentials.read', 'members.read'] }] };
    else {
      const [resource, id] = path.replace(`/api/workspaces/${workspace}/`, '').split('/');
      value = id ? rows[resource]?.find(row => row.id === id) : list(rows[resource] ?? []);
    }
    return route.fulfill({ status: value ? 200 : 404, contentType: 'application/json', body: JSON.stringify(value ?? { error: `Missing fixture ${path}` }) });
  });
  if (process.env.DETAIL_SCREENSHOT_DIR) await mkdir(process.env.DETAIL_SCREENSHOT_DIR, { recursive: true });
  for (const [resource, id] of [['nodes', 'n1'], ['tags', 't1'], ['credentials', 'c1'], ['members', 'm1']]) {
    await page.goto(`${origin}/workspaces/${workspace}/${resource}/${id}`);
    await page.locator('.detail-section').first().waitFor({ timeout: 10000 }).catch(async error => {
      console.error(resource, await page.locator('body').innerText(), errors);
      throw error;
    });
    await page.waitForFunction(() => !document.body.innerText.includes('正在载入'));
    for (const width of [1920, 1280, 768, 375]) {
      await page.setViewportSize({ width, height: 900 });
      for (const dark of [false, true]) {
        await page.evaluate(value => document.documentElement.classList.toggle('dark', value), dark);
        await page.waitForTimeout(200); // Allow existing theme color transitions to settle.
        const geometry = await page.locator('.resource-detail').evaluate(form => ({
          width: form.getBoundingClientRect().width,
          available: form.parentElement.getBoundingClientRect().width,
          overflow: form.scrollWidth - form.clientWidth,
          fields: [...form.querySelectorAll('.resource-field')].map(field => ({ width: field.clientWidth, scroll: field.scrollWidth })),
        }));
        assert(geometry.width >= geometry.available - 2, `${resource}: details leave unused width at ${width}`);
        assert(geometry.overflow <= 1, `${resource}: detail overflow at ${width}`);
        assert(geometry.fields.every(field => field.scroll <= field.width + 1), `${resource}: field overflow at ${width}`);
        assert.equal(await page.getByRole('button', { name: '返回列表', exact: true }).count(), 0);
        if (resource === 'nodes' && process.env.DETAIL_SCREENSHOT_DIR) await page.screenshot({ path: `${process.env.DETAIL_SCREENSHOT_DIR}/node-${width}-${dark ? 'dark' : 'light'}.png`, fullPage: true });
      }
    }
    if (resource === 'nodes') {
      assert.equal(await page.locator('.resource-detail').getByText('Production', { exact: true }).count(), 1);
      assert.equal(await page.locator('.resource-detail').getByText('Critical', { exact: true }).count(), 1);
      assert.equal(await page.locator('.resource-detail').getByText(/^SSH login（/).count(), 1);
      assert.equal(await page.locator('.resource-detail input:not([type=hidden])').count(), 0);
    }
  }
  assert.deepEqual(errors, []);
  console.log('PASS shared node/tag/credential/member details: 375/768/1280/1920px, light/dark, long values, resolved labels, full width, no disabled text inputs');
} finally {
  await browser?.close();
  await server.close();
}
