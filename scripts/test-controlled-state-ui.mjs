/** Production UI: legacy Chinese state seeds must follow the active locale. */
import assert from 'node:assert/strict';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.FERRY_UI_ORIGIN || `http://127.0.0.1:${server.httpServer.address().port}`;
const labels = { 'en-US': 'Online', 'zh-CN': '在线', 'zh-TW': '連線中', 'ja-JP': 'オンライン', 'ko-KR': '온라인', 'ms-MY': 'Dalam talian' };
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  for (const [locale, expected] of Object.entries(labels)) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/api/**', async route => {
      const path = new URL(route.request().url()).pathname;
      let json = { items: [], total: 0, page: 1, page_size: 20 };
      if (path === '/api/me') json = { user: { id: '1', name: 'Tester', email: 'tester@example.invalid' }, workspaces: [{ id: '1', slug: 'ops', name: 'Workspace', permissions: ['hosts.read', 'host-groups.read'] }] };
      else if (path === '/api/setup') json = { required: false };
      else if (path.endsWith('/preferences')) json = { default_page_size: 20, dictionaries: { 'host-state': [{ value: 'online', label: '在线', labels, color: '#123456' }] } };
      await route.fulfill({ json });
    });
    await page.goto(`${origin}/${locale}/workspaces/1/nodes`);
    const control = page.locator('form [role=combobox]').last();
    await control.waitFor();
    await control.click();
    await page.getByRole('option', { name: expected, exact: true }).click();
    assert.ok((await control.innerText()).includes(expected), `${locale}: localized filter selection`);
    // The browser must submit the fixed SQL/API value, never a translated label.
    const request = page.waitForRequest(request => request.url().includes('/nodes?') && new URL(request.url()).searchParams.get('state') === 'online');
    await page.locator('form button[type=submit]').click();
    await request;
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log('PASS six-language state dropdowns, legacy labels, colors and fixed API filter values');
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
